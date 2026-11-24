// SPDX-License-Identifier: GPL-3.0-or-later
//! Retain protocol-5 cache compatibility. Each writer publishes a unique temp
//! file atomically, including when two documents open the same prototype.
use super::*;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn path(v: &Obj, mesh: &Obj, fp: &Obj) -> PyResult<PathBuf> {
    let dir = v.call_method0("_chunk_cache_dir")?.extract::<PathBuf>()?;
    let key = v
        .call_method1("_chunk_cache_key", (mesh, fp))?
        .extract::<String>()?;
    Ok(dir.join(format!("{key}.chunk")))
}
#[pyfunction]
fn chunk_cache_load<'py>(
    v: &Obj<'py>,
    group: &Obj<'py>,
    fp: &Obj<'py>,
    vkey: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let mesh = group.getattr("mesh")?;
    let faces = mesh.getattr("faces")?;
    if faces.len()? < v.getattr("_CHUNK_CACHE_MIN_FACES")?.extract()? {
        return Ok(none(py));
    }
    let read = || -> PyResult<Obj> {
        let path = path(v, &mesh, fp)?;
        let bytes = fs::read(&path)?;
        let entry = py
            .import("pickle")?
            .call_method1("loads", (PyBytes::new(py, &bytes),))?;
        if !get(&entry, "nf")?.eq(faces.len()?)?
            || !entry.contains("dback")?
            || !entry.contains("edge_layer")?
        {
            return Ok(none(py));
        }
        entry.set_item("fp", fp)?;
        entry.set_item("vkey", vkey)?;
        entry.set_item("rev", 0)?;
        entry.set_item("uid", uid(py)?)?;
        entry.set_item(
            "faces",
            py.import("builtins")?.call_method1("list", (&faces,))?,
        )?;
        fs::File::open(path)?.set_modified(std::time::SystemTime::now())?;
        Ok(entry)
    };
    match read() {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => Ok(none(py)),
        result => result,
    }
}
fn publish(path: &Path, bytes: &[u8], keep: usize) -> std::io::Result<()> {
    let temp = path.with_extension(format!(
        "{}.{}.part",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut f = fs::File::create_new(&temp)?;
    let written = f.write_all(bytes);
    drop(f);
    if let Err(e) = written.and_then(|_| fs::rename(&temp, path)) {
        let _ = fs::remove_file(temp);
        return Err(e);
    }
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let mut files = Vec::new();
    for f in fs::read_dir(parent)?.flatten() {
        let p = f.path();
        if p.extension().is_some_and(|e| e == "chunk") {
            if let Ok(t) = f.metadata().and_then(|m| m.modified()) {
                files.push((t, p))
            }
        }
    }
    files.sort();
    if keep > 0 {
        for (_, p) in files.iter().take(files.len().saturating_sub(keep)) {
            let _ = fs::remove_file(p);
        }
    }
    Ok(())
}
#[pyfunction]
fn write_document_chunk(payload: &Obj, path: PathBuf, keep: usize) -> PyResult<()> {
    let py = payload.py();
    let write = || -> PyResult<()> {
        let bytes = py
            .import("pickle")?
            .call_method1("dumps", (payload, 5))?
            .cast_into::<PyBytes>()?;
        let data = bytes.as_bytes();
        py.detach(|| publish(&path, data, keep))?;
        Ok(())
    };
    match write() {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => Ok(()),
        result => result,
    }
}
#[pyfunction]
fn chunk_cache_store(v: &Obj, mesh: &Obj, entry: &Obj) -> PyResult<()> {
    let py = v.py();
    if entry
        .get_item("nf")?
        .lt(v.getattr("_CHUNK_CACHE_MIN_FACES")?)?
    {
        return Ok(());
    }
    let snapshot = || -> PyResult<_> {
        let path = path(v, mesh, &entry.get_item("fp")?)?;
        let payload = PyDict::new(py);
        for k in v.getattr("_CHUNK_CACHE_FIELDS")?.try_iter()? {
            let k = k?;
            payload.set_item(
                &k,
                if k.eq("smooth_vcol")? {
                    entry.call_method1("get", (&k, PyBytes::new(py, b"")))?
                } else {
                    entry.get_item(&k)?
                },
            )?
        }
        Ok((payload, path))
    };
    let (payload, path) = match snapshot() {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => return Ok(()),
        r => r?,
    };
    let args = PyDict::new(py);
    args.set_item("target", wrap_pyfunction!(write_document_chunk, py)?)?;
    args.set_item("daemon", true)?;
    args.set_item("args", (payload, path, v.getattr("_CHUNK_CACHE_KEEP")?))?;
    py.import("threading")?
        .getattr("Thread")?
        .call((), Some(&args))?
        .call_method0("start")?;
    Ok(())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(chunk_cache_load, m)?)?;
    m.add_function(wrap_pyfunction!(chunk_cache_store, m)?)?;
    Ok(())
}

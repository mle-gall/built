// SPDX-License-Identifier: GPL-3.0-or-later
//! On-demand component catalogue. Fetch from the configured mirror, publish
//! complete cache entries atomically and keep the last good copy when offline.
use pyo3::{
    prelude::*,
    sync::PyOnceLock,
    types::{PyBytes, PyDict, PyList},
};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};
mod sizing;
type Obj<'p> = Bound<'p, PyAny>;
const BASE: &str = "https://ingetrazo.com/biblioteca";
fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && !id.contains("..")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn cache(py: Python) -> PyResult<PathBuf> {
    let p: PathBuf = py
        .import("core.texture")?
        .call_method0("texture_cache_root")?
        .extract()?;
    Ok(p.parent().unwrap_or(Path::new(".")).join("library"))
}
fn base() -> String {
    std::env::var("INGETRAZO_LIBRARY")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| BASE.into())
        .trim_end_matches('/')
        .into()
}
fn path<'p>(py: Python<'p>, p: &Path) -> PyResult<Obj<'p>> {
    py.import("pathlib")?.call_method1("Path", (p,))
}
fn optional<'p>(py: Python<'p>, p: Option<PathBuf>) -> PyResult<Option<Obj<'p>>> {
    p.as_deref().map(|p| path(py, p)).transpose()
}
#[pyfunction]
fn library_cache(py: Python) -> PyResult<Obj> {
    path(py, &cache(py)?)
}
fn remote(py: Python, base: &str, rel: &str) -> PyResult<Vec<u8>> {
    if !base.contains("://") {
        return Ok(fs::read(Path::new(base).join(rel))?);
    }
    let request = py.import("urllib.request")?;
    let headers = PyDict::new(py);
    headers.set_item(
        "User-Agent",
        py.import("core.version")?.getattr("USER_AGENT")?,
    )?;
    let kw = PyDict::new(py);
    kw.set_item("headers", headers)?;
    let req = request.call_method("Request", (format!("{base}/{rel}"),), Some(&kw))?;
    let kw = PyDict::new(py);
    kw.set_item("timeout", 20)?;
    kw.set_item(
        "context",
        py.import("core.tls")?.call_method0("https_context")?,
    )?;
    let response = request.call_method("urlopen", (req,), Some(&kw))?;
    let reader = response.call_method0("__enter__")?;
    let result = reader.call_method0("read").and_then(|b| b.extract());
    response.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
    result
}
fn temp(parent: &Path) -> PathBuf {
    parent.join(format!(
        ".fetch-{}",
        crate::builtproject::manifest::new_id()
    ))
}
fn publish(dst: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = dst.parent().unwrap();
    fs::create_dir_all(parent)?;
    let tmp = temp(parent);
    let result = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, dst));
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
fn cached(py: Python, root: &Path, base: &str, rel: &str, refresh: bool) -> Option<PathBuf> {
    let dst = root.join(rel);
    if !refresh && dst.is_file() {
        return Some(dst);
    }
    let bytes = remote(py, base, rel).ok();
    if let Some(data) = bytes {
        let _ = py.detach(|| publish(&dst, &data));
    }
    dst.is_file().then_some(dst)
}
#[pyfunction(signature=(refresh=false))]
fn index<'p>(py: Python<'p>, refresh: bool) -> PyResult<Bound<'p, PyList>> {
    let root = cache(py)?;
    let entries = PyList::empty(py);
    let Some(file) = cached(py, &root, &base(), "index.json", refresh) else {
        return Ok(entries);
    };
    let Some(data) = fs::read(file)
        .ok()
        .and_then(|b| {
            py.import("json")
                .ok()?
                .call_method1("loads", (PyBytes::new(py, &b),))
                .ok()
        })
        .and_then(|d| d.get_item("modelos").ok())
    else {
        return Ok(entries);
    };
    if let Ok(rows) = data.cast::<PyList>() {
        for row in rows {
            if row
                .get_item("id")
                .and_then(|x| x.extract::<String>())
                .is_ok_and(|id| valid(&id))
            {
                entries.append(row)?;
            }
        }
    }
    Ok(entries)
}
fn preview(id: &str) -> String {
    format!("miniaturas/{id}.png")
}
#[pyfunction]
fn thumbnail<'p>(py: Python<'p>, ident: &str) -> PyResult<Option<Obj<'p>>> {
    if !valid(ident) {
        return Ok(None);
    }
    optional(py, cached(py, &cache(py)?, &base(), &preview(ident), false))
}
#[pyfunction]
fn cached_thumbnail<'p>(py: Python<'p>, ident: &str) -> PyResult<Option<Obj<'p>>> {
    if !valid(ident) {
        return Ok(None);
    }
    let p = cache(py)?.join(preview(ident));
    optional(py, p.is_file().then_some(p))
}

type Ticket = (PathBuf, String);
static PENDING: LazyLock<Mutex<HashSet<Ticket>>> = LazyLock::new(Default::default);
static POOL: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
#[pyfunction]
fn thumbnails_pending() -> bool {
    !PENDING.lock().unwrap().is_empty()
}
#[pyfunction]
fn fetch_thumbnail(py: Python, root: PathBuf, base: &str, ident: String) {
    let _ = cached(py, &root, base, &preview(&ident), false);
    PENDING.lock().unwrap().remove(&(root, ident));
}
#[pyfunction]
fn prefetch_thumbnails(idents: &Obj) -> PyResult<()> {
    let py = idents.py();
    let root = cache(py)?;
    let base = base();
    let worker = wrap_pyfunction!(fetch_thumbnail, py)?;
    for id in idents.try_iter()? {
        let Ok(id) = id?.extract::<String>() else {
            continue;
        };
        if !valid(&id) || root.join(preview(&id)).is_file() {
            continue;
        }
        let ticket = (root.clone(), id.clone());
        if !PENDING.lock().unwrap().insert(ticket.clone()) {
            continue;
        }
        let submitted = (|| -> PyResult<()> {
            let pool = POOL.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
                let kw = PyDict::new(py);
                kw.set_item("thread_name_prefix", "library")?;
                Ok(py
                    .import("concurrent.futures")?
                    .call_method("ThreadPoolExecutor", (16,), Some(&kw))?
                    .unbind())
            })?;
            pool.bind(py)
                .call_method1("submit", (&worker, &root, &base, id))?;
            Ok(())
        })();
        if let Err(e) = submitted {
            PENDING.lock().unwrap().remove(&ticket);
            return Err(e);
        }
    }
    Ok(())
}
fn unpack(py: Python, archive: &Path, stage: &Path, id: &str) -> PyResult<()> {
    let z = py.import("zipfile")?.call_method1("ZipFile", (archive,))?;
    let load = (|| -> PyResult<()> {
        py.import("core.zip_limits")?
            .call_method1("check", (&z, id))?;
        for member in z.call_method0("infolist")?.try_iter()? {
            let member = member?;
            if member.call_method0("is_dir")?.is_truthy()? {
                continue;
            }
            let orig = member.getattr("filename")?.extract::<String>()?;
            let Some(name) = orig.rsplit(['/', '\\']).next() else {
                continue;
            };
            if name.is_empty() || matches!(name, "." | "..") || name.contains(':') {
                continue;
            }
            let bytes = z.call_method1("read", (member,))?.cast_into::<PyBytes>()?;
            fs::write(stage.join(name), bytes.as_bytes())?;
        }
        Ok(())
    })();
    z.call_method0("close")?;
    load
}
fn populated(dir: &Path) -> bool {
    fs::read_dir(dir)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_some()
}
fn folder(py: Python, id: &str) -> PyResult<Option<PathBuf>> {
    if !valid(id) {
        return Ok(None);
    }
    let root = cache(py)?;
    let dst = root.join("modelos").join(id);
    if populated(&dst) {
        return Ok(Some(dst));
    }
    let Some(zip) = cached(py, &root, &base(), &format!("modelos/{id}.zip"), false) else {
        return Ok(None);
    };
    let stage = temp(dst.parent().unwrap());
    if fs::create_dir(&stage).is_err() {
        return Ok(None);
    }
    match unpack(py, &zip, &stage, id) {
        Ok(()) => {
            let _ = fs::rename(&stage, &dst);
        }
        Err(err) => {
            if err.is_instance(py, &py.import("zipfile")?.getattr("BadZipFile")?) {
                // A future request can fetch a repaired archive from the mirror.
                let _ = fs::remove_file(&zip);
            }
        }
    }
    let _ = fs::remove_dir_all(&stage);
    Ok(populated(&dst).then_some(dst))
}
#[pyfunction]
fn model_dir<'p>(py: Python<'p>, ident: &str) -> PyResult<Option<Obj<'p>>> {
    optional(py, folder(py, ident)?)
}
#[pyfunction]
fn model_file<'p>(entry: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let py = entry.py();
    let Some(id) = entry
        .get_item("id")
        .ok()
        .and_then(|v| v.extract::<String>().ok())
    else {
        return Ok(None);
    };
    let Some(dir) = folder(py, &id)? else {
        return Ok(None);
    };
    if let Ok(name) = entry.get_item("obj").and_then(|v| v.extract::<String>()) {
        let local = Path::new(&name);
        if local
            .file_name()
            .is_some_and(|file| file == local.as_os_str())
            && !name.contains(['\\', ':'])
        {
            let full = dir.join(local);
            if full.is_file() {
                return optional(py, Some(full));
            }
        }
    }
    let found = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "obj"))
        .min();
    optional(py, found)
}
pub(crate) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "component_library")?;
    for f in [
        wrap_pyfunction!(library_cache, &sub)?,
        wrap_pyfunction!(index, &sub)?,
        wrap_pyfunction!(thumbnail, &sub)?,
        wrap_pyfunction!(cached_thumbnail, &sub)?,
        wrap_pyfunction!(prefetch_thumbnails, &sub)?,
        wrap_pyfunction!(thumbnails_pending, &sub)?,
        wrap_pyfunction!(model_dir, &sub)?,
        wrap_pyfunction!(model_file, &sub)?,
        wrap_pyfunction!(sizing::model_matrix, &sub)?,
    ] {
        sub.add_function(f)?
    }
    m.add_submodule(&sub)
}

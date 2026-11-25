// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve ZIP entries and their validation metadata. A nonstandard member
//! follows the archive object's ordinary read path, including its CRC errors.
use pyo3::{
    exceptions::{PyKeyError, PyTypeError, PyUnicodeEncodeError},
    prelude::*,
    types::{PyDict, PyList, PySet, PySlice, PyString},
};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("formats.igz")
}
fn get<'p>(d: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (k,))
}
fn row_count(d: &Obj<'_>, k: &str) -> PyResult<usize> {
    let rows = get(d, k)?;
    if rows.is_truthy()? {
        rows.len()
    } else {
        Ok(0)
    }
}
#[pyfunction]
fn igz_read_member_flat<'p>(archive: &Obj<'p>, name: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = archive.py();
    let info = archive.call_method1("getinfo", (name,))?;
    let len = info.getattr("file_size")?.extract::<usize>()?;
    if len < api(py)?.getattr("_NATIVE_JSON_MIN")?.extract()? {
        return archive.call_method1("read", (name,));
    }
    let builtins = py.import("builtins")?;
    let buf = builtins.call_method1("bytearray", (len,))?;
    let view = builtins.call_method1("memoryview", (&buf,))?;
    let file = archive.call_method1("open", (info,))?;
    let mut at = 0;
    let read = (|| -> PyResult<()> {
        while at < len {
            let part = view.get_item(PySlice::new(
                py,
                at as isize,
                len.min(at.saturating_add(1 << 24)) as isize,
                1,
            ))?;
            let n = file.call_method1("readinto", (part,))?;
            if !n.is_truthy()? {
                break;
            }
            at += n.extract::<usize>()?;
        }
        Ok(())
    })();
    file.call_method0("close")?;
    view.call_method0("release")?;
    read?;
    if at < len {
        buf.del_item(PySlice::new(py, at as isize, isize::MAX, 1))?
    }
    Ok(buf)
}
#[pyfunction]
#[pyo3(signature=(archive,path,payload,skip=None,*,built_format=false))]
fn igz_read_geometry_members<'p>(
    archive: &Obj<'p>,
    path: &Obj<'p>,
    payload: &Obj<'p>,
    skip: Option<&Obj<'p>>,
    built_format: bool,
) -> PyResult<Obj<'p>> {
    let py = archive.py();
    let codec = api(py)?;
    let prefix = codec.getattr("_GEOM_PREFIX")?.extract::<String>()?;
    let names = PyList::empty(py);
    for n in archive.call_method0("namelist")?.try_iter()? {
        let n = n?;
        if n.extract::<String>()?.starts_with(&prefix)
            && !skip.map(|s| s.contains(&n)).transpose()?.unwrap_or(false)
        {
            names.append(n)?
        }
    }
    let out = py.import("ingetrazo_core")?.call_method0("GeometryMembers")?;
    out.setattr("built_format", built_format)?;
    if !names.is_empty() {
        let module = py.import("core._native")?.getattr("native")?;
        if let Ok(reader) = module.getattr("igz_read_geometry") {
            if !reader.is_none() {
                if let Err(err) = codec.call_method1(
                    "_read_geometry_native",
                    (reader, archive, path, payload, &names, &out),
                ) {
                    if !err.is_instance_of::<pyo3::exceptions::PyException>(py) {
                        return Err(err);
                    }
                    py.import("logging")?
                        .call_method1("getLogger", ("formats.igz",))?
                        .call_method1(
                            "warning",
                            (
                                "Native geometry reader failed, using Python: %s",
                                err.value(py),
                            ),
                        )?;
                    out.call_method0("clear")?;
                    out.getattr("checked")?.call_method0("clear")?;
                }
            }
        }
    }
    let missing = PyList::empty(py);
    for n in names.iter() {
        if !out.contains(&n)? {
            missing.append(n)?
        }
    }
    out.call_method1(
        "update",
        (codec.call_method1("_read_members", (archive, missing))?,),
    )?;
    Ok(out)
}
#[pyfunction]
fn igz_read_geometry_native<'p>(
    native: &Obj<'p>,
    archive: &Obj<'p>,
    path: &Obj<'p>,
    payload: &Obj<'p>,
    names: &Obj<'p>,
    out: &Obj<'p>,
) -> PyResult<()> {
    let py = archive.py();
    let codec = api(py)?;
    let metas = PyDict::new(py);
    let shared = PySet::empty(py)?;
    for b in codec.call_method1("_mesh_blocks", (payload,))?.try_iter()? {
        let b = b?;
        if !b.is_instance_of::<PyDict>() {
            continue;
        }
        let m = get(&b, "geom")?;
        if !m.is_instance_of::<PyDict>() {
            continue;
        }
        let member = get(&m, "member")?;
        if !member.is_instance_of::<PyString>() {
            continue;
        }
        if metas.get_item(&member)?.is_some_and(|prior| !prior.is(&m)) {
            shared.add(&member)?
        }
        metas.set_item(member, m)?;
    }
    let allowed = codec.getattr("_NATIVE_ZIP_FLAGS")?.extract::<u32>()?;
    let mut todo = Vec::new();
    let entries = PyList::empty(py);
    let checks = PyList::empty(py);
    for n in names.try_iter()? {
        let n = n?;
        let info = archive.call_method1("getinfo", (&n,))?;
        let flags = info.getattr("flag_bits")?.extract::<u32>()?;
        let method = info.getattr("compress_type")?.extract::<u32>()?;
        if flags & !allowed != 0 || !matches!(method, 0 | 8) {
            continue;
        }
        let enc = if flags & 0x800 != 0 { "utf-8" } else { "cp437" };
        let local = match info
            .getattr("orig_filename")?
            .call_method1("encode", (enc,))
        {
            Ok(v) => v,
            Err(e) if e.is_instance_of::<PyUnicodeEncodeError>(py) => continue,
            Err(e) => return Err(e),
        };
        let m = if shared.contains(&n)? {
            None
        } else {
            metas.get_item(&n)?
        };
        let check = if let Some(meta) = &m {
            let r = (|| -> PyResult<_> {
                Ok((
                    meta.get_item("arrays")?,
                    row_count(meta, "face_attrs")?,
                    row_count(meta, "edge_layers")?,
                ))
            })();
            match r {
                Ok(v) => Some(v),
                Err(e)
                    if e.is_instance_of::<PyTypeError>(py)
                        || e.is_instance_of::<PyKeyError>(py) =>
                {
                    None
                }
                Err(e) => return Err(e),
            }
        } else {
            None
        };
        todo.push((n, check.is_some().then_some(m).flatten()));
        entries.append((
            info.getattr("header_offset")?,
            method,
            info.getattr("compress_size")?,
            info.getattr("file_size")?,
            info.getattr("CRC")?,
            local,
            flags,
        ))?;
        checks.append(check)?;
    }
    if todo.is_empty() {
        return Ok(());
    }
    let kw = PyDict::new(py);
    kw.set_item("built_format", out.getattr("built_format")?)?;
    if archive.is_instance(&codec.getattr("DocumentArchive")?)? {
        let offsets = PyList::empty(py);
        for (n, _) in &todo {
            let member = archive.call_method1("open", (n,))?;
            let offset = member.getattr("_orig_compress_start").ok();
            member.call_method0("close")?;
            match offset.filter(|v| !v.is_none()) {
                Some(o) => offsets.append(o)?,
                None => break,
            }
        }
        if offsets.len() == todo.len() {
            kw.set_item("fd", archive.getattr("fp")?.call_method0("fileno")?)?;
            kw.set_item("member_offsets", offsets)?
        }
    }
    let result = native.call((path.str()?, entries, checks), Some(&kw))?;
    for ((n, m), item) in todo.into_iter().zip(result.try_iter()?) {
        let item = item?;
        let blob = item.get_item(0)?;
        if blob.is_none() {
            continue;
        }
        out.set_item(&n, blob)?;
        if item.get_item(1)?.is_truthy()? {
            if let Some(m) = m {
                out.getattr("checked")?.set_item(n, m)?
            }
        }
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_read_member_flat, m)?)?;
    m.add_function(wrap_pyfunction!(igz_read_geometry_members, m)?)?;
    m.add_function(wrap_pyfunction!(igz_read_geometry_native, m)?)
}

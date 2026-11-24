// SPDX-License-Identifier: GPL-3.0-or-later
//! Dispatch a file to the first available reader with usable geometry.
use pyo3::{exceptions::PyException, prelude::*, types::PyDict};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("formats.skp")
}
fn detect(path: &Path) -> &'static str {
    let mut header = [0; 64];
    let Ok(n) = std::fs::File::open(path).and_then(|f| f.take(64).read(&mut header)) else {
        return "unknown";
    };
    let bytes = &header[..n];
    if bytes.starts_with(b"PK\x03\x04") || bytes.windows(11).any(|s| s == b"S\0k\0e\0t\0c\0h") {
        "skp"
    } else {
        "unknown"
    }
}
fn supports(reader: &Obj<'_>, format: &str) -> PyResult<bool> {
    Ok(reader.call_method0("available")?.is_truthy()?
        && reader.call_method1("supports", (format,))?.is_truthy()?)
}
#[pyfunction]
fn skp_detect_format(path: PathBuf) -> &'static str {
    detect(&path)
}
#[pyfunction]
fn skp_backends_status(py: Python<'_>) -> PyResult<Vec<(String, bool)>> {
    api(py)?
        .getattr("_BACKENDS")?
        .try_iter()?
        .map(|b| {
            let b = b?;
            Ok((
                b.getattr("name")?.extract()?,
                b.call_method0("available")?.is_truthy()?,
            ))
        })
        .collect()
}
#[pyfunction]
fn skp_can_handle(py: Python<'_>, path: PathBuf) -> PyResult<bool> {
    let fmt = detect(&path);
    for r in api(py)?.getattr("_BACKENDS")?.try_iter()? {
        if supports(&r?, fmt)? {
            return Ok(true);
        }
    }
    Ok(false)
}
#[pyfunction(signature=(path,progress=None))]
fn skp_dispatch_parse<'p>(py: Python<'p>, path: PathBuf, progress: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let module = api(py)?;
    let fmt = detect(&path);
    let path = py.import("pathlib")?.call_method1("Path", (path,))?;
    let options = PyDict::new(py);
    options.set_item("progress", progress)?;
    let mut cause = None;
    for r in module.getattr("_BACKENDS")?.try_iter()? {
        let r = r?;
        if !supports(&r, fmt)? {
            continue;
        }
        match r.call_method("parse", (&path,), Some(&options)) {
            Ok(p) if p.is_truthy()? => {
                for key in ["groups", "protos", "empty"] {
                    if p.call_method1("get", (key,))?.is_truthy()? {
                        p.set_item("_consume_geometry", true)?;
                        return Ok(p);
                    }
                }
            }
            Err(e) if e.is_instance_of::<PyException>(py) => {
                let kw = PyDict::new(py);
                kw.set_item("exc_info", (e.get_type(py), e.value(py), e.traceback(py)))?;
                py.import("logging")?
                    .call_method1("getLogger", ("formats.skp",))?
                    .call_method(
                        "warning",
                        (
                            "SKP backend %s could not read %s",
                            r.getattr("name")?,
                            &path,
                        ),
                        Some(&kw),
                    )?;
                cause = Some(e);
            }
            Err(e) => return Err(e),
            _ => (),
        }
    }
    let reason = match &cause {
        Some(e) => e.value(py).str()?.to_string(),
        None => String::new(),
    };
    let err = PyErr::from_value(module.call_method1("NeedsConverter", (path, fmt, reason))?);
    err.set_cause(py, cause);
    Err(err)
}
#[pyfunction(signature=(scene,path,progress=None))]
fn skp_load<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    path: PathBuf,
    progress: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let module = api(py)?;
    let kw = PyDict::new(py);
    kw.set_item("progress", progress)?;
    let payload = module.call_method("parse_skp", (path,), Some(&kw))?;
    module.call_method1("apply_payload", (scene, payload))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(skp_detect_format, m)?)?;
    m.add_function(wrap_pyfunction!(skp_backends_status, m)?)?;
    m.add_function(wrap_pyfunction!(skp_can_handle, m)?)?;
    m.add_function(wrap_pyfunction!(skp_dispatch_parse, m)?)?;
    m.add_function(wrap_pyfunction!(skp_load, m)?)?;
    m.add_function(wrap_pyfunction!(super::payload::skp_soften_bulk, m)?)
}

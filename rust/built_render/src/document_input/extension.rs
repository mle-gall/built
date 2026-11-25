// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn pick_extension_item<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    if let Some(entries) = v.getattr_opt("_ext_pickables")? {
        for entry in entries.try_iter()? {
            let entry = entry?;
            let picked = entry.get_item("pick").and_then(|f| f.call1((v, x, y)));
            match picked {
                Ok(item) if !item.is_none() => {
                    return Ok((entry, item).into_pyobject(py)?.into_any())
                }
                Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => {
                    let logger = py
                        .import("logging")?
                        .call_method1("getLogger", ("views.document_interaction",))?;
                    let kw = PyDict::new(py);
                    kw.set_item("exc_info", e.value(py))?;
                    logger.call_method("exception", ("pickable failed",), Some(&kw))?;
                }
                Err(e) => return Err(e),
                _ => (),
            }
        }
    }
    Ok(none(py))
}
#[pyfunction]
fn set_extension_pick(v: &Obj, hit: &Obj) -> PyResult<()> {
    v.call_method0("clear_extension_pick")?;
    let (entry, item): (Obj, Obj) = hit.extract()?;
    v.setattr(
        "extension_pick",
        (&entry, &item, v.getattr("scene")?.getattr("version")?),
    )?;
    let selected = entry.call_method1("get", ("on_select",))?;
    if !selected.is_none() {
        selected.call1((item,))?;
    }
    update(v)
}
#[pyfunction]
fn clear_extension_pick(v: &Obj, notify: bool) -> PyResult<()> {
    let pick = v.getattr("extension_pick")?;
    v.setattr("extension_pick", v.py().None())?;
    if notify && !pick.is_none() {
        let selected = pick.get_item(0)?.call_method1("get", ("on_select",))?;
        if !selected.is_none() {
            selected.call1((v.py().None(),))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn delete_extension_pick(v: &Obj) -> PyResult<bool> {
    let pick = v.getattr("extension_pick")?;
    if pick.is_none() {
        return Ok(false);
    }
    let (entry, item, version): (Obj, Obj, Obj) = pick.extract()?;
    v.call_method0("clear_extension_pick")?;
    if !version.eq(v.getattr("scene")?.getattr("version")?)? {
        return Ok(true);
    }
    let delete = entry.call_method1("get", ("delete",))?;
    if delete.is_none() {
        return Ok(false);
    }
    delete.call1((item,))?;
    update(v)?;
    Ok(true)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(pick_extension_item, m)?)?;
    m.add_function(wrap_pyfunction!(set_extension_pick, m)?)?;
    m.add_function(wrap_pyfunction!(clear_extension_pick, m)?)?;
    m.add_function(wrap_pyfunction!(delete_extension_pick, m)?)?;
    Ok(())
}

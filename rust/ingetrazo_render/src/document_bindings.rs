// SPDX-License-Identifier: GPL-3.0-or-later
//! Dispatch document extension hooks without leaking a callback's painter state.
use crate::document_host::{opt, Obj};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

fn paint_one(v: &Obj, p: &Obj, callback: &Obj) -> PyResult<()> {
    match callback.call1((v, p)) {
        Ok(_) => Ok(()),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(v.py()) => {
            let py = v.py();
            let logger = py
                .import("logging")?
                .call_method1("getLogger", ("ingetrazo.plugins",))?;
            let kw = PyDict::new(py);
            kw.set_item("exc_info", e.value(py))?;
            logger.call_method(
                "exception",
                ("extension overlay %r failed; removed", callback),
                Some(&kw),
            )?;
            match v
                .getattr("_ext_overlays")?
                .call_method1("remove", (callback,))
            {
                Err(e) if !e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => Err(e),
                _ => Ok(()),
            }
        }
        Err(e) => Err(e),
    }
}
#[pyfunction]
fn document_paint_extensions(v: &Obj, painter: &Obj) -> PyResult<()> {
    let Some(hooks) = v.getattr_opt("_ext_overlays")? else {
        return Ok(());
    };
    // A callback may remove itself or append new hooks during painting.
    let snapshot = v.py().get_type::<PyList>().call1((hooks,))?;
    for callback in snapshot.try_iter()? {
        let callback = callback?;
        painter.call_method0("save")?;
        let result = paint_one(v, painter, &callback);
        painter.call_method0("restore")?;
        result?;
    }
    Ok(())
}

#[pyfunction]
fn document_edge_result<'py>(
    v: &Obj<'py>,
    x: f64,
    y: f64,
    visible: bool,
    which: &str,
    hidden: &Obj<'py>,
    with_distance: bool,
) -> PyResult<Obj<'py>> {
    let result = v.call_method1("_pick_edge_d", (x, y, visible, which, hidden))?;
    if with_distance {
        return if result.is_none() {
            Ok((v.py().None(), f64::INFINITY)
                .into_pyobject(v.py())?
                .into_any())
        } else {
            Ok(result)
        };
    }
    if result.is_none() {
        Ok(result)
    } else {
        result.get_item(0)
    }
}

#[pyfunction]
fn document_scene_changed(v: &Obj) -> PyResult<()> {
    v.call_method0("prune_stale_selection")?;
    let ver = v.getattr("scene")?.getattr("version")?;
    let watch = v
        .getattr("__dict__")?
        .call_method1("get", ("_document_watch",))?;
    if watch.is_none() {
        v.getattr("sceneVersionChanged")?
            .call_method1("emit", (ver,))?;
    } else {
        watch.call_method0("observe")?;
    }
    v.call_method0("update")?;
    Ok(())
}
#[pyfunction]
fn document_status(v: &Obj, text: &str, msec: i64) -> PyResult<()> {
    let win = v.call_method0("window")?;
    if !win.is_none() {
        let bar = win.call_method0("statusBar")?;
        if !bar.is_none() {
            bar.call_method1("showMessage", (text, msec))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn document_tool_pointer<'py>(v: &Obj<'py>, key: &Obj<'py>, plus: bool) -> PyResult<Obj<'py>> {
    let factory = opt(&opt(v, "_host")?, "tool_cursor")?;
    if !factory.is_none() {
        return factory.call1((key, plus));
    }
    // No factory of the host's own: the pointer by its name, in the
    // preferred style.
    let py = v.py();
    let style = py.import("ingetrazo_core.general_settings")?.call_method1("value", ("cursor_style", "tool"))?.str()?;
    let key: Option<String> = key.extract()?;
    let name = crate::tool_cursors::tool_cursor_name(key.as_deref(), plus, style.to_str()?);
    Ok(pyo3::IntoPyObjectExt::into_bound_py_any(name, py)?)
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(document_paint_extensions, m)?)?;
    m.add_function(wrap_pyfunction!(document_edge_result, m)?)?;
    m.add_function(wrap_pyfunction!(document_scene_changed, m)?)?;
    m.add_function(wrap_pyfunction!(document_status, m)?)?;
    m.add_function(wrap_pyfunction!(document_tool_pointer, m)?)?;
    Ok(())
}

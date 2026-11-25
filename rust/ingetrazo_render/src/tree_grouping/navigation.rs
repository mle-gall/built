//! Browser actions use the same edit path and undo history as viewport gestures.
use pyo3::{exceptions::PyValueError, prelude::*};

#[pyfunction]
pub(crate) fn open_to(viewport: &Bound<PyAny>, path: Vec<Bound<PyAny>>) -> PyResult<bool> {
    for group in &path {
        if group.getattr("hidden")?.is_truthy()?
            || group
                .getattr_opt("locked")?
                .map(|v| v.is_truthy())
                .transpose()?
                .unwrap_or(false)
        {
            return Ok(false);
        }
    }
    let scene = viewport.getattr("scene")?;
    let outliner = viewport.py().import("core.outliner")?;
    let current: Vec<Bound<PyAny>> = outliner.call_method1("open_path", (&scene,))?.extract()?;
    let same = current
        .iter()
        .zip(&path)
        .take_while(|(a, b)| a.is(*b))
        .count();
    for _ in same..current.len() {
        viewport.call_method0("end_one_group_edit")?;
    }
    for group in &path[same..] {
        viewport.call_method1("begin_group_edit", (group,))?;
        if !scene.getattr("edit_group")?.is(group) {
            return Ok(false);
        }
    }
    outliner.call_method1("open_path", (scene,))?.eq(path)
}
#[pyfunction]
pub(crate) fn execute(viewport: &Bound<PyAny>, command: &Bound<PyAny>) -> PyResult<()> {
    let compound = viewport
        .py()
        .import("core.history")?
        .getattr("CompoundCommand")?;
    if command.is_instance(&compound)? && !command.getattr("commands")?.is_truthy()? {
        return Ok(());
    }
    let history = viewport.getattr("history")?;
    history.call_method1("execute", (command,))?;
    for attr in ["last_error", "last_rejected"] {
        let error = history.getattr(attr)?;
        if error.is_truthy()? {
            return Err(PyValueError::new_err(error.str()?.to_string()));
        }
    }
    viewport.call_method0("refresh_after_edit")?;
    Ok(())
}

pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let sub = PyModule::new(py, "ingetrazo_render.tree_navigation")?;
    sub.add_function(wrap_pyfunction!(open_to, &sub)?)?;
    sub.add_function(wrap_pyfunction!(execute, &sub)?)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item(sub.name()?, &sub)?;
    parent.add("tree_navigation", sub)
}

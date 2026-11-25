//! Hide and unhide use the same history entries as the document.
use super::model_edit::{matching, translated};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

fn add_hidden<'py>(
    out: &Bound<'py, PyList>,
    items: &Bound<'py, PyAny>,
    predicate: &Bound<'py, PyAny>,
) -> PyResult<()> {
    for item in items.try_iter()? {
        let item = item?;
        if predicate.call1((&item,))?.is_truthy()? {
            out.append(item)?;
        }
    }
    Ok(())
}
#[pyfunction]
fn hidden_everywhere<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let py = scene.py();
    let out = PyList::empty(py);
    let hidden = py.import("ingetrazo_core")?.getattr("is_hidden")?;
    let groups = py
        .import("core.purge")?
        .call_method1("iter_groups", (scene.getattr("groups")?,))?;
    add_hidden(&out, &groups, &hidden)?;
    let mesh = scene.getattr("mesh")?;
    for key in ["faces", "edges"] {
        add_hidden(&out, &mesh.getattr(key)?, &hidden)?;
    }
    Ok(out)
}
fn last<'py>(
    history: &Bound<'py, PyAny>,
    hide: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let py = history.py();
    let hidden = py.import("ingetrazo_core")?.getattr("is_hidden")?;
    let stack = py
        .import("builtins")?
        .call_method1("reversed", (history.getattr("undo_stack")?,))?;
    let out = PyList::empty(py);
    for cmd in stack.try_iter()? {
        let cmd = cmd?;
        if cmd.is_instance(hide)? && cmd.getattr("hides")?.is_truthy()? {
            for e in cmd.getattr("entities")?.try_iter()? {
                let e = e?;
                if hidden.call1((&e,))?.is_truthy()? {
                    out.append(e)?;
                }
            }
            if !out.is_empty() {
                break;
            }
        }
    }
    Ok(out)
}
#[pyfunction]
fn change_visibility(win: &Bound<'_, PyAny>, verb: &str) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let history = vp.getattr("history")?;
    let lib = py.import("core.history")?;
    let cmd = lib.getattr("HideCommand")?;
    let hide = verb == "hide";
    let (targets, empty, ms) = match verb {
        "hide" => (
            win.call_method1("_hideable", (scene.getattr("selection")?,))?,
            "Select an object, faces or edges first.",
            3000,
        ),
        "selected" => {
            let selection = scene.getattr("selection")?;
            let hidden = py.import("ingetrazo_core")?.getattr("is_hidden")?;
            let out = PyList::empty(py);
            for (module, name) in [
                ("core.mesh", "Edge"),
                ("core.mesh", "Face"),
                ("core.group", "Group"),
            ] {
                add_hidden(&out, matching(&selection, module, name)?.as_any(), &hidden)?;
            }
            (out.into_any(),"Nothing hidden in the selection — turn on Camera ▸ Hidden Objects / Geometry to select hidden things.",4000)
        }
        "last" => (last(&history, &cmd)?.into_any(), "Nothing to unhide.", 3000),
        "all" => (
            win.call_method0("_hidden_everywhere")?,
            "Nothing is hidden.",
            3000,
        ),
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "unknown visibility command",
            ))
        }
    };
    let bar = win.call_method0("statusBar")?;
    if targets.len()? == 0 {
        bar.call_method1("showMessage", (translated(py, empty, None)?, ms))?;
        return Ok(());
    }
    let kw = PyDict::new(py);
    kw.set_item("hidden", hide)?;
    history.call_method1("execute", (cmd.call((&targets,), Some(&kw))?,))?;
    vp.call_method0("update")?;
    let vars = PyDict::new(py);
    let (msg, ms) = if hide {
        let objs = matching(&targets, "core.group", "Group")?.len();
        let faces = matching(&targets, "core.mesh", "Face")?.len();
        vars.set_item("objects", objs)?;
        vars.set_item("faces", faces)?;
        vars.set_item("edges", targets.len()? - objs - faces)?;
        ("Hid {objects} object(s), {faces} face(s) and {edges} edge(s) — Edit ▸ Unhide brings them back",4000)
    } else {
        vars.set_item("n", targets.len()?)?;
        ("Unhid {n} entities.", 3000)
    };
    bar.call_method1("showMessage", (translated(py, msg, Some(&vars))?, ms))?;
    Ok(())
}
#[pyfunction]
fn toggle_edit_visibility(win: &Bound<'_, PyAny>, similar: bool) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    if similar {
        let on = vp
            .getattr("scene")?
            .getattr("hide_similar_components")
            .is_ok_and(|v| v.is_truthy().unwrap_or(false));
        vp.call_method1("set_hide_similar_components", (!on,))?;
    } else {
        let mode = vp.getattr("edit_rest_mode")?;
        if mode.eq("hide")? {
            vp.call_method1("set_edit_rest_mode", (win.getattr("_rest_shown_mode")?,))?;
        } else {
            win.setattr("_rest_shown_mode", mode)?;
            vp.call_method1("set_edit_rest_mode", ("hide",))?;
        }
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hidden_everywhere, m)?)?;
    m.add_function(wrap_pyfunction!(change_visibility, m)?)?;
    m.add_function(wrap_pyfunction!(toggle_edit_visibility, m)?)?;
    Ok(())
}

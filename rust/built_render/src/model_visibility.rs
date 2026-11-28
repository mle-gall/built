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
    let hidden = py.import("built_core")?.getattr("is_hidden")?;
    let groups = py
        .import("built_core")?
        .call_method1("purge_iter_groups", (scene.getattr("groups")?,))?;
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
    let hidden = py.import("built_core")?.getattr("is_hidden")?;
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
    let lib = py.import("built_core.history")?;
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
            let hidden = py.import("built_core")?.getattr("is_hidden")?;
            let out = PyList::empty(py);
            for (module, name) in [
                ("built_core.mesh", "Edge"),
                ("built_core.mesh", "Face"),
                ("built_core.group", "Group"),
            ] {
                add_hidden(&out, matching(&selection, module, name)?.as_any(), &hidden)?;
            }
            (out.into_any(),"Nothing hidden in the selection — turn on View ▸ Hidden Geometry to select hidden things.",4000)
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
    if hide {
        let groups = matching(&targets, "built_core.group", "Group")?;
        let faces = matching(&targets, "built_core.mesh", "Face")?.len();
        let edges = targets.len()? - groups.len() - faces;
        let mut parts = Vec::new();
        for (n, singular, plural) in [
            (groups.len(), "1 object", "{n} objects"),
            (faces, "1 face", "{n} faces"),
            (edges, "1 edge", "{n} edges"),
        ] {
            if n > 0 {
                vars.set_item("n", n)?;
                parts.push(
                    translated(py, if n == 1 { singular } else { plural }, Some(&vars))?
                        .extract::<String>()?,
                );
            }
        }
        if groups.len() == 1 {
            if let Some(name) = groups
                .get_item(0)?
                .getattr_opt("name")?
                .and_then(|s| s.extract::<String>().ok())
                .filter(|s| !s.is_empty())
            {
                parts[0].push_str(&format!(" «{name}»"));
            }
        }
        vars.set_item("items", parts.join(", "))?;
        let text = translated(py, "Hid {items}", Some(&vars))?;
        let (history, cmd, targets, vp) = (
            history.unbind(),
            cmd.unbind(),
            targets.unbind(),
            vp.unbind(),
        );
        let show = crate::settings_rows::func(py, move |py, _| {
            let hidden = PyList::empty(py);
            add_hidden(
                &hidden,
                targets.bind(py),
                &py.import("built_core")?.getattr("is_hidden")?,
            )?;
            if !hidden.is_empty() {
                history
                    .bind(py)
                    .call_method1("execute", (cmd.bind(py).call1((hidden, false))?,))?;
                vp.bind(py).call_method0("update")?;
            }
            Ok(py.None())
        })?;
        win.call_method1(
            "notify",
            ("info", text, [(translated(py, "Show", None)?, show)]),
        )?;
    } else {
        vars.set_item("n", targets.len()?)?;
        bar.call_method1(
            "showMessage",
            (translated(py, "Unhid {n} entities.", Some(&vars))?, 3000),
        )?;
    }
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

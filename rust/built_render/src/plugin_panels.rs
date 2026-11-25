// SPDX-License-Identifier: GPL-3.0-or-later
//! Connect document-scoped extension models to the egui panel protocol.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
mod binding;
use binding::Binding;
type Obj<'p> = Bound<'p, PyAny>;

fn scope(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("uuid")?.call_method0("uuid4")?.getattr("hex")
}
fn scene<'p>(win: &Obj<'p>) -> PyResult<Obj<'p>> {
    win.getattr("viewport")?.getattr("scene")
}
fn flagged(obj: &Obj, key: &str) -> PyResult<bool> {
    Ok(obj
        .getattr_opt(key)?
        .map(|v| v.is_truthy())
        .transpose()?
        .unwrap_or(false))
}
fn isolated(py: Python, err: &PyErr) -> bool {
    err.is_instance_of::<pyo3::exceptions::PyException>(py)
        || err.is_instance_of::<pyo3::exceptions::PySystemExit>(py)
}
fn empty_label() -> Py<PyAny> {
    Python::attach(|py| pyo3::types::PyString::intern(py, "").into_any().unbind())
}
#[pyfunction(signature=(kind,id,label=empty_label(),**data))]
fn item<'p>(
    py: Python<'p>,
    kind: &Obj<'p>,
    id: &Obj<'p>,
    label: Py<PyAny>,
    data: Option<&Bound<'p, PyDict>>,
) -> PyResult<Bound<'p, PyDict>> {
    let row = PyDict::new(py);
    row.set_item("kind", kind)?;
    row.set_item("id", id)?;
    row.set_item("label", label)?;
    if let Some(d) = data {
        row.update(d.as_mapping())?;
    }
    Ok(row)
}
#[pyfunction(signature=(id,label,r#type,value,**data))]
fn field<'p>(
    py: Python<'p>,
    id: &Obj<'p>,
    label: Py<PyAny>,
    r#type: &Obj<'p>,
    value: &Obj<'p>,
    data: Option<&Bound<'p, PyDict>>,
) -> PyResult<Bound<'p, PyDict>> {
    if data
        .map(|d| d.contains("kind"))
        .transpose()?
        .unwrap_or(false)
    {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "multiple values for kind",
        ));
    }
    let row = item(py, &"field".into_pyobject(py)?.into_any(), id, label, None)?;
    row.set_item("type", r#type)?;
    row.set_item("value", value)?;
    if let Some(d) = data {
        row.update(d.as_mapping())?;
    }
    Ok(row)
}
#[pyfunction(signature=(id,title,*items))]
fn section<'p>(
    py: Python<'p>,
    id: &Obj<'p>,
    title: &Obj<'p>,
    items: &Bound<'p, PyTuple>,
) -> PyResult<Bound<'p, PyDict>> {
    let row = PyDict::new(py);
    row.set_item("id", id)?;
    row.set_item("title", title)?;
    row.set_item("items", PyList::new(py, items.iter())?)?;
    Ok(row)
}
#[pyfunction]
pub(crate) fn install(win: &Obj, plugins: &Obj) -> PyResult<()> {
    let factories = PyDict::new(win.py());
    for plugin in plugins.try_iter()? {
        let p = plugin?;
        let panel = p.getattr("panel")?;
        if panel.is_truthy()? {
            factories.set_item(p.getattr("stem")?, panel)?
        }
    }
    win.setattr("_panel_factories", factories)
}

#[pyfunction(signature=(scene,win,close,hook,closed_scene=None))]
fn finished(
    scene: &Obj,
    win: &Obj,
    close: &Obj,
    hook: &Bound<PyList>,
    closed_scene: Option<&Obj>,
) -> PyResult<()> {
    if closed_scene.is_some_and(|s| !s.is(scene)) {
        return Ok(());
    }
    let cb = hook.get_item(0)?;
    win.getattr("_doc_close_hooks")?
        .call_method1("remove", (&cb,))?;
    win.getattr("_extension_closed")?
        .call_method1("disconnect", (&cb,))?;
    hook.call_method0("clear")?;
    close.call0()?;
    Ok(())
}
#[pyfunction]
fn on_document_close(app: &Obj, close: &Obj) -> PyResult<()> {
    let py = app.py();
    let win = app.getattr("window")?;
    let holder = PyList::empty(py);
    let cb = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(finished, py)?,
            app.getattr("scene")?,
            &win,
            close,
            &holder,
        ),
    )?;
    holder.append(&cb)?;
    app.call_method1("on_document_closed", (&cb,))?;
    win.getattr("_extension_closed")?
        .call_method1("connect", (&cb,))?;
    Ok(())
}
fn bindings<'p>(view: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    match view.getattr_opt("_plugin_bindings")? {
        Some(v) => Ok(v.cast_into()?),
        None => Ok(PyDict::new(view.py())),
    }
}
fn lookup<'p>(view: &Obj<'p>, key: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    for (_, b) in bindings(view)? {
        if b.getattr("key")?.eq(key)? {
            return Ok(Some(b));
        }
    }
    Ok(None)
}
#[pyfunction]
pub(crate) fn open_panel(win: &Obj, key: &Obj) -> PyResult<bool> {
    let view = crate::dialogs_host::view_of(win.py(), win);
    if !flagged(&view, "_ui_attached")? || !view.getattr("renderer")?.hasattr("ui_plugin_open")? {
        return Ok(false);
    }
    sync(&view, win)?;
    let Some(b) = lookup(&view, key)? else {
        return Ok(false);
    };
    if let Some(opened) = b
        .getattr("model")?
        .getattr_opt("on_open")?
        .filter(|v| !v.is_none())
    {
        opened.call0()?;
        b.call_method0("invalidate")?;
    }
    view.getattr("renderer")?
        .call_method1("ui_plugin_open", (b.getattr("instance")?,))?;
    b.call_method0("push")?;
    view.call_method0("update")?;
    Ok(true)
}
#[pyfunction]
fn show_utility<'p>(win: &Obj<'p>, key: &Obj<'p>, model: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let view = crate::dialogs_host::view_of(win.py(), win);
    if !flagged(&view, "_ui_attached")? {
        return Ok(None);
    }
    sync(&view, win)?;
    let Some(rows) = view
        .getattr_opt("_plugin_bindings")?
        .filter(|v| !v.is_none())
    else {
        return Ok(None);
    };
    if !view.getattr("_plugin_scene")?.is(&scene(win)?) {
        return Ok(None);
    }
    let b = if let Some(b) = lookup(&view, key)? {
        b.setattr("model", model)?;
        b
    } else {
        let b = Binding::create(win, key, binding::Source::Utility(model), &view)?.into_any();
        rows.set_item(b.getattr("instance")?, &b)?;
        b
    };
    b.call_method0("invalidate")?;
    b.call_method0("push")?;
    view.getattr("renderer")?
        .call_method1("ui_plugin_open", (b.getattr("instance")?,))?;
    Ok(Some(b))
}

#[pyfunction]
pub(crate) fn sync(view: &Obj, win: &Obj) -> PyResult<()> {
    let py = view.py();
    if !win.getattr("__dict__")?.contains("_panel_factories")?
        || !view.is(&crate::dialogs_host::view_of(py, win))
    {
        return Ok(());
    }
    let renderer = view.getattr("renderer")?;
    if !renderer.hasattr("ui_plugin_panel")? {
        return Ok(());
    }
    let scene = scene(win)?;
    if !view
        .getattr_opt("_plugin_scene")?
        .is_some_and(|s| s.is(&scene))
    {
        view.setattr("_plugin_scene", scene)?;
        view.setattr("_plugin_document", scope(py)?)?;
        view.setattr("_plugin_bindings", PyDict::new(py))?;
        view.setattr("_plugin_factories_seen", PySet::empty(py)?)?;
        view.setattr("_ui_plugins_sent", false)?;
    }
    let rows = bindings(view)?;
    let seen = view
        .getattr("_plugin_factories_seen")?
        .cast_into::<PySet>()?;
    for (key, factory) in win.getattr("_panel_factories")?.cast::<PyDict>()? {
        if seen.contains(&key)? {
            continue;
        }
        seen.add(&key)?;
        match Binding::create(win, &key, binding::Source::Factory(&factory), view) {
            Ok(b) => rows.set_item(b.getattr("instance")?, b)?,
            Err(e) if isolated(py, &e) => {
                let kw = PyDict::new(py);
                kw.set_item("exc_info", e.value(py))?;
                py.import("logging")?
                    .call_method1("getLogger", ("core.plugin_panels",))?
                    .call_method("error", ("Plugin panel %s failed to load", key), Some(&kw))?;
            }
            Err(e) => return Err(e),
        }
    }
    if !flagged(view, "_ui_plugins_sent")? {
        let live = PyList::empty(py);
        for doc in win.getattr("_documents")?.try_iter()? {
            if let Some(v) = doc?.getattr_opt("stand_in")? {
                if let Some(id) = v.getattr_opt("_plugin_document")? {
                    if id.is_truthy()? {
                        live.append(id)?;
                    }
                }
            }
        }
        renderer.call_method1(
            "ui_plugin_document",
            (view.getattr("_plugin_document")?, live),
        )?;
        for (_, b) in &rows {
            b.setattr("dirty", true)?;
            b.setattr("sent_revision", py.None())?;
            b.setattr("last_event", 0)?;
        }
        view.setattr("_ui_plugins_sent", true)?;
    }
    for (_, b) in &rows {
        b.call_method0("push")?;
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn dispatch(win: &Obj, raw: &Obj) -> PyResult<()> {
    let py = win.py();
    let view = crate::dialogs_host::view_of(py, win);
    let event = py
        .import("json")?
        .call_method1("loads", (raw,))?
        .cast_into::<PyDict>()?;
    let get = |k: &str| event.as_any().get_item(k);
    let Some(instance) = event.get_item("instance")? else {
        return Ok(());
    };
    let Some(b) = bindings(&view)?.get_item(instance)? else {
        return Ok(());
    };
    let scene_now = scene(win)?;
    if !b.getattr("scene")?.is(&scene_now) {
        return Ok(());
    }
    for (name, expected) in [
        ("revision", scene_now.getattr("version")?),
        ("api", 1_i32.into_pyobject(py)?.into_any()),
        ("document", b.getattr("document")?),
        ("plugin", b.getattr("key")?),
        ("panel", b.getattr("id")?),
    ] {
        let Some(v) = event.get_item(name)? else {
            return Ok(());
        };
        if !v.eq(expected)? {
            return Ok(());
        }
    }
    let event_id = get("event_id")?;
    if event_id.le(b.getattr("last_event")?)? {
        return Ok(());
    }
    b.setattr("last_event", event_id)?;
    let model = b.getattr("model")?;
    let control = get("control")?;
    let verb = get("event")?;
    if control.eq("_panel")? && verb.eq("closed")? && !flagged(&model, "notify_close")? {
        return Ok(());
    }
    if control.eq("_retry")? {
        b.call_method0("invalidate")?;
        return Ok(());
    }
    let deliver = || -> PyResult<bool> {
        if verb.eq("browse")? {
            let answer = py
                .import("built_render.shell_files")?
                .getattr("ShellFileDialog")?
                .call_method1("getOpenFileName", (win, "Choose file"))?;
            let path = answer.get_item(0)?;
            if !path.is_truthy()?
                || !scene(win)?.is(&scene_now)
                || !get("revision")?.eq(scene_now.getattr("version")?)?
            {
                return Ok(false);
            }
            model.call_method1("event", (control, "commit", path))?;
        } else {
            model.call_method1("event", (control, verb, get("value")?))?;
        }
        Ok(true)
    };
    match deliver() {
        Ok(false) => return Ok(()),
        Ok(true) => b.setattr("error", "")?,
        Err(e) if isolated(py, &e) => b.setattr("error", e.value(py).str()?)?,
        Err(e) => return Err(e),
    }
    b.call_method0("invalidate")?;
    Ok(())
}
#[pyfunction]
fn overlay_primitives<'p>(view: &Obj<'p>, viewport: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let result = PyList::empty(view.py());
    for (_, b) in bindings(view)? {
        let Some(overlay) = b
            .getattr("model")?
            .getattr_opt("overlay")?
            .filter(|v| !v.is_none())
        else {
            continue;
        };
        // Materialize each extension independently: a failed iterator publishes no partial overlay.
        let collect = || {
            overlay
                .call1((viewport,))?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()
        };
        match collect() {
            Ok(items) => {
                for row in items {
                    result.append(row)?;
                }
            }
            Err(e) if isolated(view.py(), &e) => {
                let msg = e.value(view.py()).str()?;
                if !b.getattr("error")?.eq(&msg)? {
                    b.setattr("error", msg)?;
                    b.call_method0("invalidate")?;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(result)
}
#[pyfunction]
pub(crate) fn draw_tool_feedback(painter: &Obj, tools: &Obj, layout: &Obj) -> PyResult<()> {
    use crate::document_registration::view::calls;
    calls::draw_builtin_feedback(painter, tools, layout)?;
    if painter.hasattr("draw_primitives")? {
        let data = overlay_primitives(&tools.getattr("view")?, tools)?;
        if !data.is_empty() {
            painter.call_method0("save")?;
            let drawn = calls::paint_primitives(painter, &data);
            painter.call_method0("restore")?;
            drawn?;
        }
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn install_plugin_panels(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<Binding>()?;
    m.add(
        "ExtensionApp",
        m.py()
            .import("views.extension_api")?
            .getattr("ExtensionApp")?,
    )?;
    for f in [
        wrap_pyfunction!(item, m)?,
        wrap_pyfunction!(field, m)?,
        wrap_pyfunction!(section, m)?,
        wrap_pyfunction!(install, m)?,
        wrap_pyfunction!(open_panel, m)?,
        wrap_pyfunction!(show_utility, m)?,
        wrap_pyfunction!(on_document_close, m)?,
        wrap_pyfunction!(sync, m)?,
        wrap_pyfunction!(dispatch, m)?,
        wrap_pyfunction!(overlay_primitives, m)?,
        wrap_pyfunction!(draw_tool_feedback, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}

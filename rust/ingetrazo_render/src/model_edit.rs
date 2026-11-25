//! Editing commands shared by the native window and the Classic adapter.
//! The document still supplies its Python entities and undo history.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

pub(super) fn busy<T>(
    win: &Bound<'_, PyAny>,
    label: &Bound<'_, PyAny>,
    op: impl FnOnce() -> PyResult<T>,
) -> PyResult<T> {
    let py = win.py();
    let chip = py
        .import("ingetrazo_render.chrome_dialogs")?
        .call_method1("busy", (win, label))?;
    chip.call_method0("__enter__")?;
    let result = op();
    match &result {
        Ok(_) => chip.call_method1("__exit__", (py.None(), py.None(), py.None()))?,
        Err(e) => chip.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?,
    };
    result
}

pub(super) fn translated<'py>(
    py: Python<'py>,
    text: &str,
    values: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    py.import("core.i18n")?.getattr("tr")?.call((text,), values)
}

pub(super) fn matching<'py>(
    items: &Bound<'py, PyAny>,
    module: &str,
    class: &str,
) -> PyResult<Bound<'py, PyList>> {
    let py = items.py();
    let ty = py.import(module)?.getattr(class)?;
    let out = PyList::empty(py);
    for item in items.try_iter()? {
        let item = item?;
        if item.is_instance(&ty)? {
            out.append(item)?;
        }
    }
    Ok(out)
}

#[pyfunction]
fn edit_selection(win: &Bound<'_, PyAny>, verb: &str) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let selection = scene.getattr("selection")?;
    let (message, count) = match verb {
        "all" => {
            selection.call_method0("clear")?;
            for field in ["edges", "faces"] {
                selection.call_method1("update", (scene.getattr(field)?,))?;
            }
            let context = scene.getattr("edit_group")?;
            if context.is_none() {
                selection.call_method1("update", (scene.getattr("groups")?,))?;
                if let Ok(guides) = scene.getattr("guides") {
                    for guide in guides.try_iter()? {
                        let guide = guide?;
                        if scene
                            .call_method1("entity_selectable", (&guide,))?
                            .is_truthy()?
                        {
                            selection.call_method1("add", (guide,))?;
                        }
                    }
                }
            } else if let Ok(children) = context.getattr("children") {
                if !children.is_none() {
                    selection.call_method1("update", (children,))?;
                }
            }
            if let Ok(dims) = scene.getattr("dimensions") {
                selection.call_method1("update", (dims,))?;
            }
            scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
            ("Selected everything ({n} entities)", selection.len()?)
        }
        "none" => {
            let n = selection.len()?;
            scene.call_method0("clear_selection")?;
            ("Selection cleared ({n} entities)", n)
        }
        "invert" => (
            "Selection inverted ({n} entities)",
            scene.call_method0("invert_selection")?.extract()?,
        ),
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "unknown selection command",
            ))
        }
    };
    vp.call_method0("notify_scene_changed")?;
    let kw = PyDict::new(py);
    kw.set_item("n", count)?;
    win.call_method0("statusBar")?
        .call_method1("showMessage", (translated(py, message, Some(&kw))?, 2500))?;
    Ok(())
}

#[pyfunction]
fn delete_entities(vp: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = vp.py();
    let sel = vp.getattr("scene")?.getattr("selection")?;
    if sel.len()? == 0 {
        return Ok(());
    }
    let commands = PyList::empty(py);
    let history = py.import("ingetrazo_core.history")?;
    let edges = matching(&sel, "ingetrazo_core.mesh", "Edge")?;
    let faces = matching(&sel, "ingetrazo_core.mesh", "Face")?;
    if !edges.is_empty() || !faces.is_empty() {
        commands.append(history.call_method1("EraseSelectionCommand", (edges, faces))?)?;
    }
    for g in matching(&sel, "core.group", "Group")?.iter() {
        commands.append(history.call_method1("DeleteGroupCommand", (g,))?)?;
    }
    let guides = PyList::empty(py);
    let is_guide = py.import("core.guide")?.getattr("is_guide")?;
    for item in sel.try_iter()? {
        let item = item?;
        if is_guide.call1((&item,))?.is_truthy()? {
            guides.append(item)?;
        }
    }
    if !guides.is_empty() {
        commands.append(history.call_method1("DeleteGuidesCommand", (guides,))?)?;
    }
    for (module, class, command) in [
        ("ingetrazo_core.image_plane", "ImagePlane", "DeleteImagePlanesCommand"),
        ("ingetrazo_core.section", "SectionPlane", "DeleteSectionPlanesCommand"),
        ("core.dimension", "Dimension", "DeleteDimensionsCommand"),
        ("core.textlabel", "TextLabel", "DeleteTextLabelsCommand"),
        ("georef.geopath", "GeoPath", "DeleteGeoPathsCommand"),
    ] {
        let chosen = matching(&sel, module, class)?;
        if !chosen.is_empty() {
            commands.append(history.call_method1(command, (chosen,))?)?;
        }
    }
    if !commands.is_empty() {
        let cmd = if commands.len() == 1 {
            commands.get_item(0)?
        } else {
            history.call_method1("CompoundCommand", (commands,))?
        };
        vp.getattr("history")?.call_method1("execute", (cmd,))?;
        vp.call_method0("update")?;
    }
    Ok(())
}

#[pyfunction]
pub(super) fn paste_model(win: &Bound<'_, PyAny>, in_place: bool) -> PyResult<()> {
    let py = win.py();
    win.call_method0("_sync_foreign_clipboard")?;
    let vp = win.getattr("viewport")?;
    let tool = super::window_document::transient_tool_class(py, "paste")?;
    if in_place {
        if tool.call_method1("in_place", (&vp,))?.is_truthy()? {
            win.call_method0("statusBar")?.call_method1(
                "showMessage",
                (translated(py, "Pasted in place.", None)?, 3000),
            )?;
        }
    } else if !vp.getattr("clipboard")?.is_none() {
        vp.call_method1("set_active_tool", (tool.call0()?,))?;
        for a in win
            .getattr("_tool_actions")?
            .call_method0("values")?
            .try_iter()?
        {
            a?.call_method1("setChecked", (false,))?;
        }
        let args = PyDict::new(py);
        args.set_item("name", translated(py, "Paste", None)?)?;
        win.setattr("tool_label", translated(py, "Tool: {name}", Some(&args))?)?;
    }
    Ok(())
}

#[pyfunction]
fn hideable<'py>(entities: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let py = entities.py();
    let mesh = py.import("ingetrazo_core.mesh")?;
    let types = (
        mesh.getattr("Edge")?,
        mesh.getattr("Face")?,
        py.import("core.group")?.getattr("Group")?,
    )
        .into_pyobject(py)?;
    let hidden = py.import("ingetrazo_core")?.getattr("is_hidden")?;
    let out = PyList::empty(py);
    for e in entities.try_iter()? {
        let e = e?;
        if e.is_instance(&types)? && !hidden.call1((&e,))?.is_truthy()? {
            out.append(e)?;
        }
    }
    Ok(out)
}

fn at_least(sel: &Bound<'_, PyAny>, class: &str, count: usize) -> PyResult<bool> {
    let ty = sel.py().import("ingetrazo_core.mesh")?.getattr(class)?;
    let mut n = 0;
    for e in sel.try_iter()? {
        if e?.is_instance(&ty)? {
            n += 1;
            if n == count {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[pyfunction]
fn makes_faces(sel: &Bound<'_, PyAny>) -> PyResult<bool> {
    at_least(sel, "Edge", 3)
}

#[pyfunction]
fn bridgeable(mesh: &Bound<'_, PyAny>, sel: &Bound<'_, PyAny>) -> PyResult<bool> {
    if at_least(sel, "Face", 3)? {
        return Ok(false);
    }
    let bridge = sel.py().import("core.bridge")?;
    match bridge.call_method1("profiles_from_selection", (mesh, sel)) {
        Ok(_) => Ok(true),
        Err(e) if e.matches(sel.py(), bridge.getattr("BridgeError")?)? => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(edit_selection, m)?)?;
    m.add_function(wrap_pyfunction!(paste_model, m)?)?;
    m.add_function(wrap_pyfunction!(delete_entities, m)?)?;
    m.add_function(wrap_pyfunction!(hideable, m)?)?;
    m.add_function(wrap_pyfunction!(makes_faces, m)?)?;
    m.add_function(wrap_pyfunction!(bridgeable, m)?)?;
    Ok(())
}

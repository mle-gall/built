// SPDX-License-Identifier: GPL-3.0-or-later
//! Document inspector verbs.
use super::placements::one;
use super::transforms::empty;
use super::*;
#[pyfunction]
fn set_location<'py>(history: &Obj<'py>, lat: &Obj<'py>, lon: &Obj<'py>) -> Reply<'py> {
    let n = history.py().import("ingetrazo_core")?;
    let edit = n.call_method1("LocationEdit", (history.getattr("scene")?, lat, lon))?;
    execute(history, &n.call_method1("SetLocationCommand", (edit,))?)
}
#[pyfunction]
fn edit_link_subset<'py>(history: &Obj<'py>, placement: &Obj<'py>, text: &Obj<'py>) -> Reply<'py> {
    let n = history.py().import("ingetrazo_core")?;
    let edit = n.getattr("LinkSubset")?.call_method1(
        "prepare",
        (one(history, placement)?, history.getattr("scene")?, text),
    )?;
    if edit.is_none() {
        return answer(history, &history.getattr("revision")?, false);
    }
    execute(history, &n.call_method1("LinkSubsetCommand", (edit,))?)
}
#[pyfunction]
#[pyo3(signature=(history,uid,field,value=""))]
fn edit_section_plane<'py>(
    history: &Obj<'py>,
    uid: &Obj<'py>,
    field: &str,
    value: &str,
) -> Reply<'py> {
    execute(
        history,
        &history.py().import("ingetrazo_core")?.call_method1(
            "section_edit",
            (history.getattr("scene")?, uid, field, value),
        )?,
    )
}
#[pyfunction]
#[pyo3(signature=(history,placement,child=None,visible=None,name=None))]
fn edit_assembly_part<'py>(
    history: &Obj<'py>,
    placement: &Obj<'py>,
    child: Option<&str>,
    visible: Option<bool>,
    name: Option<&str>,
) -> Reply<'py> {
    let cmd = crate::assembly_inspector::plan_part_edit(
        &history.getattr("scene")?,
        &one(history, placement)?,
        child,
        visible,
        name,
    )?;
    execute(
        history,
        &cmd.unwrap_or_else(|| history.py().None().into_bound(history.py())),
    )
}
#[pyfunction]
#[pyo3(signature=(history,verb,key="",name=""))]
fn edit_visibility_set<'py>(history: &Obj<'py>, verb: &str, key: &str, name: &str) -> Reply<'py> {
    execute(
        history,
        &history.py().import("ingetrazo_core")?.call_method1(
            "visibility_set_command",
            (history.getattr("scene")?, verb, key, name),
        )?,
    )
}
#[pyfunction]
#[pyo3(signature=(history,verb,**args))]
fn edit_saved_scene<'py>(
    history: &Obj<'py>,
    verb: &str,
    args: Option<&Bound<'py, PyDict>>,
) -> Reply<'py> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let cmd =
        py.import("ingetrazo_core")?
            .call_method("plan_scene_command", (&scene, verb), args)?;
    let out = execute(history, &cmd)?;
    if cmd.is_none() {
        out.set_item("error", py.None())?;
        out.set_item("rejected", py.None())?;
    } else if out.as_any().get_item("ok")?.is_truthy()? && matches!(verb, "add" | "duplicate") {
        out.set_item(
            "index",
            scene
                .getattr("saved_views")?
                .call_method1("index", (cmd.getattr("view")?,))?,
        )?;
    }
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(history,tags=None,folders=None,visible=None,locked=None))]
fn set_tag_state<'py>(
    history: &Obj<'py>,
    tags: Option<&Obj<'py>>,
    folders: Option<&Obj<'py>>,
    visible: Option<&Obj<'py>>,
    locked: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let unique = |seq| {
        py.get_type::<PyDict>()
            .call_method1("fromkeys", (empty(py, seq),))
    };
    let tags = unique(tags)?;
    let folders = unique(folders)?;
    for k in tags.try_iter()? {
        let k = k?;
        if scene.call_method1("layer_by_key", (&k,))?.is_none() {
            return Err(PyKeyError::new_err(format!(
                "no tag with key {}",
                k.repr()?
            )));
        }
    }
    let known = scene
        .getattr_opt("layer_folders")?
        .filter(|v| !v.is_none())
        .unwrap_or_else(|| PyTuple::empty(py).into_any());
    for k in folders.try_iter()? {
        let k = k?;
        if !known.contains(&k)? {
            return Err(PyKeyError::new_err(format!(
                "no tag folder with key {}",
                k.repr()?
            )));
        }
    }
    if visible.is_none() && locked.is_none() {
        return Err(PyValueError::new_err(
            "set_tag_state needs visible or locked",
        ));
    }
    if folders.is_truthy()? && locked.is_some() && visible.is_none() {
        return Err(PyValueError::new_err("tag folders have no lock"));
    }
    let cmd = py.import("ingetrazo_core")?.call_method1(
        "SetTagStateCommand",
        (
            tags,
            folders,
            visible.map(|v| v.is_truthy()).transpose()?,
            locked.map(|v| v.is_truthy()).transpose()?,
        ),
    )?;
    execute(history, &cmd)
}
#[pyfunction]
fn rename_component<'py>(history: &Obj<'py>, placement: &Obj<'py>, name: &Obj<'py>) -> Reply<'py> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let name = name.str()?.call_method0("strip")?;
    if !name.is_truthy()? {
        return Err(PyValueError::new_err("a component needs a name"));
    }
    let mut group = None;
    if placement.is_instance_of::<pyo3::types::PyString>() {
        if let Some(kept) = scene
            .getattr_opt("kept_components")?
            .filter(|v| !v.is_none())
        {
            for g in kept.try_iter()? {
                let g = g?;
                if let Some(uid) = g.getattr_opt("uid")? {
                    if uid.eq(placement)? {
                        group = Some(g);
                    }
                }
            }
        }
    }
    let group = match group {
        Some(g) => g,
        None => one(history, placement)?,
    };
    let mesh = group.getattr("mesh")?;
    let module = py.import("core.kept_components")?;
    let targets = module.call_method1("copies_of", (&scene, &mesh))?;
    let template = module.call_method1("kept_template", (&scene, &mesh))?;
    let changed = PyDict::new(py);
    for g in targets.try_iter()?.chain([Ok(template), Ok(group)]) {
        let g = g?;
        if !g.is_none() && g.getattr("name")?.ne(&name)? {
            changed.set_item(g, &name)?;
        }
    }
    if changed.is_empty() {
        return answer(history, &history.getattr("revision")?, false);
    }
    let cmd = py
        .import("ingetrazo_core")?
        .getattr("NativeCommand")?
        .call_method1("rename_each", (changed.keys(), changed.values()))?;
    execute(history, &cmd)
}
fn settings<'py>(
    h: &Obj<'py>,
    label: &str,
    method: &str,
    args: impl pyo3::call::PyCallArgs<'py>,
) -> Reply<'py> {
    let n = h.py().import("ingetrazo_core")?;
    let edit = n.getattr("DocumentSettings")?.call_method1(method, args)?;
    execute(
        h,
        &n.call_method1("DocumentSettingsCommand", (label, edit))?,
    )
}
#[pyfunction]
fn set_model_units<'py>(history: &Obj<'py>, units: &Obj<'py>) -> Reply<'py> {
    settings(history, "Set model units", "model_units", (units,))
}
#[pyfunction]
fn set_dimension_style<'py>(history: &Obj<'py>, key: &Obj<'py>, value: &Obj<'py>) -> Reply<'py> {
    settings(
        history,
        "Set dimension style",
        "dimension_style",
        (key, value),
    )
}
#[pyfunction]
fn set_annotation_font<'py>(
    history: &Obj<'py>,
    family: &Obj<'py>,
    bold: &Obj<'py>,
    italic: &Obj<'py>,
    size: &Obj<'py>,
) -> Reply<'py> {
    settings(
        history,
        "Set annotation font",
        "annotation_font",
        (family, bold, italic, size),
    )
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(set_location, m)?)?;
    m.add_function(wrap_pyfunction!(edit_link_subset, m)?)?;
    m.add_function(wrap_pyfunction!(edit_section_plane, m)?)?;
    m.add_function(wrap_pyfunction!(edit_assembly_part, m)?)?;
    m.add_function(wrap_pyfunction!(edit_visibility_set, m)?)?;
    m.add_function(wrap_pyfunction!(edit_saved_scene, m)?)?;
    m.add_function(wrap_pyfunction!(set_tag_state, m)?)?;
    m.add_function(wrap_pyfunction!(rename_component, m)?)?;
    m.add_function(wrap_pyfunction!(set_model_units, m)?)?;
    m.add_function(wrap_pyfunction!(set_dimension_style, m)?)?;
    m.add_function(wrap_pyfunction!(set_annotation_font, m)?)?;
    Ok(())
}

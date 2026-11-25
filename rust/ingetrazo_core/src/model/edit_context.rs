// SPDX-License-Identifier: GPL-3.0-or-later
//! The group-edit stack of a scene (`Scene.begin_group_edit`,
//! `end_one_group_edit`, `end_group_edit`, `core/scene.py`): entering a
//! group makes tools edit its mesh, entering a child of the open one
//! nests, leaving steps out one level. The open path is the document
//! root's (`_enter_edit` / `_leave_edit`); what is here is the rest of an
//! entry, step for step as the Python did it: the open group's frame, the
//! store's occurrence view of an instance (or the world copy of a singular
//! placement, `command_protocol.enter_reference`, in its own transaction),
//! the attachments of labels, sheet holds, signs and dimensions carried
//! across the mesh swap, the dimension holds read either side of it, the
//! selection cleared and the version moved.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use super::module;

type Obj<'py> = Bound<'py, PyAny>;

/// Read every dimension's endpoints once, either side of an edit context
/// swapping a group's mesh: before the swap, the last known spots are made
/// current; after it, a hold on the mesh that went away finds the vertex
/// now standing there, before any edit can move it from under that spot.
#[pyfunction]
#[pyo3(name = "scene_refresh_dimension_holds")]
pub fn refresh_dimension_holds(scene: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    for d in scene.getattr(pyo3::intern!(py, "dimensions"))?.try_iter()? {
        let d = d?;
        d.getattr(pyo3::intern!(py, "a"))?;
        d.getattr(pyo3::intern!(py, "b"))?;
    }
    Ok(())
}

/// `(object, name, reference, saved)` for every attachment a mesh swap
/// must carry: text labels, sheet holds that are not dicts, placement
/// signs, both ends of every dimension.
fn attachment_swap<'py>(scene: &Obj<'py>) -> PyResult<Vec<(Obj<'py>, String, Obj<'py>, Obj<'py>)>> {
    let py = scene.py();
    let mut refs: Vec<(Obj<'py>, String)> = Vec::new();
    for label in scene.getattr("text_labels")?.try_iter()? {
        refs.push((label?, "attachment".into()));
    }
    let holds = module(py, "core.composition")?.getattr("sheet_holds")?;
    for sheet in scene.getattr("compositions")?.try_iter()? {
        let sheet = sheet?;
        for list in ["cotas", "etiquetas", "niveles"] {
            for item in sheet.getattr(list)?.try_iter()? {
                let item = item?;
                for pair in holds.call1((&item,))?.try_iter()? {
                    let (name, held): (String, Obj<'py>) = pair?.extract()?;
                    let r = held.get_item(1)?;
                    if !r.is_instance_of::<PyDict>() {
                        refs.push((item.clone(), format!("hold_{name}")));
                    }
                }
            }
        }
    }
    for sign in scene.getattr("text_placement_anchors")?.try_iter()? {
        refs.push((sign?, "attachment".into()));
    }
    for dim in scene.getattr("dimensions")?.try_iter()? {
        let dim = dim?;
        refs.push((dim.clone(), "attachment_a".into()));
        refs.push((dim, "attachment_b".into()));
    }
    let mut out = Vec::new();
    for (obj, name) in refs {
        let value = obj.getattr_opt(name.as_str())?.unwrap_or_else(|| py.None().into_bound(py));
        if value.is_none() {
            continue;
        }
        let r = if name.starts_with("hold_") { value.get_item(1)? } else { value };
        let saved = r.call_method0("before_mesh_swap")?;
        out.push((obj, name, r, saved));
    }
    Ok(out)
}

fn restore_attachments<'py>(refs: Vec<(Obj<'py>, String, Obj<'py>, Obj<'py>)>) -> PyResult<()> {
    for (obj, name, r, saved) in refs {
        let mut replacement = r.call_method1("after_mesh_swap", (saved,))?;
        if !replacement.is(&r) {
            if name.starts_with("hold_") {
                let first = obj.getattr(name.as_str())?.get_item(0)?;
                replacement = PyTuple::new(obj.py(), [first, replacement])?.into_any();
            }
            obj.setattr(name.as_str(), replacement)?;
        }
    }
    Ok(())
}

fn bump<'py>(scene: &Obj<'py>, name: &str) -> PyResult<()> {
    let v = scene.getattr(name)?;
    scene.setattr(name, v.add(1)?)
}

fn stack<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    scene.getattr("_edit_stack")
}

/// `Scene.begin_group_edit(group)`: enter `group` (nesting when it is a
/// child of the group open now; else what is open closes first, and the
/// group's own ancestors are entered first, as double-clicks would).
#[pyfunction]
#[pyo3(name = "scene_begin_group_edit")]
pub fn begin_group_edit<'py>(scene: &Obj<'py>, group: &Obj<'py>) -> PyResult<()> {
    let py = scene.py();
    let open = scene.getattr("edit_group")?;
    let nesting = !open.is_none() && {
        let kids = open.getattr_opt("children")?;
        match kids.filter(|k| k.is_truthy().unwrap_or(false)) {
            Some(k) => k.contains(group)?,
            None => false,
        }
    };
    if !open.is_none() && !nesting {
        end_group_edit(scene)?;
    }
    if !nesting {
        // Tree and protocol actions can open a nested row directly.
        let parent = group.getattr("_parent")?;
        if parent.is_instance(&module(py, "core.group")?.getattr("Group")?)? {
            begin_group_edit(scene, &parent)?;
        }
    }
    refresh_dimension_holds(scene)?;
    let mut attachments = Vec::new();
    let entry_mesh = group.getattr("mesh")?;
    let before = scene.getattr("mesh")?;
    // The context's own axes, read before anything rewrites the placement.
    let mut frame = module(py, "core.group")?.call_method1("group_frame", (group,))?;
    // The definition its sibling instances share (Hide Similar).
    let is_comp = group.getattr_opt("is_component")?;
    let definition = match is_comp {
        Some(f) if f.is_callable() && f.call0()?.is_truthy()? => group.getattr("mesh")?,
        _ => py.None().into_bound(py),
    };
    let mut parts = py.None().into_bound(py);
    let mut active = py.None().into_bound(py);
    let levels = stack(scene)?;
    let affine_above = levels.is_truthy()? && {
        let last = levels.get_item(-1)?;
        last.call_method1("get", ("affine",))?.is_truthy()?
    };
    if !group.getattr("xform")?.is_none() || affine_above {
        let world = scene.getattr("_root")?.call_method1("_edit_matrix", (group,))?;
        active = module(py, "ingetrazo_core.store_mesh")?.call_method1("instance_view", (group.getattr("mesh")?, world))?;
        if !active.is_none() {
            frame = active.getattr("_store")?.call_method0("tool_placement")?;
            let axes = group.getattr("axes")?;
            if !axes.is_none() {
                frame = frame.mul(axes)?;
            }
        }
    }
    let mut share = py.None().into_bound(py);
    let mut entry_transaction = py.None().into_bound(py);
    if active.is_none() && (group.getattr("children")?.is_truthy()? || !group.getattr("xform")?.is_none()) {
        attachments = attachment_swap(scene)?;
        // A singular placement needs the compatibility world copy; keep the
        // placement swaps on entry so its History step can replay them.
        let dm = module(py, "core.doc_model")?;
        let store_mesh = module(py, "ingetrazo_core.store_mesh")?.getattr("StoreMesh")?;
        let depth: i64 = if entry_mesh.get_type().is(&store_mesh) { dm.call_method0("begin")?.extract()? } else { 0 };
        let entered = module(py, "ingetrazo_core")?.getattr("command_protocol")?
            .call_method1("enter_reference", (scene, group, &frame));
        match entered {
            Ok(pair) => {
                share = pair.get_item(0)?;
                parts = pair.get_item(1)?;
                if depth != 0 {
                    entry_transaction = dm.call_method0("commit")?;
                }
            }
            Err(e) => {
                if depth != 0 {
                    dm.call_method0("rollback")?;
                }
                return Err(e);
            }
        }
    }
    if !stack(scene)?.is_truthy()? {
        scene.setattr("_loose_mesh", &before)?;
    }
    let level = PyDict::new(py);
    level.set_item("group", group)?;
    level.set_item("mesh", &before)?;
    level.set_item("affine", !active.is_none())?;
    level.set_item("session", module(py, "builtins")?.getattr("object")?.call0()?)?;
    level.set_item("share", &share)?;
    level.set_item("entry_transaction", &entry_transaction)?;
    level.set_item("frame", &frame)?;
    level.set_item("definition", &definition)?;
    level.set_item("parts", &parts)?;
    let now = group.getattr("mesh")?;
    level.set_item("world_copy", if entry_mesh.is(&now) { py.None().into_bound(py) } else { now.clone() })?;
    scene.getattr("_root")?.call_method1("_enter_edit", (group, level))?;
    bump(scene, "_placement_context_bumps")?;
    scene.setattr("mesh", if active.is_none() { group.getattr("mesh")? } else { active })?;
    scene.setattr("edit_group", group)?;
    restore_attachments(attachments)?;
    refresh_dimension_holds(scene)?;
    scene.getattr("selection")?.call_method0("clear")?;
    bump(scene, "version")
}

/// `Scene.end_one_group_edit()`: leave the innermost open group only.
#[pyfunction]
#[pyo3(name = "scene_end_one_group_edit")]
pub fn leave_level(scene: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let levels = stack(scene)?;
    if !levels.is_truthy()? {
        return Ok(());
    }
    refresh_dimension_holds(scene)?;
    let shared = levels.get_item(-1)?.call_method1("get", ("share",))?.is_truthy()?;
    let attachments = if shared { attachment_swap(scene)? } else { Vec::new() };
    let level = scene.getattr("_root")?.call_method0("_leave_edit")?;
    let mut unchanged = true;
    let share = level.call_method1("pop", ("share", py.None()))?;
    if !share.is_none() {
        unchanged = module(py, "ingetrazo_core")?.getattr("command_protocol")?
            .call_method1("finish_reference", (share, level.call_method1("get", ("parts",))?))?
            .is_truthy()?;
    }
    if unchanged {
        bump(scene, "_placement_context_bumps")?;
    }
    scene.setattr("mesh", level.get_item("mesh")?)?;
    let levels = stack(scene)?;
    if levels.is_truthy()? {
        scene.setattr("edit_group", levels.get_item(-1)?.get_item("group")?)?;
    } else {
        scene.setattr("edit_group", py.None())?;
        scene.setattr("_loose_mesh", py.None())?;
    }
    restore_attachments(attachments)?;
    refresh_dimension_holds(scene)?;
    scene.getattr("selection")?.call_method0("clear")?;
    bump(scene, "version")
}

/// `Scene.end_group_edit()`: leave every open group, back to the loose
/// mesh; when this returns, nothing is open.
#[pyfunction]
#[pyo3(name = "scene_end_group_edit")]
pub fn end_group_edit(scene: &Obj<'_>) -> PyResult<()> {
    while stack(scene)?.is_truthy()? {
        leave_level(scene)?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(refresh_dimension_holds, m)?)?;
    m.add_function(wrap_pyfunction!(begin_group_edit, m)?)?;
    m.add_function(wrap_pyfunction!(leave_level, m)?)?;
    m.add_function(wrap_pyfunction!(end_group_edit, m)?)?;
    Ok(())
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! `core.scene`, served from here (`core/scene.py` is gone): the scene
//! container.
//!
//! `edges` and `faces` are read-only views onto the mesh, so render, bounds
//! and `.igz` save consume them unchanged. Every mutation goes through mesh
//! methods (via the `Command` layer), which keep shared-vertex connectivity
//! and incidence in sync.
//!
//! `version` bumps on every mutation, so the viewport can cheaply decide
//! whether to rebuild its dynamic buffers.
//!
//! `Scene` is still a dataclass, with the fields, order and defaults it had,
//! and its tables still live on the document root (`ROOT_TABLES`). The rules
//! here are the per-entity ones: visible, selectable, locked, the layer an
//! entity shows under an open group, Hide Similar.
//!
//! Elsewhere:
//! - the group-edit stack is `edit_context.rs`;
//! - `clear` is `scene_clear.rs`;
//! - the boxes are `scene_bounds.rs`;
//! - the revision is `scene_revision.rs`.

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyCFunction, PyDict, PyList, PySet, PyString, PyTuple};

use super::layers_api::{layer_of, DEFAULT_LAYER};
use super::module;
use super::py_class::{method, property, static_method, FastAttr};

type Obj<'py> = Bound<'py, PyAny>;

const MODULE: &str = "core.scene";

/// The scene fields that live on the root node (their dataclass defaults
/// still apply: `__init__` assigns them through these).
const ROOT_TABLES: [&str; 25] = [
    "units", "active_layer", "layer_folders", "folder_parents", "folder_visibility", "layer_folder_uids",
    "layer_folder_names", "plugin_data", "link_registry", "dimensions", "text_labels", "text_placement_anchors",
    "dimension_style", "saved_views", "scene_folders", "scene_folder_parents", "compositions", "guides",
    "image_planes", "geo_points", "geo_paths", "custom_scales", "section_planes", "show_section_planes",
    "show_section_cuts",
];

struct Cache {
    group: Py<PyAny>,
    element_set: Py<PyAny>,
    effective_visible: Py<PyAny>,
    edge: Py<PyAny>,
    face: Py<PyAny>,
}

static CACHE: PyOnceLock<Cache> = PyOnceLock::new();
static SCENE_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

fn cache(py: Python<'_>) -> PyResult<&Cache> {
    CACHE.get_or_try_init(py, || -> PyResult<Cache> {
        let mesh = py.import("ingetrazo_core.mesh")?;
        Ok(Cache {
            group: py.import("core.group")?.getattr("Group")?.unbind(),
            element_set: py.import("ingetrazo_core")?.getattr("ElementSet")?.unbind(),
            effective_visible: native(py)?.getattr("tag_edits")?.getattr("effective_visible")?.unbind(),
            edge: mesh.getattr("Edge")?.unbind(),
            face: mesh.getattr("Face")?.unbind(),
        })
    })
}

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn native(py: Python<'_>) -> PyResult<Obj<'_>> {
    module(py, "ingetrazo_core")
}

fn opt<'py>(o: &Obj<'py>, name: &Bound<'py, PyString>) -> PyResult<Obj<'py>> {
    Ok(o.attr_opt(name)?.unwrap_or_else(|| none(o.py())))
}

fn flag(o: &Obj<'_>, name: &Bound<'_, PyString>) -> PyResult<bool> {
    match o.attr_opt(name)? {
        Some(v) => v.is_truthy(),
        None => Ok(false),
    }
}

fn root<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    me.getattr(intern!(me.py(), "_root"))
}

fn edit_stack<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    root(me)?.getattr(intern!(me.py(), "_edit_levels"))
}

fn bump(me: &Obj<'_>, name: &Bound<'_, PyString>) -> PyResult<()> {
    me.setattr(name, me.getattr(name)?.add(1)?)
}

// ------------------------------------------------------------ the dataclass

fn truth<'py>(py: Python<'py>, b: bool) -> Obj<'py> {
    pyo3::types::PyBool::new(py, b).to_owned().into_any()
}

fn fast_setattr<'py>(me: &Obj<'py>, args: &[Obj<'py>]) -> PyResult<Obj<'py>> {
    let [name, value] = args else { return Err(pyo3::exceptions::PyTypeError::new_err("__setattr__ takes a name and a value")) };
    scene_setattr(me, name.cast::<PyString>()?, value)?;
    Ok(none(me.py()))
}
crate::fast!(setattr_entry, fast_setattr);

fn one<'a, 'py>(args: &'a [Obj<'py>]) -> PyResult<&'a Obj<'py>> {
    match args {
        [a] => Ok(a),
        _ => Err(pyo3::exceptions::PyTypeError::new_err("takes exactly one argument")),
    }
}

fn fast_visible<'py>(me: &Obj<'py>, args: &[Obj<'py>]) -> PyResult<Obj<'py>> {
    Ok(truth(me.py(), entity_visible(me, one(args)?)?))
}
crate::fast!(visible_entry, fast_visible);

fn fast_selectable<'py>(me: &Obj<'py>, args: &[Obj<'py>]) -> PyResult<Obj<'py>> {
    Ok(truth(me.py(), entity_selectable(me, one(args)?)?))
}
crate::fast!(selectable_entry, fast_selectable);

fn fast_layer_by_key<'py>(me: &Obj<'py>, args: &[Obj<'py>]) -> PyResult<Obj<'py>> {
    scene_layer_by_key(me, one(args)?)
}
crate::fast!(layer_by_key_entry, fast_layer_by_key);

fn fast_layer<'py>(me: &Obj<'py>, args: &[Obj<'py>]) -> PyResult<Obj<'py>> {
    scene_layer(me, one(args)?)
}
crate::fast!(layer_entry, fast_layer);

fn scene_setattr<'py>(me: &Obj<'py>, name: &Bound<'py, PyString>, value: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    // All command/tool assignments share the same key-based membership.
    let mut value = value.clone();
    if name.to_str()? == "selection" {
        let es = cache(py)?.element_set.bind(py);
        if !value.is_instance(es)? {
            value = es.call1((value,))?;
        }
    }
    // SAFETY: `object.__setattr__(me, name, value)` on live objects.
    let rc = unsafe { pyo3::ffi::PyObject_GenericSetAttr(me.as_ptr(), name.as_ptr(), value.as_ptr()) };
    if rc < 0 {
        return Err(PyErr::fetch(py));
    }
    Ok(())
}

#[pyfunction]
fn scene_edit_stack<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    edit_stack(me)
}

#[pyfunction]
fn scene_post_init<'py>(me: &Obj<'py>, groups: &Obj<'py>, layers: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    if !groups.is_none() {
        me.setattr(intern!(py, "groups"), groups)?;
    }
    if !layers.is_none() {
        me.setattr(intern!(py, "layers"), layers)?;
    }
    Ok(())
}

fn object_setattr(o: &Obj<'_>, name: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    // SAFETY: as in `scene_setattr`.
    let rc = unsafe { pyo3::ffi::PyObject_GenericSetAttr(o.as_ptr(), name.as_ptr(), value.as_ptr()) };
    if rc < 0 { Err(PyErr::fetch(o.py())) } else { Ok(()) }
}

#[pyfunction]
fn scene_deepcopy<'py>(me: &Obj<'py>, memo: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let deepcopy = module(py, "copy")?.getattr(intern!(py, "deepcopy"))?;
    let builtins = py.import("builtins")?;
    let out = builtins.getattr("object")?.call_method1("__new__", (me.get_type(),))?;
    memo.set_item(me.as_ptr() as usize, &out)?;
    let own = me.getattr(intern!(py, "__dict__"))?;
    for item in own.cast::<PyDict>()?.items().iter() {
        let (name, value): (Obj<'py>, Obj<'py>) = item.extract()?;
        if !name.eq("_root")? {
            object_setattr(&out, &name, &deepcopy.call1((value, memo))?)?;
        }
    }
    object_setattr(&out, PyString::new(py, "_root").as_any(), &native(py)?.call_method0("doc_new_root")?)?;
    let out_root = root(&out)?;
    for level in edit_stack(me)?.try_iter()? {
        let copied = deepcopy.call1((level?, memo))?;
        out_root.call_method1("_enter_edit", (copied.get_item("group")?, &copied))?;
    }
    let list = builtins.getattr("list")?;
    out.setattr("groups", deepcopy.call1((list.call1((me.getattr("groups")?,))?, memo))?)?;
    out.setattr("layers", deepcopy.call1((list.call1((me.getattr("layers")?,))?, memo))?)?;
    for name in ROOT_TABLES {
        out.setattr(name, deepcopy.call1((me.getattr(name)?, memo))?)?;
    }
    out.setattr("materials", deepcopy.call1((builtins.getattr("dict")?.call1((me.getattr("materials")?,))?, memo))?)?;
    out.setattr("kept_components", deepcopy.call1((list.call1((me.getattr("kept_components")?,))?, memo))?)?;
    Ok(out)
}

#[pyfunction]
fn scene_edges<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    me.getattr(intern!(py, "mesh"))?.getattr(intern!(py, "edges"))
}

#[pyfunction]
fn scene_faces<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    me.getattr(intern!(py, "mesh"))?.getattr(intern!(py, "faces"))
}

// ------------------------------------------------------------------ layers

/// The tag whose key is `key`: what entities carry.
fn scene_layer_by_key<'py>(me: &Obj<'py>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    if key.is_instance_of::<PyString>() {
        return root(me)?.call_method1(intern!(py, "_tag_by_key"), (key,));
    }
    for ly in me.getattr(intern!(py, "layers"))?.try_iter()? {
        let ly = ly?;
        if ly.getattr(intern!(py, "key"))?.eq(key)? {
            return Ok(ly);
        }
    }
    Ok(none(py))
}

/// The tag `name` names: by its key (what entities carry), else by its
/// label (the first with it, labels being free to repeat).
fn scene_layer<'py>(me: &Obj<'py>, name: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    if !name.is_truthy()? {
        return Ok(none(py));
    }
    let ly = scene_layer_by_key(me, name)?;
    if !ly.is_none() {
        return Ok(ly);
    }
    if name.is_instance_of::<PyString>() {
        return root(me)?.call_method1(intern!(py, "_tag_by_name"), (name,));
    }
    for ly in me.getattr(intern!(py, "layers"))?.try_iter()? {
        let ly = ly?;
        if ly.getattr(intern!(py, "name"))?.eq(name)? {
            return Ok(ly);
        }
    }
    Ok(none(py))
}

/// `(visible, locked)` of the layer `entity` carries; unknown layer names
/// read as the default (visible, unlocked).
fn layer_state<'py>(me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<(bool, Obj<'py>)> {
    let py = me.py();
    let mut name = layer_of(entity)?;
    if name.eq(DEFAULT_LAYER)? && !me.getattr(intern!(py, "edit_group"))?.is_none() {
        let open = open_group_layer(me, entity)?;
        if open.is_truthy()? {
            name = open;
        }
    }
    let ly = scene_layer_by_key(me, &name)?;
    if ly.is_none() {
        return Ok((true, pyo3::types::PyBool::new(py, false).to_owned().into_any()));
    }
    let visible = cache(py)?.effective_visible.bind(py).call1((me, &ly))?.is_truthy()?;
    Ok((visible, ly.getattr(intern!(py, "locked"))?))
}

#[pyfunction]
#[pyo3(name = "_layer_state")]
fn scene_layer_state<'py>(me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let (visible, locked) = layer_state(me, entity)?;
    PyTuple::new(me.py(), [pyo3::types::PyBool::new(me.py(), visible).to_owned().into_any(), locked])
}

fn open_path<'py>(me: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let py = me.py();
    py.import("core.outliner")?.call_method1(intern!(py, "open_path"), (me,))?.try_iter()?.collect()
}

/// The layer a face or edge with none of its own follows while its group
/// is open: the open group's, or the nearest enclosing group's that has
/// one. Closed, the group's layer already decides for everything in it;
/// open, its geometry is drawn and picked as loose geometry, and hiding the
/// default layer took the contents of a group on another layer with it
/// (#366).
fn open_group_layer<'py>(me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    if entity.has_attr(intern!(py, "children"))? || !(entity.has_attr(intern!(py, "attrs"))? || entity.has_attr(intern!(py, "v0"))?) {
        return Ok(none(py));
    }
    let mut group_layer = none(py);
    for group in open_path(me)?.iter().rev() {
        group_layer = opt(group, intern!(py, "layer"))?;
        if group_layer.is_truthy()? {
            break;
        }
    }
    if !group_layer.is_truthy()? {
        return Ok(none(py));
    }
    let mesh = me.getattr(intern!(py, "mesh"))?;
    let faces = mesh.getattr(intern!(py, "faces"))?;
    let edges = mesh.getattr(intern!(py, "edges"))?;
    let key = PyTuple::new(py, [
        (mesh.as_ptr() as usize).into_pyobject(py)?.into_any(),
        me.getattr(intern!(py, "version"))?,
        faces.len()?.into_pyobject(py)?.into_any(),
        edges.len()?.into_pyobject(py)?.into_any(),
    ])?;
    let identity = py.import("ingetrazo_core.mesh")?.getattr(intern!(py, "element_identity"))?;
    let memo = match me.attr_opt(intern!(py, "_open_mesh_ids"))? {
        Some(m) if !m.is_none() && m.get_item(0)?.eq(&key)? => m,
        _ => {
            let ids = PySet::empty(py)?;
            for f in faces.try_iter()? {
                ids.add(identity.call1((f?,))?)?;
            }
            for e in edges.try_iter()? {
                ids.add(identity.call1((e?,))?)?;
            }
            let memo = PyTuple::new(py, [key.into_any(), ids.into_any()])?.into_any();
            me.setattr(intern!(py, "_open_mesh_ids"), &memo)?;
            memo
        }
    };
    if memo.get_item(1)?.contains(identity.call1((entity,))?)? { Ok(group_layer) } else { Ok(none(py)) }
}

#[pyfunction]
#[pyo3(name = "_open_group_layer")]
fn scene_open_group_layer<'py>(me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<Obj<'py>> {
    open_group_layer(me, entity)
}

/// The paint the open group's default faces wear: its own, or the nearest
/// enclosing open group's that has one (what they wore with the group
/// closed, #1101). None outside a group or when nothing is painted.
#[pyfunction]
fn scene_open_group_paint<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    for group in open_path(me)?.iter().rev() {
        let paint = opt(group, intern!(py, "material"))?;
        if paint.is_truthy()? {
            return Ok(paint);
        }
    }
    Ok(none(py))
}

/// Hide on an OBJECT (a group or component). Edges carry a `hidden` of
/// their own with older, narrower semantics (they stay in the topology and
/// the draw passes skip them themselves), so only a group answers here.
#[pyfunction]
fn object_hidden(entity: &Obj<'_>) -> PyResult<bool> {
    let py = entity.py();
    Ok(flag(entity, intern!(py, "hidden"))? && entity.has_attr(intern!(py, "children"))?)
}

/// Hide on a face: `attrs["hidden"]`.
#[pyfunction]
fn face_hidden(entity: &Obj<'_>) -> PyResult<bool> {
    let py = entity.py();
    let attrs = opt(entity, intern!(py, "attrs"))?;
    Ok(attrs.is_truthy()? && attrs.call_method1(intern!(py, "get"), (intern!(py, "hidden"),))?.is_truthy()?)
}

/// Hidden by Hide (object or face), regardless of the layer, and of
/// whether the hidden-things view is on.
#[pyfunction]
fn scene_entity_hidden<'py>(_me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<bool> {
    Ok(object_hidden(entity)? || face_hidden(entity)?)
}

fn navigator_key<'py>(me: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let uid = group.getattr(intern!(py, "uid"))?;
    match me.attr_opt(intern!(py, "_navigator_keys"))? {
        Some(keys) => keys.call_method1(intern!(py, "get"), (group, uid)),
        None => Ok(uid),
    }
}

#[pyfunction]
fn scene_navigator_key<'py>(me: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    navigator_key(me, group)
}

/// Whether the navigator's filter leaves `entity` out.
fn filtered_out<'py>(me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<bool> {
    let py = me.py();
    let Some(allowed) = me.attr_opt(intern!(py, "_navigator_allowed"))? else { return Ok(false) };
    if allowed.is_none() || !entity.is_instance(cache(py)?.group.bind(py))? {
        return Ok(false);
    }
    Ok(!allowed.contains(navigator_key(me, entity)?)?)
}

fn entity_visible<'py>(me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<bool> {
    if filtered_out(me, entity)? {
        return Ok(false);
    }
    // A hidden thing is gone from every consumer that asks this (render,
    // pick, snap, bounds, export), whatever its layer says. The
    // hidden-things view does NOT change that: it draws them as a ghost in
    // its own pass and only makes them selectable.
    if object_hidden(entity)? || face_hidden(entity)? || similar_hidden(me, entity)? {
        return Ok(false);
    }
    Ok(layer_state(me, entity)?.0)
}

/// Whether the inference may land on `edge`: the visibility rule the
/// render, `bounds()` and the pick already follow.
///
/// An EDGE can be hidden on its own (Hide with the eraser, the seams of a
/// smoothed surface) without hiding what it bounds, and the draw passes
/// skip it: no line, no profile. The snap engine did not ask, so the point
/// offered to the cursor existed nowhere on screen, and a line drawn to it
/// landed on a spot the user could not see, cutting the face it lay on (the
/// "phantom vertex" segmentations). Hidden EDGES come back for inference
/// only with Camera ▸ Hidden Geometry on, the same switch that draws them as
/// a ghost and makes them selectable.
#[pyfunction]
fn scene_edge_snappable<'py>(me: &Obj<'py>, edge: &Obj<'py>) -> PyResult<bool> {
    let py = me.py();
    if !entity_visible(me, edge)? {
        return Ok(false);
    }
    Ok(!flag(edge, intern!(py, "hidden"))? || flag(me, intern!(py, "show_hidden_geometry"))?)
}

/// Visible group geometry is a read-only drawing reference, even when the
/// group or its tag is locked. Revealed editable geometry keeps its existing
/// inference behaviour.
#[pyfunction]
fn scene_group_snappable<'py>(me: &Obj<'py>, group: &Obj<'py>) -> PyResult<bool> {
    Ok(entity_visible(me, group)? || entity_selectable(me, group)?)
}

fn edit_definition<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let levels = edit_stack(me)?;
    if !levels.is_truthy()? {
        return Ok(none(py));
    }
    levels.get_item(-1)?.call_method1(intern!(py, "get"), (intern!(py, "definition"),))
}

/// The shared prototype mesh of the component instance open at the
/// innermost level, or None (at the root, or inside a classic group).
#[pyfunction]
fn scene_edit_definition<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    edit_definition(me)
}

/// Out of the frame by Hide Similar Components: another instance of the
/// definition being edited (or a nested placement of one). Nothing is
/// hidden at the root, so saving and exporting never see it.
fn similar_hidden<'py>(me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<bool> {
    let py = me.py();
    if !me.getattr(intern!(py, "hide_similar_components"))?.is_truthy()? || !edit_stack(me)?.is_truthy()? {
        return Ok(false);
    }
    let open = me.getattr(intern!(py, "edit_group"))?;
    if !entity.has_attr(intern!(py, "children"))? || entity.is(&open) {
        return Ok(false);
    }
    let definition = edit_definition(me)?;
    if definition.is_none() {
        return Ok(false);
    }
    for g in [entity.clone(), opt(entity, intern!(py, "owner"))?] {
        if !g.is_none() && !g.is(&open) && !opt(&g, intern!(py, "xform"))?.is_none() && opt(&g, intern!(py, "mesh"))?.is(&definition) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[pyfunction]
fn scene_similar_hidden<'py>(me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<bool> {
    similar_hidden(me, entity)
}

/// Locked as an object: the group itself, or (for a placement the viewport
/// picks as part of its parent) the object that owns it.
#[pyfunction]
fn entity_locked(entity: &Obj<'_>) -> PyResult<bool> {
    let py = entity.py();
    for g in [entity.clone(), opt(entity, intern!(py, "owner"))?] {
        if !g.is_none() && flag(&g, intern!(py, "locked"))? && g.has_attr(intern!(py, "children"))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn entity_selectable<'py>(me: &Obj<'py>, entity: &Obj<'py>) -> PyResult<bool> {
    let py = me.py();
    if filtered_out(me, entity)? || similar_hidden(me, entity)? {
        return Ok(false);
    }
    if object_hidden(entity)? && !me.getattr(intern!(py, "show_hidden_objects"))?.is_truthy()? {
        return Ok(false);
    }
    if face_hidden(entity)? && !me.getattr(intern!(py, "show_hidden_geometry"))?.is_truthy()? {
        return Ok(false);
    }
    if entity_locked(entity)? {
        return Ok(false);
    }
    let c = cache(py)?;
    if (entity.is_instance(c.edge.bind(py))? || entity.is_instance(c.face.bind(py))?) && root(me)?.call_method0(intern!(py, "_loose_locked"))?.is_truthy()? {
        return Ok(false);
    }
    let (visible, locked) = layer_state(me, entity)?;
    Ok(visible && !locked.is_truthy()?)
}

#[pyfunction]
fn scene_repeated_uids<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    root(me)?.call_method0(intern!(me.py(), "_repeated_uids"))
}

/// Isolating occurrence keys, from the native tree (doc/occurrences.rs):
/// (matched keys, shown keys, drawn-through keys, {group: address key}).
#[pyfunction]
#[pyo3(signature = (me, wanted, exact=false))]
fn scene_occurrence_closure<'py>(me: &Obj<'py>, wanted: &Obj<'py>, exact: bool) -> PyResult<Bound<'py, PyTuple>> {
    let py = me.py();
    let wanted = PySet::new(py, wanted.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
    let got = root(me)?.call_method1(intern!(py, "_occurrence_closure"), (wanted, exact))?;
    let qualified = py.import("builtins")?.getattr("dict")?.call1((got.get_item(3)?,))?;
    PyTuple::new(py, [got.get_item(0)?, got.get_item(1)?, got.get_item(2)?, qualified])
}

/// The UIDs from this scene's root through the placement.
#[pyfunction]
fn scene_occurrence_path<'py>(me: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    root(me)?.call_method1(intern!(me.py(), "_occurrence_path"), (group,))
}

/// Look up an ordered path of persisted placement UIDs from this root.
#[pyfunction]
fn scene_resolve_occurrence<'py>(me: &Obj<'py>, path: &Obj<'py>) -> PyResult<Obj<'py>> {
    root(me)?.call_method1(intern!(me.py(), "_resolve_occurrence"), (path,))
}

/// `uid → group` over the whole tree (nested placements too).
#[pyfunction]
fn scene_groups_by_uid<'py>(me: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = me.py();
    let out = PyDict::new(py);
    for g in native(py)?.call_method1(intern!(py, "purge_iter_groups"), (me.getattr(intern!(py, "groups"))?,))?.try_iter()? {
        let g = g?;
        out.set_item(g.getattr(intern!(py, "uid"))?, g)?;
    }
    Ok(out)
}

// --------------------------------------------------------------- sections

/// The section plane currently cutting the model, or None.
#[pyfunction]
fn scene_active_section<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    for sp in me.getattr(intern!(py, "section_planes"))?.try_iter()? {
        let sp = sp?;
        if sp.getattr(intern!(py, "active"))?.is_truthy()? {
            // A plane on a hidden layer cuts nothing (#316).
            return Ok(if layer_state(me, &sp)?.0 { sp } else { none(py) });
        }
    }
    Ok(none(py))
}

/// Make `plane` the ONE active cut (None deactivates all): one active cut
/// per context.
#[pyfunction]
fn scene_set_active_section<'py>(me: &Obj<'py>, plane: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    for sp in me.getattr(intern!(py, "section_planes"))?.try_iter()? {
        let sp = sp?;
        let on = sp.is(plane);
        sp.setattr(intern!(py, "active"), on)?;
    }
    Ok(())
}

// -------------------------------------------------- the group-edit context

/// The axes drawing happens on (issue #44): the world's (None) at the top
/// level, the open group's own axes inside it.
#[pyfunction]
fn scene_drawing_frame<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let levels = edit_stack(me)?;
    if levels.is_truthy()? {
        return levels.get_item(-1)?.call_method1(intern!(py, "get"), (intern!(py, "frame"),));
    }
    Ok(none(py))
}

/// The real loose mesh regardless of the edit context.
#[pyfunction]
fn scene_loose_mesh<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    if me.getattr(intern!(py, "edit_group"))?.is_none() { me.getattr(intern!(py, "mesh")) } else { me.getattr(intern!(py, "_loose_mesh")) }
}

// ----------------------------------------------------------- render views

fn walk<'py>(me: &Obj<'py>, pg: &Obj<'py>, base: &Obj<'py>, out: &Bound<'py, PyList>) -> PyResult<()> {
    let py = me.py();
    let own = opt(pg, intern!(py, "xform"))?;
    let m = if base.is_none() { own } else if own.is_none() { base.clone() } else { base.mul(own)? };
    out.append((pg, &m))?;
    let kids = opt(pg, intern!(py, "children"))?;
    if kids.is_truthy()? {
        for child in kids.try_iter()? {
            let child = child?;
            if flag(&child, intern!(py, "billboard"))? || !entity_visible(me, &child)? {
                continue;
            }
            walk(me, &child, &m, out)?;
        }
    }
    Ok(())
}

fn placements<'py>(me: &Obj<'py>, precise: bool) -> PyResult<Bound<'py, PyList>> {
    let py = me.py();
    let out = PyList::empty(py);
    let nothing = none(py);
    for g in me.getattr(intern!(py, "groups"))?.try_iter()? {
        let g = g?;
        if !entity_visible(me, &g)? || flag(&g, intern!(py, "billboard"))? {
            continue;
        }
        if precise {
            let scene = me.clone().unbind();
            let visible = PyCFunction::new_closure(py, None, None, move |args, _kw| -> PyResult<bool> {
                let child = args.get_item(0)?;
                let py = child.py();
                Ok(!child.getattr(intern!(py, "billboard"))?.is_truthy()? && entity_visible(scene.bind(py), &child)?)
            })?;
            let kw = PyDict::new(py);
            kw.set_item("visible", visible)?;
            for item in native(py)?.getattr(intern!(py, "group_model_placements"))?.call((&g,), Some(&kw))?.try_iter()? {
                out.append(item?)?;
            }
        } else {
            walk(me, &g, &nothing, &out)?;
        }
    }
    Ok(out)
}

/// Every visible placement in the scene: each group and, below it, the
/// nested ones a component keeps inside itself, as `(group,
/// world_matrix_or_None)`.
///
/// Face-me billboards are left out (they are drawn per frame, not from
/// their mesh). A nested placement that is not visible takes its subtree
/// along, as a hidden top-level group always did.
#[pyfunction]
#[pyo3(signature = (me, *, precise=false))]
fn scene_placements<'py>(me: &Obj<'py>, precise: bool) -> PyResult<Obj<'py>> {
    placements(me, precise)?.try_iter().map(Bound::into_any)
}

fn render_items<'py>(me: &Obj<'py>, what: &Bound<'py, PyString>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let out = PyList::empty(py);
    for e in scene_loose_mesh(me)?.getattr(what)?.try_iter()? {
        let e = e?;
        if entity_visible(me, &e)? {
            out.append(e)?;
        }
    }
    for item in placements(me, false)?.iter() {
        for e in item.get_item(0)?.getattr(intern!(py, "mesh"))?.getattr(what)?.try_iter()? {
            out.append(e?)?;
        }
    }
    Ok(out.try_iter()?.into_any())
}

#[pyfunction]
fn scene_render_edges<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    render_items(me, intern!(me.py(), "edges"))
}

#[pyfunction]
fn scene_render_faces<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    render_items(me, intern!(me.py(), "faces"))
}

// -------------------------------------------------------------- mutations

#[pyfunction]
fn scene_add_edge<'py>(me: &Obj<'py>, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    let edge = me.getattr(intern!(py, "mesh"))?.call_method1(intern!(py, "add_edge"), (a, b))?;
    bump(me, intern!(py, "version"))?;
    Ok(edge)
}

fn bump_view(me: &Obj<'_>) -> PyResult<()> {
    let py = me.py();
    bump(me, intern!(py, "version"))?;
    bump(me, intern!(py, "view_version"))
}

/// Put `edges` (any entities) into the selection the way `mode` says, the
/// usual click modifiers: `"replace"` (a plain click), `"add"` (Ctrl),
/// `"toggle"` (Shift: what is in goes out, what is out comes in) and
/// `"remove"` (Shift+Ctrl). `additive=True` is the old spelling of `"add"`
/// and still works; an explicit `mode` wins over it.
#[pyfunction]
#[pyo3(signature = (me, edges, additive=false, mode=None))]
fn scene_select<'py>(me: &Obj<'py>, edges: &Obj<'py>, additive: bool, mode: Option<&Obj<'py>>) -> PyResult<()> {
    let py = me.py();
    let mode: String = match mode.filter(|m| m.is_truthy().unwrap_or(false)) {
        Some(m) => m.extract()?,
        None => if additive { "add".into() } else { "replace".into() },
    };
    let sel = me.getattr(intern!(py, "selection"))?;
    if mode == "replace" {
        sel.call_method0(intern!(py, "clear"))?;
    }
    if mode == "toggle" {
        for ent in edges.try_iter()? {
            let ent = ent?;
            if sel.contains(&ent)? {
                sel.call_method1(intern!(py, "discard"), (&ent,))?;
            } else {
                sel.call_method1(intern!(py, "add"), (&ent,))?;
            }
        }
    } else if mode == "remove" {
        sel.call_method1(intern!(py, "difference_update"), (edges,))?;
    } else {
        sel.call_method1(intern!(py, "update"), (edges,))?;
    }
    bump_view(me)
}

#[pyfunction]
fn scene_clear_selection<'py>(me: &Obj<'py>) -> PyResult<()> {
    let py = me.py();
    let sel = me.getattr(intern!(py, "selection"))?;
    if sel.is_truthy()? {
        sel.call_method0(intern!(py, "clear"))?;
        bump_view(me)?;
    }
    Ok(())
}

/// A change of what is shown, not of the document (the selection): the
/// caches keyed on `version` refresh, the document stays clean.
#[pyfunction]
fn scene_bump_view(me: &Obj<'_>) -> PyResult<()> {
    bump_view(me)
}

/// The section planes changed and nothing else (see `section_version`):
/// the document is edited, the model is not.
#[pyfunction]
fn scene_bump_sections(me: &Obj<'_>) -> PyResult<()> {
    let py = me.py();
    bump(me, intern!(py, "version"))?;
    bump(me, intern!(py, "section_version"))
}

fn content_version<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = me.py();
    me.getattr(intern!(py, "version"))?.sub(me.getattr(intern!(py, "view_version"))?)
}

/// `content_version` without the section planes' own bumps: what the
/// faces, edges and objects are.
#[pyfunction]
fn scene_geometry_version<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    content_version(me)?.sub(me.getattr(intern!(me.py(), "section_version"))?)
}

/// `version` minus the view-only bumps: what "unsaved changes" compares
/// against the version that was saved.
#[pyfunction]
fn scene_content_version<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    content_version(me)
}

/// Edit ▸ Invert Selection (Ctrl+Shift+I): select every entity of the open
/// context that is NOT selected now, and drop what is. The universe is
/// Select All's (the loose edges and faces, the context's groups, the
/// dimensions and, in the model, the construction guides) minus what a
/// click or a box could not pick either. Returns the size of the new
/// selection.
#[pyfunction]
fn scene_invert_selection<'py>(me: &Obj<'py>) -> PyResult<usize> {
    let py = me.py();
    let ctx = me.getattr(intern!(py, "edit_group"))?;
    let groups = if ctx.is_none() {
        me.getattr(intern!(py, "groups"))?
    } else {
        let kids = opt(&ctx, intern!(py, "children"))?;
        if kids.is_truthy()? { kids } else { PyList::empty(py).into_any() }
    };
    let show_hidden = me.getattr(intern!(py, "show_hidden_geometry"))?.is_truthy()?;
    let mut universe = Vec::new();
    for e in scene_edges(me)?.try_iter()? {
        let e = e?;
        if entity_selectable(me, &e)? && (show_hidden || !flag(&e, intern!(py, "hidden"))?) {
            universe.push(e);
        }
    }
    let mut add_from = |items: Obj<'py>| -> PyResult<()> {
        for x in items.try_iter()? {
            let x = x?;
            if entity_selectable(me, &x)? {
                universe.push(x);
            }
        }
        Ok(())
    };
    add_from(scene_faces(me)?)?;
    add_from(groups)?;
    add_from(me.getattr(intern!(py, "dimensions"))?)?;
    if ctx.is_none() {
        add_from(me.getattr(intern!(py, "guides"))?)?; // guides belong to the model (#627)
    }
    let sel = me.getattr(intern!(py, "selection"))?;
    let mut fresh = Vec::new();
    for ent in universe {
        if !sel.contains(&ent)? {
            fresh.push(ent);
        }
    }
    let n = fresh.len();
    sel.call_method0(intern!(py, "clear"))?;
    sel.call_method1(intern!(py, "update"), (PyList::new(py, fresh)?,))?;
    bump_view(me)?; // the GL colour caches are keyed on it
    Ok(n)
}

/// What `bounds` depends on besides the content: the edit context (it swaps
/// `mesh`), Hide Similar (it follows the context) and the layers'
/// visibility. Every change of these bumps `version` already; the key is
/// there so that a view-only bump (the selection) is told apart from them.
#[pyfunction]
#[pyo3(name = "_bounds_key")]
fn scene_bounds_key<'py>(me: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = me.py();
    let levels = PyTuple::new(py, edit_stack(me)?.try_iter()?.map(|l| Ok(l?.get_item("group")?.as_ptr() as usize)).collect::<PyResult<Vec<_>>>()?)?;
    let ev = cache(py)?.effective_visible.bind(py);
    let mut tags = Vec::new();
    for ly in me.getattr(intern!(py, "layers"))?.try_iter()? {
        let ly = ly?;
        let visible = ev.call1((me, &ly))?;
        tags.push(PyTuple::new(py, [ly.getattr(intern!(py, "key"))?, visible])?);
    }
    PyTuple::new(py, [
        content_version(me)?,
        levels.into_any(),
        (me.getattr(intern!(py, "mesh"))?.as_ptr() as usize).into_pyobject(py)?.into_any(),
        pyo3::types::PyBool::new(py, me.getattr(intern!(py, "hide_similar_components"))?.is_truthy()?).to_owned().into_any(),
        PyTuple::new(py, tags)?.into_any(),
    ])
}

// ---------------------------------------------------- document-held fields

fn groups_owner<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    // `_groups_owner`: a placement whose children stand in for the top
    // level while a command runs inside an open group (GroupInContextCommand).
    let py = me.py();
    let own = me.getattr(intern!(py, "__dict__"))?;
    let owner = own.cast::<PyDict>()?.get_item(intern!(py, "_groups_owner"))?;
    match owner.filter(|o| !o.is_none()) {
        Some(o) => Ok(o),
        None => root(me),
    }
}

#[pyfunction]
fn scene_get_groups<'py>(me: &Obj<'py>) -> PyResult<Obj<'py>> {
    groups_owner(me)?.getattr(intern!(me.py(), "children"))
}

#[pyfunction]
fn scene_set_groups<'py>(me: &Obj<'py>, value: &Obj<'py>) -> PyResult<()> {
    groups_owner(me)?.setattr(intern!(me.py(), "children"), value)
}

fn root_property<'py>(py: Python<'py>, attr: &'static str) -> PyResult<Obj<'py>> {
    let get = PyCFunction::new_closure(py, None, None, move |args, _kw| -> PyResult<Py<PyAny>> {
        let me = args.get_item(0)?;
        Ok(root(&me)?.getattr(attr)?.unbind())
    })?;
    let put = PyCFunction::new_closure(py, None, None, move |args, _kw| -> PyResult<()> {
        let me = args.get_item(0)?;
        root(&me)?.setattr(attr, args.get_item(1)?)
    })?;
    py.import("builtins")?.getattr("property")?.call1((get, put))
}

/// A scene table kept on the scene's root node as a document attribute:
/// the tag folders and their tables, the active tag, plugin data. A dict or
/// a list reads as a tracked container that writes back when edited in
/// place (`core.doc_model`).
fn root_table<'py>(py: Python<'py>, name: &'static str) -> PyResult<Obj<'py>> {
    let track = name != "active_layer";
    let get = PyCFunction::new_closure(py, None, None, move |args, _kw| -> PyResult<Py<PyAny>> {
        let me = args.get_item(0)?;
        Ok(root(&me)?.call_method1(intern!(me.py(), "_attr"), (name, track))?.unbind())
    })?;
    let put = PyCFunction::new_closure(py, None, None, move |args, _kw| -> PyResult<()> {
        let me = args.get_item(0)?;
        root(&me)?.call_method1(intern!(me.py(), "_set_attr"), (name, args.get_item(1)?))?;
        Ok(())
    })?;
    let kw = PyDict::new(py);
    kw.set_item("doc", format!("``{name}``, kept in the document."))?;
    py.import("builtins")?.getattr("property")?.call((get, put), Some(&kw))
}

// ---------------------------------------------------------------- install

fn factory<'py>(py: Python<'py>, make: impl Fn(Python<'_>) -> PyResult<Py<PyAny>> + Send + Sync + 'static) -> PyResult<Obj<'py>> {
    Ok(PyCFunction::new_closure(py, None, None, move |args, _kw| make(args.py()))?.into_any())
}

fn build_scene<'py>(m: &Bound<'py, PyModule>) -> PyResult<Obj<'py>> {
    let py = m.py();
    let dc = py.import("dataclasses")?;
    let field = dc.getattr("field")?;
    let init_var = dc.getattr("InitVar")?;
    let builtins = py.import("builtins")?;
    let list = builtins.getattr("list")?;
    let dict = builtins.getattr("dict")?;
    let object = builtins.getattr("object")?;
    let mesh = py.import("ingetrazo_core.mesh")?.getattr("Mesh")?;
    let element_set = py.import("ingetrazo_core")?.getattr("ElementSet")?;
    let new_root = py.import("core.doc_model")?.getattr("new_scene_root")?;
    let fld = |kw: &[(&str, Obj<'py>)]| -> PyResult<Obj<'py>> {
        let d = PyDict::new(py);
        for (k, v) in kw {
            d.set_item(*k, v)?;
        }
        field.call((), Some(&d))
    };
    let f = pyo3::types::PyBool::new(py, false).to_owned().into_any();
    let t = pyo3::types::PyBool::new(py, true).to_owned().into_any();
    let zero = 0i64.into_pyobject(py)?.into_any();
    let nothing = none(py);
    let by = |factory: &Obj<'py>| fld(&[("default_factory", factory.clone())]);
    let units = factory(py, |py| {
        let d = PyDict::new(py);
        d.set_item("length", "m")?;
        d.set_item("precision", 2)?;
        Ok(d.into_any().unbind())
    })?;
    let dim_style = factory(py, |py| {
        let d = PyDict::new(py);
        d.set_item("decimals", 2)?;
        d.set_item("units", "m")?;
        d.set_item("font_size", 9)?;
        d.set_item("color", PyList::new(py, [45, 55, 75])?)?;
        d.set_item("norma", "iso")?;
        d.set_item("base_step_mm", 8.0)?;
        d.set_item("ends", "arrow")?;
        d.set_item("show_unit", true)?;
        Ok(d.into_any().unbind())
    })?;
    let style = m.getattr("_make_style")?;
    let shadows = m.getattr("_make_shadows")?;
    let animation = m.getattr("_make_animation")?;
    let opt_list = init_var.call1((&object,))?;
    let spec: Vec<(&str, Obj<'py>)> = vec![
        ("mesh", by(&mesh)?),
        ("selection", by(&element_set)?),
        ("version", zero.clone()),
        // How many of `version`'s bumps changed only what is SHOWN (the
        // selection) and not the document (issue #159).
        ("view_version", zero.clone()),
        ("_scene_catalog_ticks", fld(&[("default", zero.clone()), ("init", f.clone()), ("repr", f.clone())])?),
        // Context changes which preserve resident placement geometry.
        ("_placement_context_bumps", fld(&[("default", zero.clone()), ("init", f.clone()), ("repr", f.clone())])?),
        // How many of `version`'s bumps moved only the section planes.
        ("section_version", zero.clone()),
        ("_root", fld(&[("default_factory", new_root.clone()), ("repr", f.clone()), ("compare", f.clone())])?),
        ("groups", nothing.clone()),
        ("dimensions", by(&list)?),
        ("text_labels", by(&list)?),
        ("text_placement_anchors", by(&list)?),
        ("geo_paths", by(&list)?),
        ("geo_points", by(&list)?),
        ("guides", by(&list)?),
        ("image_planes", by(&list)?),
        ("layers", nothing.clone()),
        ("active_layer", nothing.clone()),
        ("layer_folders", by(&list)?),
        ("folder_parents", by(&dict)?),
        ("folder_visibility", by(&dict)?),
        ("layer_folder_uids", by(&dict)?),
        ("layer_folder_names", by(&dict)?),
        ("hatch_patterns", by(&dict)?),
        ("line_types", by(&dict)?),
        ("linetype_colors", by(&dict)?),
        ("bricks", by(&dict)?),
        ("materials", by(&dict)?),
        ("saved_views", by(&list)?),
        ("scene_folders", by(&list)?),
        ("scene_folder_parents", by(&dict)?),
        ("compositions", by(&list)?),
        ("custom_scales", by(&list)?),
        ("kept_components", by(&list)?),
        ("camera_home", nothing.clone()),
        ("views_home", nothing.clone()),
        ("units", by(&units)?),
        ("plugin_data", by(&dict)?),
        ("link_registry", by(&dict)?),
        ("dimension_style", by(&dim_style)?),
        ("back_face_color", nothing.clone()),
        ("display_style", by(&style)?),
        ("shadows", by(&shadows)?),
        ("animation", by(&animation)?),
        ("section_planes", by(&list)?),
        ("show_section_planes", t.clone()),
        ("show_hidden_objects", f.clone()),
        ("show_hidden_geometry", f.clone()),
        ("show_axes", t.clone()),
        ("north_deg", 0.0f64.into_pyobject(py)?.into_any()),
        ("hide_similar_components", f.clone()),
        ("show_section_cuts", t.clone()),
        ("georef", nothing.clone()),
        ("tile_layer", nothing.clone()),
        ("terrain", nothing.clone()),
        ("photo_mesh", nothing.clone()),
        ("active_ifc", nothing.clone()),
        // Group-edit context: while set, `mesh` POINTS AT the edited group's
        // mesh; `_loose_mesh` keeps the real loose mesh for render and
        // restore. `edit_group` is the INNERMOST open group.
        ("edit_group", nothing.clone()),
        ("_loose_mesh", nothing.clone()),
    ];
    let fields = PyList::empty(py);
    for (name, default) in spec {
        let kind = if name == "groups" || name == "layers" { opt_list.clone() } else { object.clone() };
        fields.append(PyTuple::new(py, [PyString::new(py, name).into_any(), kind, default])?)?;
    }
    let ns = PyDict::new(py);
    ns.set_item("__doc__", "Scene container, backed by a shared-vertex mesh (rust/ingetrazo_core/src/model/scene_api.rs).")?;
    super::py_class::fast_method(&ns, c"__setattr__", setattr_entry)?;
    property(&ns, "_edit_stack", wrap_pyfunction!(scene_edit_stack, m)?.into_any(), None)?;
    method(&ns, "__post_init__", wrap_pyfunction!(scene_post_init, m)?)?;
    method(&ns, "__deepcopy__", wrap_pyfunction!(scene_deepcopy, m)?)?;
    property(&ns, "edges", wrap_pyfunction!(scene_edges, m)?.into_any(), None)?;
    property(&ns, "faces", wrap_pyfunction!(scene_faces, m)?.into_any(), None)?;
    super::py_class::fast_method(&ns, c"layer_by_key", layer_by_key_entry)?;
    super::py_class::fast_method(&ns, c"layer", layer_entry)?;
    method(&ns, "_layer_state", wrap_pyfunction!(scene_layer_state, m)?)?;
    method(&ns, "_open_group_layer", wrap_pyfunction!(scene_open_group_layer, m)?)?;
    method(&ns, "open_group_paint", wrap_pyfunction!(scene_open_group_paint, m)?)?;
    static_method(&ns, "_object_hidden", wrap_pyfunction!(object_hidden, m)?.into_any())?;
    static_method(&ns, "_face_hidden", wrap_pyfunction!(face_hidden, m)?.into_any())?;
    method(&ns, "entity_hidden", wrap_pyfunction!(scene_entity_hidden, m)?)?;
    super::py_class::fast_method(&ns, c"entity_visible", visible_entry)?;
    method(&ns, "edge_snappable", wrap_pyfunction!(scene_edge_snappable, m)?)?;
    method(&ns, "group_snappable", wrap_pyfunction!(scene_group_snappable, m)?)?;
    method(&ns, "edit_definition", wrap_pyfunction!(scene_edit_definition, m)?)?;
    method(&ns, "similar_hidden", wrap_pyfunction!(scene_similar_hidden, m)?)?;
    super::py_class::fast_method(&ns, c"entity_selectable", selectable_entry)?;
    static_method(&ns, "entity_locked", wrap_pyfunction!(entity_locked, m)?.into_any())?;
    method(&ns, "navigator_key", wrap_pyfunction!(scene_navigator_key, m)?)?;
    method(&ns, "repeated_uids", wrap_pyfunction!(scene_repeated_uids, m)?)?;
    method(&ns, "occurrence_closure", wrap_pyfunction!(scene_occurrence_closure, m)?)?;
    method(&ns, "occurrence_path", wrap_pyfunction!(scene_occurrence_path, m)?)?;
    method(&ns, "resolve_occurrence", wrap_pyfunction!(scene_resolve_occurrence, m)?)?;
    method(&ns, "groups_by_uid", wrap_pyfunction!(scene_groups_by_uid, m)?)?;
    method(&ns, "active_section", wrap_pyfunction!(scene_active_section, m)?)?;
    method(&ns, "set_active_section", wrap_pyfunction!(scene_set_active_section, m)?)?;
    let rs = native(py)?;
    // The group-edit stack (`edit_context.rs`), `clear` (`scene_clear.rs`),
    // the boxes (`scene_bounds.rs`).
    for (name, native_name) in [
        ("begin_group_edit", "scene_begin_group_edit"),
        ("end_one_group_edit", "scene_end_one_group_edit"),
        ("end_group_edit", "scene_end_group_edit"),
        ("clear", "scene_clear"),
        ("selection_bounds", "scene_selection_bounds"),
        ("bounds", "scene_bounds"),
    ] {
        method(&ns, name, rs.getattr(native_name)?)?;
    }
    property(&ns, "drawing_frame", wrap_pyfunction!(scene_drawing_frame, m)?.into_any(), None)?;
    property(&ns, "loose_mesh", wrap_pyfunction!(scene_loose_mesh, m)?.into_any(), None)?;
    method(&ns, "placements", wrap_pyfunction!(scene_placements, m)?)?;
    method(&ns, "render_edges", wrap_pyfunction!(scene_render_edges, m)?)?;
    method(&ns, "render_faces", wrap_pyfunction!(scene_render_faces, m)?)?;
    method(&ns, "add_edge", wrap_pyfunction!(scene_add_edge, m)?)?;
    method(&ns, "select", wrap_pyfunction!(scene_select, m)?)?;
    method(&ns, "clear_selection", wrap_pyfunction!(scene_clear_selection, m)?)?;
    method(&ns, "bump_view", wrap_pyfunction!(scene_bump_view, m)?)?;
    method(&ns, "bump_sections", wrap_pyfunction!(scene_bump_sections, m)?)?;
    property(&ns, "geometry_version", wrap_pyfunction!(scene_geometry_version, m)?.into_any(), None)?;
    property(&ns, "revision", rs.getattr("scene_revision")?, None)?;
    property(&ns, "content_version", wrap_pyfunction!(scene_content_version, m)?.into_any(), None)?;
    method(&ns, "invert_selection", wrap_pyfunction!(scene_invert_selection, m)?)?;
    method(&ns, "_bounds_key", wrap_pyfunction!(scene_bounds_key, m)?)?;
    let kw = PyDict::new(py);
    kw.set_item("namespace", ns)?;
    kw.set_item("module", MODULE)?;
    let cls = dc.call_method("make_dataclass", ("Scene", fields), Some(&kw))?;
    // The document-held fields, set after the dataclass is made: its
    // `__init__` assigns their defaults through these.
    let pk = PyDict::new(py);
    pk.set_item("doc", "The root placements.")?;
    cls.setattr("groups", builtins.getattr("property")?.call((wrap_pyfunction!(scene_get_groups, m)?, wrap_pyfunction!(scene_set_groups, m)?), Some(&pk))?)?;
    let tags = root_property(py, "_tags")?;
    cls.setattr("layers", tags)?;
    for name in ROOT_TABLES {
        cls.setattr(name, root_table(py, name)?)?;
    }
    cls.setattr("materials", root_property(py, "_materials")?)?;
    cls.setattr("kept_components", root_property(py, "_kept")?)?;
    Ok(cls)
}

/// Fill the module `core.scene`.
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__doc__", "Scene container, backed by a shared-vertex mesh (rust/ingetrazo_core/src/model/scene_api.rs).")?;
    m.add("ROOT_TABLES", PyTuple::new(py, ROOT_TABLES)?)?;
    // The factories of the display style, shadows and animation a new scene
    // (and `clear`, `scene_clear.rs`) starts with.
    m.add("_make_style", factory(py, |py| Ok(py.import("ingetrazo_core.style")?.getattr("Style")?.call0()?.unbind()))?)?;
    m.add("_make_shadows", factory(py, |py| Ok(py.import("ingetrazo_core.sun")?.getattr("ShadowSettings")?.call0()?.unbind()))?)?;
    m.add("_make_animation", factory(py, |py| Ok(native(py)?.getattr("SceneAnimation")?.call0()?.unbind()))?)?;
    let scene = SCENE_CLASS.get_or_try_init(py, || -> PyResult<Py<PyAny>> { Ok(build_scene(m)?.unbind()) })?;
    m.add("Scene", scene.bind(py))?;
    Ok(())
}

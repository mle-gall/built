// SPDX-License-Identifier: GPL-3.0-or-later
//! `core.layers`, served from here (`core/layers.py` is gone): the layer /
//! tag system, visibility and locking.
//!
//! A layer never *owns* geometry. It is a label an entity carries: faces
//! via `attrs["layer"]`, edges via their `layer` slot, groups via `.layer`.
//! Hiding a layer removes its entities from render, picking and box
//! selection; locking keeps them visible but unpickable. Everything
//! unlabelled lives on the default layer, which always exists and cannot be
//! removed.
//!
//! This is what makes the '2D that emerges' workflow real. Structure,
//! walls, plumbing and furniture live on their own layers of one model, and
//! a top view in parallel projection with the right layers on IS the plan
//! drawing.
//!
//! What lives elsewhere:
//! - The styles are `tag_styles.rs`.
//! - The line types are `linetypes.rs`.
//! - The folders and display state are `tag_edits.rs` and
//!   `tag_folders.rs`.
//!
//! What is here is the `Layer` class, the per-entity rules (`layer_of`,
//! `assign_layer`), and the tagging of what a command made.

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyList, PySet, PyString, PyTuple};

use super::module;
use super::py_class::{class, method, FastAttr};

type Obj<'py> = Bound<'py, PyAny>;

const MODULE: &str = "core.layers";
pub const DEFAULT_LAYER: &str = "Layer 0";
/// Where an imported photogrammetric survey lands (Track G, G6). It gets
/// its own layer by default, not the default one. The whole point of
/// importing a survey is to draw on top of it, and you need to switch it
/// off to look at what you drew, which would take your model with it if
/// they shared a layer. A plain stored name like `DEFAULT_LAYER`, not a
/// translated one: layer names are document data and must not change with
/// the interface language.
const SURVEY_LAYER: &str = "Survey";
/// The key prefix of a line type the user made or brought in from a line
/// type file (`.lin`), kept in the document (`Scene.line_types`):
/// `"lin:<name>"`.
const CUSTOM_LINETYPE: &str = "lin:";
const CORE_TAG_FIELDS: [&str; 7] = ["uid", "name", "key", "visible", "locked", "color", "folder"];
const STYLE_FIELDS: [&str; 16] = [
    "linetype", "lineweight", "cut_weight", "hatch", "hatch_ground", "hatch_color", "hatch_angle", "hatch_scale",
    "hatch_offset", "hatch_on_paper", "brick", "hatch_cross", "joints", "mortar", "opacity", "printable",
];
/// The order `Layer.__init__` sets its fields in.
const INIT_ORDER: [&str; 23] = [
    "uid", "name", "key", "visible", "locked", "color", "folder", "linetype", "lineweight", "cut_weight", "hatch",
    "hatch_ground", "hatch_color", "hatch_angle", "hatch_scale", "hatch_offset", "hatch_on_paper", "brick",
    "hatch_cross", "joints", "mortar", "opacity", "printable",
];
/// The layers guides, dimensions and leader texts go to by themselves while
/// no layer is active: `(scene collection, English name)`. The name is
/// translated when the layer is made, so a Spanish document gets «Guías»,
/// «Cotas» and «Textos».
const AUTO_LAYERS: [(&str, &str); 3] = [("guides", "Guides"), ("dimensions", "Dimensions"), ("text_labels", "Texts")];
const OBJECT_LISTS: [&str; 3] = ["dimensions", "text_labels", "guides"];

static LAYER_CLASS: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
static TAGGABLE: PyOnceLock<Py<PyTuple>> = PyOnceLock::new();

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn native(py: Python<'_>) -> PyResult<Obj<'_>> {
    module(py, "ingetrazo_core")
}

fn layer_class(py: Python<'_>) -> PyResult<Obj<'_>> {
    match LAYER_CLASS.get(py) {
        Some(c) => Ok(c.bind(py).clone()),
        None => py.import(MODULE)?.getattr("Layer"),
    }
}

/// `getattr(obj, name, None) or ()` as a list of items.
fn items_of<'py>(obj: &Obj<'py>, name: &str) -> PyResult<Vec<Obj<'py>>> {
    match obj.attr_opt(name)? {
        Some(v) if !v.is_none() && v.is_truthy()? => v.try_iter()?.collect(),
        _ => Ok(Vec::new()),
    }
}

fn id(o: &Obj<'_>) -> usize {
    o.as_ptr() as usize
}

// ----------------------------------------------------------------- the tag

/// Whether `key` names a line type: a built-in one or a custom one.
#[pyfunction]
fn known_linetype(key: &Obj<'_>) -> PyResult<bool> {
    let py = key.py();
    if module(py, MODULE)?.getattr(intern!(py, "LINETYPES"))?.contains(key)? {
        return Ok(true);
    }
    Ok(match key.cast::<PyString>() {
        Ok(s) => {
            let s = s.to_str()?;
            s.starts_with(CUSTOM_LINETYPE) && s.chars().count() > CUSTOM_LINETYPE.len()
        }
        Err(_) => false,
    })
}

/// Whether anything beyond the name and visibility is set: what makes a
/// document worth writing its layer list for.
#[pyfunction]
fn layer_has_style(me: &Obj<'_>) -> PyResult<bool> {
    let py = me.py();
    let get = |n: &str| me.getattr(n);
    let set = |n: &str| -> PyResult<bool> { Ok(!get(n)?.is_none()) };
    let truthy = |n: &str| -> PyResult<bool> { get(n)?.is_truthy() };
    Ok(set("color")?
        || truthy("folder")?
        || get("linetype")?.ne("continuous")?
        || set("lineweight")?
        || get("opacity")?.lt(1.0)?
        || !truthy("printable")?
        || set("cut_weight")?
        || set("hatch")?
        || set("hatch_ground")?
        || set("hatch_color")?
        || truthy("hatch_angle")?
        || get("hatch_scale")?.sub(1.0)?.call_method0(intern!(py, "__abs__"))?.gt(1e-9)?
        || set("hatch_offset")?
        || set("hatch_on_paper")?
        || set("brick")?
        || truthy("hatch_cross")?
        || set("joints")?
        || truthy("mortar")?)
}

/// A tag from its stored form, read defensively (`file_styles.rs`).
#[pyfunction]
fn layer_from_dict<'py>(cls: &Obj<'py>, raw: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = cls.py();
    native(py)?.call_method1(intern!(py, "igz_load_layer"), (raw, cls))
}

fn tag_fields(py: Python<'_>) -> PyResult<Bound<'_, PyTuple>> {
    PyTuple::new(py, CORE_TAG_FIELDS.iter().chain(STYLE_FIELDS.iter()).collect::<Vec<_>>())
}

#[pyfunction]
fn layer_reduce<'py>(me: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = me.py();
    let state = PyDict::new(py);
    state.update(me.getattr(intern!(py, "__dict__"))?.cast::<PyDict>()?.as_mapping())?;
    state.update(super::doc_model_api::handle_state(me, tag_fields(py)?.as_any())?.as_mapping())?;
    let new = py.import("core.doc_model")?.getattr(intern!(py, "new_handle"))?;
    PyTuple::new(py, [new, PyTuple::new(py, [me.get_type()])?.into_any(), state.into_any()])
}

/// Every field of a tag by name: what `vars(ly)` gave when all of them
/// were instance attributes (in the order `__init__` sets them).
#[pyfunction]
fn layer_values<'py>(ly: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = ly.py();
    let out = PyDict::new(py);
    for name in INIT_ORDER {
        out.set_item(name, ly.getattr(name)?)?;
    }
    out.update(ly.getattr(intern!(py, "__dict__"))?.cast::<PyDict>()?.as_mapping())?;
    Ok(out)
}

// ---------------------------------------------------------- the line types

#[pyfunction]
fn text_offset<'py>(pattern: &Obj<'py>, at: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = pattern.py();
    let at = py.import("builtins")?.getattr("int")?.call1((at,))?;
    native(py)?.call_method1(intern!(py, "linetype_text_offset"), (pattern, at))
}

/// `segs` ((E, 2, 3) world) cut as the dash `pattern` (paper mm: dash, gap,
/// dash…; a dot is a dash of 0) at `unit` metres per paper mm, restarting
/// on each segment as a pen does: `(dashes, gaps)`, two (K, 2, 3) float32
/// arrays. A dot is drawn as a short dash. Too fine a pattern for a long
/// edge (over 400 periods) leaves it whole.
#[pyfunction]
#[pyo3(signature = (segs, pattern, unit=0.05))]
fn dash_segments<'py>(segs: &Obj<'py>, pattern: &Obj<'py>, unit: f64) -> PyResult<Obj<'py>> {
    let py = segs.py();
    let pieces = if pattern.is_truthy()? { PyTuple::new(py, pattern.try_iter()?.collect::<PyResult<Vec<_>>>()?)? } else { PyTuple::empty(py) };
    native(py)?.call_method1(intern!(py, "linetype_dash_segments"), (segs, pieces, unit))
}

fn as_tuple<'py>(v: Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(v.py(), v.try_iter()?.collect::<PyResult<Vec<_>>>()?)
}

/// Pieces as a line type file has them (dash > 0, gap < 0, dot 0) as a pen
/// draws them: dash, gap, dash, gap… in turn, all ≥ 0, starting with a
/// dash; two pieces of a kind in a row joined, a leading gap moved to the
/// end.
#[pyfunction]
fn drawn_dashes<'py>(pieces: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = pieces.py();
    as_tuple(native(py)?.call_method1(intern!(py, "linetype_drawn_dashes"), (pieces,))?)
}

/// `drawn_dashes` the other way: dash, −gap, … for a file.
#[pyfunction]
fn file_dashes<'py>(drawn: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = drawn.py();
    as_tuple(native(py)?.call_method1(intern!(py, "linetype_file_dashes"), (drawn,))?)
}

// ------------------------------------------------------ the entity's layer

/// The layer name an entity carries (default when unlabelled).
#[pyfunction]
pub fn layer_of<'py>(entity: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = entity.py();
    match layer_attrs_or_own(entity)? {
        Some(f) if f.is_truthy()? => Ok(f),
        _ => Ok(intern!(py, "Layer 0").clone().into_any()),
    }
}

/// `attrs["layer"]` for a face, the `layer` slot for anything else.
fn layer_attrs_or_own<'py>(entity: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = entity.py();
    Ok(match entity.attr_opt(intern!(py, "attrs"))? {
        // Face
        Some(attrs) if attrs.is_exact_instance_of::<PyDict>() => attrs.cast::<PyDict>()?.get_item(intern!(py, "layer"))?,
        Some(attrs) if !attrs.is_none() => Some(attrs.call_method1(intern!(py, "get"), (intern!(py, "layer"),))?),
        _ => entity.attr_opt(intern!(py, "layer"))?,
    })
}

fn is_default(name: &Obj<'_>) -> PyResult<bool> {
    name.eq(DEFAULT_LAYER)
}

/// Label an entity with a layer (default name clears the label).
#[pyfunction]
pub fn assign_layer<'py>(entity: &Obj<'py>, name: &Obj<'py>) -> PyResult<()> {
    let py = entity.py();
    let value = if is_default(name)? { none(py) } else { name.clone() };
    match entity.attr_opt(intern!(py, "attrs"))? {
        Some(attrs) if !attrs.is_none() => {
            // Face
            if value.is_none() {
                attrs.call_method1(intern!(py, "pop"), (intern!(py, "layer"), py.None()))?;
            } else {
                attrs.set_item(intern!(py, "layer"), value)?;
            }
        }
        _ => {
            if entity.has_attr(intern!(py, "layer"))? {
                entity.setattr(intern!(py, "layer"), value)?;
            }
        }
    }
    Ok(())
}

/// Give the tag `old_name` (its key, or its label) the label `new_name`.
/// Only the label changes: everything carries the tag's key, which stays,
/// so nothing is relabelled; geometry, kept definitions, the active tag and
/// the scenes that hide it all follow by themselves. Two tags may share a
/// label («Beams» in every folder).
///
/// Not an undo step: renaming is easy to redo by hand, and Ctrl+Z after
/// drawing should take back the drawing, not a name typed in between.
/// Returns False (and changes nothing) for the default layer, an empty
/// name, the default layer's own name, or a layer that does not exist.
#[pyfunction]
fn rename_layer<'py>(scene: &Obj<'py>, old_name: &Obj<'py>, new_name: &Obj<'py>) -> PyResult<bool> {
    let py = scene.py();
    let ly = scene.call_method1(intern!(py, "layer"), (old_name,))?;
    if ly.is_none()
        || ly.getattr(intern!(py, "key"))?.eq(DEFAULT_LAYER)?
        || !new_name.is_truthy()?
        || !new_name.call_method0(intern!(py, "strip"))?.is_truthy()?
        || is_default(new_name)?
    {
        return Ok(false);
    }
    ly.setattr(intern!(py, "name"), new_name)?;
    Ok(true)
}

/// The layer names in the order the Layers tab lists them with «Sort by
/// order created», folders unfolded: the default layer first, then each
/// folder and every layer out of one as they were made (folders before
/// layers, as the tab ranks them), with what a folder holds inside it, its
/// subfolders first (`tag_folders.rs`).
///
/// Ordering the Browser by layer follows this list, so the rows on the
/// layers of a folder stay together, in the folder's place. The model is
/// not touched: this is how the tab reads it.
#[pyfunction]
fn ordered_names<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let rows = PyList::empty(py);
    for ly in items_of(scene, "layers")? {
        rows.append((ly.getattr(intern!(py, "key"))?, ly.getattr(intern!(py, "folder"))?))?;
    }
    let te = native(py)?.getattr(intern!(py, "tag_edits"))?;
    let folders = te.call_method1(intern!(py, "folders_of"), (scene,))?;
    let parents = match scene.attr_opt(intern!(py, "folder_parents"))? {
        Some(p) if p.is_truthy()? => p,
        _ => PyDict::new(py).into_any(),
    };
    native(py)?.call_method1(intern!(py, "tag_ordered_keys"), (rows, folders, parents, DEFAULT_LAYER))
}

fn taggable_kinds(py: Python<'_>) -> PyResult<&Bound<'_, PyTuple>> {
    Ok(TAGGABLE
        .get_or_try_init(py, || -> PyResult<Py<PyTuple>> {
            let mesh = py.import("ingetrazo_core.mesh")?;
            let guide = py.import("ingetrazo_core.guide")?;
            let kinds = [
                mesh.getattr("Face")?,
                mesh.getattr("Edge")?,
                py.import("core.group")?.getattr("Group")?,
                py.import("core.dimension")?.getattr("Dimension")?,
                py.import("ingetrazo_core.textlabel")?.getattr("TextLabel")?,
                py.import("ingetrazo_core.section")?.getattr("SectionPlane")?,
                guide.getattr("Guide")?,
                guide.getattr("GuideCircle")?,
                py.import("ingetrazo_core.image_plane")?.getattr("ImagePlane")?,
            ];
            Ok(PyTuple::new(py, kinds)?.unbind())
        })?
        .bind(py))
}

/// The entities a layer is assigned to: faces, edges, groups and
/// components, dimensions, leader texts, section planes, guides and
/// reference images. The one list Entity Info, the right-click, the Layers
/// panel and its toolbar all ask.
#[pyfunction]
fn taggable<'py>(entities: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = entities.py();
    let out = PyList::empty(py);
    if entities.is_none() || !entities.is_truthy()? {
        return Ok(out);
    }
    let kinds = taggable_kinds(py)?;
    for e in entities.try_iter()? {
        let e = e?;
        if e.is_instance(kinds)? {
            out.append(e)?;
        }
    }
    Ok(out)
}

/// The layers what is selected sits on (empty with nothing selected): what
/// the Layers panel and the Layers toolbar highlight.
#[pyfunction]
fn selection_layers<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    let py = scene.py();
    let out = PySet::empty(py)?;
    let sel = scene.attr_opt(intern!(py, "selection"))?.unwrap_or_else(|| none(py));
    for e in taggable(&sel)?.iter() {
        out.add(layer_of(&e)?)?;
    }
    Ok(out)
}

fn iter_groups<'py>(groups: Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let py = groups.py();
    native(py)?.call_method1(intern!(py, "purge_iter_groups"), (groups,))?.try_iter()?.collect()
}

fn scene_groups<'py>(scene: &Obj<'py>, include_groups: bool) -> PyResult<Obj<'py>> {
    let py = scene.py();
    if !include_groups {
        return Ok(PyTuple::empty(py).into_any());
    }
    Ok(match scene.attr_opt(intern!(py, "groups"))? {
        Some(g) if !g.is_none() && g.is_truthy()? => g,
        _ => PyTuple::empty(py).into_any(),
    })
}

/// Identity of every object in the document (groups and components at any
/// depth, by `id` and by `uid`; dimensions, leader texts, guides). Compared
/// before and after a command to find what it created (`tag_new_objects`).
/// Geometry-only commands pass `include_groups=False`: no group can be
/// created by those edits.
#[pyfunction]
#[pyo3(signature = (scene, *, include_groups=true))]
fn object_ids<'py>(scene: &Obj<'py>, include_groups: bool) -> PyResult<Bound<'py, PySet>> {
    let py = scene.py();
    let ids = PySet::empty(py)?;
    for g in iter_groups(scene_groups(scene, include_groups)?)? {
        ids.add(id(&g))?;
        if let Some(uid) = g.attr_opt(intern!(py, "uid"))? {
            if uid.is_truthy()? {
                ids.add(uid)?;
            }
        }
    }
    for coll in OBJECT_LISTS {
        for o in items_of(scene, coll)? {
            ids.add(id(&o))?;
        }
    }
    Ok(ids)
}

/// Identity of the guides, dimensions and texts in the document: the cheap
/// «before» of a command run with no active layer (`tag_new_objects` then
/// only looks at those lists).
#[pyfunction]
fn auto_ids<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    let ids = PySet::empty(scene.py())?;
    for (coll, _) in AUTO_LAYERS {
        for o in items_of(scene, coll)? {
            ids.add(id(&o))?;
        }
    }
    Ok(ids)
}

/// The automatic layer for `english` («Guides», «Dimensions», «Texts») in
/// the interface language, made the first time it is needed. One the user
/// hid or locked hands back to the default layer: nothing new is ever drawn
/// straight into hiding, as with the active layer. «Guides» is born not
/// printed: guides never reach a sheet, and a printed layer would stand in
/// every sheet's layer legend.
#[pyfunction]
fn auto_layer<'py>(scene: &Obj<'py>, english: &str) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let name = py.import("ingetrazo_core.i18n")?.call_method1(intern!(py, "tr"), (english,))?;
    let ly = scene.call_method1(intern!(py, "layer"), (&name,))?;
    if ly.is_none() {
        let kw = PyDict::new(py);
        kw.set_item("printable", english != "Guides")?;
        let made = layer_class(py)?.call((&name,), Some(&kw))?;
        scene.getattr(intern!(py, "layers"))?.call_method1(intern!(py, "append"), (made,))?;
        return Ok(name);
    }
    if !ly.getattr(intern!(py, "visible"))?.is_truthy()? || ly.getattr(intern!(py, "locked"))?.is_truthy()? {
        return Ok(PyString::new(py, DEFAULT_LAYER).into_any());
    }
    ly.getattr(intern!(py, "key"))
}

/// With no layer active, the new guides go onto «Guides», the new
/// dimensions onto «Dimensions» and the new texts onto «Texts»
/// (scaffolding and annotation are what is switched off all at once);
/// everything else stays where it was.
fn tag_automatic<'py>(scene: &Obj<'py>, before: &Obj<'py>) -> PyResult<usize> {
    let mut n = 0;
    for (coll, english) in AUTO_LAYERS {
        let mut fresh = Vec::new();
        for o in items_of(scene, coll)? {
            if !before.contains(id(&o))? && is_default(&layer_of(&o)?)? {
                fresh.push(o);
            }
        }
        if fresh.is_empty() {
            continue;
        }
        let name = auto_layer(scene, english)?;
        if is_default(&name)? {
            continue;
        }
        for obj in fresh {
            assign_layer(&obj, &name)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Put the objects a command created on the active layer.
///
/// A group, a component, a dimension, a leader text or a guide made while a
/// layer is active goes onto it. Loose lines and faces stay on the default
/// layer (a layer hidden never leaves half a solid). Only the outermost new
/// object is tagged: a new group's own children follow it. One that already
/// carries a layer (a copy, a pasted object) keeps its own. With no layer
/// active, guides, dimensions and texts go to their automatic layers
/// (`AUTO_LAYERS`); `before` is then `auto_ids`. Returns how many objects
/// were tagged.
#[pyfunction]
#[pyo3(signature = (scene, before, *, include_groups=true, tagged_groups=None))]
fn tag_new_objects<'py>(scene: &Obj<'py>, before: &Obj<'py>, include_groups: bool, tagged_groups: Option<&Obj<'py>>) -> PyResult<usize> {
    let py = scene.py();
    let name = native(py)?.getattr(intern!(py, "tag_edits"))?.call_method1(intern!(py, "active_layer"), (scene,))?;
    if is_default(&name)? {
        return tag_automatic(scene, before);
    }
    // A copy that REPLACES an object (Make Unique, Replace Component…) keeps
    // its uid: that one is not new, and keeps the layer it had.
    let mut fresh = Vec::new();
    for g in iter_groups(scene_groups(scene, include_groups)?)? {
        let uid = g.attr_opt(intern!(py, "uid"))?.unwrap_or_else(|| none(py));
        if !before.contains(id(&g))? && !before.contains(uid)? {
            fresh.push(g);
        }
    }
    // Only the outermost new objects: a new group's own children keep the
    // default layer and follow their container.
    let mut inner = std::collections::HashSet::new();
    for g in &fresh {
        for c in items_of(g, "children")? {
            inner.insert(id(&c));
        }
    }
    fresh.retain(|g| !inner.contains(&id(g)));
    for coll in OBJECT_LISTS {
        for o in items_of(scene, coll)? {
            if !before.contains(id(&o))? {
                fresh.push(o);
            }
        }
    }
    let mut n = 0;
    for obj in fresh {
        if is_default(&layer_of(&obj)?)? {
            assign_layer(&obj, &name)?;
            if let Some(t) = tagged_groups.filter(|t| !t.is_none()) {
                if obj.has_attr(intern!(py, "mesh"))? {
                    t.call_method1(intern!(py, "append"), (&obj,))?;
                }
            }
            n += 1;
        }
    }
    Ok(n)
}

/// The layers whose timber is crossed where it is cut across its grain
/// (`Layer.hatch_cross`): for them a view measures how far each cut piece
/// runs through the plane.
#[pyfunction]
fn grain_layers<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = scene.py();
    let wood = native(py)?.getattr("hatches")?.getattr("WOOD")?;
    let out = PyList::empty(py);
    let layers = scene.attr_opt(intern!(py, "layers"))?.unwrap_or_else(|| PyTuple::empty(py).into_any());
    for ly in layers.try_iter()? {
        let ly = ly?;
        let cross = ly.attr_opt(intern!(py, "hatch_cross"))?.map(|c| c.is_truthy()).transpose()?.unwrap_or(false);
        if cross && wood.contains(ly.attr_opt(intern!(py, "hatch"))?.unwrap_or_else(|| none(py)))? {
            out.append(ly.getattr(intern!(py, "key"))?)?;
        }
    }
    Ok(out)
}

/// What kind of cut a view of `scene` shows: `"plan"` (the active section
/// plane lies flat), `"vertical"` (it stands), None (no section). Masonry
/// is drawn one way in a plan, another in a section.
#[pyfunction]
fn section_cut_kind<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    if !scene.has_attr(intern!(py, "active_section"))? {
        return Ok(none(py));
    }
    let plane = scene.call_method0(intern!(py, "active_section"))?;
    if plane.is_none() {
        return Ok(none(py));
    }
    let z: f64 = plane.getattr(intern!(py, "normal"))?.call_method0(intern!(py, "z"))?.extract()?;
    Ok(PyString::new(py, if z.abs() >= 0.7 { "plan" } else { "vertical" }).into_any())
}

/// Fill the module `core.layers`.
pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__doc__", "Layer / tag system: visibility and locking (rust/ingetrazo_core/src/model/layers_api.rs).")?;
    let rs = native(py)?;
    let dm = py.import("core.doc_model")?;
    m.add("_rs", &rs)?;
    m.add("_dm", &dm)?;
    m.add("new_uid", py.import("core.group")?.getattr("new_uid")?)?;
    m.add("PALETTE", rs.getattr("TAG_PALETTE")?)?;
    m.add("automatic_tag_color", rs.getattr("automatic_tag_color")?)?;
    m.add("DEFAULT_LAYER", DEFAULT_LAYER)?;
    m.add("SURVEY_LAYER", SURVEY_LAYER)?;
    // The default layer's own colour: the plain model white, so what nobody
    // sorted onto a layer reads as "not classified".
    m.add("DEFAULT_LAYER_COLOR", (0.96, 0.95, 0.925))?;
    // Line types a layer can ink its edges with, and their dash pattern in
    // millimetres on paper (dash, gap, …; empty = continuous), with their
    // names in a DXF (`linetypes.rs`).
    let linetypes = PyDict::new(py);
    let dxf = PyDict::new(py);
    for row in rs.call_method0("linetype_table")?.try_iter()? {
        let row = row?;
        let key = row.get_item(0)?;
        linetypes.set_item(&key, as_tuple(row.get_item(1)?)?)?;
        dxf.set_item(&key, row.get_item(2)?)?;
    }
    m.add("LINETYPES", linetypes)?;
    m.add("DXF_LINETYPES", dxf)?;
    m.add("CUSTOM_LINETYPE", CUSTOM_LINETYPE)?;
    // The model draws a line type's dashes as a sheet at 1:50 does: metres
    // of model per paper millimetre.
    m.add("MODEL_DASH_SCALE", 0.05)?;
    // How much a locked layer's faces wash toward white: drawn, but plainly
    // not in play.
    m.add("LOCKED_FADE", 0.55)?;
    m.add("CORE_TAG_FIELDS", PyTuple::new(py, CORE_TAG_FIELDS)?)?;
    m.add("STYLE_FIELDS", PyTuple::new(py, STYLE_FIELDS)?)?;
    m.add("TAG_FIELDS", tag_fields(py)?)?;
    m.add("_INIT_ORDER", PyTuple::new(py, INIT_ORDER)?)?;
    // `Layer.hatch_ground` / `hatch_color` for «the layer's own colour»
    // (light, for a ground; as it is, for the ink): by layer.
    m.add("GROUND_LAYER", "layer")?;
    // The tag a linked file's untagged content wears in the host
    // (`link_tag_name`): «<file> / Untagged». It stands for the source's
    // default tag, so for colour it reads as none.
    m.add("LINK_UNTAGGED_SUFFIX", " / Untagged")?;
    // The key a layer template file is recognised by.
    m.add("TEMPLATE_KEY", "ingetrazo_layers")?;
    m.add("AUTO_LAYERS", PyTuple::new(py, AUTO_LAYERS)?)?;

    // The styles, the folder queries and the active tag that plugins and
    // scripting use, by their Python names.
    let styles = rs.getattr("tag_styles")?;
    m.add("legend_rows", styles.getattr("legend_rows")?)?;
    m.add("unprinted", styles.getattr("unprinted")?)?;
    let te = rs.getattr("tag_edits")?;
    m.add("_te", &te)?;
    m.add("folders_of", te.getattr("folders_of")?)?;
    m.add("folder_parent", te.getattr("folder_parent")?)?;
    m.add("layers_in_folder", te.getattr("layers_in_folder")?)?;
    m.add("active_layer", te.getattr("active_layer")?)?;
    m.add("subfolders", te.getattr("tag_scene_subfolders")?)?;
    for (name, native_name) in [
        ("text_width", "linetype_text_width"),
        ("clean_line_text", "linetype_clean_line_text"),
        ("with_text", "linetype_with_text"),
        ("text_marks", "linetype_text_marks"),
        ("clean_pattern", "linetype_clean_pattern"),
        ("parse_lin", "linetype_parse_lin"),
        ("to_lin", "linetype_to_lin"),
    ] {
        m.add(name, rs.getattr(native_name)?)?;
    }
    m.add_function(wrap_pyfunction!(known_linetype, m)?)?;
    m.add_function(wrap_pyfunction!(text_offset, m)?)?;
    m.add_function(wrap_pyfunction!(dash_segments, m)?)?;
    m.add_function(wrap_pyfunction!(drawn_dashes, m)?)?;
    m.add_function(wrap_pyfunction!(file_dashes, m)?)?;
    m.add_function(wrap_pyfunction!(layer_values, m)?)?;
    m.add_function(wrap_pyfunction!(layer_of, m)?)?;
    m.add_function(wrap_pyfunction!(assign_layer, m)?)?;
    m.add_function(wrap_pyfunction!(rename_layer, m)?)?;
    m.add_function(wrap_pyfunction!(ordered_names, m)?)?;
    m.add_function(wrap_pyfunction!(taggable, m)?)?;
    m.add_function(wrap_pyfunction!(selection_layers, m)?)?;
    m.add_function(wrap_pyfunction!(object_ids, m)?)?;
    m.add_function(wrap_pyfunction!(auto_ids, m)?)?;
    m.add_function(wrap_pyfunction!(auto_layer, m)?)?;
    m.add_function(wrap_pyfunction!(tag_new_objects, m)?)?;
    m.add_function(wrap_pyfunction!(grain_layers, m)?)?;
    m.add_function(wrap_pyfunction!(section_cut_kind, m)?)?;

    let layer = LAYER_CLASS.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            "A tag with display state.\n\nEntities carry the tag's ``key`` (an identity that never changes),\nand ``name`` is only its label: free to repeat (twenty «Beams», one per\nfolder) and to rename without relabelling anything (#tags-by-key). The\nkey defaults to the name the tag is made with, so a document written\nbefore keys existed (where names were the identity) reads unchanged.\n\n``color`` is an RGB tuple (0-1) or ``None`` for the automatic one;\n``folder`` is the KEY of the folder the layer is filed under in the\nLayers panel, or ``None``. Folder visibility gates the tag without\nchanging its own visibility flag.",
        )?;
        // `Layer(name, visible=True, locked=False, color=None, folder=None,
        // linetype="continuous", lineweight=None, opacity=1.0,
        // printable=True, uid=None, key=None, cut_weight=None, hatch=None,
        // …)`: every value cleaned (`tag_styles.rs: tag_init`).
        method(&ns, "__init__", styles.getattr("tag_init")?)?;
        method(&ns, "to_dict", rs.getattr("igz_archive_layer")?)?;
        method(&ns, "has_style", wrap_pyfunction!(layer_has_style, m)?)?;
        ns.set_item("from_dict", py.import("builtins")?.getattr("classmethod")?.call1((wrap_pyfunction!(layer_from_dict, m)?,))?)?;
        method(&ns, "__reduce__", wrap_pyfunction!(layer_reduce, m)?)?;
        ns.set_item("__setstate__", dm.getattr("set_handle_state")?)?;
        let cls = class(py, MODULE, "Layer", &[rs.getattr("Tag")?], &ns)?;
        for name in STYLE_FIELDS {
            let kw = PyDict::new(py);
            kw.set_item("track", false)?;
            cls.setattr(name, dm.getattr("attribute")?.call((name,), Some(&kw))?)?;
        }
        Ok(cls.unbind())
    })?;
    m.add("Layer", layer.bind(py))?;
    Ok(())
}

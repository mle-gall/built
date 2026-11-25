// SPDX-License-Identifier: GPL-3.0-or-later
//! What the scene's rules answer for every face and edge of one mesh at once
//! (`Scene.entity_visible`, `entity_selectable` and `edge_snappable`), for
//! the loops that asked them one element at a time: the model's box, the
//! pick index, the snap. The answers are read from the store (a face's Hide
//! and tag from its attribute row, an edge's from its flags and tag) and the
//! tags' state is worked out once per call, not once per element.
//!
//! The rules for a face or an edge, as `core/scene.py` has them: hidden by
//! its own Hide (faces only); its tag, or while a group is open and it has
//! none, the open group's (`_open_group_layer`, the open group's own mesh
//! only); a tag shows when it and its folders do; selectable when it shows,
//! its tag is not locked, its Hide is shown, and the context's loose
//! geometry is not locked. An element the store does not know (a stale
//! view, an object of another kind) is asked through the scene itself.
//! Pinned by `tests/test_scene_visibility_parity.py`.

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::store::keys::Kind;
use crate::store::py::PyMeshStore;
use crate::store::{Store, HIDDEN};

/// Tag key → (shown, locked); a tag the scene does not know shows, unlocked.
struct Tags(HashMap<String, (bool, bool)>);

impl Tags {
    fn read(py: Python<'_>, scene: &Bound<'_, PyAny>) -> PyResult<Tags> {
        let effective = super::module(py, "built_core")?.getattr("tag_edits")?.getattr("effective_visible")?;
        let mut out = HashMap::new();
        for ly in scene.getattr("layers")?.try_iter()? {
            let ly = ly?;
            let Ok(key) = ly.getattr("key")?.extract::<String>() else { continue };
            // `layer_by_key` answers the first tag with a key.
            out.entry(key).or_insert((effective.call1((scene, &ly))?.is_truthy()?, ly.getattr("locked")?.is_truthy()?));
        }
        Ok(Tags(out))
    }
    fn state(&self, key: &str) -> (bool, bool) {
        self.0.get(key).copied().unwrap_or((true, false))
    }
}

/// A face's (Hide, tag) from its own dict when it has one, else its row.
fn face_look(own: Option<&Bound<'_, PyDict>>, table: Option<&Bound<'_, PyList>>, store: &Store, f: u32, key: u128)
    -> PyResult<(bool, Option<String>)> {
    let row: Option<Bound<'_, PyAny>> = match own.map(|o| o.get_item(key)).transpose()?.flatten() {
        Some(d) => Some(d),
        None => {
            let r = *store.state.fattr.get(f as usize);
            if r == 0 { None } else { table.map(|t| t.get_item(r as usize - 1)).transpose()? }
        }
    };
    let Some(d) = row.filter(|d| d.is_truthy().unwrap_or(false)) else { return Ok((false, None)) };
    let hidden = d.call_method1("get", ("hidden",))?.is_truthy()?;
    let layer = d.call_method1("get", ("layer",))?;
    Ok((hidden, if layer.is_truthy()? { Some(layer.extract()?) } else { None }))
}

/// `scene_mesh_states(scene, mesh, faces, edges)`: for `faces` (views of
/// `mesh`) the lists `(visible, selectable)`, and for `edges`
/// `(visible, snappable, selectable)`, as the scene's rules answer one by one.
#[pyfunction]
#[allow(clippy::type_complexity)]
pub(crate) fn scene_mesh_states(py: Python<'_>, scene: &Bound<'_, PyAny>, mesh: &Bound<'_, PyAny>, faces: &Bound<'_, PyAny>,
                     edges: &Bound<'_, PyAny>) -> PyResult<((Vec<bool>, Vec<bool>), (Vec<bool>, Vec<bool>, Vec<bool>))> {
    let tags = Tags::read(py, scene)?;
    let reveal = scene.getattr("show_hidden_geometry")?.is_truthy()?;
    let loose_locked = scene.getattr("_root")?.call_method0("_loose_locked")?.is_truthy()?;
    let default = super::module(py, "core.layers")?.getattr("DEFAULT_LAYER")?.extract::<String>()?;
    // The tag an untagged element follows while its group is open, for the
    // elements whose key the open group's mesh (`scene.mesh`) holds: the
    // rule asks by key (`element_identity`), and a prototype's elements
    // share their keys with the world copy an open instance edits.
    let open_tag: Option<String> = if !scene.getattr("edit_group")?.is_none() {
        let mut tag = None;
        let path = super::module(py, "core.outliner")?.getattr("open_path")?.call1((scene,))?;
        let path: Vec<Bound<'_, PyAny>> = path.try_iter()?.collect::<PyResult<_>>()?;
        for g in path.iter().rev() {
            let l = g.getattr_opt("layer")?;
            if let Some(l) = l.filter(|l| l.is_truthy().unwrap_or(false)) {
                tag = Some(l.extract()?);
                break;
            }
        }
        tag
    } else {
        None
    };
    let open_keys: std::collections::HashSet<u128> = match &open_tag {
        None => Default::default(),
        Some(_) => {
            let open = scene.getattr("mesh")?;
            let mut keys = std::collections::HashSet::new();
            for kind in ["faces", "edges"] {
                for e in open.getattr(kind)?.try_iter()? {
                    if let Ok(k) = e?.getattr("key").and_then(|k| k.extract::<u128>()) {
                        keys.insert(k);
                    }
                }
            }
            keys
        }
    };
    let tag_state = |own: Option<String>, key: u128| -> (bool, bool) {
        let name = own.filter(|t| !t.is_empty()).unwrap_or_else(|| default.clone());
        let name = match &open_tag {
            Some(t) if name == default && open_keys.contains(&key) => t.clone(),
            _ => name,
        };
        tags.state(&name)
    };
    let dict = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    let store_obj = dict.get_item("_store")?;
    let owner = match dict.get_item("_instance_definition")? {
        Some(o) => o.getattr("__dict__")?.cast_into::<PyDict>()?,
        None => dict.clone(),
    };
    let own = owner.get_item("_attr_own")?.and_then(|o| o.cast_into::<PyDict>().ok());
    let table = owner.get_item("_attr_table")?.and_then(|t| t.cast_into::<PyList>().ok());
    let store_ref = match &store_obj {
        Some(s) => Some(s.cast::<PyMeshStore>()?.try_borrow()?),
        None => None,
    };
    let store: Option<&Store> = store_ref.as_ref().map(|s| &s.inner);
    let key_of = |e: &Bound<'_, PyAny>| -> Option<u128> {
        let m = e.getattr_opt("_m").ok().flatten()?;
        if !m.is(mesh) {
            return None;
        }
        e.getattr("key").ok()?.extract().ok()
    };
    let (mut fv, mut fs) = (Vec::new(), Vec::new());
    for f in faces.try_iter()? {
        let f = f?;
        let slot = match (store, key_of(&f)) {
            (Some(st), Some(k)) => st.slot(Kind::Face, k).map(|s| (st, s, k)),
            _ => None,
        };
        let Some((st, s, k)) = slot else {
            fv.push(scene.call_method1("entity_visible", (&f,))?.is_truthy()?);
            fs.push(scene.call_method1("entity_selectable", (&f,))?.is_truthy()?);
            continue;
        };
        let (hidden, tag) = face_look(own.as_ref(), table.as_ref(), st, s, k)?;
        let (shown, locked) = tag_state(tag, k);
        fv.push(!hidden && shown);
        fs.push(!(hidden && !reveal) && !loose_locked && shown && !locked);
    }
    let (mut ev, mut es, mut esel) = (Vec::new(), Vec::new(), Vec::new());
    for e in edges.try_iter()? {
        let e = e?;
        let slot = match (store, key_of(&e)) {
            (Some(st), Some(k)) => st.slot(Kind::Edge, k).map(|s| (st, s, k)),
            _ => None,
        };
        let Some((st, s, k)) = slot else {
            ev.push(scene.call_method1("entity_visible", (&e,))?.is_truthy()?);
            es.push(scene.call_method1("edge_snappable", (&e,))?.is_truthy()?);
            esel.push(scene.call_method1("entity_selectable", (&e,))?.is_truthy()?);
            continue;
        };
        let flags = *st.state.eflags.get(s as usize);
        let l = *st.state.elayer.get(s as usize);
        let tag = if l == crate::store::keys::NONE { None } else { Some(st.state.layers[l as usize].clone()) };
        let (shown, locked) = tag_state(tag, k);
        ev.push(shown);
        es.push(shown && (flags & HIDDEN == 0 || reveal));
        esel.push(!loose_locked && shown && !locked);
    }
    Ok(((fv, fs), (ev, es, esel)))
}

/// What the scene state that decides placements is, read once per batch:
/// the navigator's allowed set, Hide Similar and the definition being
/// edited, Hide shown.
struct PlacementRules<'py> {
    allowed: Option<Bound<'py, PyAny>>,
    similar: Option<(Bound<'py, PyAny>, Bound<'py, PyAny>)>,
    show_hidden: bool,
    group_type: Bound<'py, PyAny>,
}

impl<'py> PlacementRules<'py> {
    fn read(py: Python<'py>, scene: &Bound<'py, PyAny>) -> PyResult<Self> {
        let allowed = scene.getattr_opt("_navigator_allowed")?.filter(|a| !a.is_none());
        let similar = if scene.getattr("hide_similar_components")?.is_truthy()? && scene.getattr("_edit_stack")?.is_truthy()? {
            let def = scene.call_method0("edit_definition")?;
            if def.is_none() { None } else { Some((scene.getattr("edit_group")?, def)) }
        } else {
            None
        };
        Ok(PlacementRules {
            allowed,
            similar,
            show_hidden: scene.getattr("show_hidden_objects")?.is_truthy()?,
            group_type: super::module(py, "core.group")?.getattr("Group")?,
        })
    }

    /// `similar_hidden(g)`.
    fn similar(&self, g: &Bound<'py, PyAny>) -> PyResult<bool> {
        let Some((edit, def)) = &self.similar else { return Ok(false) };
        if !g.hasattr("children")? || g.is(edit) {
            return Ok(false);
        }
        for x in [Some(g.clone()), g.getattr_opt("owner")?.filter(|o| !o.is_none())].into_iter().flatten() {
            if !x.is(edit) && x.getattr_opt("xform")?.is_some_and(|m| !m.is_none())
                && x.getattr_opt("mesh")?.is_some_and(|m| m.is(def)) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// `scene_group_states(scene, groups)`: for each placement of `groups` the
/// triple `(entity_visible, group_snappable, entity_selectable)`, as the
/// scene's rules answer one by one, the tags and the context read once.
#[pyfunction]
pub(crate) fn scene_group_states(py: Python<'_>, scene: &Bound<'_, PyAny>, groups: &Bound<'_, PyAny>)
    -> PyResult<Vec<(bool, bool, bool)>> {
    let tags = Tags::read(py, scene)?;
    let rules = PlacementRules::read(py, scene)?;
    let default = super::module(py, "core.layers")?.getattr("DEFAULT_LAYER")?.extract::<String>()?;
    let mut out = Vec::new();
    for g in groups.try_iter()? {
        let g = g?;
        let out_of_view = match &rules.allowed {
            Some(a) if g.is_instance(&rules.group_type)? => !a.contains(scene.call_method1("navigator_key", (&g,))?)?,
            _ => false,
        };
        if out_of_view {
            out.push((false, false, false));
            continue;
        }
        let has_children = g.hasattr("children")?;
        if !has_children || g.hasattr("attrs")? || g.hasattr("v0")? {
            // Not a placement: the scene's rules, one by one.
            let v = scene.call_method1("entity_visible", (&g,))?.is_truthy()?;
            let s = scene.call_method1("entity_selectable", (&g,))?.is_truthy()?;
            out.push((v, v || s, s));
            continue;
        }
        let hidden = g.getattr_opt("hidden")?.map(|h| h.is_truthy()).transpose()?.unwrap_or(false);
        let similar = rules.similar(&g)?;
        let tag = g.getattr_opt("layer")?.filter(|l| l.is_truthy().unwrap_or(false));
        let name = match tag { Some(t) => t.extract::<String>()?, None => default.clone() };
        let (shown, tag_locked) = tags.state(&name);
        let mut locked = false;
        for x in [Some(g.clone()), g.getattr_opt("owner")?.filter(|o| !o.is_none())].into_iter().flatten() {
            if x.getattr_opt("locked")?.is_some_and(|l| l.is_truthy().unwrap_or(false)) && x.hasattr("children")? {
                locked = true;
            }
        }
        let visible = !hidden && !similar && shown;
        let selectable = !similar && !(hidden && !rules.show_hidden) && !locked && shown && !tag_locked;
        out.push((visible, visible || selectable, selectable));
    }
    Ok(out)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scene_mesh_states, m)?)?;
    m.add_function(wrap_pyfunction!(scene_group_states, m)?)?;
    Ok(())
}

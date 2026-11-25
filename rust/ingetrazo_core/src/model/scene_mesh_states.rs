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
        let effective = super::module(py, "ingetrazo_core")?.getattr("tag_edits")?.getattr("effective_visible")?;
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

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scene_mesh_states, m)?)?;
    Ok(())
}

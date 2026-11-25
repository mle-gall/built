// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! What a vector sheet frame draws, read from the document and the mesh
//! stores (docs/rust-sheets.md step 4): world-space triangles, hard edges
//! and soft edges with their faces' normals, each with the tag it is inked
//! with. It is `core.hlr.collect_geometry` under the frame's saved view,
//! without the view ever being applied to the live scene:
//!
//! * the loose geometry: a face shows when it is not hidden and its tag
//!   shows; an edge when its tag shows and it is not a hidden edge;
//! * each top-level placement the frame shows (not Hide, tag shown, not a
//!   billboard), with the nested placements `visible_placements` keeps (a
//!   hidden subtree stays out, an untagged child takes the nearest tag
//!   above, a hidden or locked tag forces its tag on everything below),
//!   each through its world matrix; inside them only Hide on a face counts;
//! * a line's tag is its own, else its top-level object's.
//!
//! Face attributes (tag, Hide) live in the mesh's Python attribute table;
//! they are read once per table row, not per face.

use super::visibility::{own_hidden, Tags, ViewState, DEFAULT_LAYER};
use crate::doc::{self, value::Value, Data, Key};
use crate::store::geometry::{face_normal, triangles, Hypot, V3};
use crate::store::keys::Kind;
use crate::store::py::PyMeshStore;
use crate::store::{Store, HIDDEN, SOFT};
use glam::{DMat4, DVec3};
use numpy::{PyArray1, PyArrayMethods};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::collections::HashMap;

/// One mesh to read: its geometry, world matrix, owner tag, and whether it
/// is the loose geometry (whose faces and edges answer to their tags).
struct Part {
    geometry: Py<PyAny>,
    world: Option<DMat4>,
    owner: String,
    loose: bool,
}

#[derive(Default)]
pub struct Collected {
    pub tris: Vec<[[f64; 3]; 3]>,
    pub hard: Vec<[[f64; 3]; 2]>,
    pub soft: Vec<[[f64; 3]; 2]>,
    pub soft_n: Vec<[[f64; 3]; 2]>,
    pub tri_tags: Vec<String>,
    pub hard_tags: Vec<String>,
    pub soft_tags: Vec<String>,
}

fn tag_of(t: Option<&str>) -> Option<String> {
    t.filter(|s| !s.is_empty()).map(str::to_string)
}

fn matrix(p: &doc::PlacementData) -> Option<DMat4> {
    p.xform.as_ref().map(|x| DMat4::from_cols_array(&x.doubles()))
}

fn billboard(p: &doc::PlacementData) -> bool {
    match p.billboard.as_deref() {
        None | Some(Value::None) | Some(Value::Bool(false)) | Some(Value::Int(0)) => false,
        Some(_) => true,
    }
}

/// The parts the frame draws, under the document lock (no Python calls).
fn parts(py: Python<'_>, root: Key, roots: &[Key], view: &ViewState) -> PyResult<(Vec<Part>, Tags)> {
    let d = doc::lock();
    let tags = Tags::read(&d, root, view)?;
    let mut out = Vec::new();
    #[allow(clippy::too_many_arguments)]
    fn walk(py: Python<'_>, d: &doc::Doc, tags: &Tags, view: &ViewState, node: &doc::Node,
            node_tag: Option<String>, world: Option<DMat4>, hidden: bool, forced: Option<String>,
            inherited: Option<String>, owner: &str, out: &mut Vec<Part>) {
        for &c in &node.children {
            let Some(cn) = d.node(c) else { continue };
            let Data::Placement(cp) = &cn.data else { continue };
            let m = match (world, matrix(cp)) {
                (None, x) => x,
                (Some(w), Some(x)) => Some(w * x),
                (Some(w), None) => Some(w),
            };
            let layer = forced.clone().or_else(|| tag_of(cp.tag.as_deref()))
                .or_else(|| inherited.clone()).or_else(|| node_tag.clone());
            let off = hidden || own_hidden(cp, view);
            if !off && layer.as_deref().is_none_or(|l| tags.shown(l)) {
                if let Some(def) = &cp.definition {
                    let geometry = def.0.bind(py).get().geometry.clone_ref(py);
                    out.push(Part { geometry, world: m, owner: owner.to_string(), loose: false });
                }
            }
            if !cn.children.is_empty() {
                let kid_forced = forced.clone().or_else(|| layer.clone().filter(|l| tags.blocking(l)));
                walk(py, d, tags, view, cn, tag_of(cp.tag.as_deref()), m, off, kid_forced, layer, owner, out);
            }
        }
    }
    for &k in roots {
        let Some(n) = d.node(k) else { continue };
        let Data::Placement(p) = &n.data else { continue };
        let tag = tag_of(p.tag.as_deref());
        let owner = tag.clone().unwrap_or_else(|| DEFAULT_LAYER.to_string());
        if own_hidden(p, view) || !tags.shown(&owner) || billboard(p) {
            continue;
        }
        let world = matrix(p);
        if let Some(def) = &p.definition {
            let geometry = def.0.bind(py).get().geometry.clone_ref(py);
            out.push(Part { geometry, world, owner: owner.clone(), loose: false });
        }
        if !n.children.is_empty() {
            let forced = tag.clone().filter(|l| tags.blocking(l));
            walk(py, &d, &tags, view, n, tag, world, false, forced, None, &owner, &mut out);
        }
    }
    Ok((out, tags))
}

/// A face's (Hide, tag) per attribute row: the face's own dict when it has
/// one, else its table row.
struct Attrs<'py> {
    own: Option<Bound<'py, PyDict>>,
    table: Option<Bound<'py, PyList>>,
    rows: HashMap<u32, (bool, Option<String>)>,
}

fn look(d: &Bound<'_, PyAny>) -> PyResult<(bool, Option<String>)> {
    let hidden = match d.get_item("hidden") {
        Ok(v) => v.is_truthy()?,
        Err(_) => false,
    };
    let layer = match d.get_item("layer") {
        Ok(v) if v.is_truthy()? => Some(v.extract::<String>()?),
        _ => None,
    };
    Ok((hidden, layer))
}

impl<'py> Attrs<'py> {
    fn of(mesh: &Bound<'py, PyAny>) -> PyResult<Attrs<'py>> {
        let dict = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
        let owner = match dict.get_item("_instance_definition")? {
            Some(o) => o.getattr("__dict__")?.cast_into::<PyDict>()?,
            None => dict,
        };
        Ok(Attrs {
            own: owner.get_item("_attr_own")?.and_then(|o| o.cast_into::<PyDict>().ok()),
            table: owner.get_item("_attr_table")?.and_then(|t| t.cast_into::<PyList>().ok()),
            rows: HashMap::new(),
        })
    }
    fn face(&mut self, store: &Store, f: u32) -> PyResult<(bool, Option<String>)> {
        if let Some(own) = &self.own {
            if !own.is_empty() {
                if let Some(d) = own.get_item(store.key(Kind::Face, f))? {
                    return look(&d);
                }
            }
        }
        let row = *store.state.fattr.get(f as usize);
        if row == 0 {
            return Ok((false, None));
        }
        if let Some(hit) = self.rows.get(&row) {
            return Ok(hit.clone());
        }
        let got = match &self.table {
            Some(t) => match t.get_item(row as usize - 1) {
                Ok(d) => look(&d)?,
                Err(_) => (false, None),
            },
            None => (false, None),
        };
        self.rows.insert(row, got.clone());
        Ok(got)
    }
}

fn arr(p: DVec3) -> [f64; 3] {
    p.to_array()
}

fn eat(py: Python<'_>, part: &Part, tags: &Tags, h: Hypot, out: &mut Collected) -> PyResult<()> {
    let mesh = part.geometry.bind(py);
    let dict = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    let Some(store) = dict.get_item("_store")? else {
        return Err(PyValueError::new_err("a mesh without a native store"));
    };
    let store = store.cast::<PyMeshStore>()?.try_borrow()?;
    let store: &Store = &store.inner;
    let mut attrs = Attrs::of(mesh)?;
    let st = &store.state;
    let map = |p: &[f64; 3]| -> DVec3 {
        let v = DVec3::from_array(*p);
        match part.world {
            Some(m) => m.transform_point3(v),
            None => v,
        }
    };
    let mirror = part.world.is_some_and(|m| m.determinant() < 0.0);
    let tag_for = |own: Option<String>| -> String {
        match own {
            Some(t) if t != DEFAULT_LAYER => t,
            _ => part.owner.clone(),
        }
    };
    // Faces: shown, their world rings and normals (for the soft edges).
    let faces = store.live_slots(Kind::Face);
    let mut shown: HashMap<u32, Option<[f64; 3]>> = HashMap::with_capacity(faces.len());
    for f in faces {
        let (hide, layer) = attrs.face(store, f)?;
        let visible = !hide && (!part.loose || tags.shown(layer.as_deref().unwrap_or(DEFAULT_LAYER)));
        if !visible {
            continue;
        }
        let rings = store.face_rings(f);
        if rings.first().is_none_or(|r| r.len() < 3) {
            shown.insert(f, None);
            continue;
        }
        let mut pos = Vec::new();
        // A mirrored copy runs its rings backwards (core.group._ring), so
        // its front stays the front.
        let local: Vec<Vec<u32>> = rings.iter().map(|ring| {
            let mut r: Vec<u32> = ring.iter().map(|&v| {
                let w = map(st.vpos.get(v as usize));
                pos.push(V3(w.x, w.y, w.z));
                (pos.len() - 1) as u32
            }).collect();
            if mirror {
                r.reverse();
            }
            r
        }).collect();
        let refs: Vec<&[u32]> = local.iter().map(Vec::as_slice).collect();
        let n = face_normal(&pos, refs[0], h);
        let tag = tag_for(layer);
        for t in triangles(&pos, &refs, n, h) {
            out.tris.push(t.map(|i| {
                let V3(x, y, z) = pos[i as usize];
                [x, y, z]
            }));
            out.tri_tags.push(tag.clone());
        }
        shown.insert(f, Some([n.0, n.1, n.2]));
    }
    let nan = [f64::NAN; 3];
    for e in store.live_slots(Kind::Edge) {
        let flags = *st.eflags.get(e as usize);
        let li = *st.elayer.get(e as usize);
        let own = st.layers.get(li as usize).cloned();
        if part.loose && !tags.shown(own.as_deref().unwrap_or(DEFAULT_LAYER)) {
            continue;
        }
        if flags & HIDDEN != 0 {
            continue;
        }
        let [a, b] = *st.ends.get(e as usize);
        let seg = [arr(map(st.vpos.get(a as usize))), arr(map(st.vpos.get(b as usize)))];
        let tag = tag_for(own);
        if flags & SOFT != 0 {
            let fs: Vec<[f64; 3]> = store.edge_faces(e).iter()
                .filter(|f| shown.contains_key(f))
                .map(|f| shown[f].unwrap_or([0.0, 0.0, 1.0]))
                .collect();
            let Some(&na) = fs.first() else { continue };
            out.soft.push(seg);
            out.soft_n.push([na, fs.get(1).copied().unwrap_or(nan)]);
            out.soft_tags.push(tag);
        } else {
            out.hard.push(seg);
            out.hard_tags.push(tag);
        }
    }
    Ok(())
}

/// Everything the frame draws: the loose mesh (``loose``), then the parts.
pub fn collect(py: Python<'_>, root: Key, roots: &[Key], loose: Option<Py<PyAny>>, view: &ViewState,
               h: Hypot) -> PyResult<Collected> {
    let (mut list, tags) = parts(py, root, roots, view)?;
    if let Some(geometry) = loose {
        list.insert(0, Part { geometry, world: None, owner: DEFAULT_LAYER.to_string(), loose: true });
    }
    let mut out = Collected::default();
    for part in &list {
        eat(py, part, &tags, h, &mut out)?;
    }
    Ok(out)
}

type Arrays<'py> = (Bound<'py, PyAny>, Bound<'py, PyAny>, Bound<'py, PyAny>, Bound<'py, PyAny>);

fn stack<'py, const N: usize>(py: Python<'py>, rows: Vec<[[f64; 3]; N]>) -> PyResult<Bound<'py, PyAny>> {
    let n = rows.len();
    Ok(PyArray1::from_vec(py, rows.into_iter().flatten().flatten().collect()).reshape([n, N, 3])?.into_any())
}

/// ``core.hlr.collect_geometry`` for a sheet frame, as the arrays
/// ``DocumentInteraction.hlr_geometry`` hands out: ``(tris (T,3,3),
/// hard (E,2,3), soft (S,2,3), soft_n (S,2,3))`` (``soft_n[:, 1]`` NaN for
/// a soft edge with one face), plus with ``tags`` the tag of each
/// triangle, hard and soft edge as three lists. ``root`` the scene root,
/// ``roots`` its top level, ``loose`` the loose mesh, ``view`` the frame's
/// saved view or None, ``hypot`` as ``core._native.qt_hypot()``.
#[pyfunction]
#[pyo3(signature = (root, roots, loose, view=None, hypot="libc++-fma", tags=false))]
#[allow(clippy::type_complexity)]
pub fn sheet_frame_geometry<'py>(py: Python<'py>, root: &Bound<'py, PyAny>, roots: Vec<Bound<'py, PyAny>>,
                                 loose: Option<Bound<'py, PyAny>>, view: Option<Bound<'py, PyAny>>,
                                 hypot: &str, tags: bool)
    -> PyResult<(Arrays<'py>, Option<(Vec<String>, Vec<String>, Vec<String>)>)> {
    py.import("core.doc_model")?.call_method0("flush")?;
    let state = match view.filter(|v| !v.is_none()) {
        Some(v) => ViewState::of_view(&super::model::read_fields(doc::py::handle_key(&v)?)?),
        None => ViewState::default(),
    };
    let root = doc::py::handle_key(root)?;
    let roots: Vec<Key> = roots.iter().map(doc::py::handle_key).collect::<PyResult<_>>()?;
    let h = crate::hypot_of(hypot)?;
    let c = collect(py, root, &roots, loose.filter(|l| !l.is_none()).map(Bound::unbind), &state, h)?;
    let arrays = (stack(py, c.tris)?, stack(py, c.hard)?, stack(py, c.soft)?, stack(py, c.soft_n)?);
    Ok((arrays, tags.then_some((c.tri_tags, c.hard_tags, c.soft_tags))))
}

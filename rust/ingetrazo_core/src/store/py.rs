// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The Python face of the store: `ingetrazo_core.MeshStore` and
//! `ingetrazo_core.StoreSnapshot`.
//!
//! Arrays come back in list order (the `lazy_mesh.Plan` layout). Keys
//! are Python ints (`namespace << 40 | local`); array readers return the
//! 40-bit local parts with `key_base` to OR in, so a million keys never
//! become a million Python ints unless asked. Construction and the bulk
//! readers run detached from the GIL; single-element ops do not (a
//! release/reacquire costs more than the op).

use super::build::Block;
use super::edit::Change;
use super::keys::Kind;
use super::stitch::{Merge, Plan};
use super::topo::OpError;
use super::{Snapshot, Store, Weld};
use std::sync::{Arc, Mutex};
use numpy::ndarray::Array2;
use numpy::{IntoPyArray, PyArray1, PyArray2, PyReadonlyArray1, PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::exceptions::{PyAssertionError, PyException, PyKeyError, PyOverflowError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

pyo3::create_exception!(
    ingetrazo_core,
    StoreFallback,
    PyException,
    "An op the store does not represent (see store/topo.rs): nothing changed; run the Python kernel."
);

#[pyclass(module = "ingetrazo_core", name = "MeshStore")]
pub struct PyMeshStore {
    pub inner: Store,
    /// Per kind, `(revision, live slots in list order, slot -> index)`:
    /// `mesh.faces[i]` / `.index(f)` in a loop must not rebuild the list
    /// order per call once tombstones or a relink made it non-trivial.
    /// Every mutation bumps the revision, so a stale entry is never read.
    cache: Mutex<[Option<OrderCache>; 3]>,
}

/// `(revision, live slots in list order, slot -> list index)`.
type OrderCache = (u64, Arc<Vec<u32>>, Arc<Vec<u32>>);

impl PyMeshStore {
    fn wrap(inner: Store) -> Self {
        Self { inner, cache: Mutex::new(Default::default()) }
    }

    fn order_of(&self, k: Kind) -> (Arc<Vec<u32>>, Arc<Vec<u32>>) {
        let mut c = self.cache.lock().unwrap();
        let rev = self.inner.revision;
        if let Some((r, live, dense)) = &c[k as usize] {
            if *r == rev {
                return (live.clone(), dense.clone());
            }
        }
        let st = &self.inner.state;
        let live = st.order[k as usize].live(&st.slots[k as usize]);
        let mut dense = vec![super::keys::NONE; st.slots[k as usize].len()];
        for (i, &s) in live.iter().enumerate() {
            dense[s as usize] = i as u32;
        }
        let (live, dense) = (Arc::new(live), Arc::new(dense));
        c[k as usize] = Some((rev, live.clone(), dense.clone()));
        (live, dense)
    }

    /// Run an op with a fresh change record; the revision is bumped even
    /// when the op fails half way (a vertex made before a NaN), so the
    /// caches above never see a changed store under an old revision.
    fn op<T>(&mut self, f: impl FnOnce(&mut Store, &mut Change) -> Result<T, OpError>) -> PyResult<(T, PyChange)> {
        let mut ch = Change::default();
        let r = f(&mut self.inner, &mut ch);
        let ch = self.inner.finish(ch);
        match r {
            Ok(v) => Ok((v, PyChange { inner: ch })),
            Err(e) => Err(op_err(e)),
        }
    }
}

impl PyMeshStore {
    /// Carry out a dissolve plan: the hole filter first when the merged
    /// face has several holes (Python's `add_face` runs `_maximal_holes`
    /// then), then the removals and the new face. `(face, removed faces,
    /// bumps)`.
    #[allow(clippy::type_complexity)]
    fn merge(
        &mut self,
        plan: Plan,
        hole_filter: &Bound<'_, PyAny>,
    ) -> PyResult<(Option<(u128, Vec<u128>, usize)>, PyChange)> {
        match plan {
            Plan::No => {
                let ch = self.inner.finish(Change::default());
                Ok((None, PyChange { inner: ch }))
            }
            Plan::Stacked(fs) => {
                let ((), ch) = self.op_all(|s, ch| {
                    for &f in &fs[1..] {
                        s.remove_face(f, ch);
                    }
                    Ok(())
                })?;
                let s = &self.inner;
                let removed = fs[1..].iter().map(|&f| s.key(Kind::Face, f)).collect();
                Ok((Some((s.key(Kind::Face, fs[0]), removed, fs.len() - 1)), ch))
            }
            Plan::Merge(m) => {
                let keep = self.hole_mask(&m, hole_filter)?;
                let removed: Vec<u128> = m.faces.iter().map(|&f| self.inner.key(Kind::Face, f)).collect();
                let ((f, n), ch) = self.op_all(|s, ch| s.apply_merge(&m, &keep, ch))?;
                Ok((Some((self.inner.key(Kind::Face, f), removed, n)), ch))
            }
        }
    }

    fn hole_mask(&self, m: &Merge, hole_filter: &Bound<'_, PyAny>) -> PyResult<Vec<bool>> {
        if m.holes.len() <= 1 {
            return Ok(Vec::new());
        }
        let st = &self.inner.state;
        let rings: Vec<Vec<(f64, f64, f64)>> = m
            .holes
            .iter()
            .map(|h| {
                h.iter()
                    .map(|&v| {
                        let p = st.vpos.get(v as usize);
                        (p[0], p[1], p[2])
                    })
                    .collect()
            })
            .collect();
        let keep: Vec<bool> = match hole_filter.call1((rings,)).and_then(|r| r.extract()) {
            Ok(k) => k,
            Err(_) => return Err(StoreFallback::new_err("add_face's hole filter raised")),
        };
        if keep.len() != m.holes.len() {
            return Err(PyValueError::new_err("hole_filter: one bool per hole"));
        }
        Ok(keep)
    }
}

impl PyMeshStore {
    /// `op` for the ops that remove faces on the way (welds, dedupe,
    /// dissolves): all or nothing. A refusal half way, or a removed face
    /// left listed on an edge (`Store::dangling`), puts the state back as
    /// it was (O(chunks)) and raises StoreFallback.
    fn op_all<T>(&mut self, f: impl FnOnce(&mut Store, &mut Change) -> Result<T, OpError>) -> PyResult<(T, PyChange)> {
        self.op(|s, ch| {
            let saved = s.state.clone();
            let mut r = f(s, ch);
            if r.is_ok() {
                let gone: Vec<u32> = ch.removed[2].iter().filter_map(|&k| s.slot_any(Kind::Face, k)).collect();
                if s.dangling(&gone) {
                    r = Err(OpError::Fallback("a removed face would stay listed on an edge"));
                }
            }
            if r.is_err() {
                s.state = saved;
                *ch = Change::default();
            }
            r
        })
    }
}

fn op_err(e: OpError) -> PyErr {
    match e {
        OpError::Nan => PyValueError::new_err("cannot convert float NaN to integer"),
        OpError::Inf => PyOverflowError::new_err("cannot convert float infinity to integer"),
        OpError::Degenerate => PyValueError::new_err("degenerate edge: endpoints weld to one vertex"),
        OpError::Full => PyValueError::new_err("more elements than a store has slots"),
        OpError::Fallback(why) => StoreFallback::new_err(why),
    }
}

/// A key's slot, live or a tombstone of this state (removed elements
/// stay readable for history: their loops, ends and faces).
fn any_of(st: &Store, kind: Kind, key: u128) -> PyResult<u32> {
    st.slot_any(kind, key).ok_or_else(|| PyKeyError::new_err(key))
}

fn live_of(st: &Store, kind: Kind, key: u128) -> PyResult<u32> {
    st.slot(kind, key).ok_or_else(|| PyKeyError::new_err(key))
}

fn p3(p: (f64, f64, f64)) -> [f64; 3] {
    [p.0, p.1, p.2]
}

#[pyclass(module = "ingetrazo_core", name = "StoreSnapshot", frozen)]
pub struct PyStoreSnapshot {
    pub inner: Snapshot,
}

fn kind_of(s: &str) -> PyResult<Kind> {
    Kind::parse(s).ok_or_else(|| PyValueError::new_err(format!("kind {s:?}: vertex, edge or face")))
}

fn slice<'a, T: numpy::Element>(a: &'a PyReadonlyArray1<'_, T>, name: &str) -> PyResult<&'a [T]> {
    a.as_slice().map_err(|_| PyValueError::new_err(format!("{name}: a contiguous 1-D array")))
}

/// Read-only preview. Slots are usable only against the originating store
/// revision. The facade resolves material dictionaries from source face keys.
#[pyclass(module = "ingetrazo_core", name = "StoreFilletPlan", frozen)]
struct PyRoundPlan {
    inner: super::fillet::RoundPlan,
    lineage: u64,
    revision: u64,
    hypot: crate::facegeom::Hypot,
    faces: Vec<u128>,
    edges: Vec<u128>,
    flags: Vec<(u128, [f64; 3], [f64; 3])>,
    materials: Vec<u128>,
}
type Points = Vec<[f64; 3]>;
fn points(ps: &[crate::facegeom::V3]) -> Points {
    ps.iter().copied().map(super::fillet::xyz).collect()
}

#[pymethods]
impl PyRoundPlan {
    #[getter]
    fn strips(&self) -> Vec<Points> {
        self.inner.strips.iter().map(|r| points(r)).collect()
    }
    #[getter]
    fn patches(&self) -> Vec<Points> {
        self.inner.patches.iter().map(|r| points(r)).collect()
    }
    #[getter]
    fn curves(&self) -> Vec<Points> {
        self.inner.curves.iter().map(|r| points(r)).collect()
    }
    #[getter]
    fn rebuilt(&self) -> Vec<Vec<Points>> {
        self.inner
            .rebuilt
            .iter()
            .map(|(_, rs)| rs.iter().map(|r| points(r)).collect())
            .collect()
    }
    #[getter]
    fn soft(&self) -> Vec<[[f64; 3]; 2]> {
        self.inner
            .soft
            .iter()
            .map(|&(a, b)| [a, b].map(super::fillet::xyz))
            .collect()
    }
    #[getter]
    fn faces(&self) -> Vec<u128> {
        self.faces.clone()
    }
    #[getter]
    fn edges(&self) -> Vec<u128> {
        self.edges.clone()
    }
    #[getter]
    fn flags(&self) -> Vec<(u128, [f64; 3], [f64; 3])> {
        self.flags.clone()
    }
    #[getter]
    fn materials(&self) -> Vec<u128> {
        self.materials.clone()
    }
}
fn round_preview(
    s: &Store,
    result: super::fillet::Result<super::fillet::RoundPlan>,
    hypot: crate::facegeom::Hypot,
) -> PyResult<PyRoundPlan> {
    let mut inner = result
        .map_err(|(message, p)| PyValueError::new_err((message, p.map(super::fillet::xyz))))?;
    inner.filter_nested_holes(hypot);
    let faces = inner
        .rebuilt
        .iter()
        .map(|(f, _)| s.key(Kind::Face, *f))
        .collect();
    let materials = inner
        .materials
        .iter()
        .map(|&f| s.key(Kind::Face, f))
        .collect();
    let edges = inner.edges.iter().map(|&e| s.key(Kind::Edge, e)).collect();
    let flags = inner
        .flags
        .iter()
        .map(|&(e, a, b)| {
            (
                s.key(Kind::Edge, e),
                super::fillet::xyz(a),
                super::fillet::xyz(b),
            )
        })
        .collect();
    Ok(PyRoundPlan {
        inner,
        faces,
        materials,
        edges,
        flags,
        lineage: s.lineage,
        revision: s.revision,
        hypot,
    })
}

/// A change record (`edit::Change`) as Python sees it: built in one
/// allocation; the key lists become Python lists only when read.
#[pyclass(module = "ingetrazo_core", name = "StoreChange", frozen)]
pub struct PyChange {
    inner: Change,
}

#[pymethods]
impl PyChange {
    #[getter]
    fn revision(&self) -> u64 {
        self.inner.revision
    }

    /// Vertex keys whose position changed.
    #[getter]
    fn moved(&self) -> Vec<u128> {
        self.inner.moved.clone()
    }

    /// `{kind: [key, ...]}` of new elements.
    #[getter]
    fn created<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        by_kind(py, &self.inner.created)
    }

    #[getter]
    fn removed<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        by_kind(py, &self.inner.removed)
    }

    #[getter]
    fn modified<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        by_kind(py, &self.inner.modified)
    }
}

fn by_kind<'py>(py: Python<'py>, lists: &[Vec<u128>; 3]) -> PyResult<Bound<'py, PyDict>> {
    let k = PyDict::new(py);
    for kind in Kind::ALL {
        k.set_item(kind.name(), PyList::new(py, &lists[kind as usize])?)?;
    }
    Ok(k)
}

fn rows3<'py, T: numpy::Element + Copy>(py: Python<'py>, rows: &[[T; 3]]) -> Bound<'py, PyArray2<T>> {
    let flat: Vec<T> = rows.iter().flat_map(|r| r.iter().copied()).collect();
    Array2::from_shape_vec((rows.len(), 3), flat).unwrap().into_pyarray(py)
}

#[pymethods]
impl PyMeshStore {
    /// An empty store in key namespace `namespace` (`next(_KEY_NAMESPACES)`).
    #[new]
    #[pyo3(signature = (namespace, scale = 10_000.0, tol2 = 1e-8, fused = false))]
    fn new(namespace: u64, scale: f64, tol2: f64, fused: bool) -> Self {
        Self::wrap(Store::empty(namespace, Weld { scale, tol2, fused }))
    }

    /// The store a format-3 block builds — `Mesh.bulk_weld` +
    /// `add_edges_welded` + `add_faces_welded` on an empty mesh, equal to
    /// `lazy_mesh.plan(...)` index for index. Arrays as `formats/igz.py`
    /// keeps them in a deferred mesh's source (`pos` float32 (P, 3),
    /// `edges` int64 (E, 2), `flags` uint8, `curves` / `layers` /
    /// `face_attr` int64 with −1 for none).
    #[staticmethod]
    #[pyo3(signature = (namespace, pos, edges, flags, curves, layers, layer_names, rings, sizes,
                        counts, face_attr = None, sequential = false, scale = 10_000.0,
                        tol2 = 1e-8, fused = false))]
    #[allow(clippy::too_many_arguments)]
    fn from_block(
        py: Python<'_>,
        namespace: u64,
        pos: PyReadonlyArray2<'_, f32>,
        edges: PyReadonlyArray2<'_, i64>,
        flags: Option<PyReadonlyArray1<'_, u8>>,
        curves: Option<PyReadonlyArray1<'_, i64>>,
        layers: Option<PyReadonlyArray1<'_, i64>>,
        layer_names: Vec<String>,
        rings: PyReadonlyArray1<'_, i64>,
        sizes: PyReadonlyArray1<'_, i64>,
        counts: PyReadonlyArray1<'_, i64>,
        face_attr: Option<PyReadonlyArray1<'_, i64>>,
        sequential: bool,
        scale: f64,
        tol2: f64,
        fused: bool,
    ) -> PyResult<Self> {
        if pos.ndim() != 2 || (pos.shape()[0] > 0 && pos.shape()[1] != 3) {
            return Err(PyValueError::new_err("pos: (P, 3) float32"));
        }
        if edges.ndim() != 2 || (edges.shape()[0] > 0 && edges.shape()[1] != 2) {
            return Err(PyValueError::new_err("edges: (E, 2) int64"));
        }
        let pflat = pos.as_slice().map_err(|_| PyValueError::new_err("pos: contiguous"))?;
        let eflat = edges.as_slice().map_err(|_| PyValueError::new_err("edges: contiguous"))?;
        // SAFETY: [f32; 3] / [i64; 2] have the alignment of their element
        // and the arrays are C-contiguous with exactly these row widths.
        let pos3: &[[f32; 3]] =
            unsafe { std::slice::from_raw_parts(pflat.as_ptr() as *const [f32; 3], pflat.len() / 3) };
        let edges2: &[[i64; 2]] =
            unsafe { std::slice::from_raw_parts(eflat.as_ptr() as *const [i64; 2], eflat.len() / 2) };
        let block = Block {
            pos: pos3,
            edges: edges2,
            flags: flags.as_ref().map(|a| slice(a, "flags")).transpose()?,
            curves: curves.as_ref().map(|a| slice(a, "curves")).transpose()?,
            layers: layers.as_ref().map(|a| slice(a, "layers")).transpose()?,
            layer_names,
            rings: slice(&rings, "rings")?,
            sizes: slice(&sizes, "sizes")?,
            counts: slice(&counts, "counts")?,
            face_attr: face_attr.as_ref().map(|a| slice(a, "face_attr")).transpose()?,
            sequential,
        };
        let w = Weld { scale, tol2, fused };
        let store = py.detach(|| Store::from_block(namespace, &block, w)).map_err(PyValueError::new_err)?;
        Ok(Self::wrap(store))
    }

    #[getter]
    fn namespace(&self) -> u64 {
        self.inner.state.ns
    }

    /// `namespace << 40`: OR it into a local key array for the ints.
    #[getter]
    fn key_base(&self) -> u128 {
        (self.inner.state.ns as u128) << super::keys::LOCAL_BITS
    }

    #[getter]
    fn revision(&self) -> u64 {
        self.inner.revision
    }

    /// Float32 semantics (QVector3D parity). Turning it off is the
    /// post-port switch to true float64; tests only for now.
    #[getter]
    fn get_f32_parity(&self) -> bool {
        self.inner.f32_parity
    }

    #[setter]
    fn set_f32_parity(&mut self, on: bool) {
        self.inner.f32_parity = on;
    }

    /// `(vertices, edges, faces)` live.
    fn counts(&self) -> (usize, usize, usize) {
        (self.inner.n(Kind::Vertex), self.inner.n(Kind::Edge), self.inner.n(Kind::Face))
    }

    /// Every array of the `Plan` at once, as a dict: pos, edges, flags,
    /// curve, layer, corners, ring_sizes, ring_counts, ef_start, ef_face,
    /// face_attr (one remap pass for all of them).
    fn arrays<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = py.detach(|| self.inner.dense());
        dense_dict(py, d)
    }

    /// Which snapshots are this store's own (`restore` keeps their keys).
    #[getter]
    fn lineage(&self) -> u64 {
        self.inner.lineage
    }

    /// Live keys of `kind` in list order, as Python ints (the facade's
    /// `mesh.faces` iteration).
    fn keys(&self, kind: &str) -> PyResult<Vec<u128>> {
        let k = kind_of(kind)?;
        let (live, _) = self.order_of(k);
        Ok(live.iter().map(|&s| self.inner.key(k, s)).collect())
    }

    /// `Vertex.position` as a float64 triple (holding float32 values).
    fn vertex_position(&self, key: u128) -> PyResult<(f64, f64, f64)> {
        let v = any_of(&self.inner, Kind::Vertex, key)?;
        let p = self.inner.state.vpos.get(v as usize);
        Ok((p[0], p[1], p[2]))
    }

    /// `Vertex.edges` as keys (disk-cycle order: most recently linked
    /// first; Python's is a set).
    fn vertex_edges(&self, key: u128) -> PyResult<Vec<u128>> {
        let v = any_of(&self.inner, Kind::Vertex, key)?;
        Ok(self.inner.edges_at(v).map(|e| self.inner.key(Kind::Edge, e)).collect())
    }

    /// `(v0, v1, flags, curve (-1 none), layer name or None)` of an edge.
    fn edge(&self, key: u128) -> PyResult<(u128, u128, u8, i64, Option<String>)> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        let st = &self.inner.state;
        let [a, b] = *st.ends.get(e as usize);
        let l = *st.elayer.get(e as usize);
        Ok((
            self.inner.key(Kind::Vertex, a),
            self.inner.key(Kind::Vertex, b),
            *st.eflags.get(e as usize),
            *st.ecurve.get(e as usize),
            if l == super::keys::NONE { None } else { Some(st.layers[l as usize].clone()) },
        ))
    }

    /// `(v0, v1)` keys of an edge.
    fn edge_ends(&self, key: u128) -> PyResult<(u128, u128)> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        let [a, b] = *self.inner.state.ends.get(e as usize);
        Ok((self.inner.key(Kind::Vertex, a), self.inner.key(Kind::Vertex, b)))
    }

    /// `Edge.faces` as keys, in order, with multiplicity.
    fn edge_faces(&self, key: u128) -> PyResult<Vec<u128>> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        Ok(self.inner.edge_faces(e).iter().map(|&f| self.inner.key(Kind::Face, f)).collect())
    }

    /// A face's rings as vertex keys (outer first, then holes).
    fn face_rings(&self, key: u128) -> PyResult<Vec<Vec<u128>>> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        Ok(self
            .inner
            .face_rings(f)
            .into_iter()
            .map(|r| r.into_iter().map(|v| self.inner.key(Kind::Vertex, v)).collect())
            .collect())
    }

    /// A face's rings as positions: what `normal()` / `area()` /
    /// `vertices` read, in one call.
    #[pyo3(signature = (key, hypot, normal=None))]
    fn face_triangles(&self, key: u128, hypot: &str, normal: Option<[f32;3]>) -> PyResult<Vec<[[f32;3];3]>> {
        let store = &self.inner;
        let face = slot_of(store, Kind::Face, key)?;
        Ok(store.face_triangles(face, normal, crate::hypot_of(hypot)?))
    }

    fn face_positions(&self, key: u128) -> PyResult<Vec<Vec<(f64, f64, f64)>>> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        let st = &self.inner.state;
        Ok(self
            .inner
            .face_rings(f)
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|v| {
                        let p = st.vpos.get(v as usize);
                        (p[0], p[1], p[2])
                    })
                    .collect()
            })
            .collect())
    }

    /// `(attribute-table index or -1, face flags)`.
    fn face_info(&self, key: u128) -> PyResult<(i64, u8)> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        let st = &self.inner.state;
        Ok((*st.fattr.get(f as usize) as i64 - 1, *st.fflags.get(f as usize)))
    }

    /// `Mesh.find_edge(v0, v1)`: the edge key, or None.
    fn find_edge(&self, a: u128, b: u128) -> PyResult<Option<u128>> {
        let va = any_of(&self.inner, Kind::Vertex, a)?;
        let vb = any_of(&self.inner, Kind::Vertex, b)?;
        Ok(self.inner.find_edge(va, vb).map(|e| self.inner.key(Kind::Edge, e)))
    }

    /// Keys of the faces `fold_nonplanar_faces` would fold, list order.
    fn nonplanar_faces(&self, py: Python<'_>, tolerance: f64) -> Vec<u128> {
        let st = &self.inner;
        let slots = py.detach(|| st.nonplanar_faces(tolerance));
        slots.into_iter().map(|f| st.key(Kind::Face, f)).collect()
    }

    /// `Edge.soft` / `hidden` / `flat` as format-3 bits (soft 1, hidden 2,
    /// flat 4).
    fn set_edge_flags(&mut self, key: u128, flags: u8) -> PyResult<u64> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        Ok(self.inner.set_edge_flags(e, flags))
    }

    /// `Edge.curve` (None: no curve).
    fn set_edge_curve(&mut self, key: u128, curve: Option<i64>) -> PyResult<u64> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        Ok(self.inner.set_edge_curve(e, curve.unwrap_or(-1)))
    }

    /// `Edge.layer` (a tag key, or None).
    fn set_edge_layer(&mut self, key: u128, layer: Option<String>) -> PyResult<u64> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        Ok(self.inner.set_edge_layer(e, layer.as_deref()))
    }

    /// A face's attribute-table index (-1: no attrs).
    fn set_face_attr(&mut self, key: u128, index: i64) -> PyResult<u64> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        if index < -1 || index >= u32::MAX as i64 {
            return Err(PyValueError::new_err("attribute index out of range"));
        }
        Ok(self.inner.set_face_attr(f, (index + 1) as u32))
    }

    /// A face's flags (interior = 1).
    fn set_face_flags(&mut self, key: u128, flags: u8) -> PyResult<u64> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        Ok(self.inner.set_face_flags(f, flags))
    }

    /// Vertex positions (V, 3) float64, list order.
    fn positions<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray2<f64>> {
        let st = &self.inner.state;
        let rows: Vec<[f64; 3]> = st.order[0].live(&st.slots[0]).into_iter().map(|v| *st.vpos.get(v as usize)).collect();
        rows3(py, &rows)
    }

    fn layer_names(&self) -> Vec<String> {
        self.inner.state.layers.as_ref().clone()
    }

    /// Live keys of `kind` in list order, 40-bit local parts (uint64).
    fn local_keys<'py>(&self, py: Python<'py>, kind: &str) -> PyResult<Bound<'py, PyArray1<u64>>> {
        Ok(self.inner.local_keys(kind_of(kind)?).into_pyarray(py))
    }

    /// The key of the `index`-th element of `kind` in list order.
    fn key_at(&self, kind: &str, index: usize) -> PyResult<u128> {
        let k = kind_of(kind)?;
        let st = &self.inner.state;
        let (order, slots) = (&st.order[k as usize], &st.slots[k as usize]);
        // Free when list order is the slots (a fresh store) or an explicit
        // list without stale entries; else the cached list order.
        let slot = match order.nth_fast(slots, index) {
            Some(s) => s,
            None => self.order_of(k).0.get(index).copied(),
        };
        let slot = slot.ok_or_else(|| PyValueError::new_err("index out of range"))?;
        Ok(self.inner.key(k, slot))
    }

    /// The list index of a live key (the inverse of `key_at`).
    fn index_of(&self, key: u128) -> PyResult<usize> {
        let (_, k, _, _) = super::keys::split_key(key).ok_or_else(|| PyKeyError::new_err(key))?;
        let slot = self.inner.slot(k, key).ok_or_else(|| PyKeyError::new_err(key))?;
        let st = &self.inner.state;
        if let Some(i) = st.order[k as usize].index_fast(&st.slots[k as usize], slot) {
            return Ok(i);
        }
        Ok(self.order_of(k).1[slot as usize] as usize)
    }

    fn is_live(&self, key: u128) -> bool {
        match super::keys::split_key(key) {
            Some((_, k, _, _)) => self.inner.slot(k, key).is_some(),
            None => false,
        }
    }

    /// `Mesh.move_vertex(v, delta)`; returns the change record.
    fn move_vertex(&mut self, key: u128, delta: (f64, f64, f64)) -> PyResult<PyChange> {
        let v = self.inner.slot(Kind::Vertex, key).ok_or_else(|| PyKeyError::new_err(key))?;
        Ok(PyChange { inner: self.inner.move_vertex(v, [delta.0, delta.1, delta.2]) })
    }

    /// `Mesh.place_vertex(v, position)`; returns the change record.
    fn place_vertex(&mut self, key: u128, position: (f64, f64, f64)) -> PyResult<PyChange> {
        let v = self.inner.slot(Kind::Vertex, key).ok_or_else(|| PyKeyError::new_err(key))?;
        Ok(PyChange { inner: self.inner.place_vertex(v, [position.0, position.1, position.2]) })
    }

    /// Batch move of many vertices by one delta (a Move drag frame),
    /// detached; one record listing them all.
    fn move_vertices(&mut self, py: Python<'_>, keys: Vec<u128>, delta: (f64, f64, f64)) -> PyResult<PyChange> {
        let slots: Vec<u32> = keys
            .iter()
            .map(|&k| self.inner.slot(Kind::Vertex, k).ok_or_else(|| PyKeyError::new_err(k)))
            .collect::<PyResult<_>>()?;
        let d = [delta.0, delta.1, delta.2];
        let inner = &mut self.inner;
        let c = py.detach(|| {
            let mut all = Change::default();
            for v in slots {
                let c = inner.move_vertex(v, d);
                all.revision = c.revision;
                all.moved.extend(c.moved);
            }
            all
        });
        Ok(PyChange { inner: c })
    }

    /// `Mesh.vertex_at(position)`: the key welding would pick, or None.
    fn vertex_at(&self, position: (f64, f64, f64)) -> Option<u128> {
        self.inner.lookup([position.0, position.1, position.2]).map(|v| self.inner.key(Kind::Vertex, v))
    }

    /// `Mesh.registers(v)`.
    fn registers(&self, key: u128) -> bool {
        match self.inner.slot_any(Kind::Vertex, key) {
            Some(v) => self.inner.registers(v),
            None => false,
        }
    }

    /// The weld registry as `(cells (N, 3) int64, vertex list index (N,))`,
    /// sorted by cell.
    fn registry<'py>(&self, py: Python<'py>) -> (Bound<'py, PyArray2<i64>>, Bound<'py, PyArray1<i64>>) {
        registry_arrays(py, &self.inner)
    }

    /// `Mesh._displaced` as `{cell: [vertex list index, ...]}`.
    fn displaced<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dv = self.inner.state.order[0].dense_of_slot(&self.inner.state.slots[0]);
        let d = PyDict::new(py);
        for (c, q) in self.inner.state.displaced.iter() {
            let ids: Vec<i64> = q.iter().map(|&v| dv[v as usize] as i64).collect();
            d.set_item((c.0, c.1, c.2), ids)?;
        }
        Ok(d)
    }

    /// `Mesh._displaced` as `{cell: [vertex key, ...]}`: a dropped vertex
    /// may still wait in a queue (Python's `drop_orphan_vertices` leaves
    /// the queues alone too), which a list index cannot name.
    fn displaced_keys<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        for (c, q) in self.inner.state.displaced.iter() {
            let ks: Vec<u128> = q.iter().map(|&v| self.inner.key(Kind::Vertex, v)).collect();
            d.set_item((c.0, c.1, c.2), ks)?;
        }
        Ok(d)
    }

    /// Faces touching the given vertices, each once (keys).
    fn faces_around(&self, keys: Vec<u128>) -> PyResult<Vec<u128>> {
        let vs: Vec<u32> = keys
            .iter()
            .map(|&k| self.inner.slot(Kind::Vertex, k).ok_or_else(|| PyKeyError::new_err(k)))
            .collect::<PyResult<_>>()?;
        Ok(self.inner.faces_around(&vs).into_iter().map(|f| self.inner.key(Kind::Face, f)).collect())
    }

    // ---- port step 2: the incidence primitives (store/topo.rs) ----------

    /// Whether `key` names an element of this store, listed or removed
    /// (a tombstone history may relink).
    fn knows(&self, key: u128) -> bool {
        match super::keys::split_key(key) {
            Some((_, k, _, _)) => self.inner.slot_any(k, key).is_some(),
            None => false,
        }
    }

    /// `Mesh._probe_neighbours(cell, position)`: the vertex key or None.
    fn probe(&self, cell: (i64, i64, i64), position: (f64, f64, f64)) -> Option<u128> {
        let p = self.inner.quantize(p3(position));
        self.inner.probe(cell, &p).map(|v| self.inner.key(Kind::Vertex, v))
    }

    /// `Mesh.vertex(position)`: `(key, change)`.
    fn vertex(&mut self, position: (f64, f64, f64)) -> PyResult<(u128, PyChange)> {
        let (v, ch) = self.op(|s, ch| s.vertex(p3(position), ch))?;
        Ok((self.inner.key(Kind::Vertex, v), ch))
    }

    /// `Mesh._link_edge(v0, v1)` on two live vertices: `(key, change)`.
    fn link_edge(&mut self, a: u128, b: u128) -> PyResult<(u128, PyChange)> {
        let va = live_of(&self.inner, Kind::Vertex, a)?;
        let vb = live_of(&self.inner, Kind::Vertex, b)?;
        let (e, ch) = self.op(|s, ch| s.link_edge(va, vb, ch))?;
        Ok((self.inner.key(Kind::Edge, e), ch))
    }

    /// `Mesh.add_edge(a, b)`: `(key, change)`.
    fn add_edge(&mut self, a: (f64, f64, f64), b: (f64, f64, f64)) -> PyResult<(u128, PyChange)> {
        let (e, ch) = self.op(|s, ch| s.add_edge(p3(a), p3(b), ch))?;
        Ok((self.inner.key(Kind::Edge, e), ch))
    }

    #[pyo3(signature = (key, path, closed, hypot, compensated=true))]
    fn sweep_stations(&self, key: u128, path: Vec<[f64; 3]>, closed: bool, hypot: &str, compensated: bool)
        -> PyResult<Option<Vec<super::sweep::Station>>> {
        let face = live_of(&self.inner, Kind::Face, key)?;
        Ok(self.inner.sweep_stations(face, &path, closed, crate::hypot_of(hypot)?, compensated))
    }

    #[pyo3(signature = (key, stations, closed, hypot, cap_holes=None))]
    fn sweep_face(&mut self, key: u128, stations: Vec<super::sweep::Station>,
                  closed: bool, hypot: &str, cap_holes: Option<[Vec<bool>; 2]>) -> PyResult<PyChange> {
        let face = live_of(&self.inner, Kind::Face, key)?;
        let h = crate::hypot_of(hypot)?;
        Ok(self.op_all(|s, ch| s.sweep_face(face, &stations, closed, h, cap_holes.as_ref(), ch))?.1)
    }

    fn finish_sweep(&mut self, before: &PyStoreSnapshot, key: u128, path: Vec<[f64; 3]>,
                    stations: Vec<super::sweep::Station>, closed: bool, hypot: &str)
        -> PyResult<(Vec<Vec<u128>>, PyChange)> {
        if before.inner.lineage != self.inner.lineage || before.inner.state.ns != self.inner.state.ns {
            return Err(PyValueError::new_err("foreign sweep snapshot"));
        }
        if path.len() < 2 || path.len() != stations.len() || stations.iter().any(|s|
            s.len() != stations[0].len() || s.iter().zip(&stations[0]).any(|(a,b)| a.len()!=b.len())) {
            return Err(PyValueError::new_err("incompatible sweep stations"));
        }
        let old = reader(&before.inner);
        let face = live_of(&old, Kind::Face, key)?;
        let h = crate::hypot_of(hypot)?;
        self.op(|s, ch| {
            s.soften_sweep(&old, h, ch);
            Ok(s.sweep_curves(&old, face, &path, &stations, closed))
        })
    }

    fn set_curve_run(&mut self, keys: Vec<u128>, curve: i64) -> PyResult<PyChange> {
        let edges = keys.into_iter().map(|k| live_of(&self.inner, Kind::Edge, k))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(self.op(|s,ch| {
            for e in edges {
                s.state.ecurve.set(e as usize,curve);
                ch.modified[1].push(s.key(Kind::Edge,e));
            }
            Ok(())
        })?.1)
    }

    fn offset_outline(&self, key: u128, normal: [f32; 3], distance: f64, hypot: &str)
        -> PyResult<Vec<super::intersect::Segment>> {
        let face = live_of(&self.inner, Kind::Face, key)?;
        Ok(self.inner.offset_outline(face, normal, distance, crate::hypot_of(hypot)?))
    }

    fn offset_hole_targets(&self, key: u128, regions: Vec<Vec<[f32; 3]>>, normal: [f32; 3],
                           hypot: &str) -> PyResult<Option<Vec<i64>>> {
        let face = live_of(&self.inner, Kind::Face, key)?;
        Ok(super::offset::place_holes(&self.inner, face, &regions, normal, crate::hypot_of(hypot)?))
    }

    fn keep_offset_regions(&self, key: u128, regions: Vec<Vec<[f32; 3]>>, normal: [f32; 3],
                           distance: f64, hypot: &str) -> PyResult<Vec<bool>> {
        let face = live_of(&self.inner, Kind::Face, key)?;
        Ok(super::offset::keep_regions(&self.inner, face, regions, normal, distance, crate::hypot_of(hypot)?))
    }

    fn offset_face(&mut self, key: u128, regions: Vec<Vec<Vec<[f64; 3]>>>)
        -> PyResult<(Vec<u128>, PyChange)> {
        let face = live_of(&self.inner, Kind::Face, key)?;
        self.op_all(|s, ch| s.offset_face(face, &regions, ch))
    }

    /// Crossing segments for loose faces in this editing context, in pair order.
    #[pyo3(signature = (selected, mode, hypot, placements=Vec::new(), other_world=Vec::new()))]
    fn intersection_segments(&self, py: Python<'_>, selected: Vec<u128>, mode: &str, hypot: &str,
                             placements: Vec<(usize, super::intersect::WorldRings)>,
                             other_world: Vec<super::intersect::WorldRings>)
        -> PyResult<Vec<super::intersect::Segment>> {
        if !matches!(mode, "context" | "selection" | "model") {
            return Err(PyValueError::new_err("unknown intersection mode"));
        }
        use super::intersect::Input;
        let mut faces = Vec::with_capacity(selected.len() + placements.len());
        let mut groups = placements.into_iter().peekable();
        for (i, key) in selected.into_iter().map(Some).chain([None]).enumerate() {
            while groups.peek().is_some_and(|(at, _)| *at == i) {
                faces.push(Input::Placement(groups.next().unwrap().1));
            }
            if let Some(k) = key {
                faces.push(Input::Face(live_of(&self.inner, Kind::Face, k)?));
            }
        }
        if groups.next().is_some() {
            return Err(PyValueError::new_err("placement offsets must be sorted within the selection"));
        }
        let h = crate::hypot_of(hypot)?;
        let store = &self.inner;
        Ok(py.detach(|| super::intersect::segments(store, &faces, &other_world, mode, h)))
    }

    /// `Mesh.add_face` with the rings as positions (outer first, holes
    /// already filtered): `(key, change)`.
    fn add_face(&mut self, rings: Vec<Vec<(f64, f64, f64)>>) -> PyResult<(u128, PyChange)> {
        let rings: Vec<Vec<[f64; 3]>> = rings.into_iter().map(|r| r.into_iter().map(p3).collect()).collect();
        let (f, ch) = self.op(|s, ch| s.add_face(&rings, ch))?;
        Ok((self.inner.key(Kind::Face, f), ch))
    }

    /// `Mesh.remove_edge(e)` (a removed edge: nothing).
    fn remove_edge(&mut self, key: u128) -> PyResult<PyChange> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        Ok(self.op(|s, ch| { s.remove_edge(e, ch); Ok(()) })?.1)
    }

    /// `Mesh.remove_face(f)`; StoreFallback when the face would stay
    /// listed on an edge (see `Store::remove_face_blocked`).
    fn remove_face(&mut self, key: u128) -> PyResult<PyChange> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        if self.inner.remove_face_blocked(f) {
            return Err(StoreFallback::new_err("remove_face would stay listed on an edge"));
        }
        Ok(self.op(|s, ch| { s.remove_face(f, ch); Ok(()) })?.1)
    }

    /// `Mesh.relink_edge(e)`: `(relinked, change)`; StoreFallback when the
    /// result is not a store state (see store/topo.rs).
    fn relink_edge(&mut self, key: u128) -> PyResult<(bool, PyChange)> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        self.op(|s, ch| s.relink_edge(e, ch))
    }

    /// `Mesh.relink_face(f)`: `(relinked, change)`.
    fn relink_face(&mut self, key: u128) -> PyResult<(bool, PyChange)> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        self.op(|s, ch| s.relink_face(f, ch))
    }

    /// `Mesh.add_hole(face, positions)`: `(loop vertex keys, change)`.
    fn add_hole(&mut self, key: u128, ring: Vec<(f64, f64, f64)>) -> PyResult<(Vec<u128>, PyChange)> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        let ring: Vec<[f64; 3]> = ring.into_iter().map(p3).collect();
        let (lp, ch) = self.op(|s, ch| s.add_hole(f, &ring, ch))?;
        Ok((lp.into_iter().map(|v| self.inner.key(Kind::Vertex, v)).collect(), ch))
    }

    /// Restore a detached hole by its vertex keys, without welding.
    fn relink_hole(&mut self, key: u128, ring: Vec<u128>) -> PyResult<PyChange> {
        let f = live_of(&self.inner, Kind::Face, key)?;
        let vertices = ring.into_iter().map(|k| live_of(&self.inner, Kind::Vertex, k))
            .collect::<PyResult<_>>()?;
        Ok(self.op(|s, ch| { s.relink_hole(f, vertices, ch); Ok(()) })?.1)
    }

    /// `Mesh.remove_hole(face, loop)` with the loop as vertex keys.
    fn remove_hole(&mut self, key: u128, ring: Vec<u128>) -> PyResult<PyChange> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        let ring: Vec<u32> = ring.iter().map(|&k| any_of(&self.inner, Kind::Vertex, k)).collect::<PyResult<_>>()?;
        Ok(self.op(|s, ch| { s.remove_hole(f, &ring, ch); Ok(()) })?.1)
    }

    /// `Mesh.purge_debris()`: `(vertices dropped, entries dropped, change)`.
    fn purge_debris(&mut self) -> PyResult<(usize, usize, PyChange)> {
        let ((a, b), ch) = self.op(|s, ch| Ok(s.purge_debris(ch)))?;
        Ok((a, b, ch))
    }

    /// `Mesh.drop_orphan_vertices(vertices)`: `(count, change)`; keys of
    /// other meshes are ignored (Python: not listed here).
    fn drop_orphan_vertices(&mut self, keys: Vec<u128>) -> PyResult<(usize, PyChange)> {
        let vs: Vec<u32> = keys.iter().filter_map(|&k| self.inner.slot_any(Kind::Vertex, k)).collect();
        self.op(|s, ch| s.drop_orphan_vertices(&vs, ch))
    }

    /// `Mesh.relink_vertices(vertices)`; every key must be this store's.
    fn relink_vertices(&mut self, keys: Vec<u128>) -> PyResult<PyChange> {
        let vs: Vec<u32> = keys.iter().map(|&k| any_of(&self.inner, Kind::Vertex, k)).collect::<PyResult<_>>()?;
        Ok(self.op(|s, ch| { s.relink_vertices(&vs, ch); Ok(()) })?.1)
    }

    fn plan_vertex_bevel(
        &self,
        keys: Vec<u128>,
        size: f64,
        segments: usize,
        profile: f64,
        hypot: &str,
    ) -> PyResult<PyRoundPlan> {
        let s = &self.inner;
        let vs = keys
            .iter()
            .map(|&k| live_of(s, Kind::Vertex, k))
            .collect::<PyResult<_>>()?;
        let h = crate::hypot_of(hypot)?;
        round_preview(
            s,
            super::bevel::plan(s, vs, size, segments.max(1), profile, h),
            h,
        )
    }

    fn plan_fillet(
        &self,
        keys: Vec<u128>,
        size: f64,
        segments: usize,
        profile: f64,
        measure: &str,
        corners: &str,
        shape: &str,
        controls: Vec<(f64, f64, bool)>,
        hypot: &str,
    ) -> PyResult<PyRoundPlan> {
        let s = &self.inner;
        let es = keys
            .iter()
            .map(|&k| live_of(s, Kind::Edge, k))
            .collect::<PyResult<_>>()?;
        let h = crate::hypot_of(hypot)?;
        let n = segments.max(1);
        let result = super::fillet::sample_profile(shape, &controls, n).and_then(|outline| {
            let mirror = matches!(shape, "custom" | "cornice" | "crown");
            super::fillet::plan(
                s,
                es,
                size,
                n,
                profile,
                measure,
                corners == "cut",
                &outline,
                mirror,
                shape == "steps",
                h,
            )
        });
        round_preview(s, result, h)
    }

    #[pyo3(signature = (plan, curve_ids, material_index=None))]
    fn apply_fillet(
        &mut self,
        plan: &PyRoundPlan,
        curve_ids: Vec<i64>,
        material_index: Option<u32>,
    ) -> PyResult<PyChange> {
        let material = match material_index {
            Some(i) => i
                .checked_add(1)
                .ok_or_else(|| PyValueError::new_err("material index overflow"))?,
            None => 0,
        };
        if plan.lineage != self.inner.lineage || plan.revision != self.inner.revision {
            return Err(PyValueError::new_err("stale or foreign fillet plan"));
        }
        if curve_ids.len() != plan.inner.curves.len() || curve_ids.iter().any(|&id| id < 0) {
            return Err(PyValueError::new_err(
                "one non-negative curve ID per arc required",
            ));
        }
        Ok(self
            .op_all(|s, ch| plan.inner.apply(s, &curve_ids, material, plan.hypot, ch))?
            .1)
    }

    // ---- port step 3: splits and the stitch (store/stitch.rs) -----------

    /// `Mesh.split_edge(edge, position)`: `((e0, e1, bumps), change)`.
    fn split_edge(&mut self, key: u128, position: (f64, f64, f64)) -> PyResult<((u128, u128, usize), PyChange)> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        let ((a, b, n), ch) = self.op(|s, ch| s.split_edge(e, p3(position), ch))?;
        Ok(((self.inner.key(Kind::Edge, a), self.inner.key(Kind::Edge, b), n), ch))
    }

    /// `Mesh.split_edge_at(edge, mid)`:
    /// `((e0, e1, e0_new, e1_new, faces), change)`; three bumps.
    #[allow(clippy::type_complexity)]
    fn split_edge_at(&mut self, key: u128, mid: u128) -> PyResult<((u128, u128, bool, bool, Vec<u128>), PyChange)> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        let m = any_of(&self.inner, Kind::Vertex, mid)?;
        let (r, ch) = self.op(|s, ch| s.split_edge_at(e, m, ch))?;
        let s = &self.inner;
        let faces = r.faces.iter().map(|&f| s.key(Kind::Face, f)).collect();
        Ok(((s.key(Kind::Edge, r.e0), s.key(Kind::Edge, r.e1), r.e0_new, r.e1_new, faces), ch))
    }

    /// `Mesh.interior_vertex_on(edge)` with Qt's `hypot` formula.
    fn interior_vertex_on(&self, key: u128, hypot: &str) -> PyResult<Option<u128>> {
        let e = any_of(&self.inner, Kind::Edge, key)?;
        let h = crate::hypot_of(hypot)?;
        Ok(self.inner.interior_vertex_on(e, h).map(|v| self.inner.key(Kind::Vertex, v)))
    }

    /// `Mesh.collapsible_vertex(v)`.
    fn collapsible_vertex(&self, key: u128, hypot: &str) -> PyResult<bool> {
        let v = any_of(&self.inner, Kind::Vertex, key)?;
        Ok(self.inner.collapsible(v, crate::hypot_of(hypot)?))
    }

    /// `Mesh.collapse_vertex(v)`:
    /// `((e1, e2, n1, n2, edge, edge_new, faces), change)`; three bumps.
    #[allow(clippy::type_complexity)]
    fn collapse_vertex(&mut self, key: u128) -> PyResult<((u128, u128, u128, u128, u128, bool, Vec<u128>), PyChange)> {
        let v = any_of(&self.inner, Kind::Vertex, key)?;
        let (c, ch) = self.op(|s, ch| {
            let mut occ = s.corner_counts();
            s.collapse_vertex(v, &mut occ, ch)
        })?;
        let s = &self.inner;
        let faces = c.faces.iter().map(|&f| s.key(Kind::Face, f)).collect();
        Ok((
            (
                s.key(Kind::Edge, c.e1),
                s.key(Kind::Edge, c.e2),
                s.key(Kind::Vertex, c.n1),
                s.key(Kind::Vertex, c.n2),
                s.key(Kind::Edge, c.edge),
                c.edge_new,
                faces,
            ),
            ch,
        ))
    }

    /// `run_stitch`'s phase 2: `((collapses, bumps, refused vertex or
    /// None), change)`. Collapses before a refused vertex stay done.
    fn collapse_collinear(&mut self, hypot: &str) -> PyResult<((usize, usize, Option<u128>), PyChange)> {
        let h = crate::hypot_of(hypot)?;
        let ((n, b, r), ch) = self.op(|s, ch| Ok(s.collapse_collinear(h, ch)))?;
        Ok(((n, b, r.map(|v| self.inner.key(Kind::Vertex, v))), ch))
    }

    /// `Mesh._weld_into(keep, dup)`: `(bumps, change)`.
    fn weld_into(&mut self, keep: u128, dup: u128) -> PyResult<(usize, PyChange)> {
        let k = any_of(&self.inner, Kind::Vertex, keep)?;
        let d = any_of(&self.inner, Kind::Vertex, dup)?;
        self.op_all(|s, ch| {
            let mut occ = s.corner_counts();
            s.weld_into(k, d, &mut occ, ch)
        })
    }

    /// `Mesh.weld_coincident()`: `((changed, bumps), change)`. All or
    /// nothing: a weld the store refuses half way puts the state back
    /// (StoreFallback, nothing changed).
    fn weld_coincident(&mut self) -> PyResult<((bool, usize), PyChange)> {
        self.op_all(|s, ch| s.weld_coincident(ch))
    }

    /// `Mesh.dedupe_faces()`: `(([(dropped, kept)], bumps), change)`.
    #[allow(clippy::type_complexity)]
    fn dedupe_faces(&mut self) -> PyResult<((Vec<(u128, u128)>, usize), PyChange)> {
        let ((pairs, b), ch) = self.op_all(|s, ch| Ok(s.dedupe_faces(ch)))?;
        let s = &self.inner;
        Ok(((pairs.into_iter().map(|(d, k)| (s.key(Kind::Face, d), s.key(Kind::Face, k))).collect(), b), ch))
    }

    /// `Mesh.dissolve_edge` (`winding_tolerant`) or
    /// `dissolve_edge_recorded`: `(None or (merged, faces, bumps), change)`.
    /// `hole_filter(holes)` is `add_face`'s nested-hole filter, called with
    /// the holes' positions when there are several; it returns a bool per
    /// hole. An exception there is a StoreFallback (nothing changed).
    #[allow(clippy::type_complexity)]
    #[pyo3(signature = (key, hypot, winding_tolerant, hole_filter))]
    fn dissolve_edge(
        &mut self,
        key: u128,
        hypot: &str,
        winding_tolerant: bool,
        hole_filter: &Bound<'_, PyAny>,
    ) -> PyResult<(Option<(u128, Vec<u128>, usize)>, PyChange)> {
        let e = live_of(&self.inner, Kind::Edge, key)?;
        let plan = self.inner.plan_dissolve_edge(e, winding_tolerant, crate::hypot_of(hypot)?);
        self.merge(plan, hole_filter)
    }

    /// `Mesh.dissolve_coplanar_pair(a, b)`:
    /// `(None or (merged, removed edges, bumps), change)`.
    #[allow(clippy::type_complexity)]
    fn dissolve_pair(
        &mut self,
        a: u128,
        b: u128,
        hypot: &str,
        hole_filter: &Bound<'_, PyAny>,
    ) -> PyResult<(Option<(u128, Vec<u128>, usize)>, PyChange)> {
        let fa = live_of(&self.inner, Kind::Face, a)?;
        let fb = live_of(&self.inner, Kind::Face, b)?;
        let plan = self.inner.plan_dissolve_pair(fa, fb, crate::hypot_of(hypot)?).map_err(op_err)?;
        let edges = match &plan {
            Plan::Merge(m) => m.edges.iter().map(|&e| self.inner.key(Kind::Edge, e)).collect(),
            _ => Vec::new(),
        };
        let (r, ch) = self.merge(plan, hole_filter)?;
        Ok((r.map(|(f, _, n)| (f, edges, n)), ch))
    }

    /// `Mesh.dissolve_coplanar_region(faces)`, faces in the caller's
    /// order: `(None or (stacked, face, bumps), change)` — `stacked`: the
    /// faces were one cycle, the first kept (the facade sets its attrs).
    #[allow(clippy::type_complexity)]
    fn dissolve_region(
        &mut self,
        faces: Vec<u128>,
        hypot: &str,
        hole_filter: &Bound<'_, PyAny>,
    ) -> PyResult<(Option<(bool, u128, usize)>, PyChange)> {
        let fs: Vec<u32> = faces.iter().map(|&k| live_of(&self.inner, Kind::Face, k)).collect::<PyResult<_>>()?;
        let plan = self.inner.plan_dissolve_region(&fs, crate::hypot_of(hypot)?);
        let stacked = matches!(plan, Plan::Stacked(_));
        let (r, ch) = self.merge(plan, hole_filter)?;
        Ok((r.map(|(f, _, n)| (stacked, f, n)), ch))
    }

    fn replace_plane_faces(&mut self, old: Vec<u128>, caps: Vec<super::caps::Cap>) -> PyResult<(Vec<u128>, PyChange)> {
        let faces: PyResult<Vec<_>> = old.iter().map(|&k| slot_of(&self.inner, Kind::Face, k)).collect();
        let faces = faces?;
        if faces.iter().copied().collect::<std::collections::HashSet<_>>().len() != faces.len() {
            return Err(PyValueError::new_err("duplicate face key"));
        }
        self.op_all(|s,ch| s.replace_caps(&faces,&caps,ch))
    }

    fn prune_plane_debris(&mut self, origin: [f32;3], normal: [f32;3], tolerance: f64, hypot: &str) -> PyResult<PyChange> {
        let h = crate::hypot_of(hypot)?;
        Ok(self.op(|s,ch| s.prune_plane_debris(origin,normal,tolerance,h,ch))?.1)
    }

    /// `turn_face_over`'s ring reversal (outer and holes).
    fn reverse_face(&mut self, key: u128) -> PyResult<PyChange> {
        let f = any_of(&self.inner, Kind::Face, key)?;
        Ok(self.op(|s, ch| {
            s.reverse_face(f, ch);
            Ok(())
        })?
        .1)
    }

    /// Keys of the removed elements of `kind` this state keeps (slot
    /// order): what a materialisation gives objects besides the listed.
    fn dead_keys(&self, kind: &str) -> PyResult<Vec<u128>> {
        let k = kind_of(kind)?;
        let s = &self.inner.state.slots[k as usize];
        Ok((0..s.len() as u32).filter(|&i| !s.alive(i)).map(|i| self.inner.key(k, i)).collect())
    }

    /// The store's invariants (tests): AssertionError naming the first
    /// one broken.
    fn check(&self) -> PyResult<()> {
        self.inner.check().map_err(PyAssertionError::new_err)
    }

    /// `(slots, live)` per kind: tombstones are `slots - live`.
    fn slot_counts(&self) -> [(usize, usize); 3] {
        let s = &self.inner.state.slots;
        [(s[0].len(), s[0].live), (s[1].len(), s[1].live), (s[2].len(), s[2].live)]
    }

    /// `capture_state`: O(chunks), whatever the mesh size.
    fn snapshot(&self) -> PyStoreSnapshot {
        PyStoreSnapshot { inner: self.inner.snapshot() }
    }

    /// `restore_state`. A snapshot of ANOTHER store needs `namespace` (a
    /// fresh one): it is adopted with new keys.
    #[pyo3(signature = (snapshot, namespace = None))]
    fn restore(&mut self, snapshot: &PyStoreSnapshot, namespace: Option<u64>) -> PyResult<u64> {
        self.inner.restore(&snapshot.inner, namespace).map_err(PyValueError::new_err)?;
        Ok(self.inner.revision)
    }

    /// `capture_state() == snapshot`.
    fn same_as(&self, snapshot: &PyStoreSnapshot) -> bool {
        snapshot.inner.lineage == self.inner.lineage && self.inner.state.same_as(&snapshot.inner.state)
    }

    /// `(chunks shared, chunks)` between the live state and a snapshot,
    /// over all columns: how much an edit since has copied.
    fn shared_chunks(&self, snapshot: &PyStoreSnapshot) -> (usize, usize) {
        let (a, b) = (&self.inner.state, &snapshot.inner.state);
        let mut tot = (0, 0);
        let mut add = |x: (usize, usize)| {
            tot.0 += x.0;
            tot.1 += x.1;
        };
        add(a.vpos.shared_with(&b.vpos));
        add(a.vedge.shared_with(&b.vedge));
        add(a.ends.shared_with(&b.ends));
        add(a.enext.shared_with(&b.enext));
        add(a.efaces.shared_with(&b.efaces));
        add(a.frings.shared_with(&b.frings));
        add(a.corners.shared_with(&b.corners));
        add(a.ring_sizes.shared_with(&b.ring_sizes));
        add(a.registry.shared_with(&b.registry));
        tot
    }

    /// `copy.deepcopy(mesh)`: shares every chunk, keys in `namespace`.
    fn copy(&self, namespace: u64) -> Self {
        Self::wrap(self.inner.copy_as(namespace))
    }

    /// Allocated bytes per column: `{name: (kind, bytes)}`.
    fn memory<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        for (name, kind, bytes) in self.inner.memory() {
            d.set_item(name, (kind.name(), bytes))?;
        }
        Ok(d)
    }
}


fn dense_dict<'py>(py: Python<'py>, d: super::read::Dense) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("pos", rows3(py, &d.pos))?;
    let eflat: Vec<i64> = d.edges.iter().flat_map(|e| e.iter().copied()).collect();
    out.set_item("edges", Array2::from_shape_vec((d.edges.len(), 2), eflat).unwrap().into_pyarray(py))?;
    out.set_item("flags", d.flags.into_pyarray(py))?;
    out.set_item("curve", d.curve.into_pyarray(py))?;
    out.set_item("layer", d.layer.into_pyarray(py))?;
    out.set_item("corners", d.corners.into_pyarray(py))?;
    out.set_item("ring_sizes", d.ring_sizes.into_pyarray(py))?;
    out.set_item("ring_counts", d.ring_counts.into_pyarray(py))?;
    out.set_item("ef_start", d.ef_start.into_pyarray(py))?;
    out.set_item("ef_face", d.ef_face.into_pyarray(py))?;
    out.set_item("face_attr", d.face_attr.into_pyarray(py))?;
    out.set_item("face_flags", d.face_flags.into_pyarray(py))?;
    Ok(out)
}

fn registry_arrays<'py>(py: Python<'py>, st: &Store) -> (Bound<'py, PyArray2<i64>>, Bound<'py, PyArray1<i64>>) {
    let dv = st.state.order[0].dense_of_slot(&st.state.slots[0]);
    let mut rows: Vec<([i64; 3], i64)> =
        st.state.registry.iter().map(|(c, &v)| ([c.0, c.1, c.2], dv[v as usize] as i64)).collect();
    rows.sort_unstable();
    let cells: Vec<[i64; 3]> = rows.iter().map(|r| r.0).collect();
    let ids: Vec<i64> = rows.iter().map(|r| r.1).collect();
    (rows3(py, &cells), ids.into_pyarray(py))
}

fn slot_of(st: &Store, kind: Kind, key: u128) -> PyResult<u32> {
    st.slot(kind, key).ok_or_else(|| PyKeyError::new_err(key))
}

/// A read-only store over a snapshot's state (shares every chunk).
fn reader(snap: &Snapshot) -> Store {
    let mut st = Store::empty(snap.state.ns, Weld::default());
    st.state = snap.state.clone();
    st.lineage = snap.lineage;
    st.revision = snap.revision;
    st
}

#[pymethods]
impl PyStoreSnapshot {
    #[pyo3(signature = (origin, normal, tolerance, hypot, keys=None))]
    fn cap_segments(&self, py: Python<'_>, origin: [f32;3], normal: [f32;3], tolerance: f64,
                    hypot: &str, keys: Option<Vec<u128>>) -> PyResult<Vec<(u128, [[f64;3];2])>> {
        use crate::facegeom::V3;
        let store = reader(&self.inner);
        let h = crate::hypot_of(hypot)?;
        let edges = match keys {
            Some(keys) => keys.into_iter().map(|k| slot_of(&store,Kind::Edge,k)).collect::<PyResult<_>>()?,
            None => store.live_slots(Kind::Edge),
        };
        let o = V3(origin[0], origin[1], origin[2]);
        let n = V3(normal[0], normal[1], normal[2]);
        Ok(py.detach(|| edges.into_iter().filter(|&e| store.edge_on_cap_plane(e,o,n,tolerance,h))
            .map(|e| (store.key(Kind::Edge,e), store.state.ends.get(e as usize)
                .map(|v| *store.state.vpos.get(v as usize)))).collect()))
    }

    fn edge_defects(&self, py: Python<'_>) -> [Vec<u128>;3] {
        let store = reader(&self.inner);
        py.detach(|| store.edge_defects())
    }

    #[pyo3(signature = (hypot, keys=None))]
    fn indexed_surface<'py>(&self, py: Python<'py>, hypot: &str, keys: Option<Vec<u128>>)
        -> PyResult<(Bound<'py,PyArray2<f64>>, Bound<'py,PyArray2<u32>>, Bound<'py,PyArray1<u32>>, Vec<u128>)> {
        let store = reader(&self.inner);
        let h = crate::hypot_of(hypot)?;
        let faces = match keys {
            Some(keys) => keys.into_iter().map(|k| slot_of(&store,Kind::Face,k)).collect::<PyResult<_>>()?,
            None => store.live_slots(Kind::Face),
        };
        let surface = py.detach(|| store.indexed_surface(&faces,h));
        Ok((rows3(py,&surface.positions), rows3(py,&surface.triangles),
            surface.owners.into_pyarray(py),surface.faces))
    }

    fn seed_orientation(&self, py: Python<'_>, key: u128) -> PyResult<(Vec<u128>, usize)> {
        let store = reader(&self.inner);
        let face = slot_of(&store, Kind::Face, key)?;
        Ok(py.detach(|| store.seed_orientation(face)))
    }

    #[pyo3(signature = (key, hypot, normal=None))]
    fn face_triangles(&self, key: u128, hypot: &str, normal: Option<[f32;3]>) -> PyResult<Vec<[[f32;3];3]>> {
        let store = &reader(&self.inner);
        let face = slot_of(store, Kind::Face, key)?;
        Ok(store.face_triangles(face, normal, crate::hypot_of(hypot)?))
    }

    fn solid_volume(&self, py: Python<'_>, hypot: &str) -> PyResult<Option<f64>> {
        let store = reader(&self.inner);
        let h = crate::hypot_of(hypot)?;
        Ok(py.detach(|| store.solid_volume(h)))
    }

    fn face_set_volume(&self, py: Python<'_>, keys: Vec<u128>, hypot: &str) -> PyResult<Option<f64>> {
        let store = reader(&self.inner);
        let faces: PyResult<Vec<_>> = keys.into_iter().map(|k| slot_of(&store,Kind::Face,k)).collect();
        let faces = faces?;
        let h = crate::hypot_of(hypot)?;
        Ok(py.detach(|| store.face_set_volume(&faces,h)))
    }

    #[getter]
    fn namespace(&self) -> u64 {
        self.inner.state.ns
    }

    #[getter]
    fn lineage(&self) -> u64 {
        self.inner.lineage
    }

    /// `capture_state() == other` between two snapshots of one store:
    /// chunk pointers first, content only where they differ.
    fn same_as(&self, other: &PyStoreSnapshot) -> bool {
        self.inner.lineage == other.inner.lineage && self.inner.state.same_as(&other.inner.state)
    }

    /// Net change between two immutable snapshots of one key namespace.
    fn changes_since(&self, py: Python<'_>, before: &PyStoreSnapshot, attrs: Vec<u128>) -> PyResult<PyChange> {
        if self.inner.lineage != before.inner.lineage || self.inner.state.ns != before.inner.state.ns {
            return Err(PyValueError::new_err("snapshots have different key namespaces"));
        }
        let (new, old) = (reader(&self.inner), reader(&before.inner));
        let mut ch = py.detach(|| new.changes_since(&old));
        ch.modified[2].extend(attrs.into_iter().filter(|&k|
            new.slot(Kind::Face, k).is_some() && old.slot(Kind::Face, k).is_some()));
        ch.dedup();
        Ok(PyChange { inner: ch })
    }

    /// Live keys of `kind` in the snapshot, list order, as ints.
    fn keys(&self, kind: &str) -> PyResult<Vec<u128>> {
        let k = kind_of(kind)?;
        let r = reader(&self.inner);
        Ok(r.live_slots(k).into_iter().map(|s| r.key(k, s)).collect())
    }

    /// Whether the snapshot holds a live element under `key`.
    fn has(&self, key: u128) -> bool {
        let r = reader(&self.inner);
        match super::keys::split_key(key) {
            Some((_, k, _, _)) => r.slot(k, key).is_some(),
            None => false,
        }
    }

    /// The `Plan` arrays of the snapshot (see `MeshStore.arrays`).
    fn arrays<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let r = reader(&self.inner);
        let d = py.detach(|| r.dense());
        dense_dict(py, d)
    }

    fn layer_names(&self) -> Vec<String> {
        self.inner.state.layers.as_ref().clone()
    }

    /// The snapshot's weld registry, as `MeshStore.registry`.
    fn registry<'py>(&self, py: Python<'py>) -> (Bound<'py, PyArray2<i64>>, Bound<'py, PyArray1<i64>>) {
        registry_arrays(py, &reader(&self.inner))
    }

    /// `(attribute-table index or -1, face flags)` in the snapshot.
    fn face_info(&self, key: u128) -> PyResult<(i64, u8)> {
        let r = reader(&self.inner);
        let f = slot_of(&r, Kind::Face, key)?;
        Ok((*r.state.fattr.get(f as usize) as i64 - 1, *r.state.fflags.get(f as usize)))
    }

    /// A vertex's position in the snapshot.
    fn vertex_position(&self, key: u128) -> PyResult<(f64, f64, f64)> {
        let r = reader(&self.inner);
        let v = slot_of(&r, Kind::Vertex, key)?;
        let p = r.state.vpos.get(v as usize);
        Ok((p[0], p[1], p[2]))
    }

    fn counts(&self) -> (usize, usize, usize) {
        let s = &self.inner.state.slots;
        (s[0].live, s[1].live, s[2].live)
    }
}

#[cfg_attr(test, allow(dead_code))]
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMeshStore>()?;
    m.add_class::<PyStoreSnapshot>()?;
    m.add_class::<PyRoundPlan>()?;
    m.add_class::<PyChange>()?;
    m.add("StoreFallback", m.py().get_type::<StoreFallback>())?;
    Ok(())
}

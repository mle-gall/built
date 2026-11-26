// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The Python face of the store: `built_core.MeshStore` and
//! `built_core.StoreSnapshot`.
//!
//! Arrays come back in list order (the `lazy_mesh.Plan` layout). Keys
//! are Python ints (`namespace << 40 | local`); array readers return the
//! 40-bit local parts with `key_base` to OR in, so a million keys never
//! become a million Python ints unless asked. Construction and the bulk
//! readers run detached from the GIL; single-element ops do not (a
//! release/reacquire costs more than the op).

mod command_face;
mod command_vertices;
mod snapshot_fields;
mod elements;
mod weld_existing;

use super::build::{Block, Points as BlockPoints};
use super::edit::Change;
use super::keys::Kind;
use super::stitch::{Merge, Plan};
use super::topo::OpError;
use super::{Snapshot, Store, Weld};
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use numpy::ndarray::Array2;
use numpy::{IntoPyArray, PyArray1, PyArray2, PyReadonlyArray1, PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::exceptions::{PyAssertionError, PyException, PyKeyError, PyOverflowError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

pyo3::create_exception!(
    built_core,
    StoreFallback,
    PyException,
    "An op the store does not represent (see store/topo.rs): nothing changed; run the Python kernel."
);

#[derive(FromPyObject)]
enum PointArray<'py> {
    Single(PyReadonlyArray2<'py, f32>),
    Double(PyReadonlyArray2<'py, f64>),
}

/// Persisted format-3 indices are normally i32. Retain the original
/// array while crossing the binding and widen only where the builder
/// needs an i64. Existing callers can continue passing i64 arrays.
enum IndexArray<'py, D: numpy::ndarray::Dimension> {
    Narrow(numpy::PyReadonlyArray<'py, i32, D>),
    Wide(numpy::PyReadonlyArray<'py, i64, D>),
}

impl<'a, 'py, D: numpy::ndarray::Dimension + 'a> FromPyObject<'a, 'py> for IndexArray<'py, D> {
    type Error = PyErr;
    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        if let Ok(a) = obj.extract() { Ok(Self::Wide(a)) }
        else { Ok(Self::Narrow(obj.extract()?)) }
    }
}
impl<D: numpy::ndarray::Dimension> IndexArray<'_, D> {
    fn indices(&self) -> PyResult<Cow<'_, [i64]>> {
        match self {
            Self::Wide(a) => Ok(Cow::Borrowed(a.as_slice()?)),
            Self::Narrow(a) => Ok(Cow::Owned(a.as_slice()?.iter().copied().map(i64::from).collect())),
        }
    }
    fn shape(&self) -> &[usize] {
        match self { Self::Wide(a) => a.shape(), Self::Narrow(a) => a.shape() }
    }
}

fn triples<'a, T: numpy::Element>(points: &'a PyReadonlyArray2<'_, T>) -> PyResult<&'a [[T; 3]]> {
    if points.shape()[1] != 3 && points.shape()[0] != 0 {
        return Err(PyValueError::new_err("pos: (P, 3) coordinates"));
    }
    let flat = points.as_slice().map_err(|_| PyValueError::new_err("pos: contiguous"))?;
    Ok(flat.as_chunks::<3>().0)
}

#[pyclass(module = "built_core", name = "MeshStore", subclass)]
pub struct PyMeshStore {
    pub inner: Store,
    /// Per kind, `(revision, live slots in list order, slot -> index)`:
    /// `mesh.faces[i]` / `.index(f)` in a loop must not rebuild the list
    /// order per call once tombstones or a relink made it non-trivial.
    /// Every mutation bumps the revision, so a stale entry is never read.
    cache: Mutex<[Option<OrderCache>; 3]>,
}

/// An occurrence uses the same keys and generation floors as its definition.
#[pyclass(extends=PyMeshStore, frozen)]
struct InstanceStore {
    definition: Py<PyMeshStore>,
    mesh: Py<PyAny>,
}

impl InstanceStore {
    fn refresh(slf: &Bound<'_, Self>) -> PyResult<()> {
        let definition = slf.get().definition.bind(slf.py()).try_borrow()?;
        if definition.inner.revision != slf.as_super().try_borrow()?.inner.revision {
            slf.as_super().try_borrow_mut()?._publish_frame(&definition)?;
        }
        Ok(())
    }
}

#[pymethods]
impl InstanceStore {
    fn __getattribute__<'py>(slf: &Bound<'py, Self>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        Self::refresh(slf)?;
        let attr = slf.as_any().py_super()?.getattr(name)?;
        if !attr.is_callable() {
            return Ok(attr);
        }
        Ok(Bound::new(slf.py(), InstanceMethod {
            view: slf.clone().unbind(), method: attr.unbind(),
            action: match name {
                "snapshot" => InstanceAction::Snapshot,
                "restore" => InstanceAction::Restore,
                _ => InstanceAction::Forward,
            },
        })?.into_any())
    }
}

enum InstanceAction { Snapshot, Restore, Forward }

#[pyclass(frozen)]
struct InstanceMethod {
    view: Py<InstanceStore>,
    method: Py<PyAny>,
    action: InstanceAction,
}

#[pymethods]
impl InstanceMethod {
    #[pyo3(signature=(*args, **kwargs))]
    fn __call__<'py>(&self, py: Python<'py>, args: &Bound<'py, pyo3::types::PyTuple>,
                     kwargs: Option<&Bound<'py, PyDict>>) -> PyResult<Bound<'py, PyAny>> {
        let view = self.view.bind(py);
        InstanceStore::refresh(view)?;
        let start = view.as_super().try_borrow()?.inner.revision;
        if matches!(self.action, InstanceAction::Snapshot) && args.is_empty() && kwargs.is_none() {
            let mut saved = view.as_super().try_borrow()?.snapshot();
            saved.definition = Some(Box::new(view.get().definition.bind(py).try_borrow()?.inner.snapshot()));
            return Ok(Bound::new(py, saved)?.into_any());
        }
        let result = self.method.bind(py).call(args, kwargs);
        let current = view.as_super().try_borrow()?;
        if current.inner.revision != start {
            let mut definition = view.get().definition.bind(py).try_borrow_mut()?;
            if definition.inner.revision != start {
                drop(current);
                view.as_super().try_borrow_mut()?._publish_frame(&definition)?;
                return Err(PyValueError::new_err("definition changed during an occurrence edit"));
            }
            let snapshot = if matches!(self.action, InstanceAction::Restore) {
                args.get_item(0).ok().or_else(|| kwargs.and_then(|kw| kw.get_item("snapshot").ok().flatten()))
            } else { None };
            let exact = snapshot.as_ref().and_then(|arg| arg.cast::<PyStoreSnapshot>().ok())
                .and_then(|snap| snap.get().definition.as_ref())
                .filter(|snap| snap.lineage == definition.inner.lineage);
            if let Some(snap) = exact {
                definition.inner.restore(snap, None).map_err(PyValueError::new_err)?;
            } else {
                definition._publish_frame(&current)?;
            }
            drop(definition);
            view.get().mesh.bind(py).call_method0("mark_modified")?;
        }
        result
    }
}

/// `(revision, live slots in list order, slot -> list index)`.
type OrderCache = (u64, Arc<Vec<u32>>, Arc<Vec<u32>>);

impl PyMeshStore {
    pub(crate) fn place_tool_batch(store: &Bound<'_, PyAny>, positions: Vec<(u128, [f64; 3])>) -> PyResult<()> {
        if store.is_exact_instance_of::<Self>() {
            return store.cast::<Self>()?.try_borrow_mut()?.place_tool_vertices(positions);
        }
        store.call_method1("place_tool_vertices", (positions,))?;
        Ok(())
    }

    /// Native queries must observe the latest definition when reading an occurrence.
    pub(crate) fn read<'py>(store: &Bound<'py, PyAny>) -> PyResult<PyRef<'py, Self>> {
        if let Ok(view) = store.cast::<InstanceStore>() {
            InstanceStore::refresh(view)?;
        }
        Ok(store.extract()?)
    }

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

    fn net_change(&self, before: &super::State) -> PyChange {
        let mut inner = self.inner.state.changes_since(before);
        inner.revision = self.inner.revision;
        PyChange { inner }
    }

    fn drawing_candidates(&self, kind: Kind, keys: Option<Vec<u128>>) -> PyResult<Vec<u32>> {
        match keys {
            None => Ok(self.inner.live_slots(kind)),
            Some(keys) => keys.into_iter().map(|k| live_of(&self.inner, kind, k)).collect(),
        }
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

pub(crate) fn op_err(e: OpError) -> PyErr {
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

#[pyclass(module = "built_core", name = "StoreSnapshot", frozen)]
pub struct PyStoreSnapshot {
    pub inner: Snapshot,
    definition: Option<Box<Snapshot>>,
}

fn kind_of(s: &str) -> PyResult<Kind> {
    Kind::parse(s).ok_or_else(|| PyValueError::new_err(format!("kind {s:?}: vertex, edge or face")))
}

fn slice<'a, T: numpy::Element>(a: &'a PyReadonlyArray1<'_, T>, name: &str) -> PyResult<&'a [T]> {
    a.as_slice().map_err(|_| PyValueError::new_err(format!("{name}: a contiguous 1-D array")))
}

/// Read-only preview. Slots are usable only against the originating store
/// revision. The facade resolves material dictionaries from source face keys.
#[pyclass(module = "built_core", name = "StoreFilletPlan", frozen)]
struct PyRoundPlan {
    inner: super::fillet::RoundPlan,
    lineage: Option<u64>,
    revision: u64,
    hypot: crate::store::geometry::Hypot,
    faces: Vec<u128>,
    edges: Vec<u128>,
    flags: Vec<(u128, [f64; 3], [f64; 3])>,
    materials: Vec<u128>,
}
type Points = Vec<[f64; 3]>;
fn points(ps: &[crate::store::geometry::V3]) -> Points {
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
    hypot: crate::store::geometry::Hypot,
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
        lineage: Some(s.lineage),
        revision: s.revision,
        hypot,
    })
}

/// A change record (`edit::Change`) as Python sees it: built in one
/// allocation; the key lists become Python lists only when read.
#[pyclass(module = "built_core", name = "StoreChange", frozen)]
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
        pos: PointArray<'_>,
        edges: IndexArray<'_, numpy::ndarray::Ix2>,
        flags: Option<PyReadonlyArray1<'_, u8>>,
        curves: Option<IndexArray<'_, numpy::ndarray::Ix1>>,
        layers: Option<IndexArray<'_, numpy::ndarray::Ix1>>,
        layer_names: Vec<String>,
        rings: IndexArray<'_, numpy::ndarray::Ix1>,
        sizes: IndexArray<'_, numpy::ndarray::Ix1>,
        counts: IndexArray<'_, numpy::ndarray::Ix1>,
        face_attr: Option<IndexArray<'_, numpy::ndarray::Ix1>>,
        sequential: bool,
        scale: f64,
        tol2: f64,
        fused: bool,
    ) -> PyResult<Self> {
        let pos3 = match &pos {
            PointArray::Single(p) => BlockPoints::Single(triples(p)?),
            PointArray::Double(p) => BlockPoints::Double(triples(p)?),
        };
        if edges.shape()[0] > 0 && edges.shape()[1] != 2 {
            return Err(PyValueError::new_err("edges: (E, 2) int64"));
        }
        let eflat = edges.indices()?;
        let edges2 = eflat.as_chunks::<2>().0;
        let curves = curves.as_ref().map(IndexArray::indices).transpose()?.map(|ids| ids.iter().copied().map(i128::from).collect::<Vec<_>>());
        let layers = layers.as_ref().map(IndexArray::indices).transpose()?;
        let face_attr = face_attr.as_ref().map(IndexArray::indices).transpose()?;
        let rings = rings.indices()?;
        let sizes = sizes.indices()?;
        let counts = counts.indices()?;
        let block = Block {
            pos: pos3,
            edges: edges2,
            flags: flags.as_ref().map(|a| slice(a, "flags")).transpose()?,
            curves: curves.as_deref(),
            layers: layers.as_deref(),
            layer_names,
            rings: &rings,
            sizes: &sizes,
            counts: &counts,
            face_attr: face_attr.as_deref(),
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

    /// Round subsequent vertex edits and their weld probes like QVector3D.
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

    /// Preserve slot generations, incidence order and binary64 positions for pickle.
    fn pickle_archive<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
        let bytes = bincode::serialize(&self.inner)
            .map_err(|err| PyValueError::new_err(err.to_string()))?;
        Ok(pyo3::types::PyBytes::new(py, &bytes))
    }

    #[staticmethod]
    fn from_pickle_archive(version: u32, bytes: &[u8], namespace: u64) -> PyResult<Self> {
        if version != 1 {
            return Err(PyValueError::new_err("unknown mesh pickle archive version"));
        }
        let mut store: Store = bincode::deserialize(bytes)
            .map_err(|err| PyValueError::new_err(err.to_string()))?;
        let fresh = Store::empty(namespace, store.weld);
        store.state.ns = namespace;
        store.lineage = fresh.lineage;
        Ok(Self::wrap(store))
    }

    /// Format-3 columns in the writer's first-use vertex and edge-tag order.
    /// Multiple holes need the legacy maximal-hole policy.
    fn igz_columns<'py>(
        &self,
        py: Python<'py>,
    ) -> PyResult<Option<(Bound<'py, PyDict>, Vec<String>)>> {
        let packed = py.detach(|| {
            let mut d = self.inner.dense();
            if d.ring_counts.iter().any(|&n| n > 2) {
                return None;
            }
            let mut vertices = Vec::new();
            let mut index = vec![-1i32; d.pos.len()];
            let mut vertex = |i: i64| {
                let k = &mut index[i as usize];
                if *k < 0 {
                    *k = vertices.len() as i32;
                    vertices.push(d.pos[i as usize]);
                }
                *k
            };
            let edges: Vec<i32> = d.edges.iter().flatten().copied().map(&mut vertex).collect();
            let rings: Vec<i32> = d.corners.iter().copied().map(vertex).collect();
            let mut names = Vec::new();
            let mut name_index = std::collections::HashMap::new();
            let layers: Vec<i32> = d
                .layer
                .iter()
                .map(|&id| {
                    if id < 0 {
                        return -1;
                    }
                    let name = &self.inner.state.layers[id as usize];
                    *name_index.entry(name).or_insert_with(|| {
                        names.push(name.clone());
                        (names.len() - 1) as i32
                    })
                })
                .collect();
            for f in &mut d.flags {
                *f &= if *f & 1 != 0 { 7 } else { 3 };
            }
            Some((d, vertices, edges, rings, layers, names))
        });
        let Some((d, vertices, edges, rings, layers, names)) = packed else {
            return Ok(None);
        };
        let out = PyDict::new(py);
        out.set_item("pos", rows3(py, &vertices))?;
        out.set_item(
            "edges",
            Array2::from_shape_vec((edges.len() / 2, 2), edges)
                .unwrap()
                .into_pyarray(py),
        )?;
        out.set_item("rings", rings.into_pyarray(py))?;
        let ints = |v: Vec<i64>| {
            v.into_iter()
                .map(|i| i as i32)
                .collect::<Vec<_>>()
                .into_pyarray(py)
        };
        out.set_item("ring_sizes", ints(d.ring_sizes))?;
        out.set_item("ring_counts", ints(d.ring_counts))?;
        out.set_item("face_attr", d.face_attr.into_pyarray(py))?;
        out.set_item("edge_flags", d.flags.into_pyarray(py))?;
        if d.curve.iter().any(|&i| i >= 0) {
            out.set_item("edge_curve", curve_array(py, d.curve)?)?;
        }
        if !names.is_empty() {
            out.set_item("edge_layer", layers.into_pyarray(py))?;
        }
        Ok(Some((out, names)))
    }

    /// Every array of the `Plan` at once, as a dict: pos, edges, flags,
    /// curve, layer, corners, ring_sizes, ring_counts, ef_start, ef_face,
    /// face_attr (one remap pass for all of them).
    fn arrays<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = py.detach(|| self.inner.dense());
        dense_dict(py, d)
    }

    fn tool_face_picker(&self, py: Python<'_>, hypot: &str) -> PyResult<crate::pick::FacePicker> {
        let hypot = crate::hypot_of(hypot)?;
        Ok(py.detach(|| crate::pick::FacePicker::from_store(&self.inner, hypot)))
    }

    fn nearest_edges(&self, py: Python<'_>, matrix: [f64;16], width: f64, height: f64,
                     x: f64, y: f64, radius: f64) -> Vec<(usize,f64,f64)> {
        let camera = crate::pick::Projector::new(&matrix, width, height);
        let st = &self.inner;
        let edges = self.order_of(Kind::Edge).0;
        py.detach(|| {
            let projected = edges.iter().map(|&edge| {
                let [a,b] = st.state.ends.get(edge as usize).map(|v|
                    camera.px(*st.state.vpos.get(v as usize)));
                (a,b)
            });
            crate::pick::nearest_projected(projected, x, y, radius)
        })
    }

    fn tool_selection(&self, key: u128, grow: &str) -> Option<(Vec<u128>, Vec<u128>)> {
        crate::tools::selection_growth::selection(&self.inner, key, grow)
    }

    fn preview_points(&self, py: Python<'_>, keys: Vec<u128>, origin: [f64; 3]) -> Option<Vec<[f64; 3]>> {
        py.detach(|| crate::tools::loose_preview::preview_points(&self.inner, keys, origin))
    }

    /// The maximal degree-two path through an Offset pick.
    fn edge_run(&self, key: u128) -> PyResult<(Vec<crate::math::Vec3>, bool)> {
        let edge = any_of(&self.inner, Kind::Edge, key)?;
        Ok(crate::tools::edge_runs::store_run(&self.inner, edge))
    }

    /// Box selection reads live topology in document order without exporting
    /// topology, attributes or adjacency arrays to the viewport.
    fn box_pick(&self, py: Python<'_>, matrix: [f64; 16], width: f64, height: f64,
                rect: [f64; 4], crossing: bool) -> (Vec<usize>, Vec<usize>) {
        use crate::pick::{face_in_box, seg_rect, Projector};
        let proj = Projector::new(&matrix, width, height);
        let store = &self.inner;
        py.detach(|| {
            let vertex_slots = &store.state.slots[Kind::Vertex as usize];
            let mut px = vec![(0., 0., false); vertex_slots.len()];
            for v in store.live_slots(Kind::Vertex) {
                px[v as usize] = proj.px(*store.state.vpos.get(v as usize));
            }
            let edges = store.live_slots(Kind::Edge).into_iter().enumerate().filter_map(|(i,e)| {
                let [a,b] = store.state.ends.get(e as usize).map(|v| px[v as usize]);
                seg_rect((a.0,a.1), (b.0,b.1), a.2 && b.2, &rect, crossing).then_some(i)
            }).collect();
            let faces = store.live_slots(Kind::Face).into_iter().enumerate().filter_map(|(i,f)| {
                let rings = store.face_rings(f);
                face_in_box(rings.iter().map(|ring| ring.iter().map(|&v| px[v as usize])), &rect, crossing).then_some(i)
            }).collect();
            (edges, faces)
        })
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

    fn set_endpoint(&mut self, edge: u128, side: usize, vertex: u128) -> PyResult<()> {
        let store = &mut self.inner;
        let e = live_of(store, Kind::Edge, edge)?;
        let v = live_of(store, Kind::Vertex, vertex)?;
        let mut ends = *store.state.ends.get(e as usize);
        if side > 1 {
            return Err(PyValueError::new_err("endpoint index must be zero or one"));
        }
        if ends[side] == v {
            return Ok(());
        }
        if !store.edge_faces(e).is_empty() {
            return Err(PyValueError::new_err(
                "edit the face ring to change a boundary edge",
            ));
        }
        if ends[1 - side] == v || store.find_edge(v, ends[1 - side]).is_some() {
            return Err(PyValueError::new_err(
                "endpoint would create a degenerate or duplicate edge",
            ));
        }
        store.disk_unlink(e);
        ends[side] = v;
        store.state.ends.set(e as usize, ends);
        store.disk_link(e);
        store.finish(Change::default());
        Ok(())
    }

    fn order_edge_faces(&mut self, edge: u128, faces: Vec<u128>) -> PyResult<()> {
        let store = &mut self.inner;
        let e = live_of(store, Kind::Edge, edge)?;
        let next: Vec<_> = faces
            .into_iter()
            .map(|k| live_of(store, Kind::Face, k))
            .collect::<PyResult<_>>()?;
        let old = store.edge_faces(e);
        if old == next {
            return Ok(());
        }
        let mut sorted = next.clone();
        let mut prior = old.to_vec();
        prior.sort_unstable();
        sorted.sort_unstable();
        if sorted != prior {
            return Err(PyValueError::new_err(
                "edge.faces membership is determined by face rings",
            ));
        }
        store.ef_write(e, next);
        store.finish(Change::default());
        Ok(())
    }

    fn replace_rings(&mut self, key: u128, rings: Vec<Vec<u128>>) -> PyResult<()> {
        let f = live_of(&self.inner, Kind::Face, key)?;
        if rings.is_empty() || rings.iter().any(Vec::is_empty) {
            return Err(PyValueError::new_err(
                "each face ring must contain vertices",
            ));
        }
        let rings: Vec<Vec<u32>> = rings
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|k| live_of(&self.inner, Kind::Vertex, k))
                    .collect()
            })
            .collect::<PyResult<_>>()?;
        let old = self.inner.face_rings(f);
        if old == rings {
            return Ok(());
        }
        // Preserve the edge order and existing incidence entries for retained segments.
        let mut work = self.inner.clone();
        let mut delta = std::collections::HashMap::<(u32, u32), i64>::new();
        for (loops, sign) in [(&old, -1), (&rings, 1)] {
            for r in loops {
                for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                    *delta.entry((a.min(b), a.max(b))).or_default() += sign;
                }
            }
        }
        let mut change = Change::default();
        for (loops, sign) in [(&old, -1), (&rings, 1)] {
            for r in loops {
                for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                    let count = delta.get_mut(&(a.min(b), a.max(b))).unwrap();
                    if count.signum() != sign {
                        continue;
                    }
                    if sign < 0 {
                        if let Some(e) = work.find_edge(a, b) {
                            work.ef_remove(e, f);
                        }
                    } else {
                        let e = work.link_edge(a, b, &mut change).map_err(op_err)?;
                        work.ef_push(e, f);
                    }
                    *count -= sign;
                }
            }
        }
        work.set_rings(f, &rings, true);
        work.finish(change);
        self.inner = work;
        Ok(())
    }

    /// Apply an ordered membership edit atomically, preserving held element keys.
    fn set_members(&mut self, kind: &str, keys: Vec<u128>) -> PyResult<()> {
        use std::collections::HashSet;
        let kind = kind_of(kind)?;
        let current = self.inner.live_slots(kind);
        let next = keys
            .into_iter()
            .map(|key| any_of(&self.inner, kind, key))
            .collect::<PyResult<Vec<_>>>()?;
        if current == next {
            return Ok(());
        }
        let wanted: HashSet<_> = next.iter().copied().collect();
        if wanted.len() != next.len() {
            return Err(PyValueError::new_err("mesh lists require unique elements"));
        }
        if current.len() == next.len() && current.iter().all(|slot| wanted.contains(slot)) {
            let state = &mut self.inner.state;
            state.order[kind as usize].set_order(next, &state.slots[kind as usize]);
            self.inner.finish(Change::default());
            return Ok(());
        }
        let mut work = self.inner.clone();
        let mut change = Change::default();
        let dropped: Vec<_> = current
            .into_iter()
            .filter(|s| !wanted.contains(s))
            .collect();
        match kind {
            Kind::Vertex => {
                if work
                    .drop_orphan_vertices(&dropped, &mut change)
                    .map_err(op_err)?
                    != dropped.len()
                {
                    return Err(PyValueError::new_err(
                        "cannot remove a vertex used by an edge",
                    ));
                }
                work.relink_vertices(&next, &mut change);
            }
            Kind::Edge => {
                for e in dropped {
                    work.remove_edge(e, &mut change);
                }
                for &e in &next {
                    if !work.alive(kind, e) {
                        if let Some(reason) = work.relink_edge_blocked(e) {
                            return Err(PyValueError::new_err(reason));
                        }
                        work.relink_edge(e, &mut change).map_err(op_err)?;
                    }
                }
            }
            Kind::Face => {
                for f in dropped {
                    if work.remove_face_blocked(f) {
                        return Err(PyValueError::new_err(
                            "face has inconsistent boundary incidence",
                        ));
                    }
                    work.remove_face(f, &mut change);
                }
                for &f in &next {
                    if !work.alive(kind, f) {
                        work.relink_face(f, &mut change).map_err(op_err)?;
                    }
                }
            }
        }
        work.state.order[kind as usize].set_order(next, &work.state.slots[kind as usize]);
        work.finish(change);
        self.inner = work;
        Ok(())
    }

    /// `Vertex.position` as a float64 triple.
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
    fn edge(&self, key: u128) -> PyResult<(u128, u128, u8, i128, Option<String>)> {
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

    #[pyo3(signature = (key, hypot, normal=None))]
    fn face_triangles(
        &self,
        key: u128,
        hypot: &str,
        normal: Option<[f64; 3]>,
    ) -> PyResult<Vec<[[f64; 3]; 3]>> {
        let store = &self.inner;
        let face = slot_of(store, Kind::Face, key)?;
        Ok(store.face_triangles(face, normal, crate::hypot_of(hypot)?))
    }

    /// The selection's face dots in one call (a huge selection must not
    /// cost a Python call per face): `(corners, rows, counts, missing)`.
    /// `corners` float32 `(3T, 3)`: every triangle of each live face of
    /// `keys` (the same triangulation as `Face.triangulate`), less
    /// `origin`; `rows` the attribute-table row of each face found (-1 for
    /// none) and `counts` its triangles, in `keys` order; `missing` the keys
    /// this store has no live face for.
    fn mark_triangles<'py>(
        &self,
        py: Python<'py>,
        keys: Vec<u128>,
        hypot: &str,
        origin: [f64; 3],
    ) -> PyResult<(Bound<'py, PyArray2<f32>>, Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<u32>>, Vec<u128>)> {
        let h = crate::hypot_of(hypot)?;
        let store = &self.inner;
        let (corners, rows, counts, missing) = py.detach(|| {
            let (mut corners, mut rows, mut counts, mut missing) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for key in keys {
                let Some(face) = store.slot(Kind::Face, key) else {
                    missing.push(key);
                    continue;
                };
                let triangles = store.face_triangles(face, None, h);
                for triangle in &triangles {
                    for p in triangle {
                        corners.extend((0..3).map(|i| (p[i] - origin[i]) as f32));
                    }
                }
                rows.push(*store.state.fattr.get(face as usize) as i64 - 1);
                counts.push(triangles.len() as u32);
            }
            (corners, rows, counts, missing)
        });
        let n = corners.len() / 3;
        let corners = Array2::from_shape_vec((n, 3), corners).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok((corners.into_pyarray(py), rows.into_pyarray(py), counts.into_pyarray(py), missing))
    }

    /// The keys of `kind` ("face", "edge" or "vertex") among `keys` this
    /// store has no live element for, in order: a huge selection checked
    /// against the mesh in one call, not by building sets of every element.
    fn stale_keys(&self, py: Python<'_>, kind: &str, keys: Vec<u128>) -> PyResult<Vec<u128>> {
        let kind = match kind {
            "face" => Kind::Face,
            "edge" => Kind::Edge,
            "vertex" => Kind::Vertex,
            _ => return Err(PyValueError::new_err("kind: face, edge or vertex")),
        };
        let store = &self.inner;
        Ok(py.detach(|| keys.into_iter().filter(|&key| store.slot(kind, key).is_none()).collect()))
    }

    /// What tells the tags of a huge selection of `kind` apart, in one
    /// call: for "face" the distinct attribute-table rows of `keys` (-1 for
    /// none; a face's tag is its row's `layer`), for "edge" the distinct
    /// layer names (None untagged), each in first-met order, plus the keys
    /// this store has no live element for.
    fn distinct_tags<'py>(&self, py: Python<'py>, kind: &str, keys: Vec<u128>) -> PyResult<(Bound<'py, PyList>, Vec<u128>)> {
        let store = &self.inner;
        let st = &store.state;
        let out = PyList::empty(py);
        let mut missing = Vec::new();
        match kind {
            "face" => {
                let mut seen = std::collections::HashSet::new();
                for key in keys {
                    match store.slot(Kind::Face, key) {
                        Some(f) => {
                            let row = *st.fattr.get(f as usize) as i64 - 1;
                            if seen.insert(row) {
                                out.append(row)?;
                            }
                        }
                        None => missing.push(key),
                    }
                }
            }
            "edge" => {
                let mut seen = std::collections::HashSet::new();
                for key in keys {
                    match store.slot(Kind::Edge, key) {
                        Some(e) => {
                            let l = *st.elayer.get(e as usize);
                            if seen.insert(l) {
                                out.append(if l == super::keys::NONE { None } else { Some(st.layers[l as usize].clone()) })?;
                            }
                        }
                        None => missing.push(key),
                    }
                }
            }
            _ => return Err(PyValueError::new_err("kind: face or edge")),
        }
        Ok((out, missing))
    }

    /// Each key's tag reference, in order: for "face" its attribute-table
    /// row (-1 none), for "edge" its index into `names` (-1 untagged); -2
    /// for a key this store has no live element for. `names` the store's
    /// layer names (edges only).
    fn element_tags(&self, py: Python<'_>, kind: &str, keys: Vec<u128>) -> PyResult<(Vec<i64>, Vec<String>)> {
        let store = &self.inner;
        let st = &store.state;
        match kind {
            "face" => Ok((py.detach(|| keys.iter().map(|&k| store.slot(Kind::Face, k).map_or(-2, |f| *st.fattr.get(f as usize) as i64 - 1)).collect()), Vec::new())),
            "edge" => Ok((
                py.detach(|| keys.iter().map(|&k| store.slot(Kind::Edge, k).map_or(-2, |e| {
                    let l = *st.elayer.get(e as usize);
                    if l == super::keys::NONE { -1 } else { l as i64 }
                })).collect()),
                st.layers.as_ref().clone(),
            )),
            _ => Err(PyValueError::new_err("kind: face or edge")),
        }
    }

    /// The selection's edge strokes in one call: `(ends, missing)`, `ends`
    /// float32 `(2E, 3)` (both ends of each live edge of `keys`, less
    /// `origin`, in order), `missing` the keys this store has no live edge
    /// for.
    fn mark_segments<'py>(&self, py: Python<'py>, keys: Vec<u128>, origin: [f64; 3]) -> PyResult<(Bound<'py, PyArray2<f32>>, Vec<u128>)> {
        let store = &self.inner;
        let (ends, missing) = py.detach(|| {
            let (mut ends, mut missing) = (Vec::with_capacity(keys.len() * 6), Vec::new());
            for key in keys {
                let Some(edge) = store.slot(Kind::Edge, key) else {
                    missing.push(key);
                    continue;
                };
                for v in *store.state.ends.get(edge as usize) {
                    let p = store.state.vpos.get(v as usize);
                    ends.extend((0..3).map(|i| (p[i] - origin[i]) as f32));
                }
            }
            (ends, missing)
        });
        let n = ends.len() / 3;
        let ends = Array2::from_shape_vec((n, 3), ends).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok((ends.into_pyarray(py), missing))
    }

    #[pyo3(signature=(key, xform=None))]
    fn curve_centers(&self, py: Python<'_>, key: u128, xform: Option<&Bound<'_, PyAny>>)
        -> PyResult<crate::snap_boundary::Centers> {
        let f = live_of(&self.inner, Kind::Face, key)?;
        crate::snap_boundary::store_centers(py, &self.inner, f, xform)
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
    fn set_edge_curve(&mut self, key: u128, curve: Option<i128>) -> PyResult<u64> {
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

    /// Candidate rows for position-key lookup in mesh order. Python applies
    /// its exact decimal-rounding predicate only to this short list.
    pub(crate) fn vertices_in_floor_cells(&self, near: std::collections::HashSet<[i64; 3]>)
        -> Vec<(u128, [f64; 3])> {
        self.inner.live_slots(Kind::Vertex).into_iter().filter_map(|v| {
            let p = *self.inner.state.vpos.get(v as usize);
            let cell = super::commands::floor_cell(p);
            near.contains(&cell).then(|| (self.inner.key(Kind::Vertex, v), p))
        }).collect()
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

    /// Resolve every key before writing an absolute tool pose. Occurrence
    /// stores publish the completed batch through their normal method wrapper.
    fn place_tool_vertices(&mut self, positions: Vec<(u128, [f64; 3])>) -> PyResult<()> {
        let targets = positions.into_iter().map(|(key, p)| {
            self.inner.slot(Kind::Vertex, key).map(|v| (v, p)).ok_or_else(|| PyKeyError::new_err(key))
        }).collect::<PyResult<Vec<_>>>()?;
        for (v, p) in targets { self.inner.place_vertex(v, p); }
        Ok(())
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
        let cell = if self.inner.state.frame.is_model() { cell } else { self.inner.cell(&p) };
        self.inner.probe(cell, &p).map(|v| self.inner.key(Kind::Vertex, v))
    }

    /// `Mesh.vertex(position)`: `(key, change)`.
    #[pyo3(signature=(points, supplied=None))]
    fn weld_existing<'py>(&mut self, py: Python<'py>, points: PyReadonlyArray2<'py, f64>,
                          supplied: Option<PyReadonlyArray2<'py, f64>>)
        -> PyResult<(Vec<u128>, Bound<'py, PyArray1<i64>>)> {
        let input = triples(&points)?;
        let exact = supplied.as_ref().map(triples).transpose()?.unwrap_or(input);
        if input.len() != exact.len() { return Err(PyValueError::new_err("supplied point count differs")); }
        let (keys, indices) = weld_existing::insert(&mut self.inner, input, exact).map_err(op_err)?;
        Ok((keys, indices.into_pyarray(py)))
    }

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

    #[pyo3(signature = (hypot, compensated, before=None))]
    fn winding_flips(
        &self,
        hypot: &str,
        compensated: bool,
        before: Option<&PyStoreSnapshot>,
    ) -> PyResult<Vec<u128>> {
        let old = before.map(|snapshot| reader(&snapshot.inner));
        Ok(self
            .inner
            .winding_flips(old.as_ref(), crate::hypot_of(hypot)?, compensated))
    }

    fn divide_edges(
        &mut self,
        keys: Vec<u128>,
        n: usize,
        next_curve: i64,
        hypot: &str,
        compensated: bool,
    ) -> PyResult<((usize, i64), PyChange)> {
        let edges = keys
            .into_iter()
            .map(|k| live_of(&self.inner, Kind::Edge, k))
            .collect::<PyResult<Vec<_>>>()?;
        let h = crate::hypot_of(hypot)?;
        let before = self.inner.state.clone();
        let (value, _) = self.op_all(|s, ch| s.divide_drawing_edges(&edges, n, next_curve, h, compensated, ch))?;
        Ok((value, self.net_change(&before)))
    }

    fn join_edges(
        &mut self,
        keys: Vec<u128>,
        hypot: &str,
        compensated: bool,
    ) -> PyResult<((Vec<u128>, Vec<(u128, &'static str)>, usize), PyChange)> {
        let edges = keys
            .into_iter()
            .map(|k| live_of(&self.inner, Kind::Edge, k))
            .collect::<PyResult<Vec<_>>>()?;
        let h = crate::hypot_of(hypot)?;
        let before = self.inner.state.clone();
        let (value, _) = self.op_all(|s, ch| s.join_drawing_edges(edges, h, compensated, ch))?;
        Ok((value, self.net_change(&before)))
    }

    fn drawing_clear(&mut self) -> PyResult<PyChange> {
        Ok(self
            .op(|s, ch| {
                s.drawing_clear(ch);
                Ok(())
            })?
            .1)
    }
    #[staticmethod]
    #[pyo3(signature=(mesh, origin, normal, faces=None))]
    fn command_rebuild_plane(mesh: &Bound<'_, PyAny>, origin: [f64;3], normal: [f64;3], faces: Option<Vec<u128>>)
        -> PyResult<Option<(bool,usize,usize)>> {
        Ok(super::command_facade::command_rebuild_plane(mesh,origin,normal,faces)?
            .map(|r: super::command_facade::RebuiltPlane| (r.ran,r.rebuilt,r.dirty)))
    }

    #[staticmethod]
    fn command_rebuild_planar(mesh: &Bound<'_, PyAny>) -> PyResult<Option<(bool,usize,usize)>> {
        Ok(super::command_facade::command_rebuild_planar(mesh)?
            .map(|r: super::command_facade::RebuiltPlanar| (r.flat,r.rebuilt,r.dirty)))
    }

    #[staticmethod]
    fn command_heal_overlaps(mesh: &Bound<'_,PyAny>) -> PyResult<Option<(Vec<u128>,usize)>> {
        Ok(super::command_facade::command_heal_overlaps(mesh)?
            .map(|r: super::command_facade::Healed| (r.faces,r.dirty)))
    }

    #[staticmethod]
    #[pyo3(signature=(mesh, group_mesh, transform=None, paint=None))]
    fn command_explode_geometry(mesh: &Bound<'_,PyAny>,group_mesh:&Bound<'_,PyAny>,transform:Option<[f64;16]>,paint:Option<&Bound<'_,PyAny>>)
        -> PyResult<Option<(Vec<u128>,Py<PyAny>,usize,usize)>> {
        Ok(super::command_facade::command_explode_geometry(mesh,group_mesh,transform,paint)?
            .map(|r: super::command_facade::Exploded| (r.added,r.copies,r.planes_rebuilt,r.dirty)))
    }

    fn drawing_paint_owners(
        &self,
        before: &PyStoreSnapshot,
        own: std::collections::HashMap<u128, bool>,
        painted_rows: Vec<u32>,
        hypot: &str,
    ) -> PyResult<Vec<(u128, u128, i64)>> {
        let old = reader(&before.inner);
        Ok(self
            .inner
            .paint_owners(&old, &own, &painted_rows.into_iter().collect(), crate::hypot_of(hypot)?))
    }
    #[pyo3(signature = (o,n,hypot,all=false))]
    fn drawing_plane_edges(
        &self,
        o: [f64; 3],
        n: [f64; 3],
        hypot: &str,
        all: bool,
    ) -> PyResult<(Vec<[[f64; 3]; 2]>, Vec<u128>)> {
        Ok(self.inner.drawing_plane_edges(o, n, crate::hypot_of(hypot)?, all))
    }
    fn drawing_coverage(
        &self,
        o: [f64; 3],
        n: [f64; 3],
        regions: Vec<Vec<Vec<[f64; 3]>>>,
        hypot: &str,
    ) -> PyResult<(Vec<u128>, Vec<(usize, u128, [f64; 3])>)> {
        Ok(self.inner.drawing_coverage(o, n, &regions, crate::hypot_of(hypot)?))
    }
    fn remove_drawing_faces(&mut self, keys: Vec<u128>) -> PyResult<PyChange> {
        let faces = self.drawing_candidates(Kind::Face, Some(keys))?;
        Ok(self
            .op_all(|s, ch| {
                for f in faces {
                    s.remove_face(f, ch);
                }
                Ok(())
            })?
            .1)
    }
    fn multiple_hole_faces(&self) -> Vec<u128> {
        self.inner
            .live_slots(Kind::Face)
            .into_iter()
            .filter(|&f| self.inner.state.frings.get(f as usize).n_rings > 2)
            .map(|f| self.inner.key(Kind::Face, f))
            .collect()
    }
    #[pyo3(signature = (face, distance, normal, keep, oriented, preview, own, rows, swapped, surface, hypot, compensated))]
    fn push_extrude(&mut self, face: u128, distance: f64, normal: Option<[f64;3]>, keep: bool, oriented: bool, preview: bool,
                    own: std::collections::HashMap<u128,u32>, rows: Vec<u32>, swapped: Vec<u32>, surface: Vec<u32>, hypot: &str, compensated: bool)
        -> PyResult<(usize,usize,Vec<(u128,u32)>,Vec<u128>,Vec<u128>,Vec<u128>)> {
        let f=live_of(&self.inner,Kind::Face,face)?;
        let h=crate::hypot_of(hypot)?;
        let classes = swapped.len();
        if classes == 0 || surface.len() != classes ||
            own.values().chain(&rows).chain(&swapped).chain(&surface).any(|&c| c as usize >= classes) {
            return Err(PyValueError::new_err("missing push paint class"));
        }
        let mut paint=super::Paint::new(own,rows);
        let (r,_)=self.op_all(|s,ch|s.push_extrude(f,distance,normal,keep,oriented,preview,&mut paint,&swapped,&surface,h,compensated,ch))?;
        Ok((r.bumps,r.steps,r.painted,r.flipped,r.faces,r.edges))
    }

    #[pyo3(signature = (face, base, top, delta, attached, far, hypot))]
    fn push_shell(&mut self, face: u128, base: Vec<Vec<[f64; 3]>>, top: Vec<Vec<[f64; 3]>>,
                  delta: [f64; 3], attached: bool, far: Option<(u128, Vec<[f64; 3]>, Option<Vec<[f64; 3]>>)>, hypot: &str)
        -> PyResult<(usize, usize, u128, Vec<u128>, Vec<u128>, Option<u128>, Vec<u128>)> {
        let face = live_of(&self.inner, Kind::Face, face)?;
        let far = far.map(|(key,back,rest)| Ok::<_,PyErr>((live_of(&self.inner, Kind::Face,key)?,back,rest))).transpose()?;
        let mut work = self.inner.clone();
        let result = work.push_shell(face,base,top,delta,attached,far,crate::hypot_of(hypot)?).map_err(op_err)?;
        self.inner = work;
        Ok((result.dirty,result.steps,result.cap,result.removed_faces,result.removed_edges,result.source,result.remainders))
    }

    fn single_plane(&self, origin: [f64;3], normal: [f64;3], hypot: &str, compensated: bool)
        -> PyResult<Option<Vec<super::caps::Cap>>> {
        self.inner.single_plane(origin, normal, crate::hypot_of(hypot)?, compensated).map_err(op_err)
    }

    fn rebuild_single_plane(&mut self, origin: [f64;3], normal: [f64;3], own: std::collections::HashMap<u128,u32>,
                            rows: Vec<u32>, hypot: &str, compensated: bool) -> PyResult<(usize, Vec<(u128,u32)>)> {
        let mut paint = super::Paint::new(own, rows);
        let h = crate::hypot_of(hypot)?;
        let mut work = self.inner.clone();
        let mut ch = Change::default();
        let result = work.rebuild_single_plane(origin, normal, &mut paint, h, compensated, &mut ch).map_err(op_err)?;
        if result.0 != 0 {
            let removed: Vec<_> = ch.removed[2].iter().filter_map(|&k| work.slot_any(Kind::Face,k)).collect();
            if work.dangling(&removed) { return Err(StoreFallback::new_err("a removed cap is still present on an edge")); }
            work.finish(ch);
            self.inner = work;
        }
        Ok(result)
    }

    fn faces_are_flat(&self, tolerance: f64, hypot: &str) -> PyResult<Option<bool>> {
        Ok(self.inner.faces_are_flat(tolerance, crate::hypot_of(hypot)?))
    }

    #[allow(clippy::type_complexity)]
    fn push_translate(&mut self, cap: Vec<[f64; 3]>, delta: [f64; 3], removing: bool, preview: bool,
                      mut next_curve: i64, own: std::collections::HashMap<u128, u32>, rows: Vec<u32>,
                      hypot: &str, compensated: bool)
        -> PyResult<(usize, i64, Vec<(u128, u128)>, Vec<(u128, u32)>, Vec<(u128, u128)>,
                     Vec<(u128, u128)>, Vec<u128>)> {
        let h = crate::hypot_of(hypot)?;
        let mut paint = super::Paint::new(own, rows);
        let (r, _) = self.op_all(|s, ch| s.push_translate(&cap, delta, removing, preview, &mut next_curve,
                                                          &mut paint, h, compensated, ch))?;
        Ok((r.bumps, next_curve, r.before, r.painted, r.after, r.replaced, r.flipped))
    }

    #[pyo3(signature = (before, base, shift, facet_cos, next_curve, painted, hypot))]
    fn finish_push(&mut self, before: &PyStoreSnapshot, base: Option<u128>, shift: Option<[f64; 3]>,
                   facet_cos: f64, mut next_curve: i64, painted: bool, hypot: &str)
        -> PyResult<(i64, Vec<u128>)> {
        let old = reader(&before.inner);
        if old.lineage != self.inner.lineage {
            return Err(PyValueError::new_err("foreign push snapshot"));
        }
        let face = base.and_then(|key| old.slot(Kind::Face, key));
        let h = crate::hypot_of(hypot)?;
        let fresh = self.inner.finish_push(&old, face, shift, facet_cos, &mut next_curve, painted, h);
        Ok((next_curve, fresh))
    }

    fn heal_plan(
        &self,
        coverage: f64,
        partial: bool,
        pad: f64,
        hypot: &str,
        compensated: bool,
    ) -> PyResult<(Vec<u128>, Vec<(u128, Vec<u128>)>)> {
        Ok(self
            .inner
            .heal_plan(coverage, partial, pad, crate::hypot_of(hypot)?, compensated))
    }

    pub(crate) fn containing_face(
        &self,
        points: Vec<[f64; 3]>,
        candidates: Option<Vec<u128>>,
        exclude: Option<u128>,
        hypot: &str,
    ) -> PyResult<Option<u128>> {
        Ok(self.inner.containing_face(
            &self.drawing_candidates(Kind::Face, candidates)?,
            &points,
            exclude,
            crate::hypot_of(hypot)?,
        ))
    }

    fn curved_edges(&self) -> Vec<(i128, u128)> {
        self.inner
            .live_slots(Kind::Edge)
            .into_iter()
            .filter_map(|e| {
                let cid = *self.inner.state.ecurve.get(e as usize);
                (cid >= 0).then(|| (cid, self.inner.key(Kind::Edge, e)))
            })
            .collect()
    }

    fn resplit_curves(&mut self, mut next_id: i64) -> i64 {
        self.inner.resplit_curves(&mut next_id, super::geometry::Hypot::LibStdCxx);
        next_id
    }

    pub(crate) fn refine_ring(
        &self,
        ring: Vec<[f64; 3]>,
        points: Vec<[f64; 3]>,
        tol: f64,
        hypot: &str,
    ) -> PyResult<Vec<[f64; 3]>> {
        Ok(super::drawing_face::refine(
            &ring,
            &points,
            tol,
            crate::hypot_of(hypot)?,
        ))
    }

    pub(crate) fn carve_face(&self, face: u128, ring: Vec<[f64; 3]>, hypot: &str) -> PyResult<Option<Vec<Vec<[f64; 3]>>>> {
        let f = live_of(&self.inner, Kind::Face, face)?;
        Ok(self.inner.outline(f, crate::hypot_of(hypot)?).carve(&ring))
    }

    pub(crate) fn merged_hole(
        &self,
        py: Python<'_>,
        key: u128,
        ring: Vec<[f64; 3]>,
        hypot: &str,
    ) -> PyResult<Option<(usize, Vec<crate::math::Vec3>)>> {
        let f = live_of(&self.inner, Kind::Face, key)?;
        Ok(self
            .inner
            .outline(f, crate::hypot_of(hypot)?)
            .merged_hole(py, &ring)?
            .map(|(i, ring)| {
                (
                    i,
                    ring.into_iter()
                        .map(|p| glam::DVec3::from_array(p).into())
                        .collect(),
                )
            }))
    }

    fn shared_hole(
        &self,
        face: u128,
        ring: Vec<[f64; 3]>,
        hypot: &str,
    ) -> PyResult<Option<(usize, Vec<[f64; 3]>, Vec<[f64; 3]>)>> {
        let f = live_of(&self.inner, Kind::Face, face)?;
        Ok(self.inner.outline(f, crate::hypot_of(hypot)?).shared_hole(&ring))
    }

    pub(crate) fn classify_segment(
        &self,
        face: u128,
        a: [f64; 3],
        b: [f64; 3],
        candidates: Option<Vec<u128>>,
        hypot: &str,
    ) -> PyResult<(&'static str, Option<u128>)> {
        let fs = self.drawing_candidates(Kind::Face, candidates)?;
        Ok(self.inner.classify_segment(
            live_of(&self.inner, Kind::Face, face)?,
            a,
            b,
            &fs,
            crate::hypot_of(hypot)?,
        ))
    }

    pub(crate) fn orphaned_edges_at(
        &self,
        points: Vec<[f64; 3]>,
        edges: Option<Vec<u128>>,
        faces: Option<Vec<u128>>,
    ) -> PyResult<Vec<u128>> {
        Ok(self.inner.orphan_edges(
            &self.drawing_candidates(Kind::Edge, edges)?,
            &self.drawing_candidates(Kind::Face, faces)?,
            &points,
        ))
    }

    fn prune_orphan_edges(&mut self, points: Vec<[f64; 3]>, edges: Option<Vec<u128>>,
                          faces: Option<Vec<u128>>) -> PyResult<Vec<u128>> {
        let removed = self.orphaned_edges_at(points, edges, faces)?;
        for &edge in &removed {
            self.remove_edge(edge)?;
        }
        Ok(removed)
    }

    pub(crate) fn loop_inside_face(
        &self,
        face: u128,
        ring: Vec<[f64; 3]>,
        exclude_holes: bool,
        hypot: &str,
    ) -> PyResult<bool> {
        let f = live_of(&self.inner, Kind::Face, face)?;
        Ok(self
            .inner
            .outline(f, crate::hypot_of(hypot)?)
            .contains(&ring, exclude_holes))
    }
    pub(crate) fn split_face_loops(
        &self,
        face: u128,
        chain: Vec<[f64; 3]>,
        chord: bool,
        hypot: &str,
    ) -> PyResult<Option<Vec<Vec<[f64; 3]>>>> {
        let f = live_of(&self.inner, Kind::Face, face)?;
        Ok(self.inner.outline(f, crate::hypot_of(hypot)?).pieces(&chain, chord))
    }
    pub(crate) fn subdividing_chain(&self, face: u128, ring: Vec<[f64; 3]>, hypot: &str) -> PyResult<Option<Vec<[f64; 3]>>> {
        let f = live_of(&self.inner, Kind::Face, face)?;
        Ok(self.inner.outline(f, crate::hypot_of(hypot)?).chain(&ring))
    }

    fn drawing_plane(&self, points: Vec<[f64; 3]>, hypot: &str) -> PyResult<Option<[[f64; 3]; 2]>> {
        Ok(self.inner.drawing_plane(&points, crate::hypot_of(hypot)?))
    }

    fn rebuild_plane(&self, points: Vec<[f64; 3]>, cap: usize, hypot: &str) -> PyResult<Option<[[f64; 3]; 2]>> {
        Ok(self.inner.rebuild_plane(&points, cap, crate::hypot_of(hypot)?))
    }

    fn curve_keys(&self, curve: i128) -> Vec<u128> {
        self.inner.curve_keys(curve)
    }

    /// Create or restamp a command edge. The public result contains only key and change data.
    #[staticmethod]
    #[pyo3(signature = (mesh, a, b, soft=None, curve=None, layer=None))]
    fn command_draw_edge(
        mesh: &Bound<'_, PyAny>, a: [f64; 3], b: [f64; 3], soft: Option<bool>,
        curve: Option<i128>, layer: Option<&str>,
    ) -> PyResult<Option<(Option<u128>, bool, u32)>> {
        Ok(super::command_facade::command_draw_edge(mesh, a, b, soft, curve, layer)?
            .map(|r| (r.key, r.flags_changed, r.dirty)))
    }

    /// Draw an edge, stamping optional attributes even on a coincident edge.
    /// Only a new edge belongs to the command. Keep each attribute write's
    /// revision for consumers of the protocol mesh record.
    #[pyo3(signature = (a, b, soft, curve, layer, default_layer))]
    fn draw_command_edge(&mut self, a: [f64; 3], b: [f64; 3], soft: Option<bool>,
                         curve: Option<i128>, layer: Option<&str>, default_layer: &str)
        -> PyResult<Option<u128>> {
        let existing = self.inner.lookup(a).zip(self.inner.lookup(b))
            .and_then(|(v, w)| self.inner.find_edge(v, w));
        let edge = match existing {
            Some(e) => e,
            None => self.op(|s, ch| s.add_edge(a, b, ch))?.0,
        };
        if let Some(soft) = soft {
            let flags = *self.inner.state.eflags.get(edge as usize);
            self.inner.set_edge_flags(edge, if soft { flags | super::SOFT } else { flags & !super::SOFT });
        }
        if let Some(curve) = curve {
            self.inner.set_edge_curve(edge, curve);
        }
        if let Some(layer) = layer {
            self.inner.set_edge_layer(edge, (layer != default_layer).then_some(layer));
        }
        Ok(existing.is_none().then(|| self.inner.key(Kind::Edge, edge)))
    }

    #[staticmethod]
    #[pyo3(signature=(points, edges, bounds, tolerance))]
    fn stitch_junctions(points: PyReadonlyArray2<'_, f64>, edges: PyReadonlyArray2<'_, i64>,
                        bounds: Option<[[f64; 3]; 2]>, tolerance: f64) -> PyResult<Vec<(usize,usize)>> {
        if edges.shape()[1] != 2 { return Err(PyValueError::new_err("edges need two indices")); }
        let edges = edges.as_slice().map_err(|_| PyValueError::new_err("edges must be contiguous"))?;
        super::stitch_command::junctions(triples(&points)?, edges.as_chunks::<2>().0, bounds, tolerance)
            .map_err(PyValueError::new_err)
    }

    fn seeded_merge_edges(&self, seeds: Vec<[f64;3]>) -> Vec<u128> {
        use super::commands::{cell, supplied_cell};
        let cells: std::collections::HashSet<_> = seeds.into_iter().map(supplied_cell).collect();
        self.inner.live_slots(Kind::Edge).into_iter().filter(|&e| {
            self.inner.edge_faces(e).len() == 2 &&
                self.inner.state.ends.get(e as usize).iter().any(|&v|
                    cells.contains(&cell(*self.inner.state.vpos.get(v as usize))))
        }).map(|e| self.inner.key(Kind::Edge,e)).collect()
    }

    #[pyo3(signature=(seeds, fresh, start, hypot))]
    fn stitch_region(&self, seeds: Vec<[f64; 3]>, fresh: Option<std::collections::HashSet<u128>>, start: usize, hypot: &str)
        -> PyResult<Option<(usize, Vec<u128>)>> {
        Ok(self.inner.stitch_region(seeds, fresh, start, crate::hypot_of(hypot)?))
    }

    fn cap_passes_wall(&self, face: u128, rim: Vec<[f64; 3]>, normal: [f64; 3], distance: f64) -> PyResult<bool> {
        Ok(self.inner.cap_passes_wall(live_of(&self.inner, Kind::Face, face)?, &rim, normal, distance))
    }

    #[staticmethod]
    fn wall_depths_crossed(walls: Vec<Vec<f64>>, distance: f64) -> bool {
        super::push_band::passes_wall(&walls, distance)
    }

    fn push_shape_matches(&self, before: &PyStoreSnapshot) -> bool {
        use super::commands::cell;
        let prior = reader(&before.inner);
        if self.counts() != before.counts() { return false; }
        let cells = |store: &Store| {
            let mut rows: Vec<_> = store.live_slots(Kind::Vertex).into_iter()
                .map(|v| cell(*store.state.vpos.get(v as usize))).collect();
            rows.sort_unstable();
            rows
        };
        cells(&self.inner) == cells(&prior)
    }

    fn push_band(
        &mut self,
        base: Vec<[f64; 3]>,
        top: Vec<[f64; 3]>,
        rim: bool,
        rails: bool,
        walls: bool,
        cap: bool,
    ) -> PyResult<(usize, usize)> {
        let band = super::push_band::Band::new(&base, &top, rim, rails, walls, cap)
            .ok_or_else(|| PyValueError::new_err("push rings have different lengths"))?;
        let mut work = self.inner.clone();
        let result = work.apply_push_band(band).map_err(op_err)?;
        self.inner = work;
        Ok(result)
    }

    pub(crate) fn landing_defects(
        &self,
        py: Python<'_>,
        vertices: Vec<u128>,
        area_eps: f64,
        hypot: &str,
    ) -> PyResult<[usize; 6]> {
        use pyo3::types::PySet;
        // The first face in a nearly coplanar pair supplies the projection.
        // Follow the facade's vertex.edges set order, including hash collisions.
        let mut seen = std::collections::HashSet::new();
        let mut near = Vec::new();
        for key in vertices {
            let v = any_of(&self.inner, Kind::Vertex, key)?;
            let incident = PySet::empty(py)?;
            for e in self.inner.edges_at(v) {
                incident.add(self.inner.key(Kind::Edge, e))?;
            }
            for item in incident.iter() {
                let e = any_of(&self.inner, Kind::Edge, item.extract()?)?;
                if seen.insert(e) {
                    near.push(e);
                }
            }
        }
        Ok(self
            .inner
            .landing_counts(&near, area_eps, crate::hypot_of(hypot)?))
    }

    #[staticmethod]
    fn handle_landing_defects(vertices: &Bound<'_, PyAny>, area_eps: f64, hypot: &str) -> PyResult<[usize;6]> {
        super::landing::handle_counts(vertices, area_eps, crate::hypot_of(hypot)?)
    }

    #[staticmethod]
    fn object_landing_defects(
        edges: Vec<super::landing::Edge>,
        faces: Vec<super::landing::Rings>,
        area_eps: f64,
        hypot: &str,
    ) -> PyResult<[usize; 6]> {
        if faces.iter().any(Vec::is_empty) {
            return Err(PyValueError::new_err("missing outer loop"));
        }
        Ok(super::landing::counts(
            edges,
            faces,
            area_eps,
            crate::hypot_of(hypot)?,
        ))
    }

    #[staticmethod]
    fn increased_landing_defects(before: [i64; 6], after: [i64; 6]) -> Vec<&'static str> {
        super::landing::increased(before, after).collect()
    }

    #[staticmethod]
    fn landing_ring(ring: Vec<(i128, bool)>) -> Vec<usize> {
        let mut order: Vec<_> = (0..ring.len()).collect();
        if ring.iter().any(|&(_, landed)| landed) {
            super::commands::trim_spurs(&mut order, |a, b| ring[a].0 == ring[b].0, |i| ring[i].1);
        }
        order
    }

    fn weld_landing(
        &mut self,
        moving: Vec<u128>,
    ) -> PyResult<Option<(bool, usize, Vec<(u128, u128)>)>> {
        let moving = moving
            .into_iter()
            .filter_map(|k| self.inner.slot(Kind::Vertex, k))
            .collect();
        match self.inner.weld_landing(&moving) {
            Err(OpError::Fallback(_)) => Ok(None),
            result => result.map_err(op_err),
        }
    }

    /// The selected transform carries a texture only when every outer and
    /// inner corner of its face belongs to that transform.
    fn carried_faces(&self, candidates: Vec<u128>, moving: Vec<u128>) -> PyResult<Vec<u128>> {
        let vertices: std::collections::HashSet<_> = moving
            .into_iter()
            .filter_map(|k| self.inner.slot(Kind::Vertex, k))
            .collect();
        let candidates = self.drawing_candidates(Kind::Face, Some(candidates))?;
        Ok(candidates
            .into_iter()
            .filter(|&f| {
                self.inner
                    .face_rings(f)
                    .iter()
                    .flatten()
                    .all(|v| vertices.contains(v))
            })
            .map(|f| self.inner.key(Kind::Face, f))
            .collect())
    }

    #[staticmethod]
    fn carried_object_faces(corners: Vec<Vec<i128>>, moving: std::collections::HashSet<i128>) -> Vec<usize> {
        corners.iter().enumerate().filter_map(|(i, ring)| {
            ring.iter().all(|v| moving.contains(v)).then_some(i)
        }).collect()
    }

    fn pretransform_face_normal(
        &self,
        key: u128,
        inverse: [f64; 16],
        hypot: &str,
    ) -> PyResult<Option<[f64; 3]>> {
        let f = live_of(&self.inner, Kind::Face, key)?;
        let ring = self.inner.outer_ring(f);
        if ring.len() < 3 {
            return Ok(None);
        }
        let matrix = glam::DMat4::from_cols_array(&inverse);
        let points = ring
            .iter()
            .map(|&v| {
                let q = matrix
                    * glam::DVec3::from_array(*self.inner.state.vpos.get(v as usize)).extend(1.);
                super::drawing::vector((q.truncate() / q.w).to_array())
            })
            .collect::<Vec<_>>();
        let indices = (0..points.len() as u32).collect::<Vec<_>>();
        let (normal, length) =
            super::geometry::face_normal_length(&points, &indices, crate::hypot_of(hypot)?);
        Ok((length >= 1e-9).then(|| normal.into()))
    }

    /// Fold all warped faces as one edit. The set of face keys preserves the
    /// historical choice of the first merge among several legal neighbours.
    fn fold_nonplanar(
        &mut self,
        py: Python<'_>,
        moved: Vec<[f64; 3]>,
        tolerance: f64,
        hypot: &str,
    ) -> PyResult<Option<(usize, Vec<(u128, Vec<u128>)>)>> {
        use pyo3::types::PySet;
        let h = crate::hypot_of(hypot)?;
        let faces = self.inner.nonplanar_faces(tolerance);
        if faces.is_empty() {
            return Ok(Some((0, vec![])));
        }
        let moved = moved.into_iter().map(super::commands::supplied_cell).collect();
        let mut work = self.inner.clone();
        let mut copies = Vec::new();
        let mut dirty = 0;
        for f in faces {
            let plan = work.plan_fold(f, &moved, tolerance, h);
            if plan.triangles.len() < 2 {
                continue;
            }
            let mut new = match work.split_fold(f, &plan) {
                Ok(new) => new,
                Err(OpError::Fallback(_)) => return Ok(None),
                Err(e) => return Err(op_err(e)),
            };
            dirty += 1 + 4 * new.len();
            let pieces = PySet::empty(py)?;
            for &k in &new {
                pieces.add(k)?;
            }
            loop {
                let order = pieces
                    .iter()
                    .map(|x| x.extract())
                    .collect::<PyResult<Vec<_>>>()?;
                match work.merge_fold(&order, &plan, h) {
                    Ok(Some((a, b, k, n))) => {
                        pieces.discard(a)?;
                        pieces.discard(b)?;
                        pieces.add(k)?;
                        new.push(k);
                        dirty += n;
                    }
                    Ok(None) => break,
                    Err(OpError::Fallback(_)) => return Ok(None),
                    Err(e) => return Err(op_err(e)),
                }
            }
            work.soften_fold(&plan);
            copies.push((work.key(Kind::Face, f), new));
        }
        self.inner = work;
        Ok(Some((dirty, copies)))
    }

    fn transform_vertices(&mut self, source: Vec<[f64; 3]>, matrix: [f64; 16], collect: bool)
        -> (usize, Vec<u128>) {
        self.inner.transform_vertices(&source, matrix, collect)
    }

    #[staticmethod]
    #[pyo3(signature = (mesh, positions, delta, carry_textures=false))]
    fn command_move_vertices(mesh: &Bound<'_, PyAny>, positions: &Bound<'_, PyAny>, delta: [f64;3], carry_textures: bool)
        -> PyResult<Option<super::command_facade::VertexResult>> {
        let points = positions.try_iter()?
            .map(|p| Ok(crate::math::xyz(&p?)?.to_array()))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(super::command_facade::command_move_vertices(mesh, points, delta, carry_textures)?
            .map(super::command_facade::MovedVertices::fields))
    }

    #[staticmethod]
    #[pyo3(signature = (mesh, positions, matrix, carry_textures=false))]
    fn command_transform_vertices(mesh: &Bound<'_, PyAny>, positions: Vec<[f64;3]>, matrix: [f64;16], carry_textures: bool)
        -> PyResult<Option<super::command_facade::VertexResult>> {
        Ok(super::command_facade::command_transform_vertices(mesh, positions, matrix, carry_textures)?
            .map(super::command_facade::MovedVertices::fields))
    }

    #[pyo3(signature = (source, landing, delta, matrix, carry, hypot, owner))]
    fn _apply_vertex_command(&mut self, py: Python<'_>, source: Vec<[f64;3]>, landing: Vec<[f64;3]>,
        delta: Option<[f64;3]>, matrix: [f64;16], carry: bool, hypot: &str, owner: &Bound<'_, PyDict>)
        -> PyResult<Option<super::command_facade::VertexResult>> {
        self.vertex_command(py, source, landing, delta, matrix, carry, hypot, owner)
    }

    /// The two halves of a loose move bracket the editor's update of texture
    /// dictionaries. Keys always refer to the same enlisted store.
    fn begin_vertex_move(
        &mut self,
        py: Python<'_>,
        source: Vec<[f64; 3]>,
        landing: Vec<[f64; 3]>,
        delta: [f64; 3],
        textures: bool,
        hypot: &str,
    ) -> PyResult<Option<(usize, Vec<u128>, Option<[usize; 6]>)>> {
        use super::commands::{cell, supplied_cell};
        use std::collections::HashSet;
        if !textures {
            if let Some(n) = self.inner.move_without_join(&source, delta) {
                return Ok(Some((n, vec![], None)));
            }
        }
        let source: HashSet<_> = source.into_iter().map(supplied_cell).collect();
        let landing: HashSet<_> = landing.into_iter().map(supplied_cell).collect();
        let mut moving = Vec::new();
        let mut adjacent = Vec::new();
        let mut check = false;
        for v in self.inner.live_slots(Kind::Vertex) {
            let p = *self.inner.state.vpos.get(v as usize);
            if p.iter().any(|x| !x.is_finite() || x.abs() >= 1e11) {
                return Ok(None);
            }
            let key = cell(p);
            let selected = source.contains(&key);
            let destination = landing.contains(&key);
            if selected {
                moving.push(self.inner.key(Kind::Vertex, v));
            }
            if selected || destination {
                adjacent.push(self.inner.key(Kind::Vertex, v));
            }
            check |= destination && !selected;
        }
        let before = if check {
            Some(self.landing_defects(py, adjacent, 1e-10, hypot)?)
        } else {
            None
        };
        self.move_vertices(py, moving.clone(), delta.into())?;
        Ok(Some((moving.len(), moving, before)))
    }

    /// A staged weld or fold that needs object incidence can fall back
    /// before publishing. The outer History record also owns translation.
    fn finish_vertex_move(
        &mut self,
        py: Python<'_>,
        moving: Vec<u128>,
        landing: Vec<[f64; 3]>,
        before: Option<[usize; 6]>,
        tolerance: f64,
        hypot: &str,
    ) -> PyResult<Option<(bool, usize, Vec<(u128, Vec<u128>)>, Option<&'static str>)>> {
        if moving.is_empty() {
            return Ok(self
                .fold_nonplanar(py, landing, tolerance, hypot)?
                .map(|(dirty, copies)| (false, dirty, copies, None)));
        }
        let mut work = Self::wrap(self.inner.clone());
        let Some((joined, dirty, trims)) = work.weld_landing(moving)? else {
            return Ok(None);
        };
        let Some((fold_dirty, folds)) =
            work.fold_nonplanar(py, landing.clone(), tolerance, hypot)?
        else {
            return Ok(None);
        };
        if let (true, Some(before)) = (joined, before) {
            let cells: std::collections::HashSet<_> = landing
                .into_iter()
                .map(super::commands::supplied_cell)
                .collect();
            let near = work
                .inner
                .live_slots(Kind::Vertex)
                .into_iter()
                .filter(|&v| {
                    cells.contains(&super::commands::cell(
                        *work.inner.state.vpos.get(v as usize),
                    ))
                })
                .map(|v| work.inner.key(Kind::Vertex, v))
                .collect();
            let after = work.landing_defects(py, near, 1e-10, hypot)?;
            if let Some(reason) = super::landing::increased(before, after).next() {
                return Ok(Some((false, 0, vec![], Some(reason))));
            }
        }
        let mut copies: Vec<_> = trims.into_iter().map(|(a, b)| (a, vec![b])).collect();
        copies.extend(folds);
        self.inner = work.inner;
        Ok(Some((joined, dirty + fold_dirty, copies, None)))
    }

    fn move_without_join(&mut self, source: Vec<[f64; 3]>, delta: [f64; 3]) -> Option<usize> {
        self.inner.move_without_join(&source, delta)
    }

    fn tag_curve_path(&mut self, points: Vec<[f64; 3]>, closed: bool, id: i64) -> bool {
        self.inner.tag_curve_path(&points, closed, id)
    }

    fn explode_curves(&mut self, ids: std::collections::HashSet<i128>, record: bool) -> Vec<(u128, i128)> {
        let mut prior = Vec::new();
        for e in self.inner.live_slots(Kind::Edge) {
            let id = *self.inner.state.ecurve.get(e as usize);
            if ids.contains(&id) {
                if record { prior.push((self.inner.key(Kind::Edge, e), id)); }
                self.inner.set_edge_curve(e, -1);
            }
        }
        prior
    }

    /// Complete the auto-division of an inserted face. Keep one store revision
    /// per primitive, as published through the protocol before this command.
    fn divide_drawn_face(&mut self, face: u128, holding: Vec<u128>, inside: Vec<u128>, hypot: &str)
        -> PyResult<Option<(usize, Option<u128>, Vec<u128>, bool)>> {
        let f = live_of(&self.inner, Kind::Face, face)?;
        self.divide_face_slots(f, &self.drawing_candidates(Kind::Face, Some(holding))?,
            &self.drawing_candidates(Kind::Face, Some(inside))?, hypot)
    }

    #[staticmethod]
    #[pyo3(signature = (mesh, outer, holes=None, attrs=None, interior=false, divide=true))]
    fn command_add_face(
        mesh: &Bound<'_, PyAny>, outer: Vec<[f64;3]>, holes: Option<Vec<Vec<[f64;3]>>>,
        attrs: Option<&Bound<'_, PyDict>>, interior: bool, divide: bool,
    ) -> PyResult<Option<super::command_facade::FaceResult>> {
        Ok(super::command_facade::command_add_face(mesh, outer, holes, attrs, interior, divide)?
            .map(super::command_facade::AddedFace::fields))
    }

    #[pyo3(signature = (rings, attrs, interior, divide, scan, hypot, owner))]
    fn _add_command_face(
        &mut self, py: Python<'_>, rings: Vec<Vec<[f64;3]>>, attrs: Option<&Bound<'_, PyDict>>,
        interior: bool, divide: bool, scan: bool, hypot: &str, owner: &Bound<'_, PyDict>,
    ) -> PyResult<Option<super::command_facade::FaceResult>> {
        self.add_face_in_frame(py, rings, attrs, interior, divide, scan, hypot, owner)
    }

    #[staticmethod]
    fn command_erase_selection(
        mesh: &Bound<'_, PyAny>, loops: Vec<Vec<[f64;3]>>, segments: Vec<[[f64;3];2]>,
    ) -> PyResult<Option<(Vec<u128>, Vec<u128>, Vec<([f64;3],[f64;3])>, Vec<u128>, u32, Py<PyAny>)>> {
        Ok(super::command_facade::command_erase_selection(mesh, loops, segments)?
            .map(|r| (r.edges, r.faces, r.slit_planes, r.across, r.dirty, r.footprint)))
    }

    #[staticmethod]
    fn command_erase_cleanup(mesh: &Bound<'_, PyAny>, footprint: &Bound<'_, PyAny>)
        -> PyResult<Option<(u32, Vec<u128>, u32, u32)>> {
        Ok(super::command_facade::command_erase_cleanup(mesh, footprint)?
            .map(|r| (r.seams, r.healed_faces, r.orphan_vertices, r.dirty)))
    }

    fn erase_selection(&mut self, loops: Vec<Vec<[f64;3]>>, ends: Vec<[[f64;3];2]>, hypot: &str)
        -> PyResult<Option<(usize, Vec<u128>, Vec<u128>, Vec<(u128,Vec<u128>,bool)>, Vec<super::erase::Plane>, Vec<u128>)>> {
        let h = crate::hypot_of(hypot)?;
        // Publish only a representable edit. The object fallback must see the
        // original incidence, revision and key allocator.
        let mut work = self.inner.clone();
        match work.erase_selection(&loops,&ends,h) {
            Ok(out) => {
                self.inner = work;
                Ok(Some((out.dirty,out.edges,out.faces,out.merges,out.slits,out.across)))
            }
            Err(OpError::Fallback(_)) => Ok(None),
            Err(err) => Err(op_err(err)),
        }
    }

    fn erase_soft_seams(&mut self) -> usize {
        self.inner.erase_soft_seams()
    }

    #[staticmethod]
    #[pyo3(signature = (mesh, segments, cascade_faces=true))]
    fn command_delete_edges(mesh: &Bound<'_, PyAny>, segments: Vec<[[f64;3];2]>, cascade_faces: bool)
        -> PyResult<Option<(Vec<u128>,Vec<u128>,u32)>> {
        Ok(super::command_facade::command_delete_edges(mesh, segments, cascade_faces)?
            .map(|r| (r.edges, r.faces, r.dirty)))
    }

    #[staticmethod]
    fn command_delete_face(mesh: &Bound<'_, PyAny>, outer: Vec<[f64;3]>)
        -> PyResult<Option<(Option<u128>,u32)>> {
        Ok(super::command_facade::command_delete_face(mesh, outer)?.map(|r| (r.face, r.dirty)))
    }

    fn delete_face_loop(&mut self, points: Vec<[f64; 3]>, candidates: Vec<u128>)
        -> PyResult<Option<u128>> {
        let candidates = self.drawing_candidates(Kind::Face, Some(candidates))?;
        use super::commands::FaceDeletion;
        match self.inner.delete_command_face(&points, &candidates) {
            FaceDeletion::Missing => Ok(None),
            FaceDeletion::Removed(key) => Ok(Some(key)),
            FaceDeletion::Blocked => Err(StoreFallback::new_err("deletion would leave the face listed on an edge")),
        }
    }

    fn delete_edge_segments(&mut self, endpoints: Vec<[[f64; 3]; 2]>, candidates: Vec<u128>)
        -> PyResult<Option<(Vec<u128>, Vec<u128>)>> {
        let faces = self.drawing_candidates(Kind::Face, Some(candidates))?;
        Ok(self.inner.delete_command_edges(&endpoints, &faces))
    }

    /// `Mesh.add_edge(a, b)`: `(key, change)`.
    fn add_edge(&mut self, a: (f64, f64, f64), b: (f64, f64, f64)) -> PyResult<(u128, PyChange)> {
        let (e, ch) = self.op(|s, ch| s.add_edge(p3(a), p3(b), ch))?;
        Ok((self.inner.key(Kind::Edge, e), ch))
    }

    #[staticmethod]
    fn project_sweep(profile: super::sweep::Station, path: Vec<[f64;3]>, closed: bool, hypot: &str, compensated: bool)
        -> PyResult<Option<(Vec<super::sweep::Station>,Vec<[f64;3]>)>> {
        Ok(super::sweep::projected_stations(profile, &path, closed, crate::hypot_of(hypot)?, compensated))
    }

    #[staticmethod]
    fn sweep_polygons(stations: Vec<super::sweep::Station>, closed: bool, start_cap: bool, hypot: &str)
        -> PyResult<Vec<super::sweep::Station>> {
        let mut faces = Vec::new();
        super::sweep::emit_sweep_faces(&stations, closed, crate::hypot_of(hypot)?, None, start_cap,
            |rings| { faces.push(rings.to_vec()); Ok(()) }).map_err(op_err)?;
        Ok(faces)
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

    fn set_curve_run(&mut self, keys: Vec<u128>, curve: i128) -> PyResult<PyChange> {
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

    fn offset_outline(&self, key: u128, normal: [f64; 3], distance: f64, hypot: &str)
        -> PyResult<Vec<super::intersect::Segment>> {
        let face = live_of(&self.inner, Kind::Face, key)?;
        Ok(self.inner.offset_outline(face, normal, distance, crate::hypot_of(hypot)?))
    }

    #[staticmethod]
    fn offset_segments(points: Vec<[f64;3]>, normal: [f64;3], distance: f64, hypot: &str, closed: bool)
        -> PyResult<Vec<([f64;3],[f64;3])>> {
        Ok(super::offset::outline(&points, normal, distance, crate::hypot_of(hypot)?, closed))
    }

    #[staticmethod]
    fn keep_offset_outlines(boundary: Vec<[f64;3]>, regions: Vec<super::offset::Ring>, normal: [f64;3], distance: f64, hypot: &str)
        -> PyResult<Vec<bool>> {
        let outline = boundary.into_iter().map(super::drawing::vector).collect::<Vec<_>>();
        Ok(super::offset::keep_outline(&outline, regions, normal, distance, crate::hypot_of(hypot)?))
    }

    #[staticmethod]
    fn offset_layout(source: super::offset::Ring, holes: Vec<super::offset::Ring>, regions: Vec<super::offset::Ring>,
                     normal: [f64;3], distance: f64, hypot: &str) -> PyResult<Option<Vec<super::offset::Region>>> {
        Ok(super::offset::layout(source, holes, regions, normal, distance, crate::hypot_of(hypot)?))
    }

    fn offset_hole_targets(&self, key: u128, regions: Vec<Vec<[f64; 3]>>, normal: [f64; 3],
                           hypot: &str) -> PyResult<Option<Vec<i64>>> {
        let face = live_of(&self.inner, Kind::Face, key)?;
        Ok(super::offset::place_holes(&self.inner, face, &regions, normal, crate::hypot_of(hypot)?))
    }

    fn keep_offset_regions(&self, key: u128, regions: Vec<Vec<[f64; 3]>>, normal: [f64; 3],
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
    #[staticmethod]
    pub(super) fn maximal_holes(holes: Vec<Vec<[f64; 3]>>, hypot: &str) -> PyResult<Vec<bool>> {
        if holes.len() < 2 {
            return Ok(vec![true; holes.len()]);
        }
        if holes.iter().any(|r| r.len() < 3) {
            return Err(pyo3::exceptions::PyIndexError::new_err("hole has fewer than three corners"));
        }
        let rings = holes.into_iter().map(|r| r.into_iter().map(super::drawing::vector).collect()).collect::<Vec<_>>();
        Ok(super::fillet::maximal_holes(&rings, crate::hypot_of(hypot)?))
    }

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

    /// Explode (`ExplodeGroupCommand`'s geometry, store/explode.rs): the
    /// faces and edges of `source` (a snapshot of the group's store) copied
    /// in, `mapped` (V, 3) its vertices in list order through the
    /// placement, `face_attr` per source face the new face's attribute
    /// value (table index + 1, 0 none). `((face, vertex, edge) key maps:
    /// per source element in list order its key here or None, change)`.
    #[allow(clippy::type_complexity)]
    fn paste_from(&mut self, source: &PyStoreSnapshot, mapped: PyReadonlyArray2<'_, f64>,
                  face_attr: Vec<u32>, hole_filter: &Bound<'_, PyAny>)
                  -> PyResult<((Vec<u128>, Vec<Option<u128>>, Vec<Option<u128>>), PyChange)> {
        let src = reader(&source.inner);
        let flat = mapped.as_slice().map_err(|_| PyValueError::new_err("mapped: contiguous (V, 3)"))?;
        let pts: Vec<[f64; 3]> = flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        let vl = src.live_slots(Kind::Vertex);
        let mut dense = vec![super::keys::NONE; src.state.slots[0].len()];
        for (i, &v) in vl.iter().enumerate() {
            dense[v as usize] = i as u32;
        }
        let fl = src.live_slots(Kind::Face);
        let keep = super::pieces::hole_masks(&src, &fl, |v| pts.get(dense[v as usize] as usize).copied().unwrap_or([0.0; 3]), hole_filter)?;
        let (made, ch) = self.op(|s, ch| s.paste_from(&src, &pts, &keep, &face_attr, ch))?;
        let st = &self.inner;
        let opt = |k: Kind, x: u32| (x != super::keys::NONE).then(|| st.key(k, x));
        Ok(((made.faces.iter().map(|&f| st.key(Kind::Face, f)).collect(),
             made.vertices.iter().map(|&v| opt(Kind::Vertex, v)).collect(),
             made.edges.iter().map(|&e| opt(Kind::Edge, e)).collect()), ch))
    }

    /// The broad phase of `_merge_overlapping_planes` after an explode
    /// (store/explode.rs): `(added faces with a normal, [(old face key,
    /// indices into that list)])`.
    fn plane_merge_candidates(&self, added: Vec<u128>, tol: f64, hypot: &str) -> PyResult<(Vec<u128>, Vec<(u128, Vec<u32>)>)> {
        let st = &self.inner;
        let fs: Vec<u32> = added.iter().filter_map(|&k| st.slot(Kind::Face, k)).collect();
        let (new, pairs) = st.plane_merge_candidates(&fs, tol, crate::hypot_of(hypot)?);
        Ok((new.iter().map(|&f| st.key(Kind::Face, f)).collect(),
            pairs.into_iter().map(|(f, hits)| (st.key(Kind::Face, f), hits)).collect()))
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

    fn plan_softening(
        &self,
        keys: &Bound<'_, PyAny>,
        angle: f64,
        coplanar: bool,
        harden: bool,
        smooth: bool,
        matches: &Bound<'_, PyAny>,
        hypot: &str,
    ) -> PyResult<Vec<super::soften::Change<u128>>> {
        let store = &self.inner;
        let h = crate::hypot_of(hypot)?;
        let mut selected = std::collections::HashSet::new();
        for key in keys.try_iter()? {
            if let Ok(k) = key?.extract::<u128>() {
                selected.insert(k);
            }
        }
        let rows = store.live_slots(Kind::Edge).into_iter().filter_map(|e| {
            let key = store.key(Kind::Edge, e);
            if !selected.contains(&key) {
                return None;
            }
            let &[a, b] = store.edge_faces(e) else {
                return None;
            };
            let bits = *store.state.eflags.get(e as usize);
            Some((
                e,
                (bits & super::SOFT != 0, bits & super::FLAT != 0),
                [a, b],
            ))
        });
        let opts = super::soften::Options {
            angle,
            coplanar,
            harden,
            smooth,
        };
        let changes = opts.plan(
            rows,
            |a, b| {
                Ok::<_, PyErr>(
                    matches.is_none()
                        || matches
                            .call1((store.key(Kind::Face, a), store.key(Kind::Face, b)))?
                            .is_truthy()?,
                )
            },
            |f| Ok(store.face_normal(f, h).into()),
        )?;
        Ok(changes
            .into_iter()
            .map(|(e, b, a)| (store.key(Kind::Edge, e), b, a))
            .collect())
    }

    #[staticmethod]
    fn plan_object_softening(
        rows: Vec<super::soften::Edge<i128>>,
        angle: f64,
        coplanar: bool,
        harden: bool,
        smooth: bool,
        matches: &Bound<'_, PyAny>,
        normal: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<super::soften::Change<i128>>> {
        let opts = super::soften::Options {
            angle,
            coplanar,
            harden,
            smooth,
        };
        opts.plan(
            rows,
            |a, b| Ok(matches.is_none() || matches.call1((a, b))?.is_truthy()?),
            |f| normal.call1((f,))?.extract(),
        )
    }

    fn apply_softening(&mut self, rows: Vec<super::soften::Change<u128>>, after: bool) {
        for (key, old, new) in rows {
            let Some(e) = self.inner.slot(Kind::Edge, key) else {
                continue;
            };
            let (soft, flat) = if after { new } else { old };
            let current = *self.inner.state.eflags.get(e as usize);
            // Observable store revision advances once per facade field write.
            let bits = if soft {
                current | super::SOFT
            } else {
                current & !super::SOFT
            };
            self.inner.set_edge_flags(e, bits);
            self.inner.set_edge_flags(
                e,
                if flat {
                    bits | super::FLAT
                } else {
                    bits & !super::FLAT
                },
            );
        }
    }

    #[staticmethod]
    fn apply_object_softening(
        edges: &Bound<'_, PyAny>,
        rows: Vec<super::soften::Change<i128>>,
        after: bool,
    ) -> PyResult<()> {
        let wanted: std::collections::HashMap<_, _> = rows
            .into_iter()
            .map(|(k, b, a)| (k, if after { a } else { b }))
            .collect();
        for e in edges.try_iter()? {
            let e = e?;
            let key: i128 = e.getattr("key")?.extract()?;
            if let Some(&(soft, flat)) = wanted.get(&key) {
                e.setattr("soft", soft)?;
                e.setattr("flat", flat)?;
            }
        }
        Ok(())
    }

    #[staticmethod]
    fn bevel_face_curve(
        origin: [f64; 3],
        a: [f64; 3],
        b: [f64; 3],
        exponent: f64,
        segments: usize,
    ) -> PyResult<Vec<[f64; 3]>> {
        use super::geometry::V3;
        let v = |[x, y, z]: [f64; 3]| V3(x, y, z);
        if segments == 0 {
            return Err(PyValueError::new_err("empty bevel section"));
        }
        Ok(
            super::bevel::face_curve(v(origin), v(a), v(b), exponent, segments)
                .into_iter()
                .map(super::fillet::xyz)
                .collect(),
        )
    }

    #[staticmethod]
    fn bevel_corner(
        origin: [f64; 3],
        vectors: Vec<[f64; 3]>,
        curves: Vec<Vec<[f64; 3]>>,
        normal: [f64; 3],
        segments: usize,
        exponent: f64,
        hypot: &str,
        compensated: bool,
    ) -> PyResult<(Vec<Vec<[f64; 3]>>, Vec<([f64; 3], [f64; 3])>)> {
        use super::fillet::{xyz, RoundPlan};
        use super::geometry::V3;
        let v = |[x, y, z]: [f64; 3]| V3(x, y, z);
        if segments == 0
            || curves
                .iter()
                .any(|r| r.len().checked_sub(1) != Some(segments))
        {
            return Err(PyValueError::new_err("inconsistent bevel sections"));
        }
        let vectors: Vec<_> = vectors.into_iter().map(v).collect();
        let curves: Vec<_> = curves
            .into_iter()
            .map(|r| r.into_iter().map(v).collect())
            .collect();
        let mut plan = RoundPlan::default();
        super::bevel::close_corner(
            &mut plan,
            v(origin),
            &vectors,
            &curves,
            v(normal),
            segments,
            exponent,
            crate::hypot_of(hypot)?,
            compensated,
        )
        .map_err(|(message, _)| PyValueError::new_err(message))?;
        Ok((
            plan.patches
                .into_iter()
                .map(|r| r.into_iter().map(xyz).collect())
                .collect(),
            plan.soft
                .into_iter()
                .map(|(a, b)| (xyz(a), xyz(b)))
                .collect(),
        ))
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
        if plan.lineage != Some(self.inner.lineage) || plan.revision != self.inner.revision {
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
    fn collapse_collinear(&mut self, hypot: &str) -> PyResult<((usize, usize), PyChange)> {
        let h = crate::hypot_of(hypot)?;
        self.op_all(|store, change| {
            let (count, bumps, blocked) = store.collapse_collinear(h, change);
            if blocked.is_some() {
                return Err(OpError::Fallback("a collinear collapse would break incidence"));
            }
            Ok((count, bumps))
        })
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

    /// `run_stitch`'s phases 0–2 as one op (weld, dedupe, the scoped
    /// T-junction rounds, the collinear collapse):
    /// `(((dropped, kept)…), bumps), change)`. All or nothing: a step the
    /// store refuses puts the state back (StoreFallback).
    #[allow(clippy::type_complexity)]
    #[pyo3(signature = (seeds, dedupe, tolerance, hypot))]
    fn stitch_connect(
        &mut self,
        seeds: Vec<[f64; 3]>,
        dedupe: bool,
        tolerance: f64,
        hypot: &str,
    ) -> PyResult<((Vec<(u128, u128)>, usize), PyChange)> {
        let h = crate::hypot_of(hypot)?;
        let (r, ch) = self.op_all(|s, ch| s.stitch_connect(&seeds, dedupe, tolerance, h, ch))?;
        let s = &self.inner;
        let pairs = r.deduped.iter().map(|&(d, k)| (s.key(Kind::Face, d), s.key(Kind::Face, k))).collect();
        Ok(((pairs, r.bumps), ch))
    }

    /// The push's plane-rebuild fixpoint and its trailing stitch, native
    /// (`store/push_rebuild.rs`). The fresh faces start as `listed` (keys
    /// in list order) or as the faces of `now` not in `before`. `own` and
    /// `rows`: the paint class of each face's own dict and of each attribute
    /// row (0: none). `((painted [(face, class)], deduped [(dropped, kept)],
    /// bumps), change)`; all or nothing (StoreFallback).
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    #[pyo3(signature = (listed, now, before, seeds, keep, removing, own, rows, compensated, hypot))]
    fn rebuild_push_planes(
        &mut self,
        listed: Option<Vec<u128>>,
        now: Option<Vec<u128>>,
        before: Option<Vec<u128>>,
        seeds: Vec<[f64; 3]>,
        keep: bool,
        removing: bool,
        own: std::collections::HashMap<u128, u32>,
        rows: Vec<u32>,
        compensated: bool,
        hypot: &str,
    ) -> PyResult<((Vec<(u128, u32)>, Vec<(u128, u128)>, usize), PyChange)> {
        let h = crate::hypot_of(hypot)?;
        let fresh = match (listed, now, before) {
            (Some(keys), _, _) => super::Fresh::Listed(keys),
            (None, Some(now), Some(before)) => super::Fresh::Since { now, before },
            _ => return Err(PyValueError::new_err("listed, or now and before")),
        };
        let mut paint = super::Paint::new(own, rows);
        let (r, ch) = self.op_all(|s, ch| {
            s.rebuild_push_planes(fresh, &seeds, keep, removing, &mut paint, compensated, h, ch)
        })?;
        Ok(((r.painted, r.deduped, r.bumps), ch))
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

    fn replace_plane_faces(
        &mut self,
        old: Vec<u128>,
        caps: Vec<super::caps::Cap>,
    ) -> PyResult<(Vec<u128>, PyChange)> {
        let faces: PyResult<Vec<_>> = old.iter().map(|&k| slot_of(&self.inner, Kind::Face, k)).collect();
        let faces = faces?;
        if faces
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != faces.len()
        {
            return Err(PyValueError::new_err("duplicate face key"));
        }
        self.op_all(|s, ch| s.replace_caps(&faces, &caps, ch))
    }

    fn prune_plane_debris(
        &mut self,
        origin: [f64; 3],
        normal: [f64; 3],
        tolerance: f64,
        hypot: &str,
    ) -> PyResult<PyChange> {
        let h = crate::hypot_of(hypot)?;
        Ok(self
            .op(|s, ch| s.prune_plane_debris(origin, normal, tolerance, h, ch))?
            .1)
    }

    fn add_welded_face(&mut self, rings: Vec<Vec<u128>>) -> PyResult<(u128, PyChange)> {
        let slots = rings.into_iter().map(|ring| ring.into_iter()
            .map(|k| live_of(&self.inner, Kind::Vertex, k)).collect::<PyResult<Vec<_>>>())
            .collect::<PyResult<Vec<_>>>()?;
        let (face, change) = self.op(|s, ch| {
            let f = s.add_welded_face(&slots, ch)?;
            for &key in &ch.created[Kind::Edge as usize] {
                let edge = s.slot(Kind::Edge, key).unwrap();
                for &v in s.state.ends.get(edge as usize) {
                    ch.modified[0].push(s.key(Kind::Vertex, v));
                }
            }
            Ok(f)
        })?;
        Ok((self.inner.key(Kind::Face, face), change))
    }

    #[staticmethod]
    #[pyo3(signature=(namespace, pos, scale=10_000., tol2=1e-8, fused=false))]
    fn _bulk_points<'py>(
        py: Python<'py>,
        namespace: u64,
        pos: PointArray<'py>,
        scale: f64,
        tol2: f64,
        fused: bool,
    ) -> PyResult<(Self, Bound<'py, PyArray1<i64>>)> {
        let pos = match &pos {
            PointArray::Single(p) => BlockPoints::Single(triples(p)?),
            PointArray::Double(p) => BlockPoints::Double(triples(p)?),
        };
        let block = Block {
            pos,
            ..Block::default()
        };
        let (store, indices) = py
            .detach(|| Store::from_block_indices(namespace, &block, Weld { scale, tol2, fused }))
            .map_err(PyValueError::new_err)?;
        Ok((Self::wrap(store), indices.into_pyarray(py)))
    }

    /// Validate every input before installing the next prefix of keys.
    #[pyo3(signature=(ia,ib,flags=None))]
    fn _bulk_edges(
        &mut self,
        py: Python<'_>,
        ia: PyReadonlyArray1<'_, i64>,
        ib: PyReadonlyArray1<'_, i64>,
        flags: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let ia = slice(&ia, "ia")?;
        let ib = slice(&ib, "ib")?;
        if ia.len() != ib.len() {
            return Err(PyValueError::new_err(
                "edge endpoint columns differ in length",
            ));
        }
        let n = ia.len();
        let nv = self.inner.state.vpos.len() as i64;
        let index = |i: i64| if i < 0 { i + nv } else { i };
        let edges: Vec<_> = ia
            .iter()
            .zip(ib)
            .map(|(&a, &b)| [index(a), index(b)])
            .collect();
        let mut bits = vec![0; n];
        let mut curves = vec![-1; n];
        let mut layers = vec![-1; n];
        let mut names = Vec::new();
        let mut by_name = std::collections::HashMap::new();
        if let Some(rows) = flags {
            for (i, row) in rows.try_iter()?.enumerate() {
                if i >= n {
                    return Err(pyo3::exceptions::PyIndexError::new_err(i));
                }
                let row = row?;
                let len = row.len()?;
                let soft = row.get_item(0)?.is_truthy()?;
                let curve = row.get_item(1)?;
                let layer = row.get_item(2)?;
                bits[i] = u8::from(soft);
                if len > 3 && row.get_item(3)?.is_truthy()? {
                    bits[i] |= super::HIDDEN;
                }
                if len > 4 && row.get_item(4)?.is_truthy()? {
                    bits[i] |= super::FLAT;
                }
                if !curve.is_none() {
                    curves[i] = py
                        .get_type::<pyo3::types::PyInt>()
                        .call1((curve,))?
                        .extract()?;
                }
                if !layer.is_none() {
                    let Ok(name) = layer.extract::<String>() else {
                        return Ok(false);
                    };
                    let id = *by_name.entry(name.clone()).or_insert_with(|| {
                        let id = names.len() as i64;
                        names.push(name);
                        id
                    });
                    layers[i] = id;
                }
            }
        }
        let block = Block {
            edges: &edges,
            flags: Some(&bits),
            curves: Some(&curves),
            layers: Some(&layers),
            layer_names: names,
            ..Block::default()
        };
        let store = py
            .detach(|| self.inner.staged_block(&block))
            .map_err(PyValueError::new_err)?;
        self._extend_bulk(&Self::wrap(store))?;
        Ok(true)
    }

    #[pyo3(signature=(rings,sizes,counts,attrs=None))]
    fn _bulk_faces(
        &mut self,
        py: Python<'_>,
        rings: PyReadonlyArray1<'_, i64>,
        sizes: PyReadonlyArray1<'_, i64>,
        counts: PyReadonlyArray1<'_, i64>,
        attrs: Option<Vec<i64>>,
    ) -> PyResult<()> {
        let rings = slice(&rings, "rings")?;
        let sizes = slice(&sizes, "sizes")?;
        let counts = slice(&counts, "counts")?;
        let state = &self.inner.state;
        let store = py
            .detach(|| {
                let nv = state.vpos.len() as i64;
                let rings: Vec<_> = rings
                    .iter()
                    .map(|&i| if i < 0 { i + nv } else { i })
                    .collect();
                let edges: Vec<_> = state.ends.iter().map(|ends| ends.map(i64::from)).collect();
                let flags = state.eflags.to_vec();
                let curves = state.ecurve.to_vec();
                let layers: Vec<_> = state
                    .elayer
                    .iter()
                    .map(|&i| {
                        if i == super::keys::NONE {
                            -1
                        } else {
                            i64::from(i)
                        }
                    })
                    .collect();
                self.inner.staged_block(&Block {
                    edges: &edges,
                    flags: Some(&flags),
                    curves: Some(&curves),
                    layers: Some(&layers),
                    layer_names: state.layers.as_ref().clone(),
                    rings: &rings,
                    sizes,
                    counts,
                    face_attr: attrs.as_deref(),
                    ..Block::default()
                })
            })
            .map_err(PyValueError::new_err)?;
        self._extend_bulk(&Self::wrap(store))
    }

    /// Internal to the staged block builder. Columns for the next stage
    /// have the preceding stage's slot order as a prefix. Install them in
    /// the live handle so snapshots and discarded branches share one HWM.
    fn _extend_bulk(&mut self, block: &PyMeshStore) -> PyResult<()> {
        for kind in Kind::ALL {
            let i = kind as usize;
            let slots = &self.inner.state.slots[i];
            if slots.live != slots.len() || self.inner.hwm[i].len != slots.len()
                || !self.inner.hwm[i].over.is_empty() || block.inner.n(kind) < slots.len()
                || block.inner.revision != 0 {
                return Err(PyValueError::new_err("bulk stages must append fresh slots"));
            }
        }
        let ns = self.inner.state.ns;
        self.inner.state = block.inner.state.clone();
        self.inner.state.ns = ns;
        self.inner.hwm = block.inner.hwm.clone();
        self.inner.revision += 1;
        Ok(())
    }

    /// Clear live geometry in the current lineage; prior snapshots retain their keys.
    fn clear(&mut self) -> PyResult<PyChange> {
        let empty = super::State { ns: self.inner.state.ns, ..Default::default() };
        let before = self.inner.state.clone();
        let snap = Snapshot { state: empty, revision: self.inner.revision, lineage: self.inner.lineage };
        self.inner.restore(&snap, None).map_err(PyValueError::new_err)?;
        Ok(self.net_change(&before))
    }

    fn apply_orientation(&mut self, marks: Vec<(u128, bool)>, flips: Vec<u128>) -> PyResult<()> {
        let marks = marks.into_iter()
            .map(|(key, interior)| Ok((live_of(&self.inner, Kind::Face, key)?, interior)))
            .collect::<PyResult<Vec<_>>>()?;
        let flips = flips.into_iter().map(|key| live_of(&self.inner, Kind::Face, key))
            .collect::<PyResult<Vec<_>>>()?;
        self.inner.apply_orientation(&marks, &flips);
        Ok(())
    }

    fn reverse_faces(&mut self, keys: Vec<u128>) -> PyResult<PyChange> {
        let slots = keys.into_iter().map(|k| live_of(&self.inner, Kind::Face, k))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(self.op(|s, ch| {
            s.reverse_faces(slots, ch);
            Ok(())
        })?.1)
    }

    /// `turn_face_over`'s ring reversal (outer and holes).
    fn reverse_ring(&mut self, key: u128, ring: usize) -> PyResult<()> {
        let face = any_of(&self.inner, Kind::Face, key)?;
        let mut change = Change::default();
        if !self.inner.reverse_ring(face, ring, &mut change) {
            return Err(pyo3::exceptions::PyIndexError::new_err("face ring no longer exists"));
        }
        self.inner.finish(change);
        Ok(())
    }

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
    #[getter]
    fn frame_origin(&self) -> [f64; 3] {
        self.inner.state.frame.origin
    }

    #[getter]
    fn _model_frame(&self) -> bool {
        self.inner.state.frame.is_model()
    }

    fn _definition_points<'py>(&self, py: Python<'py>, points: PyReadonlyArray2<'_, f64>)
        -> PyResult<Bound<'py, PyArray2<f64>>> {
        let rows: Vec<_> = triples(&points)?.iter().map(|p|
            self.inner.state.frame.definition_point(*p)).collect();
        Ok(rows3(py, &rows))
    }

    #[getter]
    fn frame_matrix(&self) -> Option<[f64; 16]> {
        self.inner.state.frame.matrix()
    }

    fn tool_placement(&self) -> Option<crate::math::Matrix4> {
        let frame = &self.inner.state.frame;
        let mut matrix = frame.matrix()?;
        for (value, origin) in matrix[12..15].iter_mut().zip(frame.origin) {
            *value -= origin;
        }
        Some(crate::math::Matrix4::from(glam::DMat4::from_cols_array(&matrix)))
    }

    fn _in_model_frame(&self, origin: [f64; 3]) -> Self {
        let mut store = self.inner.clone();
        let frame = super::frame::Frame::default().rebased(origin);
        store.state = store.state.in_frame(frame, None);
        Self::wrap(store)
    }

    fn _in_frame(&self, origin: [f64; 3]) -> Self {
        let mut st = self.inner.clone();
        st.state = st.state.in_frame(st.state.frame.rebased(origin), None);
        Self::wrap(st)
    }

    /// Column-major definition-to-world placement and camera origin, both f64.
    fn _in_instance_frame(&self, matrix: [f64; 16], origin: [f64; 3]) -> PyResult<Self> {
        let frame = super::frame::Frame::instance(matrix, origin)
            .ok_or_else(|| PyValueError::new_err("expected a finite invertible affine placement"))?;
        let mut store = self.inner.clone();
        store.state = store.state.in_frame(frame, None);
        Ok(Self::wrap(store))
    }

    fn _instance_view<'py>(slf: &Bound<'py, Self>, mesh: Py<PyAny>, matrix: [f64; 16], origin: [f64; 3])
        -> PyResult<Bound<'py, InstanceStore>> {
        let base = slf.try_borrow()?._in_instance_frame(matrix, origin)?;
        let instance = InstanceStore { definition: slf.clone().unbind(), mesh };
        Bound::new(slf.py(), (instance, base))
    }

    fn _publish_frame(&mut self, source: &PyMeshStore) -> PyResult<()> {
        if self.inner.lineage != source.inner.lineage {
            return Err(PyValueError::new_err("coordinate frames must share a lineage"));
        }
        let state = source.inner.state.in_frame(self.inner.state.frame.clone(), Some(&self.inner.state));
        self.inner = source.inner.clone();
        self.inner.state = state;
        *self.cache.lock().unwrap() = Default::default();
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> PyStoreSnapshot {
        PyStoreSnapshot { inner: self.inner.snapshot(), definition: None }
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
        snapshot.inner.lineage == self.inner.lineage && self.inner.state.same_as(
            &snapshot.inner.state.in_frame(self.inner.state.frame.clone(), Some(&self.inner.state)))
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


fn curve_array(py: Python<'_>, values: Vec<i128>) -> PyResult<Bound<'_, PyAny>> {
    if let Ok(compact) = values.iter().copied().map(i64::try_from).collect::<Result<Vec<_>, _>>() {
        return Ok(compact.into_pyarray(py).into_any());
    }
    py.import("numpy")?.call_method1("array", (values, "object"))
}

fn dense_dict<'py>(py: Python<'py>, d: super::read::Dense) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("pos", rows3(py, &d.pos))?;
    let eflat: Vec<i64> = d.edges.iter().flat_map(|e| e.iter().copied()).collect();
    out.set_item("edges", Array2::from_shape_vec((d.edges.len(), 2), eflat).unwrap().into_pyarray(py))?;
    out.set_item("flags", d.flags.into_pyarray(py))?;
    out.set_item("curve", curve_array(py, d.curve)?)?;
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
    pub(crate) fn canonical(&self) -> Self {
        Self { inner: self.definition.as_deref().unwrap_or(&self.inner).clone(), definition: None }
    }

    fn cap_scope(&self, py: Python<'_>, seeds: Vec<u128>) -> (Vec<(u128, u32)>, Vec<u128>) {
        let store = reader(&self.inner);
        py.detach(|| store.cap_scope(&seeds))
    }

    #[pyo3(signature = (hypot, compensated, near=None))]
    fn crack_planes(
        &self,
        py: Python<'_>,
        hypot: &str,
        compensated: bool,
        near: Option<[f64; 6]>,
    ) -> PyResult<Vec<([f64; 3], [f64; 3])>> {
        let store = reader(&self.inner);
        let h = crate::hypot_of(hypot)?;
        Ok(py.detach(|| store.crack_planes(near, h, compensated)))
    }

    fn seam_planes(
        &self,
        py: Python<'_>,
        keys: Vec<u128>,
        tolerance: f64,
        hair_cos2: f64,
        hypot: &str,
        compensated: bool,
    ) -> PyResult<Vec<super::caps::Seam>> {
        let store = reader(&self.inner);
        let faces = keys
            .into_iter()
            .filter_map(|k| store.slot(Kind::Face, k))
            .collect::<Vec<_>>();
        let h = crate::hypot_of(hypot)?;
        Ok(py.detach(|| store.seam_planes(&faces, tolerance / 2., hair_cos2, h, compensated)))
    }

    fn edges_along(
        &self,
        py: Python<'_>,
        segments: Vec<[[f64; 3]; 2]>,
        tolerance: f64,
        hypot: &str,
    ) -> PyResult<Vec<u128>> {
        let store = reader(&self.inner);
        let h = crate::hypot_of(hypot)?;
        Ok(py.detach(|| store.edges_along(&segments, tolerance, h)))
    }

    fn triangles_by_face(
        &self,
        py: Python<'_>,
        keys: Vec<u128>,
        hypot: &str,
    ) -> PyResult<Vec<Option<Vec<[[f64; 3]; 3]>>>> {
        let store = reader(&self.inner);
        let h = crate::hypot_of(hypot)?;
        Ok(py.detach(|| {
            keys.into_iter()
                .map(|k| {
                    store
                        .slot(Kind::Face, k)
                        .map(|f| store.face_triangles(f, None, h))
                })
                .collect()
        }))
    }

    #[staticmethod]
    fn trace_cap_outlines(
        py: Python<'_>,
        regions: Vec<super::cap_outline::Region>,
        creases: Vec<[[f64; 2]; 2]>,
        tolerance: f64,
    ) -> Option<Vec<Vec<[f64; 2]>>> {
        py.detach(|| super::cap_outline::trace(regions, &creases, tolerance))
    }

    #[pyo3(signature = (origin, normal, probes, fresh, tolerance, hypot, compensated, scope=None))]
    fn cap_coverage(
        &self,
        py: Python<'_>,
        origin: [f64; 3],
        normal: [f64; 3],
        probes: Vec<(f64, f64)>,
        fresh: Vec<u128>,
        tolerance: f64,
        hypot: &str,
        compensated: bool,
        scope: Option<Vec<u128>>,
    ) -> PyResult<Vec<(bool, u8, u8)>> {
        let store = reader(&self.inner);
        let h = crate::hypot_of(hypot)?;
        let slots = |keys: Vec<u128>| {
            keys.into_iter()
                .map(|k| slot_of(&store, Kind::Face, k))
                .collect::<PyResult<Vec<_>>>()
        };
        let scope = match scope {
            Some(keys) => slots(keys)?,
            None => store.live_slots(Kind::Face),
        };
        let fresh = slots(fresh)?;
        Ok(py.detach(|| {
            store.cap_coverage(&scope, &fresh, origin, normal, &probes, tolerance, h, compensated)
        }))
    }

    #[pyo3(signature = (origin, normal, tolerance, hypot, keys=None))]
    fn cap_segments(
        &self,
        py: Python<'_>,
        origin: [f64; 3],
        normal: [f64; 3],
        tolerance: f64,
        hypot: &str,
        keys: Option<Vec<u128>>,
    ) -> PyResult<Vec<(u128, [[f64; 3]; 2])>> {
        use crate::store::geometry::V3;
        let store = reader(&self.inner);
        let h = crate::hypot_of(hypot)?;
        let edges = match keys {
            Some(keys) => keys
                .into_iter()
                .map(|k| slot_of(&store, Kind::Edge, k))
                .collect::<PyResult<_>>()?,
            None => store.live_slots(Kind::Edge),
        };
        let o = V3(origin[0], origin[1], origin[2]);
        let n = V3(normal[0], normal[1], normal[2]);
        Ok(py.detach(|| {
            edges
                .into_iter()
                .filter(|&e| store.edge_on_cap_plane(e, o, n, tolerance, h))
                .map(|e| {
                    (
                        store.key(Kind::Edge, e),
                        store
                            .state
                            .ends
                            .get(e as usize)
                            .map(|v| *store.state.vpos.get(v as usize)),
                    )
                })
                .collect()
        }))
    }

    fn edge_defects(&self, py: Python<'_>) -> [Vec<u128>; 3] {
        let store = reader(&self.inner);
        py.detach(|| store.edge_defects())
    }

    #[pyo3(signature = (hypot, keys=None))]
    fn indexed_surface<'py>(
        &self,
        py: Python<'py>,
        hypot: &str,
        keys: Option<Vec<u128>>,
    ) -> PyResult<(
        Bound<'py, PyArray2<f64>>,
        Bound<'py, PyArray2<u32>>,
        Bound<'py, PyArray1<u32>>,
        Vec<u128>,
    )> {
        let store = reader(&self.inner);
        let h = crate::hypot_of(hypot)?;
        let faces = match keys {
            Some(keys) => keys
                .into_iter()
                .map(|k| slot_of(&store, Kind::Face, k))
                .collect::<PyResult<_>>()?,
            None => store.live_slots(Kind::Face),
        };
        let surface = py.detach(|| store.indexed_surface(&faces, h));
        Ok((
            rows3(py, &surface.positions),
            rows3(py, &surface.triangles),
            surface.owners.into_pyarray(py),
            surface.faces,
        ))
    }

    fn seed_orientation(&self, py: Python<'_>, key: u128) -> PyResult<(Vec<u128>, usize)> {
        let store = reader(&self.inner);
        let face = slot_of(&store, Kind::Face, key)?;
        Ok(py.detach(|| store.seed_orientation(face)))
    }

    #[pyo3(signature = (key, hypot, normal=None))]
    fn face_triangles(
        &self,
        key: u128,
        hypot: &str,
        normal: Option<[f64; 3]>,
    ) -> PyResult<Vec<[[f64; 3]; 3]>> {
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
        let faces: PyResult<Vec<_>> = keys.into_iter().map(|k| slot_of(&store, Kind::Face, k)).collect();
        let faces = faces?;
        let h = crate::hypot_of(hypot)?;
        Ok(py.detach(|| store.face_set_volume(&faces, h)))
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
        self.inner.lineage == other.inner.lineage && self.inner.state.same_as(
            &other.inner.state.in_frame(self.inner.state.frame.clone(), Some(&self.inner.state)))
    }

    /// Net change between two immutable snapshots of one store. Across a
    /// foreign restore adopted under a fresh namespace (same store, new
    /// keys: `Scene.share_back`) everything before is removed and
    /// everything after created (store/diff.rs). Snapshots of two
    /// different stores refuse.
    fn changes_since(&self, py: Python<'_>, before: &PyStoreSnapshot, attrs: Vec<u128>) -> PyResult<PyChange> {
        if self.inner.lineage != before.inner.lineage {
            return Err(PyValueError::new_err("snapshots of different stores"));
        }
        let (new, mut old) = (reader(&self.inner), reader(&before.inner));
        old.state = old.state.in_frame(new.state.frame.clone(), Some(&new.state));
        let mut ch = py.detach(|| new.changes_since(&old));
        ch.modified[2].extend(attrs.into_iter().filter(|&k|
            new.slot(Kind::Face, k).is_some() && old.slot(Kind::Face, k).is_some()));
        ch.dedup();
        Ok(PyChange { inner: ch })
    }

    fn field<'py>(&self, name: &str, mesh: &Bound<'py, PyAny>, own: &Bound<'py, PyDict>,
                  table: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        snapshot_fields::field(&reader(&self.inner), name, mesh, own, table)
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

    /// `built_core.pieces.connected_parts`: `[(face keys, edge keys)]`, largest
    /// first (store/pieces.rs).
    fn connected_parts(&self, py: Python<'_>) -> Vec<(Vec<u128>, Vec<u128>)> {
        let r = reader(&self.inner);
        let parts = py.detach(|| r.connected_parts());
        super::pieces::keyed(&r, parts)
    }

    /// `built_core.pieces.solid_parts`: the physical pieces, `[(face keys, edge
    /// keys)]`, in Python's order (store/pieces.rs).
    fn solid_parts(&self, py: Python<'_>, hypot: &str) -> PyResult<Vec<(Vec<u128>, Vec<u128>)>> {
        let h = crate::hypot_of(hypot)?;
        let r = reader(&self.inner);
        let parts = py.detach(|| r.solid_parts(h));
        Ok(super::pieces::keyed(&r, parts))
    }

    /// Each face's attribute-table row (−1 none), by key (−1 also for a
    /// key the snapshot does not hold live).
    fn face_rows<'py>(&self, py: Python<'py>, keys: Vec<u128>) -> Bound<'py, PyArray1<i64>> {
        let st = &self.inner.state;
        let r = reader(&self.inner);
        let rows: Vec<i64> = keys
            .iter()
            .map(|&k| r.slot(Kind::Face, k).map_or(-1, |f| *st.fattr.get(f as usize) as i64 - 1))
            .collect();
        rows.into_pyarray(py)
    }

    /// `built_core.pieces.copy_part` (Split into Pieces): a new store in
    /// `namespace` holding the faces and edges named (keys of this
    /// snapshot, in order), `face_attr` their rows in the new table (-1 none).
    #[pyo3(signature = (namespace, faces, edges, face_attr, hole_filter, scale = 10_000.0, tol2 = 1e-8, fused = false))]
    #[allow(clippy::too_many_arguments)]
    fn extract(&self, namespace: u64, faces: Vec<u128>, edges: Vec<u128>, face_attr: Vec<i64>,
               hole_filter: &Bound<'_, PyAny>, scale: f64, tol2: f64, fused: bool) -> PyResult<PyMeshStore> {
        let r = reader(&self.inner);
        let fs: Vec<u32> = faces.iter().map(|&k| slot_of(&r, Kind::Face, k)).collect::<PyResult<_>>()?;
        let es: Vec<u32> = edges.iter().map(|&k| slot_of(&r, Kind::Edge, k)).collect::<PyResult<_>>()?;
        let keep = super::pieces::hole_masks(&r, &fs, |v| *r.state.vpos.get(v as usize), hole_filter)?;
        let store = r.extract(namespace, &fs, &es, &keep, &face_attr, Weld { scale, tol2, fused })
            .map_err(PyValueError::new_err)?;
        Ok(PyMeshStore::wrap(store))
    }
}

#[cfg_attr(test, allow(dead_code))]
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMeshStore>()?;
    m.add_class::<elements::Elements>()?;
    m.add_function(wrap_pyfunction!(weld_existing::weld_candidates, m)?)?;
    m.add_class::<PyStoreSnapshot>()?;
    m.add_class::<PyRoundPlan>()?;
    m.add_class::<super::intersect::PyPolygon>()?;
    m.add_class::<PyChange>()?;
    m.add("StoreFallback", m.py().get_type::<StoreFallback>())?;
    Ok(())
}

#[path = "fillet_preview.rs"]
pub(crate) mod fillet_preview;

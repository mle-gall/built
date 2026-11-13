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
use super::{Snapshot, Store, Weld};
use numpy::ndarray::Array2;
use numpy::{IntoPyArray, PyArray1, PyArray2, PyReadonlyArray1, PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

#[pyclass(module = "ingetrazo_core", name = "MeshStore")]
pub struct PyMeshStore {
    pub inner: Store,
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
    #[pyo3(signature = (namespace, scale = 10_000.0, tol2 = 1e-8))]
    fn new(namespace: u64, scale: f64, tol2: f64) -> Self {
        Self { inner: Store::empty(namespace, Weld { scale, tol2 }) }
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
                        tol2 = 1e-8))]
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
        let w = Weld { scale, tol2 };
        let store = py.detach(|| Store::from_block(namespace, &block, w)).map_err(PyValueError::new_err)?;
        Ok(Self { inner: store })
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
        Ok(out)
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
        let slot = st.order[k as usize]
            .live(&st.slots[k as usize])
            .get(index)
            .copied()
            .ok_or_else(|| PyValueError::new_err("index out of range"))?;
        Ok(self.inner.key(k, slot))
    }

    /// The list index of a live key (the inverse of `key_at`).
    fn index_of(&self, key: u128) -> PyResult<usize> {
        let (_, k, _, _) = super::keys::split_key(key).ok_or_else(|| PyKeyError::new_err(key))?;
        let slot = self.inner.slot(k, key).ok_or_else(|| PyKeyError::new_err(key))?;
        let st = &self.inner.state;
        Ok(st.order[k as usize].dense_of_slot(&st.slots[k as usize])[slot as usize] as usize)
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
        match self.inner.slot(Kind::Vertex, key) {
            Some(v) => self.inner.registers(v),
            None => false,
        }
    }

    /// The weld registry as `(cells (N, 3) int64, vertex list index (N,))`,
    /// sorted by cell.
    fn registry<'py>(&self, py: Python<'py>) -> (Bound<'py, PyArray2<i64>>, Bound<'py, PyArray1<i64>>) {
        let dv = self.inner.state.order[0].dense_of_slot(&self.inner.state.slots[0]);
        let mut rows: Vec<([i64; 3], i64)> = self
            .inner
            .state
            .registry
            .iter()
            .map(|(c, &v)| ([c.0, c.1, c.2], dv[v as usize] as i64))
            .collect();
        rows.sort_unstable();
        let cells: Vec<[i64; 3]> = rows.iter().map(|r| r.0).collect();
        let ids: Vec<i64> = rows.iter().map(|r| r.1).collect();
        (rows3(py, &cells), ids.into_pyarray(py))
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

    /// Faces touching the given vertices, each once (keys).
    fn faces_around(&self, keys: Vec<u128>) -> PyResult<Vec<u128>> {
        let vs: Vec<u32> = keys
            .iter()
            .map(|&k| self.inner.slot(Kind::Vertex, k).ok_or_else(|| PyKeyError::new_err(k)))
            .collect::<PyResult<_>>()?;
        Ok(self.inner.faces_around(&vs).into_iter().map(|f| self.inner.key(Kind::Face, f)).collect())
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
        Self { inner: self.inner.copy_as(namespace) }
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

#[pymethods]
impl PyStoreSnapshot {
    #[getter]
    fn namespace(&self) -> u64 {
        self.inner.state.ns
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
    m.add_class::<PyChange>()?;
    Ok(())
}

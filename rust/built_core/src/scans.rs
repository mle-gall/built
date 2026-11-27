// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! `MeshScan`: one mesh at one revision as flat arrays, for the drawing
//! commands' searches over every face and edge (`core/scan_index.py`).
//!
//! Drawing one line across a face of a 267k-face mesh spent 20–25 s in
//! Python loops that asked each face and edge a question most of them
//! cannot answer yes to: is your box near the segment, is this loop inside
//! you, do you lie on this plane, does an edge end at this point. Here those
//! loops run over arrays, and every query is a PREFILTER: it returns, in
//! list order, a superset of the elements the Python predicate can accept
//! (its tolerances widened, never narrowed), and the caller runs that
//! predicate, unchanged, on what comes back. So the answer is the full
//! scan's, element for element, whatever the arithmetic differences
//! between f64 here and Qt's float32 there.
//!
//! A scan is built either straight off a `MeshStore` (no Python object
//! read) or from a Python mesh's three lists; it never changes after that.
//! Keys of a store-backed scan come from a clone of the store state taken
//! at build (O(chunks)), so an index stays meaningful after the mesh moves
//! on.

use crate::store::keys::{full_key, local_key, Kind, NONE};
use crate::store::py::PyMeshStore;
use crate::store::State;
use numpy::ndarray::Array2;
use numpy::{IntoPyArray, PyArray2, PyReadonlyArray2};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyString};
use crate::store::cow::FxHashMap;
use std::collections::HashMap;

/// Edge flag bits in `MeshScan::eflag`.
const SOFT: u8 = 1;
const CURVE: u8 = 2;

/// Parts of a scan (`MeshScan::have`): a Python mesh's are read from its
/// objects only when a query needs them.
const VERTS: u8 = 1;
const EFLAGS: u8 = 2;
const EDGES: u8 = 4;
const FACES: u8 = 8;
const ALL: u8 = VERTS | EFLAGS | EDGES | FACES;
/// Parts only the push/pull passes read (`crate::orient`), read on first
/// use from the store state or the Python objects alike: every ring of a
/// face (holes too), each edge's faces, the faces' interior flags.
pub(crate) const RINGS: u8 = 16;
pub(crate) const EFACES: u8 = 32;
pub(crate) const FFLAGS: u8 = 64;

/// Outer-loop box of a face / box of an edge: `[lo; hi]`.
pub(crate) type Box6 = [f64; 6];

/// A face's plane as `Face.normal()` makes it, in f64.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Plane {
    /// First outer corner.
    pub(crate) o: [f64; 3],
    /// Unit Newell normal of the outer loop, `(0, 0, 1)` when degenerate.
    pub(crate) n: [f64; 3],
    /// Farthest outer corner from that plane, plus slack: a point Python
    /// projects inside the face lies within the face's box grown by this
    /// (and the loop's own distance to the plane).
    pub(crate) reach: f64,
    /// Newell vector so short that Python's float32 length may fall on the
    /// other side of its 1e-9 guard (and pick ``(0, 0, 1)``): no plane
    /// query may leave this face out.
    pub(crate) vague: bool,
}

#[pyclass(module = "built_core", name = "MeshScan")]
pub struct MeshScan {
    /// Vertex positions: the mesh's vertices in list order, then any vertex
    /// an edge or face references that the list does not hold (a corner
    /// whose edges were erased, still in its face's loop).
    pos: Vec<[f64; 3]>,
    /// Vertices in the mesh's own list (the rest are extras).
    n_listed: usize,
    ends: Vec<[u32; 2]>,
    /// `len(edge.faces)`, multiplicity included.
    enf: Vec<u32>,
    eflag: Vec<u8>,
    /// Outer loops: face `f` has corners `corners[fstart[f]..fstart[f+1]]`.
    fstart: Vec<u32>,
    corners: Vec<u32>,
    /// Store-backed: the state at build (for keys) and dense -> slot.
    state: Option<State>,
    live: [Vec<u32>; 3],
    /// Python-backed: the element lists at build (index -> element), read
    /// part by part as queries need them (``have``).
    lists: Option<[Py<PyList>; 3]>,
    have: u8,
    vindex: HashMap<usize, u32>,
    /// All rings (`RINGS`): face `f`'s ring sizes are
    /// `rlen[rface[f]..rface[f+1]]`, their corners consecutive in
    /// `rcorners` from `rcorner[f]`.
    pub(crate) rface: Vec<u32>,
    pub(crate) rlen: Vec<u32>,
    pub(crate) rcorner: Vec<u32>,
    pub(crate) rcorners: Vec<u32>,
    /// `edge.faces` as face indices, in order with multiplicity
    /// (`EFACES`); a face the face list does not hold is `NONE`.
    pub(crate) efstart: Vec<u32>,
    pub(crate) efaces: Vec<u32>,
    /// `face.interior` (`FFLAGS`).
    pub(crate) finterior: Vec<bool>,
    /// Store-backed: vertex slot -> index in `pos` (`NONE`: not read yet).
    vdense: Vec<u32>,
    /// The push/pull passes' per-revision geometry (`crate::orient`).
    pub(crate) geom: Option<std::sync::Arc<crate::orient::Geom>>,
    // Derived on first use.
    fbox: Option<Vec<Box6>>,
    ebox: Option<Vec<Box6>>,
    fplane: Option<Vec<Plane>>,
    graph: Option<crate::store::drawing::DrawingGraph>,
    ecells: Option<FxHashMap<(i64, i64, i64), Vec<u32>>>,
}

fn kind_of(s: &str) -> PyResult<Kind> {
    Kind::parse(s).ok_or_else(|| PyValueError::new_err(format!("kind {s:?}: vertex, edge or face")))
}

impl MeshScan {
    fn empty() -> Self {
        Self {
            pos: Vec::new(),
            n_listed: 0,
            ends: Vec::new(),
            enf: Vec::new(),
            eflag: Vec::new(),
            fstart: vec![0],
            corners: Vec::new(),
            state: None,
            live: Default::default(),
            lists: None,
            have: ALL,
            vindex: HashMap::new(),
            rface: vec![0],
            rlen: Vec::new(),
            rcorner: vec![0],
            rcorners: Vec::new(),
            efstart: vec![0],
            efaces: Vec::new(),
            finterior: Vec::new(),
            vdense: Vec::new(),
            geom: None,
            fbox: None,
            ebox: None,
            fplane: None,
            ecells: None,
            graph: None,
        }
    }

    pub(crate) fn n_faces(&self) -> usize {
        self.fstart.len() - 1
    }

    pub(crate) fn n_edges(&self) -> usize {
        self.ends.len()
    }

    /// Vertices in the mesh's own list (the rest of `positions` are
    /// corners no list holds).
    pub(crate) fn n_listed_vertices(&self) -> usize {
        self.n_listed
    }

    pub(crate) fn edge_ends(&self, e: usize) -> [u32; 2] {
        self.ends[e]
    }

    pub(crate) fn vertex_slot_indices(&self) -> &[u32] {
        &self.vdense
    }

    pub(crate) fn positions(&self) -> &[[f64; 3]] {
        &self.pos
    }

    /// `len(edge.faces)`, multiplicity included.
    pub(crate) fn edge_face_counts(&self) -> &[u32] {
        &self.enf
    }

    /// Build directly from the transaction's state, including optional
    /// ring, incidence and interior columns for orientation.
    pub(crate) fn from_state(state: State, parts: u8) -> Self {
        let st = &state;
        let mut s = MeshScan::empty();
        let live: [Vec<u32>; 3] = [
            st.order[0].live(&st.slots[0]),
            st.order[1].live(&st.slots[1]),
            st.order[2].live(&st.slots[2]),
        ];
        let mut vdense = vec![NONE; st.slots[0].len()];
        s.pos.reserve(live[0].len());
        for (i, &v) in live[0].iter().enumerate() {
            vdense[v as usize] = i as u32;
            s.pos.push(*st.vpos.get(v as usize));
        }
        s.n_listed = s.pos.len();
        let mut dense = |slot: u32, pos: &mut Vec<[f64; 3]>| -> u32 {
            let d = vdense[slot as usize];
            if d != NONE {
                return d;
            }
            let i = pos.len() as u32;
            pos.push(*st.vpos.get(slot as usize));
            vdense[slot as usize] = i;
            i
        };
        s.ends.reserve(live[1].len());
        for &e in &live[1] {
            let [a, b] = *st.ends.get(e as usize);
            let da = dense(a, &mut s.pos);
            let db = dense(b, &mut s.pos);
            s.ends.push([da, db]);
            let inc = st.efaces.get(e as usize);
            let nf = if inc.len <= 2 { inc.len as usize } else { st.ef_spill.get(inc.a[0] as usize).len() };
            s.enf.push(nf as u32);
            let mut fl = 0u8;
            if *st.eflags.get(e as usize) & crate::store::SOFT != 0 {
                fl |= SOFT;
            }
            if *st.ecurve.get(e as usize) >= 0 {
                fl |= CURVE;
            }
            s.eflag.push(fl);
        }
        s.fstart.reserve(live[2].len());
        for &f in &live[2] {
            let r = *st.frings.get(f as usize);
            if r.n_rings > 0 {
                let size = *st.ring_sizes.get(r.ring_start as usize) as usize;
                for j in 0..size {
                    let c = *st.corners.get(r.corner_start as usize + j);
                    let d = dense(c, &mut s.pos);
                    s.corners.push(d);
                }
            }
            s.fstart.push(s.corners.len() as u32);
        }
        s.live = live;
        s.state = Some(state);
        s.vdense = vdense;
        s.have = ALL | parts;
        if parts != 0 {
            s.store_parts(parts);
        }
        s
    }

    /// Ordered candidate slots for the two containment tests of AddFace
    /// (now `FaceIndex::drawn_candidates`, held to this one by its test).
    #[cfg(test)]
    pub(crate) fn drawn_face_candidates(
        &mut self, py: Python<'_>, slot: u32,
    ) -> PyResult<(Vec<u32>, Vec<u32>)> {
        let faces = &self.live[Kind::Face as usize];
        let row = faces.iter().position(|&f| f == slot)
            .ok_or_else(|| PyValueError::new_err("drawn face is absent from its scan"))?;
        let corners = self.face_corners(row).len();
        if corners == 0 {
            return Ok((faces.clone(), faces.clone()));
        }
        let [x, y, z, xx, yy, zz] = self.fboxes()[row];
        let pad = 0.002 + 1e-6 * [x,y,z,xx,yy,zz].into_iter().map(f64::abs).fold(0.0, f64::max);
        let plane = self.fplanes()[row];
        let holding = self.faces_holding(py, (x,y,z), (xx,yy,zz), pad)?;
        let slots = |scan: &Self, rows: Vec<u32>| rows.into_iter()
            .map(|i| scan.live[Kind::Face as usize][i as usize]).collect::<Vec<_>>();
        let inside = if corners < 3 || plane.vague {
            self.live[Kind::Face as usize].clone()
        } else {
            let g = pad + plane.reach;
            let rows = self.faces_within(py, (x-g,y-g,z-g), (xx+g,yy+g,zz+g))?;
            slots(self, rows)
        };
        Ok((slots(self, holding), inside))
    }

    /// Whether the scan reads a store state (keys) or Python objects.
    pub(crate) fn store_backed(&self) -> bool {
        self.state.is_some()
    }

    /// Face `f`'s rings (outer first) as vertex indices.
    pub(crate) fn rings_of(&self, f: usize) -> Vec<&[u32]> {
        let mut out = Vec::with_capacity((self.rface[f + 1] - self.rface[f]) as usize);
        let mut c = self.rcorner[f] as usize;
        for r in self.rface[f] as usize..self.rface[f + 1] as usize {
            let n = self.rlen[r] as usize;
            out.push(&self.rcorners[c..c + n]);
            c += n;
        }
        out
    }

    /// Store-backed: the key namespace of the state.
    pub(crate) fn namespace(&self) -> u64 {
        self.state.as_ref().map(|s| s.ns).unwrap_or(0)
    }

    /// Store-backed: `(slot, generation)` of a face -> its index in the
    /// face list (`NONE` when the scan's state does not list it).
    pub(crate) fn face_slot_table(&self) -> impl Fn(u32, u32) -> u32 + '_ {
        let st = self.state.as_ref().expect("store-backed scan");
        let mut dense = vec![NONE; st.slots[2].len()];
        for (i, &f) in self.live[2].iter().enumerate() {
            dense[f as usize] = i as u32;
        }
        move |slot: u32, gen: u32| {
            if (slot as usize) < dense.len() && dense[slot as usize] != NONE && st.slots[2].gen(slot) == gen {
                dense[slot as usize]
            } else {
                NONE
            }
        }
    }

    /// Python-backed: face object address -> its index in the face list.
    pub(crate) fn face_pointer_map(&self, py: Python<'_>) -> PyResult<HashMap<usize, u32>> {
        let mut map = HashMap::new();
        if let Some(l) = &self.lists {
            let fs = l[2].bind(py);
            map.reserve(fs.len());
            for (i, f) in fs.iter().enumerate() {
                map.entry(f.as_ptr() as usize).or_insert(i as u32);
            }
        }
        Ok(map)
    }

    /// The faces of edge `e` (indices; `NONE` for one not in the list).
    pub(crate) fn faces_of_edge(&self, e: usize) -> &[u32] {
        &self.efaces[self.efstart[e] as usize..self.efstart[e + 1] as usize]
    }

    /// Read the push/pull parts `what` (`RINGS`, `EFACES`, `FFLAGS`, and
    /// any of the base parts they rest on).
    pub(crate) fn need_parts(&mut self, py: Python<'_>, what: u8) -> PyResult<()> {
        // The push/pull parts index the base ones (positions, the face and
        // edge lists, the edges' face counts): all of them first.
        self.need(py, ALL)?;
        let missing = what & !self.have & (RINGS | EFACES | FFLAGS);
        if missing == 0 {
            return Ok(());
        }
        if self.state.is_some() {
            self.store_parts(missing);
        } else {
            self.object_parts(py, missing)?;
        }
        self.have |= missing;
        Ok(())
    }

    fn face_corners(&self, f: usize) -> &[u32] {
        &self.corners[self.fstart[f] as usize..self.fstart[f + 1] as usize]
    }

    fn fboxes(&mut self) -> &Vec<Box6> {
        if self.fbox.is_none() {
            let out = (0..self.n_faces()).map(|f| self.face_box(f)).collect();
            self.fbox = Some(out);
        }
        self.fbox.as_ref().unwrap()
    }

    fn eboxes(&mut self) -> &Vec<Box6> {
        if self.ebox.is_none() {
            let out = (0..self.n_edges()).map(|e| self.edge_box(e)).collect();
            self.ebox = Some(out);
        }
        self.ebox.as_ref().unwrap()
    }

    /// Per face: its plane (see [`Plane`]).
    fn fplanes(&mut self) -> &Vec<Plane> {
        if self.fplane.is_none() {
            let out = (0..self.n_faces()).map(|f| self.face_plane(f)).collect();
            self.fplane = Some(out);
        }
        self.fplane.as_ref().unwrap()
    }

    fn face_box(&self, f: usize) -> Box6 {
        box_of(self.face_corners(f).iter().map(|&v| &self.pos[v as usize]))
    }

    fn face_plane(&self, f: usize) -> Plane {
        let c = self.face_corners(f);
        plane_at(c.len(), |i| self.pos[c[i] as usize])
    }

    fn edge_box(&self, e: usize) -> Box6 {
        let [a, b] = self.ends[e];
        box_of([&self.pos[a as usize], &self.pos[b as usize]].into_iter())
    }

    /// Edges by the topology key cell of their ends (`built_core.topology._key`:
    /// `round(x, 4)` per axis, as the integer `round(x * 1e4)`). A
    /// coordinate whose product lands within float error of a half is filed
    /// under both neighbouring cells, so the map holds every edge that can
    /// end at a key (the caller compares exact keys).
    fn edge_cells(&mut self) -> &FxHashMap<(i64, i64, i64), Vec<u32>> {
        if self.ecells.is_none() {
            // One cell per vertex; the rare doubtful one keeps all its
            // candidates aside.
            let mut cell: Vec<(i64, i64, i64)> = Vec::with_capacity(self.pos.len());
            let mut doubt: FxHashMap<u32, Vec<(i64, i64, i64)>> = FxHashMap::default();
            for (v, p) in self.pos.iter().enumerate() {
                let (cx, cy, cz) = (axis_cells(p[0]), axis_cells(p[1]), axis_cells(p[2]));
                cell.push((cx[0], cy[0], cz[0]));
                if cx[0] != cx[1] || cy[0] != cy[1] || cz[0] != cz[1] {
                    doubt.insert(v as u32, cells_of(p));
                }
            }
            let mut map: FxHashMap<(i64, i64, i64), Vec<u32>> = FxHashMap::default();
            map.reserve(self.pos.len());
            for (e, [a, b]) in self.ends.iter().enumerate() {
                let e = e as u32;
                for v in [*a, *b] {
                    match doubt.get(&v) {
                        None => {
                            let list = map.entry(cell[v as usize]).or_default();
                            if list.last() != Some(&e) {
                                list.push(e);
                            }
                        }
                        Some(cs) => {
                            for c in cs {
                                let list = map.entry(*c).or_default();
                                if list.last() != Some(&e) {
                                    list.push(e);
                                }
                            }
                        }
                    }
                }
            }
            self.ecells = Some(map);
        }
        self.ecells.as_ref().unwrap()
    }
}

/// A face's outer-loop box from its corners' positions.
pub(crate) fn face_box(pts: &[[f64; 3]]) -> Box6 {
    box_of(pts.iter())
}

/// A face's plane (see [`Plane`]) from its outer corners' positions.
pub(crate) fn face_plane(pts: &[[f64; 3]]) -> Plane {
    plane_at(pts.len(), |i| pts[i])
}

/// The plane of an outer loop of `n` corners at `at(i)`.
fn plane_at(n_corners: usize, at: impl Fn(usize) -> [f64; 3]) -> Plane {
    let o = if n_corners > 0 { at(0) } else { [0.0; 3] };
    let mut n = [0.0f64, 0.0, 1.0];
    let mut vague = false;
    if n_corners >= 3 {
        let (mut nx, mut ny, mut nz) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..n_corners {
            let p = at(i);
            let q = at((i + 1) % n_corners);
            nx += (p[1] - q[1]) * (p[2] + q[2]);
            ny += (p[2] - q[2]) * (p[0] + q[0]);
            nz += (p[0] - q[0]) * (p[1] + q[1]);
        }
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        vague = len < 1e-8;
        if len >= 1e-9 {
            n = [nx / len, ny / len, nz / len];
        }
    }
    let mut reach = 0.0f64;
    let mut diag = 0.0f64;
    for i in 0..n_corners {
        let (d, span) = dot_rel(&at(i), &o, &n);
        reach = reach.max(d.abs());
        diag = diag.max(span);
    }
    reach += slack(diag);
    Plane { o, n, reach, vague }
}

fn box_of<'a>(pts: impl Iterator<Item = &'a [f64; 3]>) -> Box6 {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut any = false;
    for p in pts {
        any = true;
        for k in 0..3 {
            b[k] = b[k].min(p[k]);
            b[k + 3] = b[k + 3].max(p[k]);
        }
    }
    if !any {
        // ``_points_box`` of nothing: the origin.
        return [0.0; 6];
    }
    b
}

/// The candidate integer cells of one coordinate of `round(x, 4)`.
fn axis_cells(x: f64) -> [i64; 2] {
    let y = x * 1e4;
    let fl = y.floor();
    let frac = y - fl;
    if !(y.abs() < 1e9) || (frac - 0.5).abs() < 1e-6 {
        [fl as i64, fl as i64 + 1]
    } else {
        let n = y.round() as i64;
        [n, n]
    }
}

fn cells_of(p: &[f64; 3]) -> Vec<(i64, i64, i64)> {
    let (cx, cy, cz) = (axis_cells(p[0]), axis_cells(p[1]), axis_cells(p[2]));
    if cx[0] == cx[1] && cy[0] == cy[1] && cz[0] == cz[1] {
        return vec![(cx[0], cy[0], cz[0])];
    }
    let mut out = Vec::with_capacity(8);
    for &x in cx.iter() {
        for &y in cy.iter() {
            for &z in cz.iter() {
                let c = (x, y, z);
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    out
}

#[inline]
fn meets(b: &Box6, w: &Box6) -> bool {
    b[3] >= w[0] && b[0] <= w[3] && b[4] >= w[1] && b[1] <= w[4] && b[5] >= w[2] && b[2] <= w[5]
}

fn window(lo: (f64, f64, f64), hi: (f64, f64, f64)) -> Box6 {
    [lo.0, lo.1, lo.2, hi.0, hi.1, hi.2]
}

#[inline]
fn dot_rel(p: &[f64; 3], o: &[f64; 3], n: &[f64; 3]) -> (f64, f64) {
    let d = [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
    let dist = d[0] * n[0] + d[1] * n[1] + d[2] * n[2];
    (dist, (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
}

/// The float32 work of a Python plane test (`QVector3D` difference, unit
/// normal and dot product) is off the exact value by a few float32 ulps of
/// the distance between the points; this slack is several times that, so a
/// point Python calls on the plane is never left out here.
#[inline]
fn slack(span: f64) -> f64 {
    1e-5 * span + 1e-7
}

fn py_xyz(obj: &Bound<'_, PyAny>, to_tuple: &Bound<'_, PyString>) -> PyResult<[f64; 3]> {
    let (x, y, z): (f64, f64, f64) = obj.call_method0(to_tuple)?.extract()?;
    Ok([x, y, z])
}

#[pymethods]
impl MeshScan {
    /// The scan of a store's current state, in list order (the
    /// `MeshStore.arrays()` indices), outer loops only.
    #[staticmethod]
    fn from_store(py: Python<'_>, store: PyRef<'_, PyMeshStore>) -> PyResult<Py<MeshScan>> {
        let state = store.inner.state.clone();
        store.scan_of(py, state)
    }

    /// The scan of a Python mesh from its three lists (``mesh.vertices``,
    /// ``mesh.edges``, ``mesh.faces``), held so an index names the element
    /// the list had at build. Nothing is read from the objects yet.
    #[staticmethod]
    fn from_objects(vertices: Bound<'_, PyList>, edges: Bound<'_, PyList>, faces: Bound<'_, PyList>) -> Self {
        let mut s = MeshScan::empty();
        s.n_listed = vertices.len();
        s.lists = Some([vertices.unbind(), edges.unbind(), faces.unbind()]);
        s.have = 0;
        s
    }

    /// ``(vertices, edges, faces)`` counts (listed vertices only).
    pub(crate) fn counts(&self, py: Python<'_>) -> (usize, usize, usize) {
        match &self.lists {
            Some(l) => (l[0].bind(py).len(), l[1].bind(py).len(), l[2].bind(py).len()),
            None => (self.n_listed, self.ends.len(), self.n_faces()),
        }
    }

    /// The elements at ``indices`` of the lists at build: Python objects of
    /// a Python-backed scan, store keys (ints) of a store-backed one.
    fn keys_at(&self, py: Python<'_>, kind: &str, indices: Vec<usize>) -> PyResult<Vec<Py<PyAny>>> {
        let k = kind_of(kind)?;
        let mut out = Vec::with_capacity(indices.len());
        if let Some(lists) = &self.lists {
            let l = lists[k as usize].bind(py);
            for i in indices {
                out.push(l.get_item(i)?.unbind());
            }
            return Ok(out);
        }
        let st = self.state.as_ref().unwrap();
        let live = &self.live[k as usize];
        for i in indices {
            let slot = *live.get(i).ok_or_else(|| PyValueError::new_err("index out of range"))?;
            let gen = st.slots[k as usize].gen(slot);
            let key = full_key(st.ns, local_key(k, gen, slot));
            out.push(key.into_pyobject(py)?.into_any().unbind());
        }
        Ok(out)
    }

    /// Faces whose outer-loop box meets ``[lo, hi]``.
    pub(crate) fn faces_meeting(&mut self, py: Python<'_>, lo: (f64, f64, f64), hi: (f64, f64, f64)) -> PyResult<Vec<u32>> {
        self.need(py, FACES)?;
        let w = window(lo, hi);
        let b = self.fboxes();
        Ok((0..b.len() as u32).filter(|&i| meets(&b[i as usize], &w)).collect())
    }

    /// Faces whose outer-loop box, grown by ``pad`` plus the face's own
    /// reach off its plane (see [`Plane`]), holds ``[lo, hi]``: every face
    /// a loop with all its corners within ``pad`` of the face's plane, and
    /// projected inside it, can lie in.
    fn faces_holding(&mut self, py: Python<'_>, lo: (f64, f64, f64), hi: (f64, f64, f64), pad: f64) -> PyResult<Vec<u32>> {
        self.need(py, FACES)?;
        let w = window(lo, hi);
        self.fboxes();
        self.fplanes();
        let (b, pl) = (self.fbox.as_ref().unwrap(), self.fplane.as_ref().unwrap());
        Ok((0..b.len() as u32)
            .filter(|&i| {
                let x = &b[i as usize];
                let p = &pl[i as usize];
                let g = pad + p.reach;
                p.vague || (0..3).all(|k| x[k] - g <= w[k] && x[k + 3] + g >= w[k + 3])
            })
            .collect())
    }

    /// Faces whose outer-loop box lies within ``[lo, hi]``.
    fn faces_within(&mut self, py: Python<'_>, lo: (f64, f64, f64), hi: (f64, f64, f64)) -> PyResult<Vec<u32>> {
        self.need(py, FACES)?;
        let w = window(lo, hi);
        let b = self.fboxes();
        Ok((0..b.len() as u32)
            .filter(|&i| {
                let x = &b[i as usize];
                (0..3).all(|k| x[k] >= w[k] && x[k + 3] <= w[k + 3])
            })
            .collect())
    }

    /// Edges whose box meets ``[lo, hi]``.
    pub(crate) fn edges_meeting(&mut self, py: Python<'_>, lo: (f64, f64, f64), hi: (f64, f64, f64)) -> PyResult<Vec<u32>> {
        self.need(py, EDGES)?;
        let w = window(lo, hi);
        let b = self.eboxes();
        Ok((0..b.len() as u32).filter(|&i| meets(&b[i as usize], &w)).collect())
    }

    /// For each row ``[lo, hi]`` of ``boxes`` (N, 6), the edges whose box
    /// meets it — one grid for all rows (``boxes.touching``).
    fn edges_meeting_many(&mut self, py: Python<'_>, boxes: PyReadonlyArray2<'_, f64>) -> PyResult<Vec<Vec<u32>>> {
        self.need(py, EDGES)?;
        let q = boxes.as_array();
        if q.shape()[1] != 6 {
            return Err(PyValueError::new_err("boxes must be (N, 6)"));
        }
        let a: Vec<crate::boxes::Box3> =
            q.rows().into_iter().map(|r| ([r[0], r[1], r[2]], [r[3], r[4], r[5]])).collect();
        let b: Vec<crate::boxes::Box3> =
            self.eboxes().iter().map(|x| ([x[0], x[1], x[2]], [x[3], x[4], x[5]])).collect();
        let (ii, jj) = py.detach(|| crate::boxes::touching(&a, &b, false, 0.0));
        let mut out = vec![Vec::new(); a.len()];
        for (i, j) in ii.into_iter().zip(jj) {
            out[i as usize].push(j as u32);
        }
        Ok(out)
    }

    /// Faces every outer corner of which lies within ``tol`` (+ the float32
    /// slack) of the plane through ``origin`` with unit ``normal``.
    fn faces_on_plane(&mut self, py: Python<'_>, origin: (f64, f64, f64), normal: (f64, f64, f64), tol: f64) -> PyResult<Vec<u32>> {
        self.need(py, FACES)?;
        let o = [origin.0, origin.1, origin.2];
        let n = [normal.0, normal.1, normal.2];
        let near = self.near_plane(&o, &n, tol);
        Ok((0..self.n_faces() as u32)
            .filter(|&f| self.face_corners(f as usize).iter().all(|&v| near[v as usize]))
            .collect())
    }

    /// Edges both ends of which lie on that plane (``curve_only``: edges of
    /// a curve only).
    fn edges_on_plane(&mut self, py: Python<'_>, origin: (f64, f64, f64), normal: (f64, f64, f64), tol: f64, curve_only: bool) -> PyResult<Vec<u32>> {
        self.need(py, EDGES | EFLAGS)?;
        let o = [origin.0, origin.1, origin.2];
        let n = [normal.0, normal.1, normal.2];
        let near = self.near_plane(&o, &n, tol);
        Ok((0..self.ends.len() as u32)
            .filter(|&e| {
                let [a, b] = self.ends[e as usize];
                (!curve_only || self.eflag[e as usize] & CURVE != 0) && near[a as usize] && near[b as usize]
            })
            .collect())
    }

    fn drawing_plane(&mut self, py: Python<'_>, extra: Vec<[f64; 3]>, hypot: &str) -> PyResult<Option<[[f64; 3]; 2]>> {
        use crate::store::geometry::V3;
        self.need(py, VERTS)?;
        let points = self.pos[..self.n_listed].iter().chain(&extra).map(|&[x, y, z]| V3(x, y, z));
        Ok(crate::store::drawing::drawing_plane(points, crate::hypot_of(hypot)?))
    }

    fn rebuild_plane(&mut self, py: Python<'_>, points: Vec<[f64; 3]>, cap: usize, hypot: &str) -> PyResult<Option<[[f64; 3]; 2]>> {
        use crate::store::geometry::V3;
        self.need(py, FACES)?;
        let planes = self.fplanes().iter().map(|p| {
            let [x, y, z] = p.o;
            let [nx, ny, nz] = p.n;
            (V3(x, y, z), V3(nx, ny, nz))
        });
        Ok(crate::store::drawing::rebuild_plane(planes, &points, cap, crate::hypot_of(hypot)?))
    }

    /// Faces whose own plane (first corner, unit Newell normal) passes
    /// within ``tol`` (+ slack) of every row of ``points`` (K, 3).
    fn faces_through(&mut self, py: Python<'_>, points: PyReadonlyArray2<'_, f64>, tol: f64) -> PyResult<Vec<u32>> {
        self.need(py, FACES)?;
        let pts: Vec<[f64; 3]> = points.as_array().rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect();
        let planes = self.fplanes();
        Ok((0..planes.len() as u32)
            .filter(|&f| {
                let pl = &planes[f as usize];
                pl.vague
                    || pts.iter().all(|p| {
                        let (d, span) = dot_rel(p, &pl.o, &pl.n);
                        d.abs() <= tol + slack(span)
                    })
            })
            .collect())
    }

    /// Faces whose first corner lies within ``tol`` (+ slack) of the plane
    /// and whose unit normal has ``|n · normal| >= dot_min``.
    fn faces_coplanar(&mut self, py: Python<'_>, origin: (f64, f64, f64), normal: (f64, f64, f64), tol: f64, dot_min: f64) -> PyResult<Vec<u32>> {
        self.need(py, FACES)?;
        let o = [origin.0, origin.1, origin.2];
        let n = [normal.0, normal.1, normal.2];
        let planes = self.fplanes();
        Ok((0..planes.len() as u32)
            .filter(|&f| {
                let pl = &planes[f as usize];
                let (d, span) = dot_rel(&pl.o, &o, &n);
                let c = pl.n[0] * n[0] + pl.n[1] * n[1] + pl.n[2] * n[2];
                d.abs() <= tol + slack(span) && (pl.vague || c.abs() >= dot_min)
            })
            .collect())
    }

    pub(crate) fn split_plan(
        &mut self,
        py: Python<'_>,
        a: [f64; 3],
        b: [f64; 3],
        indices: Vec<usize>,
        extras: Vec<[[f64; 3]; 2]>,
        hypot: &str,
        compensated: bool,
    ) -> PyResult<(Vec<[f64; 3]>, Vec<(usize, [f64; 3])>)> {
        self.need(py, EDGES)?;
        let h = crate::hypot_of(hypot)?;
        if indices.iter().any(|&i| i >= self.ends.len()) {
            return Err(PyValueError::new_err("edge index outside scan"));
        }
        let edges = indices
            .into_iter()
            .map(|i| self.ends[i].map(|v| self.pos[v as usize]))
            .chain(extras);
        Ok(crate::store::drawing::split_plan(a, b, edges, h, compensated))
    }

    pub(crate) fn drawing_cycles(
        &mut self,
        py: Python<'_>,
        a: [f64; 3],
        b: [f64; 3],
        removed: Vec<usize>,
        extra: Vec<[[f64; 3]; 2]>,
        limit: usize,
        both: bool,
    ) -> PyResult<Vec<Vec<[f64; 3]>>> {
        self.need(py, EDGES)?;
        // A store-backed scan's graph names edges by slot, so the next
        // revision's scan can carry it over (`refreshed`).
        let backed = self.state.is_some();
        let id = |i: usize, live: &[u32]| if backed { live[i] as usize } else { i };
        let graph = self.graph.get_or_insert_with(|| {
            let mut graph = crate::store::drawing::DrawingGraph::default();
            for (i, ends) in self.ends.iter().enumerate() {
                graph.append(id(i, &self.live[1]), ends.map(|v| self.pos[v as usize]));
            }
            graph
        });
        let dead = removed.into_iter().filter(|&i| i < self.ends.len()).map(|i| id(i, &self.live[1])).collect();
        Ok(graph.cycles(a, b, dead, extra, limit, both))
    }

    /// Edges with an end whose topology key cell may be ``cell``:
    /// ``(edge, end_a, end_b)`` rows, ascending.
    fn edges_at_cell(&mut self, py: Python<'_>, cell: (i64, i64, i64)) -> PyResult<Vec<(u32, u32, u32)>> {
        self.need(py, EDGES)?;
        self.edge_cells();
        let (map, ends) = (self.ecells.as_ref().unwrap(), &self.ends);
        match map.get(&cell) {
            None => Ok(Vec::new()),
            Some(list) => Ok(list.iter().map(|&e| (e, ends[e as usize][0], ends[e as usize][1])).collect()),
        }
    }

    /// The outer loops of faces ``indices`` as scan vertex indices.
    fn face_loops(&mut self, py: Python<'_>, indices: Vec<usize>) -> PyResult<Vec<Vec<u32>>> {
        self.need(py, FACES)?;
        indices
            .into_iter()
            .map(|f| {
                if f >= self.n_faces() {
                    return Err(PyValueError::new_err("face out of range"));
                }
                Ok(self.face_corners(f).to_vec())
            })
            .collect()
    }

    /// The position of scan vertex ``v``.
    fn position(&mut self, py: Python<'_>, v: usize) -> PyResult<(f64, f64, f64)> {
        self.need(py, VERTS)?;
        let p = self.pos.get(v).ok_or_else(|| PyValueError::new_err("vertex out of range"))?;
        Ok((p[0], p[1], p[2]))
    }

    /// Ordered T-junction splits. Keep the dense arrays in the scan and only
    /// return the edge and vertex indices which the facade has to reattach.
    fn stitch_hits(
        &mut self,
        py: Python<'_>,
        seeds: Vec<[f64; 3]>,
        tolerance: f64,
    ) -> PyResult<Vec<(u32, u32)>> {
        use crate::store::stitch_command::{junction_scope, junctions};
        self.need(py, EDGES)?;
        let bounds = (!seeds.is_empty()).then(|| {
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for p in seeds {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            [lo.map(|x| x - tolerance), hi.map(|x| x + tolerance)]
        });
        let (verts, edges) = junction_scope(
            (0..self.n_listed as u32).collect(),
            (0..self.ends.len() as u32).collect(),
            bounds,
            tolerance,
            |v| self.pos[v as usize],
            |e| self.ends[e as usize],
        );
        if edges.is_empty() || verts.is_empty() {
            return Ok(vec![]);
        }
        let points: Vec<_> = verts.iter().map(|&v| self.pos[v as usize]).collect();
        let mut local = vec![-1; self.pos.len()];
        for (i, &v) in verts.iter().enumerate() {
            local[v as usize] = i as i64;
        }
        let ends: Vec<_> = edges
            .iter()
            .map(|&e| self.ends[e as usize].map(|v| local[v as usize]))
            .collect();
        let hits = junctions(&points, &ends, bounds, tolerance).map_err(PyValueError::new_err)?;
        Ok(hits
            .into_iter()
            .map(|(e, v)| (edges[e], verts[v]))
            .collect())
    }

    /// Loose edges to remove and the uncovered stretches to restore, in live-list order.
    fn orphan_pieces(&mut self, py: Python<'_>) -> PyResult<Vec<(usize, Vec<[[f64; 3]; 2]>)>> {
        use crate::store::geometry::{Hypot, V3};
        self.need(py, EFLAGS)?;
        let orphans: Vec<_> = (0..self.enf.len()).filter(|&e| self.enf[e] == 0).collect();
        if orphans.is_empty() {
            return Ok(Vec::new());
        }
        self.need(py, EDGES)?;
        let segments: Vec<_> = self
            .ends
            .iter()
            .map(|ends| {
                ends.map(|v| {
                    let [x, y, z] = self.pos[v as usize];
                    V3(x, y, z)
                })
            })
            .collect();
        let cuts =
            crate::store::orphan_overlap::orphan_pieces(&segments, &orphans, Hypot::LibStdCxx);
        Ok(cuts
            .into_iter()
            .map(|(i, pieces)| (i, pieces.into_iter().map(|s| s.map(Into::into)).collect()))
            .collect())
    }

    /// Edges bounding no face (``soft_only``: soft ones only).
    fn edges_faceless(&mut self, py: Python<'_>, soft_only: bool) -> PyResult<Vec<u32>> {
        self.need(py, EFLAGS)?;
        Ok((0..self.enf.len() as u32)
            .filter(|&e| self.enf[e as usize] == 0 && (!soft_only || self.eflag[e as usize] & SOFT != 0))
            .collect())
    }

    /// Whether any edge belongs to a curve.
    fn any_curve(&mut self, py: Python<'_>) -> PyResult<bool> {
        self.need(py, EFLAGS)?;
        Ok(self.eflag.iter().any(|f| f & CURVE != 0))
    }

    /// The listed vertices' positions, (V, 3).
    fn vertex_positions<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyArray2<f64>>> {
        self.need(py, VERTS)?;
        let flat: Vec<f64> = self.pos[..self.n_listed].iter().flat_map(|p| p.iter().copied()).collect();
        Ok(Array2::from_shape_vec((self.n_listed, 3), flat).unwrap().into_pyarray(py))
    }

    /// Per edge, its box ``(E, 6)`` (``lo`` then ``hi``).
    fn edge_boxes<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyArray2<f64>>> {
        self.need(py, EDGES)?;
        let b = self.eboxes();
        let flat: Vec<f64> = b.iter().flat_map(|x| x.iter().copied()).collect();
        Ok(Array2::from_shape_vec((b.len(), 6), flat).unwrap().into_pyarray(py))
    }
}

impl MeshScan {
    /// The scan of `state`, a later (or earlier) state of this scan's
    /// store: its arrays read afresh (`from_state`, a few ms however big),
    /// what this scan derived from them carried over for every face and
    /// edge the edit left as it was — boxes, planes and the drawing graph,
    /// the costly parts to rebuild. The result answers as the fresh scan.
    /// None when nothing can be carried (Python-backed, push/pull parts,
    /// another namespace, or an edit too big for it to pay).
    pub(crate) fn refreshed(&mut self, state: State) -> Option<MeshScan> {
        let old = self.state.as_ref()?;
        if self.lists.is_some() || old.ns != state.ns || self.pos.len() != self.n_listed || self.have != ALL {
            return None;
        }
        if self.fbox.is_none() && self.fplane.is_none() && self.ebox.is_none() && self.graph.is_none() {
            return None;
        }
        let ch = state.changes_since(old);
        let slot_of = |key: u128| crate::store::keys::split_key(key).map(|(_, _, _, s)| s);
        let st = &state;
        let n_v = st.slots[0].len();
        let mut dirty_e = vec![false; st.slots[1].len().max(old.slots[1].len())];
        let mut dirty_f = vec![false; st.slots[2].len().max(old.slots[2].len())];
        let mut count = 0usize;
        for (k, dirty) in [(1usize, &mut dirty_e), (2, &mut dirty_f)] {
            for &key in ch.created[k].iter().chain(&ch.removed[k]).chain(&ch.modified[k]) {
                if let Some(x) = slot_of(key) {
                    dirty[x as usize] = true;
                    count += 1;
                }
            }
        }
        // Edges and faces at a moved vertex: same indices, new geometry.
        for &key in &ch.moved {
            let Some(v) = slot_of(key) else { continue };
            if (v as usize) >= n_v || !st.slots[0].alive(v) {
                continue;
            }
            for e in st.edges_at(v) {
                dirty_e[e as usize] = true;
                count += 1;
                for &f in st.edge_faces(e) {
                    dirty_f[f as usize] = true;
                }
            }
        }
        if count > 4096 + (self.ends.len() + self.n_faces()) / 8 {
            return None;
        }
        let mut s = MeshScan::from_state(state.clone(), 0);
        if s.pos.len() != s.n_listed {
            return None;
        }
        // This scan's index of each new edge / face left as it was.
        let carried = |old_live: &[u32], new_live: &[u32], n_old: usize, dirty: &[bool]| -> Vec<u32> {
            let mut at = vec![NONE; n_old];
            for (i, &x) in old_live.iter().enumerate() {
                at[x as usize] = i as u32;
            }
            new_live.iter().map(|&x| if dirty[x as usize] { NONE } else { at.get(x as usize).copied().unwrap_or(NONE) }).collect()
        };
        let edge_was = carried(&self.live[1], &s.live[1], old.slots[1].len(), &dirty_e);
        if self.fbox.is_some() || self.fplane.is_some() {
            let face_was = carried(&self.live[2], &s.live[2], old.slots[2].len(), &dirty_f);
            if let Some(old_box) = &self.fbox {
                s.fbox = Some(face_was.iter().enumerate()
                    .map(|(j, &i)| if i == NONE { s.face_box(j) } else { old_box[i as usize] }).collect());
            }
            if let Some(old_plane) = &self.fplane {
                s.fplane = Some(face_was.iter().enumerate()
                    .map(|(j, &i)| if i == NONE { s.face_plane(j) } else { old_plane[i as usize] }).collect());
            }
        }
        if let Some(old_box) = &self.ebox {
            s.ebox = Some(edge_was.iter().enumerate()
                .map(|(j, &i)| if i == NONE { s.edge_box(j) } else { old_box[i as usize] }).collect());
        }
        // The drawing graph, by edge slot: the old one's lists lose the
        // edges this edit changed and gain them back where they now are.
        if let Some(mut graph) = self.graph.take() {
            let st = s.state.as_ref().unwrap();
            for (i, &e) in self.live[1].iter().enumerate() {
                if dirty_e[e as usize] {
                    graph.remove(e as usize, self.ends[i].map(|d| self.pos[d as usize]));
                }
            }
            for (j, &e) in s.live[1].iter().enumerate() {
                if edge_was[j] == NONE {
                    let ends = s.ends[j].map(|d| s.pos[d as usize]);
                    graph.remove(e as usize, ends);
                    graph.insert(e as usize, ends, |x| st.list_rank(Kind::Edge, x as u32));
                }
            }
            s.graph = Some(graph);
        }
        Some(s)
    }

    /// Read the parts ``what`` of a Python-backed scan from its objects.
    fn need(&mut self, py: Python<'_>, what: u8) -> PyResult<()> {
        let missing = what & !self.have;
        if missing == 0 {
            return Ok(());
        }
        let lists = match &self.lists {
            Some(l) => [l[0].clone_ref(py), l[1].clone_ref(py), l[2].clone_ref(py)],
            None => return Ok(()),
        };
        let a_position = PyString::intern(py, "position");
        let a_to_tuple = PyString::intern(py, "toTuple");
        if missing & (VERTS | EDGES | FACES) != 0 && self.have & VERTS == 0 {
            let vs = lists[0].bind(py);
            self.pos.reserve(vs.len());
            self.vindex.reserve(vs.len());
            for v in vs.iter() {
                let p = py_xyz(&v.getattr(&a_position)?, &a_to_tuple)?;
                self.vindex.entry(v.as_ptr() as usize).or_insert(self.pos.len() as u32);
                self.pos.push(p);
            }
            self.have |= VERTS;
        }
        if missing & EFLAGS != 0 {
            let a_faces = PyString::intern(py, "faces");
            let a_soft = PyString::intern(py, "soft");
            let a_curve = PyString::intern(py, "curve");
            let es = lists[1].bind(py);
            self.enf.reserve(es.len());
            self.eflag.reserve(es.len());
            for e in es.iter() {
                self.enf.push(e.getattr(&a_faces)?.len()? as u32);
                let mut fl = 0u8;
                if e.getattr(&a_soft)?.is_truthy()? {
                    fl |= SOFT;
                }
                if !e.getattr(&a_curve)?.is_none() {
                    fl |= CURVE;
                }
                self.eflag.push(fl);
            }
            self.have |= EFLAGS;
        }
        // Vertices held by edges / faces but not listed (a corner whose
        // edges were erased): alive while the element holding them is.
        if missing & EDGES != 0 {
            let a_v0 = PyString::intern(py, "v0");
            let a_v1 = PyString::intern(py, "v1");
            let es = lists[1].bind(py);
            self.ends.reserve(es.len());
            for e in es.iter() {
                let a = self.vid(&e.getattr(&a_v0)?, &a_position, &a_to_tuple)?;
                let b = self.vid(&e.getattr(&a_v1)?, &a_position, &a_to_tuple)?;
                self.ends.push([a, b]);
            }
            self.have |= EDGES;
        }
        if missing & FACES != 0 {
            let a_loop = PyString::intern(py, "loop");
            let fs = lists[2].bind(py);
            self.fstart.reserve(fs.len());
            for f in fs.iter() {
                let lp = f.getattr(&a_loop)?;
                for v in lp.try_iter()? {
                    let d = self.vid(&v?, &a_position, &a_to_tuple)?;
                    self.corners.push(d);
                }
                self.fstart.push(self.corners.len() as u32);
            }
            self.have |= FACES;
        }
        Ok(())
    }

    fn vid(&mut self, v: &Bound<'_, PyAny>, a_position: &Bound<'_, PyString>, a_to_tuple: &Bound<'_, PyString>) -> PyResult<u32> {
        let k = v.as_ptr() as usize;
        if let Some(&i) = self.vindex.get(&k) {
            return Ok(i);
        }
        let i = self.pos.len() as u32;
        self.pos.push(py_xyz(&v.getattr(a_position)?, a_to_tuple)?);
        self.vindex.insert(k, i);
        Ok(i)
    }

    /// The push/pull parts of a store-backed scan, from its state.
    fn store_parts(&mut self, what: u8) {
        let st = self.state.as_ref().unwrap();
        if what & RINGS != 0 {
            let nf = self.live[2].len();
            self.rface = Vec::with_capacity(nf + 1);
            self.rcorner = Vec::with_capacity(nf + 1);
            self.rface.push(0);
            self.rcorner.push(0);
            self.rlen.clear();
            self.rcorners.clear();
            for &f in &self.live[2] {
                let r = *st.frings.get(f as usize);
                let mut c = r.corner_start as usize;
                for k in 0..r.n_rings as usize {
                    let size = *st.ring_sizes.get(r.ring_start as usize + k) as usize;
                    self.rlen.push(size as u32);
                    for j in 0..size {
                        let slot = *st.corners.get(c + j) as usize;
                        let mut d = self.vdense[slot];
                        if d == NONE {
                            d = self.pos.len() as u32;
                            self.pos.push(*st.vpos.get(slot));
                            self.vdense[slot] = d;
                        }
                        self.rcorners.push(d);
                    }
                    c += size;
                }
                self.rface.push(self.rlen.len() as u32);
                self.rcorner.push(self.rcorners.len() as u32);
            }
        }
        if what & (EFACES | FFLAGS) != 0 {
            let mut fdense = vec![NONE; st.slots[2].len()];
            for (i, &f) in self.live[2].iter().enumerate() {
                fdense[f as usize] = i as u32;
            }
            if what & EFACES != 0 {
                self.efstart = Vec::with_capacity(self.live[1].len() + 1);
                self.efstart.push(0);
                self.efaces.clear();
                for &e in &self.live[1] {
                    let inc = st.efaces.get(e as usize);
                    let fs: &[u32] = if inc.len <= 2 {
                        &inc.a[..inc.len as usize]
                    } else {
                        st.ef_spill.get(inc.a[0] as usize)
                    };
                    for &f in fs {
                        let d = fdense.get(f as usize).copied().unwrap_or(NONE);
                        self.efaces.push(d);
                    }
                    self.efstart.push(self.efaces.len() as u32);
                }
            }
            if what & FFLAGS != 0 {
                self.finterior = self.live[2]
                    .iter()
                    .map(|&f| *st.fflags.get(f as usize) & crate::store::INTERIOR != 0)
                    .collect();
            }
        }
    }

    /// The push/pull parts of a Python-backed scan, from its objects.
    fn object_parts(&mut self, py: Python<'_>, what: u8) -> PyResult<()> {
        let lists = match &self.lists {
            Some(l) => [l[0].clone_ref(py), l[1].clone_ref(py), l[2].clone_ref(py)],
            None => return Ok(()),
        };
        let a_position = PyString::intern(py, "position");
        let a_to_tuple = PyString::intern(py, "toTuple");
        let fs = lists[2].bind(py);
        if what & RINGS != 0 {
            let a_loop = PyString::intern(py, "loop");
            let a_holes = PyString::intern(py, "hole_loops");
            self.rface = Vec::with_capacity(fs.len() + 1);
            self.rcorner = Vec::with_capacity(fs.len() + 1);
            self.rface.push(0);
            self.rcorner.push(0);
            self.rlen.clear();
            self.rcorners.clear();
            for f in fs.iter() {
                let ring = |lp: Bound<'_, PyAny>, s: &mut Self| -> PyResult<()> {
                    let mut n = 0u32;
                    for v in lp.try_iter()? {
                        let d = s.vid(&v?, &a_position, &a_to_tuple)?;
                        s.rcorners.push(d);
                        n += 1;
                    }
                    s.rlen.push(n);
                    Ok(())
                };
                ring(f.getattr(&a_loop)?, self)?;
                for h in f.getattr(&a_holes)?.try_iter()? {
                    ring(h?, self)?;
                }
                self.rface.push(self.rlen.len() as u32);
                self.rcorner.push(self.rcorners.len() as u32);
            }
        }
        if what & EFACES != 0 {
            let a_faces = PyString::intern(py, "faces");
            let mut fidx: HashMap<usize, u32> = HashMap::with_capacity(fs.len());
            for (i, f) in fs.iter().enumerate() {
                fidx.entry(f.as_ptr() as usize).or_insert(i as u32);
            }
            let es = lists[1].bind(py);
            self.efstart = Vec::with_capacity(es.len() + 1);
            self.efstart.push(0);
            self.efaces.clear();
            for e in es.iter() {
                for f in e.getattr(&a_faces)?.try_iter()? {
                    let f = f?;
                    self.efaces.push(fidx.get(&(f.as_ptr() as usize)).copied().unwrap_or(NONE));
                }
                self.efstart.push(self.efaces.len() as u32);
            }
        }
        if what & FFLAGS != 0 {
            let a_interior = PyString::intern(py, "interior");
            self.finterior = Vec::with_capacity(fs.len());
            for f in fs.iter() {
                self.finterior.push(f.getattr(&a_interior)?.is_truthy()?);
            }
        }
        Ok(())
    }

    fn near_plane(&self, o: &[f64; 3], n: &[f64; 3], tol: f64) -> Vec<bool> {
        self.pos
            .iter()
            .map(|p| {
                let (d, span) = dot_rel(p, o, n);
                d.abs() <= tol + slack(span)
            })
            .collect()
    }
}

#[cfg(test)]
impl MeshScan {
    pub(crate) fn faces_within_rows(&mut self, py: Python<'_>, w: Box6) -> Vec<u32> {
        self.faces_within(py, (w[0], w[1], w[2]), (w[3], w[4], w[5])).unwrap()
    }
    pub(crate) fn faces_meeting_rows(&mut self, py: Python<'_>, w: Box6) -> Vec<u32> {
        self.faces_meeting(py, (w[0], w[1], w[2]), (w[3], w[4], w[5])).unwrap()
    }
    pub(crate) fn faces_holding_rows(&mut self, py: Python<'_>, w: Box6, pad: f64) -> Vec<u32> {
        self.faces_holding(py, (w[0], w[1], w[2]), (w[3], w[4], w[5]), pad).unwrap()
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use crate::store::edit::Change;
    use crate::store::{Store, Weld};

    fn quad(st: &mut Store, x: f64, y: f64) -> u32 {
        let ring = vec![[x, y, 0.0], [x + 1.0, y, 0.0], [x + 1.0, y + 1.0, 0.0], [x, y + 1.0, 0.0]];
        st.add_face(&[ring], &mut Change::default()).unwrap()
    }

    /// Every array and derived part of `a` and `b`, the graph's lists too.
    fn same(a: &mut MeshScan, b: &mut MeshScan) {
        assert_eq!(a.pos, b.pos);
        assert_eq!(a.n_listed, b.n_listed);
        assert_eq!(a.ends, b.ends);
        assert_eq!(a.enf, b.enf);
        assert_eq!(a.eflag, b.eflag);
        assert_eq!(a.fstart, b.fstart);
        assert_eq!(a.corners, b.corners);
        assert_eq!(a.live, b.live);
        assert_eq!(a.vdense, b.vdense);
        assert_eq!(a.fboxes(), b.fboxes());
        assert_eq!(a.fplanes(), b.fplanes());
        assert_eq!(a.eboxes(), b.eboxes());
        let graph = |s: &mut MeshScan| {
            if s.graph.is_none() {
                let mut g = crate::store::drawing::DrawingGraph::default();
                for (i, ends) in s.ends.iter().enumerate() {
                    g.append(s.live[1][i] as usize, ends.map(|v| s.pos[v as usize]));
                }
                s.graph = Some(g);
            }
            s.graph.as_ref().unwrap().sorted()
        };
        assert_eq!(graph(a), graph(b));
    }

    #[test]
    fn a_carried_scan_is_the_fresh_one() {
        let mut st = Store::empty(9, Weld::default());
        for i in 0..12 {
            for j in 0..12 {
                quad(&mut st, i as f64, j as f64);
            }
        }
        let mut scan = MeshScan::from_state(st.state.clone(), 0);
        let check = |st: &Store, scan: &mut MeshScan| {
            // Derive everything on the old scan so the carried copy has it.
            scan.fboxes();
            scan.fplanes();
            scan.eboxes();
            if scan.graph.is_none() {
                let mut g = crate::store::drawing::DrawingGraph::default();
                for (i, ends) in scan.ends.iter().enumerate() {
                    g.append(scan.live[1][i] as usize, ends.map(|v| scan.pos[v as usize]));
                }
                scan.graph = Some(g);
            }
            let mut carried = scan.refreshed(st.state.clone()).expect("a small edit carries");
            let mut fresh = MeshScan::from_state(st.state.clone(), 0);
            same(&mut carried, &mut fresh);
            *scan = carried;
        };
        // A line across cells: edges split, a face added over others.
        let e = st.find_edge(0, 1).unwrap();
        st.split_edge(e, [0.5, 0.0, 0.0], &mut Change::default()).unwrap();
        check(&st, &mut scan);
        let s0 = st.snapshot();
        let f = (0..st.state.slots[2].len() as u32).find(|&f| st.state.slots[2].alive(f)).unwrap();
        st.remove_face(f, &mut Change::default());
        check(&st, &mut scan);
        st.move_vertex(5, [0.0, 0.0, 0.25]);
        check(&st, &mut scan);
        quad(&mut st, 30.0, 30.0);
        check(&st, &mut scan);
        let e = st.find_edge(5, 6).or_else(|| st.find_edge(6, 5));
        if let Some(e) = e {
            st.remove_edge(e, &mut Change::default());
            check(&st, &mut scan);
            st.relink_edge(e, &mut Change::default()).unwrap();
            check(&st, &mut scan);
        }
        // Undo back past all of it.
        st.restore(&s0, None).unwrap();
        check(&st, &mut scan);
    }
}

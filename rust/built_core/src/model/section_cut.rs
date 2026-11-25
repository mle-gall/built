//! A section plane's cut through collected geometry (`built_core.hlr`): the
//! triangles, hard and soft edges clipped to the kept side, the chords where
//! the plane crosses the triangles, and those chords chained into the closed
//! rings a drafter fills with poché (`docs/model-port.md`, hidden lines).
//!
//! Ported line by line from Python, operation for operation (the plane's
//! distance is `((x·nx + y·ny) + z·nz) − c`), so the answers are bit for bit
//! the old ones (`tests/test_hlr_section_parity.py`). Geometry the cut keeps
//! whole comes back as the caller's own objects, as it did.

use std::collections::HashMap;
use std::hash::BuildHasherDefault;

use numpy::{PyArray1, PyArrayMethods};
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};

const EPS: f64 = 1e-9;

type P = [f64; 3];

fn point(obj: &Bound<'_, PyAny>) -> PyResult<P> {
    Ok([obj.get_item(0)?.extract()?, obj.get_item(1)?.extract()?, obj.get_item(2)?.extract()?])
}

struct Plane {
    n: P,
    c: f64,
}

impl Plane {
    fn of(plane: &Bound<'_, PyAny>) -> PyResult<Plane> {
        let n = plane.getattr("normal")?;
        let p = plane.getattr("point")?;
        let (nx, ny, nz): (f64, f64, f64) =
            (n.call_method0("x")?.extract()?, n.call_method0("y")?.extract()?, n.call_method0("z")?.extract()?);
        let (px, py, pz): (f64, f64, f64) =
            (p.call_method0("x")?.extract()?, p.call_method0("y")?.extract()?, p.call_method0("z")?.extract()?);
        Ok(Plane { n: [nx, ny, nz], c: nx * px + ny * py + nz * pz })
    }
    fn dist(&self, p: &P) -> f64 {
        p[0] * self.n[0] + p[1] * self.n[1] + p[2] * self.n[2] - self.c
    }
}

fn lerp(a: &P, b: &P, da: f64, db: f64) -> P {
    let t = da / (da - db);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn tup<'py>(py: Python<'py>, p: &P) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(py, p)
}

/// One end of a clipped segment: the caller's point, or a new one.
enum End<'py> {
    Given(Bound<'py, PyAny>),
    Made(P),
}

impl<'py> End<'py> {
    fn obj(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match self {
            End::Given(o) => Ok(o.clone()),
            End::Made(p) => Ok(tup(py, p)?.into_any()),
        }
    }
}

fn clip_seg<'py>(plane: &Plane, a: Bound<'py, PyAny>, b: Bound<'py, PyAny>) -> PyResult<Option<(End<'py>, End<'py>)>> {
    let (pa, pb) = (point(&a)?, point(&b)?);
    let (d0, d1) = (plane.dist(&pa), plane.dist(&pb));
    if d0 > EPS && d1 > EPS {
        return Ok(None);
    }
    if d0 <= EPS && d1 <= EPS {
        return Ok(Some((End::Given(a), End::Given(b))));
    }
    let x = lerp(&pa, &pb, d0, d1);
    Ok(Some(if d0 <= EPS { (End::Given(a), End::Made(x)) } else { (End::Made(x), End::Given(b)) }))
}

/// `clip_to_section(tris, hard, soft, plane, split_cuts=False)`: see
/// `built_core.hlr.clip_to_section`.
#[pyfunction]
#[pyo3(signature = (tris, hard, soft, plane, split_cuts=false))]
pub fn section_clip<'py>(py: Python<'py>, tris: &Bound<'py, PyAny>, hard: &Bound<'py, PyAny>, soft: &Bound<'py, PyAny>,
                         plane: &Bound<'py, PyAny>, split_cuts: bool) -> PyResult<Bound<'py, PyTuple>> {
    let pl = Plane::of(plane)?;
    let out_tris = PyList::empty(py);
    let cut_edges = PyList::empty(py);
    for tri in tris.try_iter()? {
        let tri = tri?;
        let v = [point(&tri.get_item(0)?)?, point(&tri.get_item(1)?)?, point(&tri.get_item(2)?)?];
        let d = [pl.dist(&v[0]), pl.dist(&v[1]), pl.dist(&v[2])];
        let nh = d.iter().filter(|di| **di > EPS).count();
        if nh == 0 {
            out_tris.append(&tri)?;
            continue;
        }
        if nh == 3 {
            continue;
        }
        // Sutherland–Hodgman (keep d <= eps); the crossing chord is the cut.
        let mut poly: Vec<End> = Vec::with_capacity(4);
        let mut crossings: Vec<P> = Vec::with_capacity(2);
        for i in 0..3 {
            let j = (i + 1) % 3;
            if d[i] <= EPS {
                poly.push(End::Given(tri.get_item(i)?));
            }
            if (d[i] > EPS) != (d[j] > EPS) {
                let x = lerp(&v[i], &v[j], d[i], d[j]);
                poly.push(End::Made(x));
                crossings.push(x);
            }
        }
        if crossings.len() == 2 {
            cut_edges.append((tup(py, &crossings[0])?, tup(py, &crossings[1])?))?;
        }
        if poly.len() >= 3 {
            let first = poly[0].obj(py)?;
            for i in 1..poly.len() - 1 {
                out_tris.append((first.clone(), poly[i].obj(py)?, poly[i + 1].obj(py)?))?;
            }
        }
    }
    let out_hard = PyList::empty(py);
    for seg in hard.try_iter()? {
        let seg = seg?;
        if let Some((a, b)) = clip_seg(&pl, seg.get_item(0)?, seg.get_item(1)?)? {
            out_hard.append((a.obj(py)?, b.obj(py)?))?;
        }
    }
    if !split_cuts {
        for c in cut_edges.iter() {
            out_hard.append(c)?;
        }
    }
    let out_soft = PyList::empty(py);
    for seg in soft.try_iter()? {
        let seg = seg?;
        if let Some((a, b)) = clip_seg(&pl, seg.get_item(0)?, seg.get_item(1)?)? {
            out_soft.append((a.obj(py)?, b.obj(py)?, seg.get_item(2)?, seg.get_item(3)?))?;
        }
    }
    if split_cuts {
        PyTuple::new(py, [out_tris.into_any(), out_hard.into_any(), out_soft.into_any(), cut_edges.into_any()])
    } else {
        PyTuple::new(py, [out_tris.into_any(), out_hard.into_any(), out_soft.into_any()])
    }
}

/// A hash for a cell's three indices: the map is asked 27 times per chord
/// end, and the default hash was most of the chaining's time.
#[derive(Default, Clone, Copy)]
struct CellHasher(u64);

impl std::hash::Hasher for CellHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_i64(*byte as i64);
        }
    }
    fn write_i64(&mut self, value: i64) {
        self.0 = (self.0.rotate_left(5) ^ value as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

/// The chords chained into closed rings (node ids), with the chord rows
/// each ring was walked from. `tol`: endpoints closer than it are one.
pub fn chain_loops(edges: &[[P; 2]], tol: Option<f64>) -> (Vec<P>, Vec<(Vec<usize>, Vec<usize>)>) {
    if edges.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let tol = tol.unwrap_or_else(|| {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for e in edges {
            for p in e {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        let span = (0..3).map(|k| hi[k] - lo[k]).fold(f64::NEG_INFINITY, f64::max);
        (span * 1e-7).max(1e-9)
    });
    let inv = 1.0 / tol;
    // The nodes of each cell of side `tol`, in the order they were made:
    // the first one alone (nearly every cell has one), then the rest.
    let mut cells: HashMap<(i64, i64, i64), (usize, Vec<usize>), BuildHasherDefault<CellHasher>> =
        HashMap::with_capacity_and_hasher(edges.len() * 2, BuildHasherDefault::default());
    let mut nodes: Vec<P> = Vec::new();
    let mut node_of = |pt: &P| -> usize {
        let c = ((pt[0] * inv).floor() as i64, (pt[1] * inv).floor() as i64, (pt[2] * inv).floor() as i64);
        for dx in [-1, 0, 1] {
            for dy in [-1, 0, 1] {
                for dz in [-1, 0, 1] {
                    if let Some((first, more)) = cells.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) {
                        for &i in std::iter::once(first).chain(more) {
                            let q = nodes[i];
                            if (q[0] - pt[0]).abs() <= tol && (q[1] - pt[1]).abs() <= tol
                                && (q[2] - pt[2]).abs() <= tol {
                                return i;
                            }
                        }
                    }
                }
            }
        }
        nodes.push(*pt);
        let id = nodes.len() - 1;
        match cells.entry(c) {
            std::collections::hash_map::Entry::Occupied(mut cell) => cell.get_mut().1.push(id),
            std::collections::hash_map::Entry::Vacant(cell) => {
                cell.insert((id, Vec::new()));
            }
        }
        id
    };
    // The chords at each node, in the order they came.
    let mut adj: Vec<Vec<usize>> = Vec::new();
    let mut chords: Vec<(usize, usize)> = Vec::new();
    let mut rows: Vec<usize> = Vec::new();
    for (k, e) in edges.iter().enumerate() {
        let a = node_of(&e[0]);
        let b = node_of(&e[1]);
        if a == b {
            continue;
        }
        let idx = chords.len();
        chords.push((a, b));
        rows.push(k);
        if adj.len() <= a.max(b) {
            adj.resize(a.max(b) + 1, Vec::new());
        }
        adj[a].push(idx);
        adj[b].push(idx);
    }
    let mut used = vec![false; chords.len()];
    let mut loops = Vec::new();
    for start in 0..chords.len() {
        if used[start] {
            continue;
        }
        let (a0, b0) = chords[start];
        used[start] = true;
        let mut ring = vec![a0, b0];
        let mut walked = vec![start];
        let (mut cur, mut prev) = (b0, start);
        let mut closed = false;
        loop {
            let next = adj.get(cur).and_then(|cs| cs.iter().copied().find(|&c| c != prev && !used[c]));
            let Some(next) = next else { break };
            used[next] = true;
            walked.push(next);
            let (x, y) = chords[next];
            cur = if x == cur { y } else { x };
            prev = next;
            if cur == a0 {
                closed = true;
                break;
            }
            ring.push(cur);
        }
        if closed && ring.len() >= 3 {
            loops.push((ring, walked.iter().map(|c| rows[*c]).collect()));
        }
    }
    (nodes, loops)
}

/// `section_loops(cut_edges, tol=None, members=None)`: see
/// `built_core.hlr.section_loops`.
#[pyfunction]
#[pyo3(signature = (cut_edges, tol=None, members=None))]
pub fn section_loops<'py>(py: Python<'py>, cut_edges: &Bound<'py, PyAny>, tol: Option<f64>,
                          members: Option<&Bound<'py, PyList>>) -> PyResult<Bound<'py, PyList>> {
    let np = py.import("numpy")?;
    let flat = np.call_method1("asarray", (cut_edges, np.getattr("float64")?))?
        .call_method1("reshape", (-1,))?
        .cast_into::<PyArray1<f64>>()?;
    let flat = flat.readonly();
    let flat = flat.as_slice()?;
    let edges: Vec<[P; 2]> = flat.chunks_exact(6)
        .map(|c| [[c[0], c[1], c[2]], [c[3], c[4], c[5]]])
        .collect();
    let (nodes, loops) = chain_loops(&edges, tol);
    let out = PyList::empty(py);
    for (ring, walked) in loops {
        let coords: Vec<f64> = ring.iter().flat_map(|i| nodes[*i]).collect();
        out.append(PyArray1::from_vec(py, coords).reshape([ring.len(), 3])?)?;
        if let Some(m) = members {
            m.append(walked)?;
        }
    }
    Ok(out)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(section_clip, m)?)?;
    m.add_function(wrap_pyfunction!(section_loops, m)?)?;
    Ok(())
}

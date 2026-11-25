// SPDX-License-Identifier: GPL-3.0-or-later
//! Recognize a regular circular edge chain and rebuild its face boundaries in place.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::f64::consts::TAU;
type Obj<'p> = Bound<'p, PyAny>;

#[pyclass(frozen, module = "built_core.curve_segments")]
struct CurveShape {
    edges: Py<PyList>,
    verts: Py<PyList>,
    #[pyo3(get)]
    closed: bool,
    center: DVec3,
    u: DVec3,
    v: DVec3,
    normal: DVec3,
    #[pyo3(get)]
    radius: f64,
    #[pyo3(get)]
    sweep: f64,
    #[pyo3(get)]
    flat: bool,
}
#[pymethods]
impl CurveShape {
    #[getter]
    fn minimum(&self) -> usize {
        if self.closed {
            3
        } else {
            2
        }
    }
    #[getter]
    fn segments(&self, py: Python<'_>) -> usize {
        self.edges.bind(py).len()
    }
    #[getter]
    fn center(&self) -> Vec3 {
        self.center.into()
    }
    #[getter]
    fn normal(&self) -> Vec3 {
        self.normal.into()
    }
    #[getter]
    fn u(&self) -> Vec3 {
        self.u.into()
    }
    #[getter]
    fn v(&self) -> Vec3 {
        self.v.into()
    }
    #[getter]
    fn edges<'p>(&self, py: Python<'p>) -> Bound<'p, PyList> {
        self.edges.bind(py).clone()
    }
    #[getter]
    fn verts<'p>(&self, py: Python<'p>) -> Bound<'p, PyList> {
        self.verts.bind(py).clone()
    }
    fn points(&self, py: Python<'_>, n: usize) -> PyResult<Vec<Vec3>> {
        if n < self.minimum() {
            return Err(pyo3::exceptions::PyValueError::new_err("too few segments"));
        }
        let len = if self.closed { n } else { n + 1 };
        let mut ps = (0..len)
            .map(|i| {
                let (s, c) = (self.sweep * i as f64 / n as f64).sin_cos();
                Vec3::from(self.center + self.radius * (self.u * c + self.v * s))
            })
            .collect::<Vec<_>>();
        let vs = self.verts.bind(py);
        ps[0] = xyz(&vs.get_item(0)?.getattr("position")?)?.into();
        if !self.closed {
            ps[n] = xyz(&vs.get_item(vs.len() - 1)?.getattr("position")?)?.into();
        }
        Ok(ps)
    }
}
#[pyfunction]
fn read_curve(mesh: &Obj<'_>, edge: &Obj<'_>) -> PyResult<Option<CurveShape>> {
    let py = mesh.py();
    if edge.getattr_opt("curve")?.is_none_or(|c| c.is_none()) {
        return Ok(None);
    }
    let chain = mesh.call_method1("curve_edges", (edge,))?;
    if chain.len()? < 2 || !chain.contains(edge)? {
        return Ok(None);
    }
    let walk = py
        .import("core.edits")?
        .call_method1("_order_chain", (&chain, edge))?;
    if walk.len()? != chain.len()? {
        return Ok(None);
    }
    let verts = PyList::empty(py);
    verts.append(walk.get_item(0)?.get_item(1)?)?;
    let edges = PyList::empty(py);
    for row in walk.try_iter()? {
        let r = row?;
        verts.append(r.get_item(2)?)?;
        edges.append(r.get_item(0)?)?;
    }
    let last = verts.len() - 1;
    let closed = verts.get_item(last)?.eq(verts.get_item(0)?)?;
    if closed {
        verts.del_item(last)?;
    }
    let n = verts.len();
    let keys = PySet::empty(py)?;
    let mut pts = Vec::with_capacity(n);
    for x in verts.iter() {
        keys.add(x.getattr("key")?)?;
        pts.push(xyz(&x.getattr("position")?)?);
    }
    if keys.len() != n || n < 3 {
        return Ok(None);
    }
    let origin = pts[0];
    for p in &mut pts {
        *p -= origin;
    }
    let (ab, ac) = if closed {
        (pts[n / 3], pts[2 * n / 3])
    } else {
        (pts[n / 2], pts[n - 1])
    };
    let cross = ab.cross(ac);
    let denom = cross.length_squared();
    if denom < 1e-18 {
        return Ok(None);
    }
    let c = (cross.cross(ab) * ac.length_squared() + ac.cross(cross) * ab.length_squared())
        / (2. * denom);
    let radius = c.length();
    if radius < 1e-9 {
        return Ok(None);
    }
    let tol = (radius * 1e-3).max(3. * f64::from(f32::EPSILON) * origin.abs().max_element());
    if pts.iter().any(|p| ((*p - c).length() - radius).abs() > tol) {
        return Ok(None);
    }
    let (short, long) = pts
        .iter()
        .zip(pts.iter().cycle().skip(1))
        .take(edges.len())
        .map(|(a, b)| a.distance(*b))
        .fold((f64::INFINITY, 0_f64), |(a, b), d| (a.min(d), b.max(d)));
    if long - short > 2. * tol {
        return Ok(None);
    }
    let u = (-c).normalize_or_zero();
    let normal = u.cross(pts[1] - c).normalize_or_zero();
    if normal.length() < 0.5 || pts.iter().any(|p| (*p - c).dot(normal).abs() > tol) {
        return Ok(None);
    }
    let v = normal.cross(u);
    let angle = |p: DVec3| (p - c).dot(v).atan2((p - c).dot(u));
    let sweep = if closed {
        TAU
    } else {
        let mut s = angle(pts[n - 1]);
        if s <= 1e-9 {
            s += TAU;
        }
        let step = s / (n - 1) as f64;
        if (angle(pts[1]) - step).abs() > (1e-3 * step.max(1.)).max(4. * tol / radius) {
            return Ok(None);
        }
        s
    };
    let mut flat = true;
    'edges: for e in edges.iter() {
        for f in e.getattr("faces")?.try_iter()? {
            let f = f?;
            if xyz(&f.call_method0("normal")?)?.dot(normal).abs() < 0.999 {
                flat = false;
                break 'edges;
            }
            for p in f.getattr("vertices")?.try_iter()? {
                if (xyz(&p?)? - origin - c).dot(normal).abs() > tol {
                    flat = false;
                    break 'edges;
                }
            }
        }
    }
    Ok(Some(CurveShape {
        verts: verts.unbind(),
        edges: edges.unbind(),
        normal,
        u,
        v,
        radius,
        sweep,
        closed,
        flat,
        center: origin + c,
    }))
}
fn splice(ring: &Obj<'_>, shape: &CurveShape, new: &[Vec3]) -> PyResult<Option<Vec<Vec3>>> {
    let verts = shape.verts.bind(ring.py());
    let n = ring.len()?;
    let required = verts.len();
    if n < required {
        return Ok(None);
    }
    let first = verts.get_item(0)?;
    let mut start = None;
    for (j, vertex) in ring.try_iter()?.enumerate() {
        if vertex?.eq(&first)? {
            start = Some(j);
            break;
        }
    }
    let Some(i) = start else {
        return Ok(None);
    };
    let forward = ring.get_item((i + 1) % n)?.eq(verts.get_item(1)?)?;
    let mut out = new.to_vec();
    if shape.closed {
        if n != required {
            return Ok(None);
        }
        let keys = PySet::empty(ring.py())?;
        for v in verts.iter() {
            keys.add(v.getattr("key")?)?;
        }
        for v in ring.try_iter()? {
            if !keys.contains(v?.getattr("key")?)? {
                return Ok(None);
            }
        }
        if !forward {
            out[1..].reverse();
        }
    } else {
        for j in 0..n {
            let index = if forward {
                (i + j) % n
            } else {
                (i + n - j) % n
            };
            let v = ring.get_item(index)?;
            if j < required {
                if v.ne(verts.get_item(j)?)? {
                    return Ok(None);
                }
            } else {
                out.push(xyz(&v.getattr("position")?)?.into());
            }
        }
        if !forward {
            out.reverse();
        }
    }
    Ok(Some(out))
}
fn ring_points(ring: &Obj<'_>) -> PyResult<Vec<Vec3>> {
    ring.try_iter()?
        .map(|v| Ok(xyz(&v?.getattr("position")?)?.into()))
        .collect()
}
fn rebuild(mesh: &Obj<'_>, shape: &CurveShape, n: usize) -> PyResult<bool> {
    let py = mesh.py();
    if n < shape.minimum() || n == shape.segments(py) || !shape.flat {
        return Ok(false);
    }
    let pts = shape.points(py, n)?;
    let faces = PyList::empty(py);
    let seen = PySet::empty(py)?;
    for edge in shape.edges.bind(py) {
        for f in edge.getattr("faces")?.try_iter()? {
            let f = f?;
            let k = f.getattr("key")?;
            if !seen.contains(&k)? {
                seen.add(k)?;
                faces.append(f)?;
            }
        }
    }
    let mut plans = vec![];
    for f in faces.iter() {
        let ring = f.getattr("loop")?;
        let replacement = splice(&ring, shape, &pts)?;
        let mut touched = replacement.is_some();
        let outer = match replacement {
            Some(ps) => ps,
            None => ring_points(&ring)?,
        };
        let mut holes = vec![];
        for h in f.getattr("hole_loops")?.try_iter()? {
            let h = h?;
            holes.push(match splice(&h, shape, &pts)? {
                Some(ps) => {
                    touched = true;
                    ps
                }
                None => ring_points(&h)?,
            });
        }
        if !touched {
            return Ok(false);
        }
        plans.push((
            outer,
            holes,
            py.import("builtins")?
                .call_method1("dict", (f.getattr("attrs")?,))?,
            f.getattr("interior")?,
        ));
    }
    let api = py.import("built_core.mesh")?;
    let flags = api.call_method1("edge_flags", (shape.edges.bind(py).get_item(0)?,))?;
    for f in faces.iter() {
        mesh.call_method1("remove_face", (f,))?;
    }
    for e in shape.edges.bind(py) {
        mesh.call_method1("remove_edge", (e,))?;
    }
    mesh.call_method0("prune_orphan_vertices")?;
    for (a, b) in pts.iter().zip(pts.iter().cycle().skip(1)).take(n) {
        let edge = mesh.call_method1("add_edge", (a.clone(), b.clone()))?;
        api.call_method1("stamp_edge_flags", (edge, &flags))?;
    }
    for (outer, holes, attrs, inside) in plans {
        let holes = (!holes.is_empty()).then_some(holes);
        let f = mesh.call_method1("add_face", (outer, holes))?;
        f.getattr("attrs")?.call_method1("update", (attrs,))?;
        f.setattr("interior", inside)?;
    }
    mesh.setattr("_chunk_dirty", true)?;
    mesh.setattr("_mut_serial", mesh.getattr("_mut_serial")?.add(1)?)?;
    Ok(true)
}
#[pyfunction]
fn mutate(edge: &Obj<'_>, n: isize, scene: &Obj<'_>) -> PyResult<()> {
    let mesh = scene.getattr("mesh")?;
    if let Some(shape) = read_curve(&mesh, edge)? {
        if n >= 0 && rebuild(&mesh, &shape, n as usize)? {
            return Ok(());
        }
    }
    Err(pyo3::PyErr::from_value(
        scene
            .py()
            .import("built_core.history")?
            .call_method1("CommandRejected", ("curve cannot be rebuilt",))?,
    ))
}
#[pyfunction]
fn init(cmd: &Obj<'_>, edge: &Obj<'_>, n: isize) -> PyResult<()> {
    let py = cmd.py();
    cmd.setattr("edge", edge)?;
    cmd.setattr("n", n)?;
    cmd.setattr("cid", edge.getattr_opt("curve")?)?;
    let edit = py
        .import("functools")?
        .call_method1("partial", (wrap_pyfunction!(mutate, py)?, edge, n))?;
    cmd.setattr(
        "_inner",
        py.import("built_core.history")?
            .call_method1("SnapshotMutation", (edit,))?,
    )
}
fn select(cmd: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let selection = scene.getattr("selection")?;
    selection.call_method0("clear")?;
    let cid = cmd.getattr("cid")?;
    for e in scene.getattr("mesh")?.getattr("edges")?.try_iter()? {
        let e = e?;
        if e.getattr("curve")?.eq(&cid)? {
            selection.call_method1("add", (e,))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn apply(cmd: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    cmd.getattr("_inner")?.call_method1("do", (scene,))?;
    select(cmd, scene)
}
#[pyfunction]
fn undo(cmd: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    cmd.getattr("_inner")?.call_method1("undo", (scene,))?;
    select(cmd, scene)
}
#[pyfunction]
fn doc_after(cmd: &Obj<'_>, scene: &Obj<'_>, _phase: &str) -> PyResult<()> {
    select(cmd, scene)?;
    scene.setattr("version", scene.getattr("version")?.add(1)?)
}

#[pyfunction]
#[pyo3(pass_module)]
fn __getattr__<'p>(m: &Bound<'p, PyModule>, name: &str) -> PyResult<Obj<'p>> {
    if name != "ResegmentCurveCommand" {
        return Err(pyo3::exceptions::PyAttributeError::new_err(name.to_owned()));
    }
    let py = m.py();
    let ns = PyDict::new(py);
    ns.set_item("__module__", m.name()?)?;
    let bind = py.import("functools")?.getattr("partialmethod")?;
    for (label, f) in [
        ("__init__", wrap_pyfunction!(init, py)?),
        ("do", wrap_pyfunction!(apply, py)?),
        ("undo", wrap_pyfunction!(undo, py)?),
        ("doc_after", wrap_pyfunction!(doc_after, py)?),
    ] {
        ns.set_item(label, bind.call1((f,))?)?;
    }
    let cls = py.import("builtins")?.call_method1(
        "type",
        (
            name,
            (py.import("built_core.history")?.getattr("MeshCommand")?,),
            ns,
        ),
    )?;
    m.add(name, &cls)?;
    Ok(cls)
}
pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "built_core.curve_segments")?;
    m.add("MAX_SEGMENTS", 999)?;
    m.add_class::<CurveShape>()?;
    m.add_function(wrap_pyfunction!(read_curve, &m)?)?;
    m.add_function(wrap_pyfunction!(__getattr__, &m)?)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("built_core.curve_segments", &m)?;
    parent.add("curve_segments", m)
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Export shading across soft, non-flat edges. Cache each face and vertex by
//! its model key (facade objects may be recreated between iterations).
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
fn loops<'p>(face: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let mut all = vec![face.getattr("loop")?];
    for hole in face.getattr("hole_loops")?.try_iter()? {
        all.push(hole?)
    }
    Ok(all)
}
#[pyclass(module = "ingetrazo_core")]
pub struct SmoothNormals {
    flat: Py<PyDict>,
    angles: Py<PyDict>,
    fans: Py<PyDict>,
    no_fan: Py<PyDict>,
}
#[pymethods]
impl SmoothNormals {
    #[new]
    pub fn new(py: Python<'_>) -> Self {
        Self {
            flat: PyDict::new(py).unbind(),
            angles: PyDict::new(py).unbind(),
            fans: PyDict::new(py).unbind(),
            no_fan: PyDict::new(py).unbind(),
        }
    }
    pub fn flat<'p>(&self, face: &Obj<'p>) -> PyResult<Obj<'p>> {
        let cache = self.flat.bind(face.py());
        if let Some(n) = cache.get_item(face)? {
            return Ok(n);
        }
        let n = face.call_method0("normal")?;
        cache.set_item(face, &n)?;
        Ok(n)
    }
    pub fn corners<'p>(&self, face: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
        let out = PyDict::new(face.py());
        for ring in loops(face)? {
            for vertex in ring.try_iter()? {
                let vertex = vertex?;
                if let Some(n) = self.fan(&vertex)?.get_item(face)? {
                    out.set_item(vertex.getattr("position")?.call_method0("toTuple")?, n)?;
                }
            }
        }
        Ok(out)
    }
}
impl SmoothNormals {
    fn angles<'p>(&self, face: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
        let cache = self.angles.bind(face.py());
        if let Some(a) = cache.get_item(face)? {
            return Ok(a.cast_into()?);
        }
        let weights = PyDict::new(face.py());
        let n = xyz(&self.flat(face)?)?;
        for ring in loops(face)? {
            let vertices: Vec<_> = ring.try_iter()?.collect::<PyResult<_>>()?;
            let pts = vertices
                .iter()
                .map(|v| xyz(&v.getattr("position")?))
                .collect::<PyResult<Vec<_>>>()?;
            for (i, v) in vertices.iter().enumerate() {
                let a = pts[(i + pts.len() - 1) % pts.len()] - pts[i];
                let b = pts[(i + 1) % pts.len()] - pts[i];
                let cross = a.cross(b);
                let a = cross.length().atan2(a.dot(b));
                weights.set_item(
                    v,
                    if cross.dot(n) > 0. {
                        std::f64::consts::TAU - a
                    } else {
                        a
                    },
                )?;
            }
        }
        cache.set_item(face, &weights)?;
        Ok(weights)
    }
    fn fan<'p>(&self, vertex: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
        let py = vertex.py();
        let cache = self.fans.bind(py);
        if let Some(map) = cache.get_item(vertex)? {
            return Ok(map.cast_into()?);
        }
        let mut seams = Vec::new();
        for e in vertex.getattr("edges")?.try_iter()? {
            let e = e?;
            if !e.getattr("soft")?.is_truthy()?
                || e.getattr("flat")
                    .is_ok_and(|f| f.is_truthy().unwrap_or(false))
            {
                continue;
            }
            let adjacent = e.getattr("faces")?;
            if adjacent.len()? < 2 {
                continue;
            }
            seams.push(adjacent);
        }
        if seams.is_empty() {
            let empty = self.no_fan.bind(py);
            cache.set_item(vertex, empty)?;
            return Ok(empty.clone());
        }
        let faces = PyList::empty(py);
        let ids = PyDict::new(py);
        let mut parent = Vec::new();
        for adjacent in seams {
            let mut first = None;
            for f in adjacent.try_iter()? {
                let f = f?;
                let id = if let Some(id) = ids.get_item(&f)? {
                    id.extract()?
                } else {
                    let n = parent.len();
                    parent.push(n);
                    faces.append(&f)?;
                    ids.set_item(f, n)?;
                    n
                };
                let r = root(&mut parent, id);
                if let Some(left) = first {
                    parent[r] = left
                } else {
                    first = Some(r)
                }
            }
        }
        let mut groups = vec![vec![]; parent.len()];
        for i in 0..parent.len() {
            groups[root(&mut parent, i)].push(i)
        }
        let out = PyDict::new(py);
        for g in groups.into_iter().filter(|g| g.len() > 1) {
            let mut total = DVec3::ZERO;
            let mut normals = vec![];
            let first = xyz(&self.flat(&faces.get_item(g[0])?)?)?;
            for id in g {
                let face = faces.get_item(id)?;
                let n = xyz(&self.flat(&face)?)?;
                let w = self
                    .angles(&face)?
                    .get_item(vertex)?
                    .map(|v| v.extract::<f64>())
                    .transpose()?
                    .unwrap_or(0.);
                let weighted = n * w;
                total += if weighted.dot(first) >= 0. {
                    weighted
                } else {
                    -weighted
                };
                normals.push((face, n));
            }
            if total.length() < 1e-9 {
                continue;
            }
            let n = total.normalize();
            let pos = Py::new(py, Vec3::from(n))?;
            let neg = Py::new(py, Vec3::from(-n))?;
            for (face, normal) in normals {
                out.set_item(face, if normal.dot(n) >= 0. { &pos } else { &neg })?
            }
        }
        cache.set_item(vertex, &out)?;
        Ok(out)
    }
}
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i]
    }
    i
}
#[pyfunction]
#[pyo3(signature=(face,smoother=None))]
fn corner_normals<'p>(face: &Obj<'p>, smoother: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    if let Some(s) = smoother {
        s.call_method1("corners", (face,))
    } else {
        Ok(SmoothNormals::new(face.py()).corners(face)?.into_any())
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SmoothNormals>()?;
    m.add_function(wrap_pyfunction!(corner_normals, m)?)
}

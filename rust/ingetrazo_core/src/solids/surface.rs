// SPDX-License-Identifier: GPL-3.0-or-later
//! Triangle soup crossing the manifold3d boundary. Face IDs carry paint and
//! soft-edge provenance through booleans, including a T-junction repair.
use super::*;
use numpy::{IntoPyArray, PyArray1, PyArray2, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::exceptions::PyValueError;

pub(super) struct Surface {
    pub points: Vec<DVec3>,
    pub triangles: Vec<[u32; 3]>,
    pub owners: Vec<u32>,
}
pub(super) struct Connections(Vec<usize>);
impl Connections {
    pub fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    pub fn root(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }
    pub fn join(&mut self, a: usize, b: usize) {
        let a = self.root(a);
        let b = self.root(b);
        self.0[b] = a;
    }
}
impl Surface {
    pub fn empty() -> Self {
        Self {
            points: vec![],
            triangles: vec![],
            owners: vec![],
        }
    }
    pub fn read(v: &Obj, t: &Obj, f: &Obj) -> PyResult<Self> {
        let py = v.py();
        let np = py.import("numpy")?;
        let v = np
            .call_method1("asarray", (v, "float64"))?
            .extract::<PyReadonlyArray2<f64>>()?;
        let t = np
            .call_method1("asarray", (t, "uint32"))?
            .extract::<PyReadonlyArray2<u32>>()?;
        let f = np
            .call_method1("asarray", (f, "uint32"))?
            .extract::<PyReadonlyArray1<u32>>()?;
        let v = v.as_array();
        let t = t.as_array();
        if v.ncols() < 3 || t.ncols() != 3 || t.nrows() != f.as_array().len() {
            return Err(PyValueError::new_err("Invalid triangle surface dimensions"));
        }
        let points = v
            .rows()
            .into_iter()
            .map(|r| DVec3::new(r[0], r[1], r[2]))
            .collect::<Vec<_>>();
        let triangles = t
            .rows()
            .into_iter()
            .map(|r| [r[0], r[1], r[2]])
            .collect::<Vec<_>>();
        if triangles
            .iter()
            .flatten()
            .any(|i| *i as usize >= points.len())
        {
            return Err(PyValueError::new_err("Triangle index out of range"));
        }
        Ok(Self {
            points,
            triangles,
            owners: f.as_array().to_vec(),
        })
    }
    fn arrays<'p>(
        &self,
        py: Python<'p>,
    ) -> PyResult<(
        Bound<'p, PyArray2<f64>>,
        Bound<'p, PyArray2<u32>>,
        Bound<'p, PyArray1<u32>>,
    )> {
        let points: Vec<_> = self.points.iter().flat_map(|p| p.to_array()).collect();
        let tris: Vec<_> = self.triangles.iter().flatten().copied().collect();
        Ok((
            points.into_pyarray(py).reshape((self.points.len(), 3))?,
            tris.into_pyarray(py).reshape((self.triangles.len(), 3))?,
            self.owners.clone().into_pyarray(py),
        ))
    }
    pub fn conform(&self, relative: f64) -> Self {
        let tol = relative
            * self
                .points
                .iter()
                .map(|p| p.abs().max_element())
                .fold(1., f64::max);
        let mut out = Self {
            points: self.points.clone(),
            triangles: Vec::new(),
            owners: Vec::new(),
        };
        for (tri, &owner) in self.triangles.iter().zip(&self.owners) {
            let mut ring = Vec::new();
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                ring.push(a);
                let (pa, pb) = (self.points[a as usize], self.points[b as usize]);
                let d = pb - pa;
                let len = d.length_squared();
                if len <= tol * tol {
                    continue;
                }
                let mut hits = Vec::new();
                let lo = pa.min(pb) - tol;
                let hi = pa.max(pb) + tol;
                for (i, p) in self.points.iter().enumerate() {
                    if i == a as usize
                        || i == b as usize
                        || !p.cmpge(lo).all()
                        || !p.cmple(hi).all()
                    {
                        continue;
                    }
                    let s = (*p - pa).dot(d) / len;
                    if s > 1e-9 && s < 1. - 1e-9 && p.distance_squared(pa + s * d) <= tol * tol {
                        hits.push((s, i as u32));
                    }
                }
                hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                ring.extend(hits.into_iter().map(|(_, i)| i));
            }
            if ring.len() == 3 {
                out.triangles.push(*tri);
                out.owners.push(owner);
                continue;
            }
            let c = out.points.len() as u32;
            out.points
                .push(tri.iter().map(|&i| self.points[i as usize]).sum::<DVec3>() / 3.);
            for (k, &i) in ring.iter().enumerate() {
                out.triangles.push([c, i, ring[(k + 1) % ring.len()]]);
                out.owners.push(owner);
            }
        }
        out
    }
    fn signed(&self, t: [u32; 3]) -> f64 {
        let [a, b, c] = t.map(|i| self.points[i as usize]);
        a.dot(b.cross(c))
    }
    pub fn outer(&self) -> Vec<bool> {
        let mut shells = Connections::new(self.points.len());
        for &[a, b, c] in &self.triangles {
            shells.join(a as usize, b as usize);
            shells.join(a as usize, c as usize);
        }
        let mut volumes = vec![0.; self.points.len()];
        let mut roots = Vec::new();
        for &t in &self.triangles {
            let root = shells.root(t[0] as usize);
            volumes[root] += self.signed(t);
            roots.push(root);
        }
        roots.into_iter().map(|r| volumes[r] >= 0.).collect()
    }
    pub fn manifold<'p>(&mut self, py: Python<'p>) -> PyResult<Obj<'p>> {
        if self.triangles.is_empty() {
            return Err(error(py, "empty"));
        }
        if self.triangles.iter().map(|&t| self.signed(t)).sum::<f64>() < 0. {
            for t in &mut self.triangles {
                t.reverse();
            }
        }
        let lib = py.import("manifold3d")?;
        let success = lib.getattr("Error")?.getattr("NoError")?;
        let make = |s: &Self| {
            let (v, t, f) = s.arrays(py)?;
            let kw = PyDict::new(py);
            kw.set_item("vert_properties", v)?;
            kw.set_item("tri_verts", t)?;
            kw.set_item("face_id", f)?;
            lib.call_method1("Manifold", (lib.getattr("Mesh64")?.call((), Some(&kw))?,))
        };
        let man = make(self)?;
        if man.call_method0("status")?.eq(&success)? {
            return Ok(man);
        }
        let man = make(&py.detach(|| self.conform(1e-9)))?;
        if !man.call_method0("status")?.eq(success)? {
            return Err(error(py, "not manifold"));
        }
        Ok(man)
    }
}
#[pyfunction(signature=(v,t,f,rel_tol=1e-9))]
pub(super) fn _conform_t_junctions<'p>(
    py: Python<'p>,
    v: &Obj<'p>,
    t: &Obj<'p>,
    f: &Obj<'p>,
    rel_tol: f64,
) -> PyResult<Obj<'p>> {
    let surface = Surface::read(v, t, f)?;
    py.detach(|| surface.conform(rel_tol))
        .arrays(py)?
        .into_bound_py_any(py)
}

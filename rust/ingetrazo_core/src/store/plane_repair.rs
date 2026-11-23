// SPDX-License-Identifier: GPL-3.0-or-later
//! Install a classified plane in mesh order. A stable plane must not touch
//! the cache token, the mesh revision or its face handles.
use super::offset::contains;
use crate::math::xyz;
use glam::DVec3;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyFloat, PyFrozenSet, PyList, PySet, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
type Ring = Vec<(f64, f64)>;

#[pyfunction]
pub(crate) fn plane_cycle<'py>(ring: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let py = ring.py();
    let mut keys = Vec::new();
    for p in ring.try_iter()? {
        let pt = xyz(&p?)?.to_array();
        // Dividing by the tolerance before rounding is the file-compatible
        // cell partition here. Python's integer keys also cover survey coordinates.
        let mut coords = Vec::with_capacity(3);
        for x in pt {
            let scaled = x / 1e-4;
            let q = scaled.round_ties_even();
            let key = if q.abs() < (1_u64 << 62) as f64 {
                (q as i64).into_pyobject(py)?.into_any()
            } else {
                PyFloat::new(py, scaled).call_method0("__round__")?
            };
            coords.push(key);
        }
        keys.push((
            pt.map(|x| (x / 1e-4).round_ties_even()),
            PyTuple::new(py, coords)?,
        ));
    }
    let start = keys
        .iter()
        .enumerate()
        .min_by(|a, b| {
            a.1 .0
                .partial_cmp(&b.1 .0)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| PyValueError::new_err("min() arg is an empty sequence"))?
        .0;
    keys.rotate_left(start);
    PyTuple::new(py, keys.into_iter().map(|(_, key)| key))
}

#[pyfunction]
pub(crate) fn plane_fingerprint<'py>(faces: &Obj<'py>) -> PyResult<Bound<'py, PyFrozenSet>> {
    let mut rows = Vec::new();
    for f in faces.try_iter()? {
        let f = f?;
        let holes = f
            .get_item(1)?
            .try_iter()?
            .map(|h| plane_cycle(&h?))
            .collect::<PyResult<Vec<_>>>()?;
        rows.push((
            plane_cycle(&f.get_item(0)?)?,
            PyFrozenSet::new(f.py(), holes)?,
            f.get_item(2)?,
        ));
    }
    PyFrozenSet::new(faces.py(), rows)
}

pub(crate) fn coplanar(
    face: &Obj<'_>,
    o: DVec3,
    n: DVec3,
    cache: &Obj<'_>,
    tol: f64,
) -> PyResult<bool> {
    let normal = if cache.is_none() {
        xyz(&face.call_method0("normal")?)?.normalize_or_zero()
    } else {
        xyz(&cache.call_method1("normal", (face,))?)?
    };
    if normal.dot(n).abs() <= 0.999 {
        return Ok(false);
    }
    let centre = if cache.is_none() {
        face.call_method0("centroid")?
    } else {
        cache.call_method1("centroid", (face,))?
    };
    Ok((xyz(&centre)? - o).dot(n).abs() < tol)
}

struct Plane {
    origin: DVec3,
    u: DVec3,
    v: DVec3,
}
impl Plane {
    fn project(&self, ring: &Obj<'_>) -> PyResult<Ring> {
        ring.try_iter()?
            .map(|pt| {
                let d = xyz(&pt?)? - self.origin;
                Ok((d.dot(self.u), d.dot(self.v)))
            })
            .collect()
    }
    fn project_face(&self, outer: &Obj<'_>, holes: &Obj<'_>) -> PyResult<(Ring, Vec<Ring>)> {
        Ok((
            self.project(outer)?,
            holes
                .try_iter()?
                .map(|h| self.project(&h?))
                .collect::<PyResult<_>>()?,
        ))
    }
}

#[pyfunction]
pub(crate) fn install_rebuilt_plane(
    mesh: &Obj<'_>,
    origin: &Obj<'_>,
    normal: &Obj<'_>,
    rebuilt: &Obj<'_>,
    cache: &Obj<'_>,
    tolerance: f64,
    replace_store: &Obj<'_>,
    probe: &Obj<'_>,
    basis: &Obj<'_>,
) -> PyResult<bool> {
    if rebuilt.is_none() {
        return Ok(false);
    }
    let py = mesh.py();
    let o = xyz(origin)?;
    let n = xyz(normal)?;
    let scope = if cache.is_none() {
        py.None().into_bound(py)
    } else {
        cache.call_method1("scope", (mesh,))?
    };
    let faces = if scope.is_none() {
        mesh.getattr("faces")?
    } else {
        scope.get_item(0)?
    };
    let old = PyList::empty(py);
    let previous = PyList::empty(py);
    for f in faces.try_iter()? {
        let f = f?;
        if coplanar(&f, o, n, cache, tolerance)? {
            previous.append((
                f.getattr("vertices")?,
                f.getattr("holes")?,
                f.getattr("interior")?,
            ))?;
            old.append(f)?;
        }
    }
    if plane_fingerprint(rebuilt)?.eq(plane_fingerprint(previous.as_any())?)? {
        return Ok(false);
    }
    if !cache.is_none() {
        cache.setattr(
            "token",
            cache.getattr("token")?.call_method1("__add__", (1,))?,
        )?;
    }
    let axes = basis.call1((normal,))?;
    let plane = Plane {
        origin: o,
        u: xyz(&axes.get_item(0)?)?,
        v: xyz(&axes.get_item(1)?)?,
    };
    let dict = py.get_type::<PyDict>();
    let mut donors = Vec::new();
    for f in old.iter() {
        let attrs = f.getattr("attrs")?;
        if attrs.is_truthy()? {
            donors.push((
                plane.project_face(&f.getattr("vertices")?, &f.getattr("holes")?)?,
                dict.call1((attrs,))?,
            ));
        }
    }
    let replacements = replace_store.call1((mesh, &old, rebuilt))?;
    if replacements.is_none() {
        for f in old.iter() {
            mesh.call_method1("remove_face", (f,))?;
        }
    }
    for (i, row) in rebuilt.try_iter()?.enumerate() {
        let row = row?;
        let outer = row.get_item(0)?;
        let holes = row.get_item(1)?;
        let f = if replacements.is_none() {
            let f =
                mesh.call_method1("add_face", (&outer, (!holes.is_empty()?).then_some(&holes)))?;
            f.setattr("interior", row.get_item(2)?)?;
            f
        } else {
            replacements.get_item(i)?
        };
        if !donors.is_empty() {
            let (rim, openings) = plane.project_face(&outer, &holes)?;
            let p = probe.call1((rim, openings))?.extract::<(f64, f64)>()?;
            for ((outside, inside), attrs) in &donors {
                if contains(outside, p) && !inside.iter().any(|hole| contains(hole, p)) {
                    f.setattr("attrs", dict.call1((attrs,))?)?;
                    break;
                }
            }
        }
    }
    Ok(true)
}

#[pyfunction]
pub(crate) fn prune_object_plane(
    mesh: &Obj<'_>,
    origin: &Obj<'_>,
    normal: &Obj<'_>,
    tolerance: f64,
) -> PyResult<()> {
    let o = xyz(origin)?;
    let n = xyz(normal)?;
    let on_plane = |p: &Obj<'_>| -> PyResult<bool> { Ok((xyz(p)? - o).dot(n).abs() < tolerance) };
    let edges = mesh
        .getattr("edges")?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?;
    for e in edges {
        if e.getattr("faces")?.is_empty()?
            && on_plane(&e.getattr("a")?)?
            && on_plane(&e.getattr("b")?)?
        {
            mesh.call_method1("remove_edge", (e,))?;
        }
    }
    let referenced = PySet::empty(mesh.py())?;
    for f in mesh.getattr("faces")?.try_iter()? {
        for r in crate::object_weld::rings(&f?)? {
            for v in r.try_iter()? {
                referenced.add(v?)?;
            }
        }
    }
    let orphans = PyList::empty(mesh.py());
    for v in mesh.getattr("vertices")?.try_iter()? {
        let v = v?;
        if v.getattr("edges")?.is_empty()? && !referenced.contains(&v)? {
            orphans.append(v)?;
        }
    }
    if !orphans.is_empty() {
        mesh.call_method1("drop_orphan_vertices", (orphans,))?;
    }
    Ok(())
}

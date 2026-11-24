// SPDX-License-Identifier: GPL-3.0-or-later
//! Walk existing materialised incidence without reissuing public handles.
use crate::math::xyz;
use crate::object_weld::rings;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
type Obj<'py> = Bound<'py, PyAny>;

fn ring_positions<'py>(ring: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let pos = pyo3::intern!(ring.py(), "position");
    let out = PyList::empty(ring.py());
    if let Ok(l) = ring.cast_exact::<PyList>() {
        for v in l.iter() {
            out.append(v.getattr(pos)?)?;
        }
    } else {
        for v in ring.try_iter()? {
            out.append(v?.getattr(pos)?)?;
        }
    }
    Ok(out)
}
#[pyfunction]
pub(crate) fn face_vertices<'py>(face: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    ring_positions(&face.getattr(pyo3::intern!(face.py(), "loop"))?)
}
#[pyfunction]
pub(crate) fn face_holes<'py>(face: &Obj<'py>) -> PyResult<Vec<Bound<'py, PyList>>> {
    face.getattr(pyo3::intern!(face.py(), "hole_loops"))?
        .try_iter()?
        .map(|r| ring_positions(&r?))
        .collect()
}
#[pyfunction]
pub(crate) fn vertex_faces<'py>(edges: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    let faces = PySet::empty(edges.py())?;
    for e in edges.try_iter()? {
        for f in e?.getattr(pyo3::intern!(edges.py(), "faces"))?.try_iter()? {
            faces.add(f?)?;
        }
    }
    Ok(faces)
}
#[pyfunction]
pub(crate) fn insert_between(
    face: &Obj<'_>,
    a: &Obj<'_>,
    b: &Obj<'_>,
    mid: &Obj<'_>,
) -> PyResult<()> {
    for r in rings(face)? {
        let n = r.len()?;
        for i in 0..n {
            let x = r.get_item(i)?;
            let y = r.get_item((i + 1) % n)?;
            if x.is(a) && y.is(b) || x.is(b) && y.is(a) {
                r.call_method1("insert", (i + 1, mid))?;
                return Ok(());
            }
        }
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn collapse_collinear(mesh: &Obj<'_>) -> PyResult<usize> {
    let py = mesh.py();
    let mut count = 0;
    loop {
        let vs = py
            .get_type::<PyList>()
            .call1((mesh.getattr("vertices")?,))?;
        let mut next = None;
        for v in vs.try_iter()? {
            let v = v?;
            if mesh
                .call_method1(pyo3::intern!(py, "collapsible_vertex"), (&v,))?
                .is_truthy()?
            {
                next = Some(v);
                break;
            }
        }
        let Some(v) = next else { return Ok(count) };
        mesh.call_method1("collapse_vertex", (v,))?;
        count += 1;
    }
}
#[pyfunction]
pub(crate) fn curve_edges<'py>(mesh: &Obj<'py>, edge: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let field = pyo3::intern!(mesh.py(), "curve");
    let Some(curve) = edge.getattr_opt(field)?.filter(|c| !c.is_none()) else {
        return Ok(vec![edge.clone()]);
    };
    let mut out = Vec::new();
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        if e.getattr(field)?.eq(&curve)? {
            out.push(e);
        }
    }
    Ok(out)
}
#[pyfunction]
pub(crate) fn dominant_attrs<'py>(faces: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mut best = PyDict::new(faces.py()).into_any();
    let mut area = -1.;
    for f in faces.try_iter()? {
        let f = f?;
        let attrs = f.getattr("attrs")?;
        if attrs.is_truthy()? {
            let a = f.call_method0("area")?.extract::<f64>()?;
            if a > area {
                best = attrs;
                area = a;
            }
        }
    }
    Ok(best)
}
#[pyfunction]
pub(crate) fn loop_area(ring: &Obj<'_>) -> PyResult<f64> {
    Ok(ring_newell(ring)?.length() * 0.5)
}
#[pyfunction]
pub(crate) fn reverse_rings(face: &Obj<'_>) -> PyResult<()> {
    for ring in rings(face)? {
        ring.call_method0("reverse")?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn mesh_key<'py>(p: &Obj<'py>) -> PyResult<Bound<'py, pyo3::types::PyTuple>> {
    let py = p.py();
    let key = xyz(p)?.to_array().map(|x| (x * 10000.).round_ties_even());
    if key.iter().all(|x| x.abs() < (1u64 << 62) as f64) {
        pyo3::types::PyTuple::new(py, key.map(|x| x as i64))
    } else {
        let mut out = Vec::new();
        for x in key {
            out.push(pyo3::types::PyFloat::new(py, x).call_method0("__round__")?);
        }
        pyo3::types::PyTuple::new(py, out)
    }
}

fn ring_newell(ring: &Obj<'_>) -> PyResult<glam::DVec3> {
    let pos = pyo3::intern!(ring.py(), "position");
    let read = |v: Obj<'_>| xyz(&v.getattr(pos)?);
    let points = if let Ok(l) = ring.cast_exact::<PyList>() {
        l.iter().map(read).collect::<PyResult<Vec<_>>>()?
    } else {
        ring.try_iter()?
            .map(|v| read(v?))
            .collect::<PyResult<Vec<_>>>()?
    };
    Ok(crate::polygon::raw_newell(&points))
}

#[pyfunction]
pub(crate) fn ring_normal(ring: &Obj<'_>) -> PyResult<crate::math::Vec3> {
    let n = ring_newell(ring)?;
    let l = n.length();
    Ok((if l < 1e-9 { glam::DVec3::Z } else { n / l }).into())
}
#[pyfunction]
pub(crate) fn orient_union(
    outer: &Obj<'_>,
    holes: &Obj<'_>,
    normal: &Obj<'_>,
    face_type: &Obj<'_>,
) -> PyResult<()> {
    let aim = xyz(normal)?;
    let aligned = |ring: &Obj| -> PyResult<f64> {
        Ok(xyz(&face_type.call1((ring,))?.call_method0("normal")?)?.dot(aim))
    };
    if aligned(outer)? < 0. {
        outer.call_method0("reverse")?;
    }
    for hole in holes.try_iter()? {
        let hole = hole?;
        if aligned(&hole)? > 0. {
            hole.call_method0("reverse")?;
        }
    }
    Ok(())
}

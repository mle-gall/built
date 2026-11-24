// SPDX-License-Identifier: GPL-3.0-or-later
//! Walk existing materialised incidence without reissuing public handles.
use crate::math::xyz;
use crate::object_weld::rings;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
type Obj<'py> = Bound<'py, PyAny>;

#[pyfunction]
pub(crate) fn ring_positions<'py>(ring: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let pos = pyo3::intern!(ring.py(), "position");
    ring.try_iter()?.map(|v| v?.getattr(pos)).collect()
}
#[pyfunction]
pub(crate) fn hole_positions<'py>(holes: &Obj<'py>) -> PyResult<Vec<Vec<Obj<'py>>>> {
    holes.try_iter()?.map(|r| ring_positions(&r?)).collect()
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
    let p = ring_positions(ring)?
        .iter()
        .map(xyz)
        .collect::<PyResult<Vec<_>>>()?;
    Ok(crate::polygon::raw_newell(&p).length() * 0.5)
}
#[pyfunction]
pub(crate) fn reverse_rings(face: &Obj<'_>) -> PyResult<()> {
    for ring in rings(face)? {
        ring.call_method0("reverse")?;
    }
    Ok(())
}

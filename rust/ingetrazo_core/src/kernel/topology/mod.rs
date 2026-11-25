// SPDX-License-Identifier: GPL-3.0-or-later
//! Public modelling topology. Store-backed edits use stable element keys.
//! The same planners accept detached vertex/edge/face handles from scripts.
mod cycles;
mod faces;
mod repair;
use crate::{
    math::{xyz, Vec3},
    object_outline::{self, positions, vectors, FaceOutline},
    tools::offset_tool::closed_offset,
};
use pyo3::{
    prelude::*,
    types::PyTuple,
};
type Obj<'py> = Bound<'py, PyAny>;
fn store_api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core.store_mesh")
}

#[pyfunction]
#[pyo3(signature=(p1,p2,p3,p4,tol=1e-4))]
fn segment_intersection(
    p1: &Obj<'_>,
    p2: &Obj<'_>,
    p3: &Obj<'_>,
    p4: &Obj<'_>,
    tol: f64,
) -> PyResult<Option<Vec3>> {
    object_outline::segment_hit(p1.py(), p1, p2, p3, p4, tol)
}
#[pyfunction]
#[pyo3(signature=(p,a,b,tol=1e-7))]
fn _on_segment_2d(p: [f64; 2], a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
    super::object_boundary::segment_contains(p, a, b, tol)
}
#[pyfunction]
fn _strictly_inside_2d(p: [f64; 2], poly: Vec<[f64; 2]>) -> bool {
    object_outline::polygon_contains_2d(p, poly, true)
}
#[pyfunction]
fn _point_inside_2d(pt: [f64; 2], poly: Vec<[f64; 2]>) -> bool {
    object_outline::polygon_contains_2d(pt, poly, false)
}
#[pyfunction]
fn _faces_coplanar(n1: &Obj<'_>, n2: &Obj<'_>) -> PyResult<bool> {
    Ok(xyz(n1)?
        .normalize_or_zero()
        .dot(xyz(n2)?.normalize_or_zero())
        .abs()
        > 0.999)
}
#[pyfunction]
fn _locate_on_loop(vertices: &Obj<'_>, point: &Obj<'_>) -> PyResult<Option<(&'static str, usize)>> {
    object_outline::locate_boundary(vertices, point)
}
#[pyfunction]
fn _loop_with_point<'py>(
    r#loop: &Obj<'py>,
    ka: [f64; 3],
    kb: [f64; 3],
    point: &Obj<'py>,
) -> PyResult<Option<Vec<Obj<'py>>>> {
    super::object_boundary::loop_with_point(r#loop, ka, kb, point)
}
#[pyfunction]
#[pyo3(signature=(faces,edge_a,edge_b,point,skip_endpoints=None))]
fn split_edge_in_faces<'py>(
    faces: &Obj<'py>,
    edge_a: &Obj<'py>,
    edge_b: &Obj<'py>,
    point: &Obj<'py>,
    skip_endpoints: Option<&Obj<'py>>,
) -> PyResult<Vec<(Obj<'py>, Vec<Obj<'py>>)>> {
    super::object_boundary::split_boundaries(
        faces,
        edge_a,
        edge_b,
        point,
        skip_endpoints.unwrap_or(PyTuple::empty(point.py()).as_any()),
    )
}
#[pyfunction]
fn _same_cycle(c1: &Obj<'_>, c2: &Obj<'_>) -> PyResult<bool> {
    super::object_boundary::same_cycle(c1, c2)
}
#[pyfunction]
fn offset_loop(r#loop: &Obj<'_>, normal: &Obj<'_>, d: f64) -> PyResult<Option<Vec<Vec3>>> {
    Ok(closed_offset(&positions(r#loop)?, xyz(normal)?.to_array(), d).map(vectors))
}
#[pyfunction]
#[pyo3(signature=(r#loop,normal,sign=1.,limit=10.))]
fn max_offset_distance(r#loop: &Obj<'_>, normal: &Obj<'_>, sign: f64, limit: f64) -> PyResult<f64> {
    let ring = positions(r#loop)?;
    let n = xyz(normal)?.to_array();
    let mut lo = 1e-4;
    let mut hi = limit.abs();
    if closed_offset(&ring, n, sign * lo).is_none() {
        return Ok(0.);
    }
    for _ in 0..40 {
        let mid = (lo + hi) / 2.;
        if closed_offset(&ring, n, sign * mid).is_some() {
            lo = mid
        } else {
            hi = mid
        }
    }
    Ok(lo)
}
#[pyfunction]
fn _loop_inside_loop(inner: &Obj<'_>, outer: &Obj<'_>, normal: &Obj<'_>) -> PyResult<bool> {
    object_outline::inside_loop(inner, outer, normal, inner.py().version_info().minor >= 12)
}
#[pyfunction]
fn _maximal_holes<'py>(holes: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mask = object_outline::object_hole_mask(holes)?;
    holes
        .try_iter()?
        .zip(mask)
        .filter_map(|(h, k)| k.then_some(h))
        .collect()
}
#[pyfunction]
fn _point_in_face_solid(face: &Obj<'_>, point: &Obj<'_>) -> PyResult<bool> {
    FaceOutline::new(face)?.solid_at(point)
}

#[pyfunction]
fn __getattr__<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    match name {
        "Face" | "Edge" => py.import("ingetrazo_core.geometry")?.getattr(name),
        _ => Err(pyo3::exceptions::PyAttributeError::new_err(name.to_owned())),
    }
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "core.topology")?;
    let s = &sub;
    let drawing = m.getattr("object_drawing")?;
    cycles::register(s)?;
    faces::register(s)?;
    repair::register(s)?;
    for (name, target) in [
        ("find_duplicate_edge", "find_duplicate_edge"),
        ("_smallest_cycle", "smallest_cycle"),
        ("_point_on_seg_incl", "point_on_segment"),
        ("face_exists", "face_exists"),
    ] {
        s.add(name, drawing.getattr(target)?)?;
    }
    for (name, target) in [
        ("is_planar", "polygon_is_planar"),
        ("QVector3D", "Vec3"),
        ("_loops_cross_2d", "loops_cross"),
        ("_loop_edges", "loop_edges"),
        ("_face_all_edges", "face_edges"),
        ("_segment_on_face_boundary", "boundary_carries"),
        ("face_is_bordered", "bordered_object"),
        ("_interior_point", "interior_point"),
        ("_HolesApart", "HoleBounds"),
    ] {
        s.add(name, m.getattr(target)?)?;
    }
    s.add(
        "DEFECT_KINDS",
        PyTuple::new(m.py(), crate::store::landing::DEFECTS)?,
    )?;
    s.add("_KEY_DECIMALS", 4)?;
    s.add("_PLANAR_TOLERANCE", 1e-3)?;
    s.add("_SPLIT_TOLERANCE", 1e-4)?;
    s.add("_HEAL_FACE_CAP", 3000)?;
    s.add("_BOX_PAD", 1e-2)?;
    s.add_function(wrap_pyfunction!(segment_intersection, s)?)?;
    s.add_function(wrap_pyfunction!(_on_segment_2d, s)?)?;
    s.add_function(wrap_pyfunction!(_strictly_inside_2d, s)?)?;
    s.add_function(wrap_pyfunction!(_point_inside_2d, s)?)?;
    s.add_function(wrap_pyfunction!(_faces_coplanar, s)?)?;
    s.add_function(wrap_pyfunction!(_locate_on_loop, s)?)?;
    s.add_function(wrap_pyfunction!(_loop_with_point, s)?)?;
    s.add_function(wrap_pyfunction!(split_edge_in_faces, s)?)?;
    s.add_function(wrap_pyfunction!(_same_cycle, s)?)?;
    s.add_function(wrap_pyfunction!(offset_loop, s)?)?;
    s.add_function(wrap_pyfunction!(max_offset_distance, s)?)?;
    s.add_function(wrap_pyfunction!(_loop_inside_loop, s)?)?;
    s.add_function(wrap_pyfunction!(_maximal_holes, s)?)?;
    s.add_function(wrap_pyfunction!(_point_in_face_solid, s)?)?;
    s.add_function(wrap_pyfunction!(__getattr__, s)?)?;
    m.add_submodule(s)
}

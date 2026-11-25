// SPDX-License-Identifier: GPL-3.0-or-later
//! Public arrangement operations on scripting points and host drawing commands.
use super::geometry::{plane_basis as basis, Hypot, V3};
use super::planar::{self, Cell, Metrics, Point, Ring, TOL};
use crate::{
    math::{xyz, Vec3},
    object_planes,
};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyFloat, PyList, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
#[pyfunction]
pub(crate) fn plane_basis(normal: &Obj<'_>) -> PyResult<(Vec3, Vec3)> {
    let p = xyz(normal)?;
    let (u, v) = basis(V3(p.x, p.y, p.z), Hypot::LibStdCxx);
    Ok((
        DVec3::new(u.0, u.1, u.2).into(),
        DVec3::new(v.0, v.1, v.2).into(),
    ))
}
#[pyfunction]
fn project_loop(ring: &Obj<'_>, normal: &Obj<'_>) -> PyResult<Vec<(f64, f64)>> {
    let (u, v) = plane_basis(normal)?;
    let (u, v) = (DVec3::from_array(u.coords()), DVec3::from_array(v.coords()));
    ring.try_iter()?
        .map(|p| {
            let p = xyz(&p?)?;
            Ok((p.dot(u), p.dot(v)))
        })
        .collect()
}

#[pyfunction]
fn loops_overlap(py: Python<'_>, a: Ring, b: Ring) -> PyResult<bool> {
    let metrics = Metrics::new(py)?;
    Ok(planar::inside(&b, metrics.interior(&a)?)
        || planar::inside(&a, metrics.interior(&b)?)
        || a.iter().any(|p| planar::inside(&b, *p))
        || b.iter().any(|p| planar::inside(&a, *p)))
}

#[pyfunction]
fn grid_key(py: Python<'_>, x: f64, y: f64) -> PyResult<Bound<'_, PyTuple>> {
    match Cell::at([x, y], TOL) {
        Some(c) => c.tuple(py),
        None => PyTuple::new(
            py,
            [
                PyFloat::new(py, x / TOL).call_method0("__round__")?,
                PyFloat::new(py, y / TOL).call_method0("__round__")?,
            ],
        ),
    }
}
#[pyfunction]
fn signed_area(r#loop: Ring) -> f64 {
    planar::area(&r#loop)
}
#[pyfunction]
fn area_floor(py: Python<'_>, xy: Ring) -> PyResult<f64> {
    Metrics::native(py.version_info().minor >= 12).floor(&xy)
}
#[pyfunction]
fn point_in_polygon(p: Point, poly: Ring) -> bool {
    planar::inside(&poly, p)
}
#[pyfunction]
fn classify<'py>(
    loops: &Obj<'py>,
    coords: &Obj<'py>,
) -> PyResult<Vec<(Vec<Obj<'py>>, Vec<Vec<Obj<'py>>>)>> {
    let rings = loops
        .try_iter()?
        .map(|r| {
            r?.try_iter()?
                .map(|k| coords.get_item(k?))
                .collect::<PyResult<Vec<_>>>()
        })
        .collect::<PyResult<Vec<_>>>()?;
    let values = rings
        .iter()
        .map(|r| {
            r.iter()
                .map(|p| p.extract::<Point>())
                .collect::<PyResult<Ring>>()
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(planar::nesting(&Metrics::new(loops.py())?, &values, true)?
        .into_iter()
        .map(|(i, hs)| {
            (
                rings[i].clone(),
                hs.into_iter().map(|j| rings[j].clone()).collect(),
            )
        })
        .collect())
}
#[pyfunction]
fn flat_drawing(scene: &Obj<'_>, new_points: &Obj<'_>) -> PyResult<bool> {
    Ok(object_planes::coplanar_plane(
        new_points,
        1e-4,
        Some(&scene.getattr("mesh")?.getattr("vertices")?),
    )?
    .is_some())
}
#[pyfunction]
fn busy_plane<'py>(
    scene: &Obj<'py>,
    new_points: &Obj<'py>,
) -> PyResult<Option<(Obj<'py>, Obj<'py>)>> {
    let Some((o, n)) = object_planes::coplanar_plane(new_points, 1e-4, None)? else {
        return Ok(None);
    };
    let n = n.into_pyobject(scene.py())?.into_any();
    let busy = object_planes::has_plane_edge(
        &scene.getattr("mesh")?.getattr("edges")?,
        Some((o.clone(), n.clone())),
        false,
    )?;
    Ok(busy.then_some((o, n)))
}

fn curve_parts<'py>(
    scene: &Obj<'py>,
    pts: &Obj<'py>,
    closed: bool,
) -> PyResult<(Bound<'py, PyList>, bool)> {
    let py = scene.py();
    let api = py.import("core.history")?;
    let after = PyList::empty(py);
    let copy = || py.get_type::<PyList>().call1((pts,));
    let tag = || {
        let kw = PyDict::new(py);
        kw.set_item("closed", closed)?;
        api.getattr("TagCurveCommand")?.call((copy()?,), Some(&kw))
    };
    // Arc builds its tag before asking the plane. Closed tools ask the plane first.
    let arc = if closed { None } else { Some(tag()?) };
    let flat = flat_drawing(scene, pts)?;
    let plane = if flat { None } else { busy_plane(scene, pts)? };
    if closed && !flat {
        after.append(api.getattr("AddFaceCommand")?.call1((copy()?,))?)?;
    }
    after.append(match arc {
        Some(c) => c,
        None => tag()?,
    })?;
    if flat {
        after.append(api.call_method0("RebuildPlanarFacesCommand")?)?;
    }
    if let Some(p) = plane {
        after.append(api.call_method1("RebuildPlaneFacesCommand", p)?)?;
    }
    Ok((after, flat))
}
#[pyfunction]
fn curve_extras<'py>(scene: &Obj<'py>, pts: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    Ok(curve_parts(scene, pts, true)?.0)
}
#[pyfunction]
#[pyo3(signature=(scene,pts,close_to=None,trim=None))]
fn arc_command<'py>(
    scene: &Obj<'py>,
    pts: &Obj<'py>,
    close_to: Option<&Obj<'py>>,
    trim: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let n = pts.len()?;
    let segments = PyList::empty(py);
    if let Some(c) = close_to {
        segments.append((c, pts.get_item(0)?))?;
    }
    for i in 1..n {
        segments.append((pts.get_item(i - 1)?, pts.get_item(i)?))?;
    }
    if let Some(c) = close_to {
        segments.append((pts.get_item(-1)?, c))?;
    }
    let mut remove = None;
    if let Some(t) = trim {
        if t.is_truthy()? {
            let factory = py.import("core.arrangement")?.getattr("_Seg")?;
            let edges = PyList::empty(py);
            for pair in t.try_iter()? {
                let pair = pair?;
                edges.append(factory.call1((
                    Vec3::from(xyz(&pair.get_item(0)?)?),
                    Vec3::from(xyz(&pair.get_item(1)?)?),
                ))?)?;
            }
            remove = Some(
                py.import("core.history")?
                    .call_method1("DeleteEdgesCommand", (edges,))?,
            );
        }
    }
    let (extra, flat) = curve_parts(scene, pts, false)?;
    if let Some(cmd) = remove {
        extra.append(cmd)?;
    }
    let kw = PyDict::new(py);
    kw.set_item("detect_faces", !flat)?;
    kw.set_item("extra", extra)?;
    py.import("core.edits")?
        .getattr("build_add_edges")?
        .call((scene, segments), Some(&kw))
}

type Region = (Vec<Vec3>, Vec<Vec<Vec3>>);
type Rebuild = (Vec<(Vec3, Vec3)>, Vec<Region>);

fn rebuild(
    segments: &Obj<'_>,
    origin: &Obj<'_>,
    normal: &Obj<'_>,
    edges: bool,
) -> PyResult<Rebuild> {
    let (u, v) = plane_basis(normal)?;
    let py = origin.py();
    super::arrangement::arrange_plane(
        segments,
        origin,
        (
            u.into_pyobject(py)?.into_any(),
            v.into_pyobject(py)?.into_any(),
        ),
        edges,
    )
}
#[pyfunction]
pub(crate) fn planar_arrangement(
    segments_3d: &Obj<'_>,
    origin: &Obj<'_>,
    normal: &Obj<'_>,
) -> PyResult<Vec<Region>> {
    Ok(rebuild(segments_3d, origin, normal, false)?.1)
}
#[pyfunction]
pub(crate) fn planar_rebuild(segments_3d: &Obj<'_>, origin: &Obj<'_>, normal: &Obj<'_>) -> PyResult<Rebuild> {
    rebuild(segments_3d, origin, normal, true)
}
#[pyfunction]
fn _interior_point(py: Python<'_>, r#loop: Ring) -> PyResult<(f64, f64)> {
    planar::plane_interior_point(py, r#loop)
}
#[pyfunction]
#[pyo3(signature=(outer,holes=Vec::new()))]
fn scan_interior_point(outer: Ring, holes: Vec<Ring>) -> Option<(f64, f64)> {
    planar::plane_scan_point(outer, holes)
}
#[pyfunction]
fn _build_graph(
    py: Python<'_>,
    segments_2d: Vec<[Point; 2]>,
) -> PyResult<(Bound<'_, PyDict>, Bound<'_, PyDict>)> {
    super::arrangement::plane_split_graph(py, segments_2d)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "core.arrangement")?;
    sub.add_function(wrap_pyfunction!(project_loop, &sub)?)?;
    sub.add_function(wrap_pyfunction!(loops_overlap, &sub)?)?;
    sub.add_function(wrap_pyfunction!(grid_key, &sub)?)?;
    sub.add_function(wrap_pyfunction!(plane_basis, &sub)?)?;
    sub.add_function(wrap_pyfunction!(area_floor, &sub)?)?;
    sub.add_function(wrap_pyfunction!(signed_area, &sub)?)?;
    sub.add_function(wrap_pyfunction!(point_in_polygon, &sub)?)?;
    sub.add_function(wrap_pyfunction!(classify, &sub)?)?;
    sub.add_function(wrap_pyfunction!(flat_drawing, &sub)?)?;
    sub.add_function(wrap_pyfunction!(busy_plane, &sub)?)?;
    sub.add_function(wrap_pyfunction!(curve_extras, &sub)?)?;
    sub.add_function(wrap_pyfunction!(arc_command, &sub)?)?;
    sub.add_function(wrap_pyfunction!(planar_arrangement, &sub)?)?;
    sub.add_function(wrap_pyfunction!(planar_rebuild, &sub)?)?;
    sub.add_function(wrap_pyfunction!(_interior_point, &sub)?)?;
    sub.add_function(wrap_pyfunction!(scan_interior_point, &sub)?)?;
    sub.add_function(wrap_pyfunction!(_build_graph, &sub)?)?;
    sub.add_function(wrap_pyfunction!(
        super::arrangement::plane_segment_parameter,
        &sub
    )?)?;
    sub.add_function(wrap_pyfunction!(
        super::arrangement::plane_intersections,
        &sub
    )?)?;
    sub.add_function(wrap_pyfunction!(object_planes::coplanar_plane, &sub)?)?;
    for (name, native) in [
        ("_key", "grid_key"),
        ("_classify", "classify"),
        ("_area_floor", "area_floor"),
        ("_signed_area", "signed_area"),
        ("_point_in_polygon", "point_in_polygon"),
        ("_intersections", "plane_intersections"),
        ("_on_seg", "plane_segment_parameter"),
    ] {
        sub.add(name, sub.getattr(native)?)?;
    }
    sub.add("_TOL", TOL)?;
    let kw = PyDict::new(m.py());
    kw.set_item("module", "core.arrangement")?;
    sub.add(
        "_Seg",
        m.py()
            .import("collections")?
            .getattr("namedtuple")?
            .call(("_Seg", "a b"), Some(&kw))?,
    )?;
    m.add_submodule(&sub)
}

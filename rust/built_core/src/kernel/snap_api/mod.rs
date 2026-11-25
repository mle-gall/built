// SPDX-License-Identifier: GPL-3.0-or-later
//! Script-visible inference helpers. Drawing axes are shared with the active
//! editing context. Returned guides preserve their public vector and edge handles.
mod gesture;
mod guides;
use crate::{
    math::{xyz, Vec3},
    snap::{self, P},
    snap_curve,
};
use glam::DVec3;
use pyo3::{
    exceptions::PyAttributeError,
    prelude::*,
    types::{PyDict, PyFrozenSet, PyList, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
type Module<'p> = Bound<'p, PyModule>;
const AXIS: [&str; 3] = ["x", "y", "z"];
fn coords(v: &Obj<'_>) -> PyResult<P> {
    Ok(xyz(v)?.to_array())
}
fn vector(p: P) -> Vec3 {
    DVec3::from_array(p).into()
}
fn point(py: Python<'_>, p: P) -> PyResult<Obj<'_>> {
    Ok(Bound::new(py, vector(p))?.into_any())
}
fn points(v: &Obj<'_>) -> PyResult<Vec<P>> {
    v.try_iter()?.map(|p| coords(&p?)).collect()
}
fn axes(m: &Module<'_>) -> PyResult<[P; 3]> {
    let axes = m.getattr("_AXIS_VECTORS")?;
    Ok([
        coords(&axes.get_item("x")?)?,
        coords(&axes.get_item("y")?)?,
        coords(&axes.get_item("z")?)?,
    ])
}
#[pyfunction(pass_module)]
fn _axis_tuples(m: &Module<'_>) -> PyResult<((f64, f64, f64), (f64, f64, f64), (f64, f64, f64))> {
    let [x, y, z] = axes(m)?;
    Ok((x.into(), y.into(), z.into()))
}
fn work_axis(m: &Module<'_>, forward: &Obj<'_>, threshold: f64) -> PyResult<Option<P>> {
    let world = m
        .py()
        .import("built_core.axes")?
        .call_method0("is_world")?
        .is_truthy()?;
    Ok(snap::drawing_axis(
        xyz(forward)?.normalize_or_zero().to_array(),
        axes(m)?,
        threshold,
        1e-9,
        world,
    )
    .map(|(_, p)| p))
}
#[pyfunction(pass_module,signature=(forward,scene_center,threshold=0.97))]
fn first_point_work_plane<'p>(
    m: &Module<'p>,
    forward: &Obj<'p>,
    scene_center: Obj<'p>,
    threshold: f64,
) -> PyResult<Option<(Obj<'p>, Vec3)>> {
    Ok(work_axis(m, forward, threshold)?.map(|p| (scene_center, vector(p))))
}
#[pyfunction(pass_module,signature=(point,r#ref,forward,threshold=0.97))]
fn project_to_view_plane<'p>(
    m: &Module<'p>,
    point: Obj<'p>,
    r#ref: &Obj<'p>,
    forward: &Obj<'p>,
    threshold: f64,
) -> PyResult<Obj<'p>> {
    let Some(a) = work_axis(m, forward, threshold)? else {
        return Ok(point);
    };
    let p = xyz(&point)?;
    let a = DVec3::from_array(a);
    Ok(Bound::new(m.py(), Vec3::from(p + a * (xyz(r#ref)? - p).dot(a)))?.into_any())
}
#[pyfunction(pass_module)]
fn _detect_axis_alignment(
    m: &Module<'_>,
    start: &Obj<'_>,
    candidate: &Obj<'_>,
    angle_deg: f64,
) -> PyResult<Option<&'static str>> {
    Ok(snap::drawing_axis(
        (xyz(candidate)? - xyz(start)?).to_array(),
        axes(m)?,
        angle_deg.to_radians().cos(),
        1e-6,
        false,
    )
    .map(|(k, _)| AXIS[k]))
}
#[pyfunction(pass_module)]
fn _detect_axis_on_screen(
    m: &Module<'_>,
    start: &Obj<'_>,
    candidate_world: &Obj<'_>,
    candidate_pixel: [f64; 2],
    world_to_pixel: &Obj<'_>,
    threshold_px: f64,
) -> PyResult<Option<&'static str>> {
    let s = xyz(start)?;
    let reach = s.distance(xyz(candidate_world)?);
    if reach < 1e-6 {
        return Ok(None);
    }
    let Some(o) = world_to_pixel
        .call1((start,))?
        .extract::<Option<[f64; 2]>>()?
    else {
        return Ok(None);
    };
    let mut samples = [None; 3];
    for (sample, axis) in samples.iter_mut().zip(axes(m)?) {
        *sample = world_to_pixel
            .call1((Vec3::from(s + DVec3::from_array(axis) * reach),))?
            .extract()?;
    }
    Ok(snap::screen_axis(o, samples, candidate_pixel, threshold_px).map(|k| AXIS[k]))
}
#[pyfunction(pass_module,signature=(edge,mode,plane_normal=None))]
fn _direction_from_edge(
    m: &Module<'_>,
    edge: &Obj<'_>,
    mode: &str,
    plane_normal: Option<&Obj<'_>>,
) -> PyResult<Option<Vec3>> {
    let n = match plane_normal {
        Some(v) => coords(v)?,
        None => axes(m)?[2],
    };
    Ok(snap::edge_direction(
        coords(&edge.getattr("a")?)?,
        coords(&edge.getattr("b")?)?,
        mode == "perpendicular",
        n,
    )
    .map(vector))
}
#[pyfunction(signature=(p,u,a,b,tol=1e-3))]
fn _line_segment_intersection(
    p: &Obj<'_>,
    u: &Obj<'_>,
    a: &Obj<'_>,
    b: &Obj<'_>,
    tol: f64,
) -> PyResult<Option<Vec3>> {
    Ok(snap::lock_crossing(coords(p)?, coords(u)?, coords(a)?, coords(b)?, tol).map(vector))
}
#[pyfunction(signature=(p,a,b,tol=1e-3))]
fn _point_on_segment_world(p: &Obj<'_>, a: &Obj<'_>, b: &Obj<'_>, tol: f64) -> PyResult<bool> {
    super::snap_surfaces::point_on_segment(p, a, b, tol)
}
#[pyfunction(signature=(points,tol_rel=0.01))]
fn fit_circle(points: &Obj<'_>, tol_rel: f64) -> PyResult<Option<(Vec3, f64)>> {
    Ok(
        snap_curve::snap_circle(points.py(), self::points(points)?, tol_rel)
            .map(|(p, r)| (vector(p), r)),
    )
}
#[pyfunction(signature=(points,fit=None))]
fn arc_sweep_midpoint(points: &Obj<'_>, fit: Option<&Obj<'_>>) -> PyResult<Option<Vec3>> {
    let coords_vec = self::points(points)?;
    let circle = match fit {
        None if coords_vec.len() < 3 => None,
        None => snap_curve::snap_circle(points.py(), coords_vec.clone(), 0.01),
        Some(f) => Some((coords(&f.get_item(0)?)?, f.get_item(1)?.extract()?)),
    };
    Ok(circle
        .and_then(|(c, r)| snap_curve::arc_midpoint(coords_vec, c, r))
        .map(vector))
}
#[pyfunction(signature=(face,xform=None))]
fn curve_centers_of_face<'p>(face: &Obj<'p>, xform: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let py = face.py();
    let api = py.import("built_core.store_mesh")?;
    if face.is_instance(&api.getattr("FaceView")?)? {
        let store = api.call_method1("_store_of", (face.getattr("_m")?,))?;
        let key = face.getattr("key")?;
        if !store.is_none() && store.call_method1("is_live", (&key,))?.is_truthy()? {
            return store.call_method1("curve_centers", (key, xform));
        }
    }
    Ok(crate::snap_boundary::curve_centers(face, xform)?
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction(signature=(face,xform=None))]
fn face_center_of_face(face: &Obj<'_>, xform: Option<&Obj<'_>>) -> PyResult<Option<Vec3>> {
    let mut rings = vec![face.getattr("vertices")?];
    let holes = face.getattr("holes")?;
    if !holes.is_none() {
        rings.extend(holes.try_iter()?.collect::<PyResult<Vec<_>>>()?);
    }
    let mut coords = Vec::new();
    for ring in rings {
        let mut row = points(&ring)?;
        if let Some(t) = xform {
            for p in &mut row {
                *p = self::coords(&t.call_method1("map", (vector(*p),))?)?;
            }
        }
        coords.push(row);
    }
    Ok(snap::face_center(face.py(), coords).map(vector))
}
#[pyfunction(signature=(vertex,line_start,line_dir,tol=1e-4))]
fn _vertex_on_line(
    vertex: &Obj<'_>,
    line_start: &Obj<'_>,
    line_dir: &Obj<'_>,
    tol: f64,
) -> PyResult<bool> {
    let rel = xyz(vertex)? - xyz(line_start)?;
    let dir = xyz(line_dir)?;
    Ok((rel - dir * rel.dot(dir)).length() < tol)
}
#[pyfunction]
fn _circle_tangent_points(c: &Obj<'_>, r: f64, n: &Obj<'_>, p: &Obj<'_>) -> PyResult<Vec<Vec3>> {
    Ok(
        snap_curve::circle_tangents((coords(c)?, r, coords(n)?), coords(p)?)
            .into_iter()
            .map(vector)
            .collect(),
    )
}
#[pyfunction]
fn _pairs_that_may_meet<'p>(edges: &Obj<'p>) -> PyResult<Obj<'p>> {
    PyList::new(
        edges.py(),
        super::snap_surfaces::pairs_that_may_meet(edges)?,
    )?
    .call_method0("__iter__")
}
#[pyfunction(pass_module)]
fn __getattr__<'p>(m: &Module<'p>, name: &str) -> PyResult<Obj<'p>> {
    let v = match name {
        "_AXIS_VECTORS" => m.py().import("built_core.axes")?.getattr("AXES")?,
        "Edge" => m.py().import("built_core.geometry")?.getattr("Edge")?,
        "kernel" => m.py().import("built_core")?.into_any(),
        _ => return Err(PyAttributeError::new_err(name.to_owned())),
    };
    m.setattr(name, &v)?;
    Ok(v)
}

#[pyfunction(signature=(cls,point,normal,contains,context=None))]
fn patch_new<'p>(
    cls: &Obj<'p>,
    point: &Obj<'p>,
    normal: &Obj<'p>,
    contains: &Obj<'p>,
    context: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    cls.py()
        .get_type::<PyTuple>()
        .call_method1("__new__", (cls, (point, normal, contains, context)))
}
fn types(m: &Module<'_>, root: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    // SnapResult is a mutable public dataclass used by extension tools.
    let dc = py.import("dataclasses")?;
    let fields = PyList::empty(py);
    fields.append(("point", "QVector3D"))?;
    fields.append(("kind", "str"))?;
    for (name, ty) in [
        ("color", "tuple[float, float, float]"),
        ("axis", "Optional[str]"),
        ("guide", "Optional[tuple]"),
        ("guide_color", "Optional[tuple[float, float, float]]"),
        ("guides", "Optional[list]"),
        ("context", "Optional[str]"),
        ("label", "Optional[str]"),
        ("lock_color", "Optional[tuple[float, float, float]]"),
        ("edge", "Optional[Edge]"),
    ] {
        let default = if name == "color" {
            m.getattr("COLOR_NONE")?
        } else {
            py.None().into_bound(py)
        };
        fields.append((name, ty, default))?;
    }
    let cls = dc.call_method1("make_dataclass", ("SnapResult", fields))?;
    cls.setattr("__module__", "built_core.snap")?;
    m.add("SnapResult", cls)?;
    let namespace = PyDict::new(py);
    namespace.set_item("__slots__", PyTuple::empty(py))?;
    namespace.set_item("__module__", "built_core.snap")?;
    namespace.set_item("__new__", wrap_pyfunction!(patch_new, m)?)?;
    let types = py.import("builtins")?.getattr("type")?;
    m.add(
        "SurfacePatch",
        types.call1(("SurfacePatch", (py.get_type::<PyTuple>(),), namespace))?,
    )?;
    let v = root.getattr("Vec3")?;
    let typing = py.import("typing")?;
    m.add("Optional", typing.getattr("Optional")?)?;
    m.add(
        "ProjectOntoLine",
        typing
            .getattr("Callable")?
            .get_item((PyList::new(py, [&v, &v])?, &v))?,
    )?;
    m.add("QVector3D", v)?;
    Ok(())
}
pub(super) fn register(root: &Module<'_>) -> PyResult<()> {
    let py = root.py();
    let m = PyModule::new(py, "built_core.snap")?;
    let green = (0.16, 0.62, 0.36);
    let cyan = (0.20, 0.66, 0.74);
    let red = (0.86, 0.22, 0.27);
    let magenta = (0.85, 0.30, 0.80);
    for (name, color) in [
        ("COLOR_ENDPOINT", green),
        ("COLOR_MIDPOINT", cyan),
        ("COLOR_ON_EDGE", red),
        ("COLOR_ON_FACE", (0.42, 0.46, 0.92)),
        ("COLOR_ORIGIN", (0.95, 0.45, 0.16)),
        ("COLOR_CLOSE", (0.20, 0.40, 0.78)),
        ("COLOR_AXIS_X", red),
        ("COLOR_AXIS_Y", green),
        ("COLOR_AXIS_Z", (0.20, 0.40, 0.78)),
        ("COLOR_REFERENCE", magenta),
        ("COLOR_EXTENSION", (0.55, 0.55, 0.58)),
        ("COLOR_LOCK_INTERSECT", (0.85, 0.20, 0.20)),
        ("COLOR_IN_GROUP", magenta),
        ("COLOR_TANGENT", cyan),
        ("COLOR_NONE", (0., 0., 0.)),
    ] {
        m.add(name, color)?;
    }
    let colors = PyDict::new(py);
    let names = PyDict::new(py);
    for k in AXIS {
        let name = k.to_uppercase();
        colors.set_item(k, m.getattr(format!("COLOR_AXIS_{name}"))?)?;
        names.set_item(k, name)?;
    }
    m.add("AXIS_COLORS", colors)?;
    m.add("AXIS_NAMES", names)?;
    m.add("_MAX_AXIS_REACH", 5.)?;
    m.add("_STICKY_AXIS_RELEASE_DEG", 20.)?;
    m.add("_IN_PLANE_TOL", 1e-3)?;
    m.add(
        "POINT_SNAP_KINDS",
        PyFrozenSet::new(
            py,
            [
                "endpoint",
                "midpoint",
                "intersection",
                "intersection_edge",
                "intersection_face",
                "on_edge",
                "on_line",
                "on_circle",
                "close",
                "origin",
                "center",
                "centre",
                "component_origin",
                "arc_midpoint",
                "quadrant",
            ],
        )?,
    )?;
    types(&m, root)?;
    for (name, source) in [
        ("face_plane_world", "face_plane_world"),
        ("_boundary_runs", "boundary_runs"),
        ("_resolve_point_candidates", "resolve_points"),
        ("_first_visible", "first_visible"),
        ("_closest_on_circle", "closest_rim"),
        ("_circle_circle_intersections", "circle_crossings"),
        ("_segment_circle_intersections", "segment_crossings"),
        ("_circle_quadrants", "quadrants"),
    ] {
        m.add(name, root.getattr(source)?)?;
    }
    let surfaces = root.getattr("snap_surfaces")?;
    m.add(
        "_closest_on_segment_2d",
        surfaces.getattr("closest_segment")?,
    )?;
    m.add(
        "face_crossing_near_ray",
        surfaces.getattr("face_crossing_near_ray")?,
    )?;
    macro_rules! add{($($f:path),*)=>{$(m.add_function(wrap_pyfunction!($f,&m)?)?;)*};}
    add!(
        first_point_work_plane,
        project_to_view_plane,
        _axis_tuples,
        _detect_axis_alignment,
        _detect_axis_on_screen,
        _direction_from_edge,
        _line_segment_intersection,
        _point_on_segment_world,
        fit_circle,
        arc_sweep_midpoint,
        curve_centers_of_face,
        face_center_of_face,
        _vertex_on_line,
        _pairs_that_may_meet,
        _circle_tangent_points,
        guides::_two_point_snap,
        guides::_first_point_from_point,
        guides::_in_plane_with_point_snap,
        guides::_from_point_snap,
        guides::_from_point_result,
        guides::_intersection_snap,
        guides::_edge_face_snap,
        guides::_lock_reference_snap,
        guides::_lock_line_snaps,
        gesture::compute_snap,
        __getattr__
    );
    root.add_submodule(&m)
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve acquired points and locked lines against the current projection.
use super::*;
use crate::{snap_guides as guides, snap_planes, snap_visibility::first_visible};
type Options<'p> = Bound<'p, PyDict>;

fn result<'p>(
    m: &Module<'p>,
    p: &Obj<'p>,
    kind: &str,
    color: &str,
    kw: Option<&Options<'p>>,
) -> PyResult<Obj<'p>> {
    m.getattr("SnapResult")?
        .call((p, kind, m.getattr(color)?), kw)
}
fn axis_color<'p>(m: &Module<'p>, k: Option<usize>) -> PyResult<Obj<'p>> {
    match k {
        Some(k) => m.getattr("AXIS_COLORS")?.get_item(AXIS[k]),
        None => m.getattr("COLOR_EXTENSION"),
    }
}
fn query<'p>(
    scene: &Obj<'p>,
    project: &Obj<'p>,
    x: f64,
    y: f64,
    existing: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    if let Some(q) = existing {
        return Ok(q.clone());
    }
    scene
        .py()
        .import("core.snap_query")?
        .call_method1("make_query", (scene.getattr("edges")?, project, (x, y)))
}
#[pyfunction(pass_module,signature=(refs,candidate,cx,cy,world_to_pixel,threshold_px,is_occluded=None))]
pub(super) fn _two_point_snap<'p>(
    m: &Module<'p>,
    refs: &Obj<'p>,
    candidate: &Obj<'p>,
    cx: f64,
    cy: f64,
    world_to_pixel: &Obj<'p>,
    threshold_px: f64,
    is_occluded: Option<&Obj<'p>>,
) -> PyResult<Option<Obj<'p>>> {
    let _ = candidate;
    let Some((cross, p, k, q, l)) = guides::cross_guide(
        refs,
        axes(m)?,
        [cx, cy],
        threshold_px,
        world_to_pixel,
        is_occluded,
    )?
    else {
        return Ok(None);
    };
    let kw = PyDict::new(m.py());
    kw.set_item("guide", (vector(coords(&p)?), &cross))?;
    kw.set_item("guide_color", axis_color(m, Some(k))?)?;
    kw.set_item(
        "guides",
        vec![(vector(coords(&q)?), &cross, axis_color(m, Some(l))?)],
    )?;
    result(m, &cross, "from_point", "COLOR_ENDPOINT", Some(&kw)).map(Some)
}

#[pyfunction(pass_module,signature=(r#ref,candidate,cx,cy,world_to_pixel,threshold_px,is_occluded=None,plane_normal=None,project_onto_line=None,free_3d=false))]
pub(super) fn _first_point_from_point<'p>(
    m: &Module<'p>,
    r#ref: &Obj<'p>,
    candidate: &Obj<'p>,
    cx: f64,
    cy: f64,
    world_to_pixel: &Obj<'p>,
    threshold_px: f64,
    is_occluded: Option<&Obj<'p>>,
    plane_normal: Option<&Obj<'p>>,
    project_onto_line: Option<&Obj<'p>>,
    free_3d: bool,
) -> PyResult<Option<Obj<'p>>> {
    let axes = m
        .getattr("_AXIS_VECTORS")?
        .call_method0("values")?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?;
    let Some((k, foot)) = guides::axis_guide(
        r#ref,
        candidate,
        axes,
        [cx, cy],
        threshold_px,
        world_to_pixel,
        is_occluded,
        plane_normal,
        project_onto_line.filter(|_| free_3d),
    )?
    else {
        return Ok(None);
    };
    let kw = PyDict::new(m.py());
    kw.set_item("axis", AXIS[k])?;
    kw.set_item("guide", (vector(coords(r#ref)?), &foot))?;
    kw.set_item("guide_color", axis_color(m, Some(k))?)?;
    result(m, &foot, "from_point", "COLOR_ENDPOINT", Some(&kw)).map(Some)
}
#[pyfunction(pass_module,signature=(r#ref,candidate,plane_normal,cx,cy,world_to_pixel,threshold_px,is_occluded=None))]
pub(super) fn _in_plane_with_point_snap<'p>(
    m: &Module<'p>,
    r#ref: &Obj<'p>,
    candidate: &Obj<'p>,
    plane_normal: Option<&Obj<'p>>,
    cx: f64,
    cy: f64,
    world_to_pixel: &Obj<'p>,
    threshold_px: f64,
    is_occluded: Option<&Obj<'p>>,
) -> PyResult<Option<Obj<'p>>> {
    let Some((k, foot, elbow, a, b, extra)) = guides::plane_guide(
        r#ref,
        candidate,
        plane_normal,
        axes(m)?,
        [cx, cy],
        threshold_px,
        world_to_pixel,
        is_occluded,
    )?
    else {
        return Ok(None);
    };
    let elbow = Bound::new(m.py(), elbow)?;
    let kw = PyDict::new(m.py());
    kw.set_item("axis", AXIS[k])?;
    kw.set_item("guide", (&elbow, &foot))?;
    kw.set_item("guide_color", axis_color(m, b)?)?;
    if extra {
        kw.set_item(
            "guides",
            vec![(vector(coords(r#ref)?), &elbow, axis_color(m, a)?)],
        )?;
    }
    result(m, &foot, "aligned", "COLOR_ENDPOINT", Some(&kw)).map(Some)
}
#[pyfunction(pass_module)]
pub(super) fn _from_point_result<'p>(
    m: &Module<'p>,
    foot: &Obj<'p>,
    r#ref: &Obj<'p>,
    kind: &str,
    color: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let kw = PyDict::new(m.py());
    kw.set_item("guide", (r#ref, foot))?;
    if kind == "from_point" {
        kw.set_item(
            "guide_color",
            axis_color(m, guides::reference_colour(foot, r#ref, axes(m)?)?)?,
        )?;
    }
    m.getattr("SnapResult")?
        .call((foot, kind, color), Some(&kw))
}

#[pyclass]
struct LineCallbacks {
    visible: Option<Py<PyAny>>,
    project: Option<Py<PyAny>>,
}
#[pymethods]
impl LineCallbacks {
    fn hidden(&self, py: Python<'_>, p: P) -> PyResult<bool> {
        self.visible
            .as_ref()
            .unwrap()
            .bind(py)
            .call1((vector(p),))?
            .is_truthy()
    }
    fn hidden_all(&self, py: Python<'_>, ps: Vec<P>) -> PyResult<Vec<bool>> {
        ps.into_iter().map(|p| self.hidden(py, p)).collect()
    }
    fn project_line(&self, py: Python<'_>, a: P, d: P) -> PyResult<Option<P>> {
        let p = self
            .project
            .as_ref()
            .unwrap()
            .bind(py)
            .call1((vector(a), vector(d)))?;
        if p.is_none() {
            Ok(None)
        } else {
            coords(&p).map(Some)
        }
    }
}
#[pyfunction(pass_module,signature=(scene,start_point,draw_dir,cx,cy,world_to_pixel,threshold_px,is_occluded,extra_point=None,axis_deg=10.,hovered_refs=false,line_dir=None,project_onto_line=None,edge_body=true,query=None))]
pub(super) fn _from_point_snap<'p>(
    m: &Module<'p>,
    scene: &Obj<'p>,
    start_point: Option<&Obj<'p>>,
    draw_dir: &Obj<'p>,
    cx: f64,
    cy: f64,
    world_to_pixel: &Obj<'p>,
    threshold_px: f64,
    is_occluded: Option<&Obj<'p>>,
    extra_point: Option<&Obj<'p>>,
    axis_deg: f64,
    hovered_refs: bool,
    line_dir: Option<&Obj<'p>>,
    project_onto_line: Option<&Obj<'p>>,
    edge_body: bool,
    query: Option<&Obj<'p>>,
) -> PyResult<Option<Obj<'p>>> {
    let Some(start) = start_point else {
        return Ok(None);
    };
    let Some(dir) = guides::reference_direction(draw_dir, line_dir, axes(m)?, axis_deg)? else {
        return Ok(None);
    };
    let py = m.py();
    let dir = Bound::new(py, dir)?.into_any();
    let q = self::query(scene, world_to_pixel, cx, cy, query)?;
    let adapters = Bound::new(
        py,
        LineCallbacks {
            visible: is_occluded.cloned().map(Bound::unbind),
            project: project_onto_line.cloned().map(Bound::unbind),
        },
    )?;
    let hidden = is_occluded
        .map(|_| adapters.getattr("hidden"))
        .transpose()?;
    let hit = q.call_method1(
        "reference_foot",
        (
            coords(start)?,
            coords(&dir)?,
            threshold_px,
            hovered_refs,
            extra_point.map(coords).transpose()?,
            hidden,
        ),
    )?;
    if !hit.is_none() {
        let (p, r, kind): (P, P, &str) = hit.extract()?;
        let color = match kind {
            "origin" => "COLOR_ORIGIN",
            "midpoint" => "COLOR_MIDPOINT",
            _ => "COLOR_ENDPOINT",
        };
        return _from_point_result(m, &point(py, p)?, &point(py, r)?, kind, &m.getattr(color)?)
            .map(Some);
    }
    if !hovered_refs || !edge_body {
        return Ok(None);
    }
    let project = project_onto_line
        .map(|_| adapters.getattr("project_line"))
        .transpose()?;
    let hidden = is_occluded
        .map(|_| adapters.getattr("hidden_all"))
        .transpose()?;
    let body = q.call_method1(
        "target",
        (threshold_px, PyList::empty(py), project, hidden, true, true),
    )?;
    if body.is_none() {
        return Ok(None);
    }
    let r = point(py, body.get_item(0)?.extract()?)?;
    let Some(foot) = guides::body_reference(
        start,
        &dir,
        &r,
        [cx, cy],
        threshold_px,
        world_to_pixel,
        is_occluded,
    )?
    else {
        return Ok(None);
    };
    _from_point_result(m, &foot, &r, "from_point", &m.getattr("COLOR_ENDPOINT")?).map(Some)
}
#[pyfunction(pass_module,signature=(cx,cy,scene,world_to_pixel,et,is_occluded,are_occluded=None,candidate_world=None))]
pub(super) fn _intersection_snap<'p>(
    m: &Module<'p>,
    cx: f64,
    cy: f64,
    scene: &Obj<'p>,
    world_to_pixel: &Obj<'p>,
    et: f64,
    is_occluded: Option<&Obj<'p>>,
    are_occluded: Option<&Obj<'p>>,
    candidate_world: Option<&Obj<'p>>,
) -> PyResult<Option<Obj<'p>>> {
    let py = m.py();
    let q = query(scene, world_to_pixel, cx, cy, None)?;
    let mut circles = Vec::new();
    if let (Some(w), Some(all)) = (candidate_world, scene.getattr_opt("circles")?) {
        let basis = py.import("built_core.axes")?.getattr("plane_axes")?;
        for c in all.try_iter()? {
            let c = c?;
            let n = c.getattr("normal")?;
            let circle = (
                coords(&c.getattr("center")?)?,
                c.getattr("radius")?.extract::<f64>()?,
                coords(&n)?,
            );
            let u = coords(&basis.call1((&n,))?.get_item(0)?)?;
            if !q
                .call_method1("rim", (circle, coords(w)?, u, et))?
                .is_none()
            {
                circles.push(circle);
            }
        }
    }
    let hits: Vec<(P, bool)> = q
        .call_method1("crossings", (et, None::<P>, None::<P>, circles))?
        .extract()?;
    let points = PyList::new(py, hits.iter().map(|(p, _)| vector(*p)))?;
    let Some(i) = first_visible(&points, is_occluded, are_occluded)? else {
        return Ok(None);
    };
    result(
        m,
        &points.get_item(i)?,
        if hits[i].1 {
            "endpoint"
        } else {
            "intersection"
        },
        "COLOR_ENDPOINT",
        None,
    )
    .map(Some)
}

#[pyfunction(pass_module,signature=(cx,cy,scene,world_to_pixel,et,is_occluded,outline,query=None))]
pub(super) fn _edge_face_snap<'p>(
    m: &Module<'p>,
    cx: f64,
    cy: f64,
    scene: &Obj<'p>,
    world_to_pixel: &Obj<'p>,
    et: f64,
    is_occluded: Option<&Obj<'p>>,
    outline: &Obj<'p>,
    query: Option<&Obj<'p>>,
) -> PyResult<Option<Obj<'p>>> {
    let patch = outline.is_instance(&m.getattr("SurfacePatch")?)?;
    let Some((p, context)) = super::super::snap_surfaces::pierce(
        outline,
        patch,
        scene,
        world_to_pixel,
        [cx, cy],
        et,
        is_occluded,
        query,
    )?
    else {
        return Ok(None);
    };
    let kw = PyDict::new(m.py());
    kw.set_item("context", context)?;
    result(m, &p, "intersection", "COLOR_ENDPOINT", Some(&kw)).map(Some)
}
#[pyfunction(pass_module,signature=(start_point,line_dir,world_to_pixel,hover_edge,hover_plane))]
pub(super) fn _lock_reference_snap<'p>(
    m: &Module<'p>,
    start_point: &Obj<'p>,
    line_dir: &Obj<'p>,
    world_to_pixel: &Obj<'p>,
    hover_edge: Option<&Obj<'p>>,
    hover_plane: Option<&Obj<'p>>,
) -> PyResult<Option<Obj<'p>>> {
    let py = m.py();
    let offer = |p: Obj<'p>, r: Obj<'p>, kind| -> PyResult<_> {
        if world_to_pixel.call1((&p,))?.is_none() {
            return Ok(None);
        }
        let kw = PyDict::new(py);
        kw.set_item("guide", (r, &p))?;
        kw.set_item("guide_color", m.getattr("COLOR_EXTENSION")?)?;
        result(m, &p, kind, "COLOR_LOCK_INTERSECT", Some(&kw)).map(Some)
    };
    if let Some(e) = hover_edge {
        if let Some((p, r)) = snap_planes::lock_on_extension(
            coords(start_point)?,
            coords(line_dir)?,
            coords(&e.getattr("a")?)?,
            coords(&e.getattr("b")?)?,
        ) {
            if let Some(hit) = offer(point(py, p)?, point(py, r)?, "intersection_edge")? {
                return Ok(Some(hit));
            }
        }
    }
    if let Some(plane) = hover_plane {
        if let Some(p) = guides::lock_plane(
            start_point,
            line_dir,
            &plane.get_item(0)?,
            &plane.get_item(1)?,
        )? {
            return offer(
                Bound::new(py, p)?.into_any(),
                plane.get_item(2)?,
                "intersection_face",
            );
        }
    }
    Ok(None)
}

#[pyfunction(pass_module,signature=(scene,start_point,line_dir,cx,cy,world_to_pixel,threshold_px,is_occluded,acquired_point,chain_first_point=None,project_onto_line=None,hover_edge=None,hover_plane=None,are_occluded=None,query=None))]
pub(super) fn _lock_line_snaps<'p>(
    m: &Module<'p>,
    scene: &Obj<'p>,
    start_point: &Obj<'p>,
    line_dir: &Obj<'p>,
    cx: f64,
    cy: f64,
    world_to_pixel: &Obj<'p>,
    threshold_px: f64,
    is_occluded: Option<&Obj<'p>>,
    acquired_point: Option<&Obj<'p>>,
    chain_first_point: Option<&Obj<'p>>,
    project_onto_line: Option<&Obj<'p>>,
    hover_edge: Option<&Obj<'p>>,
    hover_plane: Option<&Obj<'p>>,
    are_occluded: Option<&Obj<'p>>,
    query: Option<&Obj<'p>>,
) -> PyResult<Option<Obj<'p>>> {
    let py = m.py();
    if let Some(p) = chain_first_point {
        if _vertex_on_line(p, start_point, line_dir, 1e-4)? {
            let pixel = world_to_pixel.call1((p,))?.extract::<Option<[f64; 2]>>()?;
            if pixel.is_some_and(|[x, y]| (x - cx).hypot(y - cy) <= threshold_px) {
                return result(m, &point(py, coords(p)?)?, "close", "COLOR_CLOSE", None).map(Some);
            }
        }
    }
    let q = self::query(scene, world_to_pixel, cx, cy, query)?;
    let line = (coords(start_point)?, coords(line_dir)?);
    let point_result = py.import("core.snap_query")?.getattr("point_result")?;
    let resolve = |midpoints: bool| -> PyResult<Option<Obj>> {
        let kw = PyDict::new(py);
        kw.set_item("midpoints", midpoints)?;
        kw.set_item("line", line)?;
        kw.set_item("hidden_many", are_occluded)?;
        let hit = point_result.call(
            (&q, scene.getattr("edges")?, threshold_px, is_occluded),
            Some(&kw),
        )?;
        Ok((!hit.is_none()).then_some(hit))
    };
    if let Some(hit) = resolve(false)? {
        return Ok(Some(hit));
    }
    let hits: Vec<(P, bool)> = q
        .call_method1("crossings", (threshold_px, line))?
        .extract()?;
    let points = PyList::new(py, hits.into_iter().map(|(p, _)| vector(p)))?;
    if let Some(i) = first_visible(&points, is_occluded, are_occluded)? {
        let p = points.get_item(i)?;
        let kw = PyDict::new(py);
        kw.set_item("guide", (start_point, &p))?;
        return result(m, &p, "intersection", "COLOR_ENDPOINT", Some(&kw)).map(Some);
    }
    if let Some(hit) = resolve(true)? {
        return Ok(Some(hit));
    }
    let edge = _lock_reference_snap(m, start_point, line_dir, world_to_pixel, hover_edge, None)?;
    let hit = _from_point_snap(
        m,
        scene,
        Some(start_point),
        line_dir,
        cx,
        cy,
        world_to_pixel,
        threshold_px,
        is_occluded,
        acquired_point,
        10.,
        true,
        Some(line_dir),
        project_onto_line,
        edge.is_none(),
        Some(&q),
    )?;
    if hit.is_some() || edge.is_some() {
        return Ok(hit.or(edge));
    }
    _lock_reference_snap(m, start_point, line_dir, world_to_pixel, None, hover_plane)
}

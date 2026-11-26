// SPDX-License-Identifier: GPL-3.0-or-later
//! Viewport services attached to a single inference gesture. The result keeps
//! the original context, edge, point and guide objects where the tool exposes them.
use crate::{
    math::{xyz, Vec3},
    snap::P,
    snap_engine::services::FrameState,
};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};

#[pyclass(dict, module = "built_core.snap_frame")]
pub(crate) struct SnapFrame;
type F<'py> = Bound<'py, SnapFrame>;
type Obj<'py> = Bound<'py, PyAny>;
fn vector(p: P) -> Vec3 {
    DVec3::from_array(p).into()
}
fn v<'py>(py: Python<'py>, p: P) -> PyResult<Obj<'py>> {
    Ok(vector(p).into_pyobject(py)?.into_any())
}
fn axis(k: usize) -> PyResult<&'static str> {
    ["x", "y", "z"]
        .get(k)
        .copied()
        .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("axis"))
}
fn color<'py>(f: &F<'py>, k: usize) -> PyResult<Obj<'py>> {
    f.getattr("_api")?
        .getattr("AXIS_COLORS")?
        .get_item(axis(k)?)
}
fn pixel(f: &F<'_>) -> PyResult<(f64, f64)> {
    f.getattr("candidate_pixel")?.extract()
}
fn point_result<'py>(
    f: &F<'py>,
    p: &Obj<'py>,
    kind: &str,
    color: &Obj<'py>,
    kw: Option<&Bound<'py, PyDict>>,
) -> PyResult<Obj<'py>> {
    f.getattr("_api")?
        .getattr("SnapResult")?
        .call((p, kind, color), kw)
}
fn api<'py>(f: &F<'py>, k: &str) -> PyResult<Obj<'py>> {
    f.getattr("_api")?.getattr(k)
}

#[pymethods]
impl SnapFrame {
    #[new]
    fn new<'py>(options: &Bound<'py, PyDict>, query: &Obj<'py>) -> PyResult<Py<Self>> {
        let py = options.py();
        let f = Bound::new(py, Self)?;
        f.getattr("__dict__")?.call_method1("update", (options,))?;
        let api = py.import("built_core.snap")?;
        f.setattr("_api", &api)?;
        f.setattr("query", query)?;
        let threshold = f.getattr("threshold_px")?;
        let et = f.getattr("edge_threshold_px")?;
        f.setattr("et", if et.is_none() { &threshold } else { &et })?;
        f.setattr("threshold", threshold)?;
        let on = f.getattr("face_under_cursor")?;
        f.setattr(
            "on_plane",
            if on.is_truthy()? {
                Some(f.getattr("work_plane_normal")?)
            } else {
                None
            },
        )?;
        f.setattr("on_face", on)?;
        f.setattr(
            "circles",
            f.getattr("scene")?
                .getattr_opt("circles")?
                .unwrap_or_else(|| PyTuple::empty(py).into_any()),
        )?;
        let state = py.get_type::<FrameState>().call1((
            &f,
            query,
            options,
            api.call_method0("_axis_tuples")?,
            api.getattr("_MAX_AXIS_REACH")?,
        ))?;
        f.setattr("state", state)?;
        Ok(f.unbind())
    }
    fn project_line(slf: &F<'_>, start: P, direction: P) -> PyResult<Option<(f64, f64, f64)>> {
        let p = slf.call_method1("project_onto_line", (vector(start), vector(direction)))?;
        if p.is_none() {
            Ok(None)
        } else {
            Ok(Some(xyz(&p)?.to_array().into()))
        }
    }
    fn hidden(slf: &F<'_>, point: P) -> PyResult<bool> {
        let test = slf.getattr("is_occluded")?;
        Ok(!test.is_none() && test.call1((vector(point),))?.is_truthy()?)
    }
    fn hidden_batch<'py>(slf: &F<'py>, points: &Obj<'py>) -> PyResult<Obj<'py>> {
        slf.py().import("built_core")?.call_method1(
            "hidden_batch",
            (
                points,
                slf.getattr("is_occluded")?,
                slf.getattr("are_occluded")?,
            ),
        )
    }
    fn close<'py>(slf: &F<'py>) -> PyResult<Option<Obj<'py>>> {
        let p = slf.getattr("chain_first_point")?;
        let hit = slf
            .call_method1("world_to_pixel", (&p,))?
            .extract::<Option<(f64, f64)>>()?;
        let (cx, cy) = pixel(slf)?;
        let threshold = slf.getattr("threshold")?.extract::<f64>()?;
        if hit.is_some_and(|(x, y)| (x - cx).hypot(y - cy) <= threshold) {
            return Ok(Some(point_result(
                slf,
                &p,
                "close",
                &api(slf, "COLOR_CLOSE")?,
                None,
            )?));
        }
        Ok(None)
    }
    fn point<'py>(slf: &F<'py>, midpoints: bool) -> PyResult<Obj<'py>> {
        let inputs = slf
            .getattr("state")?
            .call_method1("point_inputs", (slf, midpoints))?;
        let kw = PyDict::new(slf.py());
        kw.set_item("midpoints", midpoints)?;
        kw.set_item("centers", inputs.get_item(0)?)?;
        kw.set_item("quadrants", inputs.get_item(1)?)?;
        kw.set_item("hidden_many", slf.getattr("are_occluded")?)?;
        slf.py()
            .import("built_core.snap_query")?
            .getattr("point_result")?
            .call(
                (
                    slf.getattr("query")?,
                    slf.getattr("scene")?.getattr("edges")?,
                    slf.getattr("threshold")?,
                    slf.getattr("is_occluded")?,
                ),
                Some(&kw),
            )
    }
    fn from_point<'py>(
        slf: &F<'py>,
        start: P,
        direction: P,
        line: Option<P>,
    ) -> PyResult<Obj<'py>> {
        let kw = PyDict::new(slf.py());
        kw.set_item("extra_point", slf.getattr("acquired_point")?)?;
        kw.set_item("extra_points", recent(slf)?)?;
        kw.set_item("line_dir", line.map(vector))?;
        kw.set_item("query", slf.getattr("query")?)?;
        let (x, y) = pixel(slf)?;
        api(slf, "_from_point_snap")?.call(
            (
                slf.getattr("scene")?,
                vector(start),
                vector(direction),
                x,
                y,
                slf.getattr("world_to_pixel")?,
                slf.getattr("threshold")?,
                slf.getattr("is_occluded")?,
            ),
            Some(&kw),
        )
    }
    fn perpendicular_from_start<'py>(slf: &F<'py>, start: P, linear: bool) -> PyResult<Obj<'py>> {
        slf.getattr("state")?
            .call_method1("perpendicular", (slf.getattr("query")?, slf, start, linear))
    }
    fn extension<'py>(slf: &F<'py>) -> PyResult<Obj<'py>> {
        slf.getattr("state")?
            .call_method1("extension", (slf.getattr("query")?, slf))
    }
    fn intersection<'py>(slf: &F<'py>, axis: Option<usize>) -> PyResult<Obj<'py>> {
        slf.getattr("state")?
            .call_method1("intersection", (slf.getattr("query")?, slf, axis))
    }
    fn tangent<'py>(slf: &F<'py>) -> PyResult<Obj<'py>> {
        slf.getattr("state")?.call_method1("tangent", (slf,))
    }
    #[pyo3(signature=(include_ends=false,skip_clipped=false,exclude=None))]
    fn target<'py>(
        slf: &F<'py>,
        include_ends: bool,
        skip_clipped: bool,
        exclude: Option<P>,
    ) -> PyResult<Obj<'py>> {
        slf.getattr("state")?.call_method1(
            "target",
            (
                slf.getattr("query")?,
                slf,
                include_ends,
                skip_clipped,
                exclude,
            ),
        )
    }
    fn piercing<'py>(slf: &F<'py>) -> PyResult<Option<Obj<'py>>> {
        let outline = slf.call_method0("face_outline")?;
        if outline.is_none() {
            return Ok(None);
        }
        let (x, y) = pixel(slf)?;
        Ok(Some(api(slf, "_edge_face_snap")?.call1((
            x,
            y,
            slf.getattr("scene")?,
            slf.getattr("world_to_pixel")?,
            slf.getattr("et")?,
            slf.getattr("is_occluded")?,
            outline,
            slf.getattr("query")?,
        ))?))
    }
    fn crossing_result<'py>(slf: &F<'py>, point: P, start: P) -> PyResult<Obj<'py>> {
        let p = v(slf.py(), point)?;
        let kw = PyDict::new(slf.py());
        kw.set_item("guide", (vector(start), &p))?;
        point_result(
            slf,
            &p,
            "intersection",
            &api(slf, "COLOR_ENDPOINT")?,
            Some(&kw),
        )
    }
    fn edge_reference_result<'py>(
        slf: &F<'py>,
        point: P,
        index: usize,
        axis: usize,
    ) -> PyResult<Obj<'py>> {
        let p = v(slf.py(), point)?;
        let e = slf.getattr("scene")?.getattr("edges")?.get_item(index)?;
        let kw = PyDict::new(slf.py());
        kw.set_item("guide", (slf.getattr("acquired_point")?, &p))?;
        kw.set_item("guide_color", color(slf, axis)?)?;
        kw.set_item("context", e.getattr_opt("context")?)?;
        point_result(
            slf,
            &p,
            "from_point",
            &api(slf, "COLOR_ENDPOINT")?,
            Some(&kw),
        )
    }
    fn edge_result<'py>(
        slf: &F<'py>,
        point: P,
        index: Option<usize>,
        kind: &str,
    ) -> PyResult<Obj<'py>> {
        let kw = PyDict::new(slf.py());
        if kind == "on_edge" {
            let e = slf.getattr("scene")?.getattr("edges")?.get_item(index)?;
            kw.set_item("context", e.getattr_opt("context")?)?;
            kw.set_item("edge", e)?;
        }
        point_result(
            slf,
            &v(slf.py(), point)?,
            kind,
            &api(slf, "COLOR_ON_EDGE")?,
            Some(&kw),
        )
    }
    fn face_crossing_result<'py>(slf: &F<'py>) -> PyResult<Option<Obj<'py>>> {
        let hit = slf.call_method0("face_crossing")?;
        if hit.is_none() {
            return Ok(None);
        }
        let kw = PyDict::new(slf.py());
        kw.set_item("context", hit.get_item(1)?)?;
        Ok(Some(point_result(
            slf,
            &hit.get_item(0)?,
            "intersection",
            &api(slf, "COLOR_ENDPOINT")?,
            Some(&kw),
        )?))
    }
    fn direction_result<'py>(
        slf: &F<'py>,
        kind: &str,
        start: P,
        direction: P,
        reference: Option<P>,
    ) -> PyResult<Obj<'py>> {
        let p = slf.call_method1("project_onto_line", (vector(start), vector(direction)))?;
        let kw = PyDict::new(slf.py());
        kw.set_item("guide", reference.map(|r| (vector(r), &p)))?;
        point_result(slf, &p, kind, &api(slf, "COLOR_REFERENCE")?, Some(&kw))
    }
    fn two_points<'py>(slf: &F<'py>) -> PyResult<Obj<'py>> {
        let (x, y) = pixel(slf)?;
        api(slf, "_two_point_snap")?.call1((
            slf.getattr("acquired_points")?,
            slf.getattr("candidate_world")?,
            x,
            y,
            slf.getattr("world_to_pixel")?,
            slf.getattr("threshold")?,
            slf.getattr("is_occluded")?,
        ))
    }
    /// Line up with the acquired point or a point hovered lately: the
    /// nearest such alignment to the cursor wins.
    fn encouraged<'py>(slf: &F<'py>) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let (x, y) = pixel(slf)?;
        let (w, project, radius, hidden, plane) = (
            slf.getattr("candidate_world")?,
            slf.getattr("world_to_pixel")?,
            slf.getattr("threshold")?,
            slf.getattr("is_occluded")?,
            slf.getattr("on_plane")?,
        );
        // A rectangle's corner lines up in its own plane, over a face or not.
        let plane = if plane.is_none() && slf.getattr("diagonal_span")?.is_truthy()? {
            slf.getattr("work_plane_normal")?
        } else {
            plane
        };
        let mut refs = Vec::new();
        let acquired = slf.getattr("acquired_point")?;
        if !acquired.is_none() {
            refs.push(acquired);
        }
        let recent = recent(slf)?;
        if !recent.is_none() {
            for r in recent.try_iter()? {
                let r = r?;
                // (point, kind) pairs from the engine; bare points from callers.
                let r = if r.is_instance_of::<PyTuple>() { r.get_item(0)? } else { r };
                if !refs.iter().any(|q| {
                    xyz(q).is_ok_and(|q| xyz(&r).is_ok_and(|r| q.distance(r) <= 1e-9))
                }) {
                    refs.push(r);
                }
            }
        }
        let mut best: Option<(f64, Obj<'py>)> = None;
        for refp in refs {
            let mut hit = api(slf, "_first_point_from_point")?.call1((
                &refp,
                &w,
                x,
                y,
                &project,
                &radius,
                &hidden,
                &plane,
                slf.getattr("project_onto_line")?,
                slf.getattr("free_3d")?,
            ))?;
            if hit.is_none() && !plane.is_none() {
                hit = api(slf, "_in_plane_with_point_snap")?
                    .call1((&refp, &w, &plane, x, y, &project, &radius, &hidden))?;
            }
            if hit.is_none() {
                continue;
            }
            let d = project
                .call1((hit.getattr("point")?,))?
                .extract::<Option<(f64, f64)>>()?
                .map_or(f64::INFINITY, |(u, v)| (u - x).hypot(v - y));
            if best.as_ref().is_none_or(|(b, _)| d < *b) {
                best = Some((d, hit));
            }
        }
        Ok(best.map_or_else(|| py.None().into_bound(py), |b| b.1))
    }
    fn axis_result<'py>(slf: &F<'py>, point: P, axis: usize, locked: bool) -> PyResult<Obj<'py>> {
        let kw = PyDict::new(slf.py());
        kw.set_item("axis", self::axis(axis)?)?;
        point_result(
            slf,
            &v(slf.py(), point)?,
            if locked { "axis" } else { "axis_inference" },
            &color(slf, axis)?,
            Some(&kw),
        )
    }
    fn color_lock<'py>(slf: &F<'py>, result: Obj<'py>, axis: usize) -> PyResult<Obj<'py>> {
        result.setattr("lock_color", color(slf, axis)?)?;
        Ok(result)
    }
    fn work_point<'py>(slf: &F<'py>) -> PyResult<Obj<'py>> {
        let p = slf.getattr("candidate_world")?;
        if slf.getattr("on_face")?.is_truthy()? {
            let kw = PyDict::new(slf.py());
            kw.set_item("context", slf.getattr("face_context")?)?;
            point_result(slf, &p, "on_face", &api(slf, "COLOR_ON_FACE")?, Some(&kw))
        } else {
            api(slf, "SnapResult")?.call1((p, "none"))
        }
    }
    fn locked<'py>(
        slf: &F<'py>,
        start: P,
        direction: P,
        axis: Option<usize>,
        shift: bool,
        guide_reference: Option<(P, usize)>,
        orient: bool,
    ) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let s = v(py, start)?;
        let mut d = v(py, direction)?;
        let locked = slf.call_method1("project_onto_line", (&s, &d))?;
        if orient && (xyz(&locked)? - DVec3::from_array(start)).dot(direction.into()) < 0. {
            d = v(py, direction.map(|x| -x))?;
        }
        let refp = if orient {
            slf.getattr("acquired_point")?
        } else {
            s.clone()
        };
        let pix = slf.getattr("candidate_pixel")?;
        let args = PyTuple::new(
            py,
            [
                slf.getattr("scene")?,
                s,
                d,
                pix.get_item(0)?,
                pix.get_item(1)?,
                slf.getattr("world_to_pixel")?,
                slf.getattr("threshold")?,
                slf.getattr("is_occluded")?,
                refp,
                slf.getattr("chain_first_point")?,
                slf.getattr("project_onto_line")?,
                slf.getattr("hover_lock_edge")?,
                slf.getattr("hover_lock_plane")?,
                slf.getattr("are_occluded")?,
                slf.getattr("query")?,
            ],
        )?;
        let kw = PyDict::new(py);
        kw.set_item("recent_points", recent(slf)?)?;
        let mut result = api(slf, "_lock_line_snaps")?.call(args, Some(&kw))?;
        if result.is_none()
            || ["intersection_edge", "intersection_face"]
                .iter()
                .any(|k| result.getattr("kind").is_ok_and(|v| v.eq(k).unwrap_or(false)))
        {
            if let Some(hit) = hovered_foot(slf, start, direction)? {
                result = hit;
            }
        }
        let shift_color = slf.getattr("shift_lock_color")?;
        let color = match axis {
            Some(k) => color(slf, k)?,
            None if shift && !shift_color.is_none() => shift_color,
            None => api(slf, "COLOR_REFERENCE")?,
        };
        if result.is_none() {
            let kw = PyDict::new(py);
            kw.set_item("axis", axis.map(self::axis).transpose()?)?;
            result = point_result(
                slf,
                &locked,
                if axis.is_some() { "axis" } else { "reference" },
                &color,
                Some(&kw),
            )?;
        } else {
            result.setattr("lock_color", color)?;
        }
        if let Some((p, k)) = guide_reference {
            if result.getattr("guide")?.is_none() {
                let p = v(py, p)?;
                let foot = slf.call_method1(
                    "project_onto_line",
                    (&p, api(slf, "_AXIS_VECTORS")?.get_item(self::axis(k)?)?),
                )?;
                result.setattr("guide", (p, foot))?;
                result.setattr("guide_color", self::color(slf, k)?)?;
            }
        }
        Ok(result)
    }
}
/// The points hovered lately (`InferenceEngine._recent_references`: points
/// or (point, kind) pairs), if handed in, while an operation has a start
/// point, less that start: as with the acquired point, the start is no
/// reference for its own line. Before the first click they are no
/// references at all (the acquired point alone lines up a first point).
fn recent<'py>(f: &F<'py>) -> PyResult<Obj<'py>> {
    let py = f.py();
    let start = f.getattr_opt("start_point")?.filter(|s| !s.is_none());
    let (Some(all), Some(start)) = (
        f.getattr_opt("recent_points")?.filter(|r| !r.is_none()),
        start.map(|s| xyz(&s)).transpose()?,
    ) else {
        return Ok(py.None().into_bound(py));
    };
    let kept = PyList::empty(py);
    for r in all.try_iter()? {
        let r = r?;
        let p = if r.is_instance_of::<PyTuple>() { r.get_item(0)? } else { r.clone() };
        if xyz(&p)?.distance(start) >= 1e-6 {
            kept.append(r)?;
        }
    }
    Ok(kept.into_any())
}
/// Under a lock, any inference point the cursor is on (a centre, a face
/// centre, a quadrant, an intersection; endpoints and midpoints come from
/// `_lock_line_snaps`) gives the locked line the foot of its perpendicular
/// plane: "from point".
fn hovered_foot<'py>(f: &F<'py>, start: P, direction: P) -> PyResult<Option<Obj<'py>>> {
    let u = DVec3::from_array(direction).normalize_or_zero();
    if u == DVec3::ZERO {
        return Ok(None);
    }
    let s = DVec3::from_array(start);
    for step in 0..3 {
        let hit = match step {
            0 => f.call_method1("point", (false,))?,
            1 => f.call_method1("intersection", (None::<usize>,))?,
            _ => f.call_method1("point", (true,))?,
        };
        if hit.is_none() || !hit.getattr("guide")?.is_none() {
            continue;
        }
        let p = xyz(&hit.getattr("point")?)?;
        let foot = s + u * (p - s).dot(u);
        if foot.distance(p) < 1e-4 {
            return Ok(Some(hit));
        }
        if foot.distance(s) < 1e-6 {
            continue; // level with the start: no line to draw
        }
        let r = api(f, "_from_point_result")?.call1((
            vector(foot.to_array()),
            vector(p.to_array()),
            "from_point",
            api(f, "COLOR_ENDPOINT")?,
            vector(direction),
        ))?;
        r.setattr("source_kind", hit.getattr("kind")?)?;
        r.setattr("context", hit.getattr("context")?)?;
        return Ok(Some(r));
    }
    Ok(None)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "built_core.snap_frame")?;
    sub.add_class::<SnapFrame>()?;
    m.add_submodule(&sub)
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed state for one pointer event, plus decisions requiring a viewport callback.
use super::Gesture;
use crate::{
    math::{xyz, Vec3},
    snap::{self, SnapQuery, P},
    snap_curve::Circle,
};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
type Obj<'py> = Bound<'py, PyAny>;
fn coords(p: &Obj<'_>) -> PyResult<P> {
    Ok(xyz(p)?.to_array())
}
fn some(p: &Obj<'_>) -> PyResult<Option<P>> {
    if p.is_none() {
        Ok(None)
    } else {
        coords(p).map(Some)
    }
}
fn vec(p: P) -> Vec3 {
    DVec3::from_array(p).into()
}
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn ends(e: &Obj<'_>) -> PyResult<(P, P)> {
    Ok((coords(&e.getattr("a")?)?, coords(&e.getattr("b")?)?))
}
fn axis(v: &Obj<'_>) -> PyResult<Option<usize>> {
    if !v.is_truthy()? {
        return Ok(None);
    }
    let k: String = v.extract()?;
    "xyz"
        .find(&k)
        .map(Some)
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("substring not found"))
}
fn result<'py>(
    frame: &Obj<'py>,
    p: P,
    kind: &str,
    color: &str,
    guide: Option<(P, P)>,
) -> PyResult<Obj<'py>> {
    let api = frame.getattr("_api")?;
    let args = (vec(p), kind, api.getattr(color)?);
    if let Some((a, b)) = guide {
        let kw = PyDict::new(frame.py());
        kw.set_item("guide", (vec(a), vec(b)))?;
        api.getattr("SnapResult")?.call(args, Some(&kw))
    } else {
        api.call_method1("SnapResult", args)
    }
}
fn circle(c: &Obj<'_>) -> PyResult<Circle> {
    Ok((
        coords(&c.getattr("center")?)?,
        c.getattr("radius")?.extract()?,
        coords(&c.getattr("normal")?)?,
    ))
}
#[pyclass(frozen)]
pub(crate) struct FrameState {
    pub(super) gesture: Gesture,
    rim_circles: Vec<Circle>,
    rim_hits: Vec<(f64, P)>,
}
#[pymethods]
impl FrameState {
    #[new]
    fn new(
        frame: &Obj<'_>,
        q: &SnapQuery,
        opts: &Bound<'_, PyDict>,
        axes: [P; 3],
        reach: f64,
    ) -> PyResult<Self> {
        let py = opts.py();
        let get = |k: &str| opts.as_any().get_item(k);
        let yes = |k: &str| get(k)?.is_truthy();
        let world = coords(&get("candidate_world")?)?;
        let start = some(&get("start_point")?)?;
        let normal = some(&get("work_plane_normal")?)?;
        let mut acquired = some(&get("acquired_point")?)?;
        if start
            .zip(acquired)
            .is_some_and(|(s, a)| DVec3::from(s).distance_squared(a.into()) < 1e-12)
        {
            acquired = None;
            frame.setattr("acquired_point", py.None())?;
        }
        let diagonal = yes("diagonal_span")?;
        let refs = get("acquired_points")?;
        let has_refs = if let Some(s) = start.filter(|_| diagonal && !refs.is_none()) {
            if refs.is_truthy()? {
                let kept = PyList::empty(py);
                for r in refs.try_iter()? {
                    let r = r?;
                    if !r.is_none() && xyz(&r)?.distance_squared(s.into()) >= 1e-12 {
                        kept.append(r)?;
                    }
                }
                frame.setattr("acquired_points", &kept)?;
                !kept.is_empty()
            } else {
                false
            }
        } else {
            refs.is_truthy()?
        };
        let shift = get("shift_lock_ref")?;
        let shift_reference = if shift.is_truthy()? {
            let k = shift.get_item(1)?;
            if frame
                .getattr("_api")?
                .getattr("AXIS_COLORS")?
                .contains(&k)?
            {
                Some((coords(&shift.get_item(0)?)?, axis(&k)?.unwrap()))
            } else {
                None
            }
        } else {
            None
        };
        let edge = get("reference_edge")?;
        let mode = get("reference_mode")?;
        let reference = if !edge.is_none() && mode.is_truthy()? {
            let (a, b) = ends(&edge)?;
            Some((a, b, mode.eq("perpendicular")?))
        } else {
            None
        };
        let e = get("acquired_edge")?;
        let acquired_edge_points = if e.is_none() { None } else { Some(ends(&e)?) };
        let first = get("chain_first_point")?;
        let has_close = start.is_some() && !first.is_none() && !first.is(get("start_point")?);
        let threshold = get("threshold_px")?.extract()?;
        let et = get("edge_threshold_px")?
            .extract::<Option<f64>>()?
            .unwrap_or(threshold);
        let circles = frame.getattr("circles")?;
        let has_circles = circles.is_truthy()?;
        let mut rim_circles = Vec::new();
        let mut rim_hits = Vec::new();
        if has_circles {
            let basis = py.import("ingetrazo_core.axes")?.getattr("plane_axes")?;
            for c in circles.try_iter()? {
                let c = c?;
                let data = circle(&c)?;
                let u = coords(&basis.call1((c.getattr("normal")?,))?.get_item(0)?)?;
                if let Some(hit) = q.rim(data, world, u, et)? {
                    rim_circles.push(data);
                    rim_hits.push(hit);
                }
            }
        }
        let mode = get("linear_mode")?;
        Ok(Self {
            rim_circles,
            rim_hits,
            gesture: Gesture {
                world,
                start,
                axes,
                normal,
                acquired,
                shift_reference,
                reference,
                acquired_edge_points,
                diagonal,
                threshold,
                et,
                has_circles,
                has_refs,
                has_close,
                shift_direction: some(&get("shift_lock_dir")?)?,
                acquired_normal: some(&get("acquired_face_normal")?)?,
                arrow: axis(&get("axis_lock")?)?,
                sticky: axis(&get("sticky_axis")?)?,
                angle: get("inference_angle_deg")?.extract()?,
                magnet: get("magnetic_axis_deg")?.extract()?,
                screen: get("screen_axis_px")?.extract()?,
                max_axis_reach: reach,
                planar: yes("axis_in_plane")?,
                radial: yes("radial_arm")?,
                shift: yes("shift_held")?,
                dwell: yes("dwell_axis_lock")?,
                projectable: !get("project_onto_line")?.is_none(),
                on_face: yes("face_under_cursor")?,
                linear: mode.eq("all")?,
                parperp: !mode.eq("off")?,
                has_outline: !get("face_outline")?.is_none(),
                has_crossing: !get("face_crossing")?.is_none(),
            },
        })
    }
    fn extension<'py>(&self, q: &SnapQuery, frame: &Obj<'py>) -> PyResult<Obj<'py>> {
        let g = &self.gesture;
        let py = frame.py();
        let Some(start) = g.start else {
            return Ok(none(py));
        };
        let project = if g.projectable {
            Some(frame.getattr("project_line")?)
        } else {
            None
        };
        let hidden = frame.getattr("hidden")?;
        for (p, end, d) in q.extensions(
            py,
            start,
            g.world,
            g.et,
            project.as_ref(),
            g.normal.filter(|_| g.on_face),
        )? {
            if hidden.call1((p,))?.is_truthy()? {
                continue;
            }
            for (hit, _) in q.crossings(py, g.et, Some((p, d)), None, None)? {
                if !hidden.call1((hit,))?.is_truthy()? {
                    return if DVec3::from(hit).distance_squared(end.into()) < 1e-12 {
                        result(frame, hit, "endpoint", "COLOR_ENDPOINT", None)
                    } else {
                        result(
                            frame,
                            hit,
                            "intersection",
                            "COLOR_ENDPOINT",
                            Some((end, hit)),
                        )
                    };
                }
            }
            return result(frame, p, "extension", "COLOR_EXTENSION", Some((end, p)));
        }
        Ok(none(py))
    }
    fn intersection<'py>(
        &self,
        q: &SnapQuery,
        frame: &Obj<'py>,
        axis: Option<usize>,
    ) -> PyResult<Obj<'py>> {
        let py = frame.py();
        let g = &self.gesture;
        let hits = q.crossings(py, g.et, None, None, Some(self.rim_circles.clone()))?;
        let points = PyList::new(py, hits.iter().map(|h| vec(h.0)))?;
        let one = frame.getattr("is_occluded")?;
        let batch = frame.getattr("are_occluded")?;
        if let Some(i) = crate::snap_visibility::first_visible(
            points.as_any(),
            (!one.is_none()).then_some(&one),
            (!batch.is_none()).then_some(&batch),
        )? {
            let (p, end) = hits.get(i).copied().ok_or_else(|| {
                pyo3::exceptions::PyIndexError::new_err("invalid visibility response")
            })?;
            if let (Some(k), Some(s)) = (axis, g.start) {
                let a = DVec3::from(g.axes[k]);
                let d = DVec3::from(p) - DVec3::from(s);
                if (d - a * d.dot(a)).length() >= 1e-4 {
                    return Ok(none(py));
                }
            }
            return result(
                frame,
                p,
                if end { "endpoint" } else { "intersection" },
                "COLOR_ENDPOINT",
                None,
            );
        }
        Ok(none(py))
    }
    #[pyo3(signature=(q,frame,include_ends=false,skip_clipped=false,exclude=None))]
    fn target(
        &self,
        q: &SnapQuery,
        frame: &Obj<'_>,
        include_ends: bool,
        skip_clipped: bool,
        exclude: Option<P>,
    ) -> PyResult<Option<(P, Option<usize>, &'static str)>> {
        let g = &self.gesture;
        let project = if g.projectable {
            Some(frame.getattr("project_line")?)
        } else {
            None
        };
        let hidden = if frame.getattr("is_occluded")?.is_none() {
            None
        } else {
            Some(frame.getattr("hidden_batch")?)
        };
        q.target(
            frame.py(),
            g.et,
            if include_ends {
                Vec::new()
            } else {
                self.rim_hits.clone()
            },
            project.as_ref(),
            hidden.as_ref(),
            include_ends,
            skip_clipped,
            exclude,
            !frame.getattr("are_occluded")?.is_none(),
        )
    }
    fn perpendicular<'py>(
        &self,
        q: &SnapQuery,
        frame: &Obj<'py>,
        start: P,
        linear: bool,
    ) -> PyResult<Obj<'py>> {
        let py = frame.py();
        let g = &self.gesture;
        let draw = snap::sub(g.world, start);
        if snap::norm(draw) <= 1e-6 {
            return Ok(none(py));
        }
        let scene = frame.getattr("scene")?.getattr("edges")?;
        let edges = PyList::empty(py);
        for i in q.starting_edges(py, start) {
            edges.append(scene.get_item(i)?)?;
        }
        let extra = frame.getattr("start_edges")?;
        if extra.is_truthy()? {
            edges.call_method1("extend", (extra,))?;
        }
        for edge in edges.iter() {
            let (a, b) = ends(&edge)?;
            if !snap::contains([a, b], start) {
                continue;
            }
            let Some(d) = snap::edge_direction(a, b, true, g.normal.unwrap_or(g.axes[2])) else {
                continue;
            };
            if snap::dot(snap::unit(draw), d).abs() < 8f64.to_radians().cos() {
                continue;
            }
            for (p, _) in q.crossings(py, g.et, Some((start, d)), Some(draw), None)? {
                if snap::dot(snap::sub(p, start), draw) > 0.
                    && !frame.call_method1("hidden", (p,))?.is_truthy()?
                {
                    return result(frame, p, "intersection", "COLOR_ENDPOINT", Some((start, p)));
                }
            }
            if linear {
                let hit = frame.call_method1("from_point", (start, draw, py.None()))?;
                if !hit.is_none() {
                    return Ok(hit);
                }
            }
            if self.target(q, frame, true, false, Some(start))?.is_some() {
                return Ok(none(py));
            }
            let foot = frame.call_method1("project_onto_line", (vec(start), vec(d)))?;
            let api = frame.getattr("_api")?;
            return api.call_method1(
                "SnapResult",
                (foot, "reference", api.getattr("COLOR_REFERENCE")?),
            );
        }
        Ok(none(py))
    }
    fn point_inputs<'py>(
        &self,
        frame: &Obj<'py>,
        midpoints: bool,
    ) -> PyResult<(Vec<Obj<'py>>, Vec<Vec3>)> {
        let mut centers = Vec::new();
        let mut quads = Vec::new();
        if !midpoints {
            let faces = frame.getattr("face_centers")?;
            if faces.is_truthy()? {
                for c in faces.try_iter()? {
                    let c = c?;
                    if self
                        .gesture
                        .start
                        .map(|s| xyz(&c.get_item(0)?).map(|p| p.distance(s.into()) >= 1e-6))
                        .transpose()?
                        .unwrap_or(true)
                    {
                        centers.push(c);
                    }
                }
            }
            for c in frame.getattr("circles")?.try_iter()? {
                let c = c?;
                quads.extend(crate::snap_guides::quadrants(
                    &c.getattr("center")?,
                    c.getattr("radius")?.extract()?,
                    &c.getattr("normal")?,
                )?);
            }
        }
        Ok((centers, quads))
    }
    fn tangent<'py>(&self, frame: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = frame.py();
        let g = &self.gesture;
        let Some(s) = g.start.filter(|_| !g.diagonal) else {
            return Ok(none(py));
        };
        let api = frame.getattr("_api")?;
        let color = api.getattr("COLOR_TANGENT")?;
        let pending = PyList::empty(py);
        let project = frame.getattr("world_to_pixel")?;
        let [x, y]: [f64; 2] = frame.getattr("candidate_pixel")?.extract()?;
        for c in frame.getattr("circles")?.try_iter()? {
            for p in crate::snap_curve::circle_tangents(circle(&c?)?, s) {
                let p = vec(p).into_pyobject(py)?;
                let px = project.call1((&p,))?;
                if let Some([u, v]) = px.extract::<Option<[f64; 2]>>()? {
                    let d = crate::sheets::pymath::hypot(u - x, v - y);
                    if d <= g.threshold {
                        pending.append((d, p, "tangent", &color, py.None(), true))?;
                    }
                }
            }
        }
        let one = frame.getattr("is_occluded")?;
        let many = frame.getattr("are_occluded")?;
        if let Some(win) = crate::snap_visibility::resolve_points(
            pending.as_any(),
            (!one.is_none()).then_some(&one),
            (!many.is_none()).then_some(&many),
        )? {
            return api.call_method1("SnapResult", (win.get_item(1)?, "tangent", color));
        }
        Ok(none(py))
    }
}

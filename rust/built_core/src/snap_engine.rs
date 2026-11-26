//! The rule order for a cursor event. The frame supplies viewport-dependent
//! operations (picking, projecting on a lock, visibility and Qt tool results).
use crate::snap::{along, dot, drawing_axis, edge_direction, norm, sub, unit, SnapQuery, P};
use pyo3::prelude::*;

#[path="snap_frame.rs"]
pub(crate) mod services;

#[derive(FromPyObject)]
pub struct Gesture {
    world: P,
    start: Option<P>,
    axes: [P; 3],
    normal: Option<P>,
    acquired: Option<P>,
    shift_direction: Option<P>,
    shift_reference: Option<(P, usize)>,
    arrow: Option<usize>,
    sticky: Option<usize>,
    reference: Option<(P, P, bool)>,
    acquired_edge_points: Option<(P, P)>,
    acquired_normal: Option<P>,
    angle: f64,
    magnet: Option<f64>,
    screen: Option<f64>,
    #[pyo3(default = 5.0)]
    max_axis_reach: f64,
    threshold: f64,
    et: f64,
    diagonal: bool,
    planar: bool,
    radial: bool,
    shift: bool,
    dwell: bool,
    projectable: bool,
    on_face: bool,
    linear: bool,
    parperp: bool,
    has_close: bool,
    has_refs: bool,
    has_circles: bool,
    has_outline: bool,
    has_crossing: bool,
    /// Points hovered lately (`recent_points`) are on hand.
    #[pyo3(default)]
    has_recent: bool,
}

impl Gesture {
    fn in_plane(&self, k: usize) -> bool {
        !self.planar
            || self
                .normal
                .filter(|&n| norm(n) > 1e-9)
                .is_none_or(|n| dot(self.axes[k], unit(n)).abs() <= 1e-3)
    }
    fn axis(&self, delta: P, angle: f64) -> Option<usize> {
        drawing_axis(delta, self.axes, angle.to_radians().cos(), 1e-6, false).map(|h| h.0)
    }
    fn reference_dir(&self, a: P, b: P, perp: bool) -> Option<P> {
        edge_direction(a, b, perp, self.normal.unwrap_or(self.axes[2]))
    }
    fn engaged(&self, q: &SnapQuery, frame: &Bound<'_, PyAny>, s: P) -> PyResult<Option<usize>> {
        let d = sub(self.world, s);
        let reach = norm(d);
        let mut axis = None;
        if self.screen.is_some() && reach >= 1e-6 {
            if let (Some(radius), Some((x, y))) = (self.screen, q.pixel(s)?) {
                let mut samples = [None; 3];
                for (slot, a) in samples.iter_mut().zip(self.axes) {
                    *slot = q.pixel(along(s, a, reach))?.map(|(x, y)| [x, y]);
                }
                if let Some(k) = crate::snap::screen_axis([x, y], samples, q.cursor, radius)
                    .filter(|&k| self.in_plane(k))
                {
                    let locked: P = frame
                        .call_method1("project_line", (s, self.axes[k]))?
                        .extract()?;
                    if norm(sub(locked, s)) <= self.max_axis_reach * reach {
                        axis = Some(k);
                    }
                }
            }
        }
        axis = axis.or_else(|| {
            self.magnet
                .and_then(|deg| self.axis(d, deg))
                .filter(|&k| self.in_plane(k))
        });
        axis = axis.or_else(|| {
            self.sticky.filter(|&k| {
                reach < 1e-6 || dot(unit(d), self.axes[k]).abs() >= 20f64.to_radians().cos()
            })
        });
        if let Some(k) = axis {
            let locked: P = frame
                .call_method1("project_line", (s, self.axes[k]))?
                .extract()?;
            let n = self.normal.filter(|&n| norm(n) > 1e-9).map(unit);
            if n.is_none_or(|n| dot(sub(locked, self.world), n).abs() > 1e-3) {
                if self.on_face
                    && n.is_some_and(|n| {
                        let rise = dot(self.axes[k], n).abs();
                        dot(sub(s, self.world), n).abs() <= 1e-3 && rise > 0.0087 && rise < 0.7071
                    })
                    || frame.call_method1("hidden", (locked,))?.is_truthy()?
                {
                    axis = None;
                }
            }
        }
        Ok(axis)
    }
}

#[pyfunction]
#[pyo3(signature = (q, frame, state=None))]
pub fn resolve_snap<'py>(
    q: &SnapQuery,
    frame: &Bound<'py, PyAny>,
    state: Option<&services::FrameState>,
) -> PyResult<Bound<'py, PyAny>> {
    let stored;
    let fallback;
    let g = match state {
        Some(state) => &state.gesture,
        None if frame.is_instance_of::<crate::kernel::SnapFrame>() => {
            stored = frame.getattr("state")?.cast_into::<services::FrameState>()?;
            &stored.get().gesture
        }
        None => {
            fallback = frame.extract()?;
            &fallback
        }
    };
    if [g.arrow, g.sticky, g.shift_reference.map(|(_, k)| k)]
        .into_iter()
        .flatten()
        .any(|k| k >= 3)
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "axis index must be 0, 1 or 2",
        ));
    }
    let line = g.start.filter(|_| !g.diagonal);
    let draw = line.map(|s| sub(g.world, s));
    let linear = g.linear && !g.radial;
    let mut axis = if let Some(s) = line.filter(|_| g.linear && g.projectable) {
        g.engaged(q, frame, s)?
    } else {
        None
    };
    // Explicit locks fetch only points on their own line. Keep the colour
    // and guide of the lock when a discrete snap wins.
    if g.projectable {
        if let Some(k) = g.arrow {
            if let Some(s) = g.start.or(g.acquired.filter(|_| g.dwell)) {
                return frame.call_method1(
                    "locked",
                    (
                        s,
                        g.axes[k],
                        Some(k),
                        false,
                        g.acquired.map(|p| (p, k)),
                        g.start.is_some(),
                    ),
                );
            }
        }
        if let Some(d) = g.shift_direction {
            if let Some(s) = g.start {
                return frame.call_method1(
                    "locked",
                    (s, d, None::<usize>, true, g.shift_reference, true),
                );
            }
            if let Some((s, k)) = g.shift_reference {
                return frame.call_method1(
                    "locked",
                    (s, g.axes[k], None::<usize>, true, g.shift_reference, false),
                );
            }
        }
        if let (Some((a, b, perp)), Some(s)) = (g.reference, g.start) {
            if let Some(d) = g.reference_dir(a, b, perp) {
                return frame.call_method1(
                    "locked",
                    (s, d, None::<usize>, false, None::<(P, usize)>, true),
                );
            }
        }
        if let Some(s) = line.filter(|_| g.linear && g.shift) {
            if let Some(k) = g.axis(sub(g.world, s), g.angle).filter(|&k| g.in_plane(k)) {
                return frame.call_method1(
                    "locked",
                    (s, g.axes[k], Some(k), false, None::<(P, usize)>, true),
                );
            }
        }
    }

    macro_rules! offer {($method:literal $(,$arg:expr)* )=>{
        {let r=frame.call_method1($method,($($arg,)*))?; if !r.is_none(){return Ok(r);}}
    }}
    if g.has_close {
        offer!("close");
    }
    offer!("point", false);
    if let Some(s) = line {
        if g.parperp && axis.is_none() && g.projectable {
            offer!("perpendicular_from_start", s, linear);
        }
        if linear {
            if let Some(p) = g.acquired {
                let d = sub(p, s);
                let draw = sub(g.world, s);
                if q.distance(p)? > g.threshold && g.projectable && aligned(draw, d, g.angle) {
                    return frame
                        .call_method1("direction_result", ("through_point", s, unit(d), Some(p)));
                }
            }
            if axis.is_none() {
                offer!("extension");
                offer!("from_point", s, sub(g.world, s), None::<P>);
            }
        }
    }
    offer!("intersection", axis);
    if g.on_face && axis.is_none() && g.has_outline {
        offer!("piercing");
    }
    offer!("point", true);
    if line.is_some() && g.has_circles {
        offer!("tangent");
    }
    if let Some(target) = frame
        .call_method0("target")?
        .extract::<Option<(P, Option<usize>, String)>>()?
    {
        let (point, index, kind) = target;
        let edge = index.map(|i| q.edge(i)).transpose()?;
        if let (Some(s), Some(k), Some([a, b])) = (line, axis, edge) {
            let dir = g.axes[k];
            if let Some(p) = crate::snap::lock_crossing(s, dir, a, b, 1e-3) {
                if q.distance(p)? <= g.et && !frame.call_method1("hidden", (p,))?.is_truthy()? {
                    return frame.call_method1("crossing_result", (p, s));
                }
            } else if kind == "on_edge"
                && g.normal.filter(|&n| norm(n) > 1e-9).is_some_and(|n| {
                    let n = unit(n);
                    let rise = dot(n, dir).abs();
                    rise > 0.0087
                        && rise < 0.7071
                        && [a, b].into_iter().all(|p| dot(n, sub(p, s)).abs() <= 1e-3)
                })
            {
                axis = None;
            }
        }
        if axis.is_none() {
            if let (Some(ref_pt), Some([a, b])) =
                (g.acquired.filter(|_| kind == "on_edge" && linear), edge)
            {
                let mut hits = Vec::new();
                for (k, dir) in g.axes.iter().enumerate() {
                    if let Some(hit) = crate::snap::lock_crossing(ref_pt, *dir, a, b, 1e-3) {
                        let distance = q.distance(hit)?;
                        if distance <= g.threshold && norm(sub(hit, ref_pt)) >= 1e-4 {
                            hits.push((distance, hit, k));
                        }
                    }
                }
                hits.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (_, hit, k) in hits {
                    if !frame.call_method1("hidden", (hit,))?.is_truthy()? {
                        return frame.call_method1("edge_reference_result", (hit, index, k));
                    }
                }
            }
            return frame.call_method1("edge_result", (point, index, kind));
        }
    }
    if g.on_face && axis.is_none() && g.has_crossing {
        offer!("face_crossing_result");
    }
    if let (Some(s), Some(draw)) = (line, draw) {
        if axis.is_none() && g.parperp && g.projectable {
            if let Some((a, b)) = g.acquired_edge_points {
                for perp in [false, true] {
                    if let Some(d) = g.reference_dir(a, b, perp) {
                        if g.axis(d, g.angle).is_none() && aligned(draw, d, g.angle) {
                            return frame
                                .call_method1("direction_result", ("reference", s, d, None::<P>));
                        }
                    }
                }
            }
            if let Some(n) = g.acquired_normal.filter(|&n| aligned(draw, n, g.angle)) {
                return frame
                    .call_method1("direction_result", ("perp_face", s, unit(n), None::<P>));
            }
        }
    }
    if linear && axis.is_none() {
        if g.has_refs {
            offer!("two_points");
        }
        if g.acquired.is_some() || g.has_recent {
            offer!("encouraged");
        }
    }
    if let (Some(s), Some(k)) = (line, axis) {
        let locked: P = frame
            .call_method1("project_line", (s, g.axes[k]))?
            .extract()?;
        let run = sub(locked, s);
        if linear && norm(run) > 1e-6 {
            let r = frame.call_method1("from_point", (s, run, Some(unit(run))))?;
            if !r.is_none() {
                return frame.call_method1("color_lock", (r, k));
            }
        }
        return frame.call_method1("axis_result", (locked, k, true));
    }
    if let Some(k) = draw
        .filter(|_| g.linear)
        .and_then(|d| g.axis(d, g.angle))
        .filter(|&k| g.in_plane(k))
    {
        return frame.call_method1("axis_result", (g.world, k, false));
    }
    frame.call_method0("work_point")
}

fn aligned(draw: P, d: P, angle: f64) -> bool {
    norm(draw) >= 1e-6
        && norm(d) >= 1e-6
        && dot(unit(draw), unit(d)).abs() >= angle.to_radians().cos()
}

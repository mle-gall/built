//! Cursor-local snap search. Coordinates and matrix share the tool frame.
//! Source ordinals preserve placement identity and stable ties across the FFI.
use crate::facegeom::Hypot;
use crate::instances::project;
use crate::store::drawing::intersection;
use numpy::{PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::prelude::*;

pub(crate) type P = [f64; 3];
type Line = (P, P);
pub(crate) fn sub(a: P, b: P) -> P {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(crate) fn along(a: P, d: P, t: f64) -> P {
    std::array::from_fn(|i| a[i] + t * d[i])
}
pub(crate) fn dot(a: P, b: P) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
pub(crate) fn norm(a: P) -> f64 {
    dot(a, a).sqrt()
}

#[pyfunction]
pub fn lock_crossing(p: P, u: P, a: P, b: P, tol: f64) -> Option<P> {
    let v = sub(b, a);
    let (lu, lv) = (norm(u), norm(v));
    if lu < 1e-9 || lv < 1e-9 {
        return None;
    }
    let (u, v) = (u.map(|x| x / lu), v.map(|x| x / lv));
    let w = sub(p, a);
    let (aa, bb, cc, dd, ee) = (dot(u, u), dot(u, v), dot(v, v), dot(u, w), dot(v, w));
    let denom = aa * cc - bb * bb;
    if denom < 1e-12 {
        return None;
    }
    let t = (aa * ee - bb * dd) / denom;
    if t < -1e-6 * lv || t > (1. + 1e-6) * lv {
        return None;
    }
    let hit = along(a, v, t);
    (norm(sub(hit, along(p, u, (bb * ee - cc * dd) / denom))) <= tol).then_some(hit)
}

/// flag 0 ordinary, 1 guide, 2 center, 3 component origin, 4 arc midpoint.
#[pyclass(frozen)]
pub struct SnapQuery {
    edges: Vec<[P; 2]>,
    tags: Vec<u8>,
    grouped: Vec<bool>,
    matrix: [f64; 16],
    size: [f64; 2],
    pub(crate) cursor: [f64; 2],
}

type Choice = (f64, P, i64, &'static str, bool);

/// Ask the viewport only for the front candidate, then in growing batches.
fn first_clear(
    found: &[Choice],
    order: &[usize],
    hidden: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<usize>> {
    let Some(test) = hidden else {
        return Ok(order.first().copied());
    };
    let mut offset = 0;
    let mut count = 1;
    while offset < order.len() {
        let batch = &order[offset..(offset + count).min(order.len())];
        let points = batch.iter().map(|&i| found[i].1).collect::<Vec<_>>();
        let mask: Vec<bool> = test.call1((points,))?.extract()?;
        if mask.len() != batch.len() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "visibility batch length",
            ));
        }
        if let Some(i) = mask.iter().position(|&hidden| !hidden) {
            return Ok(Some(batch[i]));
        }
        offset += batch.len();
        count = 8.max(count * 2);
    }
    Ok(None)
}

impl SnapQuery {
    pub(crate) fn edge(&self, i: usize) -> PyResult<[P; 2]> {
        self.edges
            .get(i)
            .copied()
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("snap edge ordinal"))
    }

    pub(crate) fn pixel(&self, p: P) -> Option<(f64, f64)> {
        project(&self.matrix, p, self.size[0], self.size[1])
    }
    pub(crate) fn distance(&self, p: P) -> f64 {
        self.pixel(p).map_or(f64::INFINITY, |(x, y)| {
            (x - self.cursor[0]).hypot(y - self.cursor[1])
        })
    }
}

#[pymethods]
impl SnapQuery {
    #[new]
    fn new(
        a: PyReadonlyArray2<'_, f64>,
        b: PyReadonlyArray2<'_, f64>,
        tags: Vec<u8>,
        grouped: Vec<bool>,
        matrix: [f64; 16],
        size: [f64; 2],
        cursor: [f64; 2],
    ) -> PyResult<Self> {
        if a.shape() != [tags.len(), 3] || a.shape() != b.shape() || grouped.len() != tags.len() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "snap edge array shape",
            ));
        }
        let edges = a
            .as_slice()?
            .chunks_exact(3)
            .zip(b.as_slice()?.chunks_exact(3))
            .map(|(a, b)| [a.try_into().unwrap(), b.try_into().unwrap()])
            .collect();
        Ok(Self {
            edges,
            tags,
            grouped,
            matrix,
            size,
            cursor,
        })
    }

    // Return the nearest and preferred tied point. The nearest must be visible
    // before it can anchor the half-pixel window; retry only on actual occlusion.
    #[pyo3(signature=(radius,midpoints,quadrants,centers,hidden=None,line=None))]
    fn points(
        &self,
        py: Python<'_>,
        radius: f64,
        midpoints: bool,
        quadrants: Vec<P>,
        centers: Vec<(P, bool)>,
        hidden: Option<&Bound<'_, PyAny>>,
        line: Option<Line>,
    ) -> PyResult<Option<(P, i64, &'static str)>> {
        let found = py.detach(|| {
            let mut found: Vec<Choice> = Vec::new();
            let mut offer = |p: P, id: i64, kind, grouped| {
                if let Some((s, u)) = line {
                    let d = sub(p, s);
                    if norm(sub(d, u.map(|v| v * dot(d, u)))) >= 1e-4 {
                        return;
                    }
                }
                let d = self.distance(p);
                if d <= radius {
                    found.push((d, p, id, kind, grouped));
                }
            };
            if !midpoints {
                for (i, &tag) in self.tags.iter().enumerate() {
                    let kind = match tag & 7 {
                        2 => "center",
                        3 => "component_origin",
                        4 => "arc_midpoint",
                        _ => continue,
                    };
                    offer(self.edges[i][0], i as i64, kind, self.grouped[i]);
                }
                for q in quadrants {
                    offer(q, -1, "quadrant", false);
                }
                offer([0.; 3], -1, "origin", false);
            }
            for (i, &[a, b]) in self.edges.iter().enumerate() {
                if self.tags[i] & 7 == 1 {
                    continue;
                }
                if midpoints {
                    if norm(sub(b, a)) >= 1e-9 {
                        offer(
                            along(a, sub(b, a), 0.5),
                            i as i64,
                            "midpoint",
                            self.grouped[i],
                        );
                    }
                } else if self.tags[i] & 7 == 0 {
                    for p in [a, b] {
                        offer(p, i as i64, "endpoint", self.grouped[i]);
                    }
                }
            }
            if midpoints && line.is_none() {
                offer([0.; 3], -1, "origin", false);
            }
            for (i, (p, g)) in centers.into_iter().enumerate() {
                offer(p, -2 - i as i64, "face_center", g);
            }
            found
        });
        if found.is_empty() {
            return Ok(None);
        }
        let mut ordered = (0..found.len()).collect::<Vec<_>>();
        ordered.sort_by(|&a, &b| found[a].0.total_cmp(&found[b].0));
        let Some(nearest) = first_clear(&found, &ordered, hidden)? else {
            return Ok(None);
        };
        let limit = found[nearest].0 + 0.5;
        let ties = (0..found.len()).filter(|&i| found[i].0 < limit);
        let mut ties = ties.collect::<Vec<_>>();
        ties.sort_by_key(|&i| found[i].4);
        Ok(first_clear(&found, &ties, hidden)?.map(|i| {
            let (_, p, id, kind, _) = found[i];
            (p, id, kind)
        }))
    }

    /// Return only directions from edges actually containing the segment start.
    fn starting_edges(&self, py: Python<'_>, p: P) -> Vec<usize> {
        py.detach(|| {
            self.edges
                .iter()
                .enumerate()
                .filter_map(|(i, &e)| contains(e, p).then_some(i))
                .collect()
        })
    }

    fn plane_crossings(&self, py: Python<'_>, radius: f64, origin: P, n: P) -> Vec<P> {
        py.detach(|| {
            let mut hits = Vec::new();
            for &[a, b] in &self.edges {
                let ab = sub(b, a);
                let l = norm(ab);
                let den = dot(n, ab);
                if l < 1e-6 || den.abs() < 1e-9 * l {
                    continue;
                }
                let t = dot(n, sub(origin, a)) / den;
                let margin = 1e-4 / l;
                if t <= margin || t >= 1. - margin {
                    continue;
                }
                let p = along(a, ab, t);
                let d = self.distance(p);
                if d <= radius {
                    hits.push((d, p));
                }
            }
            hits.sort_by(|a, b| a.0.total_cmp(&b.0));
            hits.into_iter().map(|h| h.1).collect()
        })
    }

    /// References whose feet, or (under an explicit lock) visible source
    /// vertices, lie within the pixel radius. Keep enumeration order for ties.
    #[pyo3(signature=(start,dir,radius,hovered,extra,hidden))]
    fn reference_foot(
        &self,
        py: Python<'_>,
        start: P,
        dir: P,
        radius: f64,
        hovered: bool,
        extra: Option<P>,
        hidden: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Option<(P, P, &'static str)>> {
        let hits = py.detach(|| {
            let mut hits = Vec::new();
            let mut offer = |p, kind| {
                let t = dot(sub(p, start), dir);
                if !hovered && t <= 1e-6 {
                    return;
                }
                let foot = along(start, dir, t);
                if norm(sub(foot, p)) < 1e-4 {
                    return;
                }
                let d = self.distance(foot);
                if !d.is_finite() {
                    return;
                }
                let dr = if hovered {
                    self.distance(p)
                } else {
                    f64::INFINITY
                };
                if dr < d && dr <= radius {
                    hits.push((dr, foot, p, kind, true));
                }
                if d <= radius {
                    hits.push((d, foot, p, kind, false));
                }
            };
            if let Some(p) = extra {
                offer(p, "from_point");
            }
            for (i, &[a, b]) in self.edges.iter().enumerate() {
                if self.tags[i] & 8 != 0 {
                    continue;
                }
                offer(a, "from_point");
                offer(b, "from_point");
                offer(along(a, sub(b, a), 0.5), "midpoint");
            }
            offer([0.; 3], "origin");
            hits.sort_by(|a, b| a.0.total_cmp(&b.0));
            hits
        });
        for (_, foot, p, kind, source_visible) in hits {
            if let Some(test) = hidden {
                if source_visible && test.call1((p,))?.is_truthy()? {
                    continue;
                }
                if test.call1((foot,))?.is_truthy()? {
                    continue;
                }
            }
            return Ok(Some((foot, p, kind)));
        }
        Ok(None)
    }

    /// The ray-to-line projection is renderer-defined. Ask for it only when
    /// a foreshortened edge passes the fallback collinearity test on screen.
    #[pyo3(signature=(s,world,radius,projector,normal))]
    fn extensions(
        &self,
        py: Python<'_>,
        s: P,
        world: P,
        radius: f64,
        projector: Option<&Bound<'_, PyAny>>,
        normal: Option<P>,
    ) -> PyResult<Vec<(P, P, P)>> {
        let draw = sub(world, s);
        if norm(draw) < 1e-6 {
            return Ok(Vec::new());
        }
        let draw = unit(draw);
        let n = normal.filter(|&n| norm(n) > 1e-9).map(unit);
        let screen = self
            .pixel(s)
            .map(|(x, y)| (self.cursor[0] - x, self.cursor[1] - y));
        let mut hits = Vec::new();
        let mut requests = Vec::new();
        let offer = |a, b, u, t: f64| {
            let t = n
                .filter(|&n| dot(n, u).abs() > 1e-6)
                .map_or(t, |n| dot(n, sub(world, a)) / dot(n, u));
            if (-1e-6..=norm(sub(b, a)) + 1e-6).contains(&t) {
                return None;
            }
            let p = along(a, u, t);
            let d = self.distance(p);
            (d <= radius).then_some((d, p, if t < 0. { a } else { b }, u))
        };
        let can_project = projector.is_some();
        py.detach(|| {
            for (i, &[a, b]) in self.edges.iter().enumerate() {
                if self.tags[i] & 16 != 0 {
                    continue;
                }
                let ab = sub(b, a);
                if norm(ab) < 1e-9 {
                    continue;
                }
                let u = unit(ab);
                if dot(draw, u).abs() >= 0.966 {
                    if let Some(hit) = offer(a, b, u, dot(sub(world, a), u)) {
                        hits.push((i, hit));
                    }
                } else if can_project {
                    let Some((dx, dy)) = screen else {
                        continue;
                    };
                    let off = sub(s, a);
                    if dx.hypot(dy) < 4. || norm(sub(off, u.map(|v| v * dot(off, u)))) > 1e-3 {
                        continue;
                    }
                    if let (Some(pa), Some(pb)) = (self.pixel(a), self.pixel(b)) {
                        let (ex, ey) = (pb.0 - pa.0, pb.1 - pa.1);
                        if ex.hypot(ey) >= 20.
                            && (dx * ex + dy * ey).abs() >= 0.966 * dx.hypot(dy) * ex.hypot(ey)
                        {
                            requests.push((i, a, b, u));
                        }
                    }
                }
            }
        });
        if let Some(project) = projector {
            for (i, a, b, u) in requests {
                let p: P = project.call1((a, u))?.extract()?;
                if let Some(hit) = offer(a, b, u, dot(sub(p, a), u)) {
                    hits.push((i, hit));
                }
            }
        }
        hits.sort_by(|a, b| a.1 .0.total_cmp(&b.1 .0).then(a.0.cmp(&b.0)));
        Ok(hits
            .into_iter()
            .map(|(_, (_, p, end, u))| (p, end, u))
            .collect())
    }
    #[pyo3(signature=(radius,line=None,forward=None,circles=None))]
    fn crossings(
        &self,
        py: Python<'_>,
        radius: f64,
        line: Option<Line>,
        forward: Option<P>,
        circles: Option<Vec<crate::snap_curve::Circle>>,
    ) -> Vec<(P, bool)> {
        py.detach(|| {
            let mut found = Vec::new();
            let mut near = Vec::new();
            for &[a, b] in &self.edges {
                if let Some((p, u)) = line {
                    if forward.is_some() && contains([a, b], p) {
                        continue;
                    }
                    if let Some(hit) = lock_crossing(p, u, a, b, 1e-3) {
                        let d = self.distance(hit);
                        if d <= radius && forward.is_none_or(|dir| dot(sub(hit, p), dir) > 0.) {
                            found.push((d, hit, false));
                        }
                    }
                } else if let (Some(pa), Some(pb)) = (self.pixel(a), self.pixel(b)) {
                    if segment_pixel(self.cursor, pa, pb).0 <= radius {
                        near.push([a, b]);
                    }
                }
            }
            // The store accepts a short overshoot and a length-scaled gap.
            // Most projected overlaps at depth have disjoint world boxes.
            let boxes = near
                .iter()
                .map(|&[a, b]| {
                    let pad = 2.5e-4 + 2e-7 * norm(sub(b, a));
                    let lo: P = std::array::from_fn(|k| a[k].min(b[k]) - pad);
                    let hi: P = std::array::from_fn(|k| a[k].max(b[k]) + pad);
                    (lo, hi)
                })
                .collect::<Vec<_>>();
            for (i, &ab) in near.iter().enumerate() {
                for (j, &cd) in near.iter().enumerate().skip(i + 1) {
                    if (0..3)
                        .any(|k| boxes[i].0[k] > boxes[j].1[k] || boxes[j].0[k] > boxes[i].1[k])
                    {
                        continue;
                    }
                    if let Some(hit) = intersection(ab, cd, Hypot::LibStdCxx, true) {
                        let d = self.distance(hit);
                        if d <= radius {
                            let corner = ab.iter().chain(&cd).any(|&p| norm(sub(p, hit)) < 1e-6);
                            found.push((d, hit, corner));
                        }
                    }
                }
            }
            if let Some(circles) = circles {
                for &c in &circles {
                    for &e in &near {
                        for hit in crate::snap_curve::segment_circle(e, c) {
                            let d = self.distance(hit);
                            if d <= radius {
                                found.push((d, hit, e.iter().any(|&p| norm(sub(hit, p)) < 1e-6)));
                            }
                        }
                    }
                    // Circle/circle ties come after every segment/circle tie.
                }
                for (i, &c) in circles.iter().enumerate() {
                    for &other in &circles[i + 1..] {
                        for hit in crate::snap_curve::circle_circle(c, other) {
                            let d = self.distance(hit);
                            if d <= radius {
                                found.push((d, hit, false));
                            }
                        }
                    }
                }
            }
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut seen = std::collections::HashSet::new();
            found
                .into_iter()
                .filter(|v| seen.insert(v.1.map(|c| (c as f32).to_bits())))
                .map(|(_, p, corner)| (p, corner))
                .collect()
        })
    }

    fn rim(
        &self,
        circle: crate::snap_curve::Circle,
        p: P,
        basis: P,
        radius: f64,
    ) -> Option<(f64, P)> {
        if circle.1 <= 0. {
            return None;
        }
        let hit = crate::snap_curve::rim(circle, p, basis);
        let d = self.distance(hit);
        (d <= radius).then_some((d, hit))
    }

    /// Keep projected segment candidates in this frame. Only the nearest
    /// visible segment's ordinal crosses back to the Python topology facade.
    #[pyo3(signature=(radius,rims,projector,visibility,include_ends,skip_clipped,exclude=None))]
    fn target(
        &self,
        py: Python<'_>,
        radius: f64,
        rims: Vec<(f64, P)>,
        projector: Option<&Bound<'_, PyAny>>,
        visibility: Option<&Bound<'_, PyAny>>,
        include_ends: bool,
        skip_clipped: bool,
        exclude: Option<P>,
    ) -> PyResult<Option<(P, Option<usize>, &'static str)>> {
        let mut targets = Vec::new();
        let mut clipped = Vec::new();
        py.detach(|| {
            for (i, &[a, b]) in self.edges.iter().enumerate() {
                if exclude.is_some_and(|p| contains([a, b], p) || norm(sub(b, a)) < 1e-9) {
                    continue;
                }
                match (self.pixel(a), self.pixel(b)) {
                    (Some(pa), Some(pb)) => {
                        let (d, t) = segment_pixel(self.cursor, pa, pb);
                        if d <= radius && (include_ends || (t > 0. && t < 1.)) {
                            targets.push((d, i, t, None));
                        }
                    }
                    (None, None) => {}
                    _ if !skip_clipped => clipped.push(i),
                    _ => {}
                }
            }
        });
        let parameter = |i: usize| -> PyResult<Option<f64>> {
            let Some(project) = projector else {
                return Ok(None);
            };
            let [a, b] = self.edges[i];
            let ab = sub(b, a);
            if norm(ab) <= 1e-9 {
                return Ok(None);
            }
            let point: Option<P> = project.call1((a, ab))?.extract()?;
            Ok(point.map(|p| (dot(sub(p, a), ab) / dot(ab, ab)).clamp(0., 1.)))
        };
        for i in clipped {
            if let Some(t) = parameter(i)? {
                let [a, b] = self.edges[i];
                let p = along(a, sub(b, a), t);
                let d = self.distance(p);
                if d <= radius && (include_ends || (t > 0. && t < 1.)) {
                    targets.push((d, i, t, Some(p)));
                }
            }
        }
        let count = self.edges.len();
        for (i, (d, p)) in rims.into_iter().enumerate() {
            targets.push((d, count + i, 0., Some(p)));
        }
        targets.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut offset = 0;
        let mut size = 1;
        while offset < targets.len() {
            let batch = &targets[offset..(offset + size).min(targets.len())];
            let mut points = Vec::with_capacity(batch.len());
            for &(_, i, t, point) in batch {
                let p = if let Some(p) = point {
                    p
                } else {
                    let [a, b] = self.edges[i];
                    along(a, sub(b, a), parameter(i)?.unwrap_or(t))
                };
                points.push(p);
            }
            let k = if let Some(test) = visibility {
                let mask: Vec<bool> = test.call1((points.as_slice(),))?.extract()?;
                if mask.len() != points.len() {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "visibility batch length",
                    ));
                }
                mask.iter().position(|h| !*h)
            } else {
                Some(0)
            };
            if let Some(k) = k {
                let i = batch[k].1;
                let kind = if i >= count {
                    "on_circle"
                } else if self.tags[i] & 7 == 1 {
                    "on_line"
                } else {
                    "on_edge"
                };
                return Ok(Some((points[k], (i < count).then_some(i), kind)));
            }
            offset += size;
            size = (size * 2).max(8);
        }
        Ok(None)
    }
}
fn contains([a, b]: [P; 2], p: P) -> bool {
    let ab = sub(b, a);
    let l = dot(ab, ab);
    if l < 1e-12 {
        return norm(sub(p, a)) < 1e-3;
    }
    let t = dot(sub(p, a), ab) / l;
    (-1e-6..=1. + 1e-6).contains(&t) && norm(sub(p, along(a, ab, t))) < 1e-3
}
fn segment_pixel(p: [f64; 2], a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (x, y) = (b.0 - a.0, b.1 - a.1);
    let den = x * x + y * y;
    let t = if den == 0. {
        0.
    } else {
        (((p[0] - a.0) * x + (p[1] - a.1) * y) / den).clamp(0., 1.)
    };
    ((p[0] - (a.0 + t * x)).hypot(p[1] - (a.1 + t * y)), t)
}

pub(crate) fn cross(a: P, b: P) -> P {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn unit(p: P) -> P {
    let l = norm(p);
    p.map(|v| v / l)
}

#[pyfunction]
pub fn drawing_axis(
    delta: P,
    axes: [P; 3],
    cosine: f64,
    min_length: f64,
    prefer_first: bool,
) -> Option<(usize, P)> {
    let length = norm(delta);
    if length < min_length {
        return None;
    }
    let mut best = 0;
    let components = axes.map(|a| dot(delta, a) / length);
    for i in 1..3 {
        if components[i].abs() > components[best].abs()
            || (!prefer_first && components[i].abs() == components[best].abs())
        {
            best = i;
        }
    }
    (components[best].abs() >= cosine).then(|| {
        (
            best,
            axes[best].map(|v| if components[best] > 0. { v } else { -v }),
        )
    })
}

#[pyfunction]
pub fn screen_axis(
    origin: [f64; 2],
    samples: [Option<[f64; 2]>; 3],
    cursor: [f64; 2],
    reach: f64,
) -> Option<usize> {
    samples
        .into_iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let p = p?;
            let (dx, dy) = (p[0] - origin[0], p[1] - origin[1]);
            let span = dx.hypot(dy);
            if span < 12. {
                return None;
            }
            let d = ((cursor[0] - origin[0]) * dy - (cursor[1] - origin[1]) * dx).abs() / span;
            (d <= reach).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|v| v.0)
}

#[pyfunction]
pub fn edge_direction(a: P, b: P, perpendicular: bool, normal: P) -> Option<P> {
    let delta = sub(b, a);
    if norm(delta) < 1e-6 {
        return None;
    }
    if perpendicular {
        let d = cross(normal, delta);
        return (norm(d) >= 1e-9).then(|| unit(d));
    }
    Some(if norm(delta) <= 1e-5 {
        [0.; 3]
    } else {
        unit(delta)
    })
}

#[pyfunction]
pub fn reference_crossings(points: Vec<P>, axes: [P; 3]) -> Vec<(P, usize, usize, usize, usize)> {
    let mut hits = Vec::new();
    for (i, &p) in points.iter().enumerate() {
        for (j, &q) in points.iter().enumerate().skip(i + 1) {
            let w = sub(p, q);
            for (k, &a) in axes.iter().enumerate() {
                for (l, &b) in axes.iter().enumerate() {
                    if k == l {
                        continue;
                    }
                    let ab = dot(a, b);
                    let den = 1. - ab * ab;
                    if den < 1e-9 {
                        continue;
                    }
                    let (wa, wb) = (dot(w, a), dot(w, b));
                    let (s, t) = ((ab * wb - wa) / den, (wb - ab * wa) / den);
                    let hit = along(p, a, s);
                    if norm(sub(hit, along(q, b, t))) <= 1e-3 && s.abs() >= 1e-3 && t.abs() >= 1e-3
                    {
                        hits.push((hit, i, k, j, l));
                    }
                }
            }
        }
    }
    hits
}

#[pyfunction]
pub fn reference_feet(p: P, candidate: P, axes: [P; 3], normal: Option<P>) -> Vec<(usize, P)> {
    let n = normal.filter(|&n| norm(n) > 1e-9).map(unit);
    let delta = sub(candidate, p);
    axes.into_iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let an = n.map_or(0., |n| dot(n, a));
            let t = if an.abs() < 1e-6 {
                if n.is_some_and(|n| dot(n, delta).abs() > 1e-4) {
                    return None;
                }
                dot(delta, a)
            } else {
                dot(delta, n?) / an
            };
            (t.abs() >= 1e-3).then(|| (i, along(p, a, t)))
        })
        .collect()
}

#[pyfunction]
pub fn face_center(py: Python<'_>, rings: Vec<Vec<P>>) -> Option<P> {
    py.detach(|| {
        let outer = rings.first()?;
        if outer.len() < 3 {
            return None;
        }
        let origin = outer[0];
        let pairs = |r: &Vec<P>| {
            r.iter()
                .copied()
                .zip(r.iter().copied().cycle().skip(1))
                .collect::<Vec<_>>()
        };
        let mut normal = [0.; 3];
        for (a, b) in pairs(outer) {
            let n = cross(sub(a, origin), sub(b, origin));
            normal = along(normal, n, 1.);
        }
        if norm(normal) < 1e-12 {
            return None;
        }
        let n = unit(normal);
        let u = unit(cross(
            n,
            if n[0].abs() < 0.9 {
                [1., 0., 0.]
            } else {
                [0., 1., 0.]
            },
        ));
        let v = cross(n, u);
        let flat = |p| {
            let d = sub(p, origin);
            (dot(d, u), dot(d, v))
        };
        let (mut sx, mut sy, mut total) = (0., 0., 0.);
        for (k, r) in rings.iter().enumerate() {
            let (mut a, mut cx, mut cy) = (0., 0., 0.);
            for (p, q) in pairs(r) {
                let ((x, y), (xx, yy)) = (flat(p), flat(q));
                let cr = x * yy - xx * y;
                a += cr;
                cx += (x + xx) * cr;
                cy += (y + yy) * cr;
            }
            if a.abs() < 1e-18 {
                continue;
            }
            let w = a.abs() * if k == 0 { 0.5 } else { -0.5 };
            total += w;
            sx += w * cx / (3. * a);
            sy += w * cy / (3. * a);
        }
        if total <= 1e-12 {
            return None;
        }
        let (x, y) = (sx / total, sy / total);
        let mut inside = false;
        for (p, q) in pairs(outer) {
            let ((xi, yi), (xj, yj)) = (flat(p), flat(q));
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                inside = !inside;
            }
        }
        inside.then(|| along(along(origin, u, x), v, y))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crosses_centimetre_edge_at_lambert_coordinates() {
        let (x, y) = (650000.01, 6860000.02);
        let p = lock_crossing(
            [x, y + 0.01, 0.],
            [1., 0., 0.],
            [x + 0.02, y, 0.],
            [x + 0.02, y + 0.02, 0.],
            1e-3,
        )
        .unwrap();
        assert_eq!(p, [x + 0.02, y + 0.01, 0.]);
    }

    #[test]
    fn screen_axis_ignores_an_end_on_projection() {
        assert_eq!(
            screen_axis(
                [0., 0.],
                [Some([0., 0.]), None, Some([24., 0.])],
                [6., 1.],
                2.
            ),
            Some(2)
        );
    }
}

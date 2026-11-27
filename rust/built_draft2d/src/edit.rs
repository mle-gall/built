// SPDX-License-Identifier: GPL-3.0-or-later
//! Drafting edits on a sketch: Trim, Extend, Fillet, Offset, driving
//! dimensions and moving shared ends. Each returns what it changed, or why
//! it could not (the tool shows that as its refusal).
use crate::geom::{angle_from, arc, carrier_intersections, intersections, polar, Shape, EPS};
use crate::regions::JOIN;
use crate::sketch::{At, Axis, Dim, DimKind, PointRef, Sketch};
use glam::DVec2;
use std::f64::consts::{PI, TAU};

pub type Refusal = &'static str;

/// Parameters where `id` meets the other curves, sorted, inside (0, 1).
fn cut_params(s: &Sketch, id: u32) -> Vec<f64> {
    let Some(c) = s.curve(id) else { return vec![] };
    let mut ts: Vec<f64> = s
        .curves
        .iter()
        .filter(|o| o.id != id)
        .flat_map(|o| intersections(&c.shape, &o.shape))
        .map(|p| c.shape.param_of(p))
        .filter(|t| t.is_finite())
        .collect();
    if matches!(c.shape, Shape::Circle { .. }) {
        ts.iter_mut().for_each(|t| *t = t.rem_euclid(1.0));
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    ts
}

/// Trim: remove the piece of `id` under `pick` between its neighbouring
/// intersections. The first remaining piece keeps the id.
pub fn trim(s: &mut Sketch, id: u32, pick: DVec2) -> Result<Vec<u32>, Refusal> {
    let c = s.curve(id).ok_or("nothing to trim")?.clone();
    let tp = c.shape.project(pick);
    let ts = cut_params(s, id);
    let len = c.shape.length();
    let round = matches!(c.shape, Shape::Circle { .. });
    let inner: Vec<f64> = ts.iter().copied().filter(|t| round || (*t * len > JOIN && (1.0 - t) * len > JOIN)).collect();
    match c.shape {
        Shape::Circle { .. } => {
            if inner.len() < 2 {
                return Err("a circle needs two crossings to trim");
            }
            // The kept arc runs from the crossing after the pick round to the one before it.
            let after = inner.iter().copied().find(|t| *t > tp).unwrap_or(inner[0]);
            let before = inner.iter().rev().copied().find(|t| *t < tp).unwrap_or(*inner.last().unwrap());
            let Shape::Circle { c: centre, r } = c.shape else { unreachable!() };
            let (a0, a1) = (after * TAU, before * TAU);
            s.curve_mut(id).unwrap().shape = arc(centre, r, a0, a1);
            Ok(vec![id])
        }
        _ => {
            if inner.is_empty() {
                return Err("nothing crosses it here: use Delete to remove it");
            }
            let lo = inner.iter().rev().copied().find(|t| *t < tp);
            let hi = inner.iter().copied().find(|t| *t > tp);
            let mut pieces = Vec::new();
            if let Some(lo) = lo {
                pieces.push(c.shape.sub(0.0, lo));
            }
            if let Some(hi) = hi {
                pieces.push(c.shape.sub(hi, 1.0));
            }
            let mut ids = vec![id];
            s.curve_mut(id).unwrap().shape = pieces[0];
            if pieces.len() > 1 {
                let nid = s.add(pieces[1]);
                s.curve_mut(nid).unwrap().segments = c.segments;
                ids.push(nid);
            }
            Ok(ids)
        }
    }
}

/// Extend the end of `id` nearer `pick` to the nearest curve it would meet.
pub fn extend(s: &mut Sketch, id: u32, pick: DVec2) -> Result<(), Refusal> {
    let c = s.curve(id).ok_or("nothing to extend")?.clone();
    let [a, b] = c.shape.ends().ok_or("a circle has no ends")?;
    let at_end = (pick - b).length() <= (pick - a).length();
    let mut best: Option<(f64, DVec2)> = None;
    for o in s.curves.iter().filter(|o| o.id != id) {
        for p in carrier_intersections(&c.shape, &o.shape) {
            let t = c.shape.param_of(p);
            // Beyond the picked end, measured along the curve.
            let gap = match c.shape {
                Shape::Line { .. } => {
                    if at_end {
                        t - 1.0
                    } else {
                        -t
                    }
                }
                Shape::Arc { c: centre, a0, a1, .. } => {
                    let ang = angle_from(centre, p, a0);
                    if ang <= a1 + 1e-12 {
                        continue;
                    }
                    if at_end {
                        ang - a1
                    } else {
                        a0 + TAU - ang
                    }
                }
                Shape::Circle { .. } => unreachable!(),
            };
            if gap > 1e-9 && best.is_none_or(|(g, _)| gap < g) {
                best = Some((gap, p));
            }
        }
    }
    let (_, p) = best.ok_or("it meets nothing that way")?;
    let shape = &mut s.curve_mut(id).unwrap().shape;
    *shape = match *shape {
        Shape::Line { a, b } => {
            if at_end {
                Shape::Line { a, b: p }
            } else {
                Shape::Line { a: p, b }
            }
        }
        Shape::Arc { c, r, a0, a1 } => {
            let ang = angle_from(c, p, a0);
            if at_end {
                arc(c, r, a0, ang)
            } else {
                arc(c, r, ang - TAU, a1)
            }
        }
        other => other,
    };
    Ok(())
}

/// The ends of two lines nearest their crossing: (corner, end of a, end of b).
fn corner(a: &Shape, b: &Shape) -> Option<(DVec2, bool, bool)> {
    let (Shape::Line { a: a0, b: a1 }, Shape::Line { a: b0, b: b1 }) = (*a, *b) else { return None };
    let (d1, d2) = (a1 - a0, b1 - b0);
    let den = d1.perp_dot(d2);
    if den.abs() < 1e-12 * d1.length() * d2.length() {
        return None;
    }
    let p = a0 + d1 * ((b0 - a0).perp_dot(d2) / den);
    Some((p, (a1 - p).length() < (a0 - p).length(), (b1 - p).length() < (b0 - p).length()))
}

/// Fillet two lines with radius `r` (0 makes a sharp corner). The lines
/// are trimmed or extended to the arc; returns the arc's id.
pub fn fillet(s: &mut Sketch, a: u32, b: u32, r: f64) -> Result<Option<u32>, Refusal> {
    let sa = s.curve(a).ok_or("pick two lines")?.shape;
    let sb = s.curve(b).ok_or("pick two lines")?.shape;
    let (p, a_end, b_end) = corner(&sa, &sb).ok_or("fillets join two lines that are not parallel")?;
    let set_end = |sh: Shape, at_end: bool, q: DVec2| match sh {
        Shape::Line { a, b } => {
            if at_end {
                Shape::Line { a, b: q }
            } else {
                Shape::Line { a: q, b }
            }
        }
        o => o,
    };
    let far = |sh: Shape, at_end: bool| match sh {
        Shape::Line { a, b } => {
            if at_end {
                a
            } else {
                b
            }
        }
        _ => unreachable!(),
    };
    if r <= EPS {
        s.curve_mut(a).unwrap().shape = set_end(sa, a_end, p);
        s.curve_mut(b).unwrap().shape = set_end(sb, b_end, p);
        return Ok(None);
    }
    let (ua, ub) = ((far(sa, a_end) - p).normalize(), (far(sb, b_end) - p).normalize());
    let half = ua.angle_to(ub).abs() / 2.0;
    if half < 1e-6 {
        return Err("the lines are parallel");
    }
    let along = r / half.tan();
    if along > (far(sa, a_end) - p).length() + JOIN || along > (far(sb, b_end) - p).length() + JOIN {
        return Err("the radius is larger than the lines allow");
    }
    let (ta, tb) = (p + ua * along, p + ub * along);
    let centre = p + (ua + ub).normalize() * (r / half.sin());
    let (t0, t1) = (angle_from(centre, ta, 0.0), angle_from(centre, tb, 0.0));
    // The short way round between the tangent points.
    let fil = if (t1 - t0).rem_euclid(TAU) <= PI { arc(centre, r, t0, t1) } else { arc(centre, r, t1, t0) };
    s.curve_mut(a).unwrap().shape = set_end(sa, a_end, ta);
    s.curve_mut(b).unwrap().shape = set_end(sb, b_end, tb);
    Ok(Some(s.add(fil)))
}

/// The chain of curves joined end to end through `id` (in order), and
/// whether it closes. Stops at branches.
pub fn chain(s: &Sketch, id: u32) -> (Vec<(u32, bool)>, bool) {
    let Some(c) = s.curve(id) else { return (vec![], false) };
    let Some([_, e]) = c.shape.ends() else { return (vec![(id, false)], true) };
    let mut out = vec![(id, false)];
    let mut cur = e;
    let mut closed = false;
    // Forward.
    loop {
        let next: Vec<_> = s.ends_at(cur, JOIN).into_iter().filter(|(cid, _)| !out.iter().any(|o| o.0 == *cid)).collect();
        let home = s.ends_at(cur, JOIN).iter().any(|(cid, _)| *cid == id) && out.len() > 1;
        if home && next.is_empty() {
            closed = true;
            break;
        }
        if next.len() != 1 {
            break;
        }
        let (nid, at) = next[0];
        let rev = at == At::End;
        out.push((nid, rev));
        let sh = s.curve(nid).unwrap().shape;
        cur = if rev { sh.start().unwrap() } else { sh.end().unwrap() };
    }
    if !closed {
        // Backward from the first curve's start.
        let mut cur = c.shape.start().unwrap();
        loop {
            let prev: Vec<_> = s.ends_at(cur, JOIN).into_iter().filter(|(cid, _)| !out.iter().any(|o| o.0 == *cid)).collect();
            if prev.len() != 1 {
                break;
            }
            let (pid, at) = prev[0];
            let rev = at == At::Start;
            out.insert(0, (pid, rev));
            let sh = s.curve(pid).unwrap().shape;
            cur = if rev { sh.end().unwrap() } else { sh.start().unwrap() };
        }
    }
    (out, closed)
}

/// Offset a single shape by `d` to its left (positive) or right.
fn offset_shape(sh: &Shape, d: f64) -> Option<Shape> {
    match *sh {
        Shape::Line { a, b } => {
            let n = (b - a).normalize().perp();
            Some(Shape::Line { a: a + n * d, b: b + n * d })
        }
        // Left of a counter-clockwise arc is its inside.
        Shape::Arc { c, r, a0, a1 } => (r - d > EPS).then(|| Shape::Arc { c, r: r - d, a0, a1 }),
        Shape::Circle { c, r } => (r - d > EPS).then(|| Shape::Circle { c, r: r - d }),
    }
}

/// Offset the chain through `id` by `d` toward `side` (a point on the side
/// to offset to). Consecutive pieces are re-joined at their crossings, or by
/// a short line where they no longer meet. Returns the new curve ids.
pub fn offset(s: &mut Sketch, id: u32, d: f64, side: DVec2) -> Result<Vec<u32>, Refusal> {
    if d <= EPS {
        return Err("type a distance above zero");
    }
    let (links, closed) = chain(s, id);
    if links.is_empty() {
        return Err("nothing to offset");
    }
    let shapes: Vec<Shape> = links.iter().map(|(cid, _)| s.curve(*cid).unwrap().shape).collect();
    let revs: Vec<bool> = links.iter().map(|l| l.1).collect();
    // Which side, in the chain's direction, from the picked point.
    let k = links.iter().position(|l| l.0 == id).unwrap();
    let here = &shapes[k];
    let t = here.project(side);
    let mut tangent = here.tangent(t);
    if revs[k] {
        tangent = -tangent;
    }
    let mut left = tangent.perp().dot(side - here.at(t)) > 0.0;
    if closed && shapes.len() > 1 {
        // A closed chain offsets inward or outward, by where the pointer is.
        let mut poly = Vec::new();
        let mut area = 0.0;
        for (sh, rev) in shapes.iter().zip(&revs) {
            let mut p = sh.polyline(48);
            if *rev {
                p.reverse();
            }
            area += if *rev { -sh.area_term() } else { sh.area_term() };
            poly.extend(p);
        }
        left = crate::regions::point_in(&poly, side) == (area > 0.0);
    }
    let signed = if left { d } else { -d };
    // A piece run backwards has its own left on the chain's right.
    let mut moved: Vec<Shape> = shapes
        .iter()
        .zip(&revs)
        .map(|(sh, rev)| offset_shape(sh, if *rev { -signed } else { signed }))
        .collect::<Option<_>>()
        .ok_or("the offset is larger than a radius")?;
    let chain_end = |sh: &Shape, rev: bool| if rev { sh.start() } else { sh.end() };
    let chain_start = |sh: &Shape, rev: bool| if rev { sh.end() } else { sh.start() };
    let n = moved.len();
    let mut extra = Vec::new();
    let joints = if closed { n } else { n.saturating_sub(1) };
    for j in 0..joints {
        let (i0, i1) = (j, (j + 1) % n);
        let (Some(e0), Some(s1)) = (chain_end(&moved[i0], revs[i0]), chain_start(&moved[i1], revs[i1])) else { continue };
        if (e0 - s1).length() <= JOIN {
            continue;
        }
        // Meet at the carriers' crossing nearest the old joint.
        let old = chain_end(&shapes[i0], revs[i0]).unwrap();
        let hits = carrier_hits_both(&moved[i0], &moved[i1]);
        if let Some(p) = hits
            .into_iter()
            .min_by(|a, b| (*a - old).length().total_cmp(&(*b - old).length()))
            .filter(|p| (*p - old).length() < 4.0 * d + JOIN)
        {
            moved[i0] = set_end_exact(&moved[i0], !revs[i0], p);
            moved[i1] = set_end_exact(&moved[i1], revs[i1], p);
        } else {
            extra.push(Shape::Line { a: e0, b: s1 });
        }
    }
    let mut ids = Vec::new();
    for (sh, (cid, _)) in moved.into_iter().zip(&links) {
        let segs = s.curve(*cid).unwrap().segments;
        let nid = s.add(sh);
        s.curve_mut(nid).unwrap().segments = segs;
        ids.push(nid);
    }
    for sh in extra {
        ids.push(s.add(sh));
    }
    Ok(ids)
}

fn carrier_hits_both(a: &Shape, b: &Shape) -> Vec<DVec2> {
    // Carriers only: extend lines and arcs as needed.
    let full = |s: &Shape| match *s {
        Shape::Arc { c, r, .. } => Shape::Circle { c, r },
        o => o,
    };
    let line = |s: &Shape| match *s {
        Shape::Line { a, b } => Shape::Line { a: a - (b - a) * 1e6, b: b + (b - a) * 1e6 },
        o => o,
    };
    carrier_intersections(&full(a), &line(&full(b))).into_iter().chain(carrier_intersections(&full(b), &line(&full(a)))).collect()
}

fn set_end_exact(sh: &Shape, at_end: bool, p: DVec2) -> Shape {
    match *sh {
        Shape::Line { a, b } => {
            if at_end {
                Shape::Line { a, b: p }
            } else {
                Shape::Line { a: p, b }
            }
        }
        Shape::Arc { c, r, a0, a1 } => {
            let t = (p - c).y.atan2((p - c).x);
            if at_end {
                let t = a0 + (t - a0).rem_euclid(TAU);
                arc(c, r, a0, t)
            } else {
                let t = a1 - (a1 - t).rem_euclid(TAU);
                arc(c, r, t, a1)
            }
        }
        o => o,
    }
}

/// Move every curve end at `from` to `to` (one propagation step).
pub fn move_ends(s: &mut Sketch, from: DVec2, to: DVec2, except: Option<u32>) {
    for (cid, at) in s.ends_at(from, JOIN) {
        if Some(cid) == except {
            continue;
        }
        let c = s.curve_mut(cid).unwrap();
        c.shape = match c.shape {
            Shape::Line { a, b } => match at {
                At::Start => Shape::Line { a: to, b },
                _ => Shape::Line { a, b: to },
            },
            Shape::Arc { c: centre, r, a0, a1 } => {
                // Keep the arc round: move its centre so both ends stay put but this one.
                let (other, this_end) = match at {
                    At::Start => (polar(centre, r, a1), false),
                    _ => (polar(centre, r, a0), true),
                };
                let mid = polar(centre, r, (a0 + a1) / 2.0);
                let (p, q) = if this_end { (other, to) } else { (to, other) };
                crate::geom::arc3(p, mid, q).unwrap_or(c.shape)
            }
            o => o,
        };
    }
}

/// The current value a dimension measures.
pub fn measure(s: &Sketch, d: &Dim) -> Option<f64> {
    match d.kind {
        DimKind::Length => Some(s.curve(d.a.curve)?.shape.length()),
        DimKind::Radius => s.curve(d.a.curve)?.shape.radius(),
        DimKind::Distance => {
            let (p, q) = (s.point(d.a)?, s.point(d.b?)?);
            Some(match d.axis {
                Axis::Aligned => (q - p).length(),
                Axis::Horizontal => (q.x - p.x).abs(),
                Axis::Vertical => (q.y - p.y).abs(),
            })
        }
        DimKind::Angle => {
            let (a, b) = (s.curve(d.a.curve)?.shape, s.curve(d.b?.curve)?.shape);
            let (ua, ub) = (a.tangent(0.0), b.tangent(0.0));
            Some(ua.angle_to(ub).abs().to_degrees().min(180.0))
        }
    }
}

/// Drive a dimension to `value`: the free end of the later-drawn entity
/// moves, then the ends that met it follow (one step, no solver).
pub fn drive(s: &mut Sketch, dim: u32, value: f64) -> Result<(), Refusal> {
    let d = s.dims.iter().find(|x| x.id == dim).cloned().ok_or("no such dimension")?;
    if !(value.is_finite() && value > 0.0) && d.kind != DimKind::Angle {
        return Err("type a value above zero");
    }
    match d.kind {
        DimKind::Length => {
            let Shape::Line { a, b } = s.curve(d.a.curve).ok_or("lost")?.shape else { return Err("length measures a line") };
            let nb = a + (b - a).normalize() * value;
            s.curve_mut(d.a.curve).unwrap().shape = Shape::Line { a, b: nb };
            move_ends(s, b, nb, Some(d.a.curve));
        }
        DimKind::Radius => {
            let id = d.a.curve;
            let old = s.curve(id).ok_or("lost")?.shape;
            let new = match old {
                Shape::Arc { c, a0, a1, .. } => Shape::Arc { c, r: value, a0, a1 },
                Shape::Circle { c, .. } => Shape::Circle { c, r: value },
                _ => return Err("radius measures an arc or a circle"),
            };
            s.curve_mut(id).unwrap().shape = new;
            if let (Some([s0, e0]), Some([s1, e1])) = (old.ends(), new.ends()) {
                move_ends(s, s0, s1, Some(id));
                move_ends(s, e0, e1, Some(id));
            }
        }
        DimKind::Distance => {
            let b = d.b.ok_or("a distance needs two points")?;
            // The later-drawn point moves.
            let (fixed, free) = if b.curve >= d.a.curve { (d.a, b) } else { (b, d.a) };
            let (p, q) = (s.point(fixed).ok_or("lost")?, s.point(free).ok_or("lost")?);
            let target = match d.axis {
                Axis::Aligned => {
                    let u = (q - p).try_normalize().unwrap_or(DVec2::X);
                    p + u * value
                }
                Axis::Horizontal => DVec2::new(p.x + (q.x - p.x).signum().max(if q.x == p.x { 1.0 } else { -1.0 }) * value, q.y),
                Axis::Vertical => DVec2::new(q.x, p.y + if q.y >= p.y { value } else { -value }),
            };
            let shift = target - q;
            match free.at {
                At::Start | At::End => {
                    move_ends(s, q, target, None);
                }
                At::Mid | At::Centre => {
                    let sh = s.curve(free.curve).ok_or("lost")?.shape;
                    s.curve_mut(free.curve).unwrap().shape = sh.translated(shift);
                    if let Some([s0, e0]) = sh.ends() {
                        move_ends(s, s0, s0 + shift, Some(free.curve));
                        move_ends(s, e0, e0 + shift, Some(free.curve));
                    }
                }
            }
        }
        DimKind::Angle => {
            let b = d.b.ok_or("an angle needs two lines")?;
            let (fixed, free) = if b.curve >= d.a.curve { (d.a.curve, b.curve) } else { (b.curve, d.a.curve) };
            let (Shape::Line { a: f0, b: f1 }, Shape::Line { a: m0, b: m1 }) = (s.curve(fixed).ok_or("lost")?.shape, s.curve(free).ok_or("lost")?.shape) else {
                return Err("angles are between two lines");
            };
            // The moving line turns about its end nearer the fixed line.
            let near = |p: DVec2| Shape::Line { a: f0, b: f1 }.distance(p);
            let (pivot, tip, tip_is_end) = if near(m0) <= near(m1) { (m0, m1, true) } else { (m1, m0, false) };
            let base = (f1 - f0).normalize();
            let cur = (tip - pivot).normalize();
            let sign = if base.perp_dot(cur) >= 0.0 { 1.0 } else { -1.0 };
            let dir = if base.dot(cur) >= 0.0 { base } else { -base };
            let ang = value.to_radians() * sign * if base.dot(cur) >= 0.0 { 1.0 } else { -1.0 };
            let (c, sn) = (ang.cos(), ang.sin());
            let rot = DVec2::new(dir.x * c - dir.y * sn, dir.x * sn + dir.y * c);
            let new_tip = pivot + rot * (tip - pivot).length();
            s.curve_mut(free).unwrap().shape = if tip_is_end { Shape::Line { a: pivot, b: new_tip } } else { Shape::Line { a: new_tip, b: pivot } };
            move_ends(s, tip, new_tip, Some(free));
        }
    }
    if let Some(x) = s.dims.iter_mut().find(|x| x.id == dim) {
        x.value = value;
    }
    Ok(())
}

/// Add a dimension measuring what it measures now.
pub fn add_dim(s: &mut Sketch, kind: DimKind, a: PointRef, b: Option<PointRef>, axis: Axis) -> Result<u32, Refusal> {
    let id = s.fresh_id();
    let mut d = Dim { id, kind, a, b, axis, value: 0.0, offset: 8.0 };
    d.value = measure(s, &d).ok_or("that cannot be measured")?;
    s.dims.push(d);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regions::profile;
    use crate::sketch::Use;
    fn v(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }
    fn close(a: DVec2, b: DVec2) -> bool {
        (a - b).length() < 1e-7
    }
    #[test]
    fn trim_removes_the_picked_piece_and_keeps_the_id_on_the_first() {
        let mut s = Sketch::new(Use::Profile);
        let h = s.add(Shape::Line { a: v(0., 0.), b: v(30., 0.) });
        s.add(Shape::Line { a: v(10., -5.), b: v(10., 5.) });
        s.add(Shape::Line { a: v(20., -5.), b: v(20., 5.) });
        let ids = trim(&mut s, h, v(15., 0.)).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], h);
        assert!(close(s.curve(h).unwrap().shape.end().unwrap(), v(10., 0.)));
        assert!(close(s.curve(ids[1]).unwrap().shape.start().unwrap(), v(20., 0.)));
        assert!(trim(&mut s, ids[1], v(25., 0.)).is_err() || s.curve(ids[1]).is_some());
    }
    #[test]
    fn trimming_a_circle_leaves_an_arc() {
        let mut s = Sketch::new(Use::Profile);
        let c = s.add(Shape::Circle { c: v(0., 0.), r: 10. });
        s.add(Shape::Line { a: v(-20., 0.), b: v(20., 0.) });
        trim(&mut s, c, v(0., -10.)).unwrap();
        let sh = s.curve(c).unwrap().shape;
        assert!(matches!(sh, Shape::Arc { .. }));
        assert!((sh.length() - PI * 10.).abs() < 1e-7);
        assert!(sh.mid().y > 0.);
    }
    #[test]
    fn extend_reaches_the_next_curve() {
        let mut s = Sketch::new(Use::Profile);
        let l = s.add(Shape::Line { a: v(0., 0.), b: v(5., 0.) });
        s.add(Shape::Line { a: v(12., -5.), b: v(12., 5.) });
        s.add(Shape::Line { a: v(20., -5.), b: v(20., 5.) });
        extend(&mut s, l, v(5., 0.)).unwrap();
        assert!(close(s.curve(l).unwrap().shape.end().unwrap(), v(12., 0.)));
        assert!(extend(&mut s, l, v(0., 0.)).is_err());
    }
    #[test]
    fn fillet_makes_a_tangent_arc_and_zero_makes_a_corner() {
        let mut s = Sketch::new(Use::Profile);
        let a = s.add(Shape::Line { a: v(0., 0.), b: v(10., 0.) });
        let b = s.add(Shape::Line { a: v(10., 0.), b: v(10., 10.) });
        let f = fillet(&mut s, a, b, 2.).unwrap().unwrap();
        let arc_shape = s.curve(f).unwrap().shape;
        assert!(close(arc_shape.centre().unwrap(), v(8., 2.)));
        assert!(close(s.curve(a).unwrap().shape.end().unwrap(), v(8., 0.)));
        assert!(close(s.curve(b).unwrap().shape.start().unwrap(), v(10., 2.)));
        // The arc's ends meet the lines.
        let [p, q] = arc_shape.ends().unwrap();
        assert!(close(p, v(8., 0.)) || close(q, v(8., 0.)));
        s.remove(f);
        fillet(&mut s, a, b, 0.).unwrap();
        assert!(close(s.curve(a).unwrap().shape.end().unwrap(), v(10., 0.)));
        assert!(fillet(&mut s, a, b, 50.).is_err());
    }
    #[test]
    fn offset_of_a_closed_rectangle_is_a_closed_rectangle() {
        let mut s = Sketch::new(Use::Profile);
        let p = [v(0., 0.), v(10., 0.), v(10., 5.), v(0., 5.)];
        let first = s.add(Shape::Line { a: p[0], b: p[1] });
        for i in 1..4 {
            s.add(Shape::Line { a: p[i], b: p[(i + 1) % 4] });
        }
        let ids = offset(&mut s, first, 1., v(5., 20.)).unwrap();
        // Outside the loop: outward, whatever side of the clicked edge.
        assert_eq!(ids.len(), 4);
        let pr = profile(&s);
        assert!(pr.ok(), "{:?}", pr.problems);
        assert_eq!(pr.regions.len(), 1);
        assert!((pr.regions[0].outer.area - 12. * 7.).abs() < 1e-7);
        assert!((pr.area - (84. - 50.)).abs() < 1e-7);
    }
    #[test]
    fn a_length_dimension_drives_its_line_and_the_corner_follows() {
        let mut s = Sketch::new(Use::Profile);
        let a = s.add(Shape::Line { a: v(0., 0.), b: v(10., 0.) });
        let b = s.add(Shape::Line { a: v(10., 0.), b: v(10., 10.) });
        let d = add_dim(&mut s, DimKind::Length, PointRef { curve: a, at: At::Mid }, None, Axis::Aligned).unwrap();
        assert!((s.dims[0].value - 10.).abs() < 1e-12);
        drive(&mut s, d, 25.).unwrap();
        assert!(close(s.curve(a).unwrap().shape.end().unwrap(), v(25., 0.)));
        assert!(close(s.curve(b).unwrap().shape.start().unwrap(), v(25., 0.)));
        assert!((measure(&s, &s.dims[0]).unwrap() - 25.).abs() < 1e-9);
        assert!(drive(&mut s, d, -1.).is_err());
    }
    #[test]
    fn radius_and_angle_dimensions_drive() {
        let mut s = Sketch::new(Use::Profile);
        let c = s.add(Shape::Circle { c: v(0., 0.), r: 5. });
        let d = add_dim(&mut s, DimKind::Radius, PointRef { curve: c, at: At::Centre }, None, Axis::Aligned).unwrap();
        drive(&mut s, d, 7.5).unwrap();
        assert_eq!(s.curve(c).unwrap().shape.radius(), Some(7.5));
        let base = s.add(Shape::Line { a: v(0., 0.), b: v(10., 0.) });
        let arm = s.add(Shape::Line { a: v(0., 0.), b: v(0., 10.) });
        let ang = add_dim(&mut s, DimKind::Angle, PointRef { curve: base, at: At::Mid }, Some(PointRef { curve: arm, at: At::Mid }), Axis::Aligned).unwrap();
        assert!((s.dims.iter().find(|d| d.id == ang).unwrap().value - 90.).abs() < 1e-9);
        drive(&mut s, ang, 45.).unwrap();
        let tip = s.curve(arm).unwrap().shape.end().unwrap();
        assert!(close(tip, v(10. / 2f64.sqrt(), 10. / 2f64.sqrt())), "{tip}");
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Inference for 2D tools: end, middle, centre, crossing, on a curve,
//! along an axis from the last point, and the ghosted context's outlines.
use crate::geom::{intersections, Shape};
use glam::DVec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    End,
    Mid,
    Centre,
    Crossing,
    Origin,
    On,
    /// Horizontal or vertical from the reference point.
    AxisX,
    AxisY,
    Free,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::End => "Endpoint",
            Kind::Mid => "Midpoint",
            Kind::Centre => "Centre",
            Kind::Crossing => "Intersection",
            Kind::Origin => "Origin",
            Kind::On => "On edge",
            Kind::AxisX => "On red axis",
            Kind::AxisY => "On green axis",
            Kind::Free => "",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub at: DVec2,
    pub kind: Kind,
    /// The curve it came from (sketch curves are ids, context curves none).
    pub curve: Option<u32>,
}

/// The best snap within `radius` of `p`. `curves`: (id, shape) of the
/// sketch; `context`: ghosted outlines (snappable, never edited).
pub fn snap(p: DVec2, radius: f64, curves: &[(u32, Shape)], context: &[Shape], origin: DVec2, reference: Option<DVec2>) -> Snap {
    let all: Vec<(Option<u32>, &Shape)> = curves.iter().map(|(i, s)| (Some(*i), s)).chain(context.iter().map(|s| (None, s))).collect();
    let mut best: Option<(f64, Snap)> = None;
    let mut offer = |at: DVec2, kind: Kind, curve: Option<u32>, rank: f64| {
        let d = (at - p).length();
        if d <= radius {
            // Points win over curves at the same distance: rank biases.
            let score = d / radius + rank;
            if best.is_none_or(|(b, _)| score < b) {
                best = Some((score, Snap { at, kind, curve }));
            }
        }
    };
    offer(origin, Kind::Origin, None, 0.0);
    for (id, s) in &all {
        if let Some([a, b]) = s.ends() {
            offer(a, Kind::End, *id, 0.0);
            offer(b, Kind::End, *id, 0.0);
            offer(s.mid(), Kind::Mid, *id, 0.1);
        }
        if let Some(c) = s.centre() {
            offer(c, Kind::Centre, *id, 0.1);
        }
    }
    for i in 0..all.len() {
        for j in i + 1..all.len() {
            if (all[i].1.bounds().0 - p).min_element() > radius * 50.0 && false {
                continue;
            }
            for x in intersections(all[i].1, all[j].1) {
                offer(x, Kind::Crossing, all[i].0, 0.05);
            }
        }
    }
    if let Some((_, s)) = best {
        return s;
    }
    // Along an axis from the reference point.
    let mut free = p;
    let mut kind = Kind::Free;
    if let Some(r) = reference {
        if (p.y - r.y).abs() <= radius * 0.6 {
            free = DVec2::new(p.x, r.y);
            kind = Kind::AxisX;
        } else if (p.x - r.x).abs() <= radius * 0.6 {
            free = DVec2::new(r.x, p.y);
            kind = Kind::AxisY;
        }
    }
    // On a curve.
    let mut on: Option<(f64, Snap)> = None;
    for (id, s) in &all {
        let q = s.at(s.project(free));
        let d = (q - free).length();
        if d <= radius * 0.6 && on.is_none_or(|(b, _)| d < b) {
            on = Some((d, Snap { at: q, kind: Kind::On, curve: *id }));
        }
    }
    if let Some((_, s)) = on {
        if kind == Kind::Free {
            return s;
        }
    }
    Snap { at: free, kind, curve: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ends_beat_curves_and_axes_lock_from_the_last_point() {
        let curves = vec![(1, Shape::Line { a: DVec2::new(0., 0.), b: DVec2::new(10., 0.) })];
        let s = snap(DVec2::new(9.7, 0.2), 0.5, &curves, &[], DVec2::new(-100., -100.), None);
        assert_eq!((s.kind, s.at), (Kind::End, DVec2::new(10., 0.)));
        let s = snap(DVec2::new(5.0, 0.3), 0.5, &curves, &[], DVec2::new(-100., -100.), None);
        assert_eq!(s.kind, Kind::Mid);
        let s = snap(DVec2::new(30., 20.1), 0.5, &curves, &[], DVec2::new(-100., -100.), Some(DVec2::new(0., 20.)));
        assert_eq!((s.kind, s.at), (Kind::AxisX, DVec2::new(30., 20.)));
        let ctx = vec![Shape::Circle { c: DVec2::new(50., 50.), r: 5. }];
        let s = snap(DVec2::new(50.2, 49.9), 0.5, &curves, &ctx, DVec2::new(-100., -100.), None);
        assert_eq!((s.kind, s.curve), (Kind::Centre, None));
    }
}

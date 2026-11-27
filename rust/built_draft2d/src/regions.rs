// SPDX-License-Identifier: GPL-3.0-or-later
//! Regions derived from a sketch's curves (never stored): closed loops,
//! outer boundaries with their holes, and what keeps a sketch from being a
//! profile (open ends, branches, crossings).
use crate::geom::{intersections, Shape};
use crate::sketch::Sketch;
use glam::DVec2;
use serde::Serialize;

/// Ends closer than this meet (sketch units).
pub const JOIN: f64 = 1e-4;

/// One boundary piece in loop order: the sketch curve and its direction.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Edge {
    pub curve: u32,
    pub reversed: bool,
    pub shape: Shape,
    pub segments: Option<u32>,
}

/// A closed boundary. Outer loops run counter-clockwise, holes clockwise.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Loop {
    pub edges: Vec<Edge>,
    /// Signed area (positive counter-clockwise).
    pub area: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Region {
    pub outer: Loop,
    pub holes: Vec<Loop>,
}

/// Why a sketch is not (yet) a profile: shown in the workspace as red ends
/// and a walkable list; users keep their last good profile meanwhile.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Problem {
    OpenEnd { at: DVec2, curve: u32 },
    Branch { at: DVec2, curves: Vec<u32> },
    Crossing { at: DVec2, curves: [u32; 2] },
    Empty,
}

/// The regions of a sketch, as sweeps and cutters read them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Profile {
    pub regions: Vec<Region>,
    /// Net area (outer minus holes), mm².
    pub area: f64,
    pub problems: Vec<Problem>,
}

impl Profile {
    pub fn ok(&self) -> bool {
        self.problems.is_empty() && !self.regions.is_empty()
    }
}

impl Loop {
    pub fn polygon(&self, segments: u32) -> Vec<DVec2> {
        let mut pts = Vec::new();
        for e in &self.edges {
            let mut p = e.shape.polyline(e.segments.unwrap_or(segments));
            if e.reversed {
                p.reverse();
            }
            if !pts.is_empty() {
                p.remove(0);
            }
            pts.extend(p);
        }
        if pts.len() > 1 && (pts[0] - pts[pts.len() - 1]).length() < JOIN {
            pts.pop();
        }
        pts
    }
    fn reverse(&mut self) {
        self.edges.reverse();
        for e in &mut self.edges {
            e.reversed = !e.reversed;
        }
        self.area = -self.area;
    }
}

pub(crate) fn point_in(poly: &[DVec2], p: DVec2) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
            inside = !inside;
        }
    }
    inside
}

/// Loops, problems and regions of the sketch's curves.
pub fn profile(s: &Sketch) -> Profile {
    let mut problems = Vec::new();
    // Nodes: distinct end points.
    let mut nodes: Vec<DVec2> = Vec::new();
    let mut node_of = |p: DVec2| -> usize {
        if let Some(i) = nodes.iter().position(|n| (*n - p).length() <= JOIN) {
            i
        } else {
            nodes.push(p);
            nodes.len() - 1
        }
    };
    let mut links: Vec<(usize, usize, usize)> = Vec::new(); // (curve index, start node, end node)
    let mut loops: Vec<Loop> = Vec::new();
    for (i, c) in s.curves.iter().enumerate() {
        match c.shape.ends() {
            Some([a, b]) => {
                let (na, nb) = (node_of(a), node_of(b));
                if na == nb {
                    // A full-turn arc closes on itself.
                    let e = Edge { curve: c.id, reversed: false, shape: c.shape, segments: c.segments };
                    loops.push(Loop { area: e.shape.area_term(), edges: vec![e] });
                } else {
                    links.push((i, na, nb));
                }
            }
            None => {
                let e = Edge { curve: c.id, reversed: false, shape: c.shape, segments: c.segments };
                loops.push(Loop { area: e.shape.area_term(), edges: vec![e] });
            }
        }
    }
    let mut at: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (k, &(_, a, b)) in links.iter().enumerate() {
        at[a].push(k);
        at[b].push(k);
    }
    for (n, users) in at.iter().enumerate() {
        match users.len() {
            1 => problems.push(Problem::OpenEnd { at: nodes[n], curve: s.curves[links[users[0]].0].id }),
            2 => {}
            _ => problems.push(Problem::Branch {
                at: nodes[n],
                curves: users.iter().map(|&k| s.curves[links[k].0].id).collect(),
            }),
        }
    }
    // Walk chains through nodes of degree two.
    let mut used = vec![false; links.len()];
    for start in 0..links.len() {
        if used[start] {
            continue;
        }
        let mut edges = Vec::new();
        let mut k = start;
        let mut node = links[start].1; // walk from the start node forwards
        let first_node = node;
        let closed = loop {
            used[k] = true;
            let (ci, a, b) = links[k];
            let reversed = a != node;
            let c = &s.curves[ci];
            edges.push(Edge { curve: c.id, reversed, shape: c.shape, segments: c.segments });
            node = if reversed { a } else { b };
            if node == first_node {
                break true;
            }
            if at[node].len() != 2 {
                break false;
            }
            let next = at[node][0] + at[node][1] - k;
            if used[next] {
                break false;
            }
            k = next;
        };
        if closed {
            let area = edges
                .iter()
                .map(|e| if e.reversed { -e.shape.area_term() } else { e.shape.area_term() })
                .sum();
            loops.push(Loop { edges, area });
        }
    }
    // Crossings between curves (shared end points are joins, not crossings).
    for i in 0..s.curves.len() {
        for j in i + 1..s.curves.len() {
            let (a, b) = (&s.curves[i].shape, &s.curves[j].shape);
            for p in intersections(a, b) {
                let at_end = |sh: &Shape| sh.ends().is_some_and(|[x, y]| (x - p).length() <= JOIN * 10.0 || (y - p).length() <= JOIN * 10.0);
                if !(at_end(a) && at_end(b)) {
                    problems.push(Problem::Crossing { at: p, curves: [s.curves[i].id, s.curves[j].id] });
                }
            }
        }
    }
    // Containment: a loop inside an odd number of others is a hole.
    let polys: Vec<Vec<DVec2>> = loops.iter().map(|l| l.polygon(48)).collect();
    let depth: Vec<usize> = (0..loops.len())
        .map(|i| {
            let probe = polys[i].first().copied().unwrap_or_default();
            (0..loops.len()).filter(|&j| j != i && loops[j].area.abs() > loops[i].area.abs() && point_in(&polys[j], probe)).count()
        })
        .collect();
    let mut regions: Vec<Region> = Vec::new();
    let mut outer_index = vec![usize::MAX; loops.len()];
    for i in 0..loops.len() {
        if depth[i] % 2 == 0 {
            let mut l = loops[i].clone();
            if l.area < 0.0 {
                l.reverse();
            }
            outer_index[i] = regions.len();
            regions.push(Region { outer: l, holes: Vec::new() });
        }
    }
    for i in 0..loops.len() {
        if depth[i] % 2 == 1 {
            // The smallest enclosing outer.
            let probe = polys[i].first().copied().unwrap_or_default();
            let host = (0..loops.len())
                .filter(|&j| depth[j] == depth[i] - 1 && point_in(&polys[j], probe))
                .min_by(|&a, &b| loops[a].area.abs().total_cmp(&loops[b].area.abs()));
            if let Some(h) = host {
                let mut l = loops[i].clone();
                if l.area > 0.0 {
                    l.reverse();
                }
                regions[outer_index[h]].holes.push(l);
            }
        }
    }
    let area = regions.iter().map(|r| r.outer.area + r.holes.iter().map(|h| h.area).sum::<f64>()).sum();
    if s.curves.is_empty() {
        problems.push(Problem::Empty);
    }
    Profile { regions, area, problems }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::Use;
    fn v(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }
    fn rect(s: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) {
        let p = [v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1)];
        for i in 0..4 {
            s.add(Shape::Line { a: p[i], b: p[(i + 1) % 4] });
        }
    }
    #[test]
    fn a_tube_is_one_region_with_one_hole() {
        let mut s = Sketch::new(Use::Profile);
        rect(&mut s, 0., 0., 100., 50.);
        // The hole drawn counter-clockwise: stored as a hole all the same.
        rect(&mut s, 5., 5., 95., 45.);
        let p = profile(&s);
        assert!(p.ok(), "{:?}", p.problems);
        assert_eq!(p.regions.len(), 1);
        assert_eq!(p.regions[0].holes.len(), 1);
        assert!((p.area - (5000. - 3600.)).abs() < 1e-9);
        assert!(p.regions[0].outer.area > 0. && p.regions[0].holes[0].area < 0.);
    }
    #[test]
    fn a_circle_with_a_bore_and_reversed_lines_close() {
        let mut s = Sketch::new(Use::Profile);
        s.add(Shape::Circle { c: v(0., 0.), r: 20. });
        s.add(Shape::Circle { c: v(0., 0.), r: 5. });
        let p = profile(&s);
        assert!(p.ok());
        assert!((p.area - std::f64::consts::PI * (400. - 25.)).abs() < 1e-9);
        // Lines drawn in any direction still make one loop.
        let mut t = Sketch::new(Use::Profile);
        t.add(Shape::Line { a: v(0., 0.), b: v(10., 0.) });
        t.add(Shape::Line { a: v(0., 10.), b: v(10., 0.) });
        t.add(Shape::Line { a: v(0., 10.), b: v(0., 0.) });
        let q = profile(&t);
        assert!(q.ok());
        assert!((q.area - 50.).abs() < 1e-9);
    }
    #[test]
    fn open_ends_branches_and_crossings_are_named() {
        let mut s = Sketch::new(Use::Profile);
        s.add(Shape::Line { a: v(0., 0.), b: v(10., 0.) });
        s.add(Shape::Line { a: v(10., 0.), b: v(10., 10.) });
        let p = profile(&s);
        assert_eq!(p.problems.iter().filter(|x| matches!(x, Problem::OpenEnd { .. })).count(), 2);
        assert!(!p.ok());
        let mut c = Sketch::new(Use::Profile);
        c.add(Shape::Line { a: v(0., 0.), b: v(10., 10.) });
        c.add(Shape::Line { a: v(0., 10.), b: v(10., 0.) });
        assert!(profile(&c).problems.iter().any(|x| matches!(x, Problem::Crossing { .. })));
        assert!(profile(&Sketch::new(Use::Profile)).problems.contains(&Problem::Empty));
    }
}

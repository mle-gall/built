// SPDX-License-Identifier: GPL-3.0-or-later
//! Import cleaning: one tolerance finds what a supplier drawing gets wrong
//! (arcs drawn as many short lines, tiny fillets, gaps, duplicates, a unit
//! that cannot be right). Each problem has its fix; Fix all applies them.
use crate::edit::{fillet, move_ends};
use crate::geom::{arc, arc3, circumcentre, Shape};
use crate::regions::JOIN;
use crate::sketch::Sketch;
use glam::DVec2;
use serde::Serialize;
use std::f64::consts::PI;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Issue {
    /// Runs of short lines on one circle: each run becomes one arc.
    Segmented { runs: Vec<Vec<u32>> },
    /// Arcs between two lines with a radius under the limit.
    SmallFillets { arcs: Vec<u32>, under: f64 },
    /// Open ends closer than the tolerance: (curve, curve, gap).
    Gaps { pairs: Vec<(u32, u32, f64)> },
    /// The same curve twice.
    Duplicates { curves: Vec<u32> },
    /// The size suggests another unit: (factor to mm, from-unit name).
    Units { factor: f64, from: String, size: DVec2 },
}

impl Issue {
    pub fn key(&self) -> &'static str {
        match self {
            Issue::Segmented { .. } => "segmented",
            Issue::SmallFillets { .. } => "fillets",
            Issue::Gaps { .. } => "gaps",
            Issue::Duplicates { .. } => "duplicates",
            Issue::Units { .. } => "units",
        }
    }
    pub fn count(&self) -> usize {
        match self {
            Issue::Segmented { runs } => runs.iter().map(Vec::len).sum(),
            Issue::SmallFillets { arcs, .. } => arcs.len(),
            Issue::Gaps { pairs } => pairs.len(),
            Issue::Duplicates { curves } => curves.len(),
            Issue::Units { .. } => 1,
        }
    }
}

/// Small fillets are those under twice the tolerance.
pub fn fillet_limit(tol: f64) -> f64 {
    2.0 * tol
}

fn line(s: &Shape) -> Option<(DVec2, DVec2)> {
    match *s {
        Shape::Line { a, b } => Some((a, b)),
        _ => None,
    }
}

/// Chains of lines through points shared by exactly two curves, in order.
fn line_chains(s: &Sketch) -> Vec<Vec<(u32, DVec2, DVec2)>> {
    let lines: Vec<(u32, DVec2, DVec2)> = s.curves.iter().filter_map(|c| line(&c.shape).map(|(a, b)| (c.id, a, b))).collect();
    let mut used = std::collections::HashSet::new();
    let mut chains = Vec::new();
    for &(id, ..) in &lines {
        if used.contains(&id) {
            continue;
        }
        // Walk back to a chain start, then forward.
        let (chain, _) = crate::edit::chain(s, id);
        let mut run = Vec::new();
        for (cid, rev) in chain {
            let Some((a, b)) = s.curve(cid).and_then(|c| line(&c.shape)) else {
                if !run.is_empty() {
                    chains.push(std::mem::take(&mut run));
                }
                continue;
            };
            if used.insert(cid) {
                let (p, q) = if rev { (b, a) } else { (a, b) };
                run.push((cid, p, q));
            }
        }
        if !run.is_empty() {
            chains.push(run);
        }
    }
    chains
}

/// Greedy runs of at least three consecutive lines whose vertices lie on
/// one circle within `tol`, turning the same way by less than 45° each.
fn segmented_runs(s: &Sketch, tol: f64) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    for chain in line_chains(s) {
        let mut i = 0;
        while i + 2 < chain.len() {
            let mut best = i;
            let mut j = i + 2;
            while j < chain.len() && fits(&chain[i..=j], tol) {
                best = j;
                j += 1;
            }
            if best >= i + 2 {
                out.push(chain[i..=best].iter().map(|c| c.0).collect());
                i = best + 1;
            } else {
                i += 1;
            }
        }
    }
    out
}

fn fits(run: &[(u32, DVec2, DVec2)], tol: f64) -> bool {
    let pts: Vec<DVec2> = std::iter::once(run[0].1).chain(run.iter().map(|r| r.2)).collect();
    let (p, q) = (pts[0], pts[pts.len() - 1]);
    let m = pts[pts.len() / 2];
    let Some(c) = circumcentre(p, m, q) else { return false };
    let r = (p - c).length();
    if pts.iter().any(|x| ((*x - c).length() - r).abs() > tol) {
        return false;
    }
    // Same turning direction, small turns, and no segment longer than a
    // quarter of the circle (a square's corners lie on a circle too).
    let mut sign = 0.0;
    for w in pts.windows(3) {
        let (u, v) = (w[1] - w[0], w[2] - w[1]);
        let turn = u.angle_to(v);
        if turn.abs() > PI / 4.0 || turn.abs() < 1e-9 {
            return false;
        }
        if sign == 0.0 {
            sign = turn.signum();
        } else if turn.signum() != sign {
            return false;
        }
    }
    // Tessellated arcs have even segments.
    let lens: Vec<f64> = run.iter().map(|x| (x.2 - x.1).length()).collect();
    let (lo, hi) = lens.iter().fold((f64::MAX, 0.0f64), |(a, b), l| (a.min(*l), b.max(*l)));
    hi <= 2.5 * lo && lens.iter().all(|l| *l < r * PI / 4.0)
}

fn small_fillets(s: &Sketch, under: f64) -> Vec<u32> {
    s.curves
        .iter()
        .filter(|c| matches!(c.shape, Shape::Arc { r, .. } if r < under))
        .filter(|c| {
            let [a, b] = c.shape.ends().unwrap();
            let line_at = |p: DVec2| s.ends_at(p, JOIN).iter().any(|(id, _)| *id != c.id && s.curve(*id).is_some_and(|o| matches!(o.shape, Shape::Line { .. })));
            line_at(a) && line_at(b)
        })
        .map(|c| c.id)
        .collect()
}

fn gaps(s: &Sketch, tol: f64) -> Vec<(u32, u32, f64)> {
    let mut open = Vec::new();
    for c in &s.curves {
        if let Some([a, b]) = c.shape.ends() {
            for p in [a, b] {
                if s.ends_at(p, JOIN).len() == 1 {
                    open.push((c.id, p));
                }
            }
        }
    }
    let mut pairs = Vec::new();
    let mut taken = vec![false; open.len()];
    for i in 0..open.len() {
        if taken[i] {
            continue;
        }
        let best = (0..open.len())
            .filter(|&j| j != i && !taken[j] && open[j].0 != open[i].0)
            .map(|j| (j, (open[j].1 - open[i].1).length()))
            .filter(|(_, g)| *g <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((j, g)) = best {
            taken[i] = true;
            taken[j] = true;
            pairs.push((open[i].0, open[j].0, g));
        }
    }
    pairs
}

fn duplicates(s: &Sketch, tol: f64) -> Vec<u32> {
    let mut out = Vec::new();
    for (i, c) in s.curves.iter().enumerate() {
        let same = s.curves[..i].iter().any(|o| {
            if out.contains(&o.id) {
                return false;
            }
            match (c.shape, o.shape) {
                (Shape::Line { a, b }, Shape::Line { a: p, b: q }) => ((a - p).length() <= tol && (b - q).length() <= tol) || ((a - q).length() <= tol && (b - p).length() <= tol),
                (Shape::Circle { c: c1, r: r1 }, Shape::Circle { c: c2, r: r2 }) => (c1 - c2).length() <= tol && (r1 - r2).abs() <= tol,
                (x @ Shape::Arc { .. }, y @ Shape::Arc { .. }) => (x.centre().unwrap() - y.centre().unwrap()).length() <= tol && (x.start().unwrap() - y.start().unwrap()).length() <= tol && (x.end().unwrap() - y.end().unwrap()).length() <= tol,
                _ => false,
            }
        });
        if same {
            out.push(c.id);
        }
    }
    out
}

/// A unit doubt: no declared unit and a size no profile has in mm, or a
/// declared unit giving a size off by a typical factor.
pub fn unit_doubt(s: &Sketch, declared: Option<&str>) -> Option<Issue> {
    let (lo, hi) = s.bounds()?;
    let size = hi - lo;
    let big = size.x.max(size.y);
    if declared.is_some_and(|u| u != "unitless") {
        return None;
    }
    // Profiles and sections are a few mm to a few metres.
    if big < 20.0 {
        Some(Issue::Units { factor: 25.4, from: "in".into(), size })
    } else if big > 20_000.0 {
        Some(Issue::Units { factor: 0.001, from: "µm".into(), size })
    } else {
        None
    }
}

/// Everything wrong, in the order the list shows it.
pub fn issues(s: &Sketch, tol: f64, declared_unit: Option<&str>) -> Vec<Issue> {
    let doubt = unit_doubt(s, declared_unit);
    // The tolerance is in millimetres: measure a doubtful drawing as it
    // will be once its unit is fixed (ids do not change with scale).
    let probe;
    let s = match &doubt {
        Some(Issue::Units { factor, .. }) => {
            let mut scaled = s.clone();
            scaled.scale(*factor);
            probe = scaled;
            &probe
        }
        _ => s,
    };
    let mut out = Vec::new();
    let runs = segmented_runs(s, tol);
    if !runs.is_empty() {
        out.push(Issue::Segmented { runs });
    }
    let under = fillet_limit(tol);
    let arcs = small_fillets(s, under);
    if !arcs.is_empty() {
        out.push(Issue::SmallFillets { arcs, under });
    }
    let pairs = gaps(s, tol);
    if !pairs.is_empty() {
        out.push(Issue::Gaps { pairs });
    }
    let dups = duplicates(s, tol.min(JOIN * 10.0).max(1e-6));
    if !dups.is_empty() {
        out.push(Issue::Duplicates { curves: dups });
    }
    if let Some(u) = doubt {
        out.push(u);
    }
    out
}

/// Apply one issue's fix. Returns how many curves changed.
pub fn fix(s: &mut Sketch, issue: &Issue) -> usize {
    match issue {
        Issue::Segmented { runs } => {
            let mut n = 0;
            for run in runs {
                let pts: Vec<DVec2> = {
                    let mut v = Vec::new();
                    // Order follows the run: shared ends chain them.
                    let shapes: Vec<_> = run.iter().filter_map(|id| s.curve(*id).map(|c| c.shape)).collect();
                    if shapes.len() != run.len() {
                        continue;
                    }
                    let (a0, b0) = line(&shapes[0]).unwrap();
                    let (a1, b1) = line(&shapes[1]).unwrap();
                    let mut cur = if (b0 - a1).length() <= JOIN || (b0 - b1).length() <= JOIN { a0 } else { b0 };
                    v.push(cur);
                    for sh in &shapes {
                        let (a, b) = line(sh).unwrap();
                        cur = if (a - cur).length() <= JOIN { b } else { a };
                        v.push(cur);
                    }
                    v
                };
                let (p, m, q) = (pts[0], pts[pts.len() / 2], pts[pts.len() - 1]);
                let Some(new) = arc3(p, m, q) else { continue };
                let keep = run[0];
                for id in &run[1..] {
                    s.remove(*id);
                }
                s.curve_mut(keep).unwrap().shape = new;
                n += run.len();
            }
            n
        }
        Issue::SmallFillets { arcs, .. } => {
            let mut n = 0;
            for id in arcs {
                let Some(c) = s.curve(*id).map(|c| c.shape) else { continue };
                let [a, b] = c.ends().unwrap();
                let pick = |p: DVec2| s.ends_at(p, JOIN).into_iter().find(|(o, _)| o != id && s.curve(*o).is_some_and(|x| matches!(x.shape, Shape::Line { .. }))).map(|x| x.0);
                let (Some(la), Some(lb)) = (pick(a), pick(b)) else { continue };
                s.remove(*id);
                if fillet(s, la, lb, 0.0).is_ok() {
                    n += 1;
                }
            }
            n
        }
        Issue::Gaps { pairs } => {
            let mut n = 0;
            for (a, b, _) in pairs {
                let (Some(ca), Some(cb)) = (s.curve(*a).map(|c| c.shape), s.curve(*b).map(|c| c.shape)) else { continue };
                // The two open ends nearest each other.
                let ends = |sh: &Shape| sh.ends().map(|e| e.to_vec()).unwrap_or_default();
                let mut best = None;
                for p in ends(&ca) {
                    for q in ends(&cb) {
                        let g = (p - q).length();
                        if best.is_none_or(|(_, _, bg)| g < bg) {
                            best = Some((p, q, g));
                        }
                    }
                }
                let Some((p, q, _)) = best else { continue };
                // Lines that are not parallel meet at their crossing; else the later end moves.
                let meet = match (ca, cb) {
                    (Shape::Line { a: a0, b: a1 }, Shape::Line { a: b0, b: b1 }) => {
                        let (d1, d2) = (a1 - a0, b1 - b0);
                        let den = d1.perp_dot(d2);
                        (den.abs() > 0.05 * d1.length() * d2.length()).then(|| a0 + d1 * ((b0 - a0).perp_dot(d2) / den)).filter(|x| (*x - p).length() < 10.0 * (p - q).length().max(JOIN))
                    }
                    _ => None,
                };
                match meet {
                    Some(x) => {
                        move_ends(s, p, x, None);
                        move_ends(s, q, x, None);
                    }
                    None => move_ends(s, q, p, None),
                }
                n += 1;
            }
            n
        }
        Issue::Duplicates { curves } => {
            curves.iter().filter(|id| s.remove(**id).is_some()).count()
        }
        Issue::Units { factor, .. } => {
            s.scale(*factor);
            s.curves.len()
        }
    }
}

/// Fix every issue in list order, re-reading the list after each (a fix
/// can remove another issue's curves). One local step for the workspace.
pub fn fix_all(s: &mut Sketch, tol: f64, declared_unit: Option<&str>) -> usize {
    let mut n = 0;
    // Units first: the tolerance is in millimetres.
    if let Some(u) = unit_doubt(s, declared_unit) {
        n += fix(s, &u);
    }
    let declared_unit = Some("mm");
    for _ in 0..8 {
        let list = issues(s, tol, declared_unit);
        let Some(first) = list.into_iter().find(|i| !matches!(i, Issue::Units { .. })) else { break };
        let done = fix(s, &first);
        if done == 0 {
            break;
        }
        n += done;
    }
    n
}

/// Scale by edge: the factor making `id` measure `real`.
pub fn edge_factor(s: &Sketch, id: u32, real: f64) -> Option<f64> {
    let len = s.curve(id)?.shape.length();
    (real > 0.0 && len > 1e-12).then(|| real / len)
}

/// A quarter circle drawn as `n` segments (test fixtures and the fixer).
pub fn segmented_arc(c: DVec2, r: f64, a0: f64, a1: f64, n: usize) -> Vec<Shape> {
    let sh = arc(c, r, a0, a1);
    let pts: Vec<DVec2> = (0..=n).map(|i| sh.at(i as f64 / n as f64)).collect();
    pts.windows(2).map(|w| Shape::Line { a: w[0], b: w[1] }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regions::profile;
    use crate::sketch::Use;
    fn v(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }
    /// A 100 x 50 plate whose top-right corner is a segmented R10 arc, with
    /// a 0.3 mm gap at the bottom-left and a 0.4 mm fillet at the top-left.
    fn plate() -> Sketch {
        let mut s = Sketch::new(Use::Profile);
        s.add(Shape::Line { a: v(0.3, 0.), b: v(100., 0.) });
        s.add(Shape::Line { a: v(100., 0.), b: v(100., 40.) });
        for l in segmented_arc(v(90., 40.), 10., 0., PI / 2., 6) {
            s.add(l);
        }
        s.add(Shape::Line { a: v(90., 50.), b: v(0.4, 50.) });
        s.add(arc(v(0.4, 49.6), 0.4, PI / 2., PI));
        s.add(Shape::Line { a: v(0., 49.6), b: v(0., 0.) });
        s
    }
    #[test]
    fn the_plate_lists_each_problem_once() {
        let s = plate();
        let list = issues(&s, 0.5, Some("mm"));
        let keys: Vec<_> = list.iter().map(Issue::key).collect();
        assert_eq!(keys, ["segmented", "fillets", "gaps"]);
        assert_eq!(list[0].count(), 6);
        assert!(!profile(&s).ok());
    }
    #[test]
    fn fix_all_leaves_a_clean_closed_profile_with_one_true_arc() {
        let mut s = plate();
        fix_all(&mut s, 0.5, Some("mm"));
        assert!(issues(&s, 0.5, Some("mm")).is_empty(), "{:?}", issues(&s, 0.5, Some("mm")));
        let p = profile(&s);
        assert!(p.ok(), "{:?}", p.problems);
        let arcs: Vec<_> = s.curves.iter().filter(|c| matches!(c.shape, Shape::Arc { .. })).collect();
        assert_eq!(arcs.len(), 1);
        assert!((arcs[0].shape.radius().unwrap() - 10.).abs() < 1e-6);
        // 100 x 50 less the R10 corner's waste.
        let expect = 5000. - (100. - 25. * PI);
        assert!((p.area - expect).abs() < 1e-3, "{}", p.area);
    }
    #[test]
    fn a_rectangle_is_not_a_segmented_arc() {
        let mut s = Sketch::new(Use::Profile);
        let p = [v(0., 0.), v(4., 0.), v(4., 3.), v(0., 3.)];
        for i in 0..4 {
            s.add(Shape::Line { a: p[i], b: p[(i + 1) % 4] });
        }
        assert!(issues(&s, 0.5, Some("mm")).is_empty());
        // ...but undeclared, a 4 x 3 drawing doubts its unit.
        let list = issues(&s, 0.5, None);
        assert!(matches!(list[0], Issue::Units { factor, .. } if factor == 25.4));
        fix(&mut s, &list[0]);
        assert!((s.bounds().unwrap().1.x - 101.6).abs() < 1e-9);
    }
    #[test]
    fn scale_by_edge_sets_every_length() {
        let mut s = Sketch::new(Use::Profile);
        let web = s.add(Shape::Line { a: v(0., 0.), b: v(0., 7.0866) });
        s.add(Shape::Circle { c: v(1., 1.), r: 0.5 });
        let k = edge_factor(&s, web, 180.).unwrap();
        s.scale(k);
        assert!((s.curve(web).unwrap().shape.length() - 180.).abs() < 1e-9);
        assert!((k - 25.4).abs() < 1e-3);
    }
}

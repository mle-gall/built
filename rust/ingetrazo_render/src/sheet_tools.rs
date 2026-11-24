// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The Sheets page's drawing tools that catch the model (docs/rust-sheets.md
//! step 6): what each frame offers to snap to (its drawn edges' ends and
//! middles with their model points, and the edges themselves, sent by
//! views/wgpu_sheet_frames.py `snaps`), the snap search (the composer's
//! `nearest_snap_point`: a corner or a middle first, else a point along an
//! edge), and the clicks a tool gathers before it makes its item.

use std::collections::HashMap;

/// What one frame offers, in page mm, with the model points behind them.
#[derive(Default, Clone, Debug)]
pub struct FrameSnaps {
    pub points: Vec<[f64; 2]>,
    pub world: Vec<[f64; 3]>,
    pub edges: Vec<[f64; 4]>,
    pub edge_world: Vec<[[f64; 3]; 2]>,
    /// The circles it sees face on: page centre and radius, mm.
    pub circles: Vec<[f64; 3]>,
}

/// A point a tool took: where on the page, and when it caught the model,
/// the model point, the frame and (along an edge) the edge's ends.
#[derive(Clone, Debug, PartialEq)]
pub struct Pt {
    pub p: [f64; 2],
    pub world: Option<[f64; 3]>,
    pub frame: Option<u64>,
    pub segment: Option<[[f64; 3]; 2]>,
}

impl Pt {
    pub fn free(p: [f64; 2]) -> Pt {
        Pt { p, world: None, frame: None, segment: None }
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"p": self.p, "world": self.world, "frame": self.frame, "segment": self.segment})
    }
}

/// The nearest snap within `thr` mm of `at`: an end or a middle of a drawn
/// edge first (frames under the cursor and the others alike), else the
/// closest point along an edge, its model point taken along it.
pub fn snap_at(snaps: &HashMap<u64, FrameSnaps>, at: [f64; 2], thr: f64) -> Option<Pt> {
    let mut best: Option<(f64, Pt)> = None;
    let mut keys: Vec<&u64> = snaps.keys().collect();
    keys.sort();
    for key in &keys {
        let s = &snaps[key];
        for (p, w) in s.points.iter().zip(&s.world) {
            let d2 = (p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2);
            if d2 < thr * thr && best.as_ref().is_none_or(|b| d2 < b.0) {
                best = Some((d2, Pt { p: *p, world: Some(*w), frame: Some(**key), segment: None }));
            }
        }
    }
    if let Some((_, pt)) = best {
        return Some(pt);
    }
    for key in &keys {
        let s = &snaps[key];
        for (e, w) in s.edges.iter().zip(&s.edge_world) {
            let (dx, dy) = (e[2] - e[0], e[3] - e[1]);
            let l2 = dx * dx + dy * dy;
            if l2 < 1e-12 {
                continue;
            }
            let t = (((at[0] - e[0]) * dx + (at[1] - e[1]) * dy) / l2).clamp(0.0, 1.0);
            let q = [e[0] + dx * t, e[1] + dy * t];
            let d2 = (q[0] - at[0]).powi(2) + (q[1] - at[1]).powi(2);
            if d2 < thr * thr && best.as_ref().is_none_or(|b| d2 < b.0) {
                let wp = std::array::from_fn(|i| w[0][i] + (w[1][i] - w[0][i]) * t);
                best = Some((d2, Pt { p: q, world: Some(wp), frame: Some(**key), segment: Some(*w) }));
            }
        }
    }
    best.map(|b| b.1)
}

/// The direction of a dimension line a→b and the normal its offset runs
/// along, ``(ux, uy, nx, ny)`` (`CotaItem.line_points`' convention).
pub fn sep_frame(a: [f64; 2], b: [f64; 2], axis: &str) -> Option<[f64; 4]> {
    match axis {
        "h" => Some([1.0, 0.0, 0.0, 1.0]),
        "v" => Some([0.0, 1.0, 1.0, 0.0]),
        _ => {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dy);
            (l >= 1e-9).then(|| [dx / l, dy / l, -dy / l, dx / l])
        }
    }
}

/// The offset the cursor gives the dimension line of a→b.
pub fn sep_of(a: [f64; 2], b: [f64; 2], axis: &str, at: [f64; 2]) -> f64 {
    match sep_frame(a, b, axis) {
        Some([_, _, nx, ny]) => (at[0] - a[0]) * nx + (at[1] - a[1]) * ny,
        None => 0.0,
    }
}

/// The dimension line's ends (`_line_ends`): a and b carried along the
/// normal by `sep`, flattened onto a row or a column when forced.
pub fn line_ends(a: [f64; 2], b: [f64; 2], sep: f64, axis: &str) -> ([f64; 2], [f64; 2]) {
    match axis {
        "h" => ([a[0], a[1] + sep], [b[0], a[1] + sep]),
        "v" => ([a[0] + sep, a[1]], [a[0] + sep, b[1]]),
        _ => match sep_frame(a, b, axis) {
            Some([_, _, nx, ny]) => ([a[0] + nx * sep, a[1] + ny * sep], [b[0] + nx * sep, b[1] + ny * sep]),
            None => (a, b),
        },
    }
}

/// The clicks a tool has gathered.
#[derive(Clone, Debug, Default)]
pub struct Pick {
    pub tool: String,
    pub pts: Vec<Pt>,
    /// A dimension forced onto a row ("h") or a column ("v").
    pub axis: String,
    /// A run's (chain, baseline) dimension line offset, once placed.
    pub sep: Option<f64>,
    /// The dimensions a run has placed.
    pub placed: usize,
}

/// The tools that snap to the model and gather clicks on the page.
pub const PICK_TOOLS: [&str; 5] = ["cota", "cota_cadena", "cota_base", "cota_ang", "cota_radio"];

/// The circle whose ring passes within `thr` of `at`: ``(cx, cy, r)``.
pub fn circle_at(snaps: &HashMap<u64, FrameSnaps>, at: [f64; 2], thr: f64) -> Option<[f64; 3]> {
    let mut best: Option<(f64, [f64; 3])> = None;
    for s in snaps.values() {
        for c in s.circles.iter().filter(|c| c[2] >= 0.5) {
            let d = ((at[0] - c[0]).hypot(at[1] - c[1]) - c[2]).abs();
            if d <= thr && best.is_none_or(|b| d < b.0) {
                best = Some((d, *c));
            }
        }
    }
    best.map(|b| b.1)
}

/// ⇧ on an angular dimension's arm: an exact multiple of 15°, the first
/// arm from the page's horizontal, the second from the first (`_ang_step`).
pub fn ang_step(at: [f64; 2], pts: &[Pt]) -> [f64; 2] {
    let Some(v) = pts.first().map(|p| p.p) else { return at };
    let (dx, dy) = (at[0] - v[0], at[1] - v[1]);
    let r = dx.hypot(dy);
    if r < 1e-9 {
        return at;
    }
    let base = pts.get(1).map_or(0.0, |a| (a.p[1] - v[1]).atan2(a.p[0] - v[0]).to_degrees());
    let deg = ((dy.atan2(dx).to_degrees() - base) / 15.0).round() * 15.0 + base;
    let rad = deg.to_radians();
    [v[0] + r * rad.cos(), v[1] + r * rad.sin()]
}

/// ⇧ while a run's offset is chosen: as DIMLINEAR, the cursor pulled out
/// above or below the two points measures horizontally, to a side
/// vertically (`_run_axis`, `_straighten_at`).
pub fn axis_out(a: [f64; 2], b: [f64; 2], at: [f64; 2], was: &str) -> String {
    let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
    let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
    let out_x = (x0 - at[0]).max(at[0] - x1).max(0.0);
    let out_y = (y0 - at[1]).max(at[1] - y1).max(0.0);
    if out_y > out_x {
        "h".into()
    } else if out_x > out_y {
        "v".into()
    } else if !was.is_empty() {
        was.into()
    } else if (b[0] - a[0]).abs() >= (b[1] - a[1]).abs() {
        "h".into()
    } else {
        "v".into()
    }
}

/// The offset that puts dimension a→b's line ON a chain's line (the first
/// dimension's, `sep` off p1→p2): the chain's offset for collinear points,
/// for a jog the distance to that line along the new one's normal
/// (`_chain_sep_for`).
pub fn chain_sep_for(p1: [f64; 2], p2: [f64; 2], sep: f64, axis: &str, a: [f64; 2], b: [f64; 2]) -> f64 {
    match axis {
        "h" => return p1[1] + sep - a[1],
        "v" => return p1[0] + sep - a[0],
        _ => {}
    }
    let (d12x, d12y) = (p2[0] - p1[0], p2[1] - p1[1]);
    let l12 = d12x.hypot(d12y);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    if l12 < 1e-9 || length < 1e-9 {
        return sep;
    }
    let (n12x, n12y) = (-d12y / l12, d12x / l12);
    let (qx, qy) = (p1[0] + n12x * sep, p1[1] + n12y * sep);
    let (nx, ny) = (-dy / length, dx / length);
    let den = nx * d12y - ny * d12x;
    if den.abs() < 1e-9 {
        return sep;
    }
    ((qx - a[0]) * d12y - (qy - a[1]) * d12x) / den
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_corner_wins_over_an_edge_and_an_edge_point_is_taken_along_it() {
        let mut s = HashMap::new();
        s.insert(5, FrameSnaps {
            points: vec![[10.0, 10.0], [20.0, 10.0], [15.0, 10.0]],
            world: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 0.0, 0.0]],
            edges: vec![[10.0, 10.0, 20.0, 10.0]],
            edge_world: vec![[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]],
            circles: vec![],
        });
        let p = snap_at(&s, [10.5, 10.4], 1.0).unwrap();
        assert_eq!((p.p, p.world, p.frame, p.segment), ([10.0, 10.0], Some([0.0, 0.0, 0.0]), Some(5), None));
        let e = snap_at(&s, [12.0, 10.6], 1.0).unwrap();
        assert_eq!(e.p, [12.0, 10.0]);
        assert!((e.world.unwrap()[0] - 0.2).abs() < 1e-12 && e.segment.is_some());
        assert!(snap_at(&s, [12.0, 13.0], 1.0).is_none());
        assert_eq!(line_ends([0.0, 0.0], [10.0, 4.0], 3.0, "h"), ([0.0, 3.0], [10.0, 3.0]));
        assert!((sep_of([0.0, 0.0], [10.0, 0.0], "", [5.0, -2.0]) - -2.0).abs() < 1e-12);
    }
}

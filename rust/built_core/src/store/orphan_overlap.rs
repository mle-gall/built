// SPDX-License-Identifier: GPL-3.0-or-later
//! Remove just the covered stretches of loose, collinear drawing edges.
use super::{
    drawing::vector,
    edit::Change,
    geometry::{length, normalized, Hypot, V3},
    keys::Kind,
    topo::OpError,
    Store,
};
use crate::boxes::{touching, Box3};

type Segment = [V3; 2];
const PLANAR: f64 = 1e-3;
const SPLIT: f64 = 1e-4;

fn overlaps([a, b]: Segment, [c, d]: Segment, h: Hypot) -> bool {
    let ab = b - a;
    let cd = d - c;
    let len = length(cd, h);
    if length(ab, h) < 1e-9 || len < 1e-9 {
        return false;
    }
    let u = normalized(cd, h);
    let t = (a - c).dot(u, h);
    let s = (b - c).dot(u, h);
    length(normalized(ab, h).cross(u, h), h) <= SPLIT
        && length(a - c - u * t, h) <= PLANAR
        && t.max(s).min(len) - t.min(s).max(0.) > SPLIT
}

fn uncovered([a, b]: Segment, cover: &[Segment], h: Hypot) -> Vec<Segment> {
    let len = length(b - a, h);
    let u = (b - a) / len;
    let mut spans: Vec<_> = cover
        .iter()
        .map(|&[p, q]| {
            let s = (p - a).dot(u, h);
            let t = (q - a).dot(u, h);
            (s.min(t), s.max(t))
        })
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let snap = |t| {
        let p = a + u * t;
        let nearest = cover
            .iter()
            .flatten()
            .min_by(|&&x, &&y| {
                let distance = |q: V3| (q - p).dot(q - p, h);
                distance(x).total_cmp(&distance(y))
            })
            .copied()
            .unwrap();
        if length(nearest - p, h) <= PLANAR {
            nearest
        } else {
            p
        }
    };
    let mut pieces = Vec::new();
    let mut at: f64 = 0.;
    for (start, end) in spans.into_iter().chain([(len, len)]) {
        if start - at > PLANAR {
            let p = if at <= 0. { a } else { snap(at) };
            let q = if start >= len { b } else { snap(start) };
            if length(q - p, h) > PLANAR {
                pieces.push([p, q]);
            }
        }
        at = at.max(end);
    }
    pieces
}

pub(crate) fn orphan_pieces(
    segments: &[Segment],
    orphans: &[usize],
    h: Hypot,
) -> Vec<(usize, Vec<Segment>)> {
    let mut plans = Vec::new();
    let boxes: Vec<Box3> = segments
        .iter()
        .map(|&[a, b]| {
            let (a, b): ([f64; 3], [f64; 3]) = (a.into(), b.into());
            (
                std::array::from_fn(|i| a[i].min(b[i]) - PLANAR),
                std::array::from_fn(|i| a[i].max(b[i]) + PLANAR),
            )
        })
        .collect();
    let queries: Vec<_> = orphans.iter().map(|&i| boxes[i]).collect();
    let (rows, columns) = touching(&queries, &boxes, false, 0.);
    let mut hits = rows.into_iter().zip(columns).peekable();
    let mut gone = vec![false; segments.len()];
    for (row, &i) in orphans.iter().enumerate() {
        let mut cover = Vec::new();
        while let Some((_, col)) = hits.next_if(|&(r, _)| r as usize == row) {
            let j = col as usize;
            if j != i && !gone[j] && overlaps(segments[i], segments[j], h) {
                cover.push(segments[j]);
            }
        }
        if cover.is_empty() {
            continue;
        }
        gone[i] = true;
        plans.push((i, uncovered(segments[i], &cover, h)));
    }
    plans
}

impl Store {
    pub(super) fn prune_overlapping_edges(
        &mut self,
        h: Hypot,
        ch: &mut Change,
    ) -> Result<(Vec<u128>, usize, bool), OpError> {
        let edges = self.live_slots(Kind::Edge);
        let orphans: Vec<_> = edges
            .iter()
            .enumerate()
            .filter_map(|(i, &e)| self.edge_faces(e).is_empty().then_some(i))
            .collect();
        let mut removed = Vec::new();
        let mut bumps = 0;
        let mut curved = false;
        if orphans.is_empty() {
            return Ok((removed, bumps, curved));
        }
        let segments: Vec<_> = edges
            .iter()
            .map(|&e| {
                self.state
                    .ends
                    .get(e as usize)
                    .map(|v| vector(*self.state.vpos.get(v as usize)))
            })
            .collect();
        for (i, pieces) in orphan_pieces(&segments, &orphans, h) {
            let e = edges[i];
            let flags = *self.state.eflags.get(e as usize);
            let layer = *self.state.elayer.get(e as usize);
            let curve = *self.state.ecurve.get(e as usize);
            removed.push(self.key(Kind::Edge, e));
            self.remove_edge(e, ch);
            bumps += 1;
            for [a, b] in pieces {
                let (a, b) = (a.into(), b.into());
                if self
                    .lookup(a)
                    .zip(self.lookup(b))
                    .and_then(|(v, w)| self.find_edge(v, w))
                    .is_some()
                {
                    continue;
                }
                let new = self.add_edge(a, b, ch)? as usize;
                self.state.eflags.set(new, flags);
                self.state.elayer.set(new, layer);
                self.state.ecurve.set(new, curve);
                curved |= curve >= 0;
                bumps += 1;
            }
        }
        Ok((removed, bumps, curved))
    }
}

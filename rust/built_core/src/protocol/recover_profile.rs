// SPDX-License-Identifier: GPL-3.0-or-later
//! Straight-sided rounded rectangles, in their own cap plane. Tangency
//! points on the four bounding lines determine the common corner radius.
use super::recover_round::{bind, Profile};
use crate::{exact::Frame, store::Store};
use glam::{DVec2, DVec3};
use std::f64::consts::{FRAC_PI_2, PI};

pub(super) fn rounded(st: &Store, ring: &[u32], p: &[DVec3], axis: DVec3) -> Option<Profile> {
    if p.len() < 12 {
        return None;
    }
    let dir = p
        .iter()
        .zip(p.iter().cycle().skip(1))
        .map(|(&a, &b)| b - a)
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?
        .try_normalize()?;
    let x = (dir - axis * dir.dot(axis)).try_normalize()?;
    let y = axis.cross(x);
    let origin = p[0];
    let uv: Vec<_> = p
        .iter()
        .map(|&v| DVec2::new((v - origin).dot(x), (v - origin).dot(y)))
        .collect();
    let lo = uv
        .iter()
        .copied()
        .fold(DVec2::splat(f64::INFINITY), DVec2::min);
    let hi = uv
        .iter()
        .copied()
        .fold(DVec2::splat(f64::NEG_INFINITY), DVec2::max);
    let size = hi - lo;
    let mid = (lo + hi) * 0.5;
    let eps = 1e-8;
    if p.iter().any(|v| (*v - origin).dot(axis).abs() > eps) {
        return None;
    }
    let sides = |p: DVec2| {
        [
            (p.x - lo.x).abs() <= eps,
            (p.x - hi.x).abs() <= eps,
            (p.y - lo.y).abs() <= eps,
            (p.y - hi.y).abs() <= eps,
        ]
    };
    let mut straight = Vec::new();
    for (&a, &b) in uv.iter().zip(uv.iter().cycle().skip(1)) {
        straight.push(sides(a).into_iter().zip(sides(b)).position(|(a, b)| a && b));
    }
    let mut distances = Vec::new();
    for i in 0..p.len() {
        let (prev, next) = (straight[(i + p.len() - 1) % p.len()], straight[i]);
        if prev.is_some() == next.is_some() {
            continue;
        }
        let a = uv[i];
        let side = prev.or(next)?;
        distances.push(if side < 2 {
            (a.y - lo.y).min(hi.y - a.y)
        } else {
            (a.x - lo.x).min(hi.x - a.x)
        });
    }
    if distances.len() != 8 {
        return None;
    }
    let r = distances.iter().sum::<f64>() / 8.;
    if r <= eps || 2. * r >= size.min_element() || distances.iter().any(|d| (d - r).abs() > eps) {
        return None;
    }
    let mut arcs = vec![Vec::new(); 4];
    let mut sweep = [0.; 4];
    let mut residual = 0f64;
    for i in 0..ring.len() {
        if straight[i].is_some() {
            continue;
        }
        let j = (i + 1) % ring.len();
        let a = (uv[i] + uv[j]) * 0.5;
        let sx = a.x >= mid.x;
        let sy = a.y >= mid.y;
        let k = sx as usize + 2 * sy as usize;
        let cx = if sx { hi.x - r } else { lo.x + r };
        let cy = if sy { hi.y - r } else { lo.y + r };
        let frame = Frame {
            origin: origin + x * cx + y * cy,
            axis,
            reference: x,
        };
        let (edge, binding, dt, err) = bind(st, ring[i], ring[j], frame, r)?;
        if sweep[k] * dt < 0. {
            return None;
        }
        sweep[k] += dt;
        residual = residual.max(err);
        arcs[k].push((edge, binding));
    }
    if sweep
        .iter()
        .any(|s| (s.abs() - FRAC_PI_2).abs() > 1e-8 || s * sweep[0] <= 0.)
    {
        return None;
    }
    Some(Profile {
        arcs,
        area: size.x * size.y - (4. - PI) * r * r,
        residual,
        rounded: true,
    })
}

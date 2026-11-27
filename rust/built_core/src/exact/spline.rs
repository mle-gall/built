// SPDX-License-Identifier: GPL-3.0-or-later
use super::{check, Result};
use glam::{DVec3, DVec4};
use serde::{Deserialize, Serialize};
use truck_geometry::prelude::{KnotVec, TOLERANCE};

/// Distinct knots in increasing order. The active interval runs from
/// expanded knot p through knot n, where p is degree and n is pole count.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Axis {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub multiplicities: Vec<usize>,
    pub periodic: bool,
}

impl Axis {
    pub fn validate(&self, count: usize) -> Result<()> {
        check(
            self.degree > 0 && self.degree < count,
            "invalid spline degree",
        )?;
        check(
            self.knots.len() >= 2 && self.knots.len() == self.multiplicities.len(),
            "invalid knot table",
        )?;
        let span = self.knots.last().unwrap() - self.knots[0];
        check(
            span > 0.
                && span.is_finite()
                && self
                    .knots
                    .windows(2)
                    .all(|k| (k[1] - k[0]) / span > TOLERANCE),
            "unresolved spline knot interval",
        )?;
        let mut total = 0usize;
        for &m in &self.multiplicities {
            check(m > 0 && m <= self.degree + 1, "invalid knot multiplicity")?;
            total = total.checked_add(m).ok_or("knot count overflow")?;
        }
        // Reject empty active domains and interior discontinuities.
        let at = |idx: usize| {
            let mut end = 0;
            self.multiplicities.iter().position(|m| {
                end += m;
                idx < end
            })
        };
        check(at(self.degree) < at(count), "empty spline domain")?;
        check(
            self.multiplicities[1..self.knots.len() - 1]
                .iter()
                .all(|&m| m <= self.degree),
            "discontinuous spline knot",
        )?;
        check(
            Some(total) == count.checked_add(self.degree + 1),
            "knot and control counts disagree",
        )
    }

    pub fn validate_periodic(&self, count: usize, pole: impl Fn(usize) -> DVec4) -> Result<()> {
        if !self.periodic {
            return Ok(());
        }
        let p = self.degree;
        let period_count = count - p;
        check(
            (0..p).all(|i| pole(i) == pole(period_count + i)),
            "periodic spline control rows do not wrap",
        )?;
        let expanded: Vec<_> = self
            .knots
            .iter()
            .zip(&self.multiplicities)
            .flat_map(|(&k, &m)| std::iter::repeat_n(k, m))
            .collect();
        let period = expanded[count] - expanded[p];
        let slack = 32. * f64::EPSILON * (expanded.last().unwrap() - expanded[0]);
        check(
            expanded
                .iter()
                .zip(expanded.iter().skip(period_count))
                .all(|(a, b)| (b - a - period).abs() <= slack),
            "periodic spline knots do not wrap",
        )
    }
    pub fn basis(&self, t: f64) -> Result<Vec<f64>> {
        let start = self.knots[0];
        let span = self.knots.last().unwrap() - start;
        let knots = KnotVec::from_single_multi(
            self.knots.iter().map(|k| (k - start) / span).collect(),
            self.multiplicities.clone(),
        )
        .map_err(|_| "invalid knot table")?;
        let (lo, hi) = (knots[self.degree], knots[knots.len() - self.degree - 1]);
        let mut param = (t - start) / span;
        check(param.is_finite(), "nonfinite parameter")?;
        if self.periodic && (param < lo || param > hi) {
            param = lo + (param - lo).rem_euclid(hi - lo);
        }
        check(
            param >= lo && param <= hi && hi > lo,
            "parameter outside spline domain",
        )?;
        knots
            .try_bspline_basis_functions(self.degree, param)
            .map_err(|_| "unresolved spline basis")
    }
}

pub fn controls(points: &[DVec4]) -> Result<()> {
    check(
        points
            .iter()
            .all(|p| p.is_finite() && p.w > 0. && (p.truncate() * p.w).is_finite()),
        "invalid weighted control point",
    )
}

pub fn rational(points: &[DVec4], basis: impl Iterator<Item = f64>) -> Result<DVec3> {
    let sum = points.iter().zip(basis).fold(DVec4::ZERO, |sum, (p, b)| {
        sum + p.truncate().extend(1.) * (p.w * b)
    });
    let point = sum.truncate() / sum.w;
    check(
        point.is_finite() && sum.w > 0.,
        "singular spline evaluation",
    )?;
    Ok(point)
}

/// Conservative derivative bounds for a homogeneous spline on its active
/// domain. Each derivative control net is made of scaled differences of
/// the preceding net. Translation reduces cancellation for distant models.
pub(super) struct Bounds {
    pub speed: f64,
    pub acceleration: f64,
}
pub(super) fn bounds(points: &[DVec4], axes: &[(&Axis, f64)]) -> Result<Bounds> {
    let origin = points[0].truncate();
    let mut extent: f64 = 0.;
    let mut w_min = f64::INFINITY;
    let mut w_max: f64 = 0.;
    for p in points {
        extent = extent.max((p.truncate() - origin).length());
        w_min = w_min.min(p.w);
        w_max = w_max.max(p.w);
    }
    let mut scale = 0.;
    for &(axis, step) in axes {
        if step == 0. {
            continue;
        }
        // A corner has an unbounded classical second derivative across the
        // knot. Keep point evaluation available, decline this chord proof.
        check(
            axis.multiplicities[1..axis.knots.len() - 1]
                .iter()
                .all(|&m| m < axis.degree),
            "trim crosses a potentially nonsmooth spline knot",
        )?;
        let spacing = axis
            .knots
            .windows(2)
            .map(|k| k[1] - k[0])
            .fold(f64::INFINITY, f64::min);
        scale += 2. * axis.degree as f64 * step.abs() / spacing;
    }
    let ratio = w_max / w_min;
    let speed = 2. * extent * ratio * scale;
    let acceleration = 2. * extent * ratio * scale * scale + 2. * speed * ratio * scale;
    check(
        speed.is_finite() && acceleration.is_finite(),
        "unresolved spline derivative bound",
    )?;
    Ok(Bounds {
        speed,
        acceleration,
    })
}

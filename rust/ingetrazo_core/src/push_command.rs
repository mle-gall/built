// SPDX-License-Identifier: GPL-3.0-or-later
//! Push distance and rollback decisions shared by staged previews and commits.
use pyo3::prelude::*;

#[pyfunction]
pub fn push_top_out(
    mesh: &Bound<'_, PyAny>,
    attempt: &Bound<'_, PyAny>,
    wanted: f64,
    minimum: f64,
    tries: usize,
) -> PyResult<(f64, bool, bool)> {
    let run = |d| attempt.call1((d,))?.extract::<(f64, bool)>();
    let (actual, refused) = run(wanted)?;
    if !refused {
        return Ok((actual, false, false));
    }
    let (mut lo, mut hi) = (0., 1.);
    let mut best = None;
    for _ in 0..tries {
        let mid = (lo + hi) * 0.5;
        let save = mesh.call_method0("capture_state")?;
        let trial = run(actual * mid);
        mesh.call_method1("restore_state", (save,))?;
        let (distance, rejected) = trial?;
        if rejected || distance.abs() < minimum {
            hi = mid;
        } else {
            lo = mid;
            best = Some(distance);
        }
    }
    match best {
        None => Ok((actual, true, false)),
        Some(d) => {
            let (distance, refused) = run(d)?;
            Ok((distance, refused, true))
        }
    }
}

/// Secondary faces must travel the entire distance. A partial push is
/// rolled back using a facade savepoint, which includes paint and IFC rows.
#[pyfunction]
pub fn push_secondary_faces(
    mesh: &Bound<'_, PyAny>,
    count: usize,
    wanted: f64,
    minimum: f64,
    lead_refused: bool,
    prepare: &Bound<'_, PyAny>,
    apply: &Bound<'_, PyAny>,
) -> PyResult<(bool, usize)> {
    let mut missed = usize::from(lead_refused);
    for index in 0..count {
        let distance: Option<f64> = prepare.call1((index,))?.extract()?;
        if distance.is_none_or(|d| (d - wanted).abs() > minimum) {
            missed += 1;
            continue;
        }
        let before = mesh.call_method0("capture_state")?;
        let (refused, partial): (bool, bool) = apply.call0()?.extract()?;
        if partial && !refused {
            mesh.call_method1("restore_state", (before,))?;
        }
        missed += usize::from(refused || partial);
    }
    let refused = missed == count + 1;
    Ok((refused, if refused { 0 } else { missed }))
}

/// Indices of each uninterrupted curve along a cyclic outline. Offset
/// enumerates run starts in corner order; push starts at the first break.
#[pyfunction]
pub fn outline_curve_runs(ids: Vec<Option<i64>>, from_break: bool) -> Vec<(Vec<usize>, bool)> {
    let n = ids.len();
    if n < 2 {
        return vec![];
    }
    let start = (0..n).find(|&i| ids[i] != ids[(i + n - 1) % n]);
    let Some(first) = start else {
        return if ids[0].is_some() {
            vec![((0..n).collect(), true)]
        } else {
            vec![]
        };
    };
    let mut runs = Vec::new();
    let start = if from_break { first } else { 0 };
    for offset in 0..n {
        let i = (start + offset) % n;
        if ids[i].is_none() || ids[i] == ids[(i + n - 1) % n] {
            continue;
        }
        let mut run = vec![i];
        let mut j = i;
        while ids[j] == ids[i] {
            j = (j + 1) % n;
            run.push(j);
        }
        runs.push((run, false));
    }
    runs
}

/// Rebuild one representative per canonical plane in sorted order. Changes to
/// an earlier plane are visible when classifying the next plane in the round.
#[pyfunction]
pub fn rebuild_push_planes(collect: &Bound<'_, PyAny>, apply: &Bound<'_, PyAny>) -> PyResult<()> {
    use crate::math::Vec3;
    use glam::DVec3;
    use std::collections::BTreeMap;
    for round in 0..4 {
        let planes = collect.call1((round,))?;
        let mut unique = BTreeMap::new();
        for pair in planes.try_iter()? {
            let pair = pair?;
            let (origin, normal): (PyRef<'_, Vec3>, PyRef<'_, Vec3>) = pair.extract()?;
            let mut n = DVec3::from_array(normal.coords()).normalize_or_zero();
            let mut axis = 0;
            for i in 1..3 {
                if n[i].abs() > n[axis].abs() {
                    axis = i;
                }
            }
            if n[axis] < 0. {
                n = -n;
            }
            let offset = n.dot(DVec3::from_array(origin.coords()));
            // Python round(x, 3) rounds the original binary64 value. Convert
            // the rounded bits into monotonically ordered integer keys.
            let key = [n.x, n.y, n.z, offset].map(|x| {
                let x = format!("{x:.3}").parse::<f64>().unwrap();
                let bits = if x == 0. { 0 } else { x.to_bits() };
                if bits >> 63 == 1 {
                    !bits
                } else {
                    bits ^ (1u64 << 63)
                }
            });
            unique.entry(key).or_insert(pair.clone());
        }
        let mut changed = false;
        for pair in unique.into_values() {
            changed |= apply.call1((pair,))?.is_truthy()?;
        }
        if !changed {
            break;
        }
    }
    Ok(())
}

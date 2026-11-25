// SPDX-License-Identifier: GPL-3.0-or-later
//! Push distance and rollback decisions shared by staged previews and commits.
use pyo3::prelude::*;

pub(crate) fn push_top_out(
    mesh: &Bound<'_, PyAny>,
    run: impl Fn(f64) -> PyResult<Option<(f64, bool)>>,
    wanted: f64,
    minimum: f64,
    tries: usize,
) -> PyResult<(f64, bool, bool)> {
    let Some((actual, refused)) = run(wanted)? else {
        return Ok((wanted, true, false));
    };
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
        let Some((distance, rejected)) = trial? else {
            return Ok((wanted, true, false));
        };
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
            let Some((distance, refused)) = run(d)? else {
                return Ok((wanted, true, false));
            };
            Ok((distance, refused, true))
        }
    }
}

/// Secondary faces must travel the entire distance. A partial push is
/// rolled back using a facade savepoint, which includes paint and IFC rows.
pub(crate) fn push_secondary_faces(
    mesh: &Bound<'_, PyAny>,
    count: usize,
    wanted: f64,
    minimum: f64,
    lead_refused: bool,
    prepare: impl Fn(usize) -> PyResult<Option<f64>>,
    apply: impl Fn() -> PyResult<(bool, bool)>,
) -> PyResult<(bool, usize)> {
    let mut missed = usize::from(lead_refused);
    for index in 0..count {
        let distance: Option<f64> = prepare(index)?;
        if distance.is_none_or(|d| (d - wanted).abs() > minimum) {
            missed += 1;
            continue;
        }
        let before = mesh.call_method0("capture_state")?;
        let (refused, partial): (bool, bool) = apply()?;
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
pub fn outline_curve_runs(ids: Vec<Option<i128>>, from_break: bool) -> Vec<(Vec<usize>, bool)> {
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

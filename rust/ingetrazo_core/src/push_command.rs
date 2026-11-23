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

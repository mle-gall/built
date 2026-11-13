// SPDX-License-Identifier: GPL-3.0-or-later
//! Packing helpers preserve source ordering; NumPy owns the returned allocation.
use numpy::{IntoPyArray, PyArray1, PyReadonlyArray2};
use pyo3::prelude::*;
use std::collections::HashMap;

/// Map each position to its first equal row, without sorting or copying positions.
/// NaN rows remain distinct, matching numpy.unique(axis=0); signed zero is equal.
#[pyfunction]
pub fn pack_weld<'py>(
    py: Python<'py>,
    pos: PyReadonlyArray2<'_, f32>,
) -> PyResult<Bound<'py, PyArray1<i64>>> {
    let p = pos.as_array();
    if p.shape()[1] != 3 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "positions must have three columns",
        ));
    }
    let remap = py.detach(|| {
        let mut first = HashMap::with_capacity(p.nrows());
        p.rows()
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                if r.iter().any(|v| v.is_nan()) {
                    return i as i64;
                }
                let key: [u32; 3] =
                    std::array::from_fn(|k| if r[k] == 0. { 0 } else { r[k].to_bits() });
                *first.entry(key).or_insert(i as i64)
            })
            .collect::<Vec<_>>()
    });
    Ok(remap.into_pyarray(py))
}

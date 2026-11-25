// SPDX-License-Identifier: GPL-3.0-or-later
//! Broad phase for picking projected edges within a radius of the cursor.
use numpy::{AllowTypeChange, PyArray1, PyArrayLike1};
use pyo3::{exceptions::PyValueError, prelude::*};
use std::collections::HashMap;

type Coords<'py> = PyArrayLike1<'py, f64, AllowTypeChange>;

#[pyclass(frozen, module = "ingetrazo_core")]
pub struct EdgeGrid {
    width: f64,
    height: f64,
    cell: f64,
    reach: f64,
    cells: HashMap<(i64, i64), Vec<i64>>,
    long: Vec<i64>,
    count: usize,
}

#[pymethods]
impl EdgeGrid {
    #[new]
    #[pyo3(signature=(ax, ay, bx, by, ok, width, height, reach, cell=32.0, max_cells=16))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        ax: Coords<'_>,
        ay: Coords<'_>,
        bx: Coords<'_>,
        by: Coords<'_>,
        ok: PyArrayLike1<'_, bool, AllowTypeChange>,
        width: f64,
        height: f64,
        reach: f64,
        cell: f64,
        max_cells: usize,
    ) -> PyResult<Self> {
        if ![width, height, reach]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.)
            || !cell.is_finite()
            || cell <= 0.
        {
            return Err(PyValueError::new_err("invalid grid dimensions"));
        }
        let (ax, ay, bx, by, ok) = (
            ax.as_array(),
            ay.as_array(),
            bx.as_array(),
            by.as_array(),
            ok.as_array(),
        );
        let count = ax.len();
        if [ay.len(), bx.len(), by.len(), ok.len()]
            .iter()
            .any(|&n| n != count)
        {
            return Err(PyValueError::new_err("edge arrays have different lengths"));
        }
        let mut cells: HashMap<_, Vec<_>> = HashMap::new();
        let mut long = Vec::new();
        let bin = |v: f64| (v / cell).floor() as i64;
        for i in 0..count {
            let (x, y, u, v) = (ax[i], ay[i], bx[i], by[i]);
            if !ok[i] || ![x, y, u, v].iter().all(|p| p.is_finite()) {
                continue;
            }
            let (left, right) = (x.min(u).max(-reach), x.max(u).min(width + reach));
            let (top, bottom) = (y.min(v).max(-reach), y.max(v).min(height + reach));
            if left > right || top > bottom {
                continue;
            }
            let (x0, x1, y0, y1) = (bin(left), bin(right), bin(top), bin(bottom));
            let area = (x1.abs_diff(x0) as u128 + 1).saturating_mul(y1.abs_diff(y0) as u128 + 1);
            if area > max_cells as u128 {
                long.push(i as i64);
            } else {
                for row in y0..=y1 {
                    for col in x0..=x1 {
                        cells.entry((col, row)).or_default().push(i as i64);
                    }
                }
            }
        }
        Ok(Self {
            width,
            height,
            cell,
            reach,
            cells,
            long,
            count,
        })
    }

    #[pyo3(signature=(px, py, reach=None))]
    fn near<'p>(
        &self,
        python: Python<'p>,
        px: f64,
        py: f64,
        reach: Option<f64>,
    ) -> Option<Bound<'p, PyArray1<i64>>> {
        if !(0.0..=self.width).contains(&px) || !(0.0..=self.height).contains(&py) {
            return None;
        }
        let r = reach.unwrap_or(self.reach).min(self.reach);
        let bin = |v: f64| (v / self.cell).floor() as i64;
        let cols = bin(px - r)..=bin(px + r);
        let rows = bin(py - r)..=bin(py + r);
        let mut found = self.long.clone();
        let span = (cols.start().abs_diff(*cols.end()) as u128 + 1)
            .saturating_mul(rows.start().abs_diff(*rows.end()) as u128 + 1);
        if span > self.cells.len() as u128 {
            for ((col, row), ids) in &self.cells {
                if cols.contains(col) && rows.contains(row) {
                    found.extend_from_slice(ids);
                }
            }
        } else {
            for row in rows {
                for col in cols.clone() {
                    if let Some(ids) = self.cells.get(&(col, row)) {
                        found.extend_from_slice(ids);
                    }
                }
            }
        }
        if found.len() < self.count / 16 {
            found.sort_unstable();
            found.dedup();
        } else {
            let mut seen = vec![false; self.count];
            for id in found.drain(..) {
                seen[id as usize] = true;
            }
            found.extend(
                seen.into_iter()
                    .enumerate()
                    .filter_map(|(i, set)| set.then_some(i as i64)),
            );
        }
        Some(PyArray1::from_vec(python, found))
    }
}

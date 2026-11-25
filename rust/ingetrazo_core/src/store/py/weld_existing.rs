// SPDX-License-Identifier: GPL-3.0-or-later
//! Batch insertion into a populated store follows the importer's two-pass order.
use super::*;
use crate::store::cow::{FxBuild, FxHashMap};
type FxHashSet<K> = std::collections::HashSet<K, FxBuild>;
type Cell = (i64, i64, i64);

fn neighbours((x, y, z): Cell) -> impl Iterator<Item = Cell> {
    (-1..=1).flat_map(move |dx| {
        (-1..=1).flat_map(move |dy| {
            (-1..=1).filter_map(move |dz| {
                if (dx, dy, dz) == (0, 0, 0) {
                    return None;
                }
                Some((x.checked_add(dx)?, y.checked_add(dy)?, z.checked_add(dz)?))
            })
        })
    })
}

#[pyfunction]
pub(crate) fn weld_candidates<'py>(
    py: Python<'py>,
    keys: PyReadonlyArray2<'py, i64>,
) -> PyResult<Bound<'py, PyArray1<bool>>> {
    let rows = keys.as_array();
    if rows.ncols() != 3 {
        return Err(PyValueError::new_err("expected three cell coordinates"));
    }
    let cells: Vec<_> = rows
        .rows()
        .into_iter()
        .map(|r| (r[0], r[1], r[2]))
        .collect();
    let set: FxHashSet<_> = cells.iter().copied().collect();
    Ok(cells
        .iter()
        .map(|&c| neighbours(c).any(|n| set.contains(&n)))
        .collect::<Vec<_>>()
        .into_pyarray(py))
}

fn batch_cell(x: f64, scale: f64) -> i64 {
    let n = (x * scale).round_ties_even();
    // Match numpy.round(...).astype(int64), including its out-of-range sentinel.
    if !(-9223372036854775808.0..9223372036854775808.0).contains(&n) {
        i64::MIN
    } else {
        n as i64
    }
}

pub(super) fn insert(
    store: &mut Store,
    points: &[[f64; 3]],
    supplied: &[[f64; 3]],
) -> Result<(Vec<u128>, Vec<i64>), OpError> {
    let scale = store.weld.scale;
    let cells: Vec<_> = points
        .iter()
        .map(|&p| {
            let [x, y, z] = store.state.frame.definition_point(p);
            (
                batch_cell(x, scale),
                batch_cell(y, scale),
                batch_cell(z, scale),
            )
        })
        .collect();
    let occupied: FxHashSet<_> = cells.iter().copied().collect();
    let candidates: FxHashSet<_> = occupied
        .iter()
        .copied()
        .filter(|&c| {
            store.state.registry.contains(&c)
                || neighbours(c).any(|n| occupied.contains(&n) || store.state.registry.contains(&n))
        })
        .collect();
    let mut work = store.clone();
    let mut indices = vec![0; points.len()];
    let mut change = Change::default();
    let mut keys = Vec::new();
    let mut by_slot = FxHashMap::default();
    let mut isolated = FxHashMap::default();
    for candidate in [false, true] {
        for (i, &cell) in cells.iter().enumerate() {
            if candidates.contains(&cell) != candidate {
                continue;
            }
            if !candidate {
                if let Some(&index) = isolated.get(&cell) {
                    indices[i] = index;
                    continue;
                }
            }
            let slot = work.vertex(supplied[i], &mut change)?;
            let index = *by_slot.entry(slot).or_insert_with(|| {
                let next = keys.len() as i64;
                keys.push(work.key(Kind::Vertex, slot));
                next
            });
            indices[i] = index;
            if !candidate {
                isolated.insert(cell, index);
            }
        }
    }
    work.finish(change);
    *store = work;
    Ok((keys, indices))
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Snap candidates keep their Python handles. Only a visibility query crosses
//! back to the viewport; repeated world positions share its answer for this query.
use crate::math::{xyz, Vec3};
use pyo3::{prelude::*, types::PyList};
use std::collections::{HashMap, HashSet};
type Obj<'py> = Bound<'py, PyAny>;
type Key = [u64; 3];
fn key(p: &Obj<'_>) -> PyResult<Key> {
    Ok(xyz(p)?
        .to_array()
        .map(|v| if v == 0. { 0 } else { v.to_bits() }))
}
fn masked(test: &Obj<'_>, p: &Obj<'_>) -> PyResult<bool> {
    test.call1((p,))?.is_truthy()
}

fn first_batch(
    n: usize,
    batched: bool,
    mut check: impl FnMut(usize, usize) -> PyResult<Option<usize>>,
) -> PyResult<Option<usize>> {
    let (mut start, mut size) = (0, 1usize);
    while start < n {
        let end = start.saturating_add(size).min(n);
        if let Some(i) = check(start, end)? {
            return Ok(Some(start + i));
        }
        start = end;
        if batched {
            size = size.saturating_mul(2).max(8);
        }
    }
    Ok(None)
}

#[pyfunction]
#[pyo3(signature=(points,is_occluded,are_occluded=None))]
pub(crate) fn first_visible(
    points: &Obj<'_>,
    is_occluded: Option<&Obj<'_>>,
    are_occluded: Option<&Obj<'_>>,
) -> PyResult<Option<usize>> {
    let n = points.len()?;
    let Some(one) = is_occluded else {
        return Ok((n > 0).then_some(0));
    };
    first_batch(n, are_occluded.is_some(), |i, end| {
        if let Some(batch) = are_occluded.filter(|_| i != 0) {
            let ps = points.get_item(pyo3::types::PySlice::new(
                points.py(),
                i as isize,
                end as isize,
                1,
            ))?;
            for (j, b) in batch.call1((ps,))?.try_iter()?.enumerate() {
                if !b?.is_truthy()? {
                    return Ok(Some(j));
                }
            }
            Ok(None)
        } else {
            Ok((!masked(one, &points.get_item(i)?)?).then_some(0))
        }
    })
}

fn fill<'py>(
    points: impl IntoIterator<Item = Obj<'py>>,
    cache: &mut HashMap<Key, bool>,
    batch: &Obj<'py>,
) -> PyResult<()> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    let args = PyList::empty(batch.py());
    for p in points {
        let k = key(&p)?;
        if !cache.contains_key(&k) && seen.insert(k) {
            keys.push(k);
            args.append(p)?;
        }
    }
    if !keys.is_empty() {
        for (k, m) in keys.into_iter().zip(batch.call1((args,))?.try_iter()?) {
            cache.insert(k, m?.is_truthy()?);
        }
    }
    Ok(())
}

struct Candidate<'py> {
    item: Obj<'py>,
    point: Obj<'py>,
    distance: f64,
    grouped: bool,
    occludable: bool,
}
impl Candidate<'_> {
    fn hidden(&self, cache: &mut HashMap<Key, bool>, one: Option<&Obj<'_>>) -> PyResult<bool> {
        let Some(one) = one.filter(|_| self.occludable) else {
            return Ok(false);
        };
        match cache.entry(key(&self.point)?) {
            std::collections::hash_map::Entry::Occupied(e) => Ok(*e.get()),
            std::collections::hash_map::Entry::Vacant(e) => {
                Ok(*e.insert(masked(one, &self.point)?))
            }
        }
    }
}
#[pyfunction]
#[pyo3(signature=(candidates,is_occluded,are_occluded=None))]
pub(crate) fn resolve_points<'py>(
    candidates: &Obj<'py>,
    is_occluded: Option<&Obj<'py>>,
    are_occluded: Option<&Obj<'py>>,
) -> PyResult<Option<Obj<'py>>> {
    let mut items = Vec::new();
    for c in candidates.try_iter()? {
        let c = c?;
        items.push(Candidate {
            distance: c.get_item(0)?.extract()?,
            point: c.get_item(1)?,
            grouped: !c.get_item(4)?.is_none(),
            occludable: c.get_item(5)?.is_truthy()?,
            item: c,
        });
    }
    let mut order = (0..items.len()).collect::<Vec<_>>();
    order.sort_by(|&a, &b| items[a].distance.total_cmp(&items[b].distance));
    let mut cache = HashMap::new();
    let nearest = first_batch(order.len(), are_occluded.is_some(), |i, end| {
        let group = &order[i..end];
        if let Some(batch) = are_occluded.filter(|_| i != 0 && is_occluded.is_some()) {
            fill(
                group
                    .iter()
                    .map(|&j| &items[j])
                    .filter(|c| c.occludable)
                    .map(|c| c.point.clone()),
                &mut cache,
                batch,
            )?;
        }
        let mut first = None;
        for (j, &idx) in group.iter().enumerate() {
            if !items[idx].hidden(&mut cache, is_occluded)? && first.is_none() {
                first = Some(j);
            }
        }
        Ok(first)
    })?;
    let Some(start) = nearest else {
        return Ok(None);
    };
    let limit = items[order[start]].distance + 0.5;
    let mut ties = order[start..]
        .iter()
        .copied()
        .filter(|&i| items[i].distance < limit)
        .collect::<Vec<_>>();
    ties.sort_unstable_by_key(|&i| (items[i].grouped, i));
    for i in ties {
        if !items[i].hidden(&mut cache, is_occluded)? {
            return Ok(Some(items[i].item.clone()));
        }
    }
    Ok(None)
}

#[pyclass]
pub(crate) struct PointVisibility {
    one: Py<PyAny>,
    many: Option<Py<PyAny>>,
    known: HashMap<Key, bool>,
}
#[pymethods]
impl PointVisibility {
    #[new]
    #[pyo3(signature=(one,many=None))]
    fn new(one: Py<PyAny>, many: Option<Py<PyAny>>) -> Self {
        Self {
            one,
            many,
            known: HashMap::new(),
        }
    }
    fn __call__(&mut self, py: Python<'_>, points: Vec<[f64; 3]>) -> PyResult<Vec<bool>> {
        let mut keys = Vec::with_capacity(points.len());
        let mut missing = Vec::new();
        let mut seen = HashSet::new();
        for p in points {
            let k = p.map(|v| if v == 0. { 0 } else { v.to_bits() });
            keys.push(k);
            if !self.known.contains_key(&k) && seen.insert(k) {
                missing.push((
                    k,
                    Vec3::from(glam::DVec3::from_array(p))
                        .into_pyobject(py)?
                        .into_any(),
                ));
            }
        }
        if let Some(batch) = self.many.as_ref().filter(|_| missing.len() > 1) {
            let args = PyList::new(py, missing.iter().map(|(_, p)| p))?;
            for ((k, _), b) in missing
                .into_iter()
                .zip(batch.bind(py).call1((args,))?.try_iter()?)
            {
                self.known.insert(k, b?.is_truthy()?);
            }
        } else {
            for (k, p) in missing {
                self.known.insert(k, masked(self.one.bind(py), &p)?);
            }
        }
        keys.into_iter()
            .map(|k| {
                self.known
                    .get(&k)
                    .copied()
                    .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(k.map(f64::from_bits)))
            })
            .collect()
    }
}

#[pyfunction]
pub(crate) fn hidden_batch<'py>(
    points: Vec<[f64; 3]>,
    one: &Obj<'py>,
    many: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = one.py();
    let pts = PyList::new(
        py,
        points
            .into_iter()
            .map(|p| Vec3::from(glam::DVec3::from_array(p))),
    )?;
    if let Some(batch) = many.filter(|_| pts.len() > 1) {
        return batch.call1((pts,));
    }
    let out = PyList::empty(py);
    for p in pts.iter() {
        out.append(one.call1((p,))?)?;
    }
    Ok(out.into_any())
}

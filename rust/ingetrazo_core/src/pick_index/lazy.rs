// SPDX-License-Identifier: GPL-3.0-or-later
//! Lazy placement bounds, query budgets and eviction.
use super::*;
use numpy::{PyReadonlyArray1, PyReadonlyArray2};
use pyo3::types::PySet;
use std::collections::{HashMap, HashSet};

fn transform<'py>(group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let data = group.getattr("xform")?.call_method0("data")?;
    group
        .py()
        .import("builtins")?
        .getattr("tuple")?
        .call1((data,))
}

fn frame(view: &Obj, group: &Obj) -> PyResult<([f64; 3], [f64; 3])> {
    Ok(view
        .call_method1("_placement_frame", (group,))?
        .get_item(0)?
        .extract::<Option<_>>()?
        .unwrap_or(([-f64::INFINITY; 3], [f64::INFINITY; 3])))
}

fn update_matrices<'py>(view: &Obj<'py>, key: &Obj<'py>, memo: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mut table = memo.get_item(1)?;
    let mut matrices = memo.get_item(2)?;
    if !table.is_none() {
        view.call_method0("_placements")?;
        let mut changed = false;
        let groups = table.get_item(0)?;
        let mut lo = table.get_item(1)?;
        let mut hi = table.get_item(2)?;
        for (i, g) in groups.try_iter()?.enumerate() {
            let g = g?;
            let xf = transform(&g)?;
            if xf.eq(matrices.get_item(i)?)? {
                continue;
            }
            if !changed {
                lo = lo.call_method0("copy")?;
                hi = hi.call_method0("copy")?;
                matrices = matrices.call_method0("copy")?;
                changed = true;
            }
            let (min, max) = frame(view, &g)?;
            lo.set_item(i, min)?;
            hi.set_item(i, max)?;
            matrices.set_item(i, xf)?;
        }
        if changed {
            table = (groups, lo, hi, table.get_item(3)?, table.get_item(4)?)
                .into_pyobject(view.py())?
                .into_any();
        }
    }
    view.setattr("_pick_lazy_memo", (key, &table, matrices))?;
    Ok(table)
}

#[pyfunction]
pub fn pick_lazy_table<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let key = view.call_method0("_placements_epoch")?;
    let memo = attr(view, "_pick_lazy_memo")?;
    let mut reused = HashMap::new();
    let mut old = none(py);
    let mut kept = none(py);
    if !memo.is_none() {
        let old_key = memo.get_item(0)?;
        if old_key.eq(&key)? {
            return memo.get_item(1);
        }
        if view
            .call_method1("_epoch_only_matrices", (&old_key, &key))?
            .is_truthy()?
        {
            return update_matrices(view, &key, &memo);
        }
        old = memo.get_item(1)?;
        if !old.is_none() {
            let kw = PyDict::new(py);
            kw.set_item("ctx", true)?;
            kept = view.call_method("_epoch_kept_tops", (old_key, &key), Some(&kw))?;
            if kept.is_truthy()? {
                for (i, g) in old.get_item(0)?.try_iter()?.enumerate() {
                    reused.insert(g?.as_ptr() as usize, i);
                }
            }
        }
    }
    let count = py.import("core.mesh")?.getattr("face_count")?;
    let groups = PyList::empty(py);
    let ids = PySet::empty(py)?;
    let mut source = Vec::new();
    let mut counts = Vec::new();
    let old_counts = if reused.is_empty() {
        None
    } else {
        Some(old.get_item(3)?.extract::<PyReadonlyArray1<i64>>()?)
    };
    for g in view.call_method0("_placements")?.try_iter()? {
        let g = g?;
        if attr(&g, "xform")?.is_none() || attr(&g, "billboard")?.is_truthy()? {
            continue;
        }
        let id = g.as_ptr() as usize;
        let row = match reused.get(&id) {
            Some(&i) if kept.contains(view.call_method1("_placement_top", (&g,))?)? => Some(i),
            _ => None,
        };
        counts.push(match (row, &old_counts) {
            (Some(i), Some(a)) => a.as_array()[i],
            _ => count.call1((g.getattr("mesh")?,))?.extract()?,
        });
        ids.add(id)?;
        groups.append(g)?;
        source.push(row);
    }
    let threshold: i64 = view.getattr("_PICK_LAZY_MIN_FACES")?.extract()?;
    if counts.iter().sum::<i64>() < threshold {
        view.setattr("_pick_lazy_memo", (key, py.None(), py.None()))?;
        return Ok(none(py));
    }
    let mut lo = Vec::new();
    let mut hi = Vec::new();
    let matrices = PyList::empty(py);
    let old_rows = if reused.is_empty() {
        None
    } else {
        Some((
            old.get_item(1)?.extract::<PyReadonlyArray2<f64>>()?,
            old.get_item(2)?.extract::<PyReadonlyArray2<f64>>()?,
            memo.get_item(2)?,
        ))
    };
    for (g, row) in groups.iter().zip(source) {
        match (row, &old_rows) {
            (Some(i), Some((a, b, xfs))) => {
                lo.extend(a.as_array().row(i));
                hi.extend(b.as_array().row(i));
                matrices.append(xfs.get_item(i)?)?;
            }
            _ => {
                let (min, max) = frame(view, &g)?;
                lo.extend(min);
                hi.extend(max);
                matrices.append(transform(&g)?)?;
            }
        }
    }
    let table = (
        groups,
        matrix(py, lo)?,
        matrix(py, hi)?,
        counts.into_pyarray(py),
        ids,
    )
        .into_pyobject(py)?
        .into_any();
    view.setattr("_pick_lazy_memo", (key, &table, matrices))?;
    Ok(table)
}

struct PlacementTable<'py> {
    groups: Obj<'py>,
    lo: PyReadonlyArray2<'py, f64>,
    hi: PyReadonlyArray2<'py, f64>,
    counts: PyReadonlyArray1<'py, i64>,
}

impl<'py> PlacementTable<'py> {
    fn new(table: &Obj<'py>) -> PyResult<Self> {
        let out = Self {
            groups: table.get_item(0)?,
            lo: table.get_item(1)?.extract()?,
            hi: table.get_item(2)?.extract()?,
            counts: table.get_item(3)?.extract()?,
        };
        let n = out.groups.len()?;
        if out.lo.as_array().dim() != (n, 3)
            || out.hi.as_array().dim() != (n, 3)
            || out.counts.as_array().len() != n
        {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "inconsistent placement bounds",
            ));
        }
        Ok(out)
    }
    fn distances(&self, indices: &[usize], view: &Obj) -> PyResult<Vec<f64>> {
        let eye = xyz(&view.getattr("camera")?.call_method0("eye")?)?.to_array();
        let (lo, hi) = (self.lo.as_array(), self.hi.as_array());
        Ok(indices
            .iter()
            .map(|&i| {
                let delta = std::array::from_fn::<_, 3, _>(|k| {
                    eye[k].max(lo[[i, k]]).min(hi[[i, k]]) - eye[k]
                });
                (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt()
            })
            .collect())
    }
    fn ray(
        &self,
        view: &Obj<'py>,
        table: &Obj<'py>,
        origin: &Obj,
        direction: &Obj,
        reach: Option<f64>,
        pixel: Option<(f64, f64)>,
    ) -> PyResult<Vec<usize>> {
        let py = view.py();
        let o = xyz(origin)?.to_array();
        let d = xyz(direction)?.to_array();
        let lo = self.lo.as_array();
        let hi = self.hi.as_array();
        let inv = d.map(|v| if v.abs() > 1e-12 { 1. / v } else { 1e12 });
        let mut hits = Vec::new();
        for i in 0..lo.nrows() {
            let mut near = 0.0f64;
            let mut far = reach.unwrap_or(f64::INFINITY);
            for k in 0..3 {
                let a = (lo[[i, k]] - o[k]) * inv[k];
                let b = (hi[[i, k]] - o[k]) * inv[k];
                near = near.max(a.min(b));
                far = far.min(a.max(b));
            }
            if near <= far {
                hits.push((i, near));
            }
        }
        if let Some(px) = pixel.filter(|_| hits.len() > 1) {
            let seen = view.call_method1("_depth_world_at", px)?;
            if !seen.is_none() && seen.get_item(0)?.eq("hit")? {
                let limit = xyz(&seen.get_item(1)?)?.distance(o.into()) * 1.02 + 0.05;
                hits.retain(|(_, near)| *near <= limit);
            }
        }
        let rows: Vec<i64> = hits.into_iter().map(|(i, _)| i as i64).collect();
        if !rows.is_empty() {
            let instance_set = view.call_method1("_pick_instance_set", (table,))?;
            if !instance_set.is_none() {
                return instance_set
                    .call_method1(
                        "ray_filter",
                        (rows.into_pyarray(py), triple(o), triple(d), reach),
                    )?
                    .extract();
            }
        }
        Ok(rows.into_iter().map(|i| i as usize).collect())
    }
}

fn bump(view: &Obj) -> PyResult<()> {
    let n = view
        .getattr_opt("_pick_gen")?
        .map_or(Ok(0), |v| v.extract::<u64>())?;
    view.setattr("_pick_gen", n + 1)
}

#[pyfunction]
pub fn pick_materialize(view: &Obj, near: &Obj) -> PyResult<()> {
    let py = view.py();
    let table = view.call_method0("_pick_lazy_table")?;
    let mut live = attr(view, "_pick_live")?;
    if table.is_none() {
        if live.is_truthy()? {
            live.call_method0("clear")?;
            bump(view)?;
        }
        return Ok(());
    }
    if live.is_none() {
        live = py.import("collections")?.getattr("OrderedDict")?.call0()?;
        view.setattr("_pick_live", &live)?;
    }
    if near.eq("skip")? {
        return Ok(());
    }
    let rows = PlacementTable::new(&table)?;
    let all = near.eq("all")?;
    let kind = if near.is_instance_of::<PyTuple>() {
        near.get_item(0)?
    } else {
        none(py)
    };
    let footprint = !all && !kind.eq("px")? && !kind.eq("ray")?;
    let mut need: Vec<usize> = if all {
        (0..rows.groups.len()?).collect()
    } else if kind.eq("px")? {
        let px = (
            near.get_item(1)?.extract::<f64>()?,
            near.get_item(2)?.extract::<f64>()?,
        );
        let (o, d): (Obj, Obj) = view.call_method1("_pixel_to_ray", px)?.extract()?;
        if o.is_none() || d.is_none() {
            return Ok(());
        }
        rows.ray(view, &table, &o, &d, None, Some(px))?
    } else if kind.eq("ray")? {
        let reach = if near.len()? > 3 {
            near.get_item(3)?.extract()?
        } else {
            None
        };
        rows.ray(
            view,
            &table,
            &near.get_item(1)?,
            &near.get_item(2)?,
            reach,
            None,
        )?
    } else {
        let (x, y) = if near.is_none() {
            let pos = attr(view, "_last_mouse_pos")?;
            if pos.is_none() {
                return Ok(());
            }
            (pos.call_method0("x")?, pos.call_method0("y")?)
        } else {
            (near.get_item(1)?, near.get_item(2)?)
        };
        view.call_method1("_footprint_need", (&table, x, y))?
            .extract()?
    };
    let counts = rows.counts.as_array();
    if need.iter().any(|i| *i >= counts.len()) {
        return Err(pyo3::exceptions::PyIndexError::new_err(
            "pick placement outside table",
        ));
    }
    let max: i64 = view.getattr("_PICK_LIVE_MAX_FACES")?.extract()?;
    if footprint && !need.is_empty() {
        let room = max as f64 * 0.8;
        if need.iter().map(|i| counts[*i]).sum::<i64>() as f64 > room {
            let distances = rows.distances(&need, view)?;
            let mut order: Vec<_> = need.into_iter().zip(distances).collect();
            order.sort_by(|a, b| a.1.total_cmp(&b.1));
            let mut sum = 0;
            need = Vec::new();
            for (i, _) in order {
                sum += counts[i];
                if need.is_empty() || sum as f64 <= room {
                    need.push(i);
                }
            }
        }
        let keep = PySet::empty(py)?;
        for &i in &need {
            keep.add(rows.groups.get_item(i)?.as_ptr() as usize)?;
        }
        view.setattr("_pick_foot_keep", keep)?;
    }
    let serial = attr(view, "_input_serial")?;
    let mut cap: usize = view.getattr("_PICK_NEW_PER_QUERY")?.extract()?;
    let mut budget = none(py);
    if footprint && !serial.is_none() {
        budget = attr(view, "_pick_budget")?;
        if budget.is_none() || !budget.get_item(0)?.eq(&serial)? {
            let list = PyList::empty(py);
            list.append(&serial)?;
            list.append(cap)?;
            budget = list.into_any();
            view.setattr("_pick_budget", &budget)?;
        }
        cap = budget.get_item(1)?.extract()?;
    }
    let mut new = Vec::new();
    for &i in &need {
        if !live.contains(rows.groups.get_item(i)?.as_ptr() as usize)? {
            new.push(i);
        }
    }
    if !budget.is_none() {
        budget.set_item(1, cap.saturating_sub(new.len()))?;
    }
    let mut rejected = HashSet::new();
    if new.len() > cap && !all {
        let distances = rows.distances(&new, view)?;
        // Use the same unstable ordering as the original admission policy for equal distances.
        let order: Vec<usize> = distances
            .into_pyarray(py)
            .call_method0("argsort")?
            .extract()?;
        rejected.extend(order.into_iter().skip(cap).map(|r| new[r]));
    }
    let mut changed = false;
    let mut needed = HashSet::new();
    for i in need {
        if rejected.contains(&i) {
            continue;
        }
        let g = rows.groups.get_item(i)?;
        let id = g.as_ptr() as usize;
        needed.insert(id);
        if live.contains(id)? {
            live.call_method1("move_to_end", (id,))?;
        } else {
            live.set_item(id, (g, counts[i]))?;
            changed = true;
        }
    }
    let mut faces = 0i64;
    for item in live.call_method0("values")?.try_iter()? {
        faces += item?.get_item(1)?.extract::<i64>()?;
    }
    if faces > max
        && !all
        && (serial.is_none() || faces > 2 * max || !attr(view, "_pick_evicted_at")?.eq(&serial)?)
    {
        view.setattr("_pick_evicted_at", &serial)?;
        let foot = attr(view, "_pick_foot_keep")?;
        let inst = attr(view, "_inst_chunks")?;
        let keys: Vec<usize> = live
            .try_iter()?
            .map(|k| k?.extract())
            .collect::<PyResult<_>>()?;
        for id in keys {
            if faces <= max {
                break;
            }
            if needed.contains(&id) || (!foot.is_none() && foot.contains(id)?) {
                continue;
            }
            let (gone, n): (Obj, i64) = live.call_method1("pop", (id,))?.extract()?;
            faces -= n;
            if !inst.is_none()
                && view
                    .call_method1("_instanced_eligible", (gone,))?
                    .is_truthy()?
            {
                inst.call_method1("pop", (id, py.None()))?;
            }
            changed = true;
        }
    }
    if changed {
        bump(view)?;
    }
    Ok(())
}

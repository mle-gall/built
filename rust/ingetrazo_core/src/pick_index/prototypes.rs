// SPDX-License-Identifier: GPL-3.0-or-later
//! Prototype reuse, background prewarming and lazy-row visibility.
use super::*;
use std::collections::{HashMap, HashSet};

fn host(py: Python<'_>) -> PyResult<Obj<'_>> {
    Ok(py.import("views.document_interaction")?.into_any())
}
fn native(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("core._native")?.getattr("native")
}
fn base<'py>(view: &Obj<'py>, host: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let paint = host.call_method1("_paint_of", (view, group))?;
    view.call_method1("_proto_base_chunk", (group.getattr("mesh")?, paint))
}
fn bake_key<'py>(b: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok((
        b.call_method1("get", ("uid",))?,
        b.call_method1("get", ("rev",))?,
    )
        .into_pyobject(b.py())?
        .into_any())
}

#[pyfunction]
pub fn pick_instance_set<'py>(view: &Obj<'py>, table: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let host = host(py)?;
    if !host.getattr("_INSTANCE_FILTER")?.is_truthy()? {
        return Ok(none(py));
    }
    let module = native(py)?;
    if module.is_none() || !module.hasattr("InstanceSet")? {
        return Ok(none(py));
    }
    let memo = attr(view, "_inst_set_memo")?;
    if !memo.is_none() && memo.get_item(0)?.is(table) {
        return memo.get_item(1);
    }
    host.call_method1("_pick_warm_take", (view,))?;
    let cache = attr(view, "_inst_proto_cache")?;
    let keep = PyDict::new(py);
    let protos = PyList::empty(py);
    let mut meshes = HashMap::new();
    let mut indices = Vec::new();
    let mut matrices = Vec::new();
    let mut todo = Vec::new();
    for group in table.get_item(0)?.try_iter()? {
        let g = group?;
        let mesh = g.getattr("mesh")?;
        let id = mesh.as_ptr() as usize;
        let index = if let Some(&i) = meshes.get(&id) {
            i
        } else {
            let b = base(view, &host, &g)?;
            let key = bake_key(&b)?;
            let proto = if cache.is_none() {
                none(py)
            } else {
                cache.call_method1("get", (&key,))?
            };
            let i = protos.len();
            if proto.is_none() {
                todo.push((i, key.clone(), b));
            }
            keep.set_item(key, &proto)?;
            protos.append(proto)?;
            meshes.insert(id, i);
            i
        };
        indices.push(index as i64);
        let xf: [f64; 16] = g.getattr("xform")?.call_method0("data")?.extract()?;
        matrices.extend(xf);
    }
    let build = host.getattr("_inst_proto_of")?;
    if todo.len() > 1 {
        let workers = py
            .import("os")?
            .call_method0("cpu_count")?
            .extract::<Option<usize>>()?
            .unwrap_or(2)
            .min(8);
        let pool = py
            .import("concurrent.futures")?
            .getattr("ThreadPoolExecutor")?
            .call1((workers,))?;
        let results = (|| -> PyResult<()> {
            let mut jobs = Vec::new();
            for (i, key, b) in todo {
                jobs.push((i, key, pool.call_method1("submit", (&build, &module, b))?));
            }
            for (i, key, job) in jobs {
                let p = job.call_method0("result")?;
                protos.set_item(i, &p)?;
                keep.set_item(key, p)?;
            }
            Ok(())
        })();
        pool.call_method0("shutdown")?;
        results?;
    } else if let Some((i, key, b)) = todo.pop() {
        let p = build.call1((&module, b))?;
        protos.set_item(i, &p)?;
        keep.set_item(key, p)?;
    }
    let n = indices.len();
    let np = py.import("numpy")?;
    let lo = np.call_method1("ascontiguousarray", (table.get_item(1)?, "float64"))?;
    let hi = np.call_method1("ascontiguousarray", (table.get_item(2)?, "float64"))?;
    let set = module.getattr("InstanceSet")?.call1((
        protos,
        indices.into_pyarray(py),
        matrices.into_pyarray(py).reshape([n, 16])?,
        lo,
        hi,
    ))?;
    view.setattr("_inst_proto_cache", keep)?;
    view.setattr("_inst_set_memo", (table, &set))?;
    Ok(set)
}

#[pyfunction]
pub fn start_pick_warmup(view: &Obj) -> PyResult<()> {
    let py = view.py();
    let host = host(py)?;
    if !host.getattr("_PICK_WARM")?.is_truthy()?
        || !host.getattr("_INSTANCE_FILTER")?.is_truthy()?
    {
        return Ok(());
    }
    let module = native(py)?;
    if module.is_none() || !module.hasattr("InstanceSet")? {
        return Ok(());
    }
    let table = view.call_method0("_pick_lazy_table")?;
    if table.is_none() {
        return Ok(());
    }
    let memo = attr(view, "_inst_set_memo")?;
    if !memo.is_none() && memo.get_item(0)?.is(&table) {
        return Ok(());
    }
    let mut seen = HashSet::new();
    let protos = PyList::empty(py);
    let bounds = PyList::empty(py);
    for g in table.get_item(0)?.try_iter()? {
        let g = g?;
        if seen.insert(g.getattr("mesh")?.as_ptr() as usize) {
            let b = base(view, &host, &g)?;
            bounds.append(&b)?;
            protos.append((bake_key(&b)?, b))?;
        }
    }
    let grids = PyList::empty(py);
    if !host.getattr("_NO_SPAN_GRID")?.is_truthy()? {
        let floor: usize = host.getattr("_SPAN_GRID_MIN")?.extract()?;
        let fixed = host
            .getattr("DocumentInteraction")?
            .call_method1("_pick_candidates", (view, false, &table))?
            .get_item(1)?;
        for g in fixed.try_iter()? {
            let g = g?;
            if attr(&g, "billboard")?.is_truthy()? {
                continue;
            }
            let c = view.call_method1("_group_chunk", (g,))?;
            let v0 = c.get_item("v0")?;
            if !v0.is_none() && v0.len()? > floor && !c.contains("_span_grid")? {
                grids.append(c)?;
            }
        }
    }
    view.setattr(
        "_pick_warm",
        host.getattr("_PickWarm")?
            .call1((module, protos, grids, bounds))?,
    )
}

#[pyfunction]
pub fn pick_lazy_native<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    if !host(py)?.getattr("_LAZY_NATIVE")?.is_truthy()? {
        return Ok(none(py));
    }
    let table = view.call_method0("_pick_lazy_table")?;
    if table.is_none() {
        return Ok(none(py));
    }
    let set = view.call_method1("_pick_instance_set", (&table,))?;
    if set.is_none() || !set.hasattr("corners_near")? {
        return Ok(none(py));
    }
    let hidden = match view.getattr_opt("_rest_is_hidden")? {
        Some(f) if f.is_callable() => f.call0()?.is_truthy()?,
        _ => false,
    };
    let scene = view.getattr("scene")?;
    let key = (
        view.call_method0("_placements_epoch")?,
        scene.getattr("edit_group")?.as_ptr() as usize,
        hidden,
        table.as_ptr() as usize,
    )
        .into_pyobject(py)?;
    let mut candidates = attr(view, "_pick_cands_memo")?;
    if candidates.is_none() || !candidates.get_item(0)?.eq(&key)? {
        let kw = PyDict::new(py);
        kw.set_item("near", "skip")?;
        view.call_method("_pick_index", (), Some(&kw))?;
        candidates = attr(view, "_pick_cands_memo")?;
        if candidates.is_none() || !candidates.get_item(0)?.eq(&key)? {
            return Ok(none(py));
        }
    }
    let memo = attr(view, "_lazy_native_memo")?;
    if !memo.is_none() && memo.get_item(0)?.is(&candidates) && memo.get_item(1)?.is(&set) {
        return memo.get_item(2);
    }
    let mut lookup = HashMap::new();
    for (i, g) in table.get_item(0)?.try_iter()?.enumerate() {
        lookup.insert(g?.as_ptr() as usize, i as i64);
    }
    let mut positions = Vec::new();
    for v in candidates.get_item(3)?.call_method0("values")?.try_iter()? {
        positions.push(v?.extract::<(usize, Obj)>()?);
    }
    positions.sort_by_key(|r| r.0);
    let mut snap = Vec::new();
    let mut vis = Vec::new();
    for (_, g) in positions {
        let row = *lookup
            .get(&(g.as_ptr() as usize))
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("missing lazy placement"))?;
        if scene.call_method1("group_snappable", (&g,))?.is_truthy()? {
            snap.push(row)
        }
        if scene.call_method1("entity_visible", (&g,))?.is_truthy()? {
            vis.push(row)
        }
    }
    let out = (&set, snap.into_pyarray(py), vis.into_pyarray(py))
        .into_pyobject(py)?
        .into_any();
    view.setattr("_lazy_native_memo", (candidates, set, &out))?;
    Ok(out)
}

#[pyfunction]
pub fn pick_lazy_selectable<'py>(view: &Obj<'py>, nat: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let memo = attr(view, "_lazy_sel_memo")?;
    if !memo.is_none() && memo.get_item(0)?.is(nat) {
        return memo.get_item(1);
    }
    let editable = view.getattr("_pick_cands_memo")?.get_item(1)?;
    let groups = view.call_method0("_pick_lazy_table")?.get_item(0)?;
    let scene = view.getattr("scene")?;
    let mut selected = Vec::new();
    for i in nat.get_item(1)?.extract::<Vec<usize>>()? {
        let g = groups.get_item(i)?;
        if editable.contains(g.as_ptr() as usize)?
            && scene.call_method1("entity_selectable", (g,))?.is_truthy()?
        {
            selected.push(i as i64);
        }
    }
    let out = selected.into_pyarray(py).into_any();
    view.setattr("_lazy_sel_memo", (nat, &out))?;
    Ok(out)
}

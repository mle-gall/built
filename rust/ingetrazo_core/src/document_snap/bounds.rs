// SPDX-License-Identifier: GPL-3.0-or-later
//! Oriented reference boxes and prototype points, invalidated by edit level.
use super::*;
use pyo3::types::PySet;
#[pyfunction]
fn editing_meshes<'py>(view: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    let py = view.py();
    let scene = view.getattr("scene")?;
    let ids = PySet::empty(py)?;
    ids.add(id(&scene.getattr("mesh")?))?;
    for level in collection(&scene, "_edit_stack")?.try_iter()? {
        let level = level?;
        if let Ok(dict) = level.cast::<PyDict>() {
            if let Some(share) = dict
                .get_item("share")?
                .filter(|s| s.is_truthy().unwrap_or(false))
            {
                ids.add(id(&share.get_item(1)?))?;
                ids.add(id(&share.get_item(0)?.getattr("mesh")?))?
            }
        }
    }
    Ok(ids)
}
fn descendants<'py>(group: &Obj<'py>) -> PyResult<Obj<'py>> {
    group
        .py()
        .import("ingetrazo_core")?
        .call_method1("purge_iter_groups", ((group,),))
}
#[pyfunction]
fn proto_points_cache<'py>(view: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let store = dict_slot(view, "_proto_pts_store")?;
    let editing = view.call_method0("_editing_meshes")?;
    let valid = PyDict::new(py);
    let deferred = py.import("ingetrazo_core.mesh")?.getattr("DeferredMesh")?;
    for g in descendants(group)?.try_iter()? {
        let mesh = g?.getattr("mesh")?;
        let k = id(&mesh);
        if mesh.is_none() || valid.contains(k)? {
            continue;
        }
        let lazy = mesh.get_type().is(&deferred);
        if !lazy && mesh.getattr("vertices")?.len()? == 0 {
            continue;
        }
        let hit = store.call_method1("get", (k,))?;
        if hit.is_none() {
            continue;
        }
        let arr = hit.get_item(1)?;
        if editing.contains(k)?
            || !opt(&mesh, "_mut_serial")?.eq(hit.get_item(0)?)?
            || (!lazy && arr.len()? != mesh.getattr("vertices")?.len()?)
        {
            store.del_item(k)?
        } else {
            valid.set_item(k, arr)?
        }
    }
    Ok(valid.into_any())
}
#[pyfunction]
fn proto_points_store(view: &Obj, group: &Obj, protos: &Bound<PyDict>) -> PyResult<()> {
    let py = view.py();
    let store = view.getattr("_proto_pts_store")?;
    if store.len()? > 512 {
        store.call_method0("clear")?;
    }
    let editing = view.call_method0("_editing_meshes")?;
    let serials = PyDict::new(py);
    for g in descendants(group)?.try_iter()? {
        let m = g?.getattr("mesh")?;
        if !m.is_none() {
            serials.set_item(id(&m), opt(&m, "_mut_serial")?)?
        }
    }
    for (k, a) in protos.iter() {
        if !editing.contains(&k)? {
            store.set_item(&k, (serials.get_item(&k)?, a))?
        }
    }
    Ok(())
}
#[pyfunction]
fn compute_obb<'py>(group: &Obj<'py>, protos: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = group.py();
    let lib = py.import("core.group")?;
    let pos = lib.call_method1("placement_points", (group, protos))?;
    let frame = lib.call_method1("group_frame", (group,))?;
    let kw = kw(py, "points", &pos)?;
    let bounds = lib.getattr("oriented_bounds")?;
    if !frame.is_none() {
        let axes = lib.call_method1("frame_axes", (frame,))?;
        return bounds.call(
            (
                py.None(),
                (axes.get_item(1)?, axes.get_item(2)?, axes.get_item(3)?),
            ),
            Some(&kw),
        );
    }
    let world = bounds.call((py.None(),), Some(&kw))?;
    let axes = lib.call_method1(
        "frame_from_points",
        (if pos.len()? > 0 { pos } else { none(py) },),
    )?;
    let own = bounds.call((py.None(), axes), Some(&kw))?;
    let volume = |b: &Obj| -> PyResult<f64> {
        let lo: [f64; 3] = b.get_item(1)?.extract()?;
        let hi: [f64; 3] = b.get_item(2)?.extract()?;
        let mut sides = std::array::from_fn::<_, 3, _>(|i| hi[i] - lo[i]);
        sides.sort_by(f64::total_cmp);
        Ok(sides[1] * sides[2] * sides[0].max(1e-6))
    };
    Ok(if volume(&own)? < 0.9 * volume(&world)? {
        own
    } else {
        world
    })
}
#[pyfunction]
fn group_obb<'py>(view: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let h = host(py)?;
    let gid = id(group);
    let kids = opt(group, "children")?;
    if kids.is_truthy()? {
        let memo = dict_slot(view, "_container_obb")?;
        let key = h.call_method1("_content_ver", (view,))?;
        let old = memo.call_method1("get", (gid,))?;
        if !old.is_none() && old.get_item(0)?.eq(&key)? {
            return old.get_item(1);
        }
        let fetch = view.getattr_opt("_proto_points_cache")?;
        let protos = match &fetch {
            Some(f) => f.call1((group,))?,
            None => none(py),
        };
        let obb = view.call_method1("_compute_obb", (group, &protos))?;
        if fetch.is_some() {
            view.call_method1("_proto_points_store", (group, protos))?;
        }
        if memo.len()? > 256 {
            memo.call_method0("clear")?;
        }
        memo.set_item(gid, (key, &obb))?;
        return Ok(obb);
    }
    let as_tuple = py.import("builtins")?.getattr("tuple")?;
    let f = py
        .import("core.group")?
        .call_method1("group_frame", (group,))?;
    let fk = if f.is_none() {
        none(py)
    } else {
        as_tuple.call1((f.call_method0("data")?,))?
    };
    let xf = opt(group, "xform")?;
    let chunks = opt(view, "_inst_chunks")?;
    if !xf.is_none() && (!chunks.is_truthy()? || !chunks.contains(gid)?) {
        let proto = h.call_method1(
            "_proto_wrapper",
            (
                view,
                group.getattr("mesh")?,
                h.call_method1("_paint_of", (view, group))?,
            ),
        )?;
        let base = view.call_method1("_group_chunk", (proto,))?;
        let key = (
            base.get_item("uid")?,
            as_tuple.call1((xf.call_method0("data")?,))?,
            base.call_method1("get", ("rev",))?,
            fk,
        )
            .into_pyobject(py)?;
        let memo = dict_slot(view, "_inst_obb")?;
        let old = memo.call_method1("get", (gid,))?;
        if !old.is_none() && old.get_item(0)?.eq(&key)? {
            return old.get_item(1);
        }
        let obb = view.call_method1("_compute_obb", (group,))?;
        if memo.len()? > 65536 {
            memo.call_method0("clear")?;
        }
        memo.set_item(gid, (key, &obb))?;
        return Ok(obb);
    }
    let entry = view.call_method1("_group_chunk", (group,))?;
    let old = entry.call_method1("get", ("obb",))?;
    if !old.is_none() && entry.call_method1("get", ("obb_frame",))?.eq(&fk)? {
        return Ok(old);
    }
    let obb = view.call_method1("_compute_obb", (group,))?;
    entry.set_item("obb", &obb)?;
    entry.set_item("obb_frame", fk)?;
    Ok(obb)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(editing_meshes, m)?)?;
    m.add_function(wrap_pyfunction!(proto_points_cache, m)?)?;
    m.add_function(wrap_pyfunction!(proto_points_store, m)?)?;
    m.add_function(wrap_pyfunction!(compute_obb, m)?)?;
    m.add_function(wrap_pyfunction!(group_obb, m)?)?;
    Ok(())
}

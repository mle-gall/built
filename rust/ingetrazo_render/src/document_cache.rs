// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry shared by views of one document and per-view cache lifetimes.
use crate::document_host::*;
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple};

#[pyfunction]
fn bake_store_of<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let h = host(py)?;
    let stores = h.getattr("_BAKE_STORES")?;
    let key = id(scene);
    let old = stores.call_method1("get", (key,))?;
    if !old.is_none() && old.get_item(0)?.call0()?.is(scene) {
        return old.get_item(1);
    }
    let weak = h.getattr("weakref")?;
    let r = match weak.call_method1("ref", (scene,)) {
        Ok(r) => r,
        Err(e) if e.is_instance_of::<PyTypeError>(py) => return Ok(none(py)),
        Err(e) => return Err(e),
    };
    let store = PyDict::new(py);
    stores.set_item(key, (&r, &store))?;
    weak.call_method1("finalize", (scene, h.getattr("_drop_bake_store")?, key, r))?;
    Ok(store.into_any())
}
#[pyfunction]
fn drop_bake_store(key: &Obj, r: &Obj) -> PyResult<()> {
    let stores = host(key.py())?.getattr("_BAKE_STORES")?;
    let old = stores.call_method1("get", (key,))?;
    if !old.is_none() && old.get_item(0)?.is(r) {
        stores.del_item(key)?;
    }
    Ok(())
}
#[pyfunction]
fn bind_bake_store(v: &Obj) -> PyResult<()> {
    let py = v.py();
    let h = host(py)?;
    let mut store = bake_store_of(&opt(v, "scene")?)?;
    if store.is_none() {
        store = PyDict::new(py).into_any();
    }
    for k in h.getattr("_SHARED_BAKES")?.try_iter()? {
        let k = k?.extract::<String>()?;
        v.setattr(&k, store.call_method1("setdefault", (&k, PyDict::new(py)))?)?;
    }
    if let Err(e) = h.getattr("_LIVE_VIEWS")?.call_method1("add", (v,)) {
        if !e.is_instance_of::<PyTypeError>(py) {
            return Err(e);
        }
    }
    Ok(())
}
#[pyfunction]
fn several_views_shown(py: Python) -> PyResult<bool> {
    let views = host(py)?
        .getattr("_LIVE_VIEWS")?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?;
    let mut shown = 0;
    for v in views {
        match v.call_method0("isVisible") {
            Ok(x) if x.is_truthy()? => {
                shown += 1;
                if shown == 2 {
                    return Ok(true);
                }
            }
            Err(e) if !e.is_instance_of::<PyRuntimeError>(py) => return Err(e),
            _ => {}
        }
    }
    Ok(false)
}
#[pyfunction]
fn decoded_texture<'py>(py: Python<'py>, path: &Obj<'py>) -> PyResult<Obj<'py>> {
    let h = host(py)?;
    let cache = h.getattr("_DECODED_TEXTURES")?.cast_into::<PyDict>()?;
    let stamp = match py.import("os")?.call_method1("stat", (path,)) {
        Ok(s) => (s.getattr("st_mtime_ns")?, s.getattr("st_size")?)
            .into_pyobject(py)?
            .into_any(),
        Err(e) if e.is_instance_of::<PyOSError>(py) => none(py),
        Err(e) => return Err(e),
    };
    if let Some(old) = cache.get_item(path)? {
        if old.get_item(0)?.eq(&stamp)? {
            cache.del_item(path)?;
            cache.set_item(path, &old)?;
            return old.get_item(1);
        }
    }
    let cls = h.getattr("QImage")?;
    let img = cls.call1((path,))?;
    cache.call_method1("pop", (path, py.None()))?;
    if img.call_method0("isNull")?.is_truthy()? {
        return Ok(none(py));
    }
    let mut cutout = false;
    if img.call_method0("hasAlphaChannel")?.is_truthy()? {
        let a = img
            .call_method1("scaled", (64, 64))?
            .call_method1("convertToFormat", (cls.getattr("Format_Alpha8")?,))?;
        let bytes = py
            .import("builtins")?
            .call_method1("bytes", (a.call_method0("constBits")?,))?
            .cast_into::<PyBytes>()?;
        let n = a.call_method0("height")?.extract::<usize>()?
            * a.call_method0("bytesPerLine")?.extract::<usize>()?;
        let a = bytes
            .as_bytes()
            .get(..n)
            .ok_or_else(|| PyRuntimeError::new_err("short alpha image"))?;
        cutout = !a.is_empty() && a.iter().filter(|&&x| x > 128).count() * 5 < 3 * n;
    }
    let out = (img.call_method0("mirrored")?, cutout).into_pyobject(py)?;
    if !h.call_method0("_several_views_shown")?.is_truthy()? {
        cache.clear();
        return Ok(out.into_any());
    }
    cache.set_item(path, (stamp, &out))?;
    let mut total = 0;
    let mut sizes = Vec::with_capacity(cache.len());
    for (k, v) in cache.iter() {
        let size = v
            .get_item(1)?
            .get_item(0)?
            .call_method0("sizeInBytes")?
            .extract::<usize>()?;
        total += size;
        sizes.push((k, size));
    }
    let budget = h.getattr("_DECODED_BUDGET")?.extract::<usize>()?;
    for (k, size) in sizes {
        if total <= budget || k.eq(path)? {
            break;
        }
        cache.del_item(k)?;
        total -= size;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn reset_document_caches(v: &Obj, keep_bakes: bool) -> PyResult<()> {
    let py = v.py();
    let scene = v.getattr("scene")?;
    for k in [
        "view_filter",
        "view_tint",
        "active_view_key",
        "_view_expression",
        "_view_color",
        "_pick_warm",
        "_inst_obb",
        "_frozen_cache_version",
        "_selected_placements_cache",
        "_selected_profile_key",
        "_sil_table",
        "_pick_cat_memo",
        "_pick_te_memo",
        "_pick_loose_memo",
        "_lazy_native_memo",
        "_static_occl_memo",
        "_sil_groups",
        "_billboard_groups",
        "_placements_memo",
        "_epoch_same",
        "_pick_lazy_memo",
        "_pick_near_memo",
        "_edge_near_memo",
        "_proto_draw_fast",
        "_arc_mid_cache",
        "_epoch_hist",
        "_kept_tops_memo",
    ] {
        v.setattr(k, py.None())?;
    }
    for k in ["_navigator_allowed", "_navigator_view_key"] {
        scene.setattr(k, py.None())?;
    }
    v.setattr("_previous_views", PyList::empty(py))?;
    v.setattr("_view_isolate", true)?;
    v.setattr("_view_exact", false)?;
    v.setattr("_pick_warm_due", false)?;
    v.setattr("_edges_version", -1)?;
    let shared = host(py)?.getattr("_SHARED_BAKES")?;
    for name in v.getattr("_DOCUMENT_CACHES")?.try_iter()? {
        let name = name?;
        if keep_bakes && shared.contains(&name)? {
            continue;
        }
        let c = opt(v, &name.extract::<String>()?)?;
        if c.is_instance_of::<PyDict>() {
            c.call_method0("clear")?;
        }
    }
    Ok(())
}
#[pyclass(frozen)]
struct StaticGroup {
    viewport: Py<PyAny>,
}
#[pymethods]
impl StaticGroup {
    fn __call__(&self, g: &Obj) -> PyResult<bool> {
        let v = self.viewport.bind(g.py());
        Ok(v.getattr("scene")?
            .call_method1("entity_visible", (g,))?
            .is_truthy()?
            && !opt(g, "billboard")?.is_truthy()?
            && !v.call_method1("_instanced_eligible", (g,))?.is_truthy()?)
    }
}
#[pyfunction]
fn static_draw_groups<'py>(v: &Obj<'py>, groups: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let epoch_of = opt(v, "_placements_epoch")?;
    let test = StaticGroup {
        viewport: v.clone().unbind(),
    };
    let filter = |test: &StaticGroup| -> PyResult<Obj<'py>> {
        let out = PyList::empty(py);
        for g in groups.try_iter()? {
            let g = g?;
            if test.__call__(&g)? {
                out.append(g)?;
            }
        }
        Ok(out.into_any())
    };
    if epoch_of.is_none() {
        return filter(&test);
    }
    let epoch = epoch_of.call0()?;
    let ids = PyTuple::new(
        py,
        groups
            .try_iter()?
            .map(|g| g.map(|g| id(&g)))
            .collect::<PyResult<Vec<_>>>()?,
    )?;
    let mesh = v.getattr("scene")?.getattr("mesh")?;
    let memo = opt(v, "_static_groups_memo")?;
    let mut out = none(py);
    if !memo.is_none() {
        let old = memo.get_item(0)?;
        let previous = memo.get_item(3)?;
        let old_ids = memo.get_item(1)?;
        let same_mesh = memo.get_item(2)?.is(&mesh);
        if old_ids.eq(&ids)?
            && same_mesh
            && (old.eq(&epoch)?
                || v.call_method1("_epoch_only_matrices", (&old, &epoch))?
                    .is_truthy()?)
        {
            out = previous;
        } else {
            let kw = PyDict::new(py);
            kw.set_item("ctx", !same_mesh)?;
            out = v.call_method(
                "_refilter",
                (
                    old,
                    previous,
                    groups,
                    Py::new(
                        py,
                        StaticGroup {
                            viewport: v.clone().unbind(),
                        },
                    )?,
                    old_ids,
                ),
                Some(&kw),
            )?;
        }
    }
    if out.is_none() {
        out = filter(&test)?;
    }
    v.setattr("_static_groups_memo", (epoch, ids, mesh, &out))?;
    Ok(out)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(bake_store_of, m)?)?;
    m.add_function(wrap_pyfunction!(bind_bake_store, m)?)?;
    m.add_function(wrap_pyfunction!(drop_bake_store, m)?)?;
    m.add_function(wrap_pyfunction!(several_views_shown, m)?)?;
    m.add_function(wrap_pyfunction!(decoded_texture, m)?)?;

    m.add_function(wrap_pyfunction!(reset_document_caches, m)?)?;
    m.add_function(wrap_pyfunction!(static_draw_groups, m)?)?;
    Ok(())
}

// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn prune_stale_selection(v: &Obj) -> PyResult<()> {
    let py = v.py();
    let scene = v.getattr("scene")?;
    let selection = scene.getattr("selection")?;
    let hover = opt(v, "_hover_entity")?;
    let m = host(py)?;
    let edge = m.getattr("Edge")?;
    let face = m.getattr("Face")?;
    // A large selection checked against the active mesh's live keys in
    // one store call: building the identity sets of every face and edge
    // was 600k calls on each scene notification (a select-all, an edit).
    let store = opt(&scene.getattr("mesh")?, "_store")?;
    let fast = selection.hasattr("element_keys")? && selection.hasattr("discard_identities")? && store.hasattr("stale_keys")?;
    let elements = if fast {
        let counts = selection.call_method0("snapshot")?;
        counts.get_item(2)?.extract::<usize>()? + counts.get_item(3)?.extract::<usize>()?
    } else {
        0
    };
    if elements > 64 {
        let stale = PyList::empty(py);
        for kind in ["edge", "face"] {
            let keys = selection.call_method1("element_keys", (kind,))?;
            if keys.len()? > 0 {
                stale.call_method1("extend", (store.call_method1("stale_keys", (kind, keys))?,))?;
            }
        }
        if !stale.is_empty() {
            selection.call_method1("discard_identities", (&stale,))?;
        }
        if hover.is_instance(&edge)? || hover.is_instance(&face)? {
            let kind = if hover.is_instance(&edge)? { "edge" } else { "face" };
            let gone = store.call_method1("stale_keys", (kind, PyList::new(py, [hover.getattr("key")?])?))?;
            if gone.len()? > 0 {
                v.setattr("_hover_entity", py.None())?
            }
        }
        return Ok(());
    }
    let mut candidates = Vec::new();
    if hover.is_instance(&edge)? || hover.is_instance(&face)? {
        candidates.push((hover, true))
    }
    for ent in selection.try_iter()? {
        let ent = ent?;
        if ent.is_instance(&edge)? || ent.is_instance(&face)? {
            candidates.push((ent, false))
        }
    }
    if candidates.is_empty() {
        return Ok(());
    }
    let mut stale = Vec::new();
    if candidates.len() > 64 {
        let mesh = scene.getattr("mesh")?;
        let identity = m.getattr("element_identity")?;
        let keys = |name| -> PyResult<Bound<PySet>> {
            let values = PySet::empty(py)?;
            for e in mesh.getattr(name)?.try_iter()? {
                values.add(identity.call1((e?,))?)?
            }
            Ok(values)
        };
        let edges = keys("edges")?;
        let faces = keys("faces")?;
        for (ent, h) in candidates {
            let live = if ent.is_instance(&edge)? {
                &edges
            } else {
                &faces
            };
            if !live.contains(ent.getattr("key")?)? {
                stale.push((ent, h))
            }
        }
    } else {
        let active = scene.getattr("mesh")?;
        let mut meshes = Vec::new();
        for g in v.call_method0("_placements")?.try_iter()? {
            let g = g?;
            if scene.call_method1("entity_visible", (&g,))?.is_truthy()? && !truth(&g, "billboard")?
            {
                let mesh = g.getattr("mesh")?;
                if !mesh.getattr("__dict__")?.contains("_deferred")? {
                    meshes.push(mesh)
                }
            }
        }
        meshes.sort_by_key(|m| !m.is(&active));
        let loose = scene.getattr("loose_mesh")?;
        for (ent, h) in candidates {
            let key = if ent.is_instance(&edge)? {
                "edges"
            } else {
                "faces"
            };
            let live = if loose.getattr(key)?.contains(&ent)? {
                scene.call_method1("entity_visible", (&ent,))?.is_truthy()?
                    || scene
                        .call_method1("entity_selectable", (&ent,))?
                        .is_truthy()?
            } else {
                let mut found = false;
                for mesh in &meshes {
                    if mesh.getattr(key)?.contains(&ent)? {
                        found = true;
                        break;
                    }
                }
                found
            };
            if !live {
                stale.push((ent, h))
            }
        }
    }
    for (ent, h) in stale {
        if h {
            v.setattr("_hover_entity", py.None())?
        } else {
            selection.call_method1("discard", (ent,))?;
        }
    }
    Ok(())
}
fn advance_preview(v: &Obj) -> PyResult<()> {
    let n = v
        .getattr_opt("_preview_epoch")?
        .map(|n| n.extract::<i64>())
        .transpose()?
        .unwrap_or(0);
    v.setattr("_preview_epoch", n + 1)?;
    v.setattr("_preview_offset", vec3(v.py(), DVec3::ZERO)?)
}
#[pyfunction]
fn begin_groups_preview(v: &Obj, groups: &Obj, external: bool) -> PyResult<()> {
    let py = v.py();
    let scene = v.getattr("scene")?;
    let movers = PyList::empty(py);
    let paint = scene.call_method0("open_group_paint")?;
    v.call_method0("_placements")?;
    let kw = PyDict::new(py);
    kw.set_item("above", paint)?;
    for g in groups.try_iter()? {
        v.call_method("_expand_placements", (g?, &movers), Some(&kw))?;
    }
    v.setattr("_preview_external", external)?;
    if !external {
        v.setattr("_frozen_cache_version", scene.getattr("version")?)?;
        v.setattr("_edges_version", -1)?
    }
    v.setattr(
        "_preview_groups",
        PySet::new(py, movers.iter().map(|o| id(&o)))?,
    )?;
    advance_preview(v)
}
#[pyfunction]
fn preview_chunk<'py>(v: &Obj<'py>, g: &Obj<'py>) -> PyResult<Obj<'py>> {
    let dict = opt(v, "_placement_proxies")?;
    let placement = if dict.is_truthy()? {
        dict.call_method1("get", (id(g), g))?
    } else {
        g.clone()
    };
    v.call_method1("_group_chunk", (placement,))
}
#[pyfunction]
fn end_groups_preview(v: &Obj) -> PyResult<()> {
    let py = v.py();
    if !truth(v, "_preview_external")? {
        v.setattr("_frozen_cache_version", py.None())?;
        v.setattr("_edges_version", -1)?
    } else {
        let groups = v
            .call_method0("_placements")?
            .try_iter()?
            .map(|g| g.map(|g| id(&g)))
            .collect::<PyResult<std::collections::HashSet<_>>>()?;
        let old = v
            .getattr("_preview_groups")?
            .extract::<std::collections::HashSet<usize>>()?;
        for name in ["_group_chunks", "_inst_chunks"] {
            let cache = opt(v, name)?;
            if cache.is_truthy()? {
                for k in old.difference(&groups) {
                    cache.call_method1("pop", (*k, py.None()))?;
                }
            }
        }
    }
    v.setattr("_preview_external", false)?;
    v.setattr("_preview_groups", PySet::empty(py)?)?;
    advance_preview(v)?;
    v.setattr("_preview_matrix", py.None())?;
    v.setattr("_pv_edges_count", 0)?;
    v.setattr("_pv_vcol_count", 0)?;
    v.setattr("_pv_tex_runs", PyList::empty(py))
}
#[pyfunction]
fn groups_preview_transform<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let matrix = v.py().import("built_core")?.getattr("Matrix4")?;
    let xf = opt(v, "_preview_matrix")?;
    if !xf.is_none() {
        return matrix.call1((xf,));
    }
    let out = matrix.call0()?;
    let off = opt(v, "_preview_offset")?;
    if !off.is_none() {
        out.call_method1("translate", (off,))?;
    }
    Ok(out)
}
#[pyfunction]
fn set_edit_rest_mode(v: &Obj, mode: &Obj) -> PyResult<()> {
    if !host(v.py())?.getattr("EDIT_REST_MODES")?.contains(mode)?
        || v.getattr("_edit_rest_mode")?.eq(mode)?
    {
        return Ok(());
    }
    v.setattr("_edit_rest_mode", mode)?;
    setting(v, "display/edit_rest_mode", mode)?;
    v.setattr("_edges_version", -1)?;
    v.call_method0("update")?;
    Ok(())
}
#[pyfunction]
fn set_edit_rest_fade(v: &Obj, strength: f64) -> PyResult<()> {
    let s = if strength.is_nan() {
        1.
    } else {
        strength.clamp(0., 1.)
    };
    if (s - v.getattr("edit_rest_fade")?.extract::<f64>()?).abs() <= 1e-9 {
        return Ok(());
    }
    v.setattr("_edit_rest_fade_strength", s)?;
    setting(
        v,
        "display/edit_rest_fade",
        s.into_pyobject(v.py())?.as_any(),
    )?;
    v.call_method0("update")?;
    Ok(())
}
#[pyfunction]
fn set_hide_similar_components(v: &Obj, on: &Obj) -> PyResult<()> {
    let py = v.py();
    let on = on.is_truthy()?;
    setting(
        v,
        "display/hide_similar_components",
        if on { "1" } else { "0" }.into_pyobject(py)?.as_any(),
    )?;
    let sc = v.getattr("scene")?;
    if truth(&sc, "hide_similar_components")? == on {
        return Ok(());
    }
    sc.setattr("hide_similar_components", on)?;
    if on {
        let sel = sc.getattr("selection")?;
        let ents = sel.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        for e in ents {
            if sc.call_method1("similar_hidden", (&e,))?.is_truthy()? {
                sel.call_method1("discard", (e,))?;
            }
        }
    }
    sc.setattr("version", sc.getattr("version")?.add(1)?)?;
    v.setattr("_edges_version", -1)?;
    v.call_method0("update")?;
    Ok(())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(prune_stale_selection, m)?)?;
    m.add_function(wrap_pyfunction!(begin_groups_preview, m)?)?;
    m.add_function(wrap_pyfunction!(preview_chunk, m)?)?;
    m.add_function(wrap_pyfunction!(end_groups_preview, m)?)?;
    m.add_function(wrap_pyfunction!(groups_preview_transform, m)?)?;
    m.add_function(wrap_pyfunction!(set_edit_rest_mode, m)?)?;
    m.add_function(wrap_pyfunction!(set_edit_rest_fade, m)?)?;
    m.add_function(wrap_pyfunction!(set_hide_similar_components, m)?)?;
    Ok(())
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! `MetalView`: making a scene resident (the row delta or a full pack), the
//! snap-index job, the upload, the surface size and the scene camera.
use super::metal_view::{attr, base, index_worker, truthy};
use super::*;
use pyo3::types::PyCFunction;

fn perf_counter(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("perf_counter")?.extract()
}
fn later<'py>(view: &Obj<'py>, delay: u32, callback: Obj<'py>) -> PyResult<()> {
    view.py().import("built_core.timers")?.call_method1("later", (view, delay, callback))?;
    Ok(())
}
fn any_of<'py>(py: Python<'py>, values: &Obj<'py>) -> PyResult<bool> {
    py.import("builtins")?.call_method1("any", (values,))?.is_truthy()
}
fn own<'py>(view: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    Ok(view.getattr("__dict__")?.cast_into::<PyDict>()?)
}
fn billboards<'py>(py: Python<'py>) -> Reply<'py> {
    py.import("views.wgpu_billboards")?.getattr("Billboards")
}
/// A callable for timers and workers: it owns what it captures until it runs.
fn callback<F>(py: Python<'_>, run: F) -> PyResult<Bound<'_, PyCFunction>>
where
    F: Fn(Python<'_>) -> PyResult<()> + Send + Sync + 'static,
{
    PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| run(args.py()))
}
fn cancel_index_build(view: &Obj<'_>) -> PyResult<()> {
    let previous = attr(view, "_inference_future")?;
    if !previous.is_none() {
        previous.call_method0("cancel")?;
        view.setattr("_inference_future", view.py().None())?;
    }
    Ok(())
}

/// Make `scene` resident. `progress(fraction, text)` reports the pack;
/// `opening` (a document coming in, its bar still up) leaves the big
/// containers' edit boxes until after the first picture. The native snap
/// build overlaps upload on a worker.
fn set_scene<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, scene, progress, opening, focus, prepared) =
        (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?.is_truthy()?, a.get_item(4)?, a.get_item(5)?);
    let np = py.import("numpy")?;
    view.call_method0("_observe_tool_source")?;
    if opening {
        // What the window runs after the first picture (see above).
        view.setattr("after_open", PyList::empty(py))?;
    }
    let same_document = scene.is(&attr(&view, "scene")?);
    if !same_document {
        view.setattr("_source_pose", py.None())?;
    }
    view.setattr("scene", &scene)?;
    let allowed = attr(&scene, "_navigator_allowed")?;
    let key = scene.call_method0("_bounds_key")?;
    let (version, stack, similar) = (key.get_item(0)?, key.get_item(1)?, key.get_item(3)?);
    // A new visible layer (e.g. the first guide) changes no resident row.
    let hidden_layers = PyList::empty(py);
    for layer in key.get_item(4)?.try_iter()? {
        let layer = layer?;
        if !layer.get_item(1)?.is_truthy()? {
            hidden_layers.append((layer.get_item(0)?, layer.get_item(1)?))?;
        }
    }
    let resident_version = version.sub(scene.getattr("_placement_context_bumps")?)?;
    let stack = if similar.is_truthy()? { stack } else { PyTuple::empty(py).into_any() };
    let token = (
        scene.as_ptr() as usize,
        (&resident_version, stack, &similar, hidden_layers.to_tuple()),
        if allowed.is_none() { none(py) } else { py.import("builtins")?.call_method1("frozenset", (&allowed,))? },
    )
        .into_bound_py_any(py)?;
    let paint_key = py.import("views.wgpu_view")?.call_method1("_paint_key", (&scene,))?;
    let resident_copies = match view.getattr_opt("_resident_edit_copies")? {
        Some(copies) => copies,
        None => PyTuple::empty(py).into_any(),
    };
    let mut restored = Vec::new();
    for entry in resident_copies.try_iter()? {
        let entry = entry?;
        let group = entry.get_item(0)?;
        if !group.getattr("mesh")?.is(&entry.get_item(1)?) {
            restored.push(group);
        }
    }
    let editing = scene.getattr("_edit_stack")?.is_truthy()?;
    let previous_token = attr(&view, "_content_token")?;
    let previous_paint = attr(&view, "_paint_key")?;
    if focus.is_none() && restored.is_empty() && token.eq(&previous_token)? && paint_key.eq(&previous_paint)? {
        let tracker = attr(&view, "_instance_updates")?;
        if !tracker.is_none() {
            // Only the context bump was skipped.
            tracker.setattr("version", &resident_version)?;
            if editing {
                view.call_method1("_reserve_edit_room", (&tracker,))?;
            }
        }
        view.call_method0("update")?;
        return Ok(none(py));
    }
    let old_billboards = attr(&view, "_billboards")?;
    if !old_billboards.is_none() {
        old_billboards.call_method0("restore")?;
    }
    // Colour by tag toggled, or a tag's colour or look changed: the
    // per-face colours are baked into the resident vertices, so the scene
    // packs again (the row delta below only moves placements).
    let repaint = !paint_key.eq(&previous_paint)?;
    let copies = PyList::empty(py);
    for level in scene.getattr("_edit_stack")?.try_iter()? {
        let level = level?;
        if level.call_method1("get", ("world_copy",))?.is_truthy()? {
            copies.append((level.get_item("group")?, level.get_item("world_copy")?))?;
        }
    }
    view.setattr("_resident_edit_copies", copies)?;
    view.setattr("_selection_rows", PyDict::new(py))?;
    view.setattr("_selection_signature", py.None())?;
    let kept = |name: &str| -> Reply<'py> {
        Ok(match own(&view)?.get_item(name)? {
            Some(kept) => kept,
            None => PyDict::new(py).into_any(),
        })
    };
    let (kept_boxes, kept_box_rows) = (kept("_container_boxes")?, kept("_container_box_rows")?);
    view.setattr("_container_boxes", PyDict::new(py))?;
    view.setattr("_container_box_rows", PyDict::new(py))?;
    view.setattr("_pick_frame_dirty", true)?;
    let start = perf_counter(py)?;
    let tracker = attr(&view, "_instance_updates")?;
    let history = attr(&view.getattr("source_viewport")?, "history")?;
    if !tracker.is_none() {
        let visibility_changed = !previous_token.is_none() && {
            let tail = |token: &Obj<'py>| token.get_item(1)?.get_item(pyo3::types::PySlice::new(py, 1, isize::MAX, 1));
            !tail(&previous_token)?.eq(tail(&token)?)? || !previous_token.get_item(2)?.eq(token.get_item(2)?)?
        };
        // A group left on its definition again (or its world copy) is a
        // swap of one row when the tracker knows the path it was opened on;
        // anything else rescans every placement.
        let mut swapped = true;
        if !visibility_changed {
            for group in &restored {
                if !tracker.call_method1("queue_swap", (group,))?.is_truthy()? {
                    swapped = false;
                    break;
                }
            }
        }
        if visibility_changed || !swapped {
            tracker.setattr("journal_valid", false)?;
        }
        tracker.call_method0("remember_edit_path")?;
        tracker.call_method1("observe", (&history,))?;
    }
    let delta = if prepared.is_none() && !tracker.is_none() && view.getattr("packed")?.getattr("instances")?.len()? != 0 && !repaint && focus.is_none() {
        tracker.call_method1("diff", (&scene,))?
    } else {
        none(py)
    };
    if !delta.is_none() {
        let (ids, rows) = (delta.get_item(0)?, delta.get_item(1)?);
        let renderer = view.getattr("renderer")?;
        tracker.call_method1("apply", (&ids, &rows, &renderer))?;
        view.call_method1("_keep_container_boxes", (&tracker, &ids, kept_boxes, kept_box_rows))?;
        if editing {
            view.call_method1("_reserve_edit_room", (&tracker,))?;
        }
        let parity = py.import("views.wgpu_parity")?;
        let packed = view.getattr("packed")?;
        let changed = tracker.getattr("changed")?.is_truthy()?;
        let removed = tracker.getattr("removed")?;
        let written = ids.len()? != 0;
        if changed {
            packed.getattr("__dict__")?.call_method1("pop", ("_face_ranges", py.None()))?;
            view.setattr("_selection_signature", py.None())?;
            let suppressed = attr(&view, "suppressed_faces")?;
            if suppressed.is_truthy()? && !renderer.is_none() {
                renderer.call_method1("set_suppressed_faces", (parity.call_method1("face_ranges", (&view, suppressed))?,))?;
            }
        }
        parity.call_method1("hidden_geometry", (&view,))?;
        // Rebaking a mesh leaves its placement and rest-of-model flags intact.
        if !tracker.getattr("last_local")?.is_truthy()? || written || removed.is_truthy()? {
            view.setattr("_rest_signature", py.None())?;
        }
        // Existing figures plus rewritten/new rows cover every placement
        // that can face the camera after this delta.
        let previous = attr(&view, "_billboards")?;
        let candidates = if previous.is_none() { none(py) } else { np.call_method1("union1d", (previous.getattr("ids")?, &ids))? };
        view.setattr("_billboards", billboards(py)?.call1((&packed, candidates))?)?;
        if written || changed || removed.is_truthy()? {
            let mut mapping_changed = removed.is_truthy()?;
            let (uids, uid_rows) = (packed.getattr("uids")?, view.getattr("uid_rows")?);
            for row in ids.try_iter()? {
                let row = row?;
                let uid = uids.get_item(&row)?;
                if !uid_rows.contains(&uid)? {
                    uid_rows.set_item(uid, PyList::new(py, [row.call_method0("__int__")?])?)?;
                    mapping_changed = true;
                }
            }
            for row in removed.try_iter()? {
                uid_rows.call_method1("pop", (uids.get_item(row?)?, py.None()))?;
            }
            if mapping_changed {
                view.setattr("_selection_signature", py.None())?;
                view.setattr("_tint_signature", py.None())?;
            }
            view.setattr("_resident_count", packed.getattr("instances")?.len()?)?;
            let index = view.getattr("inference_index")?;
            if !index.is_none() {
                let kw = PyDict::new(py);
                kw.set_item("code_rows", tracker.getattr("changed_code_rows")?)?;
                index.call_method("update", (&ids, &packed), Some(&kw))?;
            } else {
                view.setattr("inference_index", py.None())?;
                cancel_index_build(&view)?;
                if !truthy(&view, "_inference_queued")? {
                    view.setattr("_inference_queued", true)?;
                    later(&view, 0, view.getattr("_refresh_inference")?)?;
                }
            }
        }
        view.setattr("_content_token", token)?;
        view.setattr("_paint_key", paint_key)?;
        view.setattr("build_seconds", perf_counter(py)? - start)?;
        view.call_method0("update")?;
        return Ok(none(py));
    }
    let source = view.getattr("source_viewport")?;
    let zero = || (0., 0., 0.).into_bound_py_any(py);
    let old_packed = view.getattr("packed")?;
    let old_origin = match old_packed.getattr_opt("origin")? {
        Some(origin) => origin,
        None => zero()?,
    };
    let focus = if focus.is_none() {
        let camera = attr(&source, "camera")?;
        if camera.is_truthy()? {
            camera.getattr("target")?.call_method0("toTuple")?
        } else {
            np.call_method1("add", (view.getattr("camera")?.getattr("target")?.call_method0("toTuple")?, &old_origin))?
        }
    } else {
        focus
    };
    if prepared.is_none() {
        let kw = PyDict::new(py);
        kw.set_item("progress", &progress)?;
        kw.set_item("focus", &focus)?;
        kw.set_item("previous_origin", if same_document { old_origin.clone() } else { zero()? })?;
        kw.set_item("previous_pack", if same_document { old_packed.clone() } else { none(py) })?;
        let packed = py.import("views.wgpu_view")?.getattr("pack_scene")?.call((&scene, attr(&source, "view_tint")?), Some(&kw))?;
        view.setattr("packed", packed)?;
    } else {
        view.setattr("packed", prepared.get_item(0)?)?;
        view.setattr("_ui_tree_model", ((scene.as_ptr() as usize, scene.getattr("content_version")?), prepared.get_item(2)?))?;
    }
    let packed = view.getattr("packed")?;
    let origins = packed.getattr("prototype_origins")?;
    let remote = PyDict::new(py);
    let mut anchored = false;
    for origin in origins.try_iter()? {
        anchored |= any_of(py, &origin?)?;
    }
    if anchored {
        let codes = packed.getattr("instances")?.get_item("info")?.get_item((pyo3::types::PySlice::full(py), 0))?;
        for (group, code) in packed.getattr("groups")?.try_iter()?.zip(codes.try_iter()?) {
            let group = group?;
            let anchor = origins.get_item(code?)?;
            if !group.is_none() && any_of(py, &anchor)? {
                let mesh = group.getattr("mesh")?;
                remote.set_item(mesh.as_ptr() as usize, (&mesh, anchor))?;
            }
        }
    }
    view.setattr("_remote_meshes", py.get_type::<PyTuple>().call1((remote.values(),))?)?;
    if !packed.getattr("model_translations")?.is_none() {
        let kw = PyDict::new(py);
        kw.set_item("install", false)?;
        view.call_method("_sync_scene_camera", (), Some(&kw))?;
    } else if (any_of(py, &old_origin)? || truthy(&view, "_source_pose")?) && !source.is_none() {
        view.setattr("camera", source.getattr("camera")?)?;
        view.setattr("_source_pose", py.None())?;
    }
    view.setattr("_billboards", billboards(py)?.call1((&packed,))?)?;
    view.setattr("_scene_dirty", true)?;
    view.setattr("_selection_signature", py.None())?;
    view.setattr("_tint_signature", py.None())?;
    if !tracker.is_none() {
        let unsubscribe = tracker.getattr("unsubscribe")?;
        if !unsubscribe.is_none() {
            unsubscribe.call0()?;
        }
    }
    let rows = packed.getattr("instances")?.len()?;
    view.setattr("_resident_count", rows)?;
    let tracker = if prepared.is_none() {
        py.import("built_core")?.getattr("InstanceUpdates")?.call1((&scene, &packed))?
    } else {
        prepared.get_item(1)?
    };
    view.setattr("_instance_updates", &tracker)?;
    view.setattr("uid_rows", tracker.getattr("uid_rows")?)?;
    tracker.call_method1("observe", (&history,))?;
    tracker.call_method0("remember_edit_path")?;
    // The stored callbacks see the view weakly: it must not be kept by its
    // own after_open list.
    let weak = py.import("weakref")?.call_method1("ref", (&view,))?.unbind();
    let weak_scene = py.import("weakref")?.call_method1("ref", (&scene,))?.unbind();
    // Measure the big containers' edit boxes ahead of the first edit (863
    // ms on a large model's first double-click otherwise). The world
    // geometry and the group's own axes survive entering/leaving a
    // container. Opening a document, they wait for idle time (_group_obb
    // still measures one on demand).
    if rows > 4096 {
        let containers = PyList::empty(py);
        for group in scene.getattr("groups")?.try_iter()? {
            let group = group?;
            if group.getattr("children")?.is_truthy()? {
                containers.append(group)?;
            }
        }
        let big = |view: &Obj<'py>, containers: &Bound<'py, PyList>| -> PyResult<Bound<'py, PyList>> {
            let big = PyList::empty(py);
            for group in containers.iter() {
                if view.call_method1("_rows_for_group", (&group,))?.len()? > 4096 {
                    big.append(group)?;
                }
            }
            Ok(big)
        };
        if opening {
            let (weak, held, containers) = (weak.clone_ref(py), weak_scene.clone_ref(py), containers.unbind());
            let measure = callback(py, move |py| {
                let (view, scene) = (weak.bind(py).call0()?, held.bind(py).call0()?);
                if view.is_none() || scene.is_none() || !view.getattr("scene")?.is(&scene) {
                    return Ok(());
                }
                let big = PyList::empty(py);
                for group in containers.bind(py).iter() {
                    if view.call_method1("_rows_for_group", (&group,))?.len()? > 4096 {
                        big.append(group)?;
                    }
                }
                view.call_method1("_measure_boxes_later", (scene, big))?;
                Ok(())
            })?;
            view.getattr("after_open")?.call_method1("append", (measure,))?;
        } else {
            for group in big(&view, &containers)?.iter() {
                view.call_method1("_group_obb", (group,))?;
            }
        }
    }
    // The batched native build releases the GIL, so it can overlap upload
    // and the first frame. Full repacks also need a fresh index on edits.
    if !opening {
        view.setattr("build_seconds", perf_counter(py)? - start)?;
    }
    if source.is_none() && !any_of(py, &packed.getattr("origin")?)? {
        let bounds = packed.getattr("world_bounds")?;
        let (low, high): ([f64; 3], [f64; 3]) = (bounds.get_item(0)?.extract()?, bounds.get_item(1)?.extract()?);
        let camera = view.getattr("camera")?;
        let middle: [f64; 3] = std::array::from_fn(|k| (low[k] + high[k]) / 2.);
        camera.setattr("target", py.import("built_core")?.getattr("Vec3")?.call1((middle[0], middle[1], middle[2]))?)?;
        let diagonal = (0..3).map(|k| (high[k] - low[k]).powi(2)).sum::<f64>().sqrt();
        camera.setattr("distance", (diagonal * 1.4).max(1.))?;
    }
    if prepared.is_none() {
        view.call_method0("_prepare_inference")?;
    } else {
        cancel_index_build(&view)?;
        view.setattr("inference_index", py.None())?;
        let held = weak_scene;
        let index = callback(py, move |py| {
            let (view, scene) = (weak.bind(py).call0()?, held.bind(py).call0()?);
            if !view.is_none() && !scene.is_none() && view.getattr("scene")?.is(&scene) {
                view.call_method0("_prepare_inference")?;
            }
            Ok(())
        })?;
        view.getattr("after_open")?.call_method1("insert", (0, index))?;
    }
    if !progress.is_none() {
        // The bar's repaint may present a frame, and a frame uploads a
        // dirty pack itself: then it is up already.
        progress.call1((1.0, "Sending the model to the graphics card…"))?;
    }
    if view.getattr("_scene_dirty")?.is_truthy()? || view.getattr("renderer")?.is_none() {
        view.call_method0("_upload")?;
    }
    view.setattr("_content_token", token)?;
    view.setattr("_paint_key", paint_key)?;
    Ok(none(py))
}

/// Entering a group: give the resident arrays spare room on the worker
/// (`InstanceUpdates.reserve`), adopted once copied.
fn reserve_edit_room<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, tracker) = (a.get_item(0)?, a.get_item(1)?);
    let packed = tracker.getattr("packed")?;
    if !packed.getattr("room")?.is_none() || !packed.getattr("reserving")?.is_none() {
        return Ok(none(py));
    }
    if !view.hasattr("_inference_worker")? {
        view.setattr("_inference_worker", index_worker(py)?)?;
    }
    // Behind a snap index build still running: they would compete.
    tracker.call_method1("reserve", (view.getattr("_inference_worker")?,))?;
    adopt_later(&view, &tracker)?;
    Ok(none(py))
}

fn adopt_later(view: &Obj<'_>, tracker: &Obj<'_>) -> PyResult<()> {
    let py = view.py();
    let (held, pending) = (view.clone().unbind(), tracker.clone().unbind());
    let adopt = callback(py, move |py| {
        let (view, tracker) = (held.bind(py), pending.bind(py));
        if truthy(view, "_closed")? || !view.getattr("_instance_updates")?.is(tracker) {
            return Ok(());
        }
        let old = tracker.call_method0("adopt_reserved")?;
        if old.is(&pyo3::types::PyBool::new(py, false)) {
            return adopt_later(view, tracker);
        }
        let index = view.getattr("inference_index")?;
        let packed = attr(&index, "packed")?;
        let current = view.getattr("packed")?;
        if old.is_truthy()? && !packed.is_none() && !packed.is(&current) && packed.getattr("vertices")?.is(&old.get_item(0)?) {
            // The index's open-time snapshot shares the old arrays; same
            // contents, so let it read the new ones and free those.
            let kw = PyDict::new(py);
            kw.set_item("vertices", current.getattr("vertices")?)?;
            kw.set_item("indices", current.getattr("indices")?)?;
            index.setattr("packed", packed.call_method("replace", (), Some(&kw))?)?;
        }
        Ok(())
    })?;
    later(view, 50, adopt.into_any())
}

/// One container box per idle slice, from a few seconds after the document
/// is on screen; a box measured on demand meanwhile is kept.
fn measure_boxes_later<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, scene) = (a.get_item(0)?, a.get_item(1)?);
    let pending = py.get_type::<PyList>().call1((a.get_item(2)?,))?;
    if !pending.is_truthy()? {
        return Ok(none(py));
    }
    measure_step(&view, &scene, &pending, 3000)?;
    Ok(none(py))
}

fn measure_step(view: &Obj<'_>, scene: &Obj<'_>, pending: &Obj<'_>, delay: u32) -> PyResult<()> {
    let py = view.py();
    let (held, of_scene, waiting) = (view.clone().unbind(), scene.clone().unbind(), pending.clone().unbind());
    let step = callback(py, move |py| {
        let (view, scene, pending) = (held.bind(py), of_scene.bind(py), waiting.bind(py));
        if truthy(view, "_closed")? || !view.getattr("scene")?.is(scene) {
            return Ok(());
        }
        let boxes = view.getattr("_shared")?.getattr("__dict__")?.call_method1("setdefault", ("_container_boxes", PyDict::new(py)))?;
        while pending.is_truthy()? && boxes.contains(pending.get_item(0)?)? {
            pending.call_method1("pop", (0,))?;
        }
        if !pending.is_truthy()? {
            return Ok(());
        }
        view.call_method1("_group_obb", (pending.call_method1("pop", (0,))?,))?;
        measure_step(view, scene, pending, 250)
    })?;
    later(view, delay, step.into_any())
}

/// Only the background, while another document loads into this view: the
/// previous model stayed on screen until the new pack went up, which on a
/// big file is a minute after its tab took the name.
fn blank<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let tracker = attr(&view, "_instance_updates")?;
    if !tracker.is_none() {
        let unsubscribe = tracker.getattr("unsubscribe")?;
        if !unsubscribe.is_none() {
            unsubscribe.call0()?;
        }
    }
    view.setattr("_instance_updates", py.None())?;
    view.setattr("_container_boxes", PyDict::new(py))?;
    view.setattr("_selection_rows", PyDict::new(py))?;
    view.setattr("_resident_edit_copies", PyTuple::empty(py))?;
    view.setattr("_parent_map", py.None())?;
    let empty = py.import("built_core.scene")?.getattr("Scene")?.call0()?;
    let packed = py.import("views.wgpu_view")?.getattr("pack_scene")?.call1((empty,))?;
    view.setattr("packed", &packed)?;
    view.setattr("_remote_meshes", PyTuple::empty(py))?;
    view.setattr("_billboards", billboards(py)?.call1((packed,))?)?;
    view.setattr("_content_token", py.None())?;
    view.setattr("_selection_signature", py.None())?;
    view.setattr("_tint_signature", py.None())?;
    view.setattr("uid_rows", PyDict::new(py))?;
    view.setattr("inference_index", py.None())?;
    view.setattr("_scene_dirty", true)?;
    view.call_method0("_upload")?;
    view.call_method0("update")
}

fn refresh_inference<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    view.setattr("_inference_queued", false)?;
    if !truthy(&view, "_closed")? {
        view.call_method0("_prepare_inference")?;
    }
    Ok(none(a.py()))
}

fn prepare_inference<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    if truthy(&view, "_closed")? {
        return Ok(none(py));
    }
    if !view.hasattr("_inference_worker")? {
        view.setattr("_inference_worker", index_worker(py)?)?;
    }
    let worker = view.getattr("_inference_worker")?;
    let previous = attr(&view, "_inference_future")?;
    if !previous.is_none() {
        previous.call_method0("cancel")?;
    }
    view.setattr("inference_index", py.None())?;
    let packed = view.call_method0("_inference_snapshot")?;
    // The job reads the document through a lease the worker's shutdown
    // takes back: a closed document is not kept until the job ends.
    let lease = py.import("built_core")?.getattr("SceneLease")?.call1((view.getattr("scene")?,))?;
    worker.setattr("lease", &lease)?;
    let camera = view.getattr("camera")?;
    let mvp = camera.call_method0("projection_matrix")?.mul(camera.call_method0("view_matrix")?)?;
    let (width, height) = (view.call_method0("width")?, view.call_method0("height")?);
    let (packed, lease, mvp, width, height) = (packed.unbind(), lease.unbind(), mvp.unbind(), width.unbind(), height.unbind());
    let build = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<Py<PyAny>> {
        let py = args.py();
        let inference = py.import("built_core")?;
        let packed = packed.bind(py);
        let index = inference.call_method1("resident_prepare", (packed, lease.bind(py)))?;
        if !index.is_none() && packed.getattr("instances")?.len()? != 0 {
            // Use the immutable placement and camera snapshots. A resize,
            // orbit or edit before publication invalidates the usual key.
            inference.call_method1("screen_boxes", (&index, packed, mvp.bind(py), width.bind(py), height.bind(py)))?;
        }
        Ok(index.unbind())
    })?;
    view.setattr("_inference_future", worker.call_method1("submit", (build,))?)?;
    Ok(none(py))
}

fn inference_snapshot<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let np = py.import("numpy")?;
    let current = view.getattr("packed")?;
    let kw = PyDict::new(py);
    for name in ["instances", "bounds", "commands", "edge_commands"] {
        kw.set_item(name, current.getattr(name)?.call_method0("copy")?)?;
    }
    let packed = current.call_method("replace", (), Some(&kw))?;
    let mut anchored = any_of(py, &packed.getattr("origin")?)?;
    for origin in packed.getattr("prototype_origins")?.try_iter()? {
        anchored |= any_of(py, &origin?)?;
    }
    if anchored {
        let codes = packed.getattr("instances")?.get_item("info")?.get_item((pyo3::types::PySlice::full(py), 0))?;
        let kw = PyDict::new(py);
        kw.set_item("return_index", true)?;
        let first = np.call_method("unique", (codes,), Some(&kw))?.get_item(1)?;
        let groups = py.get_type::<PyList>().call1((packed.getattr("groups")?,))?;
        packed.setattr("groups", &groups)?;
        let sources = PyDict::new(py);
        packed.setattr("_snap_sources", &sources)?;
        let loose = view.getattr("scene")?.getattr("loose_mesh")?;
        for row in first.try_iter()? {
            let group = groups.get_item(row?)?;
            let mesh = if group.is_none() { loose.clone() } else { group.getattr("mesh")? };
            let store = mesh.getattr("__dict__")?.call_method1("get", ("_store",))?;
            let key = mesh.as_ptr() as usize;
            if !store.is_none() && !sources.contains(key)? {
                sources.set_item(key, (store.call_method0("snapshot")?, store.getattr("frame_origin")?))?;
            }
        }
    }
    Ok(packed)
}

fn upload<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, force) = (a.get_item(0)?, a.get_item(1)?.is_truthy()?);
    if view.getattr("renderer")?.is_none() && !force && !view.call_method0("_can_create_surface")?.is_truthy()? {
        return Ok(none(py));
    }
    let packed = view.getattr("packed")?;
    if packed.is_none() || !(force || view.call_method0("_presentable")?.is_truthy()?) {
        return Ok(none(py));
    }
    let render = py.import("built_render")?;
    let start = perf_counter(py)?;
    if view.getattr("renderer")?.is_none() {
        let handles = view.call_method0("_window_handles")?;
        let renderer = render.getattr("Renderer")?.call(
            (handles.get_item(0)?, view.call_method0("width")?, view.call_method0("height")?, view.call_method0("devicePixelRatioF")?),
            Some(&handles.get_item(1)?.cast_into::<PyDict>()?),
        )?;
        view.setattr("renderer", renderer)?;
        view.call_method0("_wrap_renderer")?;
        py.import("views.wgpu_ui")?.call_method1("attach", (&view,))?;
    }
    let renderer = view.getattr("renderer")?;
    let rows = packed.getattr("instances")?.len()?;
    view.setattr("_resident_count", rows.max(1))?;
    if rows != 0 {
        packed.call_method1("upload", (&renderer,))?;
    } else {
        // The renderer ABI requires one row. A hidden scratch row lets an
        // empty document show the first tool rubber band.
        let vec3 = py.import("built_core")?.getattr("Vec3")?;
        let mesh = py.import("built_core.mesh")?.getattr("Mesh")?.call0()?;
        mesh.call_method1("add_edge", (vec3.call0()?, vec3.call1((1, 0, 0))?))?;
        let kw = PyDict::new(py);
        kw.set_item("groups", PyList::new(py, [py.import("built_core.group")?.getattr("Group")?.call1((mesh,))?])?)?;
        let stage = py.import("built_core.scene")?.getattr("Scene")?.call((), Some(&kw))?;
        let scratch = py.import("views.wgpu_view")?.getattr("pack_scene")?.call1((stage,))?;
        scratch.getattr("instances")?.get_item("info")?.set_item((pyo3::types::PySlice::full(py), 2), 1)?;
        scratch.call_method1("upload", (&renderer,))?;
    }
    let suppressed = attr(&view, "suppressed_faces")?;
    if suppressed.is_truthy()? {
        let ranges = py.import("views.wgpu_parity")?.call_method1("face_ranges", (&view, suppressed))?;
        renderer.call_method1("set_suppressed_faces", (ranges,))?;
    }
    view.setattr("_rest_signature", py.None())?;
    view.setattr("_frame_style", py.None())?;
    view.setattr("_axes_state", py.None())?;
    let sprites = attr(&view, "_billboards")?;
    if !sprites.is_none() {
        sprites.setattr("key", py.None())?;
    }
    view.setattr("upload_seconds", perf_counter(py)? - start)?;
    view.setattr("_scene_dirty", false)?;
    view.setattr("_pick_frame_dirty", true)?;
    view.call_method0("update")?;
    for peer in view.call_method0("peers")?.try_iter()? {
        let peer = peer?;
        peer.setattr("_pick_frame_dirty", true)?;
        peer.call_method0("update")?;
    }
    Ok(none(py))
}

fn show_event<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    base(&view)?.call_method1("showEvent", (a.get_item(1)?,))?;
    if view.getattr("renderer")?.is_none() || view.getattr("_scene_dirty")?.is_truthy()? {
        view.call_method0("_upload")?;
    }
    Ok(none(a.py()))
}

fn resize_event<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    base(&view)?.call_method1("resizeEvent", (a.get_item(1)?,))?;
    // Keep the last Metal frame during a splitter/window drag. Surface
    // configuration waits for GPU work and reallocates all render targets;
    // doing it per pixel can consume the entire input frame budget.
    let (width, height): (f64, f64) = (view.call_method0("width")?.extract()?, view.call_method0("height")?.extract()?);
    let aspect = width / height.max(1.);
    view.getattr("camera")?.setattr("aspect", aspect)?;
    let shared = attr(&view.getattr("source_viewport")?, "camera")?;
    if !shared.is_none() {
        shared.setattr("aspect", view.getattr("camera")?.getattr("aspect")?)?;
    }
    view.getattr("_resize_timer")?.call_method0("start")?;
    Ok(none(a.py()))
}

fn settled_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    let renderer = view.getattr("renderer")?;
    if attr(&renderer, "needs_shadow_fit")?.is(&pyo3::types::PyBool::new(a.py(), true)) {
        renderer.call_method0("settle_shadows")?;
    }
    view.call_method0("update")
}

fn finish_resize<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let renderer = view.getattr("renderer")?;
    if !renderer.is_none() {
        let physical = (view.call_method0("width")?, view.call_method0("height")?, view.call_method0("devicePixelRatioF")?).into_bound_py_any(py)?;
        if !physical.eq(attr(&view, "_physical_size")?)? {
            renderer.getattr("resize")?.call1(physical.clone().cast_into::<PyTuple>()?)?;
            view.setattr("_physical_size", physical)?;
        }
    }
    view.setattr("_pick_frame_dirty", true)?;
    view.call_method0("update")
}

fn draw_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    if view.getattr("renderer")?.is_none() || truthy(&view, "_drawing_frame")? {
        return Ok(none(py));
    }
    // Asked for explicitly (a pick, the view grid, an export): the pending
    // surface size first. Only the repaints of a live drag wait for the
    // resize to settle (paintEvent).
    let resize = view.getattr("_resize_timer")?;
    if resize.call_method0("isActive")?.is_truthy()? {
        resize.call_method0("stop")?;
        view.call_method0("_finish_resize")?;
    }
    view.setattr("_drawing_frame", true)?;
    let frame = (|| -> Reply<'py> {
        let ui = py.import("views.wgpu_ui")?;
        if direct(py, super::chrome_frame::chrome_only, (&view,))?.is_truthy()? {
            return ui.call_method1("chrome_frame", (&view,)); // a document is coming in
        }
        let sheets = py.import("built_render.sheet_workspace")?;
        if sheets.call_method1("shown", (&view,))?.is_truthy()? {
            return sheets.call_method1("frame", (&view,)); // the Sheets workspace: the page, not the model
        }
        view.call_method0("_present_frame")
    })();
    view.setattr("_drawing_frame", false)?;
    frame
}

/// Mirror the document pose and subtract its target frame before upload.
fn sync_scene_camera<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, install) = (a.get_item(0)?, a.get_item(1)?.is_truthy()?);
    let np = py.import("numpy")?;
    let copy = py.import("copy")?.getattr("copy")?;
    let frame = py.import("built_core.coordinate_frame")?;
    let choose_origin = frame.getattr("choose_origin")?;
    let camera_point = py.import("built_core")?.getattr("Vec3")?;
    let vec3 = py.import("built_core")?.getattr("Vec3")?;
    let signature = |value: &Obj<'py>| -> Reply<'py> {
        if value.is_instance(&vec3)? { value.call_method0("toTuple") } else { Ok(value.clone()) }
    };
    let packed = view.getattr("packed")?;
    let mut source = attr(&view.getattr("source_viewport")?, "camera")?;
    let previous = attr(&view, "_source_pose")?;
    if source.is_none() {
        if previous.is_none() {
            source = copy.call1((view.getattr("camera")?,))?;
            view.setattr("_standalone_camera", &source)?;
        } else {
            source = view.getattr("_standalone_camera")?;
        }
    }
    let target_of = |camera: &Obj<'py>| camera.getattr("target")?.call_method0("toTuple");
    let mut keep_local_target = false;
    if previous.is_none() || !previous.get_item(1)?.is(&source) {
        let bounds = packed.getattr("world_bounds")?;
        let (low, high) = (bounds.get_item(0)?, bounds.get_item(1)?);
        let norm = np.getattr("linalg")?.getattr("norm")?;
        let target = np.call_method1("subtract", (target_of(&source)?, packed.getattr("origin")?))?;
        let away: f64 = norm.call1((target.sub(low.add(&high)?.mul(0.5)?)?,))?.extract()?;
        let span: f64 = norm.call1((high.sub(&low)?,))?.extract()?;
        let distance: f64 = source.getattr("distance")?.extract()?;
        let outside = away > 4. * distance.max(span);
        if outside && !attr(&attr(&view, "scene")?, "camera_home")?.is_truthy()? {
            let origin = packed.getattr("origin")?;
            source.call_method1("fit_to", (camera_point.call1((low.add(&origin)?,))?, camera_point.call1((high.add(&origin)?,))?))?;
        }
        view.setattr("camera", copy.call1((&source,))?)?;
    } else {
        let old_origin = previous.get_item(0)?;
        for item in previous.get_item(2)?.call_method0("items")?.try_iter()? {
            let item = item?;
            let name: String = item.get_item(0)?.extract()?;
            if signature(&source.getattr(name.as_str())?)?.eq(item.get_item(1)?)? {
                let local = view.getattr("camera")?.getattr(name.as_str())?;
                if name == "target" {
                    keep_local_target = true;
                    source.setattr("target", local.add(camera_point.call1((&old_origin,))?)?)?;
                } else {
                    source.setattr(name.as_str(), local)?;
                }
            }
        }
    }
    let mut origin = choose_origin.call1((target_of(&source)?, packed.getattr("origin")?))?;
    if !origin.eq(packed.getattr("origin")?)? && !any_of(py, &choose_origin.call1((target_of(&source)?,))?)? {
        origin = (0., 0., 0.).into_bound_py_any(py)?;
    }
    let sprites = attr(&view, "_billboards")?;
    if !origin.eq(packed.getattr("origin")?)? && install && !sprites.is_none() {
        sprites.call_method0("restore")?;
    }
    let changed = packed.call_method1("rebase", (&origin,))?.is_truthy()?;
    let state = PyDict::new(py);
    let camera = view.getattr("camera")?;
    for item in py.import("builtins")?.call_method1("vars", (&source,))?.call_method0("items")?.try_iter()? {
        let item = item?;
        let name: String = item.get_item(0)?.extract()?;
        if name.starts_with('_') || name == "aspect" || name == "depth_reach" {
            continue;
        }
        let value = item.get_item(1)?;
        state.set_item(&name, signature(&value)?)?;
        if name == "target" {
            if keep_local_target && previous.get_item(0)?.eq(&origin)? {
                continue;
            }
            camera.setattr("target", value.sub(camera_point.call1((&origin,))?)?)?;
        } else {
            camera.setattr(name.as_str(), value)?;
        }
    }
    if previous.is_none() || !previous.get_item(2)?.eq(&state)? || !previous.get_item(0)?.eq(&origin)? {
        view.setattr("_pick_frame_dirty", true)?;
    }
    if !previous.is_none() && !previous.get_item(0)?.eq(&origin)? && truthy(&view, "_drag_nav")? {
        let pivot = attr(&view, "_pivot")?;
        if !pivot.is_none() {
            let shift = camera_point.call1((np.call_method1("subtract", (previous.get_item(0)?, &origin))?,))?;
            view.setattr("_pivot", pivot.add(shift)?)?;
        }
    }
    let reach = source.getattr("depth_reach")?;
    if reach.is_truthy()? {
        camera.setattr("depth_reach", (reach.get_item(0)?.sub(camera_point.call1((&origin,))?)?, reach.get_item(1)?))?;
    } else {
        camera.setattr("depth_reach", py.None())?;
    }
    view.setattr("_source_pose", (&origin, &source, &state))?;
    let bake_focus = packed.getattr("bake_focus")?;
    if install && !bake_focus.is_none() && !choose_origin.call1((target_of(&source)?, &bake_focus))?.eq(&bake_focus)? {
        // A single prototype spans distant sites. Bake its visible
        // neighbourhood before the first narrowing to vertex float32.
        let kw = PyDict::new(py);
        kw.set_item("focus", target_of(&source)?)?;
        view.getattr("_shared")?.call_method("set_scene", (view.getattr("scene")?,), Some(&kw))?;
        return Ok(none(py));
    }
    if changed && install {
        // Their base holds translations in the previous frame.
        view.setattr("_billboards", billboards(py)?.call1((&packed,))?)?;
        let rows = packed.getattr("instances")?;
        let ids = py.get_type::<PyList>().call1((py.import("builtins")?.call_method1("range", (rows.len()?,))?,))?;
        let renderer = view.getattr("renderer")?;
        if !renderer.is_none() && !view.getattr("_scene_dirty")?.is_truthy()? {
            renderer.call_method1("set_instance_world", (ids, rows.get_item("world")?.call_method0("flatten")?))?;
        }
        view.setattr("_pick_frame_dirty", true)?;
        let index = view.getattr("inference_index")?;
        if !index.is_none() {
            view.setattr("inference_index", py.import("built_core")?.call_method1("resident_rebase", (index, &packed))?)?;
        }
    }
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("set_scene", Mode::Method, set_scene, &["self", "scene", "*", "progress=None", "opening=False", "focus=None", "prepared=None"], "", ""),
    ("_reserve_edit_room", Mode::Method, reserve_edit_room, &["self", "tracker"], "", ""),
    ("_measure_boxes_later", Mode::Method, measure_boxes_later, &["self", "scene", "groups"], "", ""),
    ("blank", Mode::Method, blank, &["self"], "", ""),
    ("_refresh_inference", Mode::Method, refresh_inference, &["self"], "", ""),
    ("_prepare_inference", Mode::Method, prepare_inference, &["self"], "", ""),
    ("_inference_snapshot", Mode::Method, inference_snapshot, &["self"], "", ""),
    ("_upload", Mode::Method, upload, &["self", "force=False"], "", ""),
    ("showEvent", Mode::Method, show_event, &["self", "event"], "", ""),
    ("resizeEvent", Mode::Method, resize_event, &["self", "event"], "", ""),
    ("_settled_frame", Mode::Method, settled_frame, &["self"], "", ""),
    ("_finish_resize", Mode::Method, finish_resize, &["self"], "", ""),
    ("draw_frame", Mode::Method, draw_frame, &["self"], "", ""),
    ("_sync_scene_camera", Mode::Method, sync_scene_camera, &["self", "*", "install=True"], "", ""),
];

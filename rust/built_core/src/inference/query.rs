// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::PyCFunction;
pub(super) fn pick_plane(vp: &Obj, x: f64, y: f64) -> PyResult<Option<(Vec3, Vec3)>> {
    let pick = match vp.getattr_opt("pick_face_inference")? {
        Some(p) if p.is_truthy()? => p,
        _ => vp.getattr("pick_face_placement")?,
    };
    let (face, owner): (Obj, Obj) = pick.call1((x, y))?.extract()?;
    if face.is_none() {
        return Ok(None);
    }
    let xf = attr(&owner, "xform")?;
    crate::tools::record_queries::face_plane_world(&face, (!xf.is_none()).then_some(&xf)).map(Some)
}
pub(super) fn hover_lock<'p>(vp: &Obj<'p>, x: f64, y: f64) -> PyResult<Obj<'p>> {
    let py = vp.py();
    let Some((o, n)) = pick_plane(vp, x, y)? else {
        return Ok(none(py));
    };
    let at = vp.call_method1("_world_from_pixel", (x, y))?;
    let ref_ = if at.is_none()
        || (xyz(&at)? - DVec3::from_array(o.coords()))
            .dot(DVec3::from_array(n.coords()))
            .abs()
            > 1e-3
    {
        o.clone().into_bound_py_any(py)?
    } else {
        at
    };
    (o, n, ref_).into_bound_py_any(py)
}
pub(super) fn centers<'p>(vp: &Obj<'p>, x: f64, y: f64) -> PyResult<Obj<'p>> {
    match vp.getattr_opt("_face_centers_near")? {
        Some(f) if f.is_callable() => f.call1((x, y)),
        _ => Ok(none(vp.py())),
    }
}
pub(super) fn options<'p>(
    engine: &Bound<'p, InferenceEngine>,
    tool: &Obj<'p>,
    mods: i64,
) -> PyResult<Bound<'p, PyDict>> {
    let py = engine.py();
    let d = PyDict::new(py);
    let vp = engine.getattr("viewport")?;
    for k in [
        "chain_first_point",
        "start_point",
        "magnetic_axis_deg",
        "screen_axis_px",
    ] {
        d.set_item(k, attr(tool, k)?)?;
    }
    for k in ["axis_lock", "reference_edge", "reference_mode"] {
        d.set_item(k, engine.getattr(k)?)?;
    }
    for (k, field) in [
        ("acquired_edge", "_acquired_edge"),
        ("acquired_face_normal", "_acquired_face_normal"),
        ("acquired_point", "_acquired_point"),
        ("acquired_points", "_encouraged"),
        ("shift_lock_ref", "_shift_lock_ref"),
    ] {
        d.set_item(k, engine.getattr(field)?)?;
    }
    let lock = engine.getattr("_shift_lock")?;
    for (i, k) in ["shift_lock_dir", "shift_lock_color"]
        .into_iter()
        .enumerate()
    {
        d.set_item(
            k,
            if lock.is_truthy()? {
                lock.get_item(i)?
            } else {
                none(py)
            },
        )?;
    }
    d.set_item("shift_held", mods & SHIFT != 0)?;
    d.set_item(
        "inference_angle_deg",
        vp.getattr_opt("inference_angle_deg")?
            .unwrap_or(3f64.into_bound_py_any(py)?),
    )?;
    d.set_item(
        "linear_mode",
        vp.getattr_opt("linear_inference_mode")?
            .unwrap_or("all".into_bound_py_any(py)?),
    )?;
    for k in ["diagonal_span", "radial_arm"] {
        d.set_item(k, flag(tool, k)?)?;
    }
    let magnet = attr(tool, "magnet_off_plane")?;
    d.set_item(
        "axis_in_plane",
        magnet.is_callable() && !magnet.call0()?.is_truthy()?,
    )?;
    d.set_item(
        "free_3d",
        !tool.is_instance(&py.import("tools.base")?.getattr("PlaneLock")?)?
            && !flag(tool, "prefers_vertical_drag")?,
    )?;
    let api = py.import("built_core.inference")?;
    d.set_item(
        "sticky_axis",
        if api.getattr("_STICKY_AXIS")?.is_truthy()? {
            engine.getattr("_sticky_axis")?
        } else {
            none(py)
        },
    )?;
    d.set_item("dwell_axis_lock", api.getattr("_DWELL_ARROW")?)?;
    Ok(d)
}
pub(super) fn snap<'p>(
    engine: &Bound<'p, InferenceEngine>,
    x: f64,
    y: f64,
    mods: i64,
) -> PyResult<Obj<'p>> {
    let py = engine.py();
    let vp = engine.getattr("viewport")?;
    let point = vp.call_method1("_world_from_pixel", (x, y))?;
    if point.is_none() {
        return Ok(point);
    }
    let tool = vp.getattr("active_tool")?;
    let api = py.import("built_core.snap")?;
    if !tool.is_none() && !flag(&tool, "uses_snap")? {
        return api.call_method1("SnapResult", (point, "none"));
    }
    let kw = options(engine, &tool, mods)?;
    let mut edge = none(py);
    let mut plane = none(py);
    if !attr(&tool, "start_point")?.is_none() && engine.borrow().direction_locked(mods) {
        edge = engine.getattr("_hover_edge")?;
        if edge.is_none() {
            plane = hover_lock(&vp, x, y)?;
        }
    }
    kw.set_item("hover_lock_edge", edge)?;
    kw.set_item("hover_lock_plane", plane)?;
    kw.set_item("candidate_world", point)?;
    kw.set_item("candidate_pixel", (x, y))?;
    kw.set_item("scene", vp.call_method1("_snap_scene", (x, y))?)?;
    kw.set_item(
        "world_to_pixel",
        if let Some(f) = vp.getattr_opt("_pixel_projector")? {
            f.call0()?
        } else {
            vp.getattr("_world_to_pixel")?
        },
    )?;
    for (k, field) in [
        ("threshold_px", "snap_threshold_px"),
        ("edge_threshold_px", "edge_snap_threshold_px"),
        ("is_occluded", "_is_occluded"),
    ] {
        kw.set_item(k, vp.getattr(field)?)?;
    }
    let project = vp.getattr("_project_to_lock_line")?.unbind();
    kw.set_item(
        "project_onto_line",
        PyCFunction::new_closure(py, None, None, move |args, _| -> PyResult<Py<PyAny>> {
            Ok(project
                .bind(args.py())
                .call1((args.get_item(0)?, args.get_item(1)?, x, y))?
                .unbind())
        })?,
    )?;
    let no_fast = flag(
        py.import("views.document_interaction")?.as_any(),
        "_NO_SNAP_FAST",
    )?;
    kw.set_item(
        "are_occluded",
        if no_fast {
            none(py)
        } else {
            vp.getattr("_occluded_many")?
        },
    )?;
    kw.set_item(
        "face_under_cursor",
        vp.call_method1("_still_face_under", (x, y))?,
    )?;
    kw.set_item(
        "face_context",
        match vp.getattr_opt("_face_context_under")? {
            Some(f) => f.call1((x, y))?,
            None => none(py),
        },
    )?;
    kw.set_item("work_plane_normal", vp.call_method0("_work_plane_normal")?)?;
    let partial = py.import("functools")?.getattr("partial")?;
    for (k, field) in [
        ("face_crossing", "_face_crossing_point"),
        ("face_outline", "_face_outline_world"),
    ] {
        kw.set_item(
            k,
            match vp.getattr_opt(field)? {
                Some(f) => partial.call1((f, x, y))?,
                None => none(py),
            },
        )?;
    }
    kw.set_item("face_centers", centers(&vp, x, y)?)?;
    let result = api.getattr("compute_snap")?.call((), Some(&kw))?;
    engine.call_method1("finish_snap", (result, x, y, tool))
}
pub(super) fn axis_cue<'p>(vp: &Obj<'p>, snap: &Obj<'p>, x: f64, y: f64) -> PyResult<Obj<'p>> {
    if snap.is_none() {
        return Ok(snap.clone());
    }
    let tool = vp.getattr("active_tool")?;
    let empty = flag(&tool, "axis_source_empty_only")?;
    let kind = snap.getattr("kind")?.extract::<String>()?;
    if !flag(&tool, "axis_source")?
        || !attr(&tool, "start_point")?.is_none()
        || !(kind == "none" || !empty && kind == "on_face")
    {
        return Ok(snap.clone());
    }
    let py = vp.py();
    let name = vp.call_method1("pick_axis", (x, y))?;
    if name.is_none() {
        return Ok(snap.clone());
    }
    let axes = py.import("built_core.axes")?;
    let axis = axes.call_method1("axis", (&name,))?;
    let o = axes.call_method0("origin")?;
    let foot = if empty {
        let f = vp.call_method1("_project_to_lock_line", (&o, &axis, x, y))?;
        if !vp
            .call_method1("_axis_source_visible", (&name, &f))?
            .is_truthy()?
            || vp.call_method1("_is_occluded", (&f,))?.is_truthy()?
        {
            return Ok(snap.clone());
        }
        f
    } else {
        let o = xyz(&o)?;
        let d = xyz(&axis)?;
        Vec3::from(o + d * (xyz(&snap.getattr("point")?)? - o).dot(d)).into_bound_py_any(py)?
    };
    let api = py.import("built_core.snap")?;
    let kw = PyDict::new(py);
    kw.set_item("axis", &name)?;
    api.getattr("SnapResult")?.call(
        (foot, "on_axis", api.getattr("AXIS_COLORS")?.get_item(name)?),
        Some(&kw),
    )
}
pub(super) fn extension<'p>(vp: &Obj<'p>, snap: &Obj<'p>, x: f64, y: f64) -> PyResult<Obj<'p>> {
    let fs = attr(vp, "_ext_snap_providers")?;
    if snap.is_none() || !fs.is_truthy()? {
        return Ok(snap.clone());
    }
    if matches!(
        snap.getattr("kind")?.extract::<String>()?.as_str(),
        "endpoint"
            | "midpoint"
            | "arc_midpoint"
            | "center"
            | "origin"
            | "component_origin"
            | "intersection"
            | "intersection_edge"
            | "intersection_face"
            | "close"
            | "on_edge"
            | "face_center"
    ) {
        return Ok(snap.clone());
    }
    let py = vp.py();
    for f in py.get_type::<PyList>().call1((fs,))?.try_iter()? {
        match f?.call1((vp, snap, x, y)) {
            Ok(r) if !r.is_none() => return Ok(r),
            Ok(_) => {}
            Err(e) => {
                let kw = PyDict::new(py);
                kw.set_item("exc_info", (e.get_type(py), e.value(py), e.traceback(py)))?;
                py.import("logging")?
                    .call_method1("getLogger", ("built.plugins",))?
                    .call_method("error", ("extension snap provider failed",), Some(&kw))?;
            }
        }
    }
    Ok(snap.clone())
}
pub(super) fn limit<'p>(snap: &Obj<'p>, tool: &Obj) -> PyResult<Obj<'p>> {
    let allowed = attr(tool, "allowed_snap_kinds")?;
    if !snap.is_none() && !allowed.is_none() {
        let kind = snap.getattr("kind")?;
        if !kind.eq("none")? && !allowed.contains(kind)? {
            let api = snap.py().import("built_core.snap")?;
            return api.call_method1(
                "SnapResult",
                (snap.getattr("point")?, "none", api.getattr("COLOR_NONE")?),
            );
        }
    }
    Ok(snap.clone())
}

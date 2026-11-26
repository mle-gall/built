// SPDX-License-Identifier: GPL-3.0-or-later
//! The Push/Pull machine chooses the transition. These callbacks bind it to
//! the live face, preview and history shared with the protocol operation.
use super::*;
use crate::math::{xyz, Vec3};

fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    imported(py, "tools.pushpull")
}
fn busy<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.getattr("dragging")
}
fn host_reading<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let row = PyDict::new(a.py());
    for (name, field) in [
        ("dragging", "dragging"),
        ("distance", "extrusion"),
        ("mode", "_keep_mode"),
    ] {
        row.set_item(name, t.getattr(field)?)?;
    }
    row.set_item("has_base", !t.getattr("base_face")?.is_none())?;
    row.set_item("repeat", t.get_type().getattr("last_distance")?)?;
    Ok(row.into_any())
}
fn pointer_event<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let kind = a.get_item(1)?;
    let ctx = a.get_item(2)?;
    let old = t.getattr("_ctx")?;
    t.setattr("_ctx", &ctx)?;
    let reply = (|| {
        t.call_method1(
            "_send",
            (
                ctx.getattr("viewport")?,
                t.call_method1("_pointer", (kind, &ctx))?,
            ),
        )
    })();
    t.setattr("_ctx", old)?;
    reply
}
fn host_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let py = a.py();
    t.call_method0("_reset")?;
    t.setattr("_keep_mode", false)?;
    let scene = vp.getattr("scene")?;
    let selection = scene.getattr("selection")?;
    if selection.len()? == 1 {
        let item = selection
            .try_iter()?
            .next()
            .transpose()?
            .ok_or_else(|| pyo3::exceptions::PyStopIteration::new_err(()))?;
        if item.is_instance(&module(py)?.getattr("MeshFace")?)?
            && scene.getattr("mesh")?.getattr("faces")?.contains(&item)?
        {
            t.setattr("hovered_face", &item)?;
            t.setattr("_hover_group", py.None())?;
            t.setattr("_selected_arm", true)?;
            vp.call_method1("set_hover", (item,))?;
        }
    }
    Ok(none(py))
}
fn host_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let py = a.py();
    t.call_method1("_revert_preview", (&vp,))?;
    vp.call_method1("set_hover", (py.None(),))?;
    vp.call_method1("set_suppressed_faces", (PySet::empty(py)?,))?;
    t.call_method0("_reset")?;
    Ok(none(py))
}
fn drag_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let py = a.py();
    if !t.getattr("dragging")?.is_truthy()? {
        return Ok(none(py));
    }
    let anchor = t.getattr("_anchor")?;
    let normal = t.getattr("_normal")?;
    if anchor.is_none() || normal.is_none() {
        return Ok(none(py));
    }
    let cam = optional(&a.get_item(1)?, "camera")?;
    if cam.is_none() {
        return Ok(none(py));
    }
    let forward = xyz(&cam.getattr("target")?)? - xyz(&cam.call_method0("eye")?)?;
    let normal = xyz(&normal)?;
    let side = forward - normal * forward.dot(normal);
    if side.length() < 1e-6 {
        return Ok(none(py));
    }
    (Vec3::from(xyz(&anchor)?), Vec3::from(side.normalize())).into_bound_py_any(py)
}
fn preview_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let ctx = a.get_item(2)?;
    let py = a.py();
    if t.getattr("base_face")?.is_none()
        || t.getattr("_anchor")?.is_none()
        || t.call_method1("_drop_stale_stage", (&vp,))?.is_truthy()?
    {
        return Ok(none(py));
    }
    t.call_method1("_revert_prism_preview", (&vp,))?;
    let inferred = t.call_method1("_infer_reference_distance", (&ctx,))?;
    if inferred.is_none() {
        let mut grab = t.getattr("_grab")?;
        if grab.is_none() {
            grab = t.getattr("_anchor")?;
        }
        let normal = t.getattr("_normal")?;
        let (x, y) = screen(&ctx)?;
        let projected = vp.call_method1("_project_to_lock_line", (&grab, &normal, x, y))?;
        let extrusion = (xyz(&projected)? - xyz(&grab)?).dot(xyz(&normal)?);
        t.setattr("extrusion", extrusion)?;
    } else {
        t.setattr("extrusion", inferred)?;
    }
    t.call_method1("_clamp_extrusion", (&vp,))?;
    t.call_method1("_present", (vp, "auto"))?;
    Ok(none(py))
}
fn host_begin<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let pick = a.get_item(2)?.is_truthy()?;
    let py = a.py();
    let ctx = t.getattr("_ctx")?;
    let (face, group) = if pick {
        vp.call_method1("pick_face_any", screen(&ctx)?)?
            .extract::<(Obj, Obj)>()?
    } else {
        (t.getattr("hovered_face")?, t.getattr("_hover_group")?)
    };
    if face.is_none() {
        return false.into_bound_py_any(py);
    }
    if !group.is_none() {
        t.call_method1("_refuse_closed_group", (vp,))?;
        return false.into_bound_py_any(py);
    }
    t.setattr("base_face", &face)?;
    let scene = vp.getattr("scene")?;
    t.call_method1("_take_extras", (&face, &scene))?;
    t.setattr("extrusion", 0.)?;
    t.setattr("dragging", true)?;
    t.setattr("_group", group)?;
    t.setattr("_selected_arm", false)?;
    let target = t.call_method1("_target_scene", (scene,))?;
    let mesh = target.getattr("mesh")?;
    orient(&mesh, &face)?;
    t.setattr("_drag_pre_oriented", true)?;
    t.setattr("_anchor", face.call_method0("centroid")?)?;
    t.setattr(
        "_normal",
        t.call_method1("_outward_normal", (&face, &mesh))?,
    )?;
    t.setattr("_grab", t.call_method1("_grab_point", (&vp, ctx))?)?;
    let (attached, cap): (Obj, Obj) = t.call_method1("_classify_base", (&target,))?.extract()?;
    t.setattr("_attached", attached)?;
    t.setattr("_prism_cap", cap)?;
    t.setattr(
        "_cap_positions",
        t.call_method1("_cap_loop_positions", (face,))?,
    )?;
    t.setattr("_prism_verts", t.call_method1("_cap_vertices", (&target,))?)?;
    t.call_method1("_compute_inward_limit", (&target,))?;
    t.call_method1("_compute_plane_levels", (target,))?;
    if !pick {
        vp.call_method1("set_hover", (py.None(),))?;
        vp.call_method0("update")?;
    }
    true.into_bound_py_any(py)
}
fn orient(mesh: &Obj, face: &Obj) -> PyResult<()> {
    let kw = PyDict::new(mesh.py());
    kw.set_item("only", (face,))?;
    module(mesh.py())?.call_method("orient_outward", (mesh,), Some(&kw))?;
    Ok(())
}
fn grab_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let py = a.py();
    let hit = optional(&vp, "_ray_hit_plane")?;
    let anchor = t.getattr("_anchor")?;
    let normal = t.getattr("_normal")?;
    if hit.is_none() || anchor.is_none() || normal.is_none() {
        return Ok(none(py));
    }
    hit.call1((screen(&a.get_item(2)?)?, (anchor, normal)))
}
fn host_restore_retype<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let py = a.py();
    let last = t.getattr("_last")?;
    if last.is_none() {
        return Ok(none(py));
    }
    let history = vp.getattr("history")?;
    let stack = optional(&history, "undo_stack")?;
    if !stack.is_truthy()? || !stack.get_item(-1)?.is(&last.get_item("cmd")?) {
        t.setattr("_last", py.None())?;
        return Ok(none(py));
    }
    history.call_method0("undo")?;
    for (attr, key) in [
        ("base_face", "face"),
        ("_group", "group"),
        ("_keep_base", "keep_base"),
        ("_prism_cap", "prism_cap"),
        ("_normal", "normal"),
        ("_cap_positions", "cap_positions"),
        ("_drag_pre_oriented", "pre_oriented"),
        ("_limit_in", "limit_in"),
        ("_extra_faces", "extras"),
        ("_extra_normals", "extra_normals"),
    ] {
        t.setattr(attr, last.get_item(key)?)?;
    }
    t.setattr("dragging", true)?;
    t.setattr("extrusion", 0.)?;
    t.setattr("_prism_applied", 0.)?;
    for attr in ["_prism_verts", "_light_faces", "_light_wires"] {
        t.setattr(attr, PyList::empty(py))?;
    }
    t.setattr("_infer_cache", py.None())?;
    t.setattr("_refused", false)?;
    t.setattr("_topped_out", false)?;
    last.get_item("sign")
}
fn host_cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let py = a.py();
    t.setattr("_light_faces", PyList::empty(py))?;
    t.setattr("_light_wires", PyList::empty(py))?;
    t.call_method1("_revert_preview", (&vp,))?;
    vp.call_method1("set_hover", (py.None(),))?;
    vp.call_method1("set_suppressed_faces", (PySet::empty(py)?,))?;
    t.call_method0("_reset")?;
    t.call_method1("_spend_keep_mode", (&vp,))?;
    vp.call_method0("update")?;
    Ok(none(py))
}
fn cap_vertices<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let py = a.py();
    let m = module(py)?;
    let key = m.getattr("_key")?;
    let keys = PySet::empty(py)?;
    let positions = t.getattr("_cap_positions")?;
    for point in positions.try_iter()? {
        keys.add(key.call1((point?,))?)?;
    }
    let group = t.getattr("_group")?;
    let mesh = if group.is_none() {
        a.get_item(1)?
    } else {
        group
    }
    .getattr("mesh")?;
    let near = m
        .getattr("push_scans")?
        .call_method1("vertices_at_keys", (&mesh, positions))?;
    let verts = if near.is_none() {
        mesh.getattr("vertices")?
    } else {
        near
    };
    let found = PyList::empty(py);
    for vert in verts.try_iter()? {
        let v = vert?;
        if keys.contains(key.call1((v.getattr("position")?,))?)? {
            found.append(v)?;
        }
    }
    Ok(found.into_any())
}
fn compute_plane_levels<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let scene = a.get_item(1)?;
    let py = a.py();
    let levels = PyList::empty(py);
    t.setattr("_plane_levels", &levels)?;
    if !t.getattr("_attached")?.is_truthy()? || t.getattr("_prism_cap")?.is_truthy()? {
        return Ok(none(py));
    }
    let base = t.getattr("base_face")?;
    let normal = t.getattr("_normal")?;
    if base.is_none() || normal.is_none() {
        return Ok(none(py));
    }
    let unit = normal.call_method0("normalized")?;
    let m = module(py)?;
    let arrangement=py.import("built_core.arrangement")?;
    let project = arrangement.getattr("project_loop")?;
    let footprint = project.call1((base.getattr("vertices")?, &unit))?;
    let near = m.getattr("push_scans")?.call_method1(
        "parallel_overlapping",
        (scene.getattr("mesh")?, &unit, &footprint),
    )?;
    let faces = if near.is_none() {
        scene.getattr("faces")?
    } else {
        near
    };
    let eps: f64 = m.getattr("_STITCH_TOL")?.extract()?;
    let direction = xyz(&normal)?;
    let axis = xyz(&unit)?;
    let anchor = xyz(&t.getattr("_anchor")?)?;
    for face in faces.try_iter()? {
        let face = face?;
        if face.eq(&base)?
            || xyz(&face.call_method0("normal")?.call_method0("normalized")?)?
                .dot(axis)
                .abs()
                <= 0.999
        {
            continue;
        }
        let level = (xyz(&face.call_method0("centroid")?)? - anchor).dot(direction);
        if level.abs() > eps
            && arrangement.call_method1(
                "loops_overlap",
                (
                    &footprint,
                    project.call1((face.getattr("vertices")?, &unit))?,
                ),
            )?
            .is_truthy()?
        {
            levels.append(level)?;
        }
    }
    Ok(none(py))
}
fn selected_faces<'py>(a: &Args<'py>) -> Reply<'py> {
    let scene = a.get_item(0)?;
    let py = a.py();
    let typ = module(py)?.getattr("MeshFace")?;
    let result = PyList::empty(py);
    for face in scene.getattr("selection")?.try_iter()? {
        let face = face?;
        if face.is_instance(&typ)? && scene.getattr("mesh")?.getattr("faces")?.contains(&face)? {
            result.append(face)?;
        }
    }
    Ok(result.into_any())
}
fn take_extras<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let face = a.get_item(1)?;
    let scene = a.get_item(2)?;
    let py = a.py();
    let faces = t.call_method1("_selected_faces", (&scene,))?;
    let extras = PyList::empty(py);
    let normals = PyList::empty(py);
    if faces.len()? < 2 || !faces.contains(&face)? {
        t.setattr("_extra_faces", extras)?;
        t.setattr("_extra_normals", normals)?;
        return Ok(none(py));
    }
    let mesh = scene.getattr("mesh")?;
    let id = imported(py, "built_core.mesh")?.getattr("element_identity")?;
    let order = PyDict::new(py);
    for (i, f) in mesh.getattr("faces")?.try_iter()?.enumerate() {
        order.set_item(id.call1((f?,))?, i)?;
    }
    let mut rows = Vec::new();
    for f in faces.try_iter()? {
        let f = f?;
        if f.ne(&face)? {
            let i = order
                .call_method1("get", (id.call1((&f,))?, 0))?
                .extract::<usize>()?;
            rows.push((i, f));
        }
    }
    rows.sort_by_key(|x| x.0);
    for (_, f) in &rows {
        extras.append(f)?;
    }
    t.setattr("_extra_faces", &extras)?;
    for (_, f) in &rows {
        orient(&mesh, f)?;
    }
    for (_, f) in rows {
        normals.append(t.call_method1("_outward_normal", (f, &mesh))?)?;
    }
    t.setattr("_extra_normals", normals)?;
    Ok(none(py))
}

fn host_hover_face<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let py = a.py();
    t.setattr("_inference_point", py.None())?;
    t.setattr("_inference_kind", py.None())?;
    let hovered = t.getattr("hovered_face")?;
    if t.getattr("_selected_arm")?.is_truthy()? && !hovered.is_none() {
        if vp
            .getattr("scene")?
            .getattr("mesh")?
            .getattr("faces")?
            .contains(&hovered)?
        {
            vp.call_method1("set_hover", (hovered,))?;
            return Ok(none(py));
        }
        t.setattr("_selected_arm", false)?;
    }
    let bake = imported(py, "views.array_bake")?;
    let (face, group): (Obj, Obj) = read_faces(&bake, || {
        vp.call_method1("pick_face_any", screen(&t.getattr("_ctx")?)?)
    })?
    .extract()?;
    t.setattr("hovered_face", &face)?;
    t.setattr("_hover_group", &group)?;
    if group.is_none() && face.is_instance(&bake.getattr("LightFace")?)? {
        t.setattr("hovered_face", face.call_method0("real")?)?;
    }
    vp.call_method1(
        "set_hover",
        (if group.is_none() {
            t.getattr("hovered_face")?
        } else {
            none(py)
        },),
    )?;
    Ok(none(py))
}
fn read_faces<'py>(bake: &Bound<'py, PyModule>, read: impl FnOnce() -> Reply<'py>) -> Reply<'py> {
    let guard = bake.call_method0("reading_only")?;
    let py = bake.py();
    guard.call_method0("__enter__")?;
    let result = read();
    match &result {
        Ok(_) => {
            guard.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
        }
        Err(e) => {
            guard.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?;
        }
    }
    result
}
fn reference_face<'py>(a: &Args<'py>) -> Reply<'py> {
    let vp = a.get_item(0)?;
    let pick = optional(&vp, "pick_face_placement")?;
    let pick = if pick.is_truthy()? {
        pick
    } else {
        vp.getattr("pick_face_any")?
    };
    read_faces(&imported(a.py(), "views.array_bake")?, || {
        pick.call1((a.get_item(1)?, a.get_item(2)?))
    })
}
fn reference_hidden<'py>(a: &Args<'py>) -> Reply<'py> {
    let vp = a.get_item(1)?;
    let p = a.get_item(2)?;
    let py = a.py();
    let cut = imported(py, "views.document_interaction")?
        .call_method1("_active_cut", (vp.getattr("scene")?,))?;
    if !cut.is_none() && cut.call_method1("side", (&p,))?.gt(1e-6)? {
        return true.into_bound_py_any(py);
    }
    let occluded = optional(&vp, "_is_occluded")?;
    if occluded.is_none() {
        return false.into_bound_py_any(py);
    }
    occluded.call1((p,))
}
fn infer_reference_distance<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let ctx = a.get_item(1)?;
    let py = a.py();
    let m = module(py)?;
    let hit = m.call_method1(
        "push_reference_target",
        (
            &t,
            ctx.getattr("viewport")?,
            screen(&ctx)?,
            m.getattr("_MIN_EXTRUDE")?,
        ),
    )?;
    let (p, k, d) = if hit.is_truthy()? {
        hit.extract::<(Obj, Obj, Obj)>()?
    } else {
        (none(py), none(py), none(py))
    };
    t.setattr("_inference_point", p)?;
    t.setattr("_inference_kind", k)?;
    Ok(d)
}
fn inference_marker<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let py = a.py();
    if !t.getattr("dragging")?.is_truthy()? {
        return Ok(none(py));
    }
    let p = t.getattr("_inference_point")?;
    if p.is_none() {
        return Ok(p);
    }
    (p, t.getattr("_inference_kind")?).into_bound_py_any(py)
}
fn inference_guide_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let py = a.py();
    let p = t.getattr("_inference_point")?;
    let lines = PyList::empty(py);
    if !p.is_none() && t.getattr("dragging")?.is_truthy()? {
        let moved = xyz(&t.getattr("_anchor")?)?
            + xyz(&t.getattr("_normal")?)? * t.getattr("extrusion")?.extract::<f64>()?;
        lines.append((p, Vec3::from(moved), t.getattr("_inference_kind")?))?;
    }
    Ok(lines.into_any())
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let py = a.py();
    if !t.getattr("dragging")?.is_truthy()? {
        return Ok(none(py));
    }
    let anchor = t.getattr("_anchor")?;
    if anchor.is_none() {
        return Ok(anchor);
    }
    let distance = t.getattr("extrusion")?.extract::<f64>()?;
    let mid = xyz(&anchor)? + xyz(&t.getattr("_normal")?)? * (distance * 0.5);
    (
        module(py)?.call_method1("fmt_len", (distance.abs(),))?,
        Vec3::from(mid),
    )
        .into_bound_py_any(py)
}
fn host_commit<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let distance = a.get_item(2)?;
    if !distance.is_none() {
        t.setattr("extrusion", distance)?;
        t.call_method1("_clamp_extrusion", (&vp,))?;
    }
    t.call_method1("_commit", (vp,))?;
    Ok(none(a.py()))
}
fn on_redo<'py>(a: &Args<'py>) -> Reply<'py> {
    let t = a.get_item(0)?;
    if t.getattr("dragging")?.is_truthy()? {
        t.call_method1("on_cancel", (a.get_item(1)?,))?;
    }
    false.into_bound_py_any(a.py())
}

fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_pointer_event", ("click", &ctx))?;
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_pointer_event", ("hover", &ctx))?;
    Ok(none(py))
}
fn on_double_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_pointer_event", ("double_click", &ctx))?;
    Ok(none(py))
}
fn on_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let mods = if tool.getattr("dragging")?.is_truthy()? {
        module.call_method1(
            "_mods",
            (module.getattr("window_input")?.call_method1("modifiers", (&viewport,))?,),
        )?
    } else {
        0.into_bound_py_any(py)?
    };
    tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "release", "mods" => &mods).into_any(),
        ),
    )?;
    Ok(none(py))
}
fn host_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_preview_at", (&viewport, tool.getattr("_ctx")?))?;
    tool.getattr("dragging")
}
fn host_close_retype<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.setattr("_last", none(py))?;
    Ok(none(py))
}
fn host_tiny_push<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let viewport = a.get_item(1)?;
    viewport.call_method1(
        "flash_status",
        (
            module.call_method1(
                "tr",
                ("Too small a push — drag further or type a distance",),
            )?,
            4000,
        ),
    )?;
    Ok(none(py))
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_light_wires")
}
fn preview_faces<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_light_faces")
}
fn build_light_faces<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let tool = a.get_item(0)?;
    let push_light_faces = imported(py, "built_core")?.getattr("push_light_faces")?;
    push_light_faces.call1((&tool, module.getattr("_MIN_EXTRUDE")?))
}
fn present<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let mode = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let present_push_preview = imported(py, "built_core")?.getattr("present_push_preview")?;
    present_push_preview.call1((
        &tool,
        &viewport,
        &mode,
        module.getattr("_MIN_EXTRUDE")?,
        module.getattr("_STITCH_TOL")?,
    ))?;
    Ok(none(py))
}
fn show_light_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_present", (&viewport, "overlay"))?;
    Ok(none(py))
}
fn refuse_closed_group<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let viewport = a.get_item(1)?;
    viewport.call_method1("flash_status", (module.call_method1("tr", ("That face belongs to a group — double-click to open it first, then push inside",))?, 4000,))?;
    Ok(none(py))
}
fn show_prism_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_present", (&viewport, "prism"))?;
    Ok(none(py))
}
fn show_staged_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_present", (&viewport, "stage"))?;
    Ok(none(py))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let key = a.get_item(2)?;
    let modifiers = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if key.eq(module.getattr("Qt")?.getattr("Key_Control")?)? {
        return Ok(tool.call_method1("_send", (&viewport, fields!(py; "type" => "key", "key" => "Control", "mods" => module.call_method1("_mods", (&modifiers,))?).into_any(),))?.get_item("consumed")?);
    }
    imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("PushPullTool")?, &tool))?
        .call_method1("on_key", (&viewport, &key, &modifiers))
}
fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if key.ne(module.getattr("Qt")?.getattr("Key_Control")?)? {
        return false.into_bound_py_any(py);
    }
    Ok(tool.call_method1("_send", (&viewport, fields!(py; "type" => "key_release", "key" => "Control", "tapped" => tool.call_method1("ctrl_tapped", (&viewport,))?).into_any(),))?.get_item("consumed")?)
}
fn host_modifier<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let values = a.get_item(2)?;
    {
        let [part0, part1] = unpack::<2>(&values)?;
        tool.setattr("_keep_mode", part0)?;
        tool.setattr("_keep_base", part1)?;
    }
    Ok(none(py))
}
fn host_show_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let enabled = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    viewport.call_method1(
        "flash_status",
        ((if enabled.is_truthy()? {
            module.call_method1("tr", ("Keep the base: on",))?
        } else {
            module.call_method1("tr", ("Keep the base: off",))?
        }),),
    )?;
    tool.call_method1("refresh_hint", (&viewport,))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn status_clause<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let tool = a.get_item(0)?;
    module.call_method1(
        "ctrl_clause",
        (
            tool.getattr("_keep_mode")?,
            "keep the base",
            "move the base",
        ),
    )
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("busy", Mode::Method, busy, &["self"], "", ""),
    ("host_reading", Mode::Method, host_reading, &["self", "viewport", "args"], "", ""),
    ("_pointer_event", Mode::Method, pointer_event, &["self", "kind", "ctx"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "", ""),
    ("on_double_click", Mode::Method, on_double_click, &["self", "ctx"], "", ""),
    ("on_release", Mode::Method, on_release, &["self", "viewport"], "", ""),
    ("host_activate", Mode::Method, host_activate, &["self", "viewport", "args"], "None", ""),
    ("host_deactivate", Mode::Method, host_deactivate, &["self", "viewport", "args"], "None", ""),
    ("drag_plane", Mode::Method, drag_plane, &["self", "viewport"], "", "Mid-drag, the plane through the anchor that CONTAINS the push\naxis and faces the camera — the cursor ray always hits it, so the\nviewport keeps delivering hovers and clicks wherever the cursor\ngoes. The ground it used otherwise stops answering above the\nhorizon (or when the ray merely grazes it), and then the box froze\nmid-pull and the click on its top was lost: «hago clic aquí para que\nquede fijado… no me deja, tengo que venirme al lateral» (Rafael,\n2026-09-16, camera at ground level). The distance itself is still\nread off the pixel (``_project_to_lock_line``); the plane only has\nto exist. Looking straight along the axis, where no plane contains\nit face-on, the base plane answers as before."),
    ("host_hover_face", Mode::Method, host_hover_face, &["self", "viewport", "args"], "", ""),
    ("host_preview", Mode::Method, host_preview, &["self", "viewport", "args"], "", ""),
    ("_preview_at", Mode::Method, preview_at, &["self", "viewport", "ctx"], "", ""),
    ("host_begin", Mode::Method, host_begin, &["self", "viewport", "pick"], "", ""),
    ("host_close_retype", Mode::Method, host_close_retype, &["self", "viewport", "args"], "", ""),
    ("host_tiny_push", Mode::Method, host_tiny_push, &["self", "viewport", "args"], "", ""),
    ("host_commit", Mode::Method, host_commit, &["self", "viewport", "distance"], "", ""),
    ("_grab_point", Mode::Method, grab_point, &["self", "viewport", "ctx:ToolContext"], "QVector3D | None", "The clicked spot on the face's plane, or ``None`` (the centroid\nthen stands in).\n\nThe drag is measured along the line through this point: the cursor\nstays on it. Through the centroid instead, a click near a corner of\na 12 m slab read -3.39 m for a 0.3 m pull, and a ring's centroid lies\nin its hole -- 1 m of cursor gave 0.13 m walls (tutorial hunt 2,\nA13/B5)."),
    ("host_restore_retype", Mode::Method, host_restore_retype, &["self", "viewport", "args"], "", "Recover the last push's handles only while its undo entry is still on top."),
    ("host_cancel", Mode::Method, host_cancel, &["self", "viewport", "args"], "", ""),
    ("on_redo", Mode::Method, on_redo, &["self", "viewport"], "bool", "Redo mid-drag abandons the push first, then lets the redo run (False)\n— on the clean model. Run under a staged preview, it would land on the\nstaged result and leave that in the model with no undo entry."),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
    ("preview_faces", Mode::Method, preview_faces, &["self"], "", ""),
    ("_build_light_faces", Mode::Method, build_light_faces, &["self"], "", ""),
    ("_present", Mode::Method, present, &["self", "viewport", "mode"], "", ""),
    ("_show_light_preview", Mode::Method, show_light_preview, &["self", "viewport"], "", ""),
    ("_refuse_closed_group", Mode::Method, refuse_closed_group, &["self", "viewport"], "None", "A face inside a group that has not been opened is not pushable.\n\nThe tool used to push it anyway — an instance even opened a short\nediting session behind your back and shared the result to every copy\n(2026-06-10). In a real drawing that reads as\nthe model changing where you did not point: a group is a box, and you\nopen a box before reaching inside (Marco, 2026-09-10) — the usual\nconvention: double-click the group or component to edit it first.\n\nRefusing has to SPEAK: a tool that does nothing without a word is the\none that reads as broken."),
    ("_show_prism_preview", Mode::Method, show_prism_preview, &["self", "viewport"], "None", ""),
    ("_cap_vertices", Mode::Method, cap_vertices, &["self", "scene"], "list", "The Vertex OBJECTS the cap loop stands on, resolved once."),
    ("_show_staged_preview", Mode::Method, show_staged_preview, &["self", "viewport"], "None", ""),
    ("inference_marker", Mode::Method, inference_marker, &["self"], "", "Return ``(world_point, kind)`` for the marker the viewport draws\nwhen the distance inference is locked onto model geometry, or\n``None``. The kind selects the matching snap colour and label,\nincluding the cyan midpoint cue."),
    ("inference_guide_lines", Mode::Method, inference_guide_lines, &["self"], "", "The «level with that point» reference segment, from the geometry\nthe distance inference locked onto to the moving cap, as\n``(reference, moved_anchor, kind)`` — ``[]`` while nothing is\nengaged."),
    ("value_label", Mode::Method, value_label, &["self"], "", "Return ``(text, midpoint_world)`` for the floating distance label.\nUses the anchor captured at drag start, which stays fixed even while a\nprism cap deforms the real geometry live."),
    ("_compute_plane_levels", Mode::Method, compute_plane_levels, &["self", "scene"], "None", "The levels (along the normal, both sides) of every face parallel to\nthe base that overlaps it laterally — where a staged preview's\ntopology changes: the far face a push punches through, a floor it\nlands flush on, an overhang a pull runs into. Stored in\n``self._plane_levels``; see _resolve_stage."),
    ("_reference_hidden", Mode::Method, reference_hidden, &["self", "vp", "world"], "bool", "Whether ``world`` is a reference the user cannot see: on the\nhidden side of the active section cut, or behind other geometry.\n\nThe snap engine runs both rules over its candidates (the viewport's\nsnap scene drops cut-hidden edges; ``_is_occluded`` raycasts the\nrest), but Push/Pull sets ``uses_snap = False`` and its mid-drag\ninference read vertices and edges the eye cannot reach — the extrusion\nlunged to an invisible corner as the cursor crossed it (the \"jumpy\npush\" reports, #270). X-ray and wireframe show everything:\n``_is_occluded`` is already False there, and the cut hides its side\nin every style — the same rule the snaps follow."),
    ("_infer_reference_distance", Mode::Method, infer_reference_distance, &["self", "ctx:ToolContext"], "", ""),
    ("_reference_face", Mode::Static, reference_face, &["viewport", "x", "y"], "", ""),
    ("_selected_faces", Mode::Static, selected_faces, &["scene"], "list", "The selected Faces of the loose mesh."),
    ("_take_extras", Mode::Method, take_extras, &["self", "face", "scene"], "None", "A drag starting on one of two or more selected faces takes the\nothers along; on any other face it is the ordinary single push.\n\nThe selection is read HERE, at every drag's start, not kept from the\nactivation: the pushed caps stay selected, and pushing the same\nfaces again moved only the one under the cursor."),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "", ""),
    ("on_key_release", Mode::Method, on_key_release, &["self", "viewport", "key"], "", ""),
    ("host_modifier", Mode::Method, host_modifier, &["self", "viewport", "values"], "", ""),
    ("host_show_mode", Mode::Method, host_show_mode, &["self", "viewport", "enabled"], "", ""),
    ("status_clause", Mode::Method, status_clause, &["self"], "str", ""),
];

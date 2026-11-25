// SPDX-License-Identifier: GPL-3.0-or-later
//! Host bindings for document_tools.
use super::*;
fn document_tools_init_document<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.document_tools")?;
    let tool = a.get_item(0)?;
    tool.setattr("camera", module.getattr("OrbitCamera")?.call0()?)?;
    tool.setattr("scene", module.getattr("Scene")?.call0()?)?;
    tool.call_method0("_bind_bake_store")?;
    let _units = py.import("built_core.units")?.into_any();
    _units.call_method1("bind_scene", (tool.getattr("scene")?,))?;
    tool.setattr(
        "history",
        module
            .getattr("History")?
            .call1((tool.getattr("scene")?,))?,
    )?;
    tool.setattr(
        "inference",
        module.getattr("InferenceEngine")?.call1((&tool,))?,
    )?;
    tool.setattr("active_tool", none(py))?;
    tool.setattr("_ext_overlays", PyList::empty(py).into_any())?;
    tool.setattr("_ext_snap_providers", PyList::empty(py).into_any())?;
    tool.setattr("clipboard", none(py))?;
    Ok(none(py))
}

fn document_tools_set_active_tool<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let next_tool = a.get_item(1)?;
    let view = a.get_item(0)?;
    let suspended = view.getattr("suspended_tool")?;
    if !suspended.is_none() {
        view.setattr("suspended_tool", none(py))?;
        if next_tool.is(&suspended) {
            view.call_method1("_resume_tool", (&next_tool,))?;
            return Ok(none(py));
        }
        suspended.call_method1("on_deactivate", (&view,))?;
    }
    if view.getattr("active_tool")?.is(&next_tool) && view.getattr("nav_mode")?.is_none() {
        if !next_tool.is_none() {
            next_tool.call_method1("on_cancel", (&view,))?;
        }
        view.getattr("inference")?
            .call_method0("clear_references")?;
        view.setattr("axis_lock", none(py))?;
        view.setattr("_shift_lock", none(py))?;
        view.setattr("reference_edge", none(py))?;
        view.setattr("reference_mode", none(py))?;
        view.setattr("linear_inference_mode", ("all").into_bound_py_any(py)?)?;
        view.call_method0("_refresh_snap")?;
        return Ok(none(py));
    }
    view.setattr("nav_mode", none(py))?;
    view.setattr("_last_pos", none(py))?;
    view.setattr("_orbit_pivot", none(py))?;
    view.setattr("_pan_mode", false)?;
    view.setattr("_look_drag", none(py))?;
    view.call_method0("unsetCursor")?;
    if !view.getattr("active_tool")?.is_none() {
        view.getattr("active_tool")?
            .call_method1("on_deactivate", (&view,))?;
    }
    view.setattr("active_tool", &next_tool)?;
    view.setattr("linear_inference_mode", ("all").into_bound_py_any(py)?)?;
    view.setattr("axis_lock", none(py))?;
    view.setattr("_hover_entity", none(py))?;
    view.getattr("inference")?
        .call_method0("clear_references")?;
    if !next_tool.is_none() {
        next_tool.call_method1("on_activate", (&view,))?;
    }
    view.call_method0("_apply_tool_cursor")?;
    view.getattr("measurementChanged")?
        .call_method1("emit", (view.call_method0("_measurement_text")?,))?;
    view.call_method0("update")?;
    Ok(none(py))
}

fn document_tools_resume_tool<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let next_tool = a.get_item(1)?;
    let view = a.get_item(0)?;
    view.setattr("nav_mode", none(py))?;
    view.setattr("_last_pos", none(py))?;
    view.setattr("_orbit_pivot", none(py))?;
    view.setattr("_pan_mode", false)?;
    view.setattr("_look_drag", none(py))?;
    view.setattr("active_tool", &next_tool)?;
    let resume = optional(&next_tool, "on_resume")?;
    if resume.is_callable() {
        resume.call1((&view,))?;
    }
    view.call_method0("_apply_tool_cursor")?;
    view.call_method0("_refresh_snap")?;
    view.getattr("measurementChanged")?
        .call_method1("emit", (view.call_method0("_measurement_text")?,))?;
    view.call_method0("update")?;
    Ok(none(py))
}

fn document_tools_begin_group_edit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let group = a.get_item(1)?;
    let module = py.import("views.document_tools")?;
    let scene = view.getattr("scene")?;
    let ancestors = py
        .import("built_core.outliner")?
        .call_method1("path_to", (scene.getattr("groups")?, &group))?;
    let mut path = Vec::new();
    if ancestors.is_truthy()? {
        path = ancestors.try_iter()?.collect::<PyResult<_>>()?;
    }
    for item in path.iter().chain(std::iter::once(&group)) {
        if attr_flag(item, "locked", false)?
            || scene
                .call_method1("_layer_state", (item,))?
                .get_item(1)?
                .is_truthy()?
        {
            view.call_method1(
                "flash_status",
                (module.call_method1("tr", ("Unlock it first",))?, 3000),
            )?;
            return Ok(none(py));
        }
    }
    let links = py.import("built_core.links")?;
    let mut linked = crate::host_context::core_module(py)?.call_method1("link_linked_ancestor", (&group,))?;
    if !linked.is_truthy()? {
        for ancestor in path.iter().rev() {
            if links.call_method1("is_linked", (ancestor,))?.is_truthy()? {
                linked = ancestor.clone();
                break;
            }
        }
    }
    if !linked.is_none() && linked.is_truthy()? {
        view.call_method1("_offer_link_source", (linked,))?;
        return Ok(none(py));
    }
    let parent = scene.getattr("edit_group")?;
    let transformed = !optional(&group, "xform")?.is_none();
    let component = attr_flag(&group, "component", true)?;
    if transformed && !component {
        let in_definition = !parent.is_none()
            && optional(&parent, "children")?
                .getattr_opt("__contains__")?
                .map(|contains| contains.call1((&group,))?.is_truthy())
                .transpose()?
                .unwrap_or(false)
            && attr_flag(&parent, "component", true)?;
        if !in_definition
            && view
                .call_method1("_mesh_shared_elsewhere", (&group,))?
                .is_truthy()?
        {
            py.import("built_core.command_protocol")?
                .call_method1("make_unique", (view.getattr("history")?, &group))?;
        }
    }
    let was_instance = !optional(&group, "xform")?.is_none()
        && (attr_flag(&group, "component", true)? || !optional(&group, "children")?.is_truthy()?);
    let before = module.call_method1("_context_stamp", (&view,))?;
    scene.call_method1("begin_group_edit", (&group,))?;
    module.call_method1("_ctx_changed", (&view, before))?;
    let mark = view.getattr("history")?.getattr("undo_stack")?.len()?;
    view.setattr("_edit_undo_mark", mark)?;
    let stack = scene.getattr("_edit_stack")?;
    let share = if stack.is_truthy()? {
        let level = stack.get_item(-1)?;
        level.set_item("undo_mark", mark)?;
        level.call_method1("get", ("share",))?
    } else {
        none(py)
    };
    view.setattr("_hover_entity", py.None())?;
    view.setattr("_edges_version", -1)?;
    if was_instance && !share.is_none() {
        let proto = share.get_item(1)?;
        let mut count = 1;
        for g in scene.getattr("groups")?.try_iter()? {
            if g?.getattr("mesh")?.is(&proto) {
                count += 1;
            }
        }
        let kw = fields!(py;"name"=>group.getattr("name")?,"n"=>count);
        let label=module.call_method("tr",("Editing component '{name}' — the change reaches its {n} copies when you leave (Esc); Make Unique first to edit one copy only",),Some(&kw))?;
        view.call_method1("flash_status", (label, 6000))?;
    } else {
        let kw = fields!(py;"path"=>view.call_method0("edit_path_text")?);
        let label = module.call_method(
            "tr",
            ("Editing {path} — Esc or click outside goes up one level",),
            Some(&kw),
        )?;
        view.call_method1("flash_status", (label, 5000))?;
    }
    view.call_method0("update")?;
    Ok(none(py))
}

fn document_tools_mesh_shared_elsewhere<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let group = a.get_item(1)?;
    let mesh = group.getattr("mesh")?;
    let groups = py
        .import("built_core")?
        .call_method1("purge_iter_groups", (view.getattr("scene")?.getattr("groups")?,))?;
    for g in groups.try_iter()? {
        let g = g?;
        if !g.is(&group) && g.getattr("mesh")?.is(&mesh) {
            return true.into_bound_py_any(py);
        }
    }
    false.into_bound_py_any(py)
}

fn document_tools_edit_path_text<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let scene = a.get_item(0)?.getattr("scene")?;
    let groups = py
        .import("built_core.outliner")?
        .call_method1("open_path", (scene,))?;
    let names = groups
        .try_iter()?
        .map(|g| g?.getattr("name")?.extract::<String>())
        .collect::<PyResult<Vec<_>>>()?;
    names.join(" ▸ ").into_bound_py_any(py)
}

fn document_tools_end_one_group_edit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.call_method(
        "_leave_group_edit",
        (),
        Some(&fields!(py; "todos" => false)),
    )?;
    Ok(none(py))
}

fn document_tools_set_document<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let scene = a.get_item(1)?;
    let history = a.get_item(2)?;
    view.call_method0("end_group_edit")?;
    let previous = view.getattr("scene")?;
    let old_history = view.getattr("history")?;
    view.getattr("inference")?.call_method0("reset")?;
    view.call_method0("reset_document_caches")?;
    let version = scene.getattr("version")?;
    let seen = view
        .getattr_opt("_versions_seen")?
        .unwrap_or(0.into_bound_py_any(py)?);
    let next = py
        .import("builtins")?
        .call_method1("max", (seen, previous.getattr("version")?, &version))?
        .add(1)?;
    let cached = optional(&scene, "_bounds_cache")?;
    let same = !cached.is_none() && cached.get_item(0)?.eq(&version)?;
    scene.setattr("version", &next)?;
    view.setattr("_versions_seen", &next)?;
    scene.setattr(
        "hide_similar_components",
        attr_flag(&previous, "hide_similar_components", false)?,
    )?;
    if same {
        let key = scene.call_method0("_bounds_key")?;
        let tail = pyo3::types::PySlice::new(py, 1, isize::MAX, 1);
        if cached
            .get_item(2)?
            .get_item(&tail)?
            .eq(key.get_item(&tail)?)?
        {
            scene.setattr("_bounds_cache", (&next, cached.get_item(1)?, key))?;
        }
    }
    view.setattr("scene", &scene)?;
    view.setattr("history", &history)?;
    let watch = view
        .getattr("__dict__")?
        .call_method1("get", ("_document_watch",))?;
    if !watch.is_none() && !history.is(&old_history) {
        view.call_method0("_unsubscribe_changes")?;
        view.setattr(
            "_unsubscribe_changes",
            history
                .getattr("feed")?
                .call_method1("subscribe", (watch,))?,
        )?;
    }
    view.call_method0("_bind_bake_store")?;
    py.import("built_core.units")?
        .call_method1("bind_scene", (scene,))?;
    view.setattr("_edges_version", -1)?;
    for name in [
        "_hover_entity",
        "_hover_edge",
        "last_snap",
        "reference_edge",
        "reference_mode",
    ] {
        view.setattr(name, py.None())?;
    }
    view.call_method0("notify_scene_changed")?;
    (previous, old_history).into_bound_py_any(py)
}

fn document_tools_end_group_edit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.call_method("_leave_group_edit", (), Some(&fields!(py; "todos" => true)))?;
    Ok(none(py))
}

fn document_tools_leave_group_edit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let all = a.get_item(1)?.is_truthy()?;
    let scene = view.getattr("scene")?;
    if scene.getattr("edit_group")?.is_none() {
        return Ok(none(py));
    }
    while all && scene.getattr("_edit_stack")?.len()? > 1 {
        view.call_method("_leave_group_edit", (), Some(&fields!(py;"todos"=>false)))?;
    }
    let stack = scene.getattr("_edit_stack")?;
    let level = if stack.is_truthy()? {
        stack.get_item(-1)?
    } else {
        PyDict::new(py).into_any()
    };
    let share = level.call_method1("pop", ("share", py.None()))?;
    let closing = scene.getattr("edit_group")?;
    let module = py.import("views.document_tools")?;
    let before = module.call_method1("_context_stamp", (&view,))?;
    scene.call_method0(if all {
        "end_group_edit"
    } else {
        "end_one_group_edit"
    })?;
    let history = view.getattr("history")?;
    let mut updated = None;
    if share.is_none() && level.contains("session")? {
        let occurrence = py.import("built_core.history")?.getattr("OccurrenceCommands")?;
        let session = level.get_item("session")?;
        if closing.call_method0("is_component")?.is_truthy()?
            && occurrence
                .call_method1("group_start", (&history, &session))?
                .lt(history.getattr("undo_stack")?.len()?)?
        {
            updated = Some((closing.clone(), closing.getattr("mesh")?));
        }
        occurrence.call_method1("fold", (&history, session))?;
    }
    module.call_method1("_ctx_changed", (&view, before))?;
    view.setattr("_hover_entity", py.None())?;
    view.setattr("_edges_version", -1)?;
    if !share.is_none()
        && py
            .import("built_core.reference_edit")?
            .call_method1("record_leave", (&history, &level, &share))?
            .is_truthy()?
    {
        updated = Some((share.get_item(0)?, share.get_item(1)?));
    }
    let (label, millis) = if let Some((group, proto)) = updated {
        let mut count = 0;
        for pair in scene.call_method0("placements")?.try_iter()? {
            if pair?.get_item(0)?.getattr("mesh")?.is(&proto) {
                count += 1;
            }
        }
        let kw = fields!(py;"name"=>group.getattr("name")?,"n"=>count);
        (
            module.call_method(
                "tr",
                ("Component '{name}' updated on its {n} copies",),
                Some(&kw),
            )?,
            4000,
        )
    } else if share.is_none() && !scene.getattr("edit_group")?.is_none() {
        let kw = fields!(py;"path"=>view.call_method0("edit_path_text")?);
        (
            module.call_method("tr", ("Up one level — now in {path}",), Some(&kw))?,
            4000,
        )
    } else {
        (module.call_method1("tr", ("Left the group",))?, 2000)
    };
    view.call_method1("flash_status", (label, millis))?;
    view.call_method0("notify_scene_changed")?;
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const DOCUMENT_TOOLS: &[Method] = &[
    ("_init_document", Mode::Method, document_tools_init_document, &["self"], "", ""),
    ("set_active_tool", Mode::Method, document_tools_set_active_tool, &["self", "tool:Optional[Tool]"], "None", ""),
    ("_resume_tool", Mode::Method, document_tools_resume_tool, &["self", "tool:Tool"], "None", "Back from a camera mode to the tool it suspended, as it was: the\nfirst point placed, the rubber band, the locks. Only the camera\ndrag ends."),
    ("begin_group_edit", Mode::Method, document_tools_begin_group_edit, &["self", "group"], "None", "Enter a group for editing (double-click-into-group). A\ncomponent's tools work in the frame of the selected occurrence.\n\nA locked group (the Browser's padlock), or one inside a locked\ngroup, does not open — whichever road asks: a double-click, the\nright-click's Edit Group or a part clicked in the Parts tab."),
    ("_mesh_shared_elsewhere", Mode::Method, document_tools_mesh_shared_elsewhere, &["self", "group"], "bool", "Whether another placement — top level or nested at any depth —\ndraws ``group``'s mesh. A group exploded out of a component copy\nshares its mesh with the group still inside the other copies\n(issue #97), and the top-level scan alone did not see it."),
    ("edit_path_text", Mode::Method, document_tools_edit_path_text, &["self"], "str", "Where you are, as a path: ``Plaza ▸ Jardinera ▸ Banca``.\n\nNesting without this reads as a broken program: you click outside,\nthe program correctly steps up ONE level into a parent that is also\na container, everything is still faded, and nothing seems to have\nhappened («hago clic en ese grupo que me sale atenuado y sigue así»,\nMarco, 2026-09-11 — he was two levels deep and had no way to know)."),
    ("end_one_group_edit", Mode::Method, document_tools_end_one_group_edit, &["self"], "None", "Step out ONE level — Esc, which leaves you inside the\nparent when the group you were editing lived in another group."),
    ("set_document", Mode::Method, document_tools_set_document, &["self", "scene", "history"], "tuple", "Show another document: ``scene`` with its own undo ``history``\n(for an extension's workspace, ``ExtensionApp.enter_workspace``).\nReturns the ``(scene, history)`` pair it replaces, untouched, to be\nhanded back later — the parked document keeps its undo steps and its\nselection.\n\nA document boundary like New / Open: the id()-keyed chunk caches\nare reset (:meth:`reset_document_caches`), and since the other\nrender caches remember the scene VERSION they were built for, not\nwhich scene it was, the incoming scene is moved past every version\nthis viewport has shown before it is drawn."),
    ("end_group_edit", Mode::Method, document_tools_end_group_edit, &["self"], "None", "Leave every open group, back to the model. What the menus and the\nsave/export paths call."),
    ("_leave_group_edit", Mode::Method, document_tools_leave_group_edit, &["self", "todos:bool"], "None", ""),
];

fn context_stamp<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let vp = a.get_item(0)?;
    let feed = optional(&optional(&vp, "history")?, "feed")?;
    (
        optional(&vp.getattr("scene")?, "content_version")?,
        (if feed.is_truthy()? {
            feed.getattr("readers")?.call_method0("stamp")?
        } else {
            (0).into_bound_py_any(py)?
        }),
    )
        .into_bound_py_any(py)
}

fn ctx_changed<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let before = a.get_item(1)?;
    let vp = a.get_item(0)?;
    let cc = optional(&optional(&vp, "history")?, "context_changed")?;
    if !cc.is_none() {
        cc.call1((&before,))?;
    }
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_context_stamp", Mode::Function, context_stamp, &["vp"], "", ""),
    ("_ctx_changed", Mode::Function, ctx_changed, &["vp", "before"], "", "``History.context_changed`` (stub viewports in tests have no\nhistory, or one without it)."),
];

//! Controller, display and change-feed lifetime of a document tab.
use super::*;

pub(super) fn doc_attr<'p>(w: &Obj<'p>, name: &str) -> PyResult<Obj<'p>> {
    let doc = optional(w, "_doc")?;
    if doc.is_none() {
        Ok(doc)
    } else {
        doc.getattr(name)
    }
}
pub(super) fn invalidate_tabs(w: &Obj<'_>) -> PyResult<()> {
    let view = doc_attr(w, "stand_in")?;
    if !view.is_none() {
        view.call_method0("update")?;
    }
    Ok(())
}
pub(super) fn make_document<'p>(w: &Obj<'p>, vp: &Obj<'p>, view: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = w.py();
    let doc = py
        .import("built_render.shell_views")?
        .call_method0("ShellDocument")?;
    doc.setattr("viewport", vp)?;
    doc.setattr("stand_in", view)?;
    doc.setattr("_renderer_wgpu", true)?;
    doc.setattr(
        "saved_content",
        vp.getattr("scene")?.getattr("content_version")?,
    )?;
    py.import(RENDER)?
        .call_method1("attach_viewport_extensions", (w, vp))?;
    let watch = py
        .import("built_core")?
        .call_method1("DocumentWatch", (&doc,))?;
    vp.setattr("_document_watch", &watch)?;
    vp.setattr(
        "_unsubscribe_changes",
        vp.getattr("history")?
            .getattr("feed")?
            .call_method1("subscribe", (watch,))?,
    )?;
    Ok(doc)
}
pub(crate) fn release_watch(doc: &Obj<'_>) -> PyResult<()> {
    let fields = doc.getattr("viewport")?.getattr("__dict__")?;
    let unsubscribe = fields.call_method1("pop", ("_unsubscribe_changes", doc.py().None()))?;
    if !unsubscribe.is_none() {
        unsubscribe.call0()?;
    }
    fields.call_method1("pop", ("_document_watch", doc.py().None()))?;
    Ok(())
}
pub(super) fn new_document<'p>(w: &Obj<'p>, activate: bool) -> PyResult<Obj<'p>> {
    let py = w.py();
    let host = w.getattr("_host")?;
    let (vp, view) = crate::shell_host::document_views(&host)?;
    let doc = make_document(w, &vp, &view)?;
    doc.setattr("untitled_no", w.call_method0("_free_untitled_no")?)?;
    let docs = w.getattr("_documents")?;
    let current = w.getattr("_doc")?;
    let pos = if docs.contains(&current)? {
        docs.call_method1("index", (&current,))?
            .extract::<usize>()?
            + 1
    } else {
        docs.len()?
    };
    docs.call_method1("insert", (pos, &doc))?;
    vp.call_method1("set_active_tool", (py.None(),))?;
    if current.is_none() {
        w.setattr("_doc", &doc)?;
    }
    if activate {
        w.call_method1("activate_document", (&doc,))?;
    }
    Ok(doc)
}
pub(super) fn activate_document(w: &Obj<'_>, doc: &Obj<'_>) -> PyResult<()> {
    let py = w.py();
    let old = w.getattr("_doc")?;
    if doc.is_none()
        || doc.is(&old)
        || !w.getattr("_documents")?.contains(doc)?
        || !w.getattr("_workspace")?.is_none()
    {
        return Ok(());
    }
    w.getattr("scene_player")?.call_method0("cancel")?;
    if !old.is_none() {
        old.getattr("model_views")?
            .call_method1("suspend", (&old,))?;
        let previous = old.getattr("viewport")?;
        if !previous.is_none() {
            py.import(RENDER)?.call_method1("leave_camera", (&old,))?;
            previous.call_method1("set_active_tool", (py.None(),))?;
            previous.call_method0("end_group_edit")?;
        }
    }
    let display = optional(&old, "stand_in")?;
    let renderer = optional(&display, "renderer")?;
    if !display.is_none() {
        display.setattr("renderer", py.None())?;
    }
    w.setattr("_doc", doc)?;
    let host = w.getattr("_host")?;
    if !renderer.is_none() {
        host.call_method1("hand_renderer", (doc.getattr("stand_in")?, renderer))?;
    }
    let vp = doc.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    py.import("built_core.units")?
        .call_method1("bind_scene", (&scene,))?;
    py.import("built_core.axes")?
        .call_method1("sync", (optional(&scene, "drawing_frame")?,))?;
    w.call_method1("_activate_tool", ("select",))?;
    vp.call_method0("prune_stale_selection")?;
    vp.call_method0("update")?;
    let kw = PyDict::new(py);
    kw.set_item("force", true)?;
    vp.getattr("_document_watch")?
        .call_method("observe", (), Some(&kw))?;
    py.import("built_render.shell_project")?
        .call_method1("persist", (w,))?;
    host.call_method0("request_frame")?;
    Ok(())
}
fn notify_hooks(w: &Obj<'_>, key: &str, where_: &str, scene: Option<&Obj<'_>>) -> PyResult<()> {
    if let Some(hooks) = w.getattr_opt(key)? {
        let snapshot: Vec<Obj<'_>> = hooks.try_iter()?.collect::<PyResult<_>>()?;
        let host = w.getattr("_host")?;
        for callback in snapshot {
            if let Some(scene) = scene {
                host.call_method1("_guard", (where_, callback, scene))?;
            } else {
                host.call_method1("_guard", (where_, callback))?;
            }
        }
    }
    Ok(())
}
pub(super) fn document_changed(w: &Obj<'_>, doc: &Obj<'_>, version: &Obj<'_>) -> PyResult<()> {
    if doc.is(&w.getattr("_doc")?) {
        w.call_method0("_update_title")?;
        notify_hooks(w, "_doc_change_hooks", "extension document changed", None)?;
    } else {
        invalidate_tabs(w)?;
    }
    let sheet = doc.getattr("composer")?;
    if !sheet.is_none() {
        sheet.call_method1("_on_model_version", (version,))?;
    }
    w.getattr("_host")?.call_method0("request_frame")?;
    Ok(())
}
pub(super) fn retire_document(w: &Obj<'_>, doc: &Obj<'_>) -> PyResult<()> {
    let py = w.py();
    let docs = w.getattr("_documents")?;
    if !docs.contains(doc)? {
        return Ok(());
    }
    doc.getattr("model_views")?
        .call_method1("suspend", (doc,))?;
    release_watch(doc)?;
    docs.call_method1("remove", (doc,))?;
    let project = py.import("built_render.shell_project")?;
    project.call_method1("closed", (w, doc))?;
    project.call_method1("persist", (w,))?;
    let vp = doc.getattr("viewport")?;
    notify_hooks(
        w,
        "_doc_close_hooks",
        "extension document closed",
        Some(&vp.getattr("scene")?),
    )?;
    let kw = PyDict::new(py);
    kw.set_item("untitled", doc.getattr("untitled_no")?)?;
    let _ = py
        .import("built_core.autosave")?
        .call_method("clear", (doc.getattr("path")?,), Some(&kw));
    let view = doc.getattr("stand_in")?;
    if !view.is_none() {
        view.setattr("renderer", py.None())?;
        let _ = view.call_method1("closeEvent", (py.None(),));
    }
    w.getattr("_host")?
        .getattr("timers")?
        .call_method1("cancel_context", (vp,))?;
    for name in ["front_view", "viewport", "stand_in"] {
        doc.setattr(name, py.None())?;
    }
    w.getattr("_reclaim_timer")?.call_method1("start", (0,))?;
    Ok(())
}

use super::*;

/// A snapshot for the built-in script runner. The window and renderer remain
/// the sources of truth for each value.
pub(super) fn snapshot<'p>(h: &Bound<'p, Host>) -> PyResult<Bound<'p, PyDict>> {
    let py = h.py();
    let w = h.getattr("window")?;
    let doc = w.getattr("_doc")?;
    let d = PyDict::new(py);
    let vp = Host::viewport(h)?;
    let view = Host::view(h)?;
    let scene = vp.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let tool = vp.getattr("active_tool")?;
    let r = Host::renderer(h)?;
    let win_vars = w.getattr("__dict__")?;
    let view_vars = view.getattr("__dict__")?;
    let jobs = PyList::empty(py);
    for j in win_vars
        .call_method1("get", ("_chrome_jobs", PyTuple::empty(py)))?
        .try_iter()?
    {
        let j = j?;
        jobs.append((
            j.getattr("label")?,
            j.getattr("fraction")?,
            j.getattr("phase")?,
        ))?;
    }
    d.set_item("opening", jobs)?;
    let packed = view.getattr("packed")?;
    d.set_item(
        "instances",
        if packed.is_none() {
            0
        } else {
            packed.getattr("instances")?.len()?
        },
    )?;
    let path = Host::path(h)?;
    d.set_item(
        "path",
        if path.is_truthy()? {
            path.str()?.into_any()
        } else {
            none(py)
        },
    )?;
    d.set_item("version", scene.getattr("version")?)?;
    for (obj, attr) in [
        (&mesh, "faces"),
        (&mesh, "edges"),
        (&scene, "groups"),
        (&scene, "selection"),
    ] {
        let v = optional(obj, attr)?;
        d.set_item(attr, if v.is_none() { 0 } else { v.len()? })?;
    }
    d.set_item(
        "input",
        if r.hasattr("ui_input_state")? {
            r.call_method0("ui_input_state")?
        } else {
            PyDict::new(py).into_any()
        },
    )?;
    d.set_item("filter", view_vars.call_method1("get", ("_ui_filter", ""))?)?;
    let loaded = py.import("sys")?.getattr("modules")?;
    let mut qt = Vec::<String>::new();
    for k in loaded.try_iter()? {
        let key: String = k?.extract()?;
        if matches!(key.split('.').next(), Some("PySide6" | "shiboken6")) {
            qt.push(key);
        }
    }
    qt.sort();
    d.set_item("qt_modules", qt)?;
    d.set_item("model_layout", doc.getattr("view_layout")?)?;
    let panes = doc.getattr("model_views")?;
    d.set_item("model_pane", panes.getattr("active")?)?;
    d.set_item("model_dividers", panes.getattr("shares")?)?;
    d.set_item(
        "view_targets",
        r.getattr_opt("target_count")?
            .and_then(|v| v.extract::<usize>().ok())
            .unwrap_or(0),
    )?;
    d.set_item("dock_lens", view_vars.call_method1("get", ("_ui_lens", 0))?)?;
    d.set_item(
        "dock_mode",
        view_vars.call_method1("get", ("_ui_mode", ""))?,
    )?;
    d.set_item(
        "tool_busy",
        vp.call_method1("_tool_busy_for_delete", (&tool,))?,
    )?;
    d.set_item(
        "edit_group",
        optional(&scene.getattr("edit_group")?, "name")?,
    )?;
    let cluster = PyList::empty(py);
    if r.hasattr("ui_cluster_buttons")? {
        for row in r.call_method0("ui_cluster_buttons")?.try_iter()? {
            cluster.append(row?.get_item(0)?)?;
        }
    }
    d.set_item("cluster", cluster)?;
    d.set_item("nav_mode", vp.getattr("nav_mode")?)?;
    let camera = vp.getattr("camera")?;
    d.set_item("camera_distance", camera.getattr("distance")?)?;
    for method in ["forward", "eye"] {
        d.set_item(
            format!("camera_{method}"),
            camera.call_method0(method)?.call_method0("toTuple")?,
        )?;
    }
    d.set_item(
        "tool",
        if tool.is_none() {
            none(py)
        } else {
            tool.get_type().getattr("__name__")?
        },
    )?;
    d.set_item(
        "status",
        w.call_method0("statusBar")?
            .call_method0("currentMessage")?,
    )?;
    let docs = w.getattr("_documents")?;
    let names = PyList::empty(py);
    let links = PyList::empty(py);
    let read_only = PyList::empty(py);
    for item in docs.try_iter()? {
        let item = item?;
        let name = item.call_method0("display_name")?;
        names.append(&name)?;
        if !crate::document_registration::view::calls::link_host(&w, &item)?
            .get_item(0)?
            .is_none()
        {
            links.append(name)?;
        }
        read_only.append(optional(&item, "read_only")?.is_truthy()?)?;
    }
    d.set_item("documents", names)?;
    d.set_item("link_tabs", links)?;
    d.set_item("read_only", read_only)?;
    d.set_item(
        "windows",
        py.import("built_render.shell_windows")?
            .call_method0("hosts")?
            .len()?,
    )?;
    let rects = PyList::empty(py);
    if r.hasattr("ui_project_tab_rects")? {
        for rect in r.call_method0("ui_project_tab_rects")?.try_iter()? {
            let rect = rect?;
            rects.append(if rect.is_none() {
                rect
            } else {
                py.import("builtins")?.call_method1("list", (rect,))?
            })?;
        }
    }
    d.set_item("tab_rects", rects)?;
    d.set_item(
        "context_menu",
        if r.hasattr("ui_context_menu_rows")? {
            r.call_method0("ui_context_menu_rows")?
        } else {
            PyList::empty(py).into_any()
        },
    )?;
    d.set_item(
        "active",
        if docs.contains(&doc)? {
            docs.call_method1("index", (&doc,))?.extract::<i64>()?
        } else {
            -1
        },
    )?;
    d.set_item("dirty", w.call_method0("_is_dirty")?.is_truthy()?)?;
    d.set_item(
        "large_sheet",
        if r.is_none() {
            String::new()
        } else {
            r.call_method0("ui_large_sheet_state")?
                .get_item(0)?
                .extract()?
        },
    )?;
    d.set_item(
        "form_open",
        !r.is_none() && r.call_method0("ui_dialog_open")?.is_truthy()?,
    )?;
    let start = win_vars.call_method1("get", ("_start_page",))?;
    d.set_item("start_page", optional(&start, "wanted")?.is_truthy()?)?;
    d.set_item("errors", h.getattr("errors")?.len()?)?;
    d.set_item(
        "classic_window_loaded",
        loaded.contains("views.main_window")?,
    )?;
    d.set_item("blocked_widgets", PyList::empty(py))?;
    d.set_item(
        "not_here",
        py.import("builtins")?
            .call_method1("list", (h.getattr("not_here_log")?,))?,
    )?;
    d.set_item("custom_cursors", h.getattr("_custom_cursors")?.len()?)?;
    d.set_item("preedit", &h.borrow()._preedit)?;
    d.set_item("ime_area", h.getattr("_ime_area")?)?;
    d.set_item("clipboard", optional(&vp, "clipboard")?.is_truthy()?)?;
    let types: Vec<String> = py
        .import("built_render.shell_services")?
        .call_method0("pasteboard_types")?
        .get_item(0)?
        .extract()?;
    d.set_item(
        "pasteboard",
        types
            .into_iter()
            .filter(|t| t.contains("built"))
            .collect::<Vec<_>>(),
    )?;
    d.set_item(
        "project",
        if win_vars.call_method1("get", ("_project",))?.is_none() {
            none(py)
        } else {
            w.call_method0("query_worksession")?
        },
    )?;
    Ok(d)
}

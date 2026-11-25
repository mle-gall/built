//! Document ownership and the command-facing window used by the native shell.
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};

mod documents;
mod scenes;
mod status;
pub(crate) use documents::release_watch;
use documents::*;
pub use scenes::ShellScenes;
use status::ShellStatusBar;

type Obj<'py> = Bound<'py, PyAny>;
const RENDER: &str = "ingetrazo_render";
const FILES: &str = "ingetrazo_render.shell_files";

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn optional<'p>(owner: &Obj<'p>, name: &str) -> PyResult<Obj<'p>> {
    Ok(owner.getattr_opt(name)?.unwrap_or_else(|| none(owner.py())))
}
fn tr<'p>(py: Python<'p>, text: &Obj<'p>) -> PyResult<Obj<'p>> {
    py.import("core.i18n")?.call_method1("tr", (text,))
}

fn discover(py: Python<'_>) -> PyResult<Obj<'_>> {
    let panels = PyDict::new(py);
    panels.set_item("render_blender", "plugins._blender_panel")?;
    panels.set_item("python_console", "plugins.python_console")?;
    for name in ["ai_assistant", "ai_bridge", "model_info", "solid_inspector"] {
        panels.set_item(name, py.None())?;
    }
    let kw = PyDict::new(py);
    kw.set_item("native_panels", panels)?;
    py.import("plugins._native")?
        .call_method("discover_plugins", (), Some(&kw))
}

fn install_extensions(w: &Obj<'_>, discovery: &Obj<'_>) -> PyResult<()> {
    let py = w.py();
    let render = py.import(RENDER)?;
    w.setattr("_extension_closed", render.call_method0("ChangeCallbacks")?)?;
    w.setattr("_extension_menus", render.call_method0("ExtensionMenus")?)?;
    w.setattr(
        "_extension_startup_state",
        py.import("core.extension_manager")?
            .call_method0("extension_startup_state")?,
    )?;
    let plugins = discovery.get_item(0)?;
    w.setattr("_pending_extensions", &plugins)?;
    py.import("core.plugin_panels")?
        .call_method1("install", (w, &plugins))?;
    let running = PySet::empty(py)?;
    for plugin in plugins.try_iter()? {
        let plugin = plugin?;
        if plugin.getattr("panel")?.is_truthy()? {
            running.add(plugin.getattr("stem")?)?;
        }
    }
    w.setattr("_running_extensions", running)?;
    let errors = PyDict::new(py);
    for error in discovery.get_item(1)?.try_iter()? {
        let error = error?;
        let info = PyDict::new(py);
        info.set_item("error", error.getattr("error")?)?;
        info.set_item("path", error.getattr("path")?.str()?)?;
        errors.set_item(error.getattr("stem")?, info)?;
    }
    w.setattr("_extension_load_errors", errors)
}

fn install_tools(w: &Obj<'_>) -> PyResult<()> {
    let py = w.py();
    let render = py.import(RENDER)?;
    let tools = render.call_method0("build_tools")?;
    w.setattr("_tools", &tools)?;
    let action = render.getattr("shell_action")?;
    let kw = PyDict::new(py);
    kw.set_item("checkable", true)?;
    kw.set_item("toggled", w.getattr("_on_toggle_eyedropper")?)?;
    w.setattr(
        "_act_eyedropper",
        action.call((w, "Sample material"), Some(&kw))?,
    )?;
    kw.del_item("toggled")?;
    let actions = PyDict::new(py);
    let defaults = PyList::empty(py);
    for (key, tool) in tools.cast::<PyDict>()?.iter() {
        let name = tool.getattr_opt("name")?.unwrap_or_else(|| key.clone());
        let a = action.call((w, tr(py, &name)?), Some(&kw))?;
        let tip = optional(&tool, "description")?;
        let desc = if tip.is_truthy()? {
            tip
        } else {
            "".into_pyobject(py)?.into_any()
        };
        a.call_method1("setProperty", ("statusTip", tr(py, &desc)?))?;
        let keys = PyList::empty(py);
        let primary = if key.eq("select")? {
            "Space".into_pyobject(py)?.into_any()
        } else {
            optional(&tool, "shortcut")?
        };
        for sc in [primary, optional(&tool, "shortcut_alt")?] {
            if sc.is_truthy()? {
                keys.append(sc)?;
            }
        }
        a.call_method1("setShortcuts", (keys,))?;
        defaults.append((
            &key,
            a.call_method0("text")?,
            a.getattr("state")?.call_method0("portable_keys")?,
        ))?;
        actions.set_item(key, a)?;
    }
    w.setattr("_tool_actions", actions)?;
    w.setattr("_tool_defaults", defaults)?;
    w.setattr("_registry_actions", PyDict::new(py))?;
    w.call_method0("refresh_commands")?;
    let registry = w.getattr("_registry_actions")?;
    for (attr, id) in [
        ("_act_bridge", "menu:bridge"),
        ("_act_make_faces", "menu:make_faces"),
        ("_act_3dtext", "menu:text3d"),
    ] {
        w.setattr(attr, registry.get_item(id)?)?;
    }
    Ok(())
}

#[pyclass(module = "ingetrazo_render", dict, weakref)]
pub struct ShellWindow;
#[pymethods]
#[allow(non_snake_case)]
impl ShellWindow {
    #[new]
    #[pyo3(signature=(host,plugins=None))]
    fn new(py: Python<'_>, host: &Obj<'_>, plugins: Option<Obj<'_>>) -> PyResult<Py<Self>> {
        let window = Bound::new(py, Self)?;
        let w = window.as_any();
        let timers = host.getattr("timers")?;
        w.setattr("_host", host)?;
        let discovery = match plugins {
            Some(p) => p,
            None => discover(py)?,
        };
        install_extensions(w, &discovery)?;
        w.setattr(
            "_command_store",
            py.import("ingetrazo_core")?
                .call_method0("WindowCommands")?,
        )?;
        w.setattr("_command_actions", PyDict::new(py))?;
        w.setattr("_rest_shown_mode", "fade")?;
        w.setattr("_style_before_xray", (py.None(), py.None()))?;
        w.setattr("_status", py.get_type::<ShellStatusBar>().call1((host,))?)?;
        for name in ["_doc", "_tool_filter", "_workspace", "_session_path"] {
            w.setattr(name, py.None())?;
        }
        for name in [
            "_empty_on",
            "_opening_session",
            "_opening_document",
            "_open_in_progress",
            "_open_aborted",
            "_open_cancelled",
            "_closed",
        ] {
            w.setattr(name, false)?;
        }
        w.setattr("_documents", PyList::empty(py))?;
        w.setattr(
            "_reclaim_timer",
            timers.call_method1(
                "timer",
                (py.import(RENDER)?.getattr("reclaim_closed_documents")?,),
            )?,
        )?;
        let tray = py.import("types")?.call_method0("SimpleNamespace")?;
        tray.setattr("scenes", py.get_type::<ShellScenes>().call1((w,))?)?;
        tray.setattr(
            "materials",
            py.import("ingetrazo_core")?.call_method1(
                "PaintRecents",
                (py.import("tools.paint")?.getattr("PaintTool")?,),
            )?,
        )?;
        w.setattr("tray", tray)?;
        w.setattr(
            "scene_player",
            py.import("views.scene_playback")?
                .call_method1("WindowPlayback", (w, &timers))?,
        )?;
        w.setattr("file_openers", PyDict::new(py))?;
        w.setattr("tool_label", "")?;
        install_tools(w)?;
        Ok(window.unbind())
    }
    fn statusBar<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.getattr("_status")
    }
    fn window(slf: Bound<'_, Self>) -> Bound<'_, Self> {
        slf
    }
    fn start_system_move(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("_host")?.setattr("window_drag_requested", true)
    }
    fn isVisible(slf: &Bound<'_, Self>) -> PyResult<bool> {
        slf.getattr("_host")?.getattr("ns_view")?.is_truthy()
    }
    fn isActiveWindow(&self) -> bool {
        true
    }
    fn setWindowTitle(slf: &Bound<'_, Self>, title: &str) -> PyResult<()> {
        slf.getattr("_host")?.call_method1("set_title", (title,))?;
        Ok(())
    }
    #[getter]
    fn _last_command<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.getattr("_command_store")?.getattr("last_command")
    }
    fn _remember_command(slf: &Bound<'_, Self>, label: &Obj<'_>, run: &Obj<'_>) -> PyResult<()> {
        slf.getattr("_command_store")?
            .call_method1("remember", (label, run))?;
        Ok(())
    }
    fn _setup_autosave(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.call_method1("_configure_recovery", (slf.getattr("_autosave_timer")?,))?;
        Ok(())
    }
    fn refresh_commands(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.py()
            .import(RENDER)?
            .call_method1("bind_window_commands", (slf,))?;
        Ok(())
    }
    fn documents<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.getattr("_documents")?.call_method0("copy")
    }
    fn active_document<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.getattr("_doc")
    }
    fn all_viewports<'p>(slf: &Bound<'p, Self>) -> PyResult<Vec<Obj<'p>>> {
        let mut result = Vec::new();
        for doc in slf.getattr("_documents")?.try_iter()? {
            let vp = doc?.getattr("viewport")?;
            if !vp.is_none() {
                result.push(vp);
            }
        }
        Ok(result)
    }
    #[pyo3(signature=(activate=true))]
    fn new_document<'p>(slf: &Bound<'p, Self>, activate: bool) -> PyResult<Obj<'p>> {
        new_document(slf.as_any(), activate)
    }
    #[pyo3(signature=(doc,closing=false))]
    fn activate_document(slf: &Bound<'_, Self>, doc: &Obj<'_>, closing: bool) -> PyResult<()> {
        let _ = closing;
        activate_document(slf.as_any(), doc)
    }
    fn _document_changed(slf: &Bound<'_, Self>, doc: &Obj<'_>, version: &Obj<'_>) -> PyResult<()> {
        document_changed(slf.as_any(), doc, version)
    }
    fn _retire_document(slf: &Bound<'_, Self>, doc: &Obj<'_>) -> PyResult<()> {
        retire_document(slf.as_any(), doc)
    }
    fn bind(slf: &Bound<'_, Self>, viewport: &Obj<'_>, view: &Obj<'_>) -> PyResult<()> {
        let doc = make_document(slf.as_any(), viewport, view)?;
        slf.getattr("_documents")?.call_method1("append", (&doc,))?;
        slf.setattr("_doc", doc)?;
        slf.call_method0("_apply_new_document_units")?;
        Ok(())
    }
    #[getter]
    fn view<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        doc_attr(slf.as_any(), "stand_in")
    }
    fn _active_view<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let doc = slf.getattr("_doc")?;
        if doc.is_none() {
            return Ok(doc);
        }
        doc.getattr("model_views")?
            .call_method1("command_view", (&doc,))
    }
    fn _doc_is_dirty(slf: &Bound<'_, Self>, doc: &Obj<'_>) -> PyResult<bool> {
        let vp = optional(doc, "viewport")?;
        if vp.is_none() {
            return Ok(false);
        }
        if doc.is(&slf.getattr("_doc")?) {
            return slf.call_method0("_is_dirty")?.is_truthy();
        }
        vp.getattr("scene")?
            .getattr("content_version")?
            .ne(doc.getattr("saved_content")?)
    }
    fn _update_title(slf: &Bound<'_, Self>) -> PyResult<()> {
        let doc = slf.getattr("_doc")?;
        if doc.is_none() || doc.getattr("viewport")?.is_none() {
            return Ok(());
        }
        let mark = if slf.call_method0("_is_dirty")?.is_truthy()? {
            " *"
        } else {
            ""
        };
        let workspace = slf.getattr("_workspace")?;
        let title = if workspace.is_none() {
            format!("{}{mark} — Built", doc.call_method0("display_name")?.str()?)
        } else {
            format!("Built — {}{mark}", workspace.call_method0("title")?.str()?)
        };
        slf.call_method1("setWindowTitle", (title,))?;
        invalidate_tabs(slf.as_any())
    }
    #[pyo3(signature=(doc=None))]
    fn _update_tab(slf: &Bound<'_, Self>, doc: Option<Obj<'_>>) -> PyResult<()> {
        let _ = doc;
        invalidate_tabs(slf.as_any())
    }
    fn tab_title(slf: &Bound<'_, Self>, index: usize) -> PyResult<String> {
        let doc = slf.getattr("_documents")?.get_item(index)?;
        let mark = if Self::_doc_is_dirty(slf, &doc)? {
            " •"
        } else {
            ""
        };
        Ok(format!(
            "{}{mark}",
            doc.call_method0("display_name")?.str()?
        ))
    }
    fn _activate_nav(slf: &Bound<'_, Self>, key: &str) -> PyResult<()> {
        let vp = slf.getattr("viewport")?;
        vp.call_method1("set_nav_mode", (key,))?;
        slf.call_method0("_update_status_hint")?;
        let tools = vp.getattr("keys_stand_in")?.getattr("tools")?;
        tools.call_method0("sync")?;
        tools.call_method0("_apply_tool_cursor")?;
        slf.getattr("view")?.call_method0("update")?;
        Ok(())
    }
    fn _activate_tool(slf: &Bound<'_, Self>, key: &str) -> PyResult<()> {
        let tool = slf.getattr("_tools")?.call_method1("get", (key,))?;
        if tool.is_none() {
            slf.getattr("_status")?.call_method1(
                "showMessage",
                (format!("{key} is unavailable in this window"), 4000),
            )?;
            return Ok(());
        }
        let actions = slf.getattr("_tool_actions")?;
        if !slf.getattr("_workspace")?.is_none()
            && !actions
                .get_item(key)?
                .call_method0("isEnabled")?
                .is_truthy()?
        {
            return Ok(());
        }
        let vp = slf.getattr("viewport")?;
        vp.call_method1("set_active_tool", (&tool,))?;
        let display = vp.getattr("keys_stand_in")?;
        display.getattr("tools")?.call_method0("pick_up")?;
        display.call_method1("set_scene", (vp.getattr("scene")?,))?;
        display.call_method0("update")?;
        for (k, a) in actions.cast::<PyDict>()?.iter() {
            a.call_method1("setChecked", (k.eq(key)?,))?;
        }
        slf.call_method0("_update_status_hint")?;
        if !matches!(
            key,
            "select" | "texture_position" | "change_axes" | "scale_manager"
        ) {
            let repeat = slf
                .py()
                .import("functools")?
                .call_method1("partial", (slf.getattr("_activate_tool")?, key))?;
            let name = tool
                .getattr_opt("name")?
                .unwrap_or(key.into_pyobject(slf.py())?.into_any());
            slf.call_method1("_remember_command", (name, repeat))?;
        }
        Ok(())
    }
    fn _on_zoom_extents(slf: &Bound<'_, Self>) -> PyResult<()> {
        let bounds = slf.call_method0("_extents_bounds")?;
        if !bounds.is_none() {
            slf.getattr("_doc")?
                .getattr("model_views")?
                .call_method1("fit_bounds", (slf, bounds))?;
        }
        Ok(())
    }
    fn _on_zoom_selection(slf: &Bound<'_, Self>) -> PyResult<()> {
        let bounds = slf
            .getattr("viewport")?
            .getattr("scene")?
            .call_method0("selection_bounds")?;
        if bounds.get_item(0)?.is_none() {
            let msg = slf
                .py()
                .import("core.i18n")?
                .call_method1("tr", ("Nothing selected",))?;
            slf.getattr("_status")?
                .call_method1("showMessage", (msg, 2500))?;
        } else {
            slf.getattr("_doc")?
                .getattr("model_views")?
                .call_method1("fit_bounds", (slf, bounds))?;
        }
        Ok(())
    }
    fn own_style<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let vp = Self::_active_view(slf)?;
        let style = vp.getattr("scene")?.getattr("display_style")?;
        if vp.is(&slf.getattr("_doc")?.getattr("viewport")?) {
            slf.call_method0("keep_other_views_style")?;
            return Ok(style);
        }
        if vp.getattr("view_style")?.is_none() && !style.is_none() {
            vp.setattr("view_style", style.call_method0("copy")?)?;
        }
        vp.getattr("view_style")
    }
    fn set_active_style(slf: &Bound<'_, Self>, style: &Obj<'_>) -> PyResult<()> {
        let vp = Self::_active_view(slf)?;
        if vp.is(&slf.getattr("_doc")?.getattr("viewport")?) {
            slf.call_method0("keep_other_views_style")?;
            vp.getattr("scene")?.setattr("display_style", style)?;
        } else {
            vp.setattr("view_style", style)?;
        }
        slf.getattr("_host")?.call_method0("request_frame")?;
        Ok(())
    }
    #[pyo3(signature=(doc=None,ask=true))]
    fn close_document(slf: &Bound<'_, Self>, doc: Option<Obj<'_>>, ask: bool) -> PyResult<bool> {
        slf.py()
            .import(FILES)?
            .call_method1("close_document", (slf, doc, ask))?
            .is_truthy()
    }
    fn close_documents(slf: &Bound<'_, Self>, docs: &Obj<'_>) -> PyResult<bool> {
        let snapshot: Vec<Obj<'_>> = docs.try_iter()?.collect::<PyResult<_>>()?;
        let open = slf.getattr("_documents")?;
        for d in snapshot {
            if open.contains(&d)? && !slf.call_method1("close_document", (d,))?.is_truthy()? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    #[pyo3(signature=(name,*args))]
    fn run_command<'p>(
        slf: &Bound<'p, Self>,
        name: &str,
        args: &Bound<'p, PyTuple>,
    ) -> PyResult<Obj<'p>> {
        let py = slf.py();
        let args = PyTuple::new(
            py,
            [slf.as_any().clone(), name.into_pyobject(py)?.into_any()]
                .into_iter()
                .chain(args.iter())
                .collect::<Vec<_>>(),
        )?;
        py.import(FILES)?.call_method1("command", args)
    }
    #[pyo3(signature=(global_pos=None,locked_image=None,locked_group=None))]
    fn show_viewport_context_menu<'p>(
        slf: &Bound<'p, Self>,
        global_pos: Option<Obj<'p>>,
        locked_image: Option<Obj<'p>>,
        locked_group: Option<Obj<'p>>,
    ) -> PyResult<Obj<'p>> {
        let _ = global_pos;
        slf.py()
            .import("plugins._context_menu")?
            .call_method1("context_menu_show", (slf, locked_image, locked_group))
    }
    fn _set_scene_visibility(slf: &Bound<'_, Self>, attr: &str, on: bool) -> PyResult<()> {
        let vp = slf.getattr("viewport")?;
        vp.getattr("scene")?.setattr(attr, on)?;
        vp.call_method0("update")?;
        Ok(())
    }
    fn _set_survey_capture(
        slf: &Bound<'_, Self>,
        datum: &Obj<'_>,
        lo: &Obj<'_>,
        hi: &Obj<'_>,
    ) -> PyResult<()> {
        let py = slf.py();
        let vp = slf.getattr("viewport")?;
        let scene = vp.getattr("scene")?;
        let mut layer = scene.getattr("tile_layer")?;
        if layer.is_none() {
            layer = py.import("georef.tiles")?.call_method1(
                "TileLayer",
                (py.import(RENDER)?.call_method0("selected_tile_source")?,),
            )?;
        }
        layer.call_method1(
            "capture_bounds",
            (
                datum,
                lo.call_method0("x")?,
                lo.call_method0("y")?,
                hi.call_method0("x")?,
                hi.call_method0("y")?,
            ),
        )?;
        scene.setattr("tile_layer", layer)?;
        vp.call_method0("reset_tiles")?;
        Ok(())
    }
    #[pyo3(signature=(title,*,monotonic=false))]
    fn _import_progress<'p>(
        slf: &Bound<'p, Self>,
        title: &str,
        monotonic: bool,
    ) -> PyResult<Obj<'p>> {
        let kw = PyDict::new(slf.py());
        kw.set_item("monotonic", monotonic)?;
        slf.py()
            .import(FILES)?
            .call_method("import_progress", (slf, title), Some(&kw))
    }
    fn _load_photomesh_threaded<'p>(
        slf: &Bound<'p, Self>,
        path: &Obj<'p>,
        datum: &Obj<'p>,
        cb: &Obj<'p>,
    ) -> PyResult<Obj<'p>> {
        let py = slf.py();
        let kw = PyDict::new(py);
        kw.set_item("datum", datum)?;
        kw.set_item(
            "limit",
            slf.getattr("viewport")?.call_method0("max_texture_size")?,
        )?;
        let pair = py.import(FILES)?.call_method(
            "parse_threaded",
            (py.import(RENDER)?.getattr("read_survey_mesh")?, path, cb),
            Some(&kw),
        )?;
        let error = pair.get_item(1)?;
        let result = pair.get_item(0)?;
        let values = if error.is_none() {
            vec![result.get_item(0)?, result.get_item(1)?, error]
        } else {
            vec![none(py), none(py), error]
        };
        Ok(PyTuple::new(py, values)?.into_any())
    }
    #[pyo3(signature=(path,scale,simplify_mode,cb,report=None))]
    fn _parse_stl_threaded<'p>(
        slf: &Bound<'p, Self>,
        path: &Obj<'p>,
        scale: &Obj<'p>,
        simplify_mode: &Obj<'p>,
        cb: &Obj<'p>,
        report: Option<Obj<'p>>,
    ) -> PyResult<Obj<'p>> {
        slf.py().import("ingetrazo_core")?.call_method1(
            "parse_stl_threaded",
            (path, scale, simplify_mode, cb, report),
        )
    }
    #[getter]
    fn _current_path<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        doc_attr(slf.as_any(), "path")
    }
    #[setter(_current_path)]
    fn set_current_path(slf: &Bound<'_, Self>, value: &Obj<'_>) -> PyResult<()> {
        slf.getattr("_doc")?.setattr("path", value)
    }
    #[getter]
    fn _import_name<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        doc_attr(slf.as_any(), "import_name")
    }
    #[setter(_import_name)]
    fn set_import_name(slf: &Bound<'_, Self>, value: &Obj<'_>) -> PyResult<()> {
        slf.getattr("_doc")?.setattr("import_name", value)
    }
    #[getter]
    fn _saved_content<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        doc_attr(slf.as_any(), "saved_content")
    }
    #[setter(_saved_content)]
    fn set_saved_content(slf: &Bound<'_, Self>, value: &Obj<'_>) -> PyResult<()> {
        slf.getattr("_doc")?.setattr("saved_content", value)
    }
    #[getter]
    fn _autosaved_version<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        doc_attr(slf.as_any(), "autosaved_version")
    }
    #[setter(_autosaved_version)]
    fn set_autosaved_version(slf: &Bound<'_, Self>, value: &Obj<'_>) -> PyResult<()> {
        slf.getattr("_doc")?.setattr("autosaved_version", value)
    }
    #[getter]
    fn _composer<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        doc_attr(slf.as_any(), "composer")
    }
    #[setter(_composer)]
    fn set_composer(slf: &Bound<'_, Self>, value: &Obj<'_>) -> PyResult<()> {
        slf.getattr("_doc")?.setattr("composer", value)
    }
    #[getter]
    fn _last_sheet<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        doc_attr(slf.as_any(), "last_sheet")
    }
    #[setter(_last_sheet)]
    fn set_last_sheet(slf: &Bound<'_, Self>, value: &Obj<'_>) -> PyResult<()> {
        slf.getattr("_doc")?.setattr("last_sheet", value)
    }
    fn _show_model<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("views.wgpu_sheets")?
            .call_method1("to_model", (slf,))
    }
    fn _to_sheets<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("views.wgpu_sheets")?
            .call_method1("to_sheets", (slf,))
    }
    fn _pick_view_layout<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("app_shell.model_views")?
            .call_method1("choose", (slf,))
    }
    fn open_project<'p>(slf: &Bound<'p, Self>, path: &Obj<'p>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("app_shell.project")?
            .call_method1("open_project", (slf, path))
    }
    fn set_active_file<'p>(slf: &Bound<'p, Self>, path: &Obj<'p>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("app_shell.project")?
            .call_method1("set_active_file", (slf, path))
    }
    fn query_worksession<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("app_shell.project")?
            .call_method1("query", (slf,))
    }
    fn persist_worksession<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("app_shell.project")?
            .call_method1("persist", (slf,))
    }
    fn _refresh_worksession<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("app_shell.project")?
            .call_method1("refresh", (slf,))
    }
    fn _show_empty_area<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("ingetrazo_render.wgpu_start")?
            .call_method1("show", (slf,))
    }
    fn _dismiss_home<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("ingetrazo_render.wgpu_start")?
            .call_method1("hide", (slf,))
    }
    fn _on_import_orthophoto<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("views.survey_import")?
            .call_method1("import_orthophoto", (slf,))
    }
    fn _on_import_photomesh<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("views.survey_import")?
            .call_method1("import_photomesh", (slf,))
    }
    fn _on_import_georef<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("views.survey_import")?
            .call_method1("import_alignment", (slf,))
    }
    fn _on_import_survey_points<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import("views.survey_import")?
            .call_method1("import_points", (slf,))
    }
    fn open_path<'p>(slf: &Bound<'p, Self>, path: &Obj<'p>) -> PyResult<Obj<'p>> {
        slf.py()
            .import(FILES)?
            .call_method1("open_path", (slf, path))
    }
    fn _on_open<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py().import(FILES)?.call_method1("open_dialog", (slf,))
    }
    fn _on_save<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py().import(FILES)?.call_method1("save", (slf,))
    }
    fn _on_save_as<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py().import(FILES)?.call_method1("save_as", (slf,))
    }
    fn _on_export_igz<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py().import(FILES)?.call_method1("export_igz", (slf,))
    }
    fn _on_new_window<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py().import(FILES)?.call_method1("new_window", (slf,))
    }
    fn _sync_foreign_clipboard<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import(FILES)?
            .call_method1("sync_foreign_clipboard", (slf,))
    }
    fn _open_recent<'p>(slf: &Bound<'p, Self>, path: &Obj<'p>) -> PyResult<Obj<'p>> {
        slf.py()
            .import(FILES)?
            .call_method1("open_path", (slf, path))
    }
    fn enter_workspace<'p>(slf: &Bound<'p, Self>, workspace: &Obj<'p>) -> PyResult<Obj<'p>> {
        slf.py()
            .import(RENDER)?
            .call_method1("enter_workspace", (slf, workspace))
    }
    fn leave_workspace<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import(RENDER)?
            .call_method1("leave_workspace", (slf,))
    }
    fn _apply_display_style<'p>(slf: &Bound<'p, Self>, preset: &Obj<'p>) -> PyResult<Obj<'p>> {
        slf.py()
            .import(RENDER)?
            .call_method1("display_style", (slf, preset))
    }
    fn _toggle_xray<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py().import(RENDER)?.call_method1("toggle_xray", (slf,))
    }
    fn _set_style_field<'p>(
        slf: &Bound<'p, Self>,
        name: &Obj<'p>,
        value: &Obj<'p>,
    ) -> PyResult<Obj<'p>> {
        slf.py()
            .import(RENDER)?
            .call_method1("style_field", (slf, name, value))
    }
    fn _on_smart_zoom<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py().import(RENDER)?.call_method1("smart_zoom", (slf,))
    }
    fn _update_status_hint<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py().import(RENDER)?.call_method1("tool_hint", (slf,))
    }
    fn _on_standard_view<'p>(slf: &Bound<'p, Self>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
        slf.py()
            .import(RENDER)?
            .call_method1("standard_view", (Self::_active_view(slf)?, key))
    }
    fn _set_hidden_view<'p>(
        slf: &Bound<'p, Self>,
        attr: &Obj<'p>,
        on: &Obj<'p>,
    ) -> PyResult<Obj<'p>> {
        slf.py()
            .import(RENDER)?
            .call_method1("hidden_view", (slf.getattr("viewport")?, attr, on))
    }
    fn _on_toggle_shadows<'p>(slf: &Bound<'p, Self>, on: &Obj<'p>) -> PyResult<Obj<'p>> {
        slf.py()
            .import(RENDER)?
            .call_method1("set_shadows", (slf.getattr("viewport")?, on))
    }
    fn _rebuild_surfaces<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.py()
            .import(RENDER)?
            .call_method1("rebuild_path_surfaces", (slf.getattr("viewport")?,))
    }

    fn active_style<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        Self::_active_view(slf)?.call_method0("_view_style")
    }
    fn set_view_layout(slf: &Bound<'_, Self>, key: &str) -> PyResult<()> {
        let doc = slf.getattr("_doc")?;
        doc.getattr("model_views")?
            .call_method1("choose", (&doc, key))?;
        Ok(())
    }
    fn _views_dict<'p>(&self, doc: &Obj<'p>) -> PyResult<Obj<'p>> {
        doc.getattr("model_views")?.call_method0("archive")
    }
    fn _maximize_model_view(slf: &Bound<'_, Self>) -> PyResult<()> {
        let doc = slf.getattr("_doc")?;
        doc.getattr("model_views")?
            .call_method1("maximize", (&doc,))?;
        Ok(())
    }
    fn keep_other_views_style(slf: &Bound<'_, Self>) -> PyResult<()> {
        let doc = slf.getattr("_doc")?;
        doc.getattr("model_views")?
            .call_method1("preserve_peer_styles", (&doc,))?;
        Ok(())
    }
    fn _set_projection(slf: &Bound<'_, Self>, mode: &str) -> PyResult<()> {
        Self::_active_view(slf)?.call_method1("set_projection", (mode,))?;
        Ok(())
    }
    fn workspace<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.getattr("_workspace")
    }
    fn release_eyedropper(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.call_method1("_on_toggle_eyedropper", (false,))?;
        Ok(())
    }
    #[staticmethod]
    fn _pump_file_command(py: Python<'_>) -> PyResult<()> {
        py.import("app_shell.modal")?.call_method0("tick")?;
        Ok(())
    }
    #[staticmethod]
    fn _clipboard_available(py: Python<'_>) -> PyResult<bool> {
        py.import(RENDER)?
            .getattr("ModelClipboard")?
            .call_method0("available")?
            .is_truthy()
    }
    fn _on_preferences<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.call_method1("run_command", ("prefs",))
    }
    fn _on_manage_extensions<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        slf.call_method1("run_command", ("extensions",))
    }
    fn repaint_styled(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("_host")?.call_method0("request_frame")?;
        Ok(())
    }
    fn _wake_link_watch(slf: &Bound<'_, Self>) -> PyResult<()> {
        Self::repaint_styled(slf)
    }
    fn _on_player_active(slf: &Bound<'_, Self>, on: bool) -> PyResult<()> {
        let _ = on;
        Self::repaint_styled(slf)
    }
    fn _sync_animation_menu(slf: &Bound<'_, Self>) -> PyResult<()> {
        Self::repaint_styled(slf)
    }
}

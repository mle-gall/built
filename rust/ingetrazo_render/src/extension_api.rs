// SPDX-License-Identifier: GPL-3.0-or-later
//! The API v2 handle passed to setup(app). Callbacks always resolve the active
//! document at invocation, except subscriptions and picks bound to one history.
use pyo3::{
    class::gc::{PyTraverseError, PyVisit},
    exceptions::{PyRuntimeError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyFrozenSet, PyList, PyTuple},
};

type Obj<'py> = Bound<'py, PyAny>;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn hook_list<'py>(owner: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    if let Some(list) = owner.getattr_opt(name)? {
        return Ok(list);
    }
    let list = PyList::empty(owner.py());
    owner.setattr(name, &list)?;
    Ok(list.into_any())
}

#[pyclass(dict, weakref, subclass, module = "views.extension_api")]
struct ExtensionApp {
    window: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    key: String,
    unsubscribe: Option<Py<PyList>>,
    layout_restored: std::sync::atomic::AtomicBool,
}

impl ExtensionApp {
    fn win<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        self.window
            .as_ref()
            .map(|w| w.bind(py).clone())
            .ok_or_else(|| PyRuntimeError::new_err("extension window has closed"))
    }
    fn menu<'p>(&self, py: Python<'p>) -> PyResult<Option<Obj<'p>>> {
        let win = self.win(py)?;
        let Some(menus) = win
            .getattr("__dict__")?
            .cast_into::<PyDict>()?
            .get_item("_extension_menus")?
        else {
            return Ok(None);
        };
        if !menus.is_truthy()? {
            return Ok(None);
        }
        Ok(Some(menus.call_method1("root", (&self.key,))?))
    }
}

#[pymethods]
impl ExtensionApp {
    #[new]
    fn new(py: Python, window: Py<PyAny>, key: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            window: Some(window),
            key: key.str()?.to_string(),
            unsubscribe: Some(PyList::empty(py).unbind()),
            layout_restored: false.into(),
        })
    }
    #[classattr]
    #[pyo3(name = "api_version")]
    const API_VERSION: u8 = 2;
    #[getter]
    fn window<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        self.win(py)
    }
    #[getter]
    fn viewport<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        self.win(py)?.getattr("viewport")
    }
    #[getter]
    fn scene<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        self.viewport(py)?.getattr("scene")
    }
    #[getter]
    fn viewport_widget<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        let vp = self.viewport(py)?;
        if let Some(view) = vp.getattr_opt("keys_stand_in")? {
            if view.is_truthy()? {
                return Ok(view);
            }
        }
        Ok(vp)
    }
    fn __traverse__(&self, visit: PyVisit) -> Result<(), PyTraverseError> {
        visit.call(&self.window)?;
        visit.call(&self.unsubscribe)
    }
    fn __clear__(&mut self) {
        self.window = None;
        self.unsubscribe = None;
    }

    #[pyo3(signature=(name, r#fn, *, description, input_schema=None, mutates=false))]
    fn add_agent_tool<'p>(
        &self,
        py: Python<'p>,
        name: &str,
        r#fn: &Obj<'p>,
        description: &str,
        input_schema: Option<&Obj<'p>>,
        mutates: bool,
    ) -> PyResult<Obj<'p>> {
        let backend = py.import("core.agent_tools")?;
        let mut reserved = name == "list_tools";
        for item in backend.getattr("BUILTIN_TOOLS")?.try_iter()? {
            if item?.get_item("name")?.eq(name)? {
                reserved = true;
                break;
            }
        }
        if reserved {
            return Err(PyValueError::new_err(format!(
                "agent tool '{name}' is already registered"
            )));
        }
        let win = self.win(py)?;
        let registry = backend.call_method1("extension_registry", (&win,))?;
        let args = PyDict::new(py);
        args.set_item("description", description)?;
        args.set_item("input_schema", input_schema)?;
        args.set_item("mutates", mutates)?;
        let off = backend
            .getattr("register")?
            .call((registry, name, r#fn), Some(&args))?;
        self.unsubscribe.as_ref().unwrap().bind(py).append(&off)?;
        let signal = match win
            .getattr_opt("_extension_closed")?
            .filter(|v| !v.is_none())
        {
            Some(s) => s,
            None => win.getattr("destroyed")?,
        };
        signal.call_method1("connect", (&off,))?;
        Ok(off)
    }
    fn remove_agent_tools(&self, py: Python) -> PyResult<()> {
        if let Some(list) = &self.unsubscribe {
            for off in list.bind(py).iter() {
                off.call0()?;
            }
            list.bind(py).call_method0("clear")?;
        }
        Ok(())
    }

    fn restore_toolbar_layout(&self, py: Python) -> PyResult<bool> {
        if self
            .layout_restored
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Ok(false);
        }
        let win = self.win(py)?;
        let Some(restore) = win.getattr_opt("restore_late_toolbars")? else {
            return Ok(false);
        };
        let restored = restore.call0()?.is_truthy()?;
        self.layout_restored
            .store(restored, std::sync::atomic::Ordering::Relaxed);
        Ok(restored)
    }
    #[pyo3(signature=(*, before=None, after=None))]
    fn add_command_listener<'p>(
        &self,
        py: Python<'p>,
        before: Option<&Obj<'p>>,
        after: Option<&Obj<'p>>,
    ) -> PyResult<Obj<'p>> {
        let kw = PyDict::new(py);
        kw.set_item("before", before)?;
        kw.set_item("after", after)?;
        self.viewport(py)?
            .getattr("history")?
            .call_method("add_listener", (), Some(&kw))
    }
    #[pyo3(signature=(default=None))]
    fn document_data<'p>(&self, py: Python<'p>, default: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
        if let Some(data) = self.scene(py)?.getattr_opt("plugin_data")? {
            if data.is_truthy()? && data.contains(&self.key)? {
                let json = py.import("json")?;
                return json.call_method1(
                    "loads",
                    (json.call_method1("dumps", (data.get_item(&self.key)?,))?,),
                );
            }
        }
        Ok(default.cloned().unwrap_or_else(|| none(py)))
    }
    fn set_document_data(&self, py: Python, value: &Obj) -> PyResult<()> {
        let vp = self.viewport(py)?;
        let cmd = py
            .import("core.history")?
            .getattr("SetPluginDataCommand")?
            .call1((&self.key, value))?;
        vp.getattr("history")?.call_method1("execute", (cmd,))?;
        if let Some(notify) = vp
            .getattr_opt("notify_scene_changed")?
            .filter(|v| v.is_callable())
        {
            notify.call0()?;
        }
        vp.call_method0("update")?;
        Ok(())
    }
    #[pyo3(signature=(path, at=None))]
    fn import_igz<'p>(
        &self,
        py: Python<'p>,
        path: &Obj<'p>,
        at: Option<&Obj<'p>>,
    ) -> PyResult<Obj<'p>> {
        let kw = PyDict::new(py);
        kw.set_item("at", at)?;
        self.win(py)?
            .call_method("import_igz_path", (path,), Some(&kw))
    }
    fn on_document_changed(&self, py: Python, r#fn: &Obj) -> PyResult<()> {
        hook_list(&self.win(py)?, "_doc_change_hooks")?.call_method1("append", (r#fn,))?;
        Ok(())
    }
    fn on_document_closed(&self, py: Python, r#fn: &Obj) -> PyResult<()> {
        hook_list(&self.win(py)?, "_doc_close_hooks")?.call_method1("append", (r#fn,))?;
        Ok(())
    }

    #[pyo3(signature=(title, widget, name=None, *, panel=None, stretch=0, start_hidden=false, icon=None))]
    fn add_panel(
        &self,
        py: Python,
        title: &str,
        widget: &Obj,
        name: Option<&str>,
        panel: Option<&str>,
        stretch: i32,
        start_hidden: bool,
        icon: Option<&Obj>,
    ) -> PyResult<String> {
        let _ = (title, stretch, start_hidden, icon);
        if !widget.is_callable() {
            return Err(PyTypeError::new_err(
                "add_panel requires a native panel factory",
            ));
        }
        let key = match (
            panel.filter(|s| !s.is_empty()),
            name.filter(|s| !s.is_empty()),
        ) {
            (Some(panel), _) => panel.to_owned(),
            (_, Some(name)) => format!("{}_{name}", self.key),
            _ => self.key.clone(),
        };
        self.win(py)?
            .getattr("_panel_factories")?
            .call_method1("setdefault", (&key, widget))?;
        Ok(key)
    }
    fn show_panel<'p>(&self, py: Python<'p>, key: &str) -> PyResult<Obj<'p>> {
        py.import("core.plugin_panels")?
            .call_method1("open_panel", (self.win(py)?, key))
    }
    #[pyo3(signature=(text, r#fn, shortcut=None, tip=None))]
    fn add_menu_action<'p>(
        &self,
        py: Python<'p>,
        text: &str,
        r#fn: &Obj<'p>,
        shortcut: Option<&str>,
        tip: Option<&str>,
    ) -> PyResult<Option<Obj<'p>>> {
        self.menu(py)?
            .map(|m| m.call_method1("add_command", (text, r#fn, shortcut, tip)))
            .transpose()
    }
    fn add_menu<'p>(&self, py: Python<'p>, title: &str) -> PyResult<Option<Obj<'p>>> {
        self.menu(py)?
            .map(|m| m.call_method1("addMenu", (title,)))
            .transpose()
    }
    fn add_outliner_kind(&self, py: Python, r#fn: &Obj) -> PyResult<()> {
        let win = self.win(py)?;
        hook_list(&win, "_outliner_kinds")?.call_method1("append", (r#fn,))?;
        if let Some(browser) = win.getattr_opt("project_browser")?.filter(|b| !b.is_none()) {
            browser.getattr("outliner")?.call_method0("refresh")?;
        }
        Ok(())
    }
    fn add_context_menu(&self, py: Python, r#fn: &Obj) -> PyResult<()> {
        let win = self.win(py)?;
        hook_list(&win, "_ext_context_menus")?.call_method1("append", (r#fn,))?;
        win.getattr("__dict__")?
            .call_method1("setdefault", ("_ext_context_owner", PyDict::new(py)))?
            .set_item(r#fn, &self.key)
    }
    fn add_overlay(&self, py: Python, r#fn: &Obj) -> PyResult<()> {
        let vp = self.viewport(py)?;
        vp.getattr("_ext_overlays")?
            .call_method1("append", (r#fn,))?;
        vp.call_method0("update")?;
        Ok(())
    }
    fn add_sheet_annotations(&self, py: Python, r#fn: &Obj) -> PyResult<()> {
        hook_list(&self.win(py)?, "_ext_sheet_annots")?.call_method1("append", (r#fn,))?;
        Ok(())
    }
    fn world_to_pixels<'p>(&self, py: Python<'p>, points: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.viewport(py)?
            .call_method1("world_to_pixels", (points,))
    }
    fn add_snap_provider(&self, py: Python, r#fn: &Obj) -> PyResult<()> {
        self.viewport(py)?
            .getattr("_ext_snap_providers")?
            .call_method1("append", (r#fn,))?;
        Ok(())
    }
    #[pyo3(signature=(pick,on_select=None,delete=None))]
    fn add_pickable(
        &self,
        py: Python,
        pick: &Obj,
        on_select: Option<&Obj>,
        delete: Option<&Obj>,
    ) -> PyResult<()> {
        let item = PyDict::new(py);
        item.set_item("key", &self.key)?;
        item.set_item("pick", pick)?;
        item.set_item("on_select", on_select)?;
        item.set_item("delete", delete)?;
        hook_list(&self.viewport(py)?, "_ext_pickables")?.call_method1("append", (item,))?;
        Ok(())
    }
    fn pick_point(&self, py: Python, prompt: &str, done: &Obj) -> PyResult<()> {
        let vp = self.viewport(py)?;
        let tool = pick_class(&py.import("views.extension_api")?)?.call0()?;
        let scene = vp.getattr("scene")?;
        tool.setattr("name", prompt)?;
        tool.setattr("window", self.win(py)?)?;
        tool.setattr("viewport", &vp)?;
        tool.setattr("revision", scene.getattr("version")?)?;
        tool.setattr("scene", scene)?;
        tool.setattr("done", done)?;
        vp.call_method1("set_active_tool", (tool,))?;
        vp.call_method0("setFocus")?;
        Ok(())
    }
    fn release_pick(&self, py: Python) -> PyResult<()> {
        let vp = self.viewport(py)?;
        if let Some(pick) = vp.getattr_opt("extension_pick")?.filter(|p| !p.is_none()) {
            if pick.get_item(0)?.get_item("key")?.eq(&self.key)? {
                let kw = PyDict::new(py);
                kw.set_item("notify", false)?;
                vp.call_method("clear_extension_pick", (), Some(&kw))?;
                vp.call_method0("update")?;
            }
        }
        Ok(())
    }
    fn add_file_opener(&self, py: Python, suffix: &str, r#fn: &Obj) -> PyResult<()> {
        let suffix = suffix.to_lowercase();
        let suffix = if suffix.starts_with('.') {
            suffix
        } else {
            format!(".{suffix}")
        };
        let module = py.import("views.extension_api")?;
        let openers = self.win(py)?.getattr("file_openers")?;
        let warning = if module.getattr("CORE_FILE_SUFFIXES")?.contains(&suffix)? {
            "extension %r may not claim %r: a core format"
        } else if openers.contains(&suffix)? {
            "extension %r's claim on %r ignored: already taken"
        } else {
            return openers.set_item(suffix, r#fn);
        };
        module
            .getattr("log")?
            .call_method1("warning", (warning, &self.key, suffix))?;
        Ok(())
    }
    fn enter_workspace(&self, py: Python, workspace: &Obj) -> PyResult<bool> {
        self.win(py)?
            .call_method1("enter_workspace", (workspace,))?
            .is_truthy()
    }
    fn leave_workspace(&self, py: Python) -> PyResult<bool> {
        self.win(py)?.call_method0("leave_workspace")?.is_truthy()
    }
    fn workspace<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        self.win(py)?.call_method0("workspace")
    }
}

#[pyfunction]
fn reset_hover(tool: &Obj, _viewport: &Obj) -> PyResult<()> {
    tool.setattr("hover_point", tool.py().None())
}
#[pyfunction]
fn hover(tool: &Obj, ctx: &Obj) -> PyResult<()> {
    tool.setattr("hover_point", ctx.getattr("world")?)?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(())
}
fn finish(tool: &Obj) -> PyResult<()> {
    let win = tool.getattr("window")?;
    let vp = tool.getattr("viewport")?;
    if win.getattr("viewport")?.is(&vp) {
        win.call_method1("_activate_tool", ("select",))?;
        vp.call_method0("setFocus")?;
    }
    Ok(())
}
#[pyfunction]
fn click(tool: &Obj, ctx: &Obj) -> PyResult<()> {
    let invoke = || -> PyResult<()> {
        let current = tool
            .getattr("window")?
            .getattr("viewport")?
            .getattr("scene")?;
        if current.is(&tool.getattr("scene")?)
            && current.getattr("version")?.eq(tool.getattr("revision")?)?
        {
            let snap = ctx.getattr("snap")?;
            let face = !snap.is_none() && snap.getattr("kind")?.eq("on_face")?;
            tool.getattr("done")?.call1((ctx.getattr("world")?, face))?;
        }
        Ok(())
    };
    let result = invoke();
    finish(tool)?;
    result
}
#[pyfunction]
fn key(tool: &Obj, _viewport: &Obj, key: u32, _modifiers: &Obj) -> PyResult<bool> {
    if key == 0x01000000 {
        finish(tool)?;
        return Ok(true);
    }
    Ok(false)
}
#[pyfunction]
fn rubber_band_lines<'p>(tool: &Obj<'p>) -> Bound<'p, PyList> {
    PyList::empty(tool.py())
}

pub(crate) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("API_VERSION", ExtensionApp::API_VERSION)?;
    m.add_class::<ExtensionApp>()?;
    let render = py.import("ingetrazo_render")?;
    for name in ["chrome_dialogs", "chrome_forms"] {
        m.add(name, render.getattr(name)?)?;
    }
    m.add("Job", m.getattr("chrome_dialogs")?.getattr("Job")?)?;
    m.add(
        "log",
        py.import("logging")?
            .call_method1("getLogger", ("ingetrazo.plugins",))?,
    )?;
    let mut suffixes: Vec<String> = py
        .import("formats.built")?
        .getattr("DOCUMENT_SUFFIXES")?
        .extract()?;
    suffixes.extend(
        [
            ".builtproject",
            ".igzproj",
            ".gltf",
            ".dae",
            ".skp",
            ".rfa",
            ".dxf",
            ".dwg",
            ".obj",
            ".stl",
            ".glb",
        ]
        .map(str::to_owned),
    );
    m.add("CORE_FILE_SUFFIXES", PyFrozenSet::new(py, suffixes)?)?;
    Ok(())
}

fn pick_class<'p>(m: &Bound<'p, PyModule>) -> PyResult<Obj<'p>> {
    if let Some(cls) = m.getattr_opt("_PointTool")? {
        return Ok(cls);
    }
    let py = m.py();
    let attrs = PyDict::new(py);
    attrs.set_item("__module__", "views.extension_api")?;
    attrs.set_item("shortcut", py.None())?;
    attrs.set_item("hover_point", py.None())?;
    attrs.set_item("uses_snap", true)?;
    let method = py.import("functools")?.getattr("partialmethod")?;
    for (name, f) in [
        ("on_activate", wrap_pyfunction!(reset_hover, m)?),
        ("on_deactivate", wrap_pyfunction!(reset_hover, m)?),
        ("on_hover", wrap_pyfunction!(hover, m)?),
        ("on_click", wrap_pyfunction!(click, m)?),
        ("on_key", wrap_pyfunction!(key, m)?),
        ("rubber_band_lines", wrap_pyfunction!(rubber_band_lines, m)?),
    ] {
        attrs.set_item(name, method.call1((f,))?)?;
    }
    let base = py.import("tools.base")?.getattr("Tool")?;
    let cls = base
        .get_type()
        .call1(("_PointTool", PyTuple::new(py, [&base])?, attrs))?;
    m.add("_PointTool", &cls)?;
    Ok(cls)
}

/// Return a resolver holding the window, so a plugin follows tab switches
/// without keeping a closed document alive. Standalone tools keep their view.
#[pyfunction]
fn live_viewport<'p>(viewport: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = viewport.py();
    let partial = py.import("functools")?.getattr("partial")?;
    let window = match viewport.call_method0("window") {
        Ok(w) => Some(w),
        Err(e)
            if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(py)
                || e.is_instance_of::<PyRuntimeError>(py) =>
        {
            None
        }
        Err(e) => return Err(e),
    };
    if let Some(win) = window {
        if win.hasattr("documents")? {
            return partial.call1((py.import("builtins")?.getattr("getattr")?, win, "viewport"));
        }
    }
    let keep = py.import("views.documents")?.getattr("_same_view")?;
    partial.call1((keep, viewport))
}
#[pyfunction]
fn _same_view<'p>(viewport: Obj<'p>) -> Obj<'p> {
    viewport
}
pub(crate) fn register_documents(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(live_viewport, m)?)?;
    m.add_function(wrap_pyfunction!(_same_view, m)?)
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Build the importable document host types only when their module is requested.
use super::*;
use pyo3::types::PyType;

fn import_names(module: &Bound<PyModule>, source: &str, names: &[(&str, &str)]) -> PyResult<()> {
    let source = module.py().import(source)?;
    for &(name, alias) in names {
        module.add(alias, source.getattr(name)?)?;
    }
    Ok(())
}
pub(super) fn class<'py>(
    module: &Bound<'py, PyModule>,
    name: &str,
    bases: &[Obj<'py>],
    attrs: &Bound<'py, PyDict>,
) -> Reply<'py> {
    let py = module.py();
    attrs.set_item("__module__", module.name()?)?;
    if let Some(doc) = match name {
        "QtViewHost"=> Some("Window services used by tools attached to a Qt wgpu surface."),
        "NdofInput"=> Some("One 3D mouse, whichever the platform. ``motion`` carries an\n:class:`NdofSample` and the seconds since the previous one; ``button``\nthe button number and whether it went down."),
        "WidgetShim"=> Some("The QWidget surface the borrowed methods use, over a host that\nknows the window's size and scale and when a frame is wanted."),
        _=>None,
    } { attrs.set_item("__doc__", doc)?; }
    let cls = py
        .get_type::<PyType>()
        .call1((name, PyTuple::new(py, bases)?, attrs))?;
    module.add(name, &cls)?;
    Ok(cls)
}
pub(super) fn classic(py: Python) -> PyResult<Bound<PyModule>> {
    py.import("views._interaction_classic")
}
pub(super) fn py_super<'py>(
    module: &Bound<'py, PyModule>,
    name: &str,
    instance: &Obj<'py>,
) -> Reply<'py> {
    module
        .py()
        .import("builtins")?
        .getattr("super")?
        .call1((module.getattr(name)?, instance))
}
pub(super) fn method(cls: &Obj, name: &str, func: &Obj) -> PyResult<()> {
    let wrapped = cls
        .py()
        .import("views")?
        .call_method1("_host_method", (func,))?;
    wrapped.setattr("__name__", name)?;
    wrapped.setattr("__module__", cls.getattr("__module__")?)?;
    wrapped.setattr(
        "__qualname__",
        format!("{}.{}", cls.getattr("__name__")?.extract::<String>()?, name),
    )?;
    if name == "_read" {
        let py = cls.py();
        let signature = py.import("inspect")?.call_method1("signature", (func,))?;
        wrapped.setattr("__annotations__", fields!(py;"return"=>"None"))?;
        wrapped.setattr(
            "__signature__",
            signature.call_method(
                "replace",
                (),
                Some(&fields!(py;"return_annotation"=>"None")),
            )?,
        )?;
    }
    cls.setattr(name, wrapped)
}
#[pyfunction]
pub(super) fn install_interaction_module(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    let full = module.name()?.extract::<String>()?;
    match full.trim_start_matches("views.") {
        "document_tools" => {
            import_names(module, "__future__", &[("annotations", "annotations")])?;
            import_names(module, "typing", &[("Optional", "Optional")])?;
            import_names(module, "core.camera", &[("OrbitCamera", "OrbitCamera")])?;
            import_names(module, "core.history", &[("History", "History")])?;
            import_names(module, "core.i18n", &[("tr", "tr")])?;
            import_names(
                module,
                "core.inference",
                &[("InferenceEngine", "InferenceEngine")],
            )?;
            import_names(module, "core.scene", &[("Scene", "Scene")])?;
            import_names(module, "tools.base", &[("Tool", "Tool")])?;
            module.add(
                "__doc__",
                "Document ownership and orchestration of the Python modelling tools.",
            )?;
            document_tools(module)?;
        }
        "view_navigation" => {
            import_names(module, "__future__", &[("annotations", "annotations")])?;
            module.add("ingetrazo_render", py.import("ingetrazo_render")?)?;
            import_names(module, "typing", &[("Optional", "Optional")])?;
            import_names(
                module,
                "core.input_types",
                &[("QPointF", "QPointF"), ("Qt", "Qt")],
            )?;
            import_names(module, "ingetrazo_core", &[("Vec3", "QVector3D")])?;
            module.add(
                "__doc__",
                "Navigation shared by document tool input and Metal presentation.",
            )?;
            view_navigation(module)?;
        }
        "view_commands" => {
            import_names(module, "functools", &[("lru_cache", "lru_cache")])?;
            import_names(
                module,
                "ingetrazo_render",
                &[
                    ("activate_view_command", "activate_view_command"),
                    ("command_chrome", "command_chrome"),
                    ("run_view_command", "run_view_command"),
                    ("view_commands", "view_commands"),
                ],
            )?;
            module.add(
                "__doc__",
                "Bindings between the Rust view command registry and the document controller.",
            )?;
            view_commands(module)?;
        }
        "view_host" => {
            import_names(module, "__future__", &[("annotations", "annotations")])?;
            import_names(module, "collections", &[("Counter", "Counter")])?;
            import_names(
                module,
                "core.input_types",
                &[
                    ("NATIVE", "NATIVE"),
                    ("QPoint", "QPoint"),
                    ("QPointF", "QPointF"),
                    ("QRect", "QRect"),
                    ("QSize", "QSize"),
                    ("Qt", "Qt"),
                ],
            )?;
            module.add(
                "__doc__",
                "Window geometry, frame requests, focus and cursor services for QObject hosts.",
            )?;
            view_host(module)?;
        }
        "document_controller" => {
            import_names(module, "views.view_host", &[("WidgetShim", "WidgetShim")])?;
            import_names(
                module,
                "views.document_interaction",
                &[("DocumentInteraction", "DocumentInteraction")],
            )?;
            module.add(
                "__doc__",
                "Document editing over a wgpu window supplied by either application.",
            )?;
            document_controller(module)?;
        }
        "ndof_input" => {
            import_names(module, "__future__", &[("annotations", "annotations")])?;
            module.add("os", py.import("os")?)?;
            module.add("math", py.import("math")?)?;
            module.add("struct", py.import("struct")?)?;
            module.add("sys", py.import("sys")?)?;
            module.add("time", py.import("time")?)?;
            import_names(
                module,
                "core.host_callbacks",
                &[("HostService", "HostService"), ("Signal", "Signal")],
            )?;
            import_names(module, "core.input_types", &[("NATIVE", "NATIVE")])?;
            import_names(
                module,
                "core.ndof",
                &[
                    ("NdofSample", "NdofSample"),
                    ("from_hid", "from_hid"),
                    ("from_spacenavd", "from_spacenavd"),
                ],
            )?;
            module.add("__doc__","3D mouse input — the drivers that feed :mod:`core.ndof` (issue #108).\n\nNo SDK, no new dependency: each platform already has a plain way to read\nthe device.\n\n* **Linux — spacenavd.** The free driver every distribution packages\n  (``spacenavd``). It serves the device on a Unix socket in a fixed binary\n  protocol: 32-byte packets of eight native ints, ``type`` first — 0 motion\n  (x y z rx ry rz period), 1 button press, 2 release. Read with a\n  ``QSocketNotifier``, so it never blocks the UI.\n* **Windows — Raw Input.** A 3D mouse is a USB HID «multi-axis controller»\n  (usage page 1, usage 8); registering for it makes Windows send\n  ``WM_INPUT`` with the device's own reports, next to (not instead of) the\n  3Dconnexion driver. Report 1 is translation (three int16) — or all six\n  axes on the newer models, report 2 rotation, report 3 the buttons. This is\n  what FreeCAD does.\n* **macOS** has no such road (only 3Dconnexion's framework): not yet.\n\n:class:`NdofInput` picks the backend for the platform and emits\n``motion(sample, dt)`` in the user's frame (see :mod:`core.ndof`).")?;
            ndof_input(module)?;
        }
        _ => return Err(pyo3::exceptions::PyImportError::new_err(full)),
    }
    Ok(())
}
fn document_tools(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    let attrs = fields!(py;"suspended_tool"=>py.None(),"__annotations__"=>fields!(py;"suspended_tool"=>"Optional[Tool]"));
    let cls = class(module, "DocumentTools", &[], &attrs)?;
    register_view_class(&cls)?;
    register_view_functions(&module.dict())
}
fn can_warp<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    if py
        .import("core.input_types")?
        .getattr("NATIVE")?
        .is_truthy()?
    {
        false.into_bound_py_any(py)
    } else {
        classic(py)?.call_method0("can_warp_pointer")
    }
}
fn view_navigation(module: &Bound<PyModule>) -> PyResult<()> {
    let attrs = fields!(module.py();"_ZOOM_OUT_STEP"=>0.05);
    let cls = class(module, "ViewNavigation", &[], &attrs)?;
    register_view_class(&cls)?;
    install(
        &cls,
        &module.dict(),
        &[(
            "_can_warp_pointer",
            Mode::Static,
            can_warp,
            &[],
            "bool",
            "Whether the host permits pointer warping.",
        )],
    )
}
fn entries<'py>(a: &Args<'py>) -> Reply<'py> {
    a.py()
        .import("views.view_commands")?
        .call_method0("view_commands")
}
fn labels<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_commands")?;
    let names = PySet::new(py, ["Navigation", "Walkthrough", "Display"])?;
    for row in module.call_method0("entries")?.try_iter()? {
        let row = row?;
        names.add(row.get_item(0)?)?;
        names.add(row.get_item(2)?)?;
    }
    let tr = py.import("core.i18n")?.getattr("tr")?;
    let result = PyDict::new(py);
    for name in names.iter() {
        result.set_item(&name, tr.call1((&name,))?)?;
    }
    Ok(result.into_any())
}
#[pyfunction(signature=(window,key,on=false))]
fn trigger_view_action(window: &Obj, key: &str, on: bool) -> PyResult<()> {
    window
        .py()
        .import("views.view_commands")?
        .call_method1("run_view_command", (window, key, on))?;
    Ok(())
}
#[pyfunction]
fn reflect_view_action(
    window: &Obj,
    action: &Obj,
    key: &str,
    bindings: &Bound<PyDict>,
    native_text: &Obj,
) -> PyResult<()> {
    let shortcut = action
        .call_method0("shortcut")?
        .call_method1("toString", (native_text,))?;
    let value = (shortcut, action.call_method0("isEnabled")?).into_bound_py_any(window.py())?;
    if let Some(current) = bindings.get_item(key)? {
        if current.eq(&value)? {
            return Ok(());
        }
    }
    bindings.set_item(key, value)?;
    let attrs = window.getattr("__dict__")?;
    let version = attrs
        .call_method1("get", ("_view_binding_version", 0))?
        .add(1)?;
    window.setattr("_view_binding_version", version)
}
fn bind_view_action<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let window = a.get_item(0)?;
    let action = a.get_item(1)?;
    let key = a.get_item(2)?;
    let sequence = classic(py)?.call_method0("shortcut_type")?;
    let entries = py.import("views.view_commands")?.call_method0("entries")?;
    let mut row = None;
    for entry in entries.try_iter()? {
        let entry = entry?;
        if entry.get_item(1)?.eq(&key)? {
            row = Some(entry);
            break;
        }
    }
    let row = row.ok_or_else(|| pyo3::exceptions::PyStopIteration::new_err(()))?;
    let shortcuts = PyList::empty(py);
    for shortcut in row.get_item(3)?.try_iter()? {
        shortcuts.append(sequence.call1((shortcut?,))?)?;
    }
    action.call_method1("setShortcuts", (shortcuts,))?;
    let signal = if a.get_item(3)?.is_truthy()? {
        "toggled"
    } else {
        "triggered"
    };
    let partial = py.import("functools")?.getattr("partial")?;
    let trigger = partial.call1((wrap_pyfunction!(trigger_view_action, py)?, &window, &key))?;
    action
        .getattr(signal)?
        .call_method1("connect", (trigger,))?;
    let bindings = window
        .getattr("__dict__")?
        .call_method1("setdefault", ("_view_bindings", PyDict::new(py)))?;
    let reflect = partial.call1((
        wrap_pyfunction!(reflect_view_action, py)?,
        &window,
        &action,
        &key,
        bindings,
        sequence.getattr("NativeText")?,
    ))?;
    action
        .getattr("changed")?
        .call_method1("connect", (&reflect,))?;
    reflect.call0()?;
    Ok(none(py))
}
fn view_commands(module: &Bound<PyModule>) -> PyResult<()> {
    register_view_functions(&module.dict())?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("entries", Mode::Function, entries, &[], "", ""),
            (
                "bind",
                Mode::Function,
                bind_view_action,
                &["win", "action", "key", "*", "toggle=False"],
                "",
                "Connect a classic host action before its shortcut customizations load.",
            ),
            ("_labels", Mode::Function, labels, &["language"], "", ""),
        ],
    )?;
    for (name, limit) in [("entries", 1), ("_labels", 4)] {
        let cached = module
            .getattr("lru_cache")?
            .call1((limit,))?
            .call1((module.getattr(name)?,))?;
        module.setattr(name, cached)?;
    }
    Ok(())
}
fn shim_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let module = py.import("views.view_host")?;
    py_super(&module, "WidgetShim", &view)?.call_method0("__init__")?;
    view.setattr("_host", a.get_item(2)?)?;
    view.setattr("_cursor", module.getattr("Qt")?.getattr("ArrowCursor")?)?;
    view.setattr("_focus", false)?;
    view.setattr("_enabled", true)?;
    Ok(none(py))
}
fn shim_missing<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let name = a.get_item(1)?.extract::<String>()?;
    if !name.starts_with("__") {
        let missing = py.import("views.view_host")?.getattr("MISSING")?;
        let n = missing.get_item(&name)?.extract::<usize>()?;
        missing.set_item(&name, n + 1)?;
    }
    Err(pyo3::exceptions::PyAttributeError::new_err(name))
}
fn shim_event<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    if py
        .import("views.view_host")?
        .getattr("NATIVE")?
        .is_truthy()?
    {
        false.into_bound_py_any(py)
    } else {
        classic(py)?.call_method1("widget_event", (a.get_item(0)?, a.get_item(1)?))
    }
}
#[pyfunction(signature = (this,*args), text_signature = "(self, *args)")]
fn redraw(this: &Obj, args: &Bound<PyTuple>) -> PyResult<()> {
    let _ = args;
    let host = this.getattr("_host")?;
    if !host.is_none() {
        host.call_method0("request_frame")?;
    }
    Ok(())
}
#[pyfunction(signature = (this,*args), text_signature = "(self, *args)")]
fn focus(this: &Obj, args: &Bound<PyTuple>) -> PyResult<()> {
    let _ = args;
    this.setattr("_focus", true)
}
#[pyfunction(signature = (this,*args,**kwargs), text_signature = "(self, *args, **kwargs)")]
fn noop(this: &Obj, args: &Bound<PyTuple>, kwargs: Option<&Bound<PyDict>>) {
    let _ = (this, args, kwargs);
}
fn view_host(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    let base = if module.getattr("NATIVE")?.is_truthy()? {
        py.import("builtins")?.getattr("object")?
    } else {
        let object = classic(py)?.call_method0("window_base")?;
        module.add("QObject", &object)?;
        object
    };
    module.add("MISSING", module.getattr("Counter")?.call0()?)?;
    module.add("__annotations__", fields!(py; "MISSING"=>"Counter"))?;
    let cls = class(
        module,
        "WidgetShim",
        &[base],
        &fields!(py;"_shim_visible"=>false),
    )?;
    register_view_class(&cls)?;
    install(
        &cls,
        &module.dict(),
        &[
            (
                "__init__",
                Mode::Method,
                shim_init,
                &["self", "parent=None", "host=None"],
                "",
                "",
            ),
            (
                "__getattr__",
                Mode::Method,
                shim_missing,
                &["self", "name"],
                "",
                "",
            ),
            ("event", Mode::Method, shim_event, &["self", "e"], "", ""),
        ],
    )?;
    method(&cls, "update", wrap_pyfunction!(redraw, module)?.as_any())?;
    cls.setattr("repaint", cls.getattr("update")?)?;
    method(&cls, "setFocus", wrap_pyfunction!(focus, module)?.as_any())?;
    method(&cls, "_noop", wrap_pyfunction!(noop, module)?.as_any())?;
    let stub = cls.getattr("_noop")?;
    for key in [
        "setFormat",
        "setMinimumSize",
        "setFocusPolicy",
        "setMouseTracking",
        "setLayoutDirection",
        "setUpdatesEnabled",
        "setToolTip",
        "setAcceptDrops",
        "setContextMenuPolicy",
        "setSizePolicy",
        "grabKeyboard",
        "releaseKeyboard",
        "grabMouse",
        "releaseMouse",
        "raise_",
        "activateWindow",
        "setStatusTip",
        "setWhatsThis",
        "show",
        "hide",
        "keyPressEvent",
        "keyReleaseEvent",
        "mousePressEvent",
        "mouseMoveEvent",
        "mouseReleaseEvent",
        "mouseDoubleClickEvent",
        "wheelEvent",
        "focusOutEvent",
        "focusInEvent",
        "leaveEvent",
        "enterEvent",
        "showEvent",
        "hideEvent",
        "resizeEvent",
        "closeEvent",
        "paintEvent",
        "contextMenuEvent",
    ] {
        cls.setattr(key, &stub)?;
    }
    Ok(())
}
fn controller_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    py_super(
        &py.import("views.document_controller")?,
        "DocumentController",
        &view,
    )?
    .call_method("__init__", (), Some(&fields!(py;"host"=>a.get_item(1)?)))?;
    view.setattr("_enabled", false)?;
    Ok(none(py))
}
fn for_widget<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let display = a.get_item(1)?;
    py.import("views.rtl")?
        .call_method1("keep_ltr", (&display,))?;
    let host = py
        .import("views.document_controller")?
        .call_method1("QtViewHost", (&display,))?;
    let view = a.get_item(0)?.call1((host,))?;
    view.call_method1("setParent", (&display,))?;
    view.setattr("keys_stand_in", &display)?;
    display.setattr("source_viewport", &view)?;
    display.setattr("camera", view.getattr("camera")?)?;
    Ok(view)
}
#[pyfunction(signature = (this,*size), text_signature = "(self, *size)")]
fn resize(this: &Obj, size: &Bound<PyTuple>) -> PyResult<()> {
    this.getattr("keys_stand_in")?
        .call_method1("resize", size)?;
    Ok(())
}
#[pyfunction(signature = (this,*args), text_signature = "(self, *args)")]
fn controller_focus(this: &Obj, args: &Bound<PyTuple>) -> PyResult<()> {
    let m = this.py().import("views.document_controller")?;
    let display = optional(this, "keys_stand_in")?;
    if this
        .getattr("_host")?
        .is_instance(&m.getattr("QtViewHost")?)?
        && !display.is_none()
    {
        display.call_method1("setFocus", args)?;
    } else {
        py_super(&m, "DocumentController", this)?.call_method1("setFocus", args)?;
    }
    Ok(())
}
fn map_to_global<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?
        .getattr("keys_stand_in")?
        .call_method1("mapToGlobal", (a.get_item(1)?,))
}
fn map_from_global<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?
        .getattr("keys_stand_in")?
        .call_method1("mapFromGlobal", (a.get_item(1)?,))
}
fn qt_host_init<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.setattr("display", a.get_item(1)?)?;
    Ok(none(a.py()))
}
fn document_controller(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    let cls = class(
        module,
        "DocumentController",
        &[
            module.getattr("DocumentInteraction")?,
            module.getattr("WidgetShim")?,
        ],
        &PyDict::new(py),
    )?;
    register_view_class(&cls)?;
    install(
        &cls,
        &module.dict(),
        &[
            (
                "__init__",
                Mode::Method,
                controller_init,
                &["self", "host=None"],
                "",
                "",
            ),
            (
                "for_widget",
                Mode::Class,
                for_widget,
                &["cls", "display"],
                "",
                "",
            ),
            (
                "mapToGlobal",
                Mode::Method,
                map_to_global,
                &["self", "point"],
                "",
                "",
            ),
            (
                "mapFromGlobal",
                Mode::Method,
                map_from_global,
                &["self", "point"],
                "",
                "",
            ),
        ],
    )?;
    method(&cls, "resize", wrap_pyfunction!(resize, module)?.as_any())?;
    method(
        &cls,
        "setFocus",
        wrap_pyfunction!(controller_focus, module)?.as_any(),
    )?;
    let host = class(module, "QtViewHost", &[], &PyDict::new(py))?;
    register_view_class(&host)?;
    install(
        &host,
        &module.dict(),
        &[(
            "__init__",
            Mode::Method,
            qt_host_init,
            &["self", "display"],
            "",
            "",
        )],
    )
}
fn ndof_input(module: &Bound<PyModule>) -> PyResult<()> {
    super::device::install_devices(module)
}

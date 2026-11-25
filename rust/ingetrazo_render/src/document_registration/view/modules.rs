// SPDX-License-Identifier: GPL-3.0-or-later
//! Build the importable document host types only when their module is requested.
use super::*;
use pyo3::types::PyType;

pub(super) fn import_names(module: &Bound<PyModule>, source: &str, names: &[(&str, &str)]) -> PyResult<()> {
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
        "extension_api" => crate::extension_api::register(module)?,
        "documents" => crate::extension_api::register_documents(module)?,
        "document_interaction" => super::interaction_host::install(module)?,
        "wgpu_view" => super::metal_view::install_module(module)?,
        "wgpu_ui" => super::chrome_input::install_module(module)?,
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
                "ingetrazo_core",
                &[("ScreenPoint", "QPointF"), ("Input", "Qt")],
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
            import_names(
                module,
                "ingetrazo_core",
                &[
                    ("PixelPoint", "QPoint"),
                    ("ScreenPoint", "QPointF"),
                    ("ScreenRect", "QRect"),
                    ("ScreenSize", "QSize"),
                    ("Input", "Qt"),
                ],
            )?;
            module.add(
                "__doc__",
                "Window geometry, frame requests, focus and cursor services for document hosts.",
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
    false.into_bound_py_any(a.py())
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
fn view_commands(module: &Bound<PyModule>) -> PyResult<()> {
    register_view_functions(&module.dict())?;
    install(
        module.dict().as_any(),
        &module.dict(),
        &[
            ("entries", Mode::Function, entries, &[], "", ""),
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
fn shim_event<'py>(a: &Args<'py>) -> Reply<'py> {
    false.into_bound_py_any(a.py())
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
    let base = py.import("builtins")?.getattr("object")?;
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

#[pyfunction(signature = (this,*size), text_signature = "(self, *size)")]
fn resize(this: &Obj, size: &Bound<PyTuple>) -> PyResult<()> {
    this.getattr("keys_stand_in")?
        .call_method1("resize", size)?;
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
    Ok(())
}
fn ndof_input(module: &Bound<PyModule>) -> PyResult<()> {
    super::device::install_devices(module)
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! The resident Metal view (`views.wgpu_view.MetalView`): native input and
//! presentation sharing the document camera and selection (render-glue
//! lane; from `views/wgpu_view.py`).
//!
//! The view is a document binding over the shell's widget shim: its state
//! is the instance's attributes, and its base classes (`ViewNavigation`,
//! then the host's `WidgetShim`) are reached through `super()`.
use super::*;

pub(super) fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    optional(o, name)
}
pub(super) fn truthy(o: &Obj<'_>, name: &str) -> PyResult<bool> {
    attr_flag(o, name, false)
}
fn any<'py>(py: Python<'py>, values: &Obj<'py>) -> PyResult<bool> {
    py.import("builtins")?.call_method1("any", (values,))?.is_truthy()
}
fn own<'py>(view: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    Ok(view.getattr("__dict__")?.cast_into::<PyDict>()?)
}
fn view_module<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.wgpu_view")
}
/// `super(MetalView, view)`.
pub(super) fn base<'py>(view: &Obj<'py>) -> Reply<'py> {
    modules::py_super(&view_module(view.py())?, "MetalView", view)
}

/// What the baked paint depends on besides the content: Colour by tag (with
/// a style that shows it), and each tag's colour, opacity and lock — what
/// `_paint_of` reads for a container.
fn paint_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let scene = a.get_item(0)?;
    let style = attr(&scene, "display_style")?;
    let mode = attr(&style, "face_mode")?;
    let by_layer = truthy(&style, "color_by_layer")? && ["textures", "shaded", "xray"].iter().any(|m| mode.eq(*m).unwrap_or(false));
    let layer_color = py.import("ingetrazo_core")?.getattr("tag_styles")?.getattr("layer_color")?;
    let layers = PyList::empty(py);
    if let Some(all) = scene.getattr_opt("layers")? {
        for layer in all.try_iter()? {
            let layer = layer?;
            let opacity: f64 = match layer.getattr_opt("opacity")? {
                Some(opacity) => opacity.call_method0("__float__")?.extract()?,
                None => 1.0,
            };
            let locked = truthy(&layer, "locked")?;
            if by_layer || opacity != 1.0 || locked {
                let colour = if by_layer { py.get_type::<PyTuple>().call1((layer_color.call1((&layer,))?,))? } else { none(py) };
                layers.append((layer.getattr("key")?, colour, opacity, locked))?;
            }
        }
    }
    (by_layer, truthy(&scene, "show_hidden_geometry")?, truthy(&scene, "show_hidden_objects")?, layers.to_tuple()).into_bound_py_any(py)
}

/// The egui chrome hears input first, in screen space, before the geometry
/// is moved into the camera frame for the tools.
pub(super) fn chrome_claims<'py>(view: &Obj<'py>, name: &str, event: &Obj<'py>) -> Reply<'py> {
    let py = view.py();
    let ui = py.import("views.wgpu_ui")?;
    match name {
        "mousePressEvent" => {
            view.call_method0("setFocus")?;
            ui.call_method1("pointer", (view, "press", event))
        }
        "mouseMoveEvent" => ui.call_method1("pointer", (view, "move", event)),
        "mouseReleaseEvent" => ui.call_method1("pointer", (view, "release", event)),
        "mouseDoubleClickEvent" => ui.call_method1("pointer", (view, "press", event)),
        "keyPressEvent" => ui.call_method1("key", (view, event, true)),
        "keyReleaseEvent" => ui.call_method1("key", (view, event, false)),
        "wheelEvent" => ui.call_method1("wheel", (view, event)),
        _ => false.into_bound_py_any(py),
    }
}
fn ui_claims<'py>(a: &Args<'py>) -> Reply<'py> {
    let name: String = a.get_item(1)?.extract()?;
    chrome_claims(&a.get_item(0)?, &name, &a.get_item(2)?)
}

/// Run an event handler's `body` as the tools see the model: not on a
/// closed view, after the chrome has had the event, with the remote
/// camera in step and the geometry in the view frame.
pub(super) fn tool_frame<'py>(view: &Obj<'py>, name: &str, event: Option<&Obj<'py>>, body: impl FnOnce() -> Reply<'py>) -> Reply<'py> {
    let py = view.py();
    if truthy(view, "_closed")? {
        return Ok(none(py));
    }
    if let Some(event) = event {
        // By the module's name: a host without chrome replaces it.
        if view_module(py)?.call_method1("_ui_claims", (view, name, event))?.is_truthy()? {
            return Ok(none(py));
        }
    }
    let packed = attr(view, "packed")?;
    let remote = !attr(&packed, "model_translations")?.is_none();
    if remote {
        view.call_method0("_sync_scene_camera")?;
    }
    let shifted = match packed.getattr_opt("origin")? {
        Some(origin) => any(py, &origin)?,
        None => false,
    };
    let result = if shifted || truthy(view, "_remote_meshes")? {
        let frame = view.call_method0("_geometry_frame")?;
        frame.call_method0("__enter__")?;
        match body() {
            Ok(result) => {
                frame.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
                result
            }
            Err(error) => {
                frame.call_method1("__exit__", (error.get_type(py), error.value(py), error.traceback(py)))?;
                return Err(error);
            }
        }
    } else {
        body()?
    };
    if remote && !attr(&view.getattr("packed")?, "model_translations")?.is_none() {
        view.call_method0("_sync_scene_camera")?;
    }
    Ok(result)
}

/// Whether a Ctrl comes and goes ALONE (`ctrl_tapped`, #183): the
/// controller's own key handler recorded it, which this view bypasses, so
/// every Ctrl tap read as a chord and the tools took it back.
pub(super) fn ctrl_tap(view: &Obj<'_>, event: &Obj<'_>) -> PyResult<()> {
    let py = view.py();
    let source = attr(view, "source_viewport")?;
    let repeat = attr(event, "isAutoRepeat")?;
    if source.is_none() || (repeat.is_callable() && repeat.call0()?.is_truthy()?) {
        return Ok(());
    }
    let codes = py.import("ingetrazo_core")?;
    let key = event.call_method0("key")?;
    if key.eq(codes.getattr("Input")?.getattr("Key_Control")?)? {
        let kind = attr(event, "type")?;
        if !kind.is_callable() || kind.call0()?.eq(codes.getattr("Type")?.getattr("KeyPress")?)? {
            source.setattr("_ctrl_alone", true)?;
        }
    } else if !py.import("views.document_interaction")?.getattr("DocumentInteraction")?.getattr("_MODIFIER_KEYS")?.contains(key)? {
        source.setattr("_ctrl_alone", false)?;
    }
    Ok(())
}
/// Shutting the view's worker down first takes the document back from a
/// snap-index job (`SceneLease`).
#[pyfunction(signature = (this, wait = true, *, cancel_futures = false))]
fn worker_shutdown(this: &Obj, wait: bool, cancel_futures: bool) -> PyResult<()> {
    let py = this.py();
    let lease = attr(this, "lease")?;
    this.setattr("lease", py.None())?;
    if !lease.is_none() {
        lease.call_method0("release")?;
    }
    let kw = PyDict::new(py);
    kw.set_item("cancel_futures", cancel_futures)?;
    let pool = py.import("concurrent.futures")?.getattr("ThreadPoolExecutor")?;
    pool.getattr("shutdown")?.call((this, wait), Some(&kw))?;
    Ok(())
}

/// The view's one background worker (the snap index, the edit room).
pub(super) fn index_worker<'py>(py: Python<'py>) -> Reply<'py> {
    let module = view_module(py)?;
    let class = match module.getattr_opt("IndexWorker")? {
        Some(class) => class,
        None => {
            let pool = py.import("concurrent.futures")?.getattr("ThreadPoolExecutor")?;
            let class = modules::class(&module, "IndexWorker", &[pool], &fields!(py; "lease" => py.None()))?;
            modules::method(&class, "shutdown", wrap_pyfunction!(worker_shutdown, &module)?.as_any())?;
            class
        }
    };
    let kw = PyDict::new(py);
    kw.set_item("max_workers", 1)?;
    class.call((), Some(&kw))
}
/// The tools' coordinate frame while a handler runs: the open mesh and the
/// remote prototypes moved into the view frame, put back on leaving.
#[pyclass(module = "ingetrazo_render", unsendable)]
struct GeometryFrame {
    view: Py<PyAny>,
    frames: Option<Py<PyAny>>,
}

#[pymethods]
impl GeometryFrame {
    fn __enter__(&mut self, py: Python<'_>) -> PyResult<()> {
        let view = self.view.bind(py);
        let mesh_frame = py.import("ingetrazo_core.coordinate_frame")?.getattr("mesh_frame")?;
        let frames = py.import("contextlib")?.getattr("ExitStack")?.call0()?;
        frames.call_method0("__enter__")?;
        let entered = (|| -> PyResult<()> {
            let active = view.getattr("scene")?.getattr("mesh")?;
            frames.call_method1("enter_context", (mesh_frame.call1((&active, view.getattr("packed")?.getattr("origin")?))?,))?;
            if let Some(remote) = view.getattr_opt("_remote_meshes")? {
                for entry in remote.try_iter()? {
                    let entry = entry?;
                    let mesh = entry.get_item(0)?;
                    if !mesh.is(&active) {
                        frames.call_method1("enter_context", (mesh_frame.call1((mesh, entry.get_item(1)?))?,))?;
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = entered {
            frames.call_method1("__exit__", (error.get_type(py), error.value(py), error.traceback(py)))?;
            return Err(error);
        }
        self.frames = Some(frames.unbind());
        Ok(())
    }
    fn __exit__<'py>(&mut self, py: Python<'py>, kind: &Obj<'py>, value: &Obj<'py>, traceback: &Obj<'py>) -> Reply<'py> {
        match self.frames.take() {
            Some(frames) => frames.bind(py).call_method1("__exit__", (kind, value, traceback)),
            None => false.into_bound_py_any(py),
        }
    }
}

fn geometry_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(Bound::new(py, GeometryFrame { view: a.get_item(0)?.unbind(), frames: None })?.into_any())
}

fn horizon<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    let source = view.getattr("source_viewport")?;
    if any(a.py(), &view.getattr("packed")?.getattr("origin")?)? {
        return source.call_method1("_horizon_ndc_y", (a.get_item(1)?, a.get_item(2)?));
    }
    source.call_method1("_horizon_ndc_y", (a.get_item(1)?,))
}

fn model_diag<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let packed = view.getattr("packed")?;
    if packed.is_none() {
        let source = view.getattr("source_viewport")?;
        return if source.is_none() { 0.0.into_bound_py_any(py) } else { source.call_method0("_model_diag") };
    }
    let bounds = packed.getattr("world_bounds")?;
    let (low, high): ([f64; 3], [f64; 3]) = (bounds.get_item(0)?.extract()?, bounds.get_item(1)?.extract()?);
    (0..3).map(|k| (high[k] - low[k]).powi(2)).sum::<f64>().sqrt().into_bound_py_any(py)
}

/// Winit already owns the surface. Widget hosts configure theirs here.
fn prepare_surface<'py>(a: &Args<'py>) -> Reply<'py> {
    Ok(none(a.py()))
}

fn view_timer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    if view.getattr("_timer_queue")?.is_none() {
        // A view without a host (tests, probes) keeps its own queue;
        // whoever drives it runs `_timer_queue.run_due()`.
        view.setattr("_timer_queue", py.import("ingetrazo_render")?.getattr("WindowTimers")?.call0()?)?;
    }
    view.getattr("_timer_queue")?.call_method1("timer", (a.get_item(1)?, a.get_item(2)?))
}

fn init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    base(&view)?.call_method1("__init__", (a.get_item(1)?,))?;
    view.setattr("_timer_queue", a.get_item(2)?)?;
    view.setattr("_resize_timer", view.call_method1("_view_timer", (view.getattr("_finish_resize")?, 80))?)?;
    view.call_method0("_prepare_surface")?;
    view.setattr("camera", py.import("core.camera")?.getattr("OrbitCamera")?.call0()?)?;
    view.setattr("source_viewport", py.None())?;
    view.call_method1("setFocusPolicy", (py.import("ingetrazo_core")?.getattr("Input")?.getattr("StrongFocus")?,))?;
    view.call_method1("setMouseTracking", (true,))?;
    view.setattr("hovered_row", py.None())?;
    view.setattr("inference_index", py.None())?;
    view.setattr("hover_ms", 0.)?;
    view.setattr("selection_ms", 0.)?;
    view.setattr("_selection_signature", py.None())?;
    view.setattr("_settle_timer", view.call_method1("_view_timer", (view.getattr("_settled_frame")?,))?)?;
    view.setattr("_last_camera", py.None())?;
    view.setattr("renderer", py.None())?;
    view.setattr("packed", py.None())?;
    view.setattr("_scene_dirty", false)?;
    view.setattr("overlay_primitives", PyList::empty(py))?;
    view.setattr("overlay_ms", 0.)?;
    view.setattr("last", py.None())?;
    view.setattr("upload_seconds", 0.)?;
    view.setattr("build_seconds", 0.)?;
    view.setattr("tools", py.import("ingetrazo_render")?.getattr("NativeTools")?.call1((&view,))?)?;
    Ok(none(py))
}

fn paint_engine<'py>(a: &Args<'py>) -> Reply<'py> {
    Ok(none(a.py()))
}

/// The view holding the resident scene: this one; a peer's first.
fn shared<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)
}

fn peers<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    match own(&a.get_item(0)?)?.get_item("_peers")? {
        Some(peers) => py.get_type::<PyList>().call1((peers,)),
        None => Ok(PyList::empty(py).into_any()),
    }
}

/// `peer` draws this view's resident scene on a surface of its own: this
/// view's calls now name its surface too.
fn add_peer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, peer) = (a.get_item(0)?, a.get_item(1)?);
    let peers = own(&view)?.call_method1("setdefault", ("_peers", PyList::empty(py)))?;
    if !peers.contains(&peer)? {
        peers.call_method1("append", (peer,))?;
    }
    view.call_method0("_wrap_renderer")
}

fn remove_peer<'py>(a: &Args<'py>) -> Reply<'py> {
    let (view, peer) = (a.get_item(0)?, a.get_item(1)?);
    if let Some(peers) = own(&view)?.get_item("_peers")? {
        if peers.contains(&peer)? {
            peers.call_method1("remove", (peer,))?;
        }
    }
    Ok(none(a.py()))
}

fn can_create_surface<'py>(a: &Args<'py>) -> Reply<'py> {
    true.into_bound_py_any(a.py())
}

/// `(window, handles)` for `Renderer`: the host's surface. The shell's view
/// hands over its NSView; a headless view a layer.
fn window_handles<'py>(_a: &Args<'py>) -> Reply<'py> {
    Err(pyo3::exceptions::PyNotImplementedError::new_err("a MetalView host supplies its surface"))
}

fn wrap_renderer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let target = py.import("ingetrazo_render")?.getattr("shell_views")?.getattr("TargetRenderer")?;
    let renderer = view.getattr("renderer")?;
    if view.call_method0("peers")?.is_truthy()? && !renderer.is_none() && renderer.hasattr("bind_target")? && !renderer.is_instance(&target)? {
        view.setattr("renderer", target.call1((renderer, 0))?)?;
    }
    Ok(none(py))
}

/// On screen here or in another view of the grid.
fn presentable<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    if view.call_method0("isVisible")?.is_truthy()? {
        return true.into_bound_py_any(py);
    }
    for peer in view.call_method0("peers")?.try_iter()? {
        if peer?.call_method0("isVisible")?.is_truthy()? {
            return true.into_bound_py_any(py);
        }
    }
    false.into_bound_py_any(py)
}

fn observe_tool_source<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let source = view.getattr("source_viewport")?;
    let previous = attr(&view, "_tool_source")?;
    if source.is(&previous) {
        return Ok(none(py));
    }
    let schedule = view.getattr("_schedule_tool_sync")?;
    let signal = attr(&previous, "measurementChanged")?;
    if signal.hasattr("disconnect")? {
        signal.call_method1("disconnect", (&schedule,))?;
    }
    view.setattr("_tool_source", &source)?;
    let signal = attr(&source, "measurementChanged")?;
    if signal.hasattr("connect")? {
        signal.call_method1("connect", (schedule,))?;
    }
    Ok(none(py))
}

#[pyfunction(signature = (this, *_args))]
fn schedule_tool_sync(this: &Obj, _args: &Bound<PyTuple>) -> PyResult<()> {
    let py = this.py();
    let tool = attr(&this.getattr("source_viewport")?, "active_tool")?;
    let tools = this.getattr("tools")?;
    let reset = !tools.getattr("start_point")?.is_none() && attr(&tool, "start_point")?.is_none();
    if this.call_method0("isVisible")?.is_truthy()? && (!tool.is(&tools.getattr("tool")?) || reset) {
        py.import("core.timers")?.call_method1("later", (this, 0, this.getattr("_sync_tool_dispatch")?))?;
    }
    Ok(())
}

fn sync_tool_dispatch<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    if view.call_method0("isVisible")?.is_truthy()? {
        view.getattr("tools")?.call_method0("sync")?;
        view.call_method0("update")?;
    }
    Ok(none(a.py()))
}

fn hide_event<'py>(a: &Args<'py>) -> Reply<'py> {
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    tool_frame(&view, "hideEvent", Some(&event), || {
        if !truthy(&view, "_closed")? {
            view.getattr("tools")?.call_method0("cancel")?;
        }
        base(&view)?.call_method1("hideEvent", (&event,))
    })
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_paint_key", Mode::Function, paint_key, &["scene"], "", ""),
    ("_ui_claims", Mode::Function, ui_claims, &["view", "name", "event"], "", ""),
];

#[rustfmt::skip]
const METHODS: &[Method] = &[
    ("_geometry_frame", Mode::Method, geometry_frame, &["self"], "", ""),
    ("_horizon", Mode::Method, horizon, &["self", "mvp", "camera"], "", ""),
    ("_model_diag", Mode::Method, model_diag, &["self"], "", ""),
    ("_prepare_surface", Mode::Method, prepare_surface, &["self"], "", ""),
    ("_view_timer", Mode::Method, view_timer, &["self", "callback", "interval=0"], "", ""),
    ("__init__", Mode::Method, init, &["self", "parent=None", "*", "timers=None"], "", ""),
    ("paintEngine", Mode::Method, paint_engine, &["self"], "", ""),
    ("_shared", Mode::Property, shared, &["self"], "", "The view holding the resident scene: this one; a peer's first."),
    ("peers", Mode::Method, peers, &["self"], "", ""),
    ("add_peer", Mode::Method, add_peer, &["self", "peer"], "", ""),
    ("remove_peer", Mode::Method, remove_peer, &["self", "peer"], "", ""),
    ("_can_create_surface", Mode::Method, can_create_surface, &["self"], "", ""),
    ("_window_handles", Mode::Method, window_handles, &["self"], "", ""),
    ("_wrap_renderer", Mode::Method, wrap_renderer, &["self"], "", ""),
    ("_presentable", Mode::Method, presentable, &["self"], "", ""),
    ("_observe_tool_source", Mode::Method, observe_tool_source, &["self"], "", ""),
    ("_sync_tool_dispatch", Mode::Method, sync_tool_dispatch, &["self"], "", ""),
    ("hideEvent", Mode::Method, hide_event, &["self", "event"], "", ""),
];

/// Put the view's native methods on `cls`.
pub(super) fn register(cls: &Obj, module: &Bound<PyModule>) -> PyResult<()> {
    install(cls, &module.dict(), METHODS)?;
    install(cls, &module.dict(), metal_view_scene::METHODS)?;
    install(cls, &module.dict(), metal_view_frame::METHODS)?;
    metal_view_capture::register(cls, module)?;
    install(cls, &module.dict(), metal_view_selection::METHODS)?;
    install(cls, &module.dict(), metal_view_input::METHODS)?;
    modules::method(cls, "_schedule_tool_sync", wrap_pyfunction!(schedule_tool_sync, module)?.as_any())
}

/// Build `views.wgpu_view`: the view class over `ViewNavigation` and the
/// module's functions.
pub(super) fn install_module(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    modules::import_names(module, "ingetrazo_core", &[("pack_scene", "pack_scene")])?;
    module.add("__doc__", "Native Metal input and presentation sharing the document camera and selection.")?;
    let navigation = py.import("views.view_navigation")?.getattr("ViewNavigation")?;
    let ready = py.import("ingetrazo_render")?.call_method0("CallbackSlot")?;
    let cls = modules::class(module, "MetalView", &[navigation], &fields!(py; "frameReady" => ready, "_ZOOM_OUT_STEP" => 0.05))?;
    install(module.dict().as_any(), &module.dict(), FUNCTIONS)?;
    register(&cls, module)
}

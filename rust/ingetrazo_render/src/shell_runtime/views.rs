//! Per-window display hooks and resident GPU targets for multiple views of a document.
//! The display bases are resolved on first construction, after the venv is active.
use super::*;
use pyo3::exceptions::PyAttributeError;

fn types(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    module(py, "shell_views")
}
#[pyfunction]
fn view_init(view: &Obj<'_>, host: &Obj<'_>) -> PyResult<()> {
    let py = view.py();
    let kw = PyDict::new(py);
    kw.set_item("timers", host.getattr("timers")?)?;
    py.import("views.wgpu_view")?
        .getattr("MetalView")?
        .call_method("__init__", (view,), Some(&kw))?;
    view.setattr("_host", host)
}
#[pyfunction]
fn presentation_timers<'p>(view: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>, Obj<'p>)> {
    Ok((
        view.getattr("_resize_timer")?,
        view.getattr("_settle_timer")?,
        view.getattr("__dict__")?
            .call_method1("get", ("_ui_timer",))?,
    ))
}
fn each_timer(view: &Obj<'_>, mut run: impl FnMut(&Obj<'_>) -> PyResult<()>) -> PyResult<()> {
    let (a, b, c) = presentation_timers(view)?;
    for t in [a, b, c] {
        if !t.is_none() {
            run(&t)?;
        }
    }
    Ok(())
}
#[pyfunction]
fn move_timers(view: &Obj<'_>, host: &Obj<'_>) -> PyResult<()> {
    let q = host.getattr("timers")?;
    view.setattr("_timer_queue", &q)?;
    each_timer(view, |t| q.call_method1("adopt", (t,)).map(drop))
}
#[pyfunction]
fn handles<'p>(view: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>)> {
    let h = view.getattr("_host")?;
    Ok((h.getattr("ns_view")?, h.getattr("surface_handles")?))
}
#[pyfunction]
fn window_id<'p>(view: &Obj<'p>) -> PyResult<Obj<'p>> {
    view.getattr("_host")?.getattr("ns_view")
}
#[pyfunction]
fn visible(view: &Obj<'_>) -> PyResult<bool> {
    window_id(view)?.is_truthy()
}
#[pyfunction(signature=(view,event=None))]
fn show(view: &Obj<'_>, event: Option<&Obj<'_>>) -> PyResult<()> {
    let _ = event;
    if view.getattr("renderer")?.is_none() || flag(view, "_scene_dirty")? {
        view.call_method0("_upload")?;
    }
    Ok(())
}
#[pyfunction(signature=(view,event=None))]
fn resize(view: &Obj<'_>, event: Option<&Obj<'_>>) -> PyResult<()> {
    let _ = event;
    view.call_method0("_finish_resize")?;
    Ok(())
}
#[pyfunction(signature=(view,event=None))]
fn hide(view: &Obj<'_>, event: Option<&Obj<'_>>) -> PyResult<()> {
    let _ = event;
    view.getattr("tools")?.call_method0("cancel")?;
    Ok(())
}
#[pyfunction(signature=(view,event=None))]
fn leave(view: &Obj<'_>, event: Option<&Obj<'_>>) -> PyResult<()> {
    let _ = event;
    view.py()
        .import("views.wgpu_ui")?
        .call_method1("pointer", (view, "leave"))?;
    Ok(())
}
#[pyfunction(signature=(view,event=None))]
fn close(view: &Obj<'_>, event: Option<&Obj<'_>>) -> PyResult<()> {
    let _ = event;
    let py = view.py();
    view.setattr("_closed", true)?;
    view.getattr("_host")?
        .getattr("timers")?
        .call_method1("cancel_context", (view,))?;
    each_timer(view, |t| t.call_method0("stop").map(drop))?;
    let tracker = attr(view, "_instance_updates")?;
    if !tracker.is_none() {
        let unsub = tracker.getattr("unsubscribe")?;
        if !unsub.is_none() {
            unsub.call0()?;
        }
    }
    view.setattr("renderer", py.None())?;
    if let Some(worker) = view.getattr_opt("_inference_worker")? {
        let kw = PyDict::new(py);
        kw.set_item("wait", false)?;
        kw.set_item("cancel_futures", true)?;
        worker.call_method("shutdown", (), Some(&kw))?;
    }
    core(py)?.call_method1("release_document_view", (view,))?;
    Ok(())
}
#[pyfunction]
fn document_init(doc: &Obj<'_>) -> PyResult<()> {
    let py = doc.py();
    py.import("core.document_host")?
        .getattr("Document")?
        .call_method1("__init__", (doc,))?;
    doc.setattr("model_views", core(py)?.call_method0("DocumentViews")?)
}
#[pyfunction]
fn layout<'p>(doc: &Obj<'p>) -> PyResult<Obj<'p>> {
    doc.getattr("model_views")?.getattr("layout")
}
#[pyclass(dict, module = "ingetrazo_render.shell_views")]
struct PaneHost {
    #[pyo3(get)]
    window_host: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    width: i32,
    #[pyo3(get, set)]
    height: i32,
    #[pyo3(get, set)]
    x: i32,
    #[pyo3(get, set)]
    y: i32,
}
#[pymethods]
impl PaneHost {
    #[new]
    fn new(window_host: Py<PyAny>) -> Self {
        Self {
            window_host: Some(window_host),
            width: 1,
            height: 1,
            x: 0,
            y: 0,
        }
    }
    fn __getattr__<'p>(&self, py: Python<'p>, name: &str) -> PyResult<Obj<'p>> {
        self.window_host
            .as_ref()
            .ok_or_else(|| PyAttributeError::new_err(name.to_owned()))?
            .bind(py)
            .getattr(name)
    }
    fn __traverse__(
        &self,
        visit: pyo3::class::gc::PyVisit<'_>,
    ) -> Result<(), pyo3::class::gc::PyTraverseError> {
        visit.call(&self.window_host)
    }
    fn __clear__(&mut self) {
        self.window_host = None;
    }
}
#[pyfunction]
fn offset(view: &Obj<'_>) -> PyResult<(f64, f64)> {
    let h = view.getattr("_host")?;
    Ok((h.getattr("x")?.extract()?, h.getattr("y")?.extract()?))
}
fn mapped<'p>(view: &Obj<'p>, p: &Obj<'p>, sign: f64) -> PyResult<Obj<'p>> {
    let (x, y) = offset(view)?;
    p.get_type().call1((
        p.call_method0("x")?.extract::<f64>()? + sign * x,
        p.call_method0("y")?.extract::<f64>()? + sign * y,
    ))
}
#[pyfunction]
fn to_global<'p>(view: &Obj<'p>, p: &Obj<'p>) -> PyResult<Obj<'p>> {
    mapped(view, p, 1.)
}
#[pyfunction]
fn from_global<'p>(view: &Obj<'p>, p: &Obj<'p>) -> PyResult<Obj<'p>> {
    mapped(view, p, -1.)
}
fn size<'p>(view: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>, Obj<'p>)> {
    Ok((
        view.call_method0("width")?,
        view.call_method0("height")?,
        view.call_method0("devicePixelRatioF")?,
    ))
}
#[pyfunction]
fn attach(view: &Obj<'_>) -> PyResult<bool> {
    if view.getattr("renderer")?.is_none() {
        let primary = view.getattr("primary")?;
        primary.call_method1("add_peer", (view,))?;
        let peer = view.py().import("views.resident_peer")?;
        let raw = peer.call_method1("raw_renderer", (primary.getattr("renderer")?,))?;
        let physical = size(view)?.into_pyobject(view.py())?;
        view.setattr("_physical_size", &physical)?;
        let target = raw.call_method1("add_tile_target", physical)?;
        view.setattr("_target", &target)?;
        view.setattr(
            "renderer",
            peer.call_method1("TargetRenderer", (&raw, target))?,
        )?;
    }
    Ok(true)
}
#[pyfunction]
fn draw_frame<'p>(view: &Obj<'p>) -> PyResult<Obj<'p>> {
    if view.getattr("primary")?.getattr("packed")?.is_none() {
        return Ok(none(view.py()));
    }
    attach(view)?;
    if !view.getattr("_physical_size")?.eq(size(view)?)? {
        view.call_method0("_finish_resize")?;
    }
    view.call_method0("_present_frame")
}
#[pyfunction]
fn dispose(view: &Obj<'_>) -> PyResult<()> {
    view.setattr("_closed", true)?;
    each_timer(view, |t| t.call_method0("stop").map(drop))?;
    view.call_method0("release")?;
    Ok(())
}

#[pyfunction]
fn make_pane<'p>(
    doc: &Obj<'p>,
    reuse_main: bool,
    initial_view: &Obj<'p>,
    saved: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let py = doc.py();
    let main = doc.getattr("viewport")?;
    let primary = doc.getattr("stand_in")?;
    let host = Bound::new(py, PaneHost::new(primary.getattr("_host")?.unbind()))?;
    let display = types(py)?.call_method1("Pane", (&primary, &host))?;
    display.setattr("idle_tools", display.getattr("tools")?)?;
    let vp = if reuse_main {
        main.setattr("_host", &host)?;
        main
    } else {
        let v = py
            .import("views.document_controller")?
            .call_method1("DocumentController", (&host,))?;
        for key in ["scene", "history"] {
            v.setattr(key, main.getattr(key)?)?;
        }
        v.call_method0("_bind_bake_store")?;
        for key in ["_ext_overlays", "_ext_snap_providers", "_ext_pickables"] {
            if let Some(value) = main.getattr_opt(key)? {
                v.setattr(key, value)?;
            }
        }
        if saved.is_none() {
            let cam = v.getattr("camera")?;
            let anchor = main.getattr("camera")?;
            let target = anchor.getattr("target")?;
            cam.setattr("target", target.get_type().call1((&target,))?)?;
            cam.setattr("distance", anchor.getattr("distance")?)?;
            cam.setattr("perspective", false)?;
            cam.call_method1("set_view", (initial_view,))?;
        } else {
            v.setattr("camera", saved.get_item(0)?)?;
            v.setattr("view_style", saved.get_item(1)?)?;
        }
        v
    };
    display.setattr("source_viewport", &vp)?;
    vp.setattr("keys_stand_in", &display)?;
    display.setattr("camera", vp.getattr("camera")?)?;
    Ok(display)
}
#[pyfunction]
fn choice(doc: &Obj<'_>, options: &Bound<'_, PyDict>, label: &Obj<'_>) -> PyResult<()> {
    if let Some(key) = options.get_item(label)? {
        doc.getattr("model_views")?
            .call_method1("choose", (doc, key))?;
    }
    Ok(())
}
#[pyfunction]
fn choose(win: &Obj<'_>) -> PyResult<()> {
    let py = win.py();
    let doc = win.getattr("_doc")?;
    let options = PyDict::new(py);
    for r in core(py)?.call_method0("view_layouts")?.try_iter()? {
        let r = r?;
        options.set_item(
            crate::settings_rows::tr(py, &r.get_item(1)?.extract::<String>()?)?,
            r.get_item(0)?,
        )?;
    }
    let idx = options
        .values()
        .call_method1("index", (doc.getattr("view_layout")?,))?;
    let kw = PyDict::new(py);
    kw.set_item(
        "done",
        bind(
            wrap_pyfunction!(choice, py)?.into_any(),
            &[doc, options.clone().into_any()],
        )?,
    )?;
    module(py, "chrome_forms")?.call_method(
        "ask_choice",
        (
            win,
            crate::settings_rows::tr(py, "Model Views")?,
            crate::settings_rows::tr(py, "Layout")?,
            options,
            idx,
        ),
        Some(&kw),
    )?;
    Ok(())
}
#[pyfunction(name = "__getattr__")]
fn resolve<'p>(py: Python<'p>, name: &str) -> PyResult<Obj<'p>> {
    let members = PyDict::new(py);
    let m = types(py)?;
    let partial = py.import("functools")?.getattr("partialmethod")?;
    let prop = py.import("builtins")?.getattr("property")?;
    macro_rules! methods {($($label:literal => $func:ident),* $(,)?)=>{$(members.set_item($label,partial.call1((wrap_pyfunction!($func,py)?,))?)?;)*};}
    let bases = match name {
        "ShellView" => {
            methods!("__init__"=>view_init,"_presentation_timers"=>presentation_timers,"move_timers"=>move_timers,"_window_handles"=>handles,"winId"=>window_id,"_can_create_surface"=>visible,"isVisible"=>visible,"showEvent"=>show,"resizeEvent"=>resize,"hideEvent"=>hide,"leaveEvent"=>leave,"closeEvent"=>close);
            members.set_item("_shim_visible", true)?;
            vec![
                py.import("views.wgpu_view")?.getattr("MetalView")?,
                py.import("views.view_host")?.getattr("WidgetShim")?,
            ]
        }
        "Pane" => {
            methods!("mapToGlobal"=>to_global,"mapFromGlobal"=>from_global,"_attach"=>attach,"draw_frame"=>draw_frame,"dispose"=>dispose);
            members.set_item(
                "chrome_offset",
                prop.call1((wrap_pyfunction!(offset, py)?,))?,
            )?;
            vec![
                py.import("views.resident_peer")?.getattr("ResidentPeer")?,
                m.getattr("ShellView")?,
            ]
        }
        "ShellDocument" => {
            methods!("__init__"=>document_init);
            members.set_item("view_layout", prop.call1((wrap_pyfunction!(layout, py)?,))?)?;
            vec![py.import("core.document_host")?.getattr("Document")?]
        }
        _ => return Err(PyAttributeError::new_err(name.to_owned())),
    };
    members.set_item("__module__", "ingetrazo_render.shell_views")?;
    let class = py
        .import("builtins")?
        .call_method1("type", (name, PyTuple::new(py, bases)?, members))?;
    m.add(name, &class)?;
    Ok(class)
}
pub(super) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = &create(parent, "shell_views")?;
    m.add_class::<PaneHost>()?;
    for f in [
        wrap_pyfunction!(make_pane, m)?,
        wrap_pyfunction!(choose, m)?,
        wrap_pyfunction!(resolve, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}

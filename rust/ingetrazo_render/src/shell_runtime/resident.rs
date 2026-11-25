//! A pane shares the primary display's resident geometry. Every other
//! renderer access, even a cached bound method, selects the pane's target.
use super::*;
use pyo3::class::gc::{PyTraverseError, PyVisit};
use pyo3::exceptions::PyAttributeError;

#[pyclass(module = "ingetrazo_render.shell_views")]
pub(super) struct TargetRenderer {
    #[pyo3(get)]
    raw: Option<Py<PyAny>>,
    #[pyo3(get)]
    target: u32,
}
#[pymethods]
impl TargetRenderer {
    #[new]
    pub(super) fn new(raw: Py<PyAny>, target: u32) -> Self {
        Self {
            raw: Some(raw),
            target,
        }
    }
    fn __getattr__<'p>(&self, py: Python<'p>, name: &str) -> PyResult<Obj<'p>> {
        let raw = self
            .raw
            .as_ref()
            .ok_or_else(|| PyAttributeError::new_err(name.to_owned()))?
            .bind(py);
        raw.call_method1("bind_target", (self.target,))?;
        let value = raw.getattr(name)?;
        if !value.is_callable() {
            return Ok(value);
        }
        let method = py.import("functools")?.call_method1(
            "partial",
            (wrap_pyfunction!(target_call, py)?, raw, self.target, value),
        )?;
        method.setattr("__name__", name)?;
        Ok(method)
    }
    fn __repr__(&self, py: Python) -> PyResult<String> {
        Ok(format!(
            "<TargetRenderer {} of {}>",
            self.target,
            self.raw
                .as_ref()
                .map(|r| r.bind(py).repr())
                .transpose()?
                .map(|s| s.to_string())
                .unwrap_or_default()
        ))
    }
    fn __traverse__(&self, visit: PyVisit) -> Result<(), PyTraverseError> {
        visit.call(&self.raw)
    }
    fn __clear__(&mut self) {
        self.raw = None;
    }
}
#[pyfunction(signature=(raw,target,method,/,*args,**kwargs))]
fn target_call<'p>(
    raw: &Obj<'p>,
    target: u32,
    method: &Obj<'p>,
    args: &Bound<'p, PyTuple>,
    kwargs: Option<&Bound<'p, PyDict>>,
) -> PyResult<Obj<'p>> {
    raw.call_method1("bind_target", (target,))?;
    method.call(args, kwargs)
}
#[pyfunction]
pub(super) fn raw_renderer<'p>(renderer: &Obj<'p>) -> PyResult<Obj<'p>> {
    if renderer.is_instance_of::<TargetRenderer>() {
        renderer.getattr("raw")
    } else {
        Ok(renderer.clone())
    }
}

const SHARED: &[&str] = &[
    "packed",
    "scene",
    "uid_rows",
    "inference_index",
    "_inference_future",
    "_inference_worker",
    "_inference_queued",
    "_instance_updates",
    "_content_token",
    "_paint_key",
    "_billboards",
    "_container_boxes",
    "_container_box_rows",
    "_resident_count",
    "_resident_edit_copies",
    "_scene_dirty",
    "_remote_meshes",
    "_parent_map",
    "_rest_signature",
    "_selection_rows",
    "suppressed_faces",
    "build_seconds",
    "upload_seconds",
    "after_open",
];
#[pyfunction]
fn init(view: &Obj, primary: &Obj, host: &Obj) -> PyResult<()> {
    view.setattr("primary", primary)?;
    super::views::view_init(view, host)?;
    view.setattr("_target", view.py().None())?;
    view.setattr("_peer_ready", true)?;
    primary.call_method1("add_peer", (view,))?;
    Ok(())
}
#[pyfunction]
fn primary<'p>(view: &Obj<'p>) -> PyResult<Obj<'p>> {
    view.getattr("primary")
}
#[pyfunction]
fn read<'p>(name: &str, view: &Obj<'p>) -> PyResult<Obj<'p>> {
    primary(view)?.getattr(name)
}
#[pyfunction]
fn write(name: &str, view: &Obj, value: &Obj) -> PyResult<()> {
    if flag(view, "_peer_ready")? {
        primary(view)?.setattr(name, value)?;
    }
    Ok(())
}
#[pyfunction]
fn drop_shared(name: &str, view: &Obj) -> PyResult<()> {
    if flag(view, "_peer_ready")? {
        primary(view)?.delattr(name)?;
    }
    Ok(())
}
#[pyfunction(signature=(view,scene,*,progress=None,opening=false))]
fn set_scene(view: &Obj, scene: &Obj, progress: Option<&Obj>, opening: bool) -> PyResult<()> {
    let kw = PyDict::new(view.py());
    kw.set_item("progress", progress)?;
    kw.set_item("opening", opening)?;
    primary(view)?.call_method("set_scene", (scene,), Some(&kw))?;
    view.call_method0("update")?;
    Ok(())
}
#[pyfunction(signature=(method,view,*args,**kwargs))]
fn on_primary<'p>(
    method: &str,
    view: &Obj<'p>,
    args: &Bound<'p, PyTuple>,
    kwargs: Option<&Bound<'p, PyDict>>,
) -> PyResult<Obj<'p>> {
    primary(view)?.call_method(method, args, kwargs)
}
#[pyfunction(signature=(view,force=false))]
fn upload(view: &Obj, force: bool) -> PyResult<()> {
    if force
        || (view.call_method0("isVisible")?.is_truthy()?
            && view.call_method0("_can_create_surface")?.is_truthy()?)
    {
        let primary = primary(view)?;
        if flag(&primary, "_scene_dirty")? || primary.getattr("renderer")?.is_none() {
            let kw = PyDict::new(view.py());
            kw.set_item("force", true)?;
            primary.call_method("_upload", (), Some(&kw))?;
        }
        view.call_method0("_attach")?;
    }
    view.call_method0("update")?;
    Ok(())
}
#[pyfunction]
fn release(view: &Obj) -> PyResult<()> {
    let renderer = view.getattr("renderer")?;
    view.setattr("renderer", view.py().None())?;
    if let Ok(wrapper) = renderer.cast::<TargetRenderer>() {
        let wrapper = wrapper.borrow();
        if let Some(raw) = &wrapper.raw {
            // The primary renderer can already have disposed this target.
            let _ = raw.call_method1(view.py(), "remove_target", (wrapper.target,));
        }
    }
    primary(view)?.call_method1("remove_peer", (view,))?;
    Ok(())
}
pub(super) fn pane_methods(members: &Bound<PyDict>) -> PyResult<()> {
    let py = members.py();
    let methods = py.import("functools")?.getattr("partialmethod")?;
    let prop = py.import("builtins")?.getattr("property")?;
    for (name, f) in [
        ("__init__", wrap_pyfunction!(init, py)?),
        ("set_scene", wrap_pyfunction!(set_scene, py)?),
        ("_upload", wrap_pyfunction!(upload, py)?),
        ("release", wrap_pyfunction!(release, py)?),
    ] {
        members.set_item(name, methods.call1((f,))?)?;
    }
    members.set_item("_shared", prop.call1((wrap_pyfunction!(primary, py)?,))?)?;
    let prefix = py.import("functools")?.getattr("partial")?;
    for name in [
        "blank",
        "_prepare_inference",
        "_refresh_inference",
        "_reserve_edit_room",
    ] {
        let delegate = prefix.call1((wrap_pyfunction!(on_primary, py)?, name))?;
        members.set_item(name, methods.call1((delegate,))?)?;
    }
    for &name in SHARED {
        members.set_item(
            name,
            prop.call1((
                prefix.call1((wrap_pyfunction!(read, py)?, name))?,
                prefix.call1((wrap_pyfunction!(write, py)?, name))?,
                prefix.call1((wrap_pyfunction!(drop_shared, py)?, name))?,
            ))?,
        )?;
    }
    Ok(())
}

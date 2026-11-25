// SPDX-License-Identifier: GPL-3.0-or-later
//! Saved deliveries on the window: the delivery and one-off export form,
//! the SVG writer's context (screen size, the view's picture), progress in
//! the job capsule, the PDF writer bridge, the window's serial export queue
//! and the dock's `dlv:` actions.
//!
//! `built_core`'s `delivery_form.rs` owns the formats, scopes, visible
//! fields, record and validation; `sheet_host.rs` shows the form. What stays
//! Python: the writers themselves (`built_core.deliveries.write_steps`,
//! `preview_svg`, the IFC writer behind them) and the protocol's delivery
//! commands.
use pyo3::{
    exceptions::PyInterruptedError,
    prelude::*,
    types::{IntoPyDict, PyDict, PyList, PyTuple},
};
use std::sync::atomic::{AtomicU64, Ordering};

fn opt<'py>(o: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    o.getattr(name).ok().filter(|v| !v.is_none())
}

fn json_dumps(py: Python<'_>, v: &Bound<'_, PyAny>) -> PyResult<String> {
    let kwargs = [("default", py.import("builtins")?.getattr("str")?)].into_py_dict(py)?;
    py.import("json")?.getattr("dumps")?.call((v,), Some(&kwargs))?.extract()
}

fn json_loads<'py>(py: Python<'py>, s: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("json")?.call_method1("loads", (s,))
}

fn dialogs(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_render.chrome_dialogs")
}

fn deliveries(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_core.deliveries")
}

fn run_command<'py>(py: Python<'py>, history: &Bound<'py, PyAny>, name: &str, kwargs: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyAny>> {
    py.import("built_core.command_protocol")?.getattr("run")?.call((history, name), Some(kwargs))
}

fn history_of<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    win.getattr("viewport")?.getattr("history")
}

/// The view the window draws in, else the active one.
fn active_view<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if win.hasattr("_active_view")? {
        win.call_method0("_active_view")
    } else {
        win.getattr("viewport")
    }
}

/// Run `fn` on the next turn of the window's loop (its host's queue, or
/// the headless one, `built_core.timers`).
fn next_turn(win: &Bound<'_, PyAny>, f: Bound<'_, PyAny>) -> PyResult<()> {
    win.py().import("built_core.timers")?.call_method1("later", (win, 0, f))?;
    Ok(())
}

/// Seconds a preview or export works before its job shows in the capsule.
static PREVIEW_JOB_DELAY: AtomicU64 = AtomicU64::new(0x3FE0_0000_0000_0000); // 0.5

fn preview_job_delay() -> f64 {
    f64::from_bits(PREVIEW_JOB_DELAY.load(Ordering::Relaxed))
}

/// Set the delay (tests); returns the previous one.
#[pyfunction]
fn set_preview_job_delay(seconds: f64) -> f64 {
    f64::from_bits(PREVIEW_JOB_DELAY.swap(seconds.to_bits(), Ordering::Relaxed))
}

fn monotonic(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("monotonic")?.extract()
}

/// One step of a job's progress: the capsule, a turn of the loop, and
/// cancellation (the job's Cancel, or the document went).
fn report_job(win: &Bound<'_, PyAny>, history: &Bound<'_, PyAny>, job: &Bound<'_, PyAny>, fraction: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    job.call_method1("update", (fraction,))?;
    py.import("built_render.shell_modal")?.call_method0("tick")?; // a turn of the shell's loop; none headless
    if job.getattr("cancelled")?.is_truthy()? || !history_of(win)?.is(history) {
        return Err(PyInterruptedError::new_err("Delivery cancelled"));
    }
    Ok(())
}

#[pyclass(module = "built_render")]
struct JobReport {
    win: Py<PyAny>,
    history: Py<PyAny>,
    job: Py<PyAny>,
}

#[pymethods]
impl JobReport {
    fn __call__(&self, py: Python<'_>, fraction: &Bound<'_, PyAny>) -> PyResult<()> {
        report_job(self.win.bind(py), self.history.bind(py), self.job.bind(py), fraction)
    }
}

#[pyclass(module = "built_render")]
struct Nothing;

#[pymethods]
impl Nothing {
    #[pyo3(signature = (*_args))]
    fn __call__(&self, _args: &Bound<'_, PyTuple>) {}
}

fn job<'py>(win: &Bound<'py, PyAny>, label: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let kwargs = [("on_cancel", Py::new(py, Nothing)?)].into_py_dict(py)?;
    dialogs(py)?.getattr("Job")?.call((win, label), Some(&kwargs))
}

/// `with svg_progress(win, history, label, current):` slow SVG work shows
/// in the job capsule after a moment; a superseded preview, a cancel or a
/// document change stops it with InterruptedError.
#[pyclass(module = "built_render")]
pub struct SvgProgress {
    win: Py<PyAny>,
    history: Py<PyAny>,
    label: Py<PyAny>,
    current: Option<Py<PyAny>>,
    started: f64,
    job: Option<Py<PyAny>>,
    reporting: Option<Py<PyAny>>,
}

impl SvgProgress {
    fn current(&self, py: Python<'_>) -> PyResult<bool> {
        match &self.current {
            Some(c) => c.bind(py).call0()?.is_truthy(),
            None => Ok(true),
        }
    }

    fn report(&mut self, py: Python<'_>, fraction: &Bound<'_, PyAny>) -> PyResult<()> {
        let win = self.win.bind(py);
        if !self.current(py)? || !history_of(win)?.is(self.history.bind(py)) {
            return Err(PyInterruptedError::new_err(()));
        }
        if self.job.is_none() {
            if monotonic(py)? - self.started < preview_job_delay() {
                return Ok(());
            }
            self.job = Some(job(win, self.label.bind(py))?.unbind());
        }
        report_job(win, self.history.bind(py), self.job.as_ref().expect("made above").bind(py), fraction)?;
        if !self.current(py)? {
            return Err(PyInterruptedError::new_err(()));
        }
        Ok(())
    }

    /// What the generator's except and finally clauses did.
    fn settle(&mut self, py: Python<'_>, error: Option<&PyErr>) -> PyResult<()> {
        let Some(job) = self.job.take() else { return Ok(()) };
        let job = job.bind(py);
        if let Some(e) = error {
            if e.is_instance_of::<PyInterruptedError>(py) {
                job.call_method0("cancel")?;
            } else if e.is_instance_of::<pyo3::exceptions::PyException>(py) {
                let kwargs = PyDict::new(py);
                kwargs.set_item("failed", true)?;
                kwargs.set_item("detail", e.value(py).str()?)?;
                job.call_method("end", (), Some(&kwargs))?;
            }
        }
        job.call_method0("end")?;
        Ok(())
    }
}

#[pymethods]
impl SvgProgress {
    #[new]
    #[pyo3(signature = (win, history, label, current=None))]
    fn new(win: Py<PyAny>, history: Py<PyAny>, label: Py<PyAny>, current: Option<Py<PyAny>>) -> Self {
        Self { win, history, label, current, started: 0.0, job: None, reporting: None }
    }

    /// `built_core.progress` reports a fraction here.
    fn __call__(&mut self, py: Python<'_>, fraction: &Bound<'_, PyAny>) -> PyResult<()> {
        self.report(py, fraction)
    }

    fn __enter__(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        slf.borrow_mut().started = monotonic(py)?;
        let zero = 0i32.into_pyobject(py)?.into_any();
        let first = slf.borrow_mut().report(py, &zero);
        if let Err(e) = first {
            slf.borrow_mut().settle(py, Some(&e))?;
            return Err(e);
        }
        let reporting = py.import("built_core.progress")?.call_method1("reporting", (slf,))?;
        reporting.call_method0("__enter__")?;
        slf.borrow_mut().reporting = Some(reporting.unbind());
        Ok(())
    }

    fn __exit__(
        slf: &Bound<'_, Self>,
        kind: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        traceback: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let py = slf.py();
        if let Some(reporting) = slf.borrow_mut().reporting.take() {
            reporting.bind(py).call_method1("__exit__", (kind, value, traceback))?;
        }
        if !value.is_none() {
            let error = PyErr::from_value(value.clone());
            slf.borrow_mut().settle(py, Some(&error))?;
            return Ok(false);
        }
        let one = 1i32.into_pyobject(py)?.into_any();
        let last = slf.borrow_mut().report(py, &one);
        match last {
            Ok(()) => {
                slf.borrow_mut().settle(py, None)?;
                Ok(false)
            }
            Err(e) => {
                slf.borrow_mut().settle(py, Some(&e))?;
                Err(e)
            }
        }
    }
}

#[pyfunction]
#[pyo3(signature = (win, history, label, current=None))]
fn svg_progress(win: Py<PyAny>, history: Py<PyAny>, label: Py<PyAny>, current: Option<Py<PyAny>>) -> SvgProgress {
    SvgProgress::new(win, history, label, current)
}

// ---- the SVG writer's context ----------------------------------------------------------

/// The view's picture under the SVG's lines: the renderer's frame at the
/// export (or preview) width, with or without the axes, framed as asked.
#[pyclass(module = "built_render")]
struct SvgPicture {
    win: Py<PyAny>,
    active: Py<PyAny>,
    scene: Py<PyAny>,
    preview: bool,
}

#[pymethods]
impl SvgPicture {
    fn __call__<'py>(&self, py: Python<'py>, axes: bool, frame: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let svg = py.import("built_render.view_export")?;
        let win = self.win.bind(py);
        let active = self.active.bind(py);
        let scene = self.scene.bind(py);
        let renderer = Some(dialogs(py)?.call_method1("view_of", (win,))?).filter(|v| v.is_truthy().unwrap_or(false)).unwrap_or_else(|| active.clone());
        let old = scene.getattr("show_axes")?;
        scene.setattr("show_axes", old.is_truthy()? && axes)?;
        let work = || -> PyResult<Bound<'py, PyAny>> {
            let framing = if frame.is_truthy()? { Some(svg.call_method1("framed", (active.getattr("camera")?, frame))?) } else { None };
            if let Some(f) = &framing {
                f.call_method0("__enter__")?;
            }
            let picture = (|| -> PyResult<Bound<'py, PyAny>> {
                let width = svg.getattr(if self.preview { "PREVIEW_IMAGE_PX" } else { "EXPORT_IMAGE_PX" })?;
                let kwargs = [("overlays", false), ("sharp_shadows", true)].into_py_dict(py)?;
                let display = opt(&renderer, "keys_stand_in").unwrap_or_else(|| renderer.clone());
                let pixels = display.getattr("render_pixels")?.call((width,), Some(&kwargs))?;
                let image = if pixels.is_truthy()? {
                    py.import("built_render")?.getattr("SurveyRaster")?.call1(pixels.cast_into::<PyTuple>()?)?
                } else {
                    py.None().into_bound(py)
                };
                let rect = py.import("core.vista_caras")?.call_method1("view_rect", (active.getattr("camera")?,))?;
                Ok(PyTuple::new(py, [image, rect])?.into_any())
            })();
            if let Some(f) = &framing {
                match &picture {
                    Ok(_) => {
                        f.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
                    }
                    Err(e) => {
                        f.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?;
                    }
                }
            }
            picture
        };
        let result = work();
        scene.setattr("show_axes", old)?;
        result
    }
}

fn svg_context<'py>(win: &Bound<'py, PyAny>, active: &Bound<'py, PyAny>, scene: &Bound<'py, PyAny>, record: &Bound<'py, PyAny>, preview: bool) -> PyResult<Bound<'py, PyDict>> {
    let py = win.py();
    let ctx = PyDict::new(py);
    let options = record.get_item("options")?;
    if options.call_method1("get", ("paper",))?.eq("screen")? {
        let viewport = Some(dialogs(py)?.call_method1("view_of", (win,))?).filter(|v| v.is_truthy().unwrap_or(false)).unwrap_or_else(|| active.clone());
        let w: f64 = viewport.call_method0("width")?.extract()?;
        let h: f64 = viewport.call_method0("height")?.extract()?;
        ctx.set_item("screen_mm", vec![w * 25.4 / 96.0, h * 25.4 / 96.0])?;
    }
    if options.call_method1("get", ("picture",))?.is_truthy()? {
        ctx.set_item("svg_picture", Py::new(py, SvgPicture { win: win.clone().unbind(), active: active.clone().unbind(), scene: scene.clone().unbind(), preview })?)?;
    }
    Ok(ctx)
}

// ---- the form ----------------------------------------------------------------------------

fn unpack<'py>(py: Python<'py>, svg: &Bound<'py, PyAny>, v: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let options = if v.call_method1("get", ("format",))?.eq("svg")? { svg.call_method0("options_json")?.extract::<String>()? } else { "{}".to_owned() };
    let record: String = py.import("built_core")?.call_method1("delivery_record", (json_dumps(py, v)?, options))?.extract()?;
    json_loads(py, &record)
}

/// Whether field `fid` shows for the form's format and scope.
#[pyclass(module = "built_render")]
struct FieldShows {
    fid: String,
}

#[pymethods]
impl FieldShows {
    fn __call__<'py>(&self, py: Python<'py>, form: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        py.import("built_core")?.call_method1("delivery_field_visible", (&self.fid, form.call_method1("value", ("format",))?, form.call_method1("value", ("scope",))?))
    }
}

fn scopes<'py>(py: Python<'py>, fmt: &Bound<'py, PyAny>, has_nodes: bool) -> PyResult<Bound<'py, PyAny>> {
    py.import("built_core")?.call_method1("delivery_scopes", (fmt, has_nodes))
}

/// A new format: its scopes and file filter.
#[pyclass(module = "built_render")]
struct FormChanged {
    has_nodes: bool,
}

#[pymethods]
impl FormChanged {
    fn __call__(&self, py: Python<'_>, form: &Bound<'_, PyAny>, fid: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        if fid == "format" {
            let choices = scopes(py, value, self.has_nodes)?;
            form.call_method1("field", ("scope",))?.setattr("options", &choices)?;
            let ids = py.import("builtins")?.getattr("dict")?.call1((&choices,))?;
            if !ids.contains(form.call_method1("value", ("scope",))?)? {
                form.call_method1("set", ("scope", choices.get_item(0)?.get_item(0)?))?;
            }
            let fmt: String = value.str()?.to_string();
            form.call_method1("field", ("destination",))?.setattr("filters", format!("{} (*.{fmt})", fmt.to_uppercase()))?;
        }
        if fid == "format" || fid == "scope" {
            form.call_method0("changed")?;
        }
        Ok(())
    }
}

/// The form's validation: the SVG options, then the delivery record.
#[pyclass(module = "built_render")]
struct FormCheck {
    export_scene: Option<Py<PyAny>>,
    svg: Py<PyAny>,
}

#[pymethods]
impl FormCheck {
    fn __call__(&self, py: Python<'_>, form: &Bound<'_, PyAny>, _answer: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
        let svg = self.svg.bind(py);
        let check = || -> PyResult<Option<String>> {
            if form.call_method1("value", ("format",))?.eq("svg")? {
                let error = svg.call_method0("validation_error")?;
                if error.is_truthy()? {
                    return Ok(Some(error.extract()?));
                }
            }
            let record = unpack(py, svg, &form.call_method0("values")?)?;
            let backend = deliveries(py)?;
            backend.call_method1("validate", (&record,))?;
            if let Some(scene) = &self.export_scene {
                if record.get_item("format")?.eq("ifc")? {
                    let scoped = backend.call_method1("scoped", (scene.bind(py), &record))?;
                    let objects = py.import("core.bim")?.call_method1("collect_objects", (scoped,))?;
                    if !objects.is_truthy()? {
                        let text = "Nothing to export: tag geometry in the BIM panel first (only tagged objects go to IFC).";
                        return Ok(Some(py.import("built_core.i18n")?.call_method1("tr", (text,))?.extract()?));
                    }
                }
            }
            Ok(None)
        };
        match check() {
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => Ok(Some(e.value(py).str()?.to_string())),
            other => other,
        }
    }
}

/// The form's answer: save or edit the delivery, and for a one-off export
/// queue it.
#[pyclass(module = "built_render")]
struct FormDone {
    win: Py<PyAny>,
    history: Py<PyAny>,
    key: Option<String>,
    one_off: bool,
    svg: Py<PyAny>,
}

#[pymethods]
impl FormDone {
    fn __call__(&self, py: Python<'_>, answer: &str, v: &Bound<'_, PyAny>) -> PyResult<()> {
        let win = self.win.bind(py);
        let history = self.history.bind(py);
        if answer == "cancel" || !history_of(win)?.is(history) {
            return Ok(());
        }
        let r = unpack(py, self.svg.bind(py), v)?;
        let mut delivery_key = self.key.as_ref().map(|k| k.clone().into_pyobject(py).map(|s| s.into_any())).transpose()?;
        if let Some(key) = &self.key {
            let kwargs = PyDict::new(py);
            kwargs.set_item("key", key)?;
            kwargs.set_item("changes", &r)?;
            run_command(py, history, "edit_delivery", &kwargs)?;
        } else if answer != "export" {
            let kwargs = [("delivery", &r)].into_py_dict(py)?;
            delivery_key = Some(run_command(py, history, "create_delivery", &kwargs)?.get_item("key")?);
        }
        if self.one_off {
            if r.get_item("format")?.eq("stl")? {
                let unit = r.get_item("options")?.get_item("unit")?;
                py.import("built_core.settings")?.call_method0("settings")?.call_method1("setValue", ("export/stl_unit", unit))?;
            }
            let key = delivery_key.clone().unwrap_or_else(|| py.None().into_bound(py));
            enqueue(win, &key, &r)?;
        }
        let view = dialogs(py)?.call_method1("view_of", (win,))?;
        if view.is_truthy()? {
            view.setattr("_delivery_key", delivery_key.unwrap_or_else(|| py.None().into_bound(py)))?;
            view.setattr("_ui_tree_sig", py.None())?;
            view.call_method0("update")?;
        }
        Ok(())
    }
}

/// The SVG preview of the form's record, as pixels for the sheet.
#[pyclass(module = "built_render")]
struct FormPreview {
    win: Py<PyAny>,
    history: Py<PyAny>,
    svg: Py<PyAny>,
    form: Option<Py<PyAny>>,
}

/// Whether the preview `ticket` is still the one wanted.
#[pyclass(module = "built_render")]
struct PreviewCurrent {
    form: Py<PyAny>,
    svg: Py<PyAny>,
    ticket: Py<PyAny>,
}

#[pymethods]
impl PreviewCurrent {
    fn __call__(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(!self.form.bind(py).getattr("closed")?.is_truthy()? && self.svg.bind(py).call_method1("preview_current", (self.ticket.bind(py),))?.is_truthy()?)
    }
}

#[pymethods]
impl FormPreview {
    fn __call__<'py>(&self, py: Python<'py>, ticket: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let win = self.win.bind(py);
        let history = self.history.bind(py);
        let svg = self.svg.bind(py);
        let form = self.form.as_ref().expect("bound after the form").bind(py);
        let active = active_view(win)?;
        let record = unpack(py, svg, &form.call_method0("values")?)?;
        let current = Py::new(py, PreviewCurrent { form: form.clone().unbind(), svg: svg.clone().unbind(), ticket: ticket.clone().unbind() })?;
        let label = "SVG preview".into_pyobject(py)?.into_any();
        let progress = Bound::new(py, SvgProgress::new(win.clone().unbind(), history.clone().unbind(), label.unbind(), Some(current.into_any())))?;
        let scene = history.getattr("scene")?;
        SvgProgress::__enter__(&progress)?;
        let text = (|| -> PyResult<Bound<'py, PyAny>> {
            let kwargs = svg_context(win, &active, &scene, &record, true)?;
            kwargs.set_item("camera", active.getattr("camera")?)?;
            deliveries(py)?.getattr("preview_svg")?.call((&scene, &record), Some(&kwargs))
        })();
        let none = py.None().into_bound(py);
        let text = match text {
            Ok(t) => {
                SvgProgress::__exit__(&progress, &none, &none, &none)?;
                t
            }
            Err(e) => {
                let (kind, value) = (e.get_type(py).into_any(), e.value(py).clone().into_any());
                SvgProgress::__exit__(&progress, &kind, &value, &none)?;
                return Err(e);
            }
        };
        py.import("built_render")?.call_method1("svg_preview", (text,))
    }
}

/// The delivery form (`key`) or the one-off Export form, on the window's
/// sheet.
pub fn export_form<'py>(win: &Bound<'py, PyAny>, key: Option<String>, one_off: bool, initial_format: &str) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let history = history_of(win)?;
    let scene = history.getattr("scene")?;
    let all = deliveries(py)?.call_method1("records", (&scene,))?;
    let record = match &key {
        Some(k) => all.get_item(k)?,
        None => PyDict::new(py).into_any(),
    };
    let options = record.call_method1("get", ("options", PyDict::new(py)))?;
    let registry = py.import("core.bim_registry")?.call_method1("get", (&scene,))?;
    let nodes = py.import("core.link_structure")?.getattr("Tree")?.call1((&scene, &registry))?.getattr("nodes")?;
    let has_nodes = nodes.is_truthy()?;
    let fmt = record.call_method1("get", ("format", initial_format))?;
    let stl_unit = py.import("built_core.settings")?.call_method0("settings")?.call_method1("value", ("export/stl_unit", "mm"))?.str()?;
    let core = py.import("built_core")?;
    let defaults = core.call_method1("delivery_defaults", (&fmt, one_off, stl_unit))?;
    let (scope, unit) = (defaults.get_item(0)?, defaults.get_item(1)?);
    let view = dialogs(py)?.call_method1("view_of", (win,))?;
    let screen = if view.is_truthy()? {
        let w: f64 = view.call_method0("width")?.extract()?;
        let h: f64 = view.call_method0("height")?.extract()?;
        (w * 25.4 / 96.0, h * 25.4 / 96.0)
    } else {
        (297.0, 210.0)
    };
    let svg = py.import("built_render")?.getattr("SvgOptions")?.call1((json_dumps(py, &options)?, screen))?;
    let forms = py.import("built_render.chrome_forms")?;
    let field_cls = forms.getattr("Field")?;
    let field = |fid: &str, kind: &str, label: &str, value: Bound<'py, PyAny>, extra: Vec<(&str, Bound<'py, PyAny>)>| -> PyResult<Bound<'py, PyAny>> {
        let kwargs = PyDict::new(py);
        for (k, v) in extra {
            kwargs.set_item(k, v)?;
        }
        field_cls.call((fid, kind, label, value), Some(&kwargs))
    };
    let shows = |fid: &str| -> PyResult<Bound<'py, PyAny>> { Ok(Py::new(py, FieldShows { fid: fid.into() })?.into_bound(py).into_any()) };
    let any = |v: &str| -> Bound<'py, PyAny> { v.into_pyobject(py).expect("str").into_any() };
    let get = |d: &Bound<'py, PyAny>, k: &str, default: Bound<'py, PyAny>| d.call_method1("get", (k, default));
    let formats = PyList::empty(py);
    for k in deliveries(py)?.getattr("FORMATS")?.try_iter()? {
        let k = k?;
        formats.append((&k, k.str()?.to_string().to_uppercase()))?;
    }
    let node_choices = PyList::empty(py);
    for (k, n) in nodes.cast::<PyDict>()?.iter() {
        node_choices.append((&k, n.get_item("name")?))?;
    }
    let first_node = nodes.try_iter()?.next().transpose()?.unwrap_or_else(|| any(""));
    let names = options.call_method1("get", ("names", PyList::empty(py)))?;
    let names = any("\n").call_method1("join", (names,))?;
    let profiles = PyList::new(py, [(any(""), any("Default IFC4"))])?;
    for (k, v) in registry.call_method1("get", ("export_profiles", PyDict::new(py)))?.cast_into::<PyDict>()?.iter() {
        profiles.append((&k, v.call_method1("get", ("name", &k))?))?;
    }
    let fmt_text = fmt.str()?.to_string();
    let filters = any(&format!("{} (*.{fmt_text})", fmt_text.to_uppercase()));
    let fields = vec![
        field("name", "edit", "Name", get(&record, "name", any("New delivery"))?, vec![("focus", true.into_pyobject(py)?.to_owned().into_any())])?,
        field("format", "choice", "Format", fmt.clone(), vec![("options", formats.into_any())])?,
        field("scope", "choice", "Scope", get(&record, "scope", scope)?, vec![("options", scopes(py, &fmt, has_nodes)?)])?,
        field("node", "choice", "Project node", get(&record, "node", first_node)?, vec![("options", node_choices.into_any()), ("visible", shows("node")?)])?,
        field("names", "edit", "Names (one per line, blank for all)", names, vec![("multiline", true.into_pyobject(py)?.to_owned().into_any()), ("visible", shows("names")?)])?,
        field("profile", "choice", "IFC profile", get(&options, "profile", any(""))?, vec![("options", profiles.into_any()), ("visible", shows("profile")?)])?,
        field("textures", "check", "Embed textures", get(&options, "textures", true.into_pyobject(py)?.to_owned().into_any())?, vec![("visible", shows("textures")?)])?,
        field("unit", "choice", "Units", get(&options, "unit", unit)?, vec![("options", PyList::new(py, ["mm", "cm", "m", "in", "ft"])?.into_any()), ("visible", shows("unit")?)])?,
        field("destination", "path", "Destination", get(&record, "destination", any(""))?, vec![("pick", any("save")), ("filters", filters)])?,
    ];
    let mut buttons = vec![("cancel", "Cancel")];
    if one_off {
        buttons.extend([("keep", "Keep as delivery"), ("export", "Export")]);
    } else {
        buttons.push(("save", "Save delivery"));
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("buttons", &buttons)?;
    kwargs.set_item("default", buttons.last().expect("buttons").0)?;
    kwargs.set_item("on_done", Py::new(py, FormDone { win: win.clone().unbind(), history: history.clone().unbind(), key: key.clone(), one_off, svg: svg.clone().unbind() })?)?;
    kwargs.set_item("on_change", Py::new(py, FormChanged { has_nodes })?)?;
    kwargs.set_item("validate", Py::new(py, FormCheck { export_scene: one_off.then(|| scene.clone().unbind()), svg: svg.clone().unbind() })?)?;
    let form = forms.getattr("Form")?.call((if one_off { "Export" } else { "Delivery" }, fields), Some(&kwargs))?;
    let preview = Bound::new(py, FormPreview { win: win.clone().unbind(), history: history.clone().unbind(), svg: svg.clone().unbind(), form: None })?;
    preview.borrow_mut().form = Some(form.clone().unbind());
    py.import("built_render")?.call_method1("show_delivery_form", (win, &form, one_off, &svg, &preview))?;
    Ok(form)
}

#[pyfunction]
#[pyo3(name = "delivery_export_form", signature = (win, key=None, one_off=false, initial_format="ifc"))]
fn delivery_export_form_py<'py>(win: &Bound<'py, PyAny>, key: Option<String>, one_off: bool, initial_format: &str) -> PyResult<Bound<'py, PyAny>> {
    export_form(win, key, one_off, initial_format)
}

// ---- running deliveries -------------------------------------------------------------------

/// Protocol Run on this document submits to the window's queue.
#[pyclass(module = "built_render")]
struct DeliveryRunner {
    win: Py<PyAny>,
    history: Py<PyAny>,
}

#[pymethods]
impl DeliveryRunner {
    fn __call__<'py>(&self, py: Python<'py>, key: &Bound<'py, PyAny>, record: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let window = self.win.bind(py).call0()?;
        if window.is_none() || !history_of(&window)?.is(&self.history.bind(py).call0()?) {
            return Err(pyo3::exceptions::PyValueError::new_err("Activate this document to run its delivery"));
        }
        enqueue(&window, key, record)?.getattr("id")
    }
}

/// Bind protocol Run to the open document's job capsule once.
#[pyfunction]
fn bind_deliveries(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let history = history_of(win)?;
    if opt(&history, "delivery_runner").is_some() {
        return Ok(());
    }
    let weak = py.import("weakref")?.getattr("ref")?;
    let runner = DeliveryRunner { win: weak.call1((win,))?.unbind(), history: weak.call1((&history,))?.unbind() };
    history.setattr("delivery_runner", Py::new(py, runner)?)
}

/// The sheets' PDF from the Rust writer, the frames rendered by the
/// window's view; the composer's only without one.
#[pyclass(module = "built_render")]
struct PdfWriter {
    win: Py<PyAny>,
}

#[pymethods]
impl PdfWriter {
    fn __call__<'py>(&self, py: Python<'py>, path: &Bound<'py, PyAny>, sheets: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let win = self.win.bind(py);
        let view = opt(win, "_doc").and_then(|d| opt(&d, "stand_in"));
        let Some(view) = view.filter(|v| opt(v, "renderer").is_some()) else {
            return py.import("built_render.sheet_workspace")?.call_method1("composer_for", (win,))?.call_method1("export_all_pdf", (path, sheets));
        };
        let frames = py.import("built_render.sheet_frames")?;
        let scene = history_of(win)?.getattr("scene")?;
        let wanted: Option<Vec<Bound<'py, PyAny>>> = if sheets.is_none() { None } else { Some(sheets.try_iter()?.map(|s| s?.getattr("_doc_key")).collect::<PyResult<_>>()?) };
        let comps = PyList::empty(py);
        for c in scene.getattr("compositions")?.try_iter()? {
            let c = c?;
            let keep = match &wanted {
                None => true,
                Some(keys) => {
                    let k = c.getattr("_doc_key")?;
                    keys.iter().any(|w| w.eq(&k).unwrap_or(false))
                }
            };
            if keep {
                comps.append(c)?;
            }
        }
        let ground = frames.getattr("Ground")?.call1((Py::new(py, Nothing)?, &view))?;
        let none = py.None().into_bound(py);
        frames.call_method1("export_pdf", (&view, comps, path, ground, opt(win, "_current_path").unwrap_or_else(|| none.clone()), opt(win, "_ext_sheet_annots").unwrap_or(none)))
    }
}

#[pyclass(module = "built_render")]
struct OpenUrl {
    target: Py<PyAny>,
}

#[pymethods]
impl OpenUrl {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        py.import("built_core.desktop_open")?.call_method1("open_url", (self.target.bind(py),))?;
        Ok(())
    }
}

/// One step of a queued delivery per turn of the loop, then the next one.
#[pyclass(module = "built_render")]
struct DeliveryStep {
    win: Py<PyAny>,
    history: Py<PyAny>,
    key: Py<PyAny>,
    record: Py<PyAny>,
    job: Py<PyAny>,
    steps: Py<PyAny>,
    current: Option<Py<PyAny>>,
}

fn record_activity(win: &Bound<'_, PyAny>, title: String, detail: String, tone: &str, action: Option<(&str, Py<OpenUrl>)>) -> PyResult<()> {
    let py = win.py();
    let d = dialogs(py)?;
    match action {
        Some((label, run)) => d.call_method1("_record", (win, title, detail, tone, label, run))?,
        None => d.call_method1("_record", (win, title, detail, tone))?,
    };
    Ok(())
}

impl DeliveryStep {
    /// Returns whether the delivery goes on (another step is queued).
    fn step(slf: &Bound<'_, Self>) -> PyResult<bool> {
        let py = slf.py();
        let me = slf.borrow();
        let (win, history, job, record) = (me.win.bind(py), me.history.bind(py), me.job.bind(py), me.record.bind(py));
        let name = record.get_item("name")?.str()?.to_string();
        let cancelled = job.getattr("cancelled")?.is_truthy()?;
        let same = history_of(win)?.is(history);
        if !cancelled && !same {
            win.getattr("viewport")?.call_method1("flash_status", (format!("Cancelled {name} after the active document changed"),))?;
        }
        if cancelled || !same {
            return Ok(false);
        }
        let reporter = Py::new(py, JobReport { win: win.clone().unbind(), history: history.clone().unbind(), job: job.clone().unbind() })?;
        let reporting = py.import("built_core.progress")?.call_method1("reporting", (reporter,))?;
        reporting.call_method0("__enter__")?;
        let next = py.import("builtins")?.getattr("next")?.call1((me.steps.bind(py),));
        let none = py.None().into_bound(py);
        match &next {
            Ok(_) => reporting.call_method1("__exit__", (&none, &none, &none))?,
            Err(e) => reporting.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?,
        };
        match next {
            Ok(step) => {
                job.call_method1("update", (step.get_item(0)?, step.get_item(1)?))?;
                job.call_method0("draw_now")?;
                next_turn(win, slf.clone().into_any())?;
                Ok(true)
            }
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyStopIteration>(py) => {
                let key = me.key.bind(py);
                if deliveries(py)?.call_method1("records", (history.getattr("scene")?,))?.contains(key)? {
                    deliveries(py)?.call_method1("finish", (history, key))?;
                }
                let path = py.import("pathlib")?.getattr("Path")?;
                let dest = path.call1((record.get_item("destination")?,))?.call_method0("expanduser")?;
                let base = match &me.current {
                    Some(c) => path.call1((c.bind(py),))?.getattr("parent")?.div(&dest)?,
                    None => dest,
                };
                let folder = base.call_method0("resolve")?.getattr("parent")?;
                win.setattr("_delivery_result", (&name, &folder))?;
                let open = Py::new(py, OpenUrl { target: folder.clone().unbind() })?;
                record_activity(win, format!("Delivered · {name}"), folder.str()?.to_string(), "info", Some(("Open folder", open)))?;
                Ok(false)
            }
            Err(e) if e.is_instance_of::<PyInterruptedError>(py) => Ok(false),
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => {
                record_activity(win, format!("Failed · {name}"), e.value(py).str()?.to_string(), "error", None)?;
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
}

#[pymethods]
impl DeliveryStep {
    fn __call__(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        if DeliveryStep::step(slf)? {
            return Ok(());
        }
        let me = slf.borrow();
        let win = me.win.bind(py);
        me.steps.bind(py).call_method0("close")?;
        me.job.bind(py).call_method0("end")?;
        let queue = win.getattr("_delivery_queue")?;
        queue.call_method0("popleft")?;
        if queue.is_truthy()? {
            next_turn(win, queue.get_item(0)?)?;
        }
        Ok(())
    }
}

/// Queue the delivery `record` (`key` when saved) on the window: one job
/// in the capsule, written one step per turn, one delivery at a time.
pub fn enqueue<'py>(win: &Bound<'py, PyAny>, key: &Bound<'py, PyAny>, record: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let history = history_of(win)?;
    let deque = py.import("collections")?.getattr("deque")?.call0()?;
    let queue = win.getattr("__dict__")?.call_method1("setdefault", ("_delivery_queue", deque))?;
    let job = job(win, &record.get_item("name")?)?;
    job.call_method1("update", (0, "Queued"))?;
    let active = active_view(win)?;
    let ctx = PyDict::new(py);
    ctx.set_item("camera", py.import("copy")?.call_method1("copy", (active.getattr("camera")?,))?)?;
    let current = opt(win, "_current_path").filter(|c| c.is_truthy().unwrap_or(false));
    if let Some(c) = &current {
        ctx.set_item("project_directory", py.import("pathlib")?.getattr("Path")?.call1((c,))?.getattr("parent")?)?;
    }
    let fmt = record.get_item("format")?;
    if fmt.eq("dxf")? && record.get_item("scope")?.eq("visible")? {
        ctx.set_item("view_geometry", active.getattr("hlr_geometry")?)?;
    }
    if fmt.eq("pdf")? {
        ctx.set_item("pdf_writer", Py::new(py, PdfWriter { win: win.clone().unbind() })?)?;
    }
    if fmt.eq("svg")? {
        ctx.update(svg_context(win, &active, &history.getattr("scene")?, record, false)?.as_mapping())?;
    }
    let steps = deliveries(py)?.getattr("write_steps")?.call((history.getattr("scene")?, record), Some(&ctx))?;
    let advance = Py::new(py, DeliveryStep {
        win: win.clone().unbind(),
        history: history.unbind(),
        key: key.clone().unbind(),
        record: record.clone().unbind(),
        job: job.clone().unbind(),
        steps: steps.unbind(),
        current: current.map(Bound::unbind),
    })?;
    queue.call_method1("append", (&advance,))?;
    if queue.len()? == 1 {
        next_turn(win, advance.into_bound(py).into_any())?;
    }
    Ok(job)
}

#[pyfunction]
#[pyo3(name = "enqueue_delivery")]
fn enqueue_py<'py>(win: &Bound<'py, PyAny>, key: &Bound<'py, PyAny>, record: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    enqueue(win, key, record)
}

/// `dlv:<verb>[:<key>]` from the dock's Deliveries.
#[pyfunction]
fn delivery_action(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, arg: &str) -> PyResult<()> {
    let py = win.py();
    let (verb, given) = arg.split_once(':').unwrap_or((arg, ""));
    let key = if given.is_empty() { opt(view, "_delivery_key") } else { Some(given.into_pyobject(py)?.into_any()) };
    let vp = win.getattr("viewport")?;
    if verb == "new" {
        export_form(win, None, false, "ifc")?;
    } else if verb == "folder" {
        let result = win.getattr("__dict__")?.call_method1("pop", ("_delivery_result", py.None()))?;
        if result.is_truthy()? {
            py.import("built_core.desktop_open")?.call_method1("open_url", (result.get_item(1)?,))?;
        }
    } else if let Some(key) = key.filter(|k| deliveries(py).and_then(|d| d.call_method1("records", (vp.getattr("scene")?,))?.contains(k)).unwrap_or(false)) {
        let history = vp.getattr("history")?;
        match verb {
            "select" => view.setattr("_delivery_key", &key)?,
            "run" => {
                view.setattr("_delivery_key", &key)?;
                run_command(py, &history, "run_delivery", &[("key", &key)].into_py_dict(py)?)?;
            }
            "edit" => {
                export_form(win, Some(key.str()?.to_string()), false, "ifc")?;
            }
            "delete" => {
                run_command(py, &history, "delete_delivery", &[("key", &key)].into_py_dict(py)?)?;
            }
            _ => {}
        }
    }
    if view.is_truthy()? {
        view.setattr("_ui_tree_sig", py.None())?;
        view.call_method0("update")?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SvgProgress>()?;
    for f in [
        wrap_pyfunction!(set_preview_job_delay, m)?,
        wrap_pyfunction!(svg_progress, m)?,
        wrap_pyfunction!(delivery_export_form_py, m)?,
        wrap_pyfunction!(bind_deliveries, m)?,
        wrap_pyfunction!(enqueue_py, m)?,
        wrap_pyfunction!(delivery_action, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}

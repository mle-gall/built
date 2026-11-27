// SPDX-License-Identifier: GPL-3.0-or-later
//! Complete staged document reads on the owning window's idle queue.
use crate::settings_rows::{tr, trf};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{IntoPyDict, PyDict, PyList, PyTuple},
};
use std::sync::atomic::{AtomicBool, Ordering};
type Obj<'p> = Bound<'p, PyAny>;
fn callbacks<'p>(win: &Obj<'p>, key: &str) -> PyResult<Obj<'p>> {
    win.getattr("__dict__")?
        .call_method1("setdefault", (key, PyDict::new(win.py())))
}
fn activity<'p>(win: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    Ok(win
        .getattr("__dict__")?
        .cast_into::<PyDict>()?
        .get_item("_pending_open")?
        .filter(|b| !b.is_none()))
}
#[pyfunction]
fn pending(win: &Obj, path: &Obj) -> PyResult<bool> {
    let Some(bar) = activity(win)? else {
        return Ok(false);
    };
    Ok(
        (bar.getattr("path")?.eq(path)? && bar.getattr_opt("target")?.is_none_or(|t| t.is_none()))
            || bar.getattr("queue")?.contains(path)?,
    )
}
#[pyfunction]
#[pyo3(signature=(win,path,callback,failed=None))]
fn after_open(win: &Obj, path: &Obj, callback: &Obj, failed: Option<&Obj>) -> PyResult<()> {
    if pending(win, path)? {
        for (key, f) in [
            ("_open_callbacks", Some(callback)),
            ("_open_failures", failed),
        ] {
            if let Some(f) = f {
                callbacks(win, key)?
                    .call_method1("setdefault", (path, PyList::empty(win.py())))?
                    .call_method1("append", (f,))?;
            }
        }
    } else {
        callback.call1((win.getattr("_doc")?,))?;
    }
    Ok(())
}
#[pyclass]
struct StagedOpen {
    win: Py<PyAny>,
    path: Py<PyAny>,
    bar: Py<PyAny>,
    worker: Py<PyAny>,
    install: Py<PyAny>,
    target: Option<Py<PyAny>>,
    announced: AtomicBool,
    polling: AtomicBool,
}
enum Step {
    Waiting,
    Installed,
    Cancelled,
}
impl StagedOpen {
    fn step(&self, py: Python) -> PyResult<Step> {
        let win = self.win.bind(py);
        let bar = self.bar.bind(py);
        let worker = self.worker.bind(py);
        if win.getattr("_open_aborted")?.is_truthy()? {
            bar.getattr("job")?.call_method0("cancel")?;
        }
        if bar.call_method0("wasCanceled")?.is_truthy()? {
            return Ok(Step::Cancelled);
        }
        if !self.announced.load(Ordering::Relaxed) {
            let job = bar.getattr("job")?;
            job.getattr("update")?
                .call1(worker.call_method0("progress")?.cast_into::<PyTuple>()?)?;
            if worker.call_method0("ready")?.is_truthy()? {
                self.announced.store(true, Ordering::Relaxed);
                let phase = if self.target.is_some() {
                    "Adding the model to the drawing…"
                } else {
                    "Sending the model to the graphics card…"
                };
                job.call_method1("update", (0.99, tr(py, phase)?))?;
            }
            return Ok(Step::Waiting);
        }
        let loaded = worker.call_method0("take")?;
        if loaded.is_none() {
            return Ok(Step::Waiting);
        }
        if let Some(target) = &self.target {
            let target = target.bind(py);
            if !win.getattr("_documents")?.contains(target)? {
                return Err(PyValueError::new_err(tr(
                    py,
                    "The drawing was closed during import.",
                )?));
            }
            if !win.getattr("_doc")?.is(target) {
                win.call_method1("activate_document", (target,))?;
            }
        } else if !win.call_method0("_doc_is_blank")?.is_truthy()? {
            win.call_method0("new_document")?;
        }
        win.setattr("_opening_document", true)?;
        if !self
            .install
            .bind(py)
            .call1((win, self.path.bind(py), bar, loaded))?
            .is_truthy()?
        {
            return Ok(Step::Cancelled);
        }
        win.call_method0("_dismiss_home")?;
        Ok(Step::Installed)
    }
    fn advance(&self, py: Python) -> PyResult<()> {
        let win = self.win.bind(py);
        let bar = self.bar.bind(py);
        let path = self.path.bind(py);
        let outcome = self.step(py);
        if matches!(outcome, Ok(Step::Waiting)) {
            return Ok(());
        }
        if let Some(t) = bar.getattr_opt("_open_timer")? {
            t.call_method0("stop")?;
            bar.delattr("_open_timer")?;
        }
        let installed = matches!(outcome, Ok(Step::Installed));
        let reason = match outcome {
            Err(e) => {
                let dialogs = py.import("built_render.chrome_dialogs")?;
                dialogs.call_method1("close_failed", (bar, e.value(py)))?;
                let why = e.value(py).str()?.to_string();
                dialogs.getattr("notify")?.call(
                    (win, tr(py, "Open failed")?, &why),
                    Some(&[("tone", "error")].into_py_dict(py)?),
                )?;
                py.import("logging")?
                    .call_method1("error", ("Opening %s: %s", path, &why))?;
                why
            }
            Ok(Step::Cancelled) => tr(
                py,
                if bar.call_method0("wasCanceled")?.is_truthy()?
                    || win.getattr("_open_aborted")?.is_truthy()?
                {
                    "cancelled"
                } else {
                    "could not be opened"
                },
            )?,
            _ => String::new(),
        };
        if self.target.is_none() {
            let mut notify = Vec::new();
            for key in ["_open_callbacks", "_open_failures"] {
                let callbacks =
                    callbacks(win, key)?.call_method1("pop", (path, PyList::empty(py)))?;
                if installed == (key == "_open_callbacks") {
                    notify.extend(callbacks.try_iter()?.collect::<PyResult<Vec<_>>>()?);
                }
            }
            for callback in notify {
                let result = if installed {
                    callback.call1((win.getattr("_doc")?,))
                } else {
                    callback.call1((&reason,))
                };
                if let Err(e) = result {
                    e.write_unraisable(py, Some(&callback));
                }
            }
        }
        bar.call_method0("close")?;
        bar.setattr("poll", py.None())?;
        win.setattr("_pending_open", py.None())?;
        win.setattr("_open_in_progress", false)?;
        win.setattr("_opening_document", false)?;
        if win.getattr("_open_aborted")?.is_truthy()? {
            for key in ["_open_callbacks", "_open_failures"] {
                callbacks(win, key)?.call_method0("clear")?;
            }
            if win.get_type().hasattr("closeEvent")? {
                py.import("built_core.timers")?
                    .call_method1("later", (win, 0, win.getattr("close")?))?;
            }
        } else {
            for request in bar.getattr("requests")?.try_iter()? {
                let (path, target): (Obj, Obj) = request?.extract()?;
                if target.is_none() {
                    win.call_method1("open_path", (path,))?;
                } else if win.getattr("_documents")?.contains(&target)? {
                    start(
                        win,
                        &path,
                        &wrap_pyfunction!(install_insertion, py)?.into_any(),
                        Some(&target),
                    )?;
                }
            }
        }
        Ok(())
    }
}
#[pymethods]
impl StagedOpen {
    fn poll(&self, py: Python) -> PyResult<()> {
        if self.polling.swap(true, Ordering::Relaxed) {
            return Ok(());
        }
        let result = self.advance(py);
        self.polling.store(false, Ordering::Relaxed);
        result
    }
}
#[pyfunction]
#[pyo3(signature=(win,path,install,*,target=None))]
fn start(win: &Obj, path: &Obj, install: &Obj, target: Option<&Obj>) -> PyResult<bool> {
    let py = win.py();
    if let Some(bar) = activity(win)? {
        let request = (path, target).into_pyobject(py)?;
        if !request.eq((bar.getattr("path")?, bar.getattr("target")?))?
            && !bar.getattr("requests")?.contains(&request)?
        {
            bar.getattr("requests")?
                .call_method1("append", (request,))?;
            if target.is_none() {
                bar.getattr("queue")?.call_method1("append", (path,))?;
            }
        }
        return Ok(true);
    }
    let worker = py.import("built_core")?.getattr("DocumentOpen")?.call(
        (path,),
        Some(&[("insert", target.is_some())].into_py_dict(py)?),
    )?;
    let title = trf(py, "Opening {name}…", &[("name", path.getattr("name")?)])?;
    let bar = py
        .import("built_render.chrome_dialogs")?
        .getattr("ChromeBar")?
        .call(
            (win, title),
            Some(&[("exclusive", target.is_some())].into_py_dict(py)?),
        )?;
    bar.setattr("path", path)?;
    bar.setattr("target", target)?;
    bar.setattr("requests", PyList::empty(py))?;
    bar.setattr("queue", PyList::empty(py))?;
    bar.getattr("canceled")?
        .call_method1("connect", (worker.getattr("cancel")?,))?;
    bar.call_method1("set_cancellable", (true,))?;
    win.setattr("_pending_open", &bar)?;
    win.setattr("_open_in_progress", true)?;
    win.setattr("_open_aborted", false)?;
    let job = Bound::new(
        py,
        StagedOpen {
            win: win.clone().unbind(),
            path: path.clone().unbind(),
            bar: bar.clone().unbind(),
            worker: worker.unbind(),
            install: install.clone().unbind(),
            target: target.cloned().map(Bound::unbind),
            announced: false.into(),
            polling: false.into(),
        },
    )?;
    if let Some(host) = win.getattr_opt("_host")?.filter(|v| !v.is_none()) {
        bar.setattr("poll", job.getattr("poll")?)?;
        bar.setattr("next_poll", 0.)?;
        host.call_method0("request_frame")?;
    } else {
        let timer = py.import("built_core.timers")?.getattr("timer")?.call(
            (win, job.getattr("poll")?, 32),
            Some(&[("repeat", true)].into_py_dict(py)?),
        )?;
        bar.setattr("_open_timer", &timer)?;
        timer.call_method1("start", (32,))?;
    }
    Ok(true)
}
fn finish_import(win: &Obj, path: &Obj, empty: bool, result: &Obj) -> PyResult<bool> {
    win.call_method0("_refresh_scene_tabs")?;
    win.call_method0("_update_title")?;
    win.getattr("viewport")?.call_method0("update")?;
    let counts = result.getattr_opt("_skp_recovery_counts")?
        .map(|value| value.extract()).transpose()?;
    let summary = crate::file_converters::skp_summary(&path.getattr("name")?, empty, counts)?;
    win.call_method0("statusBar")?.call_method1("showMessage", summary)?;
    Ok(true)
}
#[pyfunction]
fn install_insertion(win: &Obj, path: &Obj, _bar: &Obj, command: &Obj) -> PyResult<bool> {
    let history = win.getattr("viewport")?.getattr("history")?;
    history.call_method1("execute", (command,))?;
    for name in ["last_error", "last_rejected"] {
        let message = history.getattr(name)?;
        if message.is_truthy()? {
            return Err(PyValueError::new_err(message.str()?.to_string()));
        }
    }
    win.setattr("_import_name", path.getattr("name")?)?;
    let empty = command.getattr("empty")?.is_truthy()?;
    if !empty {
        win.call_method0("_on_zoom_extents")?;
    }
    finish_import(win, path, empty, command)
}
#[pyfunction]
fn install_skp(win: &Obj, path: &Obj, bar: &Obj, staged: &Obj) -> PyResult<bool> {
    let py = win.py();
    let (scene, prepared, history, empty): (Obj, Obj, Obj, bool) = staged.extract()?;
    let vp = win.getattr("viewport")?;
    let doc = win.getattr("_doc")?;
    let host = win.getattr_opt("_host")?.filter(|h| !h.is_none());
    let offscreen = host.is_none()
        && doc
            .getattr_opt("_renderer_wgpu")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false));
    if offscreen {
        doc.setattr("_awaiting_picture", true)?;
        doc.setattr("_after_picture", PyList::empty(py))?;
    }
    history.setattr("max_steps", vp.getattr("history")?.getattr("max_steps")?)?;
    vp.call_method1("set_document", (&scene, &history))?;
    win.setattr("_import_name", path.getattr("name")?)?;
    win.setattr("_autosaved_version", py.None())?;
    win.setattr("_saved_version", -1)?;
    if !empty {
        win.call_method0("_on_zoom_extents")?;
    }
    if let Some(host) = host {
        let view = win.getattr("view")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("opening", true)?;
        kwargs.set_item("prepared", prepared)?;
        view.call_method("set_scene", (&scene,), Some(&kwargs))?;
        view.setattr("_shell_resident_version", scene.getattr("version")?)?;
        for f in view.getattr("after_open")?.try_iter()? {
            host.call_method1("after_picture", (f?,))?;
        }
        view.setattr("after_open", PyList::empty(py))?;
    } else if offscreen {
        let display = doc.getattr("stand_in")?;
        let timer = display.getattr("_refresh_timer")?;
        timer.call_method0("stop")?;
        display.call_method(
            "_rebuild",
            (),
            Some(
                &[
                    ("progress", bar.getattr("job")?.getattr("update")?),
                    ("prepared", prepared),
                ]
                .into_py_dict(py)?,
            ),
        )?;
        doc.getattr("_after_picture")?
            .call_method1("extend", (display.getattr("after_open")?,))?;
        display.setattr("after_open", PyList::empty(py))?;
        timer.call_method1("start", (0,))?;
    }
    if win
        .getattr_opt("_session_path")?
        .is_some_and(|s| s.is_truthy().unwrap_or(false))
        && !win.getattr("_opening_session")?.is_truthy()?
    {
        win.call_method0("_refresh_worksession")?;
    }
    finish_import(win, path, empty, &history)
}
pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let name = "built_render.document_open";
    let m = PyModule::new(py, name)?;
    m.add_function(wrap_pyfunction!(pending, &m)?)?;
    m.add_function(wrap_pyfunction!(after_open, &m)?)?;
    m.add_function(wrap_pyfunction!(start, &m)?)?;
    m.add_function(wrap_pyfunction!(install_skp, &m)?)?;
    m.add_function(wrap_pyfunction!(install_insertion, &m)?)?;
    parent.add("document_open", &m)?;
    py.import("sys")?.getattr("modules")?.set_item(name, m)
}

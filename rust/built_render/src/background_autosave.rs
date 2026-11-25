// SPDX-License-Identifier: GPL-3.0-or-later
//! Capture on the window's timer queue. The worker only receives a detached
//! snapshot, staging path and result queue. Publication belongs to the UI thread.
use pyo3::{
    exceptions::PyStopIteration,
    prelude::*,
    types::{PyDict, PyList},
};
use std::{fs, path::PathBuf, time::Instant};

type Obj<'p> = Bound<'p, PyAny>;

#[pyclass(dict, weakref, module = "built_render")]
struct BackgroundAutosave {
    #[pyo3(get)]
    writing: bool,
    #[pyo3(get, set, name = "SLICE_SECONDS")]
    slice_seconds: f64,
    #[pyo3(get)]
    max_slice_seconds: f64,
}

fn available(win: &Obj<'_>, doc: &Obj<'_>) -> PyResult<bool> {
    Ok(win.getattr("_documents")?.contains(doc)?
        && !doc.getattr("viewport")?.is_none()
        && !opening(win)?
        && !(doc.is(&win.getattr("_doc")?) && !win.getattr("_workspace")?.is_none()))
}
fn opening(win: &Obj<'_>) -> PyResult<bool> {
    win.getattr_opt("_opening_document")?
        .map_or(Ok(false), |v| v.is_truthy())
}

fn valid(win: &Obj<'_>, job: &Obj<'_>, capturing: bool) -> PyResult<bool> {
    let doc = job.get_item("doc")?;
    if job.contains("cancelled")? || !available(win, &doc)? {
        return Ok(false);
    }
    let scene = doc.getattr("viewport")?.getattr("scene")?;
    let store = win.py().import("built_core.autosave")?;
    if !scene.is(&job.get_item("scene")?)
        || doc.getattr("path")?.ne(job.get_item("path")?)?
        || doc.getattr("untitled_no")?.ne(job.get_item("untitled")?)?
        || store
            .call_method1("slot_epoch", (job.get_item("slot")?,))?
            .ne(job.get_item("epoch")?)?
    {
        return Ok(false);
    }
    Ok(!capturing
        || (scene
            .getattr("content_version")?
            .eq(job.get_item("version")?)?
            && scene.getattr("mesh")?.is(&job.get_item("mesh")?)
            && scene.getattr("edit_group")?.is(&job.get_item("context")?)))
}

fn next_job<'p>(manager: &Obj<'p>, win: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = win.py();
    let pending = manager.getattr("pending")?.cast_into::<PyList>()?;
    while !pending.is_empty() {
        let doc = pending.call_method1("pop", (0,))?;
        if !available(win, &doc)? || !win.call_method1("_doc_is_dirty", (&doc,))?.is_truthy()? {
            continue;
        }
        let vp = doc.getattr("viewport")?;
        let scene = vp.getattr("scene")?;
        let version = scene.getattr("content_version")?;
        if version.eq(doc.getattr("autosaved_version")?)? {
            continue;
        }
        let camera = win.call_method1("_camera_dict", (&vp,))?;
        scene.setattr("camera_home", &camera)?;
        if let Some(views) = win.getattr_opt("_views_dict")?.filter(|v| !v.is_none()) {
            scene.setattr("views_home", views.call1((&doc,))?)?;
        }
        let store = py.import("built_core.autosave")?;
        let path = doc.getattr("path")?;
        let untitled = doc.getattr("untitled_no")?;
        let slot = store.call_method1("slot_for", (&path, &untitled))?;
        let job = PyDict::new(py);
        for (k, v) in [
            ("doc", doc),
            ("version", version),
            ("path", path),
            ("untitled", untitled),
            ("mesh", scene.getattr("mesh")?),
            ("context", scene.getattr("edit_group")?),
            ("epoch", store.call_method1("slot_epoch", (&slot,))?),
            ("slot", slot),
            ("camera", camera),
        ] {
            job.set_item(k, v)?;
        }
        job.set_item("navigation_until", 0.0)?;
        job.set_item("scene", &scene)?;
        let steps = py
            .import("built_core")?
            .call_method1("snapshot_steps", (scene,))?;
        manager.setattr("steps", steps)?;
        manager.setattr("job", &job)?;
        return Ok(job.into_any());
    }
    manager.getattr("timer")?.call_method0("stop")?;
    Ok(py.None().into_bound(py))
}

#[pyfunction]
fn write_snapshot(prepared: &Obj<'_>, stage: &Obj<'_>, results: &Obj<'_>) -> PyResult<()> {
    let py = stage.py();
    let result = (|| -> PyResult<()> {
        let kw = PyDict::new(py);
        kw.set_item("background", true)?;
        py.import("built_core.formats.igz")?
            .call_method("write_prepared", (prepared, stage), Some(&kw))?;
        Ok(())
    })();
    let error = result.err().map(|e| e.value(py).to_string());
    results.call_method1("put", ((stage, error),))?;
    Ok(())
}

fn start_writer(manager: &Obj<'_>, job: &Obj<'_>, prepared: &Obj<'_>) -> PyResult<()> {
    let py = manager.py();
    let slot = job.get_item("slot")?;
    let uuid = py.import("uuid")?.call_method0("uuid4")?.getattr("hex")?;
    let name = format!(
        ".{}.{}.pending{}",
        slot.getattr("name")?,
        uuid,
        slot.getattr("suffix")?
    );
    let stage = slot.call_method1("with_name", (name,))?;
    let kw = PyDict::new(py);
    kw.set_item("target", wrap_pyfunction!(write_snapshot, py)?)?;
    kw.set_item("args", (prepared, stage, manager.getattr("results")?))?;
    kw.set_item("name", "IGZ autosave")?;
    kw.set_item("daemon", true)?;
    py.import("threading")?
        .call_method("Thread", (), Some(&kw))?
        .call_method0("start")?;
    Ok(())
}

fn report(manager: &Obj<'_>, job: &Obj<'_>, error: &str) -> PyResult<()> {
    let doc = job.get_item("doc")?;
    manager.setattr("last_error", error)?;
    manager
        .getattr("finished")?
        .call_method1("emit", (doc.call_method0("display_name")?, error))?;
    manager
        .py()
        .import("logging")?
        .call_method1("getLogger", ("built.autosave",))?
        .call_method1("warning", ("Autosave: %s", error))?;
    Ok(())
}

fn publish(
    manager: &Obj<'_>,
    win: &Obj<'_>,
    job: &Obj<'_>,
    stage: &Obj<'_>,
    error: &Obj<'_>,
) -> PyResult<()> {
    let py = manager.py();
    let doc = job.get_item("doc")?;
    let publication = (|| -> PyResult<()> {
        if !error.is_none() {
            return report(manager, job, error.extract()?);
        }
        if valid(win, job, false)? && win.call_method1("_doc_is_dirty", (&doc,))?.is_truthy()? {
            stage.call_method1("replace", (job.get_item("slot")?,))?;
            doc.setattr("autosaved_version", job.get_item("version")?)?;
            manager
                .getattr("finished")?
                .call_method1("emit", (doc.call_method0("display_name")?, ""))?;
            let msg = py
                .import("built_core.i18n")?
                .call_method1("tr", ("Auto-saved.",))?;
            win.call_method0("statusBar")?
                .call_method1("showMessage", (msg, 2000))?;
        }
        Ok(())
    })();
    let cleanup = fs::remove_file(stage.extract::<PathBuf>()?);
    manager.setattr("job", py.None())?;
    if let Err(e) = publication {
        report(manager, job, &e.value(py).to_string())?;
    }
    if let Err(e) = cleanup {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(e.into());
        }
    }
    Ok(())
}

#[pymethods]
impl BackgroundAutosave {
    #[getter]
    fn busy(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(slf.borrow().writing
            || !slf.getattr("job")?.is_none()
            || slf.getattr("pending")?.is_truthy()?)
    }

    fn request(slf: &Bound<'_, Self>, documents: &Obj<'_>) -> PyResult<()> {
        let pending = slf.getattr("pending")?.cast_into::<PyList>()?;
        let job = slf.getattr("job")?;
        for doc in documents.try_iter()? {
            let doc = doc?;
            if !pending.contains(&doc)? && (job.is_none() || !doc.is(&job.get_item("doc")?)) {
                pending.append(doc)?;
            }
        }
        if Self::busy(slf)? {
            slf.getattr("timer")?.call_method0("start")?;
        }
        Ok(())
    }

    fn cancel(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.getattr("pending")?.call_method0("clear")?;
        let job = slf.getattr("job")?;
        if !job.is_none() {
            job.set_item("cancelled", true)?;
        }
        if !slf.borrow().writing {
            slf.setattr("steps", slf.py().None())?;
            slf.setattr("job", slf.py().None())?;
            slf.getattr("timer")?.call_method0("stop")?;
        }
        Ok(())
    }

    fn advance(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let manager = slf.as_any();
        let win = slf.getattr("window")?;
        let mut job = slf.getattr("job")?;
        if slf.borrow().writing {
            let results = slf.getattr("results")?;
            if !results.call_method0("empty")?.is_truthy()? {
                let result = results.call_method0("get")?;
                slf.borrow_mut().writing = false;
                publish(
                    manager,
                    &win,
                    &job,
                    &result.get_item(0)?,
                    &result.get_item(1)?,
                )?;
            }
            return Ok(());
        }
        if opening(&win)?
            || py
                .import("built_core")?
                .getattr("window_input")?
                .call_method1("mouse_buttons", (&win,))?
                .is_truthy()?
        {
            return Ok(());
        }
        if job.is_none() {
            job = next_job(manager, &win)?;
        }
        if job.is_none() {
            return Ok(());
        }
        let doc = job.get_item("doc")?;
        if !valid(&win, &job, true)? {
            if available(&win, &doc)? && win.call_method1("_doc_is_dirty", (&doc,))?.is_truthy()? {
                slf.getattr("pending")?.call_method1("append", (doc,))?;
            }
            slf.setattr("steps", py.None())?;
            slf.setattr("job", py.None())?;
            return Ok(());
        }
        let vp = doc.getattr("viewport")?;
        let camera = win.call_method1("_camera_dict", (&vp,))?;
        let now = py
            .import("time")?
            .call_method0("monotonic")?
            .extract::<f64>()?;
        if camera.ne(job.get_item("camera")?)? {
            job.set_item("camera", camera)?;
            job.set_item("navigation_until", now + 0.25)?;
        }
        let tool = vp.getattr("active_tool")?;
        if now < job.get_item("navigation_until")?.extract()?
            || (!tool.is_none() && vp.call_method1("_tool_busy", (tool,))?.is_truthy()?)
        {
            return Ok(());
        }
        let start = Instant::now();
        let slice = slf.borrow().slice_seconds;
        let steps = slf.getattr("steps")?;
        let capture = (|| -> PyResult<()> {
            while start.elapsed().as_secs_f64() < slice {
                match steps.call_method0("__next__") {
                    Ok(_) => (),
                    Err(e) if e.is_instance_of::<PyStopIteration>(py) => {
                        let prepared = e.value(py).getattr("value")?;
                        start_writer(manager, &job, &prepared)?;
                        slf.setattr("steps", py.None())?;
                        slf.borrow_mut().writing = true;
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        })();
        if let Err(e) = capture {
            report(manager, &job, &e.value(py).to_string())?;
            slf.setattr("steps", py.None())?;
            slf.setattr("job", py.None())?;
        }
        let mut state = slf.borrow_mut();
        state.max_slice_seconds = state.max_slice_seconds.max(start.elapsed().as_secs_f64());
        Ok(())
    }
}

#[pyfunction]
pub(crate) fn background_autosave<'p>(win: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = win.py();
    let manager = Bound::new(
        py,
        BackgroundAutosave {
            writing: false,
            slice_seconds: 0.006,
            max_slice_seconds: 0.0,
        },
    )?;
    manager.setattr("_host", win.getattr_opt("_host")?)?;
    manager.setattr("window", win)?;
    manager.setattr("pending", PyList::empty(py))?;
    for key in ["job", "steps", "last_error"] {
        manager.setattr(key, py.None())?;
    }
    manager.setattr("results", py.import("queue")?.call_method0("SimpleQueue")?)?;
    manager.setattr(
        "finished",
        py.import("built_render")?
            .call_method0("ChangeCallbacks")?,
    )?;
    let timer = py
        .import("built_core.timers")?
        .call_method1("timer", (&manager, manager.getattr("advance")?, 16, true))?;
    manager.setattr("timer", timer)?;
    Ok(manager.into_any())
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Prepare an isolated document and its resident CPU arrays. No window,
//! renderer, controller or Qt event loop crosses the worker boundary.
use pyo3::{exceptions::PyInterruptedError, prelude::*, types::PyDict};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{sync_channel, Receiver},
    Arc, Mutex, Weak,
};

thread_local! {
    static STAGING: std::cell::RefCell<Option<Weak<State>>> = const { std::cell::RefCell::new(None) };
    static LAST_YIELD: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Check detached native parsers without reacquiring the interpreter.
pub(crate) fn cancelled() -> bool {
    STAGING.with_borrow(|state| {
        state
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|state| state.cancelled.load(Ordering::Acquire))
    })
}

/// Large record lists may build thousands of facade objects in one extension
/// call. Let either event loop take the interpreter between short batches.
pub(crate) use checkpoint as skp_checkpoint;
pub(crate) fn checkpoint(py: Python<'_>) -> PyResult<()> {
    STAGING.with_borrow(|state| {
        let Some(state) = state.as_ref().and_then(Weak::upgrade) else {
            return Ok(());
        };
        if state.cancelled.load(Ordering::Acquire) {
            return Err(PyInterruptedError::new_err("Opening cancelled"));
        }
        LAST_YIELD.with(|last| {
            if last
                .get()
                .is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(6))
            {
                py.detach(|| std::thread::sleep(std::time::Duration::from_micros(250)));
                last.set(Some(std::time::Instant::now()));
            }
        });
        Ok(())
    })
}

type OpenResult = PyResult<Py<PyAny>>;
type Worker = (Weak<State>, std::thread::JoinHandle<()>);
static WORKERS: Mutex<Vec<Worker>> = Mutex::new(Vec::new());

struct State {
    cancelled: AtomicBool,
    progress: Mutex<(f64, String)>,
    ready: AtomicBool,
    result: Mutex<Option<Receiver<OpenResult>>>,
}

impl State {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.result.lock().unwrap().take();
    }
}

#[pyclass]
struct Progress {
    state: Arc<State>,
    base: f64,
    span: f64,
}

#[pymethods]
impl Progress {
    fn __call__(&self, py: Python<'_>, fraction: f64, phase: String) -> PyResult<()> {
        if self.state.cancelled.load(Ordering::Acquire) {
            return Err(PyInterruptedError::new_err("Opening cancelled"));
        }
        let mut progress = self.state.progress.lock().unwrap();
        progress.0 = progress
            .0
            .max(self.base + self.span * fraction.clamp(0., 1.));
        progress.1 = phase;
        drop(progress);
        // Entity batches and phase changes share one yield budget. Publishing
        // several progress labels for a small drawing should not sleep once
        // per label. Long imports still release the interpreter between batches.
        checkpoint(py)
    }
}

/// Stage the import undo step on the worker as well as its geometry. Redo
/// restores the built objects, so the command can release all parser inputs.
fn read_skp<'py>(
    py: Python<'py>,
    path: &Bound<'py, PyAny>,
    scene: &Bound<'py, PyAny>,
    args: &Bound<'py, PyDict>,
) -> PyResult<(Bound<'py, PyAny>, bool)> {
    let reader = py.import("formats.skp")?;
    let payload = reader.getattr("parse_skp")?.call((path,), Some(args))?;
    let empty = payload.call_method1("get", ("empty", false))?.is_truthy()?;
    let history = py
        .import("core.history")?
        .getattr("History")?
        .call1((scene,))?;
    // A private document has no listeners to notify. Reserving the editor's
    // process-wide transaction writer here would reject edits in other tabs.
    let before = history.call_method0("_objects_before")?;
    let original = crate::skp::SkpOpenUndo::capture(scene)?;
    crate::skp::skp_apply(
        scene,
        payload.cast::<PyDict>()?,
        args.get_item("progress")?.as_ref(),
        None,
    )?;
    history.call_method1("_tag_new_objects", (before,))?;
    let command = Py::new(
        py,
        crate::skp::SkpOpenUndo::new(original, crate::skp::SkpOpenUndo::capture(scene)?),
    )?
    .into_bound(py);
    command.setattr("_history_mesh", scene.getattr("mesh")?)?;
    let stack = history.getattr("_native")?;
    stack.call_method1("push", (&command, history.getattr("max_steps")?))?;
    stack.call_method1("advance", ("execute", "SkpOpenUndo"))?;
    Ok((history, empty))
}

fn prepare(
    py: Python<'_>,
    path: Py<PyAny>,
    state: &Arc<State>,
    insert: bool,
) -> PyResult<Py<PyAny>> {
    let args = PyDict::new(py);
    let report = |base, span| Progress {
        state: state.clone(),
        base,
        span,
    };
    let reading = report(0., 0.60);
    reading.__call__(py, 0., "Reading the document…".into())?;
    args.set_item("progress", Py::new(py, reading)?)?;
    let scene = py.import("core.scene")?.getattr("Scene")?.call0()?;
    if insert {
        let reader = py.import("formats.skp")?;
        let payload = reader
            .call_method("parse_skp", (path,), Some(&args))?
            .cast_into::<PyDict>()?;
        let empty = payload
            .get_item("empty")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false));
        crate::skp::skp_apply(&scene, &payload, args.get_item("progress")?.as_ref(), None)?;
        return Ok(Py::new(
            py,
            crate::skp::SkpInsertion::stage(&scene, &payload, empty)?,
        )?
        .into_any());
    }
    let source = path.extract::<std::path::PathBuf>(py)?;
    let skp = if source
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("skp"))
    {
        Some(read_skp(py, path.bind(py), &scene, &args)?)
    } else {
        py.import("formats.igz")?
            .getattr("load_into")?
            .call((&scene, path), Some(&args))?;
        None
    };
    let packing = report(0.60, 0.38);
    packing.__call__(py, 0., "Preparing the view…".into())?;
    args.set_item("progress", Py::new(py, packing)?)?;
    let focus = py
        .import("views.wgpu_scene")?
        .getattr("open_focus")?
        .call1((&scene,))?;
    args.set_item("focus", focus)?;
    let packed = py
        .import("views.wgpu_scene")?
        .getattr("pack_scene")?
        .call((&scene,), Some(&args))?;
    report(0.97, 0.).__call__(py, 0., "Preparing graphics buffers…".into())?;
    packed.call_method0("prepare_upload")?;
    report(0.98, 0.).__call__(py, 0., "Indexing the model…".into())?;
    let tracker = py
        .import("views.wgpu_updates")?
        .getattr("InstanceUpdates")?
        .call1((&scene, &packed))?;
    let tree = py
        .import("views.wgpu_ui")?
        .getattr("prepare_tree_model")?
        .call1((&scene,))?;
    if let Some((history, empty)) = skp {
        return Ok((scene, (packed, tracker, tree), history, empty)
            .into_pyobject(py)?
            .into_any()
            .unbind());
    }
    Ok((scene, packed, tracker, tree)
        .into_pyobject(py)?
        .into_any()
        .unbind())
}

#[pyclass]
pub struct DocumentOpen {
    state: Arc<State>,
}

#[pymethods]
impl DocumentOpen {
    #[staticmethod]
    fn staging() -> bool {
        STAGING.with_borrow(Option::is_some)
    }

    #[new]
    #[pyo3(signature=(path, *, insert=false))]
    fn new(path: Py<PyAny>, insert: bool) -> PyResult<Self> {
        Self::spawn(move |py, worker| prepare(py, path, worker, insert))
    }

    pub(crate) fn cancel(&self) {
        self.state.cancel();
    }

    /// Called after the native event loop exits, before unloading Qt/Python.
    #[staticmethod]
    fn shutdown(py: Python<'_>) {
        let workers = std::mem::take(&mut *WORKERS.lock().unwrap());
        for (state, _) in &workers {
            if let Some(state) = state.upgrade() {
                state.cancel();
            }
        }
        py.detach(move || {
            for (_, thread) in workers {
                let _ = thread.join();
            }
        });
    }

    pub(crate) fn ready(&self) -> bool {
        self.state.ready.load(Ordering::Acquire)
    }

    pub(crate) fn progress(&self) -> (f64, String) {
        self.state.progress.lock().unwrap().clone()
    }

    /// Headless hosts have no frame pump. Keep the same worker and handoff.
    pub(crate) fn wait(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let receiver = self
            .state
            .result
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| PyInterruptedError::new_err("Opening cancelled"))?;
        py.detach(move || receiver.recv())
            .map_err(|_| PyInterruptedError::new_err("Document worker exited"))?
    }

    fn take(&self) -> PyResult<Option<Py<PyAny>>> {
        self.state
            .result
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
            .transpose()
    }
}

impl DocumentOpen {
    /// Stage an import, forwarding progress only on the caller's thread.
    /// A link already on the document worker shares its cancellation state.
    pub(crate) fn stage(
        py: Python<'_>,
        progress: Option<&Bound<'_, PyAny>>,
        job: impl for<'p> FnOnce(Python<'p>, Bound<'p, PyAny>) -> OpenResult + Send + 'static,
    ) -> OpenResult {
        if let Some(state) = STAGING.with_borrow(|s| s.as_ref().and_then(Weak::upgrade)) {
            let cb = Py::new(
                py,
                Progress {
                    state,
                    base: 0.,
                    span: 1.,
                },
            )?;
            return job(py, cb.into_bound(py).into_any());
        }
        let ready = Py::new(py, Self::parse(job)?)?;
        let run = || -> PyResult<_> {
            let modal = py.import("app_shell.modal")?;
            let wait = ready.bind(py).getattr("ready")?;
            let mut last = None;
            while !ready.borrow(py).ready() {
                if let Some(cb) = progress {
                    let next = ready.borrow(py).progress();
                    if !next.1.is_empty() && last.as_ref() != Some(&next) {
                        cb.call1(next.clone())?;
                        last = Some(next);
                    }
                }
                let finished = modal
                    .call_method1("run_until", (&wait, 0.016))?
                    .is_truthy()?;
                if !finished {
                    if progress.is_none() && modal.call_method0("host")?.is_none() {
                        break;
                    }
                    py.detach(|| std::thread::sleep(std::time::Duration::from_millis(2)));
                }
            }
            let result = ready.borrow(py).wait(py)?.into_bound(py);
            if let Some(cb) = progress {
                let next = ready.borrow(py).progress();
                if !next.1.is_empty() && last.as_ref() != Some(&next) {
                    cb.call1(next)?;
                }
            }
            Ok(result)
        };
        let result = match run() {
            Ok(r) => r,
            Err(e) => {
                ready.borrow(py).cancel();
                return Err(e);
            }
        };
        Ok(result.unbind())
    }

    pub(crate) fn parse(
        job: impl for<'py> FnOnce(Python<'py>, Bound<'py, PyAny>) -> OpenResult + Send + 'static,
    ) -> PyResult<Self> {
        Self::spawn(move |py, worker| {
            let progress = Progress {
                state: worker.clone(),
                base: 0.,
                span: 1.,
            };
            job(py, Py::new(py, progress)?.into_bound(py).into_any())
        })
    }

    fn spawn(
        job: impl FnOnce(Python<'_>, &Arc<State>) -> OpenResult + Send + 'static,
    ) -> PyResult<Self> {
        let (sender, receiver) = sync_channel(0);
        let state = Arc::new(State {
            cancelled: false.into(),
            ready: false.into(),
            progress: Mutex::default(),
            result: Mutex::new(Some(receiver)),
        });
        let worker = state.clone();
        let thread = std::thread::Builder::new()
            .name("document-open".into())
            .spawn(move || {
                // The legacy scene adapters share decoder scratch state across
                // nested links. Serialize workers without holding the GIL.
                static DECODER: Mutex<()> = Mutex::new(());
                let _guard = DECODER.lock().unwrap_or_else(|e| e.into_inner());
                // Opening is user-initiated even if another window covers this
                // one. Let the system sleep normally while preventing App Nap
                // from throttling the decoder and progress timer together.
                #[cfg(target_os = "macos")]
                let (process, activity) = {
                    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
                    let process = NSProcessInfo::processInfo();
                    let activity = process.beginActivityWithOptions_reason(
                        NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                        &NSString::from_str("Opening a document"),
                    );
                    (process, activity)
                };
                STAGING.with_borrow_mut(|active| *active = Some(Arc::downgrade(&worker)));
                let result = Python::attach(|py| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // Promote short batches while constructing the private
                        // scene. A single young collection of a whole linked
                        // file can otherwise hold the GIL for half a second.
                        let gc = py.import("gc")?;
                        let previous = gc.call_method0("get_threshold")?;
                        let budget = (4096, 4, i32::MAX);
                        gc.call_method1("set_threshold", budget)?;
                        LAST_YIELD.with(|last| last.set(Some(std::time::Instant::now())));
                        let result = job(py, &worker);
                        if gc
                            .call_method0("get_threshold")?
                            .extract::<(i32, i32, i32)>()?
                            == budget
                        {
                            gc.call_method1(
                                "set_threshold",
                                previous.cast::<pyo3::types::PyTuple>()?,
                            )?;
                        }
                        result
                    }))
                    .unwrap_or_else(|_| {
                        Err(pyo3::exceptions::PyRuntimeError::new_err(
                            "Document worker panicked",
                        ))
                    })
                });
                #[cfg(target_os = "macos")]
                // SAFETY: activity is the token issued by this process above.
                unsafe {
                    process.endActivity(&activity)
                };
                drop(_guard);
                // The unopened scene stays on this thread until a frame
                // accepts it. Cancelling a completed pack must not release
                // tens of thousands of Python objects on the window thread.
                worker.ready.store(true, Ordering::Release);
                if let Err(undelivered) = sender.send(result) {
                    Python::attach(|_| drop(undelivered));
                }
            })?;
        let mut workers = WORKERS.lock().unwrap();
        workers.retain(|(_, t)| !t.is_finished());
        workers.push((Arc::downgrade(&state), thread));
        Ok(Self { state })
    }
}

impl Drop for DocumentOpen {
    fn drop(&mut self) {
        self.state.cancel();
    }
}

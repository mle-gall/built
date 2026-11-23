// SPDX-License-Identifier: GPL-3.0-or-later
//! Prepare an isolated document and its resident CPU arrays. No window,
//! renderer, controller or Qt event loop crosses the worker boundary.
use pyo3::{exceptions::PyInterruptedError, prelude::*, types::PyDict};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{sync_channel, Receiver},
    Arc, Mutex, Weak,
};

thread_local! { static STAGING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

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
        // Small NumPy/extension calls can reacquire the GIL faster than
        // winit or Qt can take it. Leave a scheduling window between batches.
        py.detach(|| std::thread::sleep(std::time::Duration::from_millis(1)));
        Ok(())
    }
}

fn prepare(py: Python<'_>, path: Py<PyAny>, state: &Arc<State>) -> PyResult<Py<PyAny>> {
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
    py.import("formats.igz")?
        .getattr("load_into")?
        .call((&scene, path), Some(&args))?;
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
        STAGING.with(|active| active.get())
    }

    #[new]
    fn new(path: Py<PyAny>) -> PyResult<Self> {
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
                STAGING.with(|active| active.set(true));
                let result = Python::attach(|py| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // Promote short batches while constructing the private
                        // scene. A single young collection of a whole linked
                        // file can otherwise hold the GIL for half a second.
                        let gc = py.import("gc")?;
                        let previous = gc.call_method0("get_threshold")?;
                        let budget = (4096, 4, i32::MAX);
                        gc.call_method1("set_threshold", budget)?;
                        let result = prepare(py, path, &worker);
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

    fn cancel(&self) {
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

    fn ready(&self) -> bool {
        self.state.ready.load(Ordering::Acquire)
    }

    fn progress(&self) -> (f64, String) {
        self.state.progress.lock().unwrap().clone()
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

impl Drop for DocumentOpen {
    fn drop(&mut self) {
        self.state.cancel();
    }
}

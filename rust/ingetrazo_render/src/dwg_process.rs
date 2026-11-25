//! A cancellable LibreDWG child. The owning window supplies its event pump.
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[pyclass]
pub struct DwgConverter {
    tick: Option<Py<PyAny>>,
    cancelled: AtomicBool,
    running: AtomicBool,
}

fn bridge_error(py: Python<'_>, message: String, kind: &str) -> PyErr {
    match py
        .import("ingetrazo_core.formats.dwg_bridge")
        .and_then(|m| m.call_method1("DwgBridgeError", (message, kind)))
    {
        Ok(exception) => PyErr::from_value(exception),
        Err(err) => err,
    }
}

impl DwgConverter {
    fn run(
        &self,
        py: Python<'_>,
        argv: &[String],
        output: &Path,
        timeout: Duration,
    ) -> PyResult<(Option<i32>, String)> {
        let (exec, args) = argv
            .split_first()
            .ok_or_else(|| PyRuntimeError::new_err("empty converter command"))?;
        let log_path = output.with_extension("log");
        let log = File::create(&log_path)?;
        let mut cmd = Command::new(exec);
        cmd.args(args)
            .stdin(Stdio::null())
            .stderr(log.try_clone()?)
            .stdout(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let mut child = cmd.spawn().map_err(|e| {
            bridge_error(
                py,
                format!("the converter could not be started: {e}"),
                "failed",
            )
        })?;
        let started = Instant::now();
        let result = (|| loop {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(bridge_error(py, "conversion cancelled".into(), "cancelled"));
            }
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            if started.elapsed() >= timeout {
                return Err(bridge_error(
                    py,
                    format!("converter timed out after {} s", timeout.as_secs_f64()),
                    "timeout",
                ));
            }
            if let Some(tick) = &self.tick {
                tick.call0(py)?;
            }
            py.check_signals()?;
            py.detach(|| std::thread::sleep(Duration::from_millis(12)));
        })();
        let status = match result {
            Ok(s) => s,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        // The backend uses the final three lines as diagnostic text. A verbose
        // converter can fill a pipe or write megabytes before it terminates.
        let mut file = File::open(log_path)?;
        let start = file.metadata()?.len().saturating_sub(65536);
        file.seek(SeekFrom::Start(start))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        Ok((status.code(), String::from_utf8_lossy(&tail).into_owned()))
    }
}

#[pymethods]
impl DwgConverter {
    #[new]
    #[pyo3(signature=(tick=None))]
    fn new(tick: Option<Py<PyAny>>) -> Self {
        Self {
            tick,
            cancelled: AtomicBool::new(false),
            running: AtomicBool::new(false),
        }
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    fn convert<'py>(
        &self,
        py: Python<'py>,
        drawing: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let backend = py.import("ingetrazo_core.formats.dwg_bridge")?;
        let timeout = Duration::try_from_secs_f64(backend.getattr("_TIMEOUT")?.extract()?)
            .map_err(|_| PyRuntimeError::new_err("invalid DWG timeout"))?;
        if self.running.swap(true, Ordering::AcqRel) {
            return Err(PyRuntimeError::new_err("DWG conversion already running"));
        }
        self.cancelled.store(false, Ordering::Release);
        let result = (|| {
            let (tool, out): (Bound<'py, PyAny>, Bound<'py, PyAny>) =
                backend.call_method0("_new_output")?.extract()?;
            let converted = (|| {
                let argv: Vec<String> = backend
                    .call_method1("_converter_command", (tool, drawing, &out))?
                    .extract()?;
                let (code, log) = self.run(py, &argv, &out.extract::<PathBuf>()?, timeout)?;
                backend.call_method1("_check_output", (&out, code, log))?;
                backend.call_method1("_repair_dxf", (&out,)).map_err(|e| {
                    bridge_error(py, format!("cannot read the converted DXF: {e}"), "failed")
                })?;
                Ok(())
            })();
            if let Err(e) = converted {
                backend.call_method1("discard_temp_dxf", (&out,))?;
                return Err(e);
            }
            Ok(out)
        })();
        self.running.store(false, Ordering::Release);
        result
    }
}

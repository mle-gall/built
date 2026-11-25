// SPDX-License-Identifier: GPL-3.0-or-later
//! Run a step in its recorded frame, with listener and nesting semantics.
use pyo3::{
    prelude::*,
    types::{PyBool, PyDict, PyList, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;

pub(crate) fn in_frame<T>(frame: &Obj<'_>, f: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    let py = frame.py();
    frame.call_method0("__enter__")?;
    let result = f();
    match &result {
        Ok(_) => frame.call_method1("__exit__", (py.None(), py.None(), py.None()))?,
        Err(e) => frame.call_method1("__exit__", (e.get_type(py), e.value(py), e.traceback(py)))?,
    };
    result
}

pub(super) fn run<'py>(host: &Obj<'py>, cmd: &Obj<'py>, phase: &str) -> PyResult<Obj<'py>> {
    let py = host.py();
    let scene = host.getattr("scene")?;
    if let Some(flag) = scene.getattr_opt("worksession_read_only")? {
        if flag.is_truthy()? {
            host.setattr(
                "last_rejected",
                "This project file is read-only here: make it the file you edit first.",
            )?;
            return Ok(PyBool::new(py, false).to_owned().into_any());
        }
    }
    let target = match cmd.getattr_opt("_history_mesh")? {
        Some(v) if v.is_truthy()? => v,
        _ => scene.getattr("mesh")?,
    };
    if let Some(store) = target
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_store")?
    {
        let tuple = py.get_type::<PyTuple>();
        let matrix = store.getattr("frame_matrix")?;
        let matrix = if matrix.is_none() {
            matrix
        } else {
            tuple.call1((matrix,))?
        };
        let origin = tuple.call1((store.getattr("frame_origin")?,))?;
        let current = (origin, matrix).into_pyobject(py)?;
        if phase == "execute" && !store.getattr("_model_frame")?.is_truthy()? {
            cmd.setattr("_tool_frame", &current)?;
        }
        if let Some(frame) = cmd.getattr_opt("_tool_frame")? {
            if !frame.is_none() && current.ne(&frame)? {
                let kw = PyDict::new(py);
                kw.set_item("matrix", frame.get_item(1)?)?;
                let context = py.import("built_core.coordinate_frame")?.call_method(
                    "mesh_frame",
                    (target, frame.get_item(0)?),
                    Some(&kw),
                )?;
                return in_frame(&context, || dispatch(host, cmd, phase));
            }
        }
    }
    dispatch(host, cmd, phase)
}

fn notify(
    listeners: &Obj<'_>,
    index: usize,
    cmd: &Obj<'_>,
    phase: &str,
    ran: bool,
) -> PyResult<()> {
    let py = listeners.py();
    let frame = py
        .import("built_core.coordinate_frame")?
        .call_method0("model_frame")?;
    in_frame(&frame, || {
        for entry in listeners.try_iter()? {
            let callback = entry?.get_item(index)?;
            if callback.is_none() {
                continue;
            }
            if let Err(e) = callback.call1((cmd, phase, ran)) {
                if !e.is_instance_of::<pyo3::exceptions::PyException>(py) {
                    return Err(e);
                }
                let args = PyDict::new(py);
                args.set_item("exc_info", (e.get_type(py), e.value(py), e.traceback(py)))?;
                py.import("logging")?
                    .call_method1("getLogger", ("built.history",))?
                    .call_method(
                        "exception",
                        ("Command listener failed (%s)", phase),
                        Some(&args),
                    )?;
            }
        }
        Ok(())
    })
}

fn dispatch<'py>(host: &Obj<'py>, cmd: &Obj<'py>, phase: &str) -> PyResult<Obj<'py>> {
    let py = host.py();
    let nested = host.getattr("_running")?.is_truthy()?;
    if !nested {
        host.setattr("_moved_signs", PyList::empty(py))?;
    }
    let listeners = py
        .get_type::<PyTuple>()
        .call1((host.getattr("_listeners")?,))?;
    notify(&listeners, 0, cmd, phase, false)?;
    host.setattr("_running", true)?;
    let mut ran = false;
    let result = (|| {
        let value = if phase == "execute" {
            let v = super::lifecycle::execute(host, cmd, nested)?;
            ran = !host.getattr("last_error")?.is_truthy()?
                && !host.getattr("last_rejected")?.is_truthy()?;
            v
        } else {
            let v = super::lifecycle::replay(host, cmd, phase == "undo")?;
            ran = v.is_truthy()?;
            v
        };
        if ran && !nested {
            let name = match cmd.getattr_opt("history_name")? {
                Some(name) => name,
                None => cmd.get_type().getattr("__name__")?,
            };
            host.getattr("_native")?
                .call_method1("advance", (phase, name))?;
        }
        Ok(value)
    })();
    host.setattr("_running", nested)?;
    notify(&listeners, 1, cmd, phase, ran)?;
    result
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Open a document descriptor, validate its archive and select the streaming decoder.
use pyo3::{
    exceptions::{PyException, PyKeyError, PyRecursionError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyModule},
};
use std::time::Duration;
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core.formats.igz")
}
#[pyfunction]
#[pyo3(signature=(work,pump=None,*,background=false,cancel=None))]
fn igz_in_worker<'p>(
    py: Python<'p>,
    work: &Obj<'p>,
    pump: Option<&Obj<'p>>,
    background: bool,
    cancel: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    if pump.is_none() {
        if !background {
            return work.call0();
        }
        let th = py.import("threading")?;
        if !th
            .call_method0("current_thread")?
            .is(&th.call_method0("main_thread")?)
        {
            return work.call0();
        }
    }
    let ctx = py
        .import("contextvars")?
        .call_method0("copy_context")?
        .unbind();
    let job = work.clone().unbind();
    let (done, mut ready) = std::sync::mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("document-io".into())
        .spawn(move || {
            let result = Python::attach(|py| {
                ctx.bind(py)
                    .call_method1("run", (job.bind(py),))
                    .map(Bound::unbind)
            });
            let _ = done.send(());
            result
        })?;
    let poll = (|| -> PyResult<()> {
        if let Some(pump) = pump {
            loop {
                let wait = |rx: &mut std::sync::mpsc::Receiver<()>| {
                    rx.recv_timeout(Duration::from_millis(100))
                };
                let pending = py.detach(|| wait(&mut ready))
                    == Err(std::sync::mpsc::RecvTimeoutError::Timeout);
                pump.call0()?;
                if !pending {
                    break;
                }
            }
        }
        Ok(())
    })();
    let stop = if poll.is_err() {
        cancel.map(|c| c.call0()).transpose()
    } else {
        Ok(None)
    };
    // The worker may hold the last reference to a scratch range or an archive
    // member. Join even when the caller cancels or its progress hook raises.
    let result = py.detach(|| worker.join());
    stop?;
    poll?;
    result
        .map_err(|_| PyRuntimeError::new_err("document worker panicked"))?
        .map(|v| v.into_bound(py))
}
#[pyfunction]
fn igz_decode_document<'p>(py: Python<'p>, raw: &Obj<'p>) -> PyResult<Obj<'p>> {
    let m = api(py)?;
    let native = m.call_method0("native_io")?;
    if !native.is_none() && raw.is_instance_of::<PyBytes>() {
        let out = native.call_method1("igz_json_loads", (raw,))?;
        if !out.is_none() {
            return Ok(out);
        }
    }
    let kw = PyDict::new(py);
    kw.set_item("parse_constant", m.getattr("_note_constant")?)?;
    match py.import("json")?.call_method(
        "loads",
        (raw.call_method1("decode", ("utf-8",))?,),
        Some(&kw),
    ) {
        Err(e) if e.is_instance_of::<PyRecursionError>(py) => {
            let out = PyValueError::new_err("Document nesting exceeds the JSON decoder limit");
            out.set_cause(py, Some(e));
            Err(out)
        }
        other => other,
    }
}
#[pyfunction]
fn igz_parse_native<'p>(py: Python<'p>, raw: &Obj<'p>) -> PyResult<Obj<'p>> {
    let m = api(py)?;
    let none = || py.None().into_bound(py);
    if raw.len()? < m.getattr("_NATIVE_JSON_MIN")?.extract::<usize>()? {
        return Ok(none());
    }
    let native = py.import("core._native")?.getattr("native")?;
    if native.is_none() || !native.hasattr("json_mesh_blocks")? {
        return Ok(none());
    }
    let got = native.call_method1("json_mesh_blocks", (raw,))?;
    if got.is_none() {
        return Ok(none());
    }
    let flag = m.getattr("_finite_flag")?.call_method0("get")?;
    let was = flag.get_item(0)?;
    let parse = || -> PyResult<_> {
        let tree = m.call_method1("_decode_document", (got.get_item(0)?,))?;
        m.call_method1("_adopt_native_blocks", (&tree, got.get_item(1)?))?;
        Ok(tree)
    };
    match parse() {
        Ok(tree) => Ok(if flag.get_item(0)?.is_truthy()? {
            none()
        } else {
            tree
        }),
        Err(e) if e.is_instance_of::<PyException>(py) => {
            flag.set_item(0, was)?;
            Ok(none())
        }
        Err(e) => Err(e),
    }
}
#[pyclass]
struct Decode {
    read: Py<PyAny>,
}
#[pymethods]
impl Decode {
    fn __call__<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        let m = api(py)?;
        let raw = self.read.bind(py).call0()?;
        let tree = m.call_method1("_parse_native", (&raw,))?;
        if tree.is_none() {
            m.call_method1("_decode_document", (raw,))
        } else {
            Ok(tree)
        }
    }
}
#[pyfunction]
#[pyo3(signature=(read,pump=None))]
fn igz_parse_document<'p>(
    py: Python<'p>,
    read: Py<PyAny>,
    pump: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    api(py)?.call_method1("_in_worker", (Py::new(py, Decode { read })?, pump))
}
#[pyclass]
struct ReadPulse {
    reader: Py<PyAny>,
    progress: Option<Py<PyAny>>,
    pump: Option<Py<PyAny>>,
}
#[pymethods]
impl ReadPulse {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        if let Some(cb) = &self.progress {
            cb.call1(py, (self.reader.bind(py).call_method0("fraction")?,))?;
        } else if let Some(cb) = &self.pump {
            cb.call0(py)?;
        }
        Ok(())
    }
}
#[pyfunction]
#[pyo3(signature=(path,pump=None,read_progress=None,*,placements=false))]
fn igz_read_document<'p>(
    py: Python<'p>,
    path: &Obj<'p>,
    pump: Option<&Obj<'p>>,
    read_progress: Option<&Obj<'p>>,
    placements: bool,
) -> PyResult<Obj<'p>> {
    let m = api(py)?;
    let io = m.call_method0("native_io")?;
    let reader = io.getattr("DocumentReader").ok();
    let limit = m.getattr("_NATIVE_JSON_MIN")?.extract::<u64>()?;
    let streaming = placements && limit > 0;
    let decode = |fd: Obj<'p>,
                  offset: u64,
                  packed: u64,
                  size: u64,
                  method: u16,
                  crc: Option<u32>|
     -> PyResult<Obj<'p>> {
        let Some(class) = &reader else {
            return Ok(py.None().into_bound(py));
        };
        if size >= limit && !streaming {
            return Ok(py.None().into_bound(py));
        }
        let r = class.call1((size,))?;
        let pulse = if pump.is_some() || read_progress.is_some() {
            Some(Py::new(
                py,
                ReadPulse {
                    reader: r.clone().unbind(),
                    pump: pump.cloned().map(Bound::unbind),
                    progress: read_progress.cloned().map(Bound::unbind),
                },
            )?)
        } else {
            None
        };
        let run = py.import("functools")?.call_method1(
            "partial",
            (
                r.getattr("parse")?,
                fd,
                offset,
                packed,
                method,
                crc,
                placements,
            ),
        )?;
        let opts = PyDict::new(py);
        opts.set_item("background", true)?;
        opts.set_item("cancel", r.getattr("cancel").ok())?;
        m.call_method("_in_worker", (run, pulse), Some(&opts))
    };
    let stream = path.call_method1("open", ("rb",))?;
    let plain = (|| -> PyResult<_> {
        let magic = m.getattr("_ZIP_MAGIC")?;
        if stream.call_method1("read", (magic.len()?,))?.eq(magic)? {
            return Ok(None);
        }
        let fd = stream.call_method0("fileno")?;
        let size = py
            .import("os")?
            .call_method1("fstat", (&fd,))?
            .getattr("st_size")?
            .extract()?;
        let decoded = decode(fd, 0, size, size, 0, None)?;
        let tree = if decoded.is_none() {
            stream.call_method1("seek", (0,))?;
            m.call_method1("_parse_document", (stream.getattr("read")?, pump))?
        } else {
            decoded
        };
        Ok(Some((tree, py.None()).into_pyobject(py)?.into_any()))
    })();
    stream.call_method0("close")?;
    if let Some(tree) = plain? {
        return Ok(tree);
    }
    let native = io.hasattr("igz_read_member")?;
    let archive = if native {
        m.call_method1("DocumentArchive", (path,))?
    } else {
        py.import("zipfile")?.call_method1("ZipFile", (path,))?
    };
    let entry = m.getattr("_DOC_ENTRY")?;
    let tree = (|| {
        crate::archive_limits::check_archive(&archive, &path.getattr("name")?.extract::<String>()?)?;
        let info = archive.call_method1("getinfo", (&entry,))?;
        let size = info.getattr("file_size")?.extract::<u64>()?;
        if reader.is_some()
            && (streaming || size < limit)
            && info.getattr("flag_bits")?.extract::<u16>()? & 1 == 0
        {
            let member = archive.call_method1("open", (&info,))?;
            let parse = (|| -> PyResult<_> {
                let Some(offset) = member
                    .getattr("_orig_compress_start")
                    .ok()
                    .filter(|v| !v.is_none())
                else {
                    return Ok(None);
                };
                let tree = decode(
                    archive.getattr("fp")?.call_method0("fileno")?,
                    offset.extract()?,
                    info.getattr("compress_size")?.extract()?,
                    size,
                    info.getattr("compress_type")?.extract()?,
                    info.getattr("CRC")?.extract()?,
                )?;
                Ok((!tree.is_none()).then_some(tree))
            })();
            member.call_method0("close")?;
            if let Some(data) = parse? {
                return Ok(data);
            }
        }
        let partial = py.import("functools")?.getattr("partial")?;
        let read = if native {
            partial.call1((archive.getattr("read")?, &entry))?
        } else {
            partial.call1((m.getattr("_read_member_flat")?, &archive, &entry))?
        };
        m.call_method1("_parse_document", (read, pump))
    })();
    match tree {
        Ok(data) => Ok((data, archive).into_pyobject(py)?.into_any()),
        Err(e) => {
            archive.call_method0("close")?;
            if e.is_instance_of::<PyKeyError>(py) {
                Err(PyValueError::new_err(format!(
                    "{} is not an IngeTrazo document (no {entry}).",
                    path.getattr("name")?
                )))
            } else {
                Err(e)
            }
        }
    }
}
#[pyfunction]
fn igz_read_members<'p>(
    py: Python<'p>,
    archive: &Obj<'p>,
    names: &Bound<'p, PyList>,
) -> PyResult<Bound<'p, PyDict>> {
    let out = PyDict::new(py);
    if names.len() < 2 {
        for name in names.iter() {
            out.set_item(&name, archive.call_method1("read", (&name,))?)?;
        }
        return Ok(out);
    }
    let n = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .min(8);
    let pool = py
        .import("concurrent.futures")?
        .call_method1("ThreadPoolExecutor", (n,))?;
    let collect = (|| -> PyResult<()> {
        let files = pool.call_method1("map", (archive.getattr("read")?, names))?;
        for (k, v) in names.iter().zip(files.try_iter()?) {
            out.set_item(k, v?)?;
        }
        Ok(())
    })();
    pool.call_method0("shutdown")?;
    collect?;
    Ok(out)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_in_worker, m)?)?;
    m.add_function(wrap_pyfunction!(igz_decode_document, m)?)?;
    m.add_function(wrap_pyfunction!(igz_parse_native, m)?)?;
    m.add_function(wrap_pyfunction!(igz_parse_document, m)?)?;
    m.add_function(wrap_pyfunction!(igz_read_document, m)?)?;
    m.add_function(wrap_pyfunction!(igz_read_members, m)?)
}

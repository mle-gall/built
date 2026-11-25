// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded scratch-file ranges shared by the snapshot encoder and ZIP writer.
use pyo3::{
    exceptions::{PyOSError, PyTypeError, PyUnicodeEncodeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyTuple, PyType},
};
type Obj<'p> = Bound<'p, PyAny>;
const CHUNK: usize = 1 << 20;
#[pyclass(name = "Fragment")]
struct Fragment {
    #[pyo3(get)]
    file: Py<PyAny>,
    #[pyo3(get)]
    offset: u64,
    #[pyo3(get)]
    size: u64,
}
#[pymethods]
impl Fragment {
    #[new]
    fn new(file: Py<PyAny>, offset: u64, size: u64) -> Self {
        Self { file, offset, size }
    }
    fn chunks(&self, py: Python<'_>) -> Chunks {
        Chunks {
            file: self.file.clone_ref(py),
            at: self.offset,
            left: self.size,
        }
    }
}
#[pyclass]
struct Chunks {
    file: Py<PyAny>,
    at: u64,
    left: u64,
}
#[pymethods]
impl Chunks {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__<'p>(&mut self, py: Python<'p>) -> PyResult<Option<Obj<'p>>> {
        if self.left == 0 {
            return Ok(None);
        }
        let f = self.file.bind(py);
        f.call_method1("seek", (self.at,))?;
        let data = f.call_method1("read", (self.left.min(CHUNK as u64),))?;
        let n = data.len()? as u64;
        if n == 0 || n > self.left {
            return Err(PyOSError::new_err("Truncated IGZ scratch data"));
        }
        self.at += n;
        self.left -= n;
        Ok(Some(data))
    }
}
#[pyclass]
struct Append {
    file: Py<PyAny>,
}
#[pymethods]
impl Append {
    fn __call__(&self, py: Python<'_>, chunk: &Obj<'_>) -> PyResult<()> {
        let file = self.file.bind(py);
        file.call_method1("seek", (0, 2))?;
        file.call_method1("write", (chunk,))?;
        Ok(())
    }
}
fn end(file: &Obj<'_>) -> PyResult<u64> {
    file.call_method1("seek", (0, 2))?.extract()
}
fn fragment(file: &Obj<'_>, start: u64) -> PyResult<Fragment> {
    Ok(Fragment::new(
        file.clone().unbind(),
        start,
        end(file)? - start,
    ))
}
#[pyfunction]
fn igz_stream_store(stream: &Obj<'_>, chunks: &Obj<'_>) -> PyResult<Fragment> {
    let py = stream.py();
    let file = stream.getattr("file")?;
    let start = end(&file)?;
    let mut buffer = Vec::new();
    let flush = |buf: &mut Vec<u8>| -> PyResult<()> {
        if !buf.is_empty() {
            end(&file)?;
            file.call_method1("write", (PyBytes::new(py, buf),))?;
            buf.clear();
        }
        Ok(())
    };
    for chunk in chunks.try_iter()? {
        let chunk = chunk?;
        if chunk.len()? >= CHUNK {
            flush(&mut buffer)?;
            end(&file)?;
            file.call_method1("write", (chunk,))?;
        } else {
            if let Ok(bytes) = chunk.cast::<PyBytes>() {
                buffer.extend_from_slice(bytes.as_bytes());
            } else {
                let bytes = py
                    .get_type::<PyBytes>()
                    .call1((chunk,))?
                    .cast_into::<PyBytes>()?;
                buffer.extend_from_slice(bytes.as_bytes());
            }
            if buffer.len() >= CHUNK {
                flush(&mut buffer)?;
            }
        }
    }
    flush(&mut buffer)?;
    fragment(&file, start)
}
enum Token<'p> {
    Data(Obj<'p>),
    Key(Obj<'p>),
    Mark(u8),
    Leave(usize),
}
// The stdlib handles non-string object keys, integer subclasses and unpaired
// surrogates. An explicit stack also handles deeply nested fragment references.
fn compatibility(file: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    let py = file.py();
    let encoder = py.import("formats.igz_stream")?.getattr("_encoder")?;
    let mut buffer = vec![];
    let mut pending = vec![Token::Data(value.clone())];
    let mut visiting = std::collections::HashSet::new();
    let flush = |buffer: &mut Vec<u8>| -> PyResult<()> {
        end(file)?;
        file.call_method1("write", (PyBytes::new(py, buffer),))?;
        buffer.clear();
        Ok(())
    };
    while let Some(item) = pending.pop() {
        match item {
            Token::Mark(c) => buffer.push(c),
            Token::Leave(id) => {
                visiting.remove(&id);
            }
            Token::Key(k) => {
                let single = PyDict::new(py);
                single.set_item(k, py.None())?;
                let raw = encoder
                    .call_method1("encode", (single,))?
                    .extract::<String>()?;
                buffer.extend_from_slice(&raw.as_bytes()[1..raw.len() - 5]);
            }
            Token::Data(v) => {
                if let Ok(f) = v.cast::<Fragment>() {
                    flush(&mut buffer)?;
                    let mut chunks = f.borrow().chunks(py);
                    while let Some(chunk) = chunks.__next__(py)? {
                        end(file)?;
                        file.call_method1("write", (chunk,))?;
                    }
                } else if v.is_instance_of::<PyDict>()
                    || v.is_instance_of::<PyList>()
                    || v.is_instance_of::<PyTuple>()
                {
                    let id = v.as_ptr() as usize;
                    if !visiting.insert(id) {
                        return Err(PyValueError::new_err("Circular reference detected"));
                    }
                    pending.push(Token::Leave(id));
                    if let Ok(d) = v.cast::<PyDict>() {
                        buffer.push(b'{');
                        pending.push(Token::Mark(b'}'));
                        let items: Vec<_> = d.iter().collect();
                        for (i, (k, v)) in items.into_iter().enumerate().rev() {
                            pending.push(Token::Data(v));
                            pending.push(Token::Key(k));
                            if i > 0 {
                                pending.push(Token::Mark(b','));
                            }
                        }
                    } else {
                        buffer.push(b'[');
                        pending.push(Token::Mark(b']'));
                        for i in (0..v.len()?).rev() {
                            pending.push(Token::Data(v.get_item(i)?));
                            if i > 0 {
                                pending.push(Token::Mark(b','));
                            }
                        }
                    }
                } else {
                    buffer.extend_from_slice(
                        encoder
                            .call_method1("encode", (v,))?
                            .extract::<String>()?
                            .as_bytes(),
                    );
                }
            }
        }
        if buffer.len() >= CHUNK {
            flush(&mut buffer)?;
        }
    }
    if !buffer.is_empty() {
        flush(&mut buffer)?;
    }
    Ok(())
}
fn encode(file: &Obj<'_>, value: &Obj<'_>) -> PyResult<Fragment> {
    let py = file.py();
    let start = end(file)?;
    let append = Py::new(
        py,
        Append {
            file: file.clone().unbind(),
        },
    )?
    .into_bound(py)
    .into_any();
    let ty: Bound<PyType> = py.get_type::<Fragment>();
    let encoded = crate::file_json::igz_encode_json(py, value, append, ty, false);
    if let Err(e) = encoded {
        file.call_method1("truncate", (start,))?;
        if !(e.is_instance_of::<PyTypeError>(py) || e.is_instance_of::<PyUnicodeEncodeError>(py)) {
            return Err(e);
        }
        compatibility(file, value)?;
    }
    fragment(file, start)
}
#[pyfunction(signature=(stream,value,*,mesh=false))]
fn igz_stream_encode(stream: &Obj<'_>, value: &Obj<'_>, mesh: bool) -> PyResult<Fragment> {
    let _ = mesh;
    encode(&stream.getattr("file")?, value)
}
#[pyfunction(signature=(value,*,mesh=false))]
fn igz_json_chunks(value: &Obj<'_>, mesh: bool) -> PyResult<Chunks> {
    let _ = mesh;
    let py = value.py();
    let file = py.import("tempfile")?.call_method0("TemporaryFile")?;
    Ok(encode(&file, value)?.chunks(py))
}
#[pyfunction(signature=(stream,*,built=false,compatibility=false,destination=None))]
fn igz_stream_start(
    stream: &Obj<'_>,
    built: bool,
    compatibility: bool,
    destination: Option<&Obj<'_>>,
) -> PyResult<()> {
    let py = stream.py();
    stream.setattr("built", built)?;
    stream.setattr("compatibility", compatibility)?;
    stream.setattr("destination", destination)?;
    stream.setattr("displacement_mm", 0.)?;
    let tempfile = py.import("tempfile")?;
    let file = if built {
        tempfile.call_method1("SpooledTemporaryFile", (CHUNK,))?
    } else {
        tempfile.call_method0("TemporaryFile")?
    };
    stream.setattr("file", file)?;
    stream.setattr("blobs", PyDict::new(py))?;
    stream.setattr("resolved", PyDict::new(py))?;
    stream.setattr("missing", 0)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Fragment>()?;
    m.add_function(wrap_pyfunction!(igz_stream_start, m)?)?;
    m.add_function(wrap_pyfunction!(igz_stream_store, m)?)?;
    m.add_function(wrap_pyfunction!(igz_stream_encode, m)?)?;
    m.add_function(wrap_pyfunction!(igz_json_chunks, m)?)
}

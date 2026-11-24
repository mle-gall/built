// SPDX-License-Identifier: GPL-3.0-or-later
//! Little-endian MFC CArchive records share one file-wide class/object table.
use pyo3::{
    exceptions::{PyIndexError, PyValueError},
    prelude::*,
    types::{PyBytes, PyTuple},
};

#[pyclass(module = "ingetrazo_core")]
pub struct SkpArchiveCursor {
    #[pyo3(get)]
    data: Py<PyBytes>,
    #[pyo3(get, set)]
    pos: usize,
}

impl SkpArchiveCursor {
    fn bytes<'a>(&'a self, py: Python<'a>, n: usize) -> PyResult<&'a [u8]> {
        self.data
            .bind(py)
            .as_bytes()
            .get(self.pos..)
            .and_then(|s| s.get(..n))
            .ok_or_else(|| {
                PyIndexError::new_err(format!("truncated MFC archive @0x{:x}", self.pos))
            })
    }
    fn word<const N: usize>(&mut self, py: Python<'_>) -> PyResult<[u8; N]> {
        let raw = self.bytes(py, N)?.try_into().unwrap();
        self.pos += N;
        Ok(raw)
    }
    fn string_count(&mut self, py: Python<'_>) -> PyResult<usize> {
        let n = self.u8(py)?;
        if n != 255 {
            return Ok(n.into());
        }
        let n = self.u16(py)?;
        if n != 65535 {
            return Ok(n.into());
        }
        Ok(self.u32(py)? as usize)
    }
}

#[pymethods]
impl SkpArchiveCursor {
    #[new]
    #[pyo3(signature=(data,pos=0))]
    fn new(data: Py<PyBytes>, pos: usize) -> Self {
        Self { data, pos }
    }
    fn u8(&mut self, py: Python<'_>) -> PyResult<u8> {
        Ok(self.word::<1>(py)?[0])
    }
    fn u16(&mut self, py: Python<'_>) -> PyResult<u16> {
        Ok(u16::from_le_bytes(self.word(py)?))
    }
    fn u32(&mut self, py: Python<'_>) -> PyResult<u32> {
        Ok(u32::from_le_bytes(self.word(py)?))
    }
    fn f64(&mut self, py: Python<'_>) -> PyResult<f64> {
        Ok(f64::from_le_bytes(self.word(py)?))
    }
    fn f64s<'py>(&mut self, py: Python<'py>, n: usize) -> PyResult<Bound<'py, PyTuple>> {
        let len = n
            .checked_mul(8)
            .ok_or_else(|| PyValueError::new_err("invalid double count"))?;
        let result = PyTuple::new(
            py,
            self.bytes(py, len)?
                .chunks_exact(8)
                .map(|v| f64::from_le_bytes(v.try_into().unwrap())),
        )?;
        self.pos += len;
        Ok(result)
    }
    fn raw<'py>(&mut self, py: Python<'py>, n: usize) -> PyResult<Bound<'py, PyBytes>> {
        let v = PyBytes::new(py, self.bytes(py, n)?);
        self.pos += n;
        Ok(v)
    }
    fn peek<'py>(&self, py: Python<'py>, n: usize) -> Bound<'py, PyBytes> {
        let tail = self
            .data
            .bind(py)
            .as_bytes()
            .get(self.pos..)
            .unwrap_or_default();
        PyBytes::new(py, &tail[..n.min(tail.len())])
    }
    fn peek_u16(&self, py: Python<'_>) -> PyResult<u16> {
        Ok(u16::from_le_bytes(self.bytes(py, 2)?.try_into().unwrap()))
    }
    fn utf16(&mut self, py: Python<'_>) -> PyResult<String> {
        if self.word::<3>(py)? != [0xff, 0xfe, 0xff] {
            return Err(archive_error(
                py,
                format!("expected a string record @0x{:x}", self.pos - 3),
            ));
        }
        let n = self.string_count(py)?;
        let len = n
            .checked_mul(2)
            .ok_or_else(|| PyValueError::new_err("invalid string length"))?;
        let units = self
            .bytes(py, len)?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes(b.try_into().unwrap()));
        let result = char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        self.pos += len;
        Ok(result)
    }
    #[pyo3(signature=(back=16,fwd=32))]
    fn ctx(&self, py: Python<'_>, back: usize, fwd: usize) -> String {
        let bytes = self.data.bind(py).as_bytes();
        let at = self.pos.min(bytes.len());
        let hex = |s: &[u8]| {
            s.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        format!(
            "@0x{:x}: …{} | {}…",
            self.pos,
            hex(&bytes[at.saturating_sub(back)..at]),
            hex(&bytes[at..at.saturating_add(fwd).min(bytes.len())])
        )
    }
}

fn archive_error(py: Python<'_>, why: String) -> PyErr {
    match py
        .import("openskp.legacy")
        .and_then(|m| m.getattr("LegacyParseError"))
        .and_then(|c| c.call1((why,)))
    {
        Ok(v) => PyErr::from_value(v),
        Err(e) => e,
    }
}

enum Tag {
    Null,
    Declaration(String, u16),
    Class(u32),
    Object(u32),
}
fn tag(r: &mut SkpArchiveCursor, py: Python<'_>) -> PyResult<Tag> {
    Ok(match r.u16(py)? {
        0 => Tag::Null,
        0xffff => {
            let schema = r.u16(py)?;
            let n = usize::from(r.u16(py)?);
            if n > 40 {
                return Err(archive_error(
                    py,
                    format!("invalid class name length @0x{:x}", r.pos),
                ));
            }
            let name = r.bytes(py, n)?;
            if !name.is_ascii() {
                return Err(archive_error(
                    py,
                    format!("invalid class name @0x{:x}", r.pos),
                ));
            }
            let name = std::str::from_utf8(name).unwrap().to_owned();
            r.pos += n;
            Tag::Declaration(name, schema)
        }
        0x7fff => {
            let n = r.u32(py)?;
            if n & 0x80000000 != 0 {
                Tag::Class(n & 0x7fffffff)
            } else {
                Tag::Object(n)
            }
        }
        n if n & 0x8000 != 0 => Tag::Class((n & 0x7fff).into()),
        n => Tag::Object(n.into()),
    })
}

#[pyfunction]
#[pyo3(signature=(archive,r,expect=None))]
pub fn skp_mfc_object<'py>(
    py: Python<'py>,
    archive: &Bound<'py, PyAny>,
    r: &Bound<'py, SkpArchiveCursor>,
    expect: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let record = tag(&mut *r.try_borrow_mut()?, py)?;
    match record {
        Tag::Null => Ok((py.None(), py.None(), py.None())
            .into_pyobject(py)?
            .into_any()),
        Tag::Class(slot) => archive.call_method1("_new_of_class", (r, slot, expect)),
        Tag::Object(slot) => archive.call_method1("_backref", (slot, r)),
        Tag::Declaration(name, schema) => {
            let slot = archive.call_method1("alloc", (("class", &name, schema),))?;
            archive.getattr("class_slot")?.set_item(&name, slot)?;
            archive.getattr("class_schema")?.set_item(&name, schema)?;
            archive.call_method1("_new_obj", (r, name))
        }
    }
}

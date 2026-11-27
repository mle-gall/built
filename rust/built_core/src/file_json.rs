// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered document JSON. The file spelling predates the native document:
//! strings are ASCII-escaped and floats use CPython's shortest representation.
use crate::doc::value::Value;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple, PyType};
use serde::de::{Deserialize, DeserializeSeed, Deserializer, Error, MapAccess, SeqAccess, Visitor};

#[derive(Clone, Copy)]
pub enum Fields {
    Plain,
    Document,
    Extensions,
    Scene,
    Prototypes,
    Prototype,
    Placements,
    Placement,
    Matrix,
}

#[pyclass(frozen, module = "built_core")]
pub struct FileMatrix(pub [f64; 16]);

impl Fields {
    fn member(self, key: &str) -> Self {
        match (self, key) {
            (Self::Document, "ext") => Self::Extensions,
            (Self::Document, "scene") => Self::Scene,
            (Self::Scene, "protos") => Self::Prototypes,
            (Self::Scene, "groups" | "kept_components") | (Self::Placement, "children") => {
                Self::Placements
            }
            (Self::Placement, "xform") => Self::Matrix,
            _ => Self::Plain,
        }
    }
}

impl<'de> DeserializeSeed<'de> for Fields {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Fields {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a document value")
    }
    fn visit_unit<E: Error>(self) -> Result<Value, E> {
        Ok(Value::None)
    }
    fn visit_bool<E: Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_str<E: Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::Str(v.to_owned()))
    }
    fn visit_i64<E: Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Int(v))
    }
    fn visit_u64<E: Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::ElementKey(v.into()))
    }
    fn visit_f64<E: Error>(self, v: f64) -> Result<Value, E> {
        Ok(Value::Float(v))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        // Materialize long tables in batches. Holding both the entire
        // native syntax tree and the scene facade doubled peak memory.
        let mut items: Vec<Value> = Vec::new();
        let mut list: Option<Py<PyList>> = None;
        let mut append = |items: &mut Vec<Value>| {
            Python::attach(|py| -> PyResult<()> {
                let list = list.get_or_insert_with(|| PyList::empty(py).unbind());
                for item in items.drain(..) {
                    list.bind(py).append(item.to_py(py)?)?;
                }
                Ok(())
            })
            .map_err(A::Error::custom)
        };
        let kind = match self {
            Fields::Placements => Fields::Placement,
            Fields::Prototypes => Fields::Prototype,
            _ => Fields::Plain,
        };
        while let Some(v) = seq.next_element_seed(kind)? {
            items.push(v);
            if items.len() == 128 {
                append(&mut items)?;
            }
        }
        if matches!(self, Fields::Matrix) && list.is_none() && items.len() == 16 {
            let mut numbers = [0.; 16];
            for (i, item) in items.iter().enumerate() {
                numbers[i] = match item {
                    Value::Float(n) => *n,
                    Value::Int(n) => *n as f64,
                    Value::ElementKey(n) => *n as f64,
                    _ => return Ok(Value::List(items)),
                };
            }
            let matrix =
                Python::attach(|py| Py::new(py, FileMatrix(numbers))).map_err(A::Error::custom)?;
            return Ok(Value::Opaque(crate::doc::value::Opaque(
                std::sync::Arc::new(matrix.into_any()),
            )));
        }
        if let Some(list) = list {
            Python::attach(|py| -> PyResult<()> {
                for item in items {
                    list.bind(py).append(item.to_py(py)?)?;
                }
                Ok(())
            })
            .map_err(A::Error::custom)?;
            Ok(Value::Opaque(crate::doc::value::Opaque(
                std::sync::Arc::new(list.into_any()),
            )))
        } else {
            Ok(Value::List(items))
        }
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut fields = Vec::new();
        let (mut edges, mut faces) = (None, None);
        let mesh = matches!(self, Fields::Placement | Fields::Scene | Fields::Prototype);
        while let Some(key) = map.next_key::<String>()? {
            if key == "$serde_json::private::Number" {
                let token: String = map.next_value()?;
                return if token.contains(['.', 'e', 'E']) {
                    token.parse().map(Value::Float).map_err(A::Error::custom)
                } else {
                    token
                        .parse()
                        .map(Value::ElementKey)
                        .map_err(A::Error::custom)
                };
            }
            // The live reader consumes its typed rows directly. Preserve the spelling
            // for the ordinary extension path when the version or rows are unknown.
            if matches!(self, Fields::Extensions) && key == "built.live" {
                let raw = map.next_value::<Box<serde_json::value::RawValue>>()?;
                let block = Python::attach(|py| Py::new(py, crate::live::FileLive(raw)))
                    .map_err(A::Error::custom)?;
                fields.push((Value::Str(key), Value::Opaque(crate::doc::value::Opaque(
                    std::sync::Arc::new(block.into_any()),
                ))));
                continue;
            }
            if mesh && matches!(key.as_str(), "edges" | "faces") {
                let array = if key == "edges" {
                    &mut edges
                } else {
                    &mut faces
                };
                if array.is_some() {
                    return Err(A::Error::custom("duplicate mesh array"));
                }
                *array = Some(map.next_value::<Box<serde_json::value::RawValue>>()?);
                fields.push((Value::Str(key), Value::List(Vec::new())));
                continue;
            }
            if mesh && key == "_arrays" {
                return Err(A::Error::custom("reserved mesh field"));
            }
            let value = map.next_value_seed(self.member(&key))?;
            fields.push((Value::Str(key), value));
        }
        if edges.is_some() || faces.is_some() {
            let block = crate::jsondoc::mesh_arrays(
                edges.as_deref().map(|r| r.get()),
                faces.as_deref().map(|r| r.get()),
            )
            .map_err(|_| A::Error::custom("legacy mesh spelling"))?;
            if let Some(block) = block {
                let source =
                    Python::attach(|py| crate::file_geometry::source(py, block).map(Bound::unbind))
                        .map_err(A::Error::custom)?;
                fields.push((
                    Value::Str("_arrays".into()),
                    Value::Opaque(crate::doc::value::Opaque(std::sync::Arc::new(source))),
                ));
            }
        }
        Ok(Value::Dict(fields))
    }
}
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(Fields::Plain)
    }
}

#[pyfunction]
pub fn igz_json_loads<'py>(py: Python<'py>, raw: &[u8]) -> PyResult<Option<Bound<'py, PyAny>>> {
    // serde_json reserves a dollar-prefixed map key for arbitrary-precision
    // numbers. Let the legacy decoder handle such documents, including escapes.
    let parsed = py.detach(|| {
        if raw.contains(&b'$') || raw.windows(6).any(|w| w == br"\u0024") {
            return None;
        }
        serde_json::from_slice::<Value>(raw).ok()
    });
    parsed.map(|v| v.to_py(py)).transpose()
}

/// Both codecs choose the shortest round-tripping decimal with even ties.
/// CPython switches to an exponent one decade earlier and pads e-06..e-09.
fn file_float(output: &mut Vec<u8>, n: f64) {
    let mut buf = zmij::Buffer::new();
    let token = buf.format_finite(n.abs());
    if n.is_sign_negative() {
        output.push(b'-');
    }
    if let Some(digits) = token.strip_prefix("0.0000") {
        output.push(digits.as_bytes()[0]);
        if digits.len() > 1 {
            output.push(b'.');
            output.extend_from_slice(&digits.as_bytes()[1..]);
        }
        output.extend_from_slice(b"e-05");
    } else if let Some((mantissa, exponent)) = token.split_once("e-") {
        output.extend_from_slice(mantissa.as_bytes());
        output.extend_from_slice(b"e-");
        if exponent.len() == 1 {
            output.push(b'0');
        }
        output.extend_from_slice(exponent.as_bytes());
    } else {
        output.extend_from_slice(token.as_bytes());
    }
}

struct Encoder<'py> {
    py: Python<'py>,
    sink: Bound<'py, PyAny>,
    fragment: Bound<'py, PyType>,
    buffer: Vec<u8>,
    pretty: bool,
}

impl Encoder<'_> {
    fn flush(&mut self) -> PyResult<()> {
        if !self.buffer.is_empty() {
            self.sink.call1((PyBytes::new(self.py, &self.buffer),))?;
            self.buffer.clear();
        }
        Ok(())
    }

    fn string(&mut self, s: &str) {
        self.buffer.push(b'"');
        let hex = b"0123456789abcdef";
        for unit in s.encode_utf16() {
            match unit {
                8 => self.buffer.extend_from_slice(br"\b"),
                9 => self.buffer.extend_from_slice(br"\t"),
                10 => self.buffer.extend_from_slice(br"\n"),
                12 => self.buffer.extend_from_slice(br"\f"),
                13 => self.buffer.extend_from_slice(br"\r"),
                34 | 92 => self.buffer.extend_from_slice(&[b'\\', unit as u8]),
                32..=126 => self.buffer.push(unit as u8),
                _ => {
                    self.buffer.extend_from_slice(br"\u");
                    for shift in [12, 8, 4, 0] {
                        self.buffer.push(hex[((unit >> shift) & 15) as usize]);
                    }
                }
            }
        }
        self.buffer.push(b'"');
    }

    fn indent(&mut self, depth: usize) {
        if self.pretty {
            self.buffer.push(b'\n');
            self.buffer.resize(self.buffer.len() + depth * 2, b' ');
        }
    }

    fn item(&mut self, v: &Bound<'_, PyAny>, depth: usize) -> PyResult<()> {
        if self.buffer.len() >= 1 << 20 {
            self.flush()?;
        }
        if depth > 128 {
            return Err(PyTypeError::new_err(
                "use legacy encoder for deep documents",
            ));
        }
        if v.is_none() {
            self.buffer.extend_from_slice(b"null");
        } else if let Ok(b) = v.cast::<PyBool>() {
            self.buffer
                .extend_from_slice(if b.is_true() { b"true" } else { b"false" });
        } else if let Ok(s) = v.cast::<PyString>() {
            self.string(s.to_str()?);
        } else if let Ok(f) = v.cast::<PyFloat>() {
            if !f.value().is_finite() {
                return Err(PyValueError::new_err("non-finite number"));
            }
            file_float(&mut self.buffer, f.value());
        } else if v.is_exact_instance_of::<PyInt>() {
            self.buffer.extend_from_slice(v.str()?.to_str()?.as_bytes());
        } else if let Ok(d) = v.cast::<PyDict>() {
            self.buffer.push(b'{');
            for (i, (k, item)) in d.iter().enumerate() {
                if i > 0 {
                    self.buffer.push(b',');
                }
                self.indent(depth + 1);
                self.string(k.cast::<PyString>()?.to_str()?);
                self.buffer.push(b':');
                if self.pretty {
                    self.buffer.push(b' ');
                }
                self.item(&item, depth + 1)?;
            }
            if !d.is_empty() {
                self.indent(depth);
            }
            self.buffer.push(b'}');
        } else if v.is_instance_of::<PyList>() || v.is_instance_of::<PyTuple>() {
            self.buffer.push(b'[');
            for (i, item) in v.try_iter()?.enumerate() {
                if i > 0 {
                    self.buffer.push(b',');
                }
                self.indent(depth + 1);
                self.item(&item?, depth + 1)?;
            }
            if v.len()? > 0 {
                self.indent(depth);
            }
            self.buffer.push(b']');
        } else if v.is_instance(&self.fragment)? {
            for chunk in v.call_method0("chunks")?.try_iter()? {
                let chunk = chunk?;
                let bytes = chunk.cast::<PyBytes>()?.as_bytes();
                self.buffer.extend_from_slice(bytes);
                if self.buffer.len() >= 1 << 20 {
                    self.flush()?;
                }
            }
        } else {
            return Err(PyTypeError::new_err("unsupported document value"));
        }
        Ok(())
    }
}

#[pyfunction]
pub fn igz_encode_json<'py>(
    py: Python<'py>,
    value: &Bound<'py, PyAny>,
    sink: Bound<'py, PyAny>,
    fragment: Bound<'py, PyType>,
    pretty: bool,
) -> PyResult<()> {
    let mut encoder = Encoder {
        py,
        sink,
        fragment,
        buffer: Vec::new(),
        pretty,
    };
    encoder.item(value, 0)?;
    encoder.flush()
}

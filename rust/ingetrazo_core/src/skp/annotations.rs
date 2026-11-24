// SPDX-License-Identifier: GPL-3.0-or-later
use super::{field, fields, point, Point};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

enum Anchor<'a> {
    Free(Point),
    Vertex(&'a [u8], Option<&'a [u8]>),
}

fn anchor(body: &[u8]) -> Option<Anchor<'_>> {
    let sub = field(body, 0x5208)?;
    if field(sub, 0x5209) == Some(&1u32.to_le_bytes()) {
        return point(field(sub, 0x520a)?).map(Anchor::Free);
    }
    let reference = field(field(sub, 0x520b)?, 0x53fc)?;
    let vertex = field(reference, 0x53fd).filter(|v| !v.is_empty())?;
    let instance = field(reference, 0x53fe).and_then(|v| {
        let (&size, tail) = v.split_first()?;
        tail.get(..usize::from(size)).filter(|id| !id.is_empty())
    });
    Some(Anchor::Vertex(vertex, instance))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(2 * bytes.len());
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}

impl Anchor<'_> {
    fn resolve(
        &self,
        vertices: &Bound<'_, PyDict>,
        transforms: &Bound<'_, PyDict>,
    ) -> PyResult<Option<Point>> {
        let Self::Vertex(key, instance) = self else {
            return Ok(match self {
                Self::Free(p) => Some(*p),
                _ => unreachable!(),
            });
        };
        let Some(p) = vertices.get_item(hex(key))? else {
            return Ok(None);
        };
        let [x, y, z] = p.extract::<[f64; 3]>()?;
        if let Some(id) = instance {
            if let Some(m) = transforms.get_item(hex(id))? {
                if !m.is_none() {
                    let m = m.extract::<Vec<f64>>()?;
                    if m.len() >= 12 {
                        let s = m.get(12).copied().unwrap_or(1.);
                        let axis = |i: usize| {
                            m[i] * s * x + m[i + 1] * s * y + m[i + 2] * s * z + m[9 + i / 3] * s
                        };
                        return Ok(Some((axis(0), axis(3), axis(6))));
                    }
                }
            }
        }
        Ok(Some((x, y, z)))
    }
}

enum Record<'a> {
    Dimension(Anchor<'a>, Anchor<'a>, Option<u32>),
    Text(String, Option<Anchor<'a>>, Option<Point>, bool),
}

fn scan(data: &[u8], dimensions: bool) -> Vec<(usize, Record<'_>)> {
    let marker = if dimensions {
        [0xcc, 0x5b]
    } else {
        [0xf0, 0x55]
    };
    data.windows(2)
        .enumerate()
        .filter_map(|(offset, tag)| {
            if tag != marker {
                return None;
            }
            let mut stream = super::Records(&data[offset..]);
            let (_, body) = stream.next()?;
            if body.len() < 40 || fields(body).is_none() {
                return None;
            }
            let record = if dimensions {
                Record::Dimension(
                    anchor(field(body, 0x5bcd)?)?,
                    anchor(field(body, 0x5bce)?)?,
                    field(body, 0x5bd1)
                        .and_then(|v| v.try_into().ok())
                        .map(u32::from_le_bytes),
                )
            } else {
                let text = String::from_utf8_lossy(field(body, 0x55f1)?)
                    .trim()
                    .to_owned();
                if text.is_empty() {
                    return None;
                }
                let point = anchor(field(body, 0x55f4)?);
                let label = field(body, 0x55f5).and_then(super::point);
                let hidden = field(body, 0x07d0)
                    .and_then(|sub| field(sub, 0x07d3))
                    .and_then(|v| v.first())
                    .is_some_and(|f| f & 1 != 0);
                Record::Text(text, point, label, hidden)
            };
            Some((offset, record))
        })
        .collect()
}

#[pyfunction]
pub fn skp_annotations<'py>(
    py: Python<'py>,
    dimensions: bool,
    data: &[u8],
    vertices: &Bound<'py, PyDict>,
    transforms: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyList>> {
    let records = py.detach(|| scan(data, dimensions));
    let result = PyList::empty(py);
    for (offset, record) in records {
        let row = PyDict::new(py);
        row.set_item("offset", offset)?;
        match record {
            Record::Dimension(a, b, axis) => {
                let (Some(a), Some(b)) = (
                    a.resolve(vertices, transforms)?,
                    b.resolve(vertices, transforms)?,
                ) else {
                    continue;
                };
                row.set_item("a", a)?;
                row.set_item("b", b)?;
                row.set_item("axis", axis)?;
            }
            Record::Text(text, at, label, hidden) => {
                row.set_item(
                    "point",
                    at.map(|p| p.resolve(vertices, transforms))
                        .transpose()?
                        .flatten(),
                )?;
                row.set_item("label_raw", label)?;
                row.set_item("text", text)?;
                row.set_item("hidden", hidden)?;
            }
        }
        result.append(row)?;
    }
    Ok(result)
}

#[pyfunction]
pub fn skp_definition_spans(data: &[u8]) -> Vec<(usize, usize)> {
    let first = super::Records(data).next();
    let model = match first {
        Some((0x01f4, body)) if body.len() + 6 == data.len() => body,
        _ => data,
    };
    let mut sub = model;
    for tag in [0x01f9, 0x1770, 0x1771] {
        let Some((_, body)) = super::Records(sub).find(|r| r.0 == tag) else {
            return Vec::new();
        };
        sub = body;
    }
    super::Records(sub)
        .filter(|r| r.0 == 0x157c)
        .map(|(_, body)| {
            let offset = body.as_ptr() as usize - data.as_ptr() as usize;
            (offset - 6, offset + body.len())
        })
        .collect()
}

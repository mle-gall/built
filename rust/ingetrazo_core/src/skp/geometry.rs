// SPDX-License-Identifier: GPL-3.0-or-later
//! Decode a single VFF definition. Preserve unknown records as byte ranges for
//! the adapter, including complete instance subtrees and faces with UV or hole data.
use super::{point, Point, Records};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyList, PyTuple},
};
use std::collections::HashSet;

enum Geometry<'a> {
    Vertex(u64, Point, &'a [u8]),
    Edge(u64, u8, u64, u64, Option<u64>),
    Face(u64, Point, Vec<(u64, i8)>, bool, Option<u64>, Option<u64>),
}

impl<'a> Records<'a> {
    fn expect_tag(&mut self, tag: u16) -> Option<&'a [u8]> {
        let (actual, payload) = self.next()?;
        (actual == tag).then_some(payload)
    }

    fn optional(&mut self, tag: u16) -> Option<&'a [u8]> {
        let mut probe = *self;
        let value = probe.expect_tag(tag)?;
        *self = probe;
        Some(value)
    }
}

fn single(data: &[u8], tag: u16) -> Option<&[u8]> {
    let mut fields = Records(data);
    let result = fields.expect_tag(tag)?;
    fields.0.is_empty().then_some(result)
}
fn id(data: &[u8]) -> Option<u64> {
    let mut raw = [0; 8];
    if data.is_empty() {
        return None;
    }
    raw.get_mut(..data.len())?.copy_from_slice(data);
    Some(u64::from_le_bytes(raw))
}

fn geometry(tag: u16, data: &[u8]) -> Option<Geometry<'_>> {
    let mut fields = Records(data);
    if tag == 0x09c4 {
        let key = single(fields.expect_tag(0x05dc)?, 0x05de)?;
        let position = point(fields.expect_tag(0x09c5)?)?;
        return fields
            .0
            .is_empty()
            .then_some(Geometry::Vertex(id(key)?, position, key));
    }
    if !matches!(tag, 0x0bb8 | 0x0dac) {
        return None;
    }
    let mut drawing = Records(fields.expect_tag(0x07d0)?);
    let key = id(single(drawing.expect_tag(0x05dc)?, 0x05de)?)?;
    let material = match drawing.optional(0x07d1) {
        Some(v) => Some(id(v)?),
        None => None,
    };
    let &[flags] = drawing.expect_tag(0x07d3)? else {
        return None;
    };
    if !drawing.0.is_empty() {
        return None;
    }
    let result = if tag == 0x0bb8 {
        let a = id(fields.expect_tag(0x0bb9)?)?;
        let b = id(fields.expect_tag(0x0bba)?)?;
        let curve = match fields.optional(0x0bbb) {
            Some(v) => Some(id(v)?).filter(|c| *c != 0),
            None => None,
        };
        Geometry::Edge(key, flags, a, b, curve)
    } else {
        let back = match fields.optional(0x0daf) {
            Some(v) => Some(id(v)?),
            None => None,
        };
        let plane = fields.expect_tag(0x0dad)?;
        if !matches!(plane.len(), 24 | 32) {
            return None;
        }
        let normal = point(&plane[..24])?;
        let ring = single(single(fields.expect_tag(0x0dae)?, 0x1194)?, 0x1195)?;
        let mut edges = Records(ring);
        let mut coedges = Vec::new();
        while !edges.0.is_empty() {
            let mut coedge = Records(edges.expect_tag(0x0fa0)?);
            let edge = id(coedge.expect_tag(0x0fa1)?)?;
            let &[sense] = coedge.expect_tag(0x0fa2)? else {
                return None;
            };
            if !coedge.0.is_empty() {
                return None;
            }
            coedges.push((edge, if sense == 0 { 1 } else { -1 }));
        }
        if coedges.is_empty() {
            return None;
        }
        Geometry::Face(key, normal, coedges, flags & 1 != 0, material, back)
    };
    fields.0.is_empty().then_some(result)
}

enum Contents<'a> {
    Geometry(Geometry<'a>),
    Records(Vec<Node<'a>>),
}
struct Node<'a> {
    tag: u16,
    start: usize,
    len: usize,
    contents: Contents<'a>,
}

fn parse<'a>(
    data: &'a [u8],
    base: usize,
    containers: &HashSet<u16>,
    decode: bool,
    depth: u16,
) -> Result<Vec<Node<'a>>, &'static str> {
    if depth > 256 {
        return Err(".skp record nesting exceeds 256 levels");
    }
    let mut nodes = Vec::new();
    let mut start = base;
    for (tag, body) in Records(data) {
        let contents = if let Some(record) = decode.then(|| geometry(tag, body)).flatten() {
            Contents::Geometry(record)
        } else if !body.is_empty() && containers.contains(&tag) {
            Contents::Records(parse(
                body,
                start + 6,
                containers,
                decode && tag != 0x1964,
                depth + 1,
            )?)
        } else {
            Contents::Records(Vec::new())
        };
        nodes.push(Node {
            tag,
            start,
            len: body.len(),
            contents,
        });
        start += 6 + body.len();
    }
    Ok(nodes)
}

struct Adapter<'a, 'py> {
    data: &'a Bound<'py, PyBytes>,
    tags: &'a Bound<'py, PyList>,
    containers: &'a Bound<'py, PyAny>,
    node: &'a Bound<'py, PyAny>,
    decoded: &'a Bound<'py, PyAny>,
}
impl<'py> Adapter<'_, 'py> {
    fn nodes(&self, py: Python<'py>, nodes: Vec<Node<'_>>) -> PyResult<Bound<'py, PyList>> {
        let result = PyList::empty(py);
        for Node {
            tag,
            start,
            len,
            contents,
        } in nodes
        {
            let tag_name = self.tags.get_item(tag as usize)?;
            let node = match contents {
                Contents::Records(children) => {
                    let kids = self.nodes(py, children)?;
                    self.node.call1((
                        start,
                        tag_name,
                        len,
                        kids,
                        self.data,
                        start + 6,
                        start + 6 + len,
                    ))?
                }
                Contents::Geometry(record) => {
                    let tuple = match record {
                        Geometry::Vertex(id, pos, key) => {
                            let hex = key
                                .iter()
                                .map(|byte| format!("{byte:02x}"))
                                .collect::<String>();
                            (id, pos, hex).into_pyobject(py)?
                        }
                        Geometry::Edge(id, flags, a, b, curve) => {
                            (id, flags, a, b, curve).into_pyobject(py)?
                        }
                        Geometry::Face(id, normal, ring, hidden, front, back) => {
                            (id, normal, PyTuple::new(py, ring)?, hidden, front, back)
                                .into_pyobject(py)?
                        }
                    };
                    self.decoded
                        .call1((start, tag_name, len, tuple, self.data, self.containers))?
                }
            };
            result.append(node)?;
        }
        Ok(result)
    }
}

#[pyfunction]
pub fn skp_geometry_nodes<'py>(
    py: Python<'py>,
    node: &Bound<'py, PyAny>,
    decoded: &Bound<'py, PyAny>,
    tags: &Bound<'py, PyList>,
    data: &Bound<'py, PyBytes>,
    start: usize,
    end: usize,
    container_tags: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let mut containers = HashSet::new();
    for tag in container_tags.try_iter()? {
        let tag = tag?.extract::<String>()?;
        if let Ok(n) = u16::from_str_radix(&tag, 16) {
            containers.insert(n.swap_bytes());
        }
    }
    let body = data
        .as_bytes()
        .get(start..end)
        .ok_or_else(|| PyValueError::new_err("Invalid .skp record bounds"))?;
    let nodes = py
        .detach(|| parse(body, start, &containers, true, 0))
        .map_err(PyValueError::new_err)?;
    Adapter {
        data,
        tags,
        containers: container_tags,
        node,
        decoded,
    }
    .nodes(py, nodes)
}

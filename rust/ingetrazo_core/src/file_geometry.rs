// SPDX-License-Identifier: GPL-3.0-or-later
//! Store input columns of a streamed legacy JSON mesh. Edge endpoints and
//! ring corners already follow file order in the parser's position array.
use crate::doc::value::Value;
use crate::jsondoc::Block;
use numpy::{IntoPyArray, PyArrayMethods};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::collections::HashMap;

pub fn source(py: Python<'_>, block: Block) -> PyResult<Bound<'_, PyAny>> {
    let vertices = block.pos.len() / 3;
    let ne = block.flags.len();
    let boundary = 2 * ne;
    let format = py.import("formats.igz")?;
    if !block.multi.is_empty() || vertices >= i32::MAX as usize {
        // Nested holes use the topology subsystem's maximal-hole policy.
        return format.call_method1("_native_blocks", (crate::mesh_tuple(py, block)?,));
    }
    let threshold: i64 = format.getattr("_BULK_MIN")?.extract()?;
    let small = ((ne + 4 * block.counts.len()) as i64) < threshold;
    let mut names = Vec::new();
    let mut ids = HashMap::new();
    let mut remap = Vec::new();
    for raw in block.layer_names {
        let name: String =
            serde_json::from_slice(&raw).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let id = if small && name.is_empty() {
            -1
        } else {
            *ids.entry(name.clone()).or_insert_with(|| {
                let id = names.len() as i32;
                names.push(name);
                id
            })
        };
        remap.push(id);
    }
    let out = PyDict::new(py);
    let meta = PyDict::new(py);
    let attrs = PyList::empty(py);
    for raw in block.attrs {
        let value: Value =
            serde_json::from_slice(&raw).map_err(|e| PyValueError::new_err(e.to_string()))?;
        attrs.append(value.to_py(py)?)?;
    }
    meta.set_item("face_attrs", attrs)?;
    if names.is_empty() {
        out.set_item("layers", py.None())?;
    } else {
        meta.set_item("edge_layers", names)?;
        let layer: Vec<_> = block
            .layers
            .into_iter()
            .map(|i| if i < 0 { -1 } else { remap[i as usize] })
            .collect();
        out.set_item("layers", layer.into_pyarray(py))?;
    }
    out.set_item("meta", meta)?;
    let shape = [vertices, 3];
    if block
        .pos
        .iter()
        .all(|n| (*n as f32 as f64).to_bits() == n.to_bits())
    {
        let compact: Vec<_> = block.pos.into_iter().map(|n| n as f32).collect();
        out.set_item("pos", compact.into_pyarray(py).reshape(shape)?)?;
    } else {
        out.set_item("pos", block.pos.into_pyarray(py).reshape(shape)?)?;
    }
    let indices = |start: usize, end: usize| {
        (start as i32..end as i32)
            .collect::<Vec<_>>()
            .into_pyarray(py)
    };
    out.set_item("edges", indices(0, boundary).reshape([ne, 2])?)?;
    out.set_item("rings", indices(boundary, vertices))?;
    out.set_item("flags", block.flags.into_pyarray(py))?;
    if block.curves.iter().any(|i| *i >= 0) {
        out.set_item("curves", block.curves.into_pyarray(py))?;
    } else {
        out.set_item("curves", py.None())?;
    }
    let ints = |v: Vec<i64>| {
        v.into_iter()
            .map(|n| n as i32)
            .collect::<Vec<_>>()
            .into_pyarray(py)
    };
    out.set_item("sizes", ints(block.sizes))?;
    out.set_item("counts", ints(block.counts))?;
    out.set_item("face_attr", block.face_attr.into_pyarray(py))?;
    out.set_item("sequential", small)?;
    Ok(out.into_any())
}

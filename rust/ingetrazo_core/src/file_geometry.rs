// SPDX-License-Identifier: GPL-3.0-or-later
//! Store input columns of a streamed legacy JSON mesh. Edge endpoints and
//! ring corners already follow file order in the parser's position array.
use crate::doc::value::Value;
use crate::jsondoc::Block;
use numpy::{IntoPyArray, PyArrayMethods, PyUntypedArrayMethods};
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

/// Select the Built/IGZ precision and measure the largest world-space
/// displacement across every placement before a compatibility export.
#[pyfunction]
pub fn igz_points<'py>(
    py: Python<'py>,
    points: &Bound<'py, PyAny>,
    precise: bool,
    placements: Vec<(
        numpy::PyReadonlyArray2<'py, f64>,
        numpy::PyReadonlyArray2<'py, f64>,
    )>,
) -> PyResult<(Bound<'py, PyAny>, f64)> {
    let transforms = placements
        .into_iter()
        .map(|(source, target)| {
            fn affine(m: numpy::PyReadonlyArray2<'_, f64>) -> PyResult<[[f64; 4]; 3]> {
                let a = m.as_array();
                if a.shape() != [4, 4] {
                    return Err(PyValueError::new_err("invalid placement shape"));
                }
                Ok(std::array::from_fn(|r| std::array::from_fn(|c| a[[r, c]])))
            }
            let a = affine(source)?;
            let b = affine(target)?;
            let delta: [[f64; 4]; 3] =
                std::array::from_fn(|r| std::array::from_fn(|c| a[r][c] - b[r][c]));
            Ok((a, delta, a != b))
        })
        .collect::<PyResult<Vec<_>>>()?;
    let single = points.extract::<numpy::PyReadonlyArray2<'_, f32>>().ok();
    let double = if single.is_none() {
        Some(points.extract::<numpy::PyReadonlyArray2<'_, f64>>()?)
    } else {
        None
    };
    if single.as_ref().is_some_and(|a| a.shape()[1] != 3)
        || double.as_ref().is_some_and(|a| a.shape()[1] != 3)
    {
        return Err(PyValueError::new_err("expected an N by 3 position array"));
    }
    let single = single.as_ref().map(|p| p.as_slice()).transpose()?;
    let double = double.as_ref().map(|p| p.as_slice()).transpose()?;
    let len = single.map_or_else(|| double.expect("f64 points").len(), <[f32]>::len);
    let mut compact = Vec::new();
    let mut exact = true;
    let mut maximum: f64 = 0.;
    for start in (0..len).step_by(3 * 16384) {
        let end = (start + 3 * 16384).min(len);
        // Copy just this tile while Python owns the GIL. The native worker
        // can scan it without borrowing a potentially mutable NumPy buffer.
        let tile: Vec<f64> = match single {
            Some(a) => a[start..end].iter().map(|&v| v as f64).collect(),
            None => double.expect("f64 points")[start..end].to_vec(),
        };
        py.detach(|| -> PyResult<()> {
            for xyz in tile.chunks_exact(3) {
                let p = [xyz[0], xyz[1], xyz[2]];
                let q = p.map(|x| x as f32);
                if p.iter().any(|x| !x.is_finite())
                    || (!precise && q.iter().any(|x| !x.is_finite()))
                {
                    return Err(PyValueError::new_err("non-finite point"));
                }
                if single.is_none() && (!precise || exact) {
                    compact.extend(q);
                }
                let q = q.map(f64::from);
                let same = p == q;
                exact &= same;
                for (m, change, moved) in &transforms {
                    if same && !moved {
                        continue;
                    }
                    let mut sum = 0.;
                    for r in 0..3 {
                        let mut d = (p[0] - q[0]) * m[r][0]
                            + (p[1] - q[1]) * m[r][1]
                            + (p[2] - q[2]) * m[r][2];
                        if *moved {
                            d += q[0] * change[r][0]
                                + q[1] * change[r][1]
                                + q[2] * change[r][2]
                                + change[r][3];
                        }
                        sum += d * d;
                    }
                    maximum = maximum.max(sum.sqrt());
                }
            }
            Ok(())
        })?;
    }
    let out = if single.is_some() || (precise && !exact) {
        points.clone()
    } else {
        compact.into_pyarray(py).reshape([len / 3, 3])?.into_any()
    };
    Ok((out, maximum * 1000.))
}

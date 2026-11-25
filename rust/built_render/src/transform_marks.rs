// SPDX-License-Identifier: GPL-3.0-or-later
//! Grab-time selection strokes. The shader moves only grabbed endpoints.
//! The face mark prefix and selection strokes share the gesture transform.
use pyo3::prelude::*;
use std::collections::HashSet;

pub fn edge_vertices(
    selection: &Bound<'_, PyAny>,
    hover: Option<&Bound<'_, PyAny>>,
    keys: HashSet<u128>,
) -> PyResult<Vec<u8>> {
    let edge = selection.py().import("built_core.mesh")?.getattr("Edge")?;
    let mut bytes = Vec::new();
    let mut add = |e: &Bound<'_, PyAny>, color: [u8; 4]| -> PyResult<()> {
        if e.is_instance(&edge)? {
            for (point, vertex) in [("a", "v0"), ("b", "v1")] {
                let point: [f32; 3] = e.getattr(point)?.call_method0("toTuple")?.extract()?;
                let grabbed = keys.contains(&e.getattr(vertex)?.getattr("key")?.extract()?);
                let marker = [if grabbed { 1f32 } else { 0f32 }, 0., 1.];
                bytes.extend_from_slice(bytemuck::bytes_of(&point));
                bytes.extend_from_slice(bytemuck::bytes_of(&marker));
                bytes.extend_from_slice(&color);
            }
        }
        Ok(())
    };
    for e in selection.try_iter()? {
        add(&e?, [242, 114, 40, 255])?;
    }
    if let Some(e) = hover.filter(|e| !e.is_none()) {
        if e.is_instance(&edge)? && !selection.contains(e)? {
            add(e, [76, 140, 242, 255])?;
        }
    }
    Ok(bytes)
}

/// Gathered face corners are prototype-local. The marks pass consumes the
/// active camera frame, including for a georeferenced loose prototype.
#[pyfunction]
pub fn prepare_face_marks(
    mut rows: numpy::PyReadwriteArray1<'_, u8>,
    color: [f64; 3],
    prototype: [f64; 3],
    frame: [f64; 3],
) -> PyResult<()> {
    let bytes = rows.as_slice_mut()?;
    if bytes.len() % 28 != 0 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid mark vertex array",
        ));
    }
    let shift = glam::DVec3::from(prototype) - glam::DVec3::from(frame);
    let rgb = color.map(|c| (c * 255.).clamp(0., 255.) as u8);
    for v in bytes.chunks_exact_mut(28) {
        if shift != glam::DVec3::ZERO {
            let p =
                glam::Vec3::from_array(bytemuck::pod_read_unaligned(&v[..12])).as_dvec3() + shift;
            v[..12].copy_from_slice(bytemuck::bytes_of(&p.as_vec3().to_array()));
        }
        v[24..28].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Ok(())
}

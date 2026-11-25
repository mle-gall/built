// SPDX-License-Identifier: GPL-3.0-or-later
//! Pack preview surfaces and depth-tested wires into the renderer's vertex ABI.
//! Imported tool faces may supply their own triangulation.
use crate::{
    math::xyz,
    store::geometry::{self, Hypot, V3},
};
use numpy::{IntoPyArray, PyArray1};
use pyo3::{prelude::*, types::PyDict};

fn vertex(out: &mut Vec<u8>, p: [f64; 3], n: [f64; 3], color: [u8; 4]) {
    for v in p.into_iter().chain(n) {
        out.extend((v as f32).to_le_bytes())
    }
    out.extend(color)
}
fn ring(points: &Bound<'_, PyAny>, positions: &mut Vec<V3>) -> PyResult<Vec<u32>> {
    let start = positions.len();
    for p in points.try_iter()? {
        let p = xyz(&p?)?;
        positions.push(V3(p.x, p.y, p.z));
    }
    Ok((start as u32..positions.len() as u32).collect())
}
fn face_color(face: &Bound<'_, PyAny>) -> PyResult<[u8; 4]> {
    let attrs = face
        .getattr_opt("attrs")?
        .and_then(|a| a.cast_into::<PyDict>().ok());
    let attr = |key| match &attrs {
        Some(a) => a.get_item(key),
        None => Ok(None),
    };
    let rgb = if let Some(color) = attr("color")? {
        [
            color.get_item(0)?.extract()?,
            color.get_item(1)?.extract()?,
            color.get_item(2)?.extract()?,
        ]
    } else {
        [0.96, 0.95, 0.925]
    };
    let [r, g, b] = rgb.map(|v: f64| (v * 255.).clamp(0., 255.) as u8);
    let alpha = attr("opacity")?
        .map(|a| a.extract::<f64>())
        .transpose()?
        .unwrap_or(1.);
    Ok([
        r,
        g,
        b,
        (alpha * 255.).round_ties_even().clamp(1., 255.) as u8,
    ])
}
type Buffer<'py> = Bound<'py, PyArray1<u8>>;
#[pyfunction]
#[pyo3(signature=(faces,edges=None))]
pub fn pack_tool_preview<'py>(
    py: Python<'py>,
    faces: &Bound<'py, PyAny>,
    edges: Option<&Bound<'py, PyAny>>,
) -> PyResult<(Buffer<'py>, Buffer<'py>)> {
    let mut tris = Vec::new();
    let mut lines = Vec::new();
    let face_class = py.import("ingetrazo_core.geometry")?.getattr("Face")?;
    for face in faces.try_iter()? {
        let face = face?;
        let rgba = face_color(&face)?;
        if !face.get_type().is(&face_class) {
            let normal = xyz(&face.call_method0("normal")?)?.to_array();
            for tri in face.call_method0("triangulate")?.try_iter()? {
                for p in tri?.try_iter()? {
                    vertex(&mut tris, xyz(&p?)?.to_array(), normal, rgba)
                }
            }
            continue;
        }
        let mut positions = Vec::new();
        let outer = ring(&face.getattr("vertices")?, &mut positions)?;
        if outer.len() < 3 {
            continue;
        }
        let mut rings = vec![outer];
        let holes = face.getattr("holes")?;
        if !holes.is_none() {
            for hole in holes.try_iter()? {
                rings.push(ring(&hole?, &mut positions)?);
            }
        }
        // The store's f64 geometry uses the same Newell normal and earcut
        // for every host; the hypot selector only varies its f32 counterpart.
        let h = Hypot::LibStdCxx;
        let normal = geometry::face_normal(&positions, &rings[0], h);
        let rings = rings.iter().map(Vec::as_slice).collect::<Vec<_>>();
        for tri in geometry::triangles(&positions, &rings, normal, h) {
            for index in tri {
                vertex(
                    &mut tris,
                    positions[index as usize].into(),
                    normal.into(),
                    rgba,
                )
            }
        }
    }
    if let Some(edges) = edges {
        for edge in edges.try_iter()? {
            let edge = edge?;
            let rgba = if edge.len()? > 2 {
                let c: Vec<f64> = edge.get_item(2)?.extract()?;
                std::array::from_fn(|i| {
                    (c.get(i).copied().unwrap_or(1.) * 255.)
                        .clamp(0., 255.)
                        .round_ties_even() as u8
                })
            } else {
                [33, 43, 59, 255]
            };
            for i in 0..2 {
                vertex(
                    &mut lines,
                    xyz(&edge.get_item(i)?)?.to_array(),
                    [0., 0., 1.],
                    rgba,
                )
            }
        }
    }
    Ok((tris.into_pyarray(py), lines.into_pyarray(py)))
}

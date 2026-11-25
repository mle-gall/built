// SPDX-License-Identifier: GPL-3.0-or-later
//! Palette quantization and face expansion without per-row NumPy temporaries.
use numpy::{IntoPyArray, PyArray2, PyReadonlyArray1};
use pyo3::prelude::*;

#[pyfunction]
pub fn pack_materials<'py>(
    py: Python<'py>,
    colors: Vec<Option<(f64, f64, f64, u8)>>,
    textures: Vec<[u32; 2]>,
    attributes: PyReadonlyArray1<'_, i64>,
) -> PyResult<Bound<'py, PyArray2<u32>>> {
    let attrs = attributes.as_array();
    if colors.len() != textures.len() || attrs.iter().any(|&a| a < 0 || a as usize >= colors.len())
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid material palette",
        ));
    }
    let rows = py.detach(|| {
        let palette: Vec<[u32; 3]> = colors
            .iter()
            .zip(&textures)
            .map(|(color, tex)| {
                let rgba = color.map_or(0, |(r, g, b, a)| {
                    u32::from_le_bytes([
                        (r * 255.).clamp(0., 255.) as u8,
                        (g * 255.).clamp(0., 255.) as u8,
                        (b * 255.).clamp(0., 255.) as u8,
                        a,
                    ])
                });
                [rgba, tex[0], tex[1]]
            })
            .collect();
        attrs.iter().flat_map(|&a| palette[a as usize]).collect()
    });
    Ok(
        numpy::ndarray::Array2::from_shape_vec((attrs.len(), 3), rows)
            .unwrap()
            .into_pyarray(py),
    )
}

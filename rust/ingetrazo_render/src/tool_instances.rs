// SPDX-License-Identifier: GPL-3.0-or-later
//! Resident placement previews for Move, Rotate and Scale. Every frame is
//! evaluated against the original instance rows, including after reanchoring.
use glam::Mat4;
use numpy::{PyArray1, PyReadonlyArray1};
use pyo3::{exceptions::PyValueError, prelude::*};

#[pyclass(module = "ingetrazo_render")]
pub struct InstancePreview {
    rows: Vec<u8>,
}

#[pymethods]
impl InstancePreview {
    #[new]
    fn new(rows: PyReadonlyArray1<'_, u8>) -> PyResult<Self> {
        let rows = rows.as_slice()?;
        if rows.len() % 80 != 0 {
            return Err(PyValueError::new_err("expected complete instance rows"));
        }
        Ok(Self {
            rows: rows.to_vec(),
        })
    }

    fn transformed<'py>(&self, py: Python<'py>, matrix: [f32; 16]) -> Bound<'py, PyArray1<u8>> {
        let transform = Mat4::from_cols_array(&matrix);
        let mut rows = self.rows.clone();
        for row in rows.chunks_exact_mut(80) {
            let world = std::array::from_fn(|i| {
                f32::from_le_bytes(row[i * 4..i * 4 + 4].try_into().unwrap())
            });
            let world = transform * Mat4::from_cols_array(&world);
            for (bytes, value) in row[..64].chunks_exact_mut(4).zip(world.to_cols_array()) {
                bytes.copy_from_slice(&value.to_le_bytes());
            }
        }
        PyArray1::from_vec(py, rows)
    }
}

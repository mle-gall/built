// SPDX-License-Identifier: GPL-3.0-or-later
use super::{input_point, store_hypot, Point};
use crate::store::commands::cell;
use pyo3::prelude::*;

pub(crate) struct MovedVertices {
    pub(crate) moved: Vec<u128>,
    pub(crate) landed: Vec<u128>,
    pub(crate) folded_faces: Vec<u128>,
    pub(crate) defects: Vec<String>,
    pub(crate) dirty: u32,
}
pub(crate) type VertexResult = (Vec<u128>, Vec<u128>, Vec<u128>, Vec<String>, u32);

impl MovedVertices {
    pub(crate) fn fields(self) -> VertexResult {
        (
            self.moved,
            self.landed,
            self.folded_faces,
            self.defects,
            self.dirty,
        )
    }
}
fn apply(
    mesh: &Bound<'_, PyAny>,
    positions: Vec<Point>,
    delta: Option<Point>,
    matrix: [f64; 16],
    textures: bool,
) -> PyResult<Option<MovedVertices>> {
    let hypot = store_hypot(mesh.py())?;
    let mut source = Vec::with_capacity(positions.len());
    let mut landing = Vec::with_capacity(positions.len());
    for p in positions {
        let p = input_point(mesh, p)?;
        source.push(cell(p).map(f64::from_bits));
        if let Some(d) = delta {
            landing.push(cell(std::array::from_fn(|i| p[i] + d[i])).map(f64::from_bits));
        }
    }
    let owner = mesh
        .py()
        .import("built_core.store_mesh")?
        .call_method1("_attribute_owner", (mesh,))?;
    let result: Option<VertexResult> = mesh
        .getattr("_store")?
        .call_method1(
            "_apply_vertex_command",
            (source, landing, delta, matrix, textures, hypot, owner),
        )?
        .extract()?;
    let Some((moved, landed, folded_faces, defects, dirty)) = result else {
        return Ok(None);
    };
    if dirty != 0 {
        mesh.call_method1("_dirty", (dirty,))?;
    }
    Ok(Some(MovedVertices {
        moved,
        landed,
        folded_faces,
        defects,
        dirty,
    }))
}

pub(crate) fn command_move_vertices(
    mesh: &Bound<'_, PyAny>,
    positions: Vec<Point>,
    delta: Point,
    carry_textures: bool,
) -> PyResult<Option<MovedVertices>> {
    let transform = glam::DMat4::from_translation(delta.into());
    apply(
        mesh,
        positions,
        Some(delta),
        transform.to_cols_array(),
        carry_textures,
    )
}
pub(crate) fn command_transform_vertices(
    mesh: &Bound<'_, PyAny>,
    positions: Vec<Point>,
    matrix: [f64; 16],
    carry_textures: bool,
) -> PyResult<Option<MovedVertices>> {
    apply(mesh, positions, None, matrix, carry_textures)
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! The resident scene: what the document looks like packed for the GPU, and
//! how it gets there (render-glue lane; from `views/wgpu_scene.py`).
//!
//! `packed_scene` is the record and its buffers; `assemble` turns the
//! prototypes and placements the pack gathered into those buffers, through
//! the scene bake.
use pyo3::prelude::*;

pub mod assemble;
pub mod packed_scene;

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    packed_scene::register(m)?;
    assemble::register(m)
}

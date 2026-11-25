// SPDX-License-Identifier: GPL-3.0-or-later
//! The resident scene: what the document looks like packed for the GPU, and
//! how it gets there (render-glue lane; from `views/wgpu_scene.py`).
//!
//! `packed_scene` is the record and its buffers; `assemble` turns the
//! prototypes and placements the pack gathered into those buffers, through
//! the scene bake; `sources` reads a prototype's arrays and paint off its
//! mesh; `pack` walks the document's placements and puts the three together.
use pyo3::prelude::*;

pub mod assemble;
pub mod pack;
pub mod packed_scene;
pub mod sources;

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    packed_scene::register(m)?;
    pack::register(m)?;
    sources::register(m)
}

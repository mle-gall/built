// SPDX-License-Identifier: GPL-3.0-or-later
//! The resident scene: what the document looks like packed for the GPU, and
//! how it gets there (render-glue lane).
//!
//! `packed_scene` is the record and its buffers; `assemble` turns the
//! prototypes and placements the pack gathered into those buffers, through
//! the scene bake; `sources` reads a prototype's arrays and paint off its
//! mesh; `pack` walks the document's placements and puts the three together;
//! `chunk` bakes a lazy mesh's drawing chunk for the picks and the hidden lines.
use pyo3::prelude::*;

mod inference;
pub mod assemble;
mod chunk;
pub mod pack;
mod plain_edit;
pub mod packed_scene;
pub mod sources;
pub mod updates;

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    inference::register(m)?;
    chunk::register(m)?;
    packed_scene::register(m)?;
    pack::register(m)?;
    sources::register(m)?;
    updates::register(m)
}

//! The model lane's Rust (`docs/lane-ownership.md`, `docs/model-port.md`):
//! what `core/units`, `layers`, `element_set`, `camera`, `hlr` (the section
//! cut, the rest of the
//! hidden-line maths), `section`, `dimension`, `saved_views`, `texture`,
//! `group` and the tag-folder tree moved here. `lib.rs` calls
//! `register_model` once; new model modules register below, never there.

use pyo3::prelude::*;

pub mod camera_live;
pub mod dimension_frame;
pub mod group_frame;
pub mod hlr_view;
pub mod linetypes;
pub mod pyfloat;
pub mod saved_view;
pub mod section_cut;
pub mod section_fill;
pub mod selection;
pub mod tag_edits;
pub mod tag_folders;
pub mod texture_maths;
pub mod units_format;

/// Every Python function and class of the model lane.
pub fn register_model(m: &Bound<'_, PyModule>) -> PyResult<()> {
    units_format::register(m)?;
    linetypes::register(m)?;
    selection::register(m)?;
    camera_live::register(m)?;
    section_cut::register(m)?;
    section_fill::register(m)?;
    group_frame::register(m)?;
    tag_folders::register(m)?;
    dimension_frame::register(m)?;
    saved_view::register(m)?;
    hlr_view::register(m)?;
    texture_maths::register(m)?;
    tag_edits::register(m)?;
    Ok(())
}

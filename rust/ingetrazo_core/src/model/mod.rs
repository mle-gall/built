//! The model lane's Rust (`docs/lane-ownership.md`, `docs/model-port.md`):
//! what `core/units`, `layers`, `element_set`, `camera`, `hlr` (the section
//! cut, the rest of the
//! hidden-line maths), `section`, `dimension`, `saved_views`, `texture`,
//! `group`, `links` and the tag-folder tree moved here. `lib.rs` calls
//! `register_model` once; new model modules register below, never there.

use pyo3::prelude::*;

pub mod anchor_broad;
pub mod camera_live;
pub mod dimension_frame;
pub mod group_frame;
pub mod hlr_view;
pub mod linetypes;
pub mod links;
pub mod placement_walks;
pub mod pyfloat;
pub mod saved_view;
pub mod saved_view_state;
pub mod scene_bounds;
pub mod scene_mesh_states;
pub mod scene_revision;
pub mod solid_checks;
pub mod section_cut;
pub mod section_fill;
pub mod selection;
pub mod tag_edits;
pub mod tag_folders;
pub mod tag_styles;
pub mod texture_maths;
pub mod units_format;

/// A module already imported, from `sys.modules`: `py.import` runs the
/// import hooks every time (over a microsecond under PySide's), and these
/// rules run per entity (`tests/test_no_import_in_hot_calls.py`). Imported
/// the ordinary way when it is not loaded yet.
pub(crate) fn module<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    static MODULES: pyo3::sync::PyOnceLock<Py<PyAny>> = pyo3::sync::PyOnceLock::new();
    let modules = MODULES.get_or_try_init(py, || py.import("sys")?.getattr("modules").map(Bound::unbind))?.bind(py);
    match modules.call_method1("get", (name,))? {
        m if !m.is_none() => Ok(m),
        _ => Ok(py.import(name)?.into_any()),
    }
}

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
    tag_styles::register(m)?;
    anchor_broad::register(m)?;
    saved_view_state::register(m)?;
    scene_bounds::register(m)?;
    scene_mesh_states::register(m)?;
    solid_checks::register(m)?;
    placement_walks::register(m)?;
    links::register(m)?;
    scene_revision::register(m)?;
    Ok(())
}

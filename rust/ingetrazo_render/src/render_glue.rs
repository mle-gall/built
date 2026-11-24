//! The render-glue lane (docs/lane-ownership.md, docs/render-glue-port.md):
//! what replaced `views/icons.py`, `views/tree_icons.py`'s marks,
//! `views/wgpu_billboards.py`'s measuring and the per-frame document list.
//! Registered once, by `register_render_glue`.
use pyo3::prelude::*;

pub mod billboards;
pub mod document_tabs;
pub mod icon_art;

pub fn register_render_glue(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(icon_art::icon_art, m)?)?;
    m.add_function(wrap_pyfunction!(icon_art::icon_keys, m)?)?;
    m.add_function(wrap_pyfunction!(icon_art::letter_art, m)?)?;
    m.add_function(wrap_pyfunction!(icon_art::row_icon_art, m)?)?;
    m.add_class::<document_tabs::DocumentTabs>()?;
    m.add_function(wrap_pyfunction!(billboards::billboard_stances, m)?)?;
    m.add_function(wrap_pyfunction!(billboards::billboard_worlds, m)?)?;
    m.add_function(wrap_pyfunction!(billboards::sprite_mapping, m)?)?;
    Ok(())
}

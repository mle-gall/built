//! The render-glue lane (docs/lane-ownership.md, docs/render-glue-port.md):
//! what replaced `views/icons.py`, `views/tree_icons.py`'s marks,
//! `views/wgpu_billboards.py`'s measuring, the per-frame document list and
//! the tools' viewport on the resident view (`native_tools`).
//! Registered once, by `register_render_glue`.
use pyo3::prelude::*;

pub mod billboards;
pub mod cursors;
pub mod document_tabs;
pub mod headless;
pub mod svg_raster;
pub mod icon_art;
pub mod native_tools;

pub fn register_render_glue(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(icon_art::icon_art, m)?)?;
    m.add_function(wrap_pyfunction!(icon_art::icon_keys, m)?)?;
    m.add_function(wrap_pyfunction!(icon_art::letter_art, m)?)?;
    m.add_function(wrap_pyfunction!(icon_art::row_icon_art, m)?)?;
    m.add_class::<document_tabs::DocumentTabs>()?;
    m.add_function(wrap_pyfunction!(cursors::tool_cursor_raster, m)?)?;
    m.add_function(wrap_pyfunction!(headless::headless_layer, m)?)?;
    m.add_function(wrap_pyfunction!(svg_raster::svg_rgba, m)?)?;
    m.add_function(wrap_pyfunction!(billboards::billboard_stances, m)?)?;
    m.add_function(wrap_pyfunction!(billboards::billboard_worlds, m)?)?;
    m.add_function(wrap_pyfunction!(billboards::sprite_mapping, m)?)?;
    native_tools::register(m)?;
    Ok(())
}

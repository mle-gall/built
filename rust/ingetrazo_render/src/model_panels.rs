// SPDX-License-Identifier: GPL-3.0-or-later
//! Native Project, Library and model-property actions.
use pyo3::prelude::*;
pub(crate) mod basemap;
pub(crate) mod component_preview;
pub(crate) mod model_appearance;
mod plans;
pub(crate) mod shadows;

pub(super) fn register_qt_host(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(basemap::selected_tile_source, module)?)?;
    module.add_function(wrap_pyfunction!(multiline_submit_hint, module)?)?;
    module.add_function(wrap_pyfunction!(plans::plan_options, module)?)?;
    module.add_function(wrap_pyfunction!(
        component_preview::component_pixels,
        module
    )?)?;
    Ok(())
}

/// Key legend for the primary action of a form with a multiline editor.
#[pyfunction]
fn multiline_submit_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘↩"
    } else {
        "Ctrl+Enter"
    }
}

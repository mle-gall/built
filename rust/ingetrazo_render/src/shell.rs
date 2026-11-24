// SPDX-License-Identifier: GPL-3.0-or-later
//! The native shell's glue in Rust (docs/shell-port.md), registered in one
//! place, after the rest of `ingetrazo_render`: the window's commands and
//! events, its menus, sheets, forms, questions and jobs, Settings, the
//! Library and the start page. The order below is the order their
//! registrations read each other.
use pyo3::prelude::*;

/// The GPU the window would draw with, by name; an error saying why there
/// is none (or why it cannot draw Built's scene) otherwise.
#[pyfunction]
fn gpu_adapter() -> PyResult<String> {
    let instance = crate::backend::instance();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("no supported GPU backend"))?;
    if !adapter.features().contains(wgpu::Features::INDIRECT_FIRST_INSTANCE) {
        return Err(pyo3::exceptions::PyValueError::new_err("GPU adapter lacks indirect first-instance drawing"));
    }
    Ok(adapter.get_info().name)
}

pub fn register_shell(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(gpu_adapter, m)?)?;
    crate::viewport_menu::register(m)?;
    crate::shell_commands::register(m)?;
    crate::report_sheets::register(m)?;
    crate::inspector_host::register(m)?;
    crate::shell_events::register(m)?;
    crate::deliveries_host::register(m)?;
    crate::sheet_runtime::register(m)?;
    crate::dialogs_host::register(m)?;
    crate::forms_host::register(m)?;
    // Settings needs PreferencesView and RestartPreferences.
    crate::settings_host::register(m)?;
    // The Library needs library_view's functions and the sheet host.
    crate::library_host::register(m)?;
    crate::start_host::register(m)?;
    // Where this user's data, caches and documents live; their languages.
    for f in [
        wrap_pyfunction!(crate::platform_paths::preferred_languages, m)?,
        wrap_pyfunction!(crate::platform_paths::documents_directory, m)?,
        wrap_pyfunction!(crate::platform_paths::recovery_directory, m)?,
        wrap_pyfunction!(crate::platform_paths::local_data_directory, m)?,
        wrap_pyfunction!(crate::platform_paths::cache_directory, m)?,
    ] {
        m.add_function(f)?;
    }
    // The verbs read DOCUMENT_CAMERA_FIELDS, survey_note and dxf_failure.
    crate::window_commands::register(m)
}

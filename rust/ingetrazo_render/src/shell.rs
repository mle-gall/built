// SPDX-License-Identifier: GPL-3.0-or-later
//! The native shell's glue in Rust (docs/shell-port.md), registered in one
//! place, after the rest of `ingetrazo_render`: the window's commands and
//! events, its menus, sheets, forms, questions and jobs, Settings, the
//! Library and the start page. The order below is the order their
//! registrations read each other.
use pyo3::prelude::*;

pub fn register_shell(m: &Bound<'_, PyModule>) -> PyResult<()> {
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
    // The verbs read DOCUMENT_CAMERA_FIELDS, survey_note and dxf_failure.
    crate::window_commands::register(m)
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Release unused glibc heap pages after a document or snap-index job closes.
//! The process starts with two arenas, configured before dispatch in built_app.
use pyo3::prelude::*;

#[pyfunction]
pub(crate) fn trim_heap() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // Zero pad lets glibc return all eligible pages. The call ran even
        // when there were no pages available to release.
        unsafe {
            libc::malloc_trim(0);
        }
        true
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        false
    }
}

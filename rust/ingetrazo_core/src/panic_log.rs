// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! A panic leaves evidence in the application's crash log.
//!
//! PyO3 turns a panic inside a Python call into `PanicException`, but a
//! packaged app has no stderr for the default hook's message, an exception
//! out of a Qt handler only prints, and a panic on a thread of our own (a
//! rayon worker, a device callback) never reaches Python at all. The hook
//! appends the message, the thread and a backtrace to the file Python names
//! (`core.black_box`), then runs the previous hook. Each native module is
//! its own copy of `std`, so each installs its own.

use pyo3::prelude::*;
use std::sync::OnceLock;

static INSTALLED: OnceLock<String> = OnceLock::new();

/// Append panics of this module to `path` (the first path given wins).
#[pyfunction]
pub fn install_panic_log(path: String) -> bool {
    if INSTALLED.set(path).is_err() {
        return false;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        if let Some(path) = INSTALLED.get() {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let thread = std::thread::current();
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let trace = std::backtrace::Backtrace::force_capture();
                let _ = writeln!(
                    f,
                    "\nRust panic in {} (thread '{}', unix time {}): {}\n{}",
                    env!("CARGO_PKG_NAME"),
                    thread.name().unwrap_or("<unnamed>"),
                    secs,
                    info,
                    trace
                );
            }
        }
        previous(info);
    }));
    true
}

/// Panic on purpose (tests of the hook only).
#[pyfunction]
pub fn _panic_for_test(message: String) {
    panic!("{}", message);
}

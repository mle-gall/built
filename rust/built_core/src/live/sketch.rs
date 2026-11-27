// SPDX-License-Identifier: GPL-3.0-or-later
//! `sketch` rows: a 2D profile or section (general-base §2.2). A sketch
//! makes no geometry of its own; its users (sweeps, section cutters) read
//! its curves. Evaluating one checks that it is a profile: an open or
//! crossing sketch is `profile_open` and its users keep their last good
//! result.
use super::table::Row;
use built_draft2d::{profile, Sketch};

/// Users of a failed sketch are left as they are.
pub(super) const UNCHANGED: &str = "unchanged";

/// The row's body as the 2D kit reads it (the row's own keys are ignored).
pub(super) fn body(row: &Row) -> Result<Sketch, String> {
    Sketch::from_json(&serde_json::json!(row))
}

/// Check a row before it is set: the body must be a sketch the 2D kit can
/// hold (an open profile is allowed; it is a state, not an error).
pub(super) fn validate(row: &Row) -> Result<(), String> {
    if !row.owner.is_empty() {
        return Err("a sketch has no owner placement".into());
    }
    body(row).map(drop)
}

/// Why the sketch is not a profile yet, if it is not.
pub(super) fn evaluate(row: &Row) -> Option<&'static str> {
    match body(row) {
        Ok(s) if profile(&s).ok() => None,
        Ok(_) => Some("profile_open"),
        Err(_) => Some("invalid_profile"),
    }
}

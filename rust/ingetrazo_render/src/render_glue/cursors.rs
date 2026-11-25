//! Tool pointers by name, as rasters with their hotspot: what the Qt
//! cursor wrapper in `views/icons.py` built, for tests and hosts without
//! Qt. The names are `tool_cursor_name`'s ("tool:line", "tool:tape+").
use pyo3::{prelude::*, types::PyBytes};

/// `(w, h, hot_x, hot_y, rgba)` for a tool pointer at ``side`` pixels: straight
/// RGBA, the hotspot in those pixels (the artwork's 48-unit hotspot scaled and
/// rounded). None for "crosshair", the arrow (None) or an unknown name.
#[pyfunction]
#[pyo3(signature = (name, side=32))]
pub fn tool_cursor_raster<'py>(py: Python<'py>, name: Option<&str>, side: u32) -> Option<(u32, u32, u32, u32, Bound<'py, PyBytes>)> {
    let key = name?.strip_prefix("tool:")?;
    let (art, hot) = crate::tool_cursors::pixels(key, side)?;
    let k = side as f32 / 48.;
    let top = side.saturating_sub(1);
    let place = |v: f32| ((v * k).round().max(0.) as u32).min(top);
    Some((side, side, place(hot[0]), place(hot[1]), PyBytes::new(py, art.as_raw())))
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for tape_host.
use super::*;

pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("_source", none(py))?;
        attrs.set_item("work_plane", none(py))?;
        attrs.set_item("chain_first_point", none(py))?;
        attrs.set_item(
            "_MODES",
            (
                ("line", "Guides: guide lines").into_bound_py_any(py)?,
                ("point", "Guides: guide points").into_bound_py_any(py)?,
                ("measure", "Guides: off — measure only").into_bound_py_any(py)?,
            ),
        )?;
        attrs.set_item("overlay_kind", "tape")?;
        attrs.set_item("draw_overlay", module.getattr("draw_overlay")?)?;
        let cls = define(module, "TapeHost", &[module.getattr("Tool")?], &attrs)?;
        register_tool_adapter(&cls)?;
    }
    Ok(())
}

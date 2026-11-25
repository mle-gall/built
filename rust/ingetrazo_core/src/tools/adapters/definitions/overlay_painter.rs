// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for overlay_painter.
use super::*;

fn draw_overlay<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    let view = a.get_item(1)?;
    let painter = a.get_item(2)?;
    if let Some(draw) = painter.getattr_opt("draw_tool")? {
        draw.call1((tool.getattr("overlay_kind")?, tool, view))
    } else {
        imported(a.py(), "tools._classic")?.call_method1("draw_tile", (tool, view, painter))
    }
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    install_methods(
        module,
        module.dict().as_any(),
        &[(
            "draw_overlay",
            Mode::Function,
            draw_overlay,
            &["tool", "viewport", "painter"],
            "",
            "",
        )],
    )?;
    module.setattr(
        "draw_overlay",
        py.import("tools")?
            .call_method1("_plugin_method", (module.getattr("draw_overlay")?,))?,
    )?;
    Ok(())
}

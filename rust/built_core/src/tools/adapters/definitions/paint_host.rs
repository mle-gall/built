// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for paint_host.
use super::*;

pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("RECENT_MAX", 6)?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("_READ_DURING", PyTuple::empty(py).into_any())?;
        attrs.set_item("current_color", (0.8, 0.45, 0.3))?;
        attrs.set_item("current_texture", none(py))?;
        attrs.set_item("current_material", none(py))?;
        attrs.set_item("current_opacity", none(py))?;
        attrs.set_item("sample_armed", false)?;
        attrs.set_item("current_texture_plane", none(py))?;
        attrs.set_item("current_is_default", false)?;
        attrs.set_item("recent", PyList::empty(py).into_any())?;
        let builtins = py.import("builtins")?;
        let annotations = PyDict::new(py);
        annotations.set_item(
            "current_color",
            builtins.getattr("tuple")?.get_item((
                builtins.getattr("float")?,
                builtins.getattr("float")?,
                builtins.getattr("float")?,
            ))?,
        )?;
        annotations.set_item(
            "current_texture",
            builtins.getattr("dict")?.bitor(py.None())?,
        )?;
        annotations.set_item(
            "current_opacity",
            builtins.getattr("float")?.bitor(py.None())?,
        )?;
        annotations.set_item("sample_armed", builtins.getattr("bool")?)?;
        annotations.set_item(
            "current_texture_plane",
            builtins.getattr("tuple")?.bitor(py.None())?,
        )?;
        annotations.set_item("current_is_default", builtins.getattr("bool")?)?;
        annotations.set_item(
            "recent",
            builtins
                .getattr("list")?
                .get_item(builtins.getattr("dict")?)?,
        )?;
        attrs.set_item("__annotations__", annotations)?;
        let cls = define(
            module,
            "PaintHost",
            &[py.import("builtins")?.getattr("object")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    register_tool_functions(&module.dict())?;
    Ok(())
}

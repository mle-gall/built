// SPDX-License-Identifier: GPL-3.0-or-later
//! Built-in tool types retain Python subclassing and module identity.
use super::*;
mod annotation_host;
mod base;
mod eraser_host;
mod fillet_host;
mod followme_host;
mod geopath_host;
mod image_host;
mod metal_coverage;
mod move_host;
mod north_host;
mod offset_host;
mod overlay_painter;
mod paint_host;
mod place_group_host;
mod protractor_host;
mod pushpull;
mod rotate_host;
mod rust_tool_base;
mod rust_tools;
mod scale_host;
mod scale_manager_host;
mod section_host;
mod select_host;
mod solid_host;
mod tape_host;
mod texture_position;
mod walkthrough_host;
fn define<'py>(
    module: &Bound<'py, PyModule>,
    name: &str,
    bases: &[Obj<'py>],
    attrs: &Bound<'py, PyDict>,
) -> Reply<'py> {
    let py = module.py();
    let cls =
        py.get_type::<pyo3::types::PyType>()
            .call1((name, PyTuple::new(py, bases)?, attrs))?;
    module.add(name, &cls)?;
    Ok(cls)
}
fn install_methods(module: &Bound<PyModule>, cls: &Obj, methods: &[Method]) -> PyResult<()> {
    let py = module.py();
    let registrar = Registrar::new(py)?
        .with_plugin_method(imported(py, "tools")?.getattr("_plugin_method")?)
        .with_module_annotations(&module.dict())?;
    for &(name, mode, callback, args, result, doc) in methods {
        registrar.define(
            cls,
            mode,
            name,
            Endpoint::Adapter(callback),
            args,
            result,
            doc,
        )?;
    }
    if cls.is_instance_of::<pyo3::types::PyType>() {
        py.import("abc")?
            .call_method1("update_abstractmethods", (cls,))?;
    }
    Ok(())
}
pub(super) fn install(name: &str, module: &Bound<PyModule>) -> PyResult<()> {
    match name {
        "base" => base::install(module)?,
        "rust_tool_base" => rust_tool_base::install(module)?,
        "offset_host" => offset_host::install(module)?,
        "protractor_host" => protractor_host::install(module)?,
        "walkthrough_host" => walkthrough_host::install(module)?,
        "annotation_host" => annotation_host::install(module)?,
        "scale_manager_host" => scale_manager_host::install(module)?,
        "select_host" => select_host::install(module)?,
        "overlay_painter" => overlay_painter::install(module)?,
        "texture_position" => texture_position::install(module)?,
        "metal_coverage" => metal_coverage::install(module)?,
        "pushpull" => pushpull::install(module)?,
        "scale_host" => scale_host::install(module)?,
        "fillet_host" => fillet_host::install(module)?,
        "move_host" => move_host::install(module)?,
        "rotate_host" => rotate_host::install(module)?,
        "solid_host" => solid_host::install(module)?,
        "followme_host" => followme_host::install(module)?,
        "tape_host" => tape_host::install(module)?,
        "paint_host" => paint_host::install(module)?,
        "eraser_host" => eraser_host::install(module)?,
        "north_host" => north_host::install(module)?,
        "section_host" => section_host::install(module)?,
        "image_host" => image_host::install(module)?,
        "geopath_host" => geopath_host::install(module)?,
        "place_group_host" => place_group_host::install(module)?,
        "rust_tools" => rust_tools::install(module)?,
        _ => {}
    }
    Ok(())
}

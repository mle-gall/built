// SPDX-License-Identifier: GPL-3.0-or-later
//! Python format import paths for codecs implemented entirely in the native module.
use pyo3::prelude::*;

#[pyfunction]
pub(super) fn install_codec_modules(package: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = package.py();
    let native = py.import("ingetrazo_core")?;
    let modules = py.import("sys")?.getattr("modules")?;
    for (name, doc, bindings) in [
        (
            "clip_payload",
            "Model clipboard bytes shared by the native and Classic pasteboards.",
            &[("clip_decode", "decode"), ("clip_encode", "encode")][..],
        ),
        (
            "gltf",
            "Write a self-contained binary glTF 2.0 scene.",
            &[("gltf_save", "save_glb")][..],
        ),
        (
            "glb",
            "glTF 2.0 and GLB import. Geometry is staged on the document worker.",
            &[
                ("gltf_build_mesh", "build_mesh"),
                ("gltf_guess_scale", "_guess_scale"),
                ("gltf_load", "load_glb"),
                ("gltf_node_transforms", "_node_transforms"),
                ("gltf_parse_container", "parse_container"),
                ("gltf_read_triangles", "read_triangles"),
            ][..],
        ),
        (
            "dxf_in",
            "CAD drawing import and the exception presented by the import dialog.",
            &[
                ("NothingToImport", "NothingToImport"),
                ("dxf_load", "load_dxf"),
                ("dxf_open_document", "open_document"),
                ("dxf_prepare_import", "prepare_import"),
                ("dxf_read_unit_scale", "read_unit_scale"),
                ("dxf_suggest_unit_scale", "suggest_unit_scale"),
                ("RECENTER_BEYOND_M", "RECENTER_BEYOND_M"),
                ("survey_offset", "survey_offset"),
            ][..],
        ),
        (
            "stl",
            "STL scripting and worker API.",
            &[
                ("STL_UNITS", "STL_UNITS"),
                ("add_stl_mesh", "add_stl_mesh"),
                ("iter_triangles", "iter_triangles"),
                ("load_stl", "load_stl"),
                ("parse_stl", "parse_stl"),
                ("save_stl", "save_stl"),
            ][..],
        ),
    ]
    .into_iter()
    .chain(
        super::bindings::CODECS
            .iter()
            .map(|&(name, bindings)| (name, "", bindings)),
    ) {
        let full = format!("formats.{name}");
        if let Some(existing) = modules
            .call_method1("get", (&full,))?
            .extract::<Option<Bound<'_, PyModule>>>()?
        {
            package.add(name, existing)?;
            continue;
        }
        let module = PyModule::new(py, &full)?;
        module.setattr("__doc__", doc)?;
        module.setattr("__package__", "formats")?;
        let spec = py
            .import("importlib.machinery")?
            .call_method1("ModuleSpec", (&full, py.None()))?;
        module.setattr("__spec__", spec)?;
        for &(source, alias) in bindings {
            module.add(alias, native.getattr(source)?)?;
        }
        super::module_state::configure(name, &module, &native, package)?;
        modules.set_item(format!("ingetrazo_core.{full}"), &module)?;
        modules.set_item(full, &module)?;
        package.add(name, module)?;
    }
    Ok(())
}

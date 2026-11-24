// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for rotate_host.
use super::*;
fn rotate_host_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let tool = a.get_item(0)?;
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("RotateHost")?, &tool))?
        .call_method0("__init__")?;
    tool.setattr("_groups", PyList::empty(py).into_any())?;
    tool.setattr("_splanes", PyList::empty(py).into_any())?;
    tool.setattr("_images", PyList::empty(py).into_any())?;
    tool.setattr("_moved_guides", PyList::empty(py).into_any())?;
    tool.setattr("_positions", PyList::empty(py).into_any())?;
    tool.setattr("_verts", PyList::empty(py).into_any())?;
    tool.setattr("_sel_faces", PyList::empty(py).into_any())?;
    tool.setattr("_sel_edges", PyList::empty(py).into_any())?;
    tool.setattr("_base_segments", module.getattr("CopyWire")?.call0()?)?;
    tool.setattr("_gesture", module.getattr("MatrixGesture")?.call0()?)?;
    Ok(none(py))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("accepts_angle_ratio", true)?;
        attrs.set_item("accepts_array", true)?;
        let cls = define(
            module,
            "RotateHost",
            &[
                module.getattr("CopyPreview")?,
                module.getattr("ProtractorBase")?,
            ],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[(
                "__init__",
                Mode::Method,
                rotate_host_init,
                &["self"],
                "None",
                "",
            )],
        )?;
    }
    Ok(())
}

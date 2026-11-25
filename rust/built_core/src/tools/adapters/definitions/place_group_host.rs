// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for place_group_host.
use super::*;
fn place_group_host_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.place_group_host")?;
    let align_to_face = a.get_item(2)?;
    let anchor = a.get_item(3)?;
    let group = a.get_item(1)?;
    let tool = a.get_item(0)?;
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("PlaceGroupHost")?, &tool))?
        .call_method0("__init__")?;
    tool.setattr("_group", &group)?;
    let placement_template = py.import("built_core")?.getattr("placement_template")?;
    {
        let [part0, part1] =
            unpack::<2>(&placement_template.call1((&group, &align_to_face, &anchor))?)?;
        tool.setattr("_anchor", part0)?;
        tool.setattr("_segments", part1)?;
    }
    tool.setattr("_align", &align_to_face)?;
    tool.setattr("_attachment", none(py))?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "load",
            (
                group.getattr("uid")?,
                tool.getattr("_anchor")?.call_method0("toTuple")?,
                tool.getattr("_align")?,
                tool.getattr("_segments")?,
            )
                .into_bound_py_any(py)?,
        ),
    )?;
    Ok(none(py))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("name", "Place component")?;
        attrs.set_item("uses_snap", true)?;
        attrs.set_item("wireframe_color", (0.13, 0.17, 0.23, 1.0))?;
        attrs.set_item("wireframe_depth_tested", false)?;
        let cls = define(
            module,
            "PlaceGroupHost",
            &[py.import("builtins")?.getattr("object")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[(
                "__init__",
                Mode::Method,
                place_group_host_init,
                &[
                    "self",
                    "group",
                    "align_to_face:bool=False",
                    "anchor:QVector3D | None=None",
                ],
                "None",
                "",
            )],
        )?;
    }
    Ok(())
}

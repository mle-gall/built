// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for rust_tools.
use super::*;
fn rectangle_flag<'py>(a: &Args<'py>) -> Reply<'py> {
    imported(a.py(), "tools.offset_host")?.call_method1("_field", (a.get_item(0)?,))
}
fn planar_machine_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    module
        .getattr("RustTool")?
        .call_method1("__init__", (&tool,))?;
    tool.setattr("_viewport", none(py))?;
    Ok(none(py))
}
fn last_fillet_last_fillet_d<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let arc_last_fillet_d = py.import("built_core")?.getattr("arc_last_fillet_d")?;
    arc_last_fillet_d.call0()
}
fn last_fillet_set_last_fillet_d<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let value = a.get_item(1)?;
    let arc_last_fillet_d = py.import("built_core")?.getattr("arc_last_fillet_d")?;
    arc_last_fillet_d.call1((
        (if value.is_none() {
            none(py)
        } else {
            py.get_type::<pyo3::types::PyFloat>()
                .into_any()
                .call1((&value,))?
        }),
        true,
    ))?;
    Ok(none(py))
}
fn rotate_tool_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    module
        .getattr("RustTool")?
        .call_method1("__init__", (&tool,))?;
    module
        .getattr("RotateHost")?
        .call_method1("__init__", (&tool,))?;
    Ok(none(py))
}
fn rotate_tool_start_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("start_point",))
}
fn rotate_tool_set_start_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let p = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_set_arm", ("start_point", &p))
}
fn rotate_tool_ref_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("ref_point",))
}
fn rotate_tool_set_ref_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let p = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_set_arm", ("ref_point", &p))
}
fn rotate_tool_copy<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.call_method1("_state", ("copy",))?.is_truthy()?).into_bound_py_any(py)
}
fn rotate_tool_set_copy<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let on = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("copy", (on.is_truthy()?).into_bound_py_any(py)?))
}
fn rotate_tool_last<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.call_method1("_state", ("retype",))?.is_truthy()? {
        true.into_bound_py_any(py)?
    } else {
        none(py)
    })
}
fn protractor_tool_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    module
        .getattr("RustTool")?
        .call_method1("__init__", (&tool,))?;
    module
        .getattr("ProtractorBase")?
        .call_method1("__init__", (&tool,))?;
    Ok(none(py))
}
fn protractor_tool_start_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("start_point",))
}
fn protractor_tool_set_start_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let p = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_set_arm", ("start_point", &p))
}
fn protractor_tool_ref_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("ref_point",))
}
fn protractor_tool_set_ref_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let p = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_set_arm", ("ref_point", &p))
}
fn protractor_tool_guides<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("guides",))
}
fn protractor_tool_set_guides<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let g = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("guides", (g.is_truthy()?).into_bound_py_any(py)?))
}
fn protractor_tool_last<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.call_method1("_state", ("retype",))?.is_truthy()? {
        true.into_bound_py_any(py)?
    } else {
        none(py)
    })
}
fn fillet_tool_sizing<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("sizing",))
}
fn fillet_tool_set_sizing<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(1)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        ("sizing", (value.is_truthy()?).into_bound_py_any(py)?),
    )
}
fn fillet_tool_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    module
        .getattr("RustTool")?
        .call_method1("__init__", (&tool,))?;
    module
        .getattr("FilletHost")?
        .call_method1("__init__", (&tool,))?;
    Ok(none(py))
}
fn tape_measure_tool_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("mode",))
}
fn tape_measure_tool_set_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    let value = a.get_item(1)?;
    tool.getattr("_machine")?
        .call_method1("set", ("mode", &value))
}
fn tape_measure_tool_measured<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("measured",))
}
fn flip_tool_lock_axis<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("lock_axis",))
}
fn flip_tool_set_lock_axis<'py>(a: &Args<'py>) -> Reply<'py> {
    let axis = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("lock_axis", &axis))
}
fn flip_tool_hover_axis<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("hover_axis",))
}
fn flip_tool_set_hover_axis<'py>(a: &Args<'py>) -> Reply<'py> {
    let axis = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("hover_axis", &axis))
}
fn flip_tool_copy<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("copy",))
}
fn flip_tool_set_copy<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(1)?;
    tool.getattr("_machine")?
        .call_method1("set", ("copy", (value.is_truthy()?).into_bound_py_any(py)?))
}
fn freehand_tool_start_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("start_point",))
}
fn freehand_tool_stroke<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("stroke",))
}
fn place_group_tool_offset<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("offset",))
}
fn sweep_key(code:i32)->Option<&'static str>{
    match code{0x1000001=>Some("Tab"),82=>Some("R"),70=>Some("F"),76=>Some("L"),0x1000004|0x1000005=>Some("Enter"),_=>None}
}
fn sweep_claims<'p>(a:&Args<'p>)->Reply<'p>{
    let code=a.get_item(1)?.extract()?;
    (a.get_item(2)?.extract::<i32>()?==0 && sweep_key(code).is_some_and(|key|key!="Enter")).into_bound_py_any(a.py())
}
fn sweep_on_key<'p>(a:&Args<'p>)->Reply<'p>{
    if let Some(key)=sweep_key(a.get_item(2)?.extract()?){
        let ev=fields!(a.py();"type"=>"key","key"=>key);
        return a.get_item(0)?.call_method1("_send",(a.get_item(1)?,ev))?.get_item("consumed")
    }
    false.into_bound_py_any(a.py())
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("json", py.import("json")?)?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "line")?;
        attrs.set_item(
            "start_point",
            module.getattr("_point_property")?.call1(("start_point",))?,
        )?;
        attrs.set_item(
            "hover_point",
            module.getattr("_point_property")?.call1(("hover_point",))?,
        )?;
        attrs.set_item(
            "__doc__",
            "Line (``rust/built_core/src/tools/line.rs``).",
        )?;
        let cls = define(module, "LineTool", &[module.getattr("RustTool")?], &attrs)?;
        register_tool_adapter(&cls)?;
        let sweep=PyDict::new(py);
        sweep.set_item("__module__",module.name()?)?;
        sweep.set_item("kind","sweep")?;
        let sweep=define(module,"SweepTool",&[cls.clone()],&sweep)?;
        install_methods(module,&sweep,&[
            ("claims_key",Mode::Method,sweep_claims,&["self","key","modifiers"],"bool",""),
            ("on_key",Mode::Method,sweep_on_key,&["self","viewport","key","modifiers"],"bool",""),
        ])?;
        let surface = PyDict::new(py);
        surface.set_item("__module__", module.name()?)?;
        surface.set_item("kind", "line_on_surface")?;
        define(module, "LineOnSurfaceTool", &[cls], &surface)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "select")?;
        attrs.set_item("_MODE_MODS", fields!(py; "replace" => 0, "add" => 67108864, "toggle" => 33554432, "remove" => 100663296).into_any())?;
        attrs.set_item("__doc__","Select (``rust/built_core/src/tools/select.rs``). ``press``,\n``drag``, ``release`` and ``double`` are the raw gesture the Metal view\nsends; the ``on_*`` methods are the older, already-decided entry points.")?;
        let cls = define(module, "SelectTool", &[module.getattr("RustTool")?], &attrs)?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item(
            "start_point",
            module.getattr("_point_property")?.call1(("start_point",))?,
        )?;
        attrs.set_item(
            "hover_point",
            module.getattr("_point_property")?.call1(("hover_point",))?,
        )?;
        attrs.set_item(
            "work_plane",
            module.getattr("LineTool")?.getattr("work_plane")?,
        )?;
        attrs.set_item(
            "__doc__",
            "Common event, projection and lock bindings of the planar drawing tools.",
        )?;
        let cls = define(
            module,
            "_PlanarMachine",
            &[module.getattr("PlaneLock")?, module.getattr("RustTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[(
                "__init__",
                Mode::Method,
                planar_machine_init,
                &["self"],
                "",
                "",
            )],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "rectangle")?;
        attrs.set_item("diagonal_span", true)?;
        attrs.set_item("vcb_comma_lists", true)?;
        attrs.set_item("wireframe_color", none(py))?;
        attrs.set_item("_KEYS", none(py))?;

        attrs.set_item(
            "_live_side",
            py.import("tools.offset_host")?
                .getattr("_field")?
                .call1(("live_side",))?,
        )?;
        attrs.set_item(
            "_arrow_square_lock",
            py.import("tools.offset_host")?
                .getattr("_field")?
                .call1(("arrow_square",))?,
        )?;
        attrs.set_item(
            "_shift_square_lock",
            py.import("tools.offset_host")?
                .getattr("_field")?
                .call1(("shift_square",))?,
        )?;
        attrs.set_item("__doc__","Rectangle (``rust/built_core/src/tools/rectangle.rs``). The\nplane lock is the machine's; ``PlaneLock`` keeps the viewport glue\n(status, redraws, what is under the cursor, the cursor preview).")?;
        let cls = define(
            module,
            "RectangleTool",
            &[module.getattr("_PlanarMachine")?],
            &attrs,
        )?;
        install_methods(
            module,
            &cls,
            &[("_flag", Mode::Method, rectangle_flag, &["name"], "", "")],
        )?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("vcb_label", "Radius")?;
        attrs.set_item("_guide_capable", false)?;
        attrs.set_item("wireframe_color", none(py))?;
        attrs.set_item("__doc__","Circle and Polygon (``rust/built_core/src/tools/circle.rs``): a\ncentre, then a radius. The machine owns the plane lock, the side count,\nthe guide mode and the retype window; this keeps the viewport glue (the\nstatus, the axis colour, the cursor's ring, the readout).")?;
        let cls = define(
            module,
            "_RadialTool",
            &[
                module.getattr("AxisMagnet")?,
                module.getattr("_PlanarMachine")?,
            ],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "circle")?;
        attrs.set_item("_guide_capable", true)?;
        define(
            module,
            "CircleTool",
            &[module.getattr("_RadialTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "polygon")?;
        attrs.set_item("shortcut", none(py))?;
        define(
            module,
            "PolygonTool",
            &[module.getattr("_RadialTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "rotated_rectangle")?;
        attrs.set_item("shortcut", none(py))?;
        attrs.set_item("plane_lock_after_the_first_point", false)?;
        attrs.set_item("wireframe_color", none(py))?;
        attrs.set_item(
            "base_point",
            module.getattr("_point_property")?.call1(("base_point",))?,
        )?;
        attrs.set_item("__doc__","Rotated Rectangle (``rust/built_core/src/tools/rotated_rectangle.rs``).\nThe machine owns the three clicks, the protractors' rules and the plane\nlock; this sends what only the view knows (the face under the first\nclick, the cursor's ray, the axis lock, the protractor's screen size).")?;
        let cls = define(
            module,
            "RotatedRectangleTool",
            &[
                module.getattr("AxisMagnet")?,
                module.getattr("_PlanarMachine")?,
            ],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    module.add("COLOR_FILLET", (0.85, 0.3, 0.8))?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("wireframe_color", none(py))?;
        attrs.set_item("__doc__","The arcs (``rust/built_core/src/tools/arc.rs``): the machine owns\nthe clicks, the plane lock, the segment count and the commit; this keeps\nthe viewport glue (status, colours, readouts).")?;
        let cls = define(
            module,
            "_ArcBase",
            &[
                module.getattr("AxisMagnet")?,
                module.getattr("_PlanarMachine")?,
            ],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("__doc__","``ArcTool.last_fillet_d``: the distance a double-click repeats, one\nfor every arc (the machine's global).")?;
        let cls = define(
            module,
            "_LastFillet",
            &[py.import("builtins")?
                .getattr("type")?
                .call1((module.getattr("RustTool")?,))?],
            &attrs,
        )?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "last_fillet_d",
                    Mode::Property,
                    last_fillet_last_fillet_d,
                    &["cls"],
                    "",
                    "",
                ),
                (
                    "last_fillet_d",
                    Mode::Setter,
                    last_fillet_set_last_fillet_d,
                    &["cls", "value"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "arc")?;
        attrs.set_item("vcb_label", "Bulge")?;
        attrs.set_item(
            "end_point",
            module.getattr("_point_property")?.call1(("end_point",))?,
        )?;
        attrs.set_item("__doc__","2-point arc. The host answers the topology the fillet reads (the\n``host_*`` queries) and projects points to pixels.")?;
        let cls = module.getattr("_LastFillet")?.call1((
            "ArcTool",
            (module.getattr("_ArcBase")?,),
            &attrs,
        ))?;
        module.add("ArcTool", &cls)?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "arc3")?;
        attrs.set_item("vcb_label", "Segments")?;
        attrs.set_item(
            "mid_point",
            module.getattr("_point_property")?.call1(("mid_point",))?,
        )?;
        attrs.set_item(
            "__doc__",
            "3-point arc: the circle through the three clicks.",
        )?;
        let cls = define(
            module,
            "ThreePointArcTool",
            &[module.getattr("_ArcBase")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "center_arc")?;
        attrs.set_item("vcb_label", "Angle")?;
        attrs.set_item(
            "arm_point",
            module.getattr("_point_property")?.call1(("arm_point",))?,
        )?;
        attrs.set_item("__doc__", "Centre arc: centre, the 0° arm, the sweep.")?;
        let cls = define(
            module,
            "CenterArcTool",
            &[module.getattr("_ArcBase")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "pie")?;
        attrs.set_item("shortcut", none(py))?;
        attrs.set_item(
            "__doc__",
            "Pie: the centre arc whose two radii close the wedge into a face.",
        )?;
        define(
            module,
            "PieTool",
            &[module.getattr("CenterArcTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "scale")?;
        attrs.set_item(
            "uses_snap",
            module.getattr("ScaleHost")?.getattr("uses_snap")?,
        )?;
        define(
            module,
            "ScaleTool",
            &[module.getattr("ScaleHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "rotate")?;
        attrs.set_item(
            "_READ_DURING",
            ("start_point", "ref_point", "copy", "array_token"),
        )?;
        let cls = define(
            module,
            "RotateTool",
            &[module.getattr("RotateHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "__init__",
                    Mode::Method,
                    rotate_tool_init,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "start_point",
                    Mode::Property,
                    rotate_tool_start_point,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "start_point",
                    Mode::Setter,
                    rotate_tool_set_start_point,
                    &["self", "p"],
                    "",
                    "",
                ),
                (
                    "ref_point",
                    Mode::Property,
                    rotate_tool_ref_point,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "ref_point",
                    Mode::Setter,
                    rotate_tool_set_ref_point,
                    &["self", "p"],
                    "",
                    "",
                ),
                ("_copy", Mode::Property, rotate_tool_copy, &["self"], "", ""),
                (
                    "_copy",
                    Mode::Setter,
                    rotate_tool_set_copy,
                    &["self", "on"],
                    "",
                    "",
                ),
                ("_last", Mode::Property, rotate_tool_last, &["self"], "", ""),
            ],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "position_camera")?;
        define(
            module,
            "PositionCameraTool",
            &[
                module.getattr("PositionCameraHost")?,
                module.getattr("RustTool")?,
            ],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "look_around")?;
        attrs.set_item("TURN_DEG_PER_WIDTH", 180.0)?;
        attrs.set_item("PITCH_DEG_PER_HEIGHT", 90.0)?;
        define(
            module,
            "LookAroundTool",
            &[module.getattr("EyeHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "walk")?;
        define(
            module,
            "WalkTool",
            &[module.getattr("WalkHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "first_person")?;
        define(
            module,
            "FirstPersonTool",
            &[
                module.getattr("FirstPersonHost")?,
                module.getattr("RustTool")?,
            ],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "offset")?;
        attrs.set_item(
            "uses_snap",
            module.getattr("OffsetHost")?.getattr("uses_snap")?,
        )?;
        define(
            module,
            "OffsetTool",
            &[module.getattr("OffsetHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "paint")?;
        define(
            module,
            "PaintTool",
            &[module.getattr("PaintHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "protractor")?;
        attrs.set_item("_READ_DURING", ("start_point", "ref_point", "guides"))?;
        let cls = define(
            module,
            "ProtractorTool",
            &[
                module.getattr("ProtractorHost")?,
                module.getattr("RustTool")?,
            ],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "__init__",
                    Mode::Method,
                    protractor_tool_init,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "start_point",
                    Mode::Property,
                    protractor_tool_start_point,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "start_point",
                    Mode::Setter,
                    protractor_tool_set_start_point,
                    &["self", "p"],
                    "",
                    "",
                ),
                (
                    "ref_point",
                    Mode::Property,
                    protractor_tool_ref_point,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "ref_point",
                    Mode::Setter,
                    protractor_tool_set_ref_point,
                    &["self", "p"],
                    "",
                    "",
                ),
                (
                    "_guides",
                    Mode::Property,
                    protractor_tool_guides,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_guides",
                    Mode::Setter,
                    protractor_tool_set_guides,
                    &["self", "g"],
                    "",
                    "",
                ),
                (
                    "_last",
                    Mode::Property,
                    protractor_tool_last,
                    &["self"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    // Each concrete class owns the values changed by the shared bevel host.
    for (kind, name) in [("fillet", "FilletTool"), ("chamfer", "ChamferTool")] {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", kind)?;
        attrs.set_item("icon", kind)?;
        attrs.set_item("numeric_profile", false)?;
        if kind == "chamfer" {
            attrs.set_item("profile", 0.25)?;
            attrs.set_item("measure", "offset")?;
        }
        attrs.set_item("_READ_DURING", ("sizing",))?;
        let cls = define(
            module,
            name,
            &[module.getattr("FilletHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "sizing",
                    Mode::Property,
                    fillet_tool_sizing,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "sizing",
                    Mode::Setter,
                    fillet_tool_set_sizing,
                    &["self", "value"],
                    "",
                    "",
                ),
                (
                    "__init__",
                    Mode::Method,
                    fillet_tool_init,
                    &["self"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "tape")?;
        attrs.set_item(
            "_READ_DURING",
            ("start_point", "hover_point", "mode", "direction"),
        )?;
        attrs.set_item(
            "start_point",
            module.getattr("_point_property")?.call1(("start_point",))?,
        )?;
        attrs.set_item(
            "hover_point",
            module.getattr("_point_property")?.call1(("hover_point",))?,
        )?;
        let cls = define(
            module,
            "TapeMeasureTool",
            &[module.getattr("TapeHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "_mode",
                    Mode::Property,
                    tape_measure_tool_mode,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_mode",
                    Mode::Setter,
                    tape_measure_tool_set_mode,
                    &["self", "value"],
                    "",
                    "",
                ),
                (
                    "_measured",
                    Mode::Property,
                    tape_measure_tool_measured,
                    &["self"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "paste")?;
        attrs.set_item("shortcut", none(py))?;
        attrs.set_item("description", none(py))?;
        attrs.set_item(
            "start_point",
            module.getattr("_point_property")?.call1(("start_point",))?,
        )?;
        attrs.set_item(
            "_offset",
            module.getattr("_point_property")?.call1(("offset",))?,
        )?;
        define(
            module,
            "PasteTool",
            &[module.getattr("PasteHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "flip")?;
        attrs.set_item("shortcut", none(py))?;
        attrs.set_item(
            "_READ_DURING",
            ("hover_point", "hover_axis", "lock_axis", "copy"),
        )?;
        attrs.set_item(
            "hover_point",
            module.getattr("_point_property")?.call1(("hover_point",))?,
        )?;
        let cls = define(
            module,
            "FlipTool",
            &[module.getattr("FlipHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "_lock_axis",
                    Mode::Property,
                    flip_tool_lock_axis,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_lock_axis",
                    Mode::Setter,
                    flip_tool_set_lock_axis,
                    &["self", "axis"],
                    "",
                    "",
                ),
                (
                    "_hover_axis",
                    Mode::Property,
                    flip_tool_hover_axis,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_hover_axis",
                    Mode::Setter,
                    flip_tool_set_hover_axis,
                    &["self", "axis"],
                    "",
                    "",
                ),
                ("_copy", Mode::Property, flip_tool_copy, &["self"], "", ""),
                (
                    "_copy",
                    Mode::Setter,
                    flip_tool_set_copy,
                    &["self", "value"],
                    "",
                    "",
                ),
            ],
        )?;
        cls.setattr("cursor_plus", cls.getattr("_copy")?)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "freehand")?;
        attrs.set_item("shortcut", none(py))?;
        attrs.set_item("work_plane", none(py))?;
        let cls = define(
            module,
            "FreehandTool",
            &[module.getattr("RustTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "start_point",
                    Mode::Property,
                    freehand_tool_start_point,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_stroke",
                    Mode::Property,
                    freehand_tool_stroke,
                    &["self"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "change_axes")?;
        attrs.set_item("target", none(py))?;
        attrs.set_item("align_face", false)?;
        attrs.set_item(
            "start_point",
            module.getattr("_point_property")?.call1(("start_point",))?,
        )?;
        attrs.set_item(
            "red_point",
            module.getattr("_point_property")?.call1(("red_point",))?,
        )?;
        attrs.set_item(
            "hover_point",
            module.getattr("_point_property")?.call1(("hover_point",))?,
        )?;
        let cls = define(
            module,
            "ChangeAxesTool",
            &[module.getattr("RustTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "followme")?;
        define(
            module,
            "FollowMeTool",
            &[module.getattr("FollowMeHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "eraser")?;
        define(
            module,
            "EraserTool",
            &[module.getattr("EraserHost")?, module.getattr("RustTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "place_group")?;
        attrs.set_item("shortcut", none(py))?;
        attrs.set_item("description", attrs.get_item("shortcut")?.unwrap())?;
        attrs.set_item(
            "_face_normal",
            module.getattr("_point_property")?.call1(("normal",))?,
        )?;
        let cls = define(
            module,
            "PlaceGroupTool",
            &[
                module.getattr("PlaceGroupHost")?,
                module.getattr("RustTool")?,
            ],
            &attrs,
        )?;
        install_methods(
            module,
            &cls,
            &[(
                "_offset",
                Mode::Property,
                place_group_tool_offset,
                &["self"],
                "",
                "",
            )],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "move")?;
        define(module, "MoveTool", &[module.getattr("MoveHost")?], &attrs)?;
    }
    register_tool_functions(&module.dict())?;
    Ok(())
}

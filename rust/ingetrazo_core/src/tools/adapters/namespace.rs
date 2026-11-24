// SPDX-License-Identifier: GPL-3.0-or-later
//! Lazy Python imports for the public per-tool modules. A path-entry finder on
//! tools.__path__ lets normal imports, reload and pkgutil use the same registry.
use super::*;
use pyo3::exceptions::PyImportError;
use pyo3::types::PyModule;

type Imports = &'static [(&'static str, &'static [&'static str])];
struct Module {
    name: &'static str,
    imports: Imports,
    doc: &'static str,
}

#[rustfmt::skip]
const MODULES: &[Module] = &[
    Module {name:"pushpull", imports:&[("__future__", &["annotations"]), ("ingetrazo_core", &["register_tool_adapter"]), ("core.input_types", &["Qt"]), ("ingetrazo_core", &["Vec3 as QVector3D", "push_reference_target"]), ("views", &["input_state"]), ("core.i18n", &["tr"]), ("core.mesh", &["_STITCH_TOL"]), ("core.mesh", &["Face as MeshFace"]), ("core.orient", &["orient_outward"]), ("core.topology", &["_key"]), ("tools.base", &["ToolContext", "ctrl_clause"]), ("tools.rust_tools", &["RustTool", "_mods"]), ("core.units", &["fmt_len"]), ("core", &["push_scans"]), ("core.push_pull", &["PushPullOperation", "_GroupScene", "_MIN_EXTRUDE", "_face_shape", "_loops_overlap_2d", "_made_since", "_paint_of", "_project_loop_2d"])], doc:"Push/Pull tool: extrude a face along its normal.\n\nUX (the usual push/pull convention):\n- Hover a face; the cursor picks the front-most face under it.\n- First click: lock onto that face and start a drag along its normal.\n- Subsequent mouse motion slides the extrusion preview (wireframe of\n  the future box) along the normal axis. Length is shown near the\n  midpoint, same overlay as the line tool.\n- Second click commits at the current distance.\n- Typing a number + Enter (VCB) commits at exactly that distance,\n  preserving the current direction's sign; a negative value (\"-2\")\n  reverses it, and unit suffixes (\"30cm\", \"1500mm\") are understood.\n- **Ctrl** = push/pull a copy: the start face stays in place (a slab\n  division) and the extrusion stacks as a new segment — how floors are\n  stacked.\n- **Double-click** repeats the last committed distance on the face under\n  the cursor (with Ctrl, as a copy).\n- **Several faces selected**: pushing or pulling any one of them moves them\n  all the same distance, each along its own normal, as one undo step.\n- Esc cancels without committing.\n\nCommit pipeline (the deterministic root-fix, no case tree):\n1. Orient the mesh outward (entry invariant; no-op on open sheets).\n2. Build the *naive* extrusion: consumed/kept base, moved cap (with holes),\n   one wall quad per boundary/hole edge — even where a quad lands on an\n   existing face's plane.\n3. Stitch connectivity (T-junctions, collinear vertices).\n4. On a solid: deterministically rebuild every touched plane (seams, overlaps,\n   cracks) from its edges via ``Store::rebuild_push_planes``; on a flat sheet, fall back\n   to the seeded coplanar merge (no outward side exists there).\n5. Orient outward again — every committed solid upholds the invariant.\n\nA bordered (embedded) base face is consumed either direction — pushing in\ncarves a recess / step / through-hole (the far face is punched when the push\nreaches it); pushing out grows the solid. A free-standing face keeps its base\nas the cap of the new box."},
    Module {name:"scale_host", imports:&[("__future__", &["annotations"]), ("ingetrazo_core", &["register_tool_adapter"]), ("core.input_types", &["Qt"]), ("ingetrazo_core", &["MatrixGesture", "ScaleGripSpec", "scale_grab", "Vec3 as QVector3D"]), ("core.i18n", &["tr"])], doc:"Scale grip picking and resident-row previews for the Rust machine."},
    Module {name:"fillet_host", imports:&[("__future__", &["annotations"]), ("ingetrazo_core", &["register_tool_adapter"]), ("core.input_types", &["Qt"]), ("ingetrazo_core", &["Vec3 as QVector3D", "bevel_nearest_end", "bevel_selection", "bevel_targets"]), ("core.fillet", &["CIRCULAR", "plan_fillet"]), ("core.units", &["fmt_len_fine"]), ("core.i18n", &["tr"]), ("core.mesh", &["Edge"]), ("core.vertex_bevel", &["plan_vertex_bevel"]), ("tools.base", &["Tool"])], doc:"Resident bevel targets, preview mesh and panel bindings for the Fillet machine."},
    Module {name:"move_host", imports:&[("__future__", &["annotations"]), ("ingetrazo_core", &["register_tool_adapter", "register_tool_functions"]), ("tools.overlay_painter", &["draw_overlay"]), ("core.input_types", &["Qt"]), ("ingetrazo_core", &["Vec3 as QVector3D"]), ("core.i18n", &["tr"]), ("tools.base", &["ToolContext"]), ("tools.rust_tool_base", &["RustTool", "_mods", "_point_property", "_tuple"]), ("core.units", &["fmt_len"]), ("ingetrazo_core", &["CopyWire", "GripTurn", "MoveGesture", "TransformPose", "moving_faces", "rotation_handles as rotation_grips", "snap_exclusions", "track_move_grips", "transform_records", "transform_targets as gather_targets"])], doc:"Viewport bindings for the Rust Move tool, including live deformation and grips."},
    Module {name:"rotate_host", imports:&[("__future__", &["annotations"]), ("ingetrazo_core", &["register_tool_adapter"]), ("core.input_types", &["Qt"]), ("ingetrazo_core", &["CopyWire", "MatrixGesture", "Vec3 as QVector3D"]), ("core.group", &["Group"]), ("core.history", &["rotation_matrix"]), ("core.i18n", &["tr"]), ("tools.move_host", &["CopyPreview"]), ("tools.protractor_host", &["ProtractorBase"])], doc:"Host picking, protractor inference and resident-row preview for Rotate."},
    Module {name:"solid_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("tools.rust_tool_base", &["RustTool"])], doc:"Both hosts' bindings for the solid tool gestures."},
    Module {name:"followme_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("ingetrazo_core", &["Vec3"]), ("core.i18n", &["tr"]), ("core.mesh", &["Edge", "Face"]), ("core.sweep", &["manual_path_extend", "manual_path_start", "order_path_edges"])], doc:"Mesh picks, renderer objects and localized messages for Follow Me."},
    Module {name:"tape_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("tools.overlay_painter", &["draw_overlay"]), ("core.input_types", &["Qt"]), ("ingetrazo_core", &["Vec3 as QVector3D"]), ("core.guide", &["Guide"]), ("core.i18n", &["tr"]), ("core.units", &["fmt_len", "fmt_len_fine"]), ("tools.base", &["Tool"])], doc:"Viewport picks, cursor and overlays for the Rust measuring tape."},
    Module {name:"paint_host", imports:&[("ingetrazo_core", &["register_tool_adapter", "register_tool_functions"]), ("core.i18n", &["tr"]), ("core.paint_ops", &["paint_stamp"]), ("core.style", &["DEFAULT_FACE_COLOR"]), ("ingetrazo_core", &["Vec3", "clicked_back_side"])], doc:"Viewport picks and shared toolbar/tray brush state for the Rust Paint tool."},
    Module {name:"eraser_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("core.input_types", &["Qt"]), ("core.dimension", &["Dimension"]), ("core.group", &["Group"]), ("core.guide", &["Guide", "is_guide"]), ("core.mesh", &["Edge"]), ("core.textlabel", &["TextLabel"]), ("georef.geopath", &["GeoPath"])], doc:"Viewport picking and wireframe bindings for an Eraser stroke."},
    Module {name:"north_host", imports:&[("ingetrazo_core", &["register_tool_adapter", "register_tool_functions"]), ("tools.rust_tool_base", &["RustTool"]), ("ingetrazo_core", &["ToolMachine", "Vec3"]), ("tools.rust_tool_base", &["_tuple"])], doc:"Both hosts' bindings for the north tool gestures."},
    Module {name:"section_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("tools.rust_tool_base", &["RustTool"]), ("tools.rust_tool_base", &["_point_property", "_mods"])], doc:"Both hosts' bindings for the section tool gestures."},
    Module {name:"image_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("tools.rust_tool_base", &["RustTool"]), ("ingetrazo_core", &["Vec3"]), ("tools.rust_tool_base", &["_point_property", "_tuple"])], doc:"Both hosts' bindings for the image tool gestures."},
    Module {name:"geopath_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("tools.rust_tool_base", &["RustTool"]), ("ingetrazo_core", &["Vec3"])], doc:"Both hosts' bindings for the geopath tool gestures."},
    Module {name:"place_group_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("ingetrazo_core", &["Vec3 as QVector3D", "ToolMachine"])], doc:"Placement bounds, projected mesh previews and host selection handoff."},
    Module {name:"rust_tools", imports:&[("__future__", &["annotations"]), ("ingetrazo_core", &["register_tool_adapter", "register_tool_functions"]), ("ingetrazo_core", &["ToolMachine", "Vec3"]), ("tools.rust_tool_base", &["RustTool", "_Host", "_mods", "_point_property", "_tuple", "selection_mode"]), ("tools.base", &["AxisMagnet", "PlaneLock"]), ("tools.scale_host", &["ScaleHost"]), ("tools.rotate_host", &["RotateHost"]), ("tools.walkthrough_host", &["EyeHost", "PositionCameraHost", "WalkHost", "FirstPersonHost", "DEFAULT_EYE_HEIGHT", "DEFAULT_LOOK_SENSITIVITY", "eye_height", "set_eye_height", "look_sensitivity", "look_deg_per_px"]), ("tools.offset_host", &["OffsetHost"]), ("tools.paint_host", &["PaintHost", "DEFAULT_FACE_COLOR", "RECENT_MAX", "recent_key", "push_recent", "rename_recent", "clicked_back_side"]), ("tools.protractor_host", &["ProtractorHost", "ProtractorBase", "DISC_PX", "TICK_DEG"]), ("tools.fillet_host", &["FilletHost"]), ("tools.tape_host", &["TapeHost"]), ("tools.paste_host", &["PasteHost"]), ("tools.flip_host", &["FlipHost"]), ("tools.followme_host", &["FollowMeHost"]), ("tools.solid_host", &["SolidTool", "OuterShellTool", "UnionTool", "SubtractTool", "TrimTool", "IntersectTool", "SplitTool", "SOLID_TOOLS"]), ("tools.scale_manager_host", &["ScaleManagerTool"]), ("tools.image_host", &["ImageTool"]), ("tools.north_host", &["NorthTool", "COMPASS_PX", "refresh_north_display", "north_deg_towards", "compass_segments"]), ("tools.section_host", &["SectionPlaneTool"]), ("tools.annotation_host", &["TextTool", "DimensionTool"]), ("tools.geopath_host", &["GeoPathTool"]), ("tools.eraser_host", &["EraserHost"]), ("tools.place_group_host", &["PlaceGroupHost"]), ("tools.move_host", &["MoveHost", "rotation_grips", "gather_targets", "gather_images"])], doc:"The Rust tools seen from Python (docs/rust-tools.md §10).\n\nEach class keeps the name, attributes and methods the hosts, the inference\nand extensions use (``on_click(ctx)``, ``rubber_band_lines()``,\n``start_point``…), turns the call into an event for its\n``ingetrazo_core.ToolMachine`` and answers the machine's host calls. The\nnative viewport host handles picks, document queries and protocol commands.\nPython retains tool identities for extensions, UI readouts and view callbacks."},
    Module {name: "arc", imports:&[("tools.rust_tools", &["ArcTool", "CenterArcTool", "PieTool", "ThreePointArcTool", "COLOR_FILLET", "commit_arc"])], doc:""},
    Module {name: "change_axes", imports:&[("tools.rust_tools", &["ChangeAxesTool"])], doc:""},
    Module {name: "circle", imports:&[("tools.rust_tools", &["CircleTool", "PolygonTool"]), ("core.arrangement", &["busy_plane", "flat_drawing"])], doc:""},
    Module {name: "dimension", imports:&[("tools.rust_tools", &["DimensionTool"])], doc:""},
    Module {name: "eraser", imports:&[("tools.rust_tools", &["EraserTool"])], doc:""},
    Module {name: "fillet", imports:&[("tools.rust_tools", &["FilletTool"])], doc:""},
    Module {name: "flip", imports:&[("tools.rust_tools", &["FlipTool"])], doc:""},
    Module {name: "flip_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("tools.overlay_painter", &["draw_overlay"]), ("core.input_types", &["Qt"]), ("ingetrazo_core", &["Vec3 as QVector3D"]), ("ingetrazo_core", &["ToolMachine"]), ("core.group", &["Group"]), ("core.mesh", &["Edge", "Face"]), ("core.axes", &["AXES as _AXES"]), ("core.i18n", &["tr"]), ("tools.base", &["Tool", "ctrl_clause"])], doc:"Flip tool — Flip (mirror).\n\nFlow: with a selection, three semi-transparent planes appear over it —\nred, green and blue, one per axis. Hovering highlights a plane; ONE CLICK\nflips the selection about it. The arrow keys pick a plane (Right = red,\nLeft = green, Up = blue); tapping Ctrl toggles COPY mode, which leaves the\noriginal and creates the flipped duplicate.\n\nDeferred (documented): dragging a plane to reposition it, Alt\nparent-context axes, and the magenta custom plane from hovering a face."},
    Module {name: "followme", imports:&[("tools.rust_tools", &["FollowMeTool"])], doc:""},
    Module {name: "freehand", imports:&[("tools.rust_tools", &["FreehandTool"])], doc:""},
    Module {name: "geopath", imports:&[("tools.rust_tools", &["GeoPathTool"])], doc:""},
    Module {name: "image", imports:&[("tools.rust_tools", &["ImageTool"])], doc:""},
    Module {name: "line", imports:&[("tools.rust_tools", &["LineTool"])], doc:""},
    Module {name: "move", imports:&[("tools.rust_tools", &["MoveTool", "rotation_grips", "gather_targets", "gather_images"])], doc:""},
    Module {name: "north", imports:&[("tools.rust_tools", &["NorthTool", "COMPASS_PX", "north_deg_towards", "compass_segments", "refresh_north_display"])], doc:""},
    Module {name: "offset", imports:&[("tools.rust_tools", &["OffsetTool"])], doc:""},
    Module {name: "paint", imports:&[("tools.rust_tools", &["PaintTool", "DEFAULT_FACE_COLOR", "RECENT_MAX", "recent_key", "push_recent", "rename_recent", "clicked_back_side"])], doc:""},
    Module {name: "paste", imports:&[("tools.rust_tools", &["PasteTool"])], doc:""},
    Module {name: "paste_host", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("ingetrazo_core", &["Vec3 as QVector3D"]), ("tools.base", &["Tool"])], doc:"Paste tool: drop a copied set of geometry, placing it with the cursor.\n\nAfter Copy/Cut fills ``viewport.clipboard``, Paste activates this tool: the copied\nfaces, edges and groups follow the cursor as a live preview (snapping like any\ndraw), and a click stamps them into the scene ONCE and returns to Select.\nThe clipboard is kept — paste again for another copy."},
    Module {name: "place_group", imports:&[("tools.rust_tools", &["PlaceGroupTool"])], doc:""},
    Module {name: "protractor", imports:&[("tools.rust_tools", &["ProtractorTool", "ProtractorBase", "DISC_PX", "TICK_DEG"])], doc:""},
    Module {name: "rectangle", imports:&[("tools.rust_tools", &["RectangleTool"])], doc:""},
    Module {name: "rotate", imports:&[("tools.rust_tools", &["RotateTool"])], doc:""},
    Module {name: "rotated_rectangle", imports:&[("tools.rust_tools", &["RotatedRectangleTool"])], doc:""},
    Module {name: "scale", imports:&[("tools.rust_tools", &["ScaleTool"])], doc:""},
    Module {name: "scale_manager", imports:&[("tools.rust_tools", &["ScaleManagerTool"])], doc:""},
    Module {name: "section", imports:&[("tools.rust_tools", &["SectionPlaneTool"])], doc:""},
    Module {name: "select", imports:&[("tools.rust_tools", &["SelectTool", "selection_mode"]), ("tools.select_host", &["delete_selection_or_hover", "erase_entities", "expand_pick"])], doc:""},
    Module {name: "solid_tools", imports:&[("tools.rust_tools", &["SolidTool", "OuterShellTool", "UnionTool", "SubtractTool", "TrimTool", "IntersectTool", "SplitTool", "SOLID_TOOLS"])], doc:""},
    Module {name: "tape", imports:&[("tools.rust_tools", &["TapeMeasureTool"])], doc:""},
    Module {name: "text", imports:&[("tools.rust_tools", &["TextTool"])], doc:""},
    Module {name: "texture_position_metal", imports:&[("ingetrazo_core", &["register_tool_adapter"]), ("time", &["perf_counter"]), ("ingetrazo_core", &["TextureToolRows"])], doc:"Submit the placement machine's UV map to the editable face's material rows."},
    Module {name: "walkthrough", imports:&[("tools.rust_tools", &["PositionCameraTool", "LookAroundTool", "WalkTool", "FirstPersonTool", "DEFAULT_EYE_HEIGHT", "DEFAULT_LOOK_SENSITIVITY", "eye_height", "set_eye_height", "look_sensitivity", "look_deg_per_px"])], doc:""},
];

// These three former host files had no independent tool logic after the port.
const HOST_CLASSES: &[(&str, &str, &str)] = &[
    ("tools.flip_host", "FlipHost", "Tool"),
    ("tools.paste_host", "PasteHost", "Tool"),
    ("tools.texture_position_metal", "TexturePreview", "object"),
];

#[pyfunction]
pub(super) fn tool_module_classes() -> Vec<(&'static str, &'static str, &'static str)> {
    let mut classes = HOST_CLASSES.to_vec();
    classes.extend_from_slice(&[
    ("tools.pushpull","PushPullTool","PushPullOperation,RustTool"),
    ("tools.scale_host","_Grip","object"),
    ("tools.scale_host","ScaleHost","object"),
    ("tools.fillet_host","FilletHost","Tool"),
    ("tools.move_host","CopyPreview","object"),
    ("tools.move_host","MoveHost","CopyPreview,RustTool"),
    ("tools.rotate_host","RotateHost","CopyPreview,ProtractorBase"),
    ("tools.solid_host","SolidTool","RustTool"),
    ("tools.solid_host","OuterShellTool","SolidTool"),
    ("tools.solid_host","UnionTool","SolidTool"),
    ("tools.solid_host","SubtractTool","SolidTool"),
    ("tools.solid_host","TrimTool","SolidTool"),
    ("tools.solid_host","IntersectTool","SolidTool"),
    ("tools.solid_host","SplitTool","SolidTool"),
    ("tools.followme_host","FollowMeHost","object"),
    ("tools.tape_host","TapeHost","Tool"),
    ("tools.paint_host","PaintHost","object"),
    ("tools.eraser_host","EraserHost","object"),
    ("tools.north_host","NorthTool","RustTool"),
    ("tools.section_host","SectionPlaneTool","RustTool"),
    ("tools.image_host","ImageTool","RustTool"),
    ("tools.geopath_host","GeoPathTool","RustTool"),
    ("tools.place_group_host","PlaceGroupHost","object"),
    ("tools.rust_tools","LineTool","RustTool"),
    ("tools.rust_tools","SelectTool","RustTool"),
    ("tools.rust_tools","_PlanarMachine","PlaneLock,RustTool"),
    ("tools.rust_tools","RectangleTool","_PlanarMachine"),
    ("tools.rust_tools","_RadialTool","AxisMagnet,_PlanarMachine"),
    ("tools.rust_tools","CircleTool","_RadialTool"),
    ("tools.rust_tools","PolygonTool","_RadialTool"),
    ("tools.rust_tools","RotatedRectangleTool","AxisMagnet,_PlanarMachine"),
    ("tools.rust_tools","_ArcBase","AxisMagnet,_PlanarMachine"),
    ("tools.rust_tools","_LastFillet","type(RustTool)"),
    ("tools.rust_tools","ArcTool","_ArcBase"),
    ("tools.rust_tools","ThreePointArcTool","_ArcBase"),
    ("tools.rust_tools","CenterArcTool","_ArcBase"),
    ("tools.rust_tools","PieTool","CenterArcTool"),
    ("tools.rust_tools","ScaleTool","ScaleHost,RustTool"),
    ("tools.rust_tools","RotateTool","RotateHost,RustTool"),
    ("tools.rust_tools","PositionCameraTool","PositionCameraHost,RustTool"),
    ("tools.rust_tools","LookAroundTool","EyeHost,RustTool"),
    ("tools.rust_tools","WalkTool","WalkHost,RustTool"),
    ("tools.rust_tools","FirstPersonTool","FirstPersonHost,RustTool"),
    ("tools.rust_tools","OffsetTool","OffsetHost,RustTool"),
    ("tools.rust_tools","PaintTool","PaintHost,RustTool"),
    ("tools.rust_tools","ProtractorTool","ProtractorHost,RustTool"),
    ("tools.rust_tools","FilletTool","FilletHost,RustTool"),
    ("tools.rust_tools","TapeMeasureTool","TapeHost,RustTool"),
    ("tools.rust_tools","PasteTool","PasteHost,RustTool"),
    ("tools.rust_tools","FlipTool","FlipHost,RustTool"),
    ("tools.rust_tools","FreehandTool","RustTool"),
    ("tools.rust_tools","ChangeAxesTool","RustTool"),
    ("tools.rust_tools","FollowMeTool","FollowMeHost,RustTool"),
    ("tools.rust_tools","EraserTool","EraserHost,RustTool"),
    ("tools.rust_tools","PlaceGroupTool","PlaceGroupHost,RustTool"),
    ("tools.rust_tools","MoveTool","MoveHost"),
    ]);
    classes
}

fn define_host(module: &Bound<'_, PyModule>, name: &str, base: &str) -> PyResult<()> {
    let py = module.py();
    let attrs = PyDict::new(py);
    attrs.set_item("__module__", module.name()?)?;
    match name {
        "FlipHost" => {
            attrs.set_item("overlay_kind", "flip")?;
            attrs.set_item("draw_overlay", module.getattr("draw_overlay")?)?;
        }
        "PasteHost" => {
            attrs.set_item("wireframe_color", (0.13, 0.17, 0.23, 1.0))?;
            attrs.set_item("wireframe_depth_tested", true)?;
            attrs.set_item("_clipboard", py.None())?;
            attrs.set_item("_preview_target", py.None())?;
        }
        _ => {}
    }
    let base = if base == "object" {
        py.import("builtins")?.getattr(base)?
    } else {
        module.getattr(base)?
    };
    let cls = py
        .get_type::<pyo3::types::PyType>()
        .call1((name, (base,), attrs))?;
    module.add(name, register_tool_adapter(&cls)?)?;
    Ok(())
}

fn definition(name: &str) -> Option<&'static Module> {
    let leaf = name.strip_prefix("tools.")?;
    MODULES.iter().find(|m| m.name == leaf)
}

#[pyclass(module = "ingetrazo_core", frozen)]
struct ToolModules {
    path: String,
}

#[pymethods]
impl ToolModules {
    // PathFinder asks for the finder again if a plugin clears its importer cache.
    fn __call__<'py>(slf: &Bound<'py, Self>, path: &str) -> PyResult<Bound<'py, Self>> {
        if path == slf.get().path {
            Ok(slf.clone())
        } else {
            Err(PyImportError::new_err("not the Built tool module path"))
        }
    }

    #[pyo3(signature = (fullname, target=None))]
    fn find_spec<'py>(
        slf: &Bound<'py, Self>,
        fullname: &str,
        target: Option<&Obj<'py>>,
    ) -> Reply<'py> {
        let _ = target;
        let py = slf.py();
        if definition(fullname).is_none() {
            return Ok(none(py));
        }
        py.import("importlib.machinery")?.call_method(
            "ModuleSpec",
            (fullname, slf),
            Some(&fields!(py; "origin"=>"ingetrazo_core")),
        )
    }

    fn create_module(&self, _spec: &Obj<'_>) {}
    fn exec_module(&self, module: &Bound<'_, PyModule>) -> PyResult<()> {
        let py = module.py();
        let full: String = module.name()?.extract()?;
        let spec = definition(&full).ok_or_else(|| PyImportError::new_err(full.clone()))?;
        if matches!(spec.name, "flip_host" | "paste_host") {
            module.add(
                "annotations",
                py.import("__future__")?.getattr("annotations")?,
            )?;
        }
        if !spec.doc.is_empty() {
            module.add("__doc__", spec.doc)?;
        }
        for &(from, names) in spec.imports {
            let source = py.import(from)?;
            for &field in names {
                let (name, alias) = field.split_once(" as ").unwrap_or((field, field));
                let value = match source.getattr(name) {
                    Ok(value) => value,
                    Err(error) if error.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) => {
                        py.import(format!("{from}.{name}"))?.into_any()
                    }
                    Err(error) => return Err(error),
                };
                module.add(alias, value)?;
            }
        }
        super::definitions::install(spec.name, module)?;
        if let Some((_, name, base)) = HOST_CLASSES.iter().find(|(path, _, _)| *path == full) {
            define_host(module, name, base)?;
        }
        Ok(())
    }
    #[pyo3(signature=(prefix=""))]
    fn iter_modules(&self, prefix: &str) -> Vec<(String, bool)> {
        MODULES
            .iter()
            .map(|m| (format!("{prefix}{}", m.name), false))
            .collect()
    }
}

/// Keep filesystem plugins discoverable on the existing path. The native
/// path entry only recognizes this package's former adapter modules.
#[pyfunction]
pub(super) fn install_tool_modules(paths: &Bound<'_, PyList>) -> PyResult<()> {
    let py = paths.py();
    let sys = py.import("sys")?;
    let first = paths.get_item(0)?.extract::<String>()?;
    let key = std::path::Path::new(&first)
        .join("__built_native_tools__")
        .to_string_lossy()
        .into_owned();
    let cache = sys.getattr("path_importer_cache")?;
    if !paths.contains(&key)? {
        let hooks = sys.getattr("path_hooks")?;
        let finder = hooks
            .try_iter()?
            .filter_map(|item| item.ok()?.cast_into::<ToolModules>().ok())
            .find(|finder| finder.get().path == key);
        let finder = match finder {
            Some(f) => f,
            None => {
                let f = Bound::new(py, ToolModules { path: key.clone() })?;
                hooks.call_method1("append", (&f,))?;
                f
            }
        };
        cache.set_item(&key, finder)?;
        paths.append(key)?;
    }
    Ok(())
}

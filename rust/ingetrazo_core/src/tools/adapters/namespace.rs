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
    HOST_CLASSES.to_vec()
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
        if !spec.doc.is_empty() {
            module.add("__doc__", spec.doc)?;
        }
        for &(from, names) in spec.imports {
            let source = py.import(from)?;
            for &field in names {
                let (name, alias) = field.split_once(" as ").unwrap_or((field, field));
                module.add(alias, source.getattr(name)?)?;
            }
        }
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

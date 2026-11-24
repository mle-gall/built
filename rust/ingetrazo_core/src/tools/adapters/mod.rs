// SPDX-License-Identifier: GPL-3.0-or-later
//! Host-side tool actions. The class and model handles belong to the plugin API;
//! events and commands still enter the same ToolMachine and history.
use built_python_bindings::{Adapter, Endpoint, Mode, Obj, Registrar};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
    IntoPyObjectExt,
};

// Most machine events consist of a few named fields.
macro_rules! fields {
    ($py:expr; $($key:expr => $value:expr),* $(,)?) => {{
        let fields = PyDict::new($py);
        $(fields.set_item($key, $value)?;)*
        fields
    }};
}

fn positional<'py>(value: &Obj<'py>) -> PyResult<Args<'py>> {
    if let Ok(tuple) = value.cast::<PyTuple>() {
        return Ok(tuple.clone());
    }
    Ok(value
        .py()
        .get_type::<PyTuple>()
        .call1((value,))?
        .cast_into()?)
}

fn unpack<'py, const N: usize>(value: &Obj<'py>) -> PyResult<[Obj<'py>; N]> {
    if value.cast_exact::<PyTuple>().is_ok() {
        return value.extract();
    }
    let mut items = value.try_iter()?;
    let row: Vec<_> = items.by_ref().take(N).collect::<PyResult<_>>()?;
    if row.len() != N || items.next().transpose()?.is_some() {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "expected {N} fields"
        )));
    }
    row.try_into()
        .map_err(|_| pyo3::exceptions::PyValueError::new_err("invalid row"))
}
mod annotation_tool_api;
mod arc_base_api;
mod arc_tool_api;
mod axis_magnet_api;
mod center_arc_tool_api;
mod change_axes_tool_api;
mod dimension_tool_api;
mod eraser_host_api;
mod eye_host_api;
mod fillet_host_api;
mod first_person_host_api;
mod flip_host_api;
mod follow_me_host_api;
mod freehand_tool_api;
mod geo_path_tool_api;
mod host_api;
mod image_tool_api;
mod line_tool_api;
mod move_host_api;
mod namespace;
mod north_tool_api;
mod offset_host_api;
mod paint_host_api;
mod paste_host_api;
mod place_group_host_api;
mod planar_machine_api;
mod plane_lock_api;
mod position_camera_host_api;
mod protractor_base_api;
mod protractor_host_api;
mod protractor_tool_api;
mod pushpull;
mod radial_tool_api;
mod rectangle_tool_api;
mod rotate_host_api;
mod rotate_tool_api;
mod rotated_rectangle_tool_api;
mod rust_tool_api;
mod scale;
mod scale_manager_tool_api;
mod section_plane_tool_api;
mod select_tool_api;
mod solid_tool_api;
mod tape_host_api;
mod tape_measure_tool_api;
mod text_tool_api;
mod texture;
mod texture_preview;
mod three_point_arc_tool_api;
mod walk_host_api;
type Args<'py> = Bound<'py, PyTuple>;
type Reply<'py> = PyResult<Obj<'py>>;
type Method = (
    &'static str,
    Mode,
    Adapter,
    &'static [&'static str],
    &'static str,
    &'static str,
);

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn optional<'py>(obj: &Obj<'py>, name: &str) -> Reply<'py> {
    Ok(obj.getattr_opt(name)?.unwrap_or_else(|| none(obj.py())))
}

fn screen<'py>(ctx: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let point = ctx.getattr("screen")?;
    Ok((point.call_method0("x")?, point.call_method0("y")?))
}

fn registration(module: &str, name: &str) -> Option<(&'static [Method], &'static str)> {
    Some(match (module, name) {
        ("tools.scale_host", "_Grip") => (scale::GRIP_METHODS, "scale"),
        ("tools.scale_host", "ScaleHost") => (scale::METHODS, stringify!(scale)),
        ("tools.texture_position_metal", "TexturePreview") => {
            (texture_preview::METHODS, stringify!(texture_preview))
        }
        ("tools.texture_position", "TexturePositionTool") => {
            (texture::METHODS, stringify!(texture))
        }
        ("tools.fillet_host", "FilletHost") => {
            (fillet_host_api::METHODS, stringify!(fillet_host_api))
        }
        ("tools.rust_tools", "SelectTool") => {
            (select_tool_api::METHODS, stringify!(select_tool_api))
        }
        ("tools.move_host", "MoveHost") => (move_host_api::METHODS, stringify!(move_host_api)),
        ("tools.rotate_host", "RotateHost") => {
            (rotate_host_api::METHODS, stringify!(rotate_host_api))
        }
        ("tools.rust_tools", "RotateTool") => {
            (rotate_tool_api::METHODS, stringify!(rotate_tool_api))
        }
        ("tools.walkthrough_host", "EyeHost") => (eye_host_api::METHODS, stringify!(eye_host_api)),
        ("tools.walkthrough_host", "PositionCameraHost") => (
            position_camera_host_api::METHODS,
            stringify!(position_camera_host_api),
        ),
        ("tools.walkthrough_host", "WalkHost") => {
            (walk_host_api::METHODS, stringify!(walk_host_api))
        }
        ("tools.walkthrough_host", "FirstPersonHost") => (
            first_person_host_api::METHODS,
            stringify!(first_person_host_api),
        ),
        ("tools.offset_host", "OffsetHost") => {
            (offset_host_api::METHODS, stringify!(offset_host_api))
        }
        ("tools.protractor_host", "ProtractorBase") => (
            protractor_base_api::METHODS,
            stringify!(protractor_base_api),
        ),
        ("tools.protractor_host", "ProtractorHost") => (
            protractor_host_api::METHODS,
            stringify!(protractor_host_api),
        ),
        ("tools.rust_tools", "ProtractorTool") => (
            protractor_tool_api::METHODS,
            stringify!(protractor_tool_api),
        ),
        ("tools.paint_host", "PaintHost") => (paint_host_api::METHODS, stringify!(paint_host_api)),
        ("tools.rust_tools", "_PlanarMachine") => {
            (planar_machine_api::METHODS, stringify!(planar_machine_api))
        }
        ("tools.rust_tools", "_RadialTool") => {
            (radial_tool_api::METHODS, stringify!(radial_tool_api))
        }
        ("tools.rust_tools", "RotatedRectangleTool") => (
            rotated_rectangle_tool_api::METHODS,
            stringify!(rotated_rectangle_tool_api),
        ),
        ("tools.rust_tools", "RectangleTool") => {
            (rectangle_tool_api::METHODS, stringify!(rectangle_tool_api))
        }
        ("tools.rust_tools", "_ArcBase") => (arc_base_api::METHODS, stringify!(arc_base_api)),
        ("tools.rust_tools", "ArcTool") => (arc_tool_api::METHODS, stringify!(arc_tool_api)),
        ("tools.rust_tools", "ThreePointArcTool") => (
            three_point_arc_tool_api::METHODS,
            stringify!(three_point_arc_tool_api),
        ),
        ("tools.rust_tools", "CenterArcTool") => (
            center_arc_tool_api::METHODS,
            stringify!(center_arc_tool_api),
        ),
        ("tools.annotation_host", "_AnnotationTool") => (
            annotation_tool_api::METHODS,
            stringify!(annotation_tool_api),
        ),
        ("tools.annotation_host", "DimensionTool") => {
            (dimension_tool_api::METHODS, stringify!(dimension_tool_api))
        }
        ("tools.annotation_host", "TextTool") => {
            (text_tool_api::METHODS, stringify!(text_tool_api))
        }
        ("tools.solid_host", "SolidTool") => (solid_tool_api::METHODS, stringify!(solid_tool_api)),
        ("tools.followme_host", "FollowMeHost") => {
            (follow_me_host_api::METHODS, stringify!(follow_me_host_api))
        }
        ("tools.tape_host", "TapeHost") => (tape_host_api::METHODS, stringify!(tape_host_api)),
        ("tools.rust_tools", "TapeMeasureTool") => (
            tape_measure_tool_api::METHODS,
            stringify!(tape_measure_tool_api),
        ),
        ("tools.flip_host", "FlipHost") => (flip_host_api::METHODS, stringify!(flip_host_api)),
        ("tools.north_host", "NorthTool") => (north_tool_api::METHODS, stringify!(north_tool_api)),
        ("tools.eraser_host", "EraserHost") => {
            (eraser_host_api::METHODS, stringify!(eraser_host_api))
        }
        ("tools.paste_host", "PasteHost") => (paste_host_api::METHODS, stringify!(paste_host_api)),
        ("tools.section_host", "SectionPlaneTool") => (
            section_plane_tool_api::METHODS,
            stringify!(section_plane_tool_api),
        ),
        ("tools.geopath_host", "GeoPathTool") => {
            (geo_path_tool_api::METHODS, stringify!(geo_path_tool_api))
        }
        ("tools.image_host", "ImageTool") => (image_tool_api::METHODS, stringify!(image_tool_api)),
        ("tools.place_group_host", "PlaceGroupHost") => (
            place_group_host_api::METHODS,
            stringify!(place_group_host_api),
        ),
        ("tools.scale_manager_host", "ScaleManagerTool") => (
            scale_manager_tool_api::METHODS,
            stringify!(scale_manager_tool_api),
        ),
        ("tools.rust_tools", "LineTool") => (line_tool_api::METHODS, stringify!(line_tool_api)),
        ("tools.rust_tools", "FreehandTool") => {
            (freehand_tool_api::METHODS, stringify!(freehand_tool_api))
        }
        ("tools.rust_tools", "ChangeAxesTool") => (
            change_axes_tool_api::METHODS,
            stringify!(change_axes_tool_api),
        ),
        ("tools.base", "AxisMagnet") => (axis_magnet_api::METHODS, stringify!(axis_magnet_api)),
        ("tools.base", "PlaneLock") => (plane_lock_api::METHODS, stringify!(plane_lock_api)),
        ("tools.rust_tool_base", "RustTool") => (rust_tool_api::METHODS, stringify!(rust_tool_api)),
        ("tools.rust_tool_base", "_Host") => (host_api::METHODS, stringify!(host_api)),
        ("tools.pushpull", "PushPullTool") => (pushpull::METHODS, stringify!(pushpull)),
        _ => return None,
    })
}

/// Dependency audit for the published Metal tool coverage. Only the developer
/// diagnostic reads source files. Interactive dispatch uses the method table.
#[pyfunction]
pub fn tool_preview_hooks(
    root: std::path::PathBuf,
    module: &str,
    name: &str,
) -> PyResult<Vec<String>> {
    use std::collections::BTreeSet;
    use std::sync::LazyLock;
    let Some((methods, source)) = registration(module, name) else {
        return Ok(Vec::new());
    };
    let path = root
        .join("rust/ingetrazo_core/src/tools/adapters")
        .join(format!("{source}.rs"));
    let content = std::fs::read_to_string(path)?;
    static HOOKS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
        r#"(?:viewport|vp)\s*\.\s*(?:getattr|call_method[01]?)\(\s*"(pick_\w+|\w*preview\w*|_world_to_pixel|_pixel_to_ray)""#
    ).expect("tool viewport hook pattern")
    });
    let mut hooks: BTreeSet<String> = HOOKS
        .captures_iter(&content)
        .map(|m| m[1].to_owned())
        .collect();
    for &(name, _, _, _, _, _) in methods {
        match name {
            "rubber_band_lines" => {
                hooks.insert(name.to_owned());
            }
            "scale_box_state" => {
                hooks.insert("scale_box_state (screen handles)".to_owned());
            }
            _ => {}
        }
    }
    Ok(hooks.into_iter().collect())
}

#[pyfunction]
pub fn register_tool_adapter<'py>(cls: &Obj<'py>) -> Reply<'py> {
    let py = cls.py();
    let factory =
        Registrar::new(py)?.with_plugin_method(py.import("tools")?.getattr("_plugin_method")?);
    let module: String = cls.getattr("__module__")?.extract()?;
    let name: String = cls.getattr("__name__")?.extract()?;
    if module == "tools.walkthrough_host" && matches!(name.as_str(), "WalkHost" | "FirstPersonHost")
    {
        factory.bind_default(
            "Qt.NoModifier",
            &py.import("tools.walkthrough_host")?
                .getattr("Qt")?
                .getattr("NoModifier")?,
        )?;
    }
    let (methods, _) = registration(&module, &name).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err(format!("Unknown tool adapter {module}.{name}"))
    })?;
    for &(name, mode, action, params, result, doc) in methods {
        factory.define(
            cls,
            mode,
            name,
            Endpoint::Adapter(action),
            params,
            result,
            doc,
        )?;
    }
    py.import("abc")?
        .call_method1("update_abstractmethods", (cls,))?;
    Ok(cls.clone())
}

mod curve_functions;
mod drawing_functions;
mod move_functions;
mod north_functions;
mod offset_functions;
mod paint_functions;
mod selection;
mod tool_functions;
mod walk_functions;
#[pyfunction]
pub fn register_tool_functions(namespace: &Bound<'_, PyDict>) -> PyResult<()> {
    let factory = Registrar::new(namespace.py())?;
    let module: String = namespace
        .get_item("__name__")?
        .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("__name__"))?
        .extract()?;
    let methods = match module.as_str() {
        "tools.select_host" => selection::METHODS,
        "tools.base" => drawing_functions::METHODS,
        "tools.paint_host" => paint_functions::METHODS,
        "tools.offset_host" => offset_functions::METHODS,
        "tools.north_host" => north_functions::METHODS,
        "tools.move_host" => move_functions::METHODS,
        "tools.walkthrough_host" => walk_functions::METHODS,
        "tools.rust_tool_base" => tool_functions::METHODS,
        "tools.rust_tools" => curve_functions::METHODS,
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Unknown tool module {module}"
            )))
        }
    };
    if module == "tools.paint_host" {
        factory.bind_default(
            "RECENT_MAX",
            &namespace
                .get_item("RECENT_MAX")?
                .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("RECENT_MAX"))?,
        )?;
    }
    for &(name, mode, action, params, result, doc) in methods {
        factory.define(
            namespace.as_any(),
            mode,
            name,
            Endpoint::Adapter(action),
            params,
            result,
            doc,
        )?;
    }
    Ok(())
}

pub fn register_interaction(m: &Bound<'_, PyModule>) -> PyResult<()> {
    use crate::{annotation_geometry, document_snap, pick_index};
    m.add_function(wrap_pyfunction!(pick_index::rays::index_ray_hits, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::rays::ray_aabb_span, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::lazy::pick_lazy_table, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::lazy::pick_materialize, m)?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::prototypes::pick_instance_set,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::prototypes::start_pick_warmup,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::prototypes::pick_lazy_native,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::prototypes::pick_lazy_selectable,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(pick_index::hits::face_hits, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::hits::index_ray_distance, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::hits::cached_face_hits, m)?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::footprint::pick_depth_limits,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(pick_index::footprint::depth_hidden, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::footprint::footprint_need, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::footprint::footprint_rows, m)?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::footprint::visible_depth_limit,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::footprint::placements_under_px,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::occluders::static_occluders,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(pick_index::occluders::static_ray_rows, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::occluders::flat_pick_index, m)?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::occluders::outer_loose_meshes,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        pick_index::occluders::loose_mesh_wrapper,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(pick_index::edges::pick_edge_distance, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::edges::linework_edge_rows, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::edges::linework_groups, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::edges::edge_cursor_point, m)?)?;
    m.add_function(wrap_pyfunction!(pick_index::hits::drop_past_cut, m)?)?;
    annotation_geometry::register(m)?;
    pick_index::visibility::register(m)?;
    document_snap::register(m)?;
    m.add_function(wrap_pyfunction!(pick_index::document_pick_index, m)?)?;
    m.add_function(wrap_pyfunction!(namespace::install_tool_modules, m)?)?;
    m.add_function(wrap_pyfunction!(namespace::tool_module_classes, m)?)?;
    m.add_function(wrap_pyfunction!(tool_preview_hooks, m)?)?;
    m.add_function(wrap_pyfunction!(register_tool_adapter, m)?)?;
    m.add_function(wrap_pyfunction!(register_tool_functions, m)?)?;
    Ok(())
}

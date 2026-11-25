// SPDX-License-Identifier: GPL-3.0-or-later
//! Host bindings for view_navigation.
use super::*;
fn view_navigation_begin_look<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let ev = a.get_item(1)?;
    let view = a.get_item(0)?;
    view.setattr(
        "_look_drag",
        (
            ev.call_method0("button")?,
            ev.call_method0("position")?.call_method0("toPoint")?,
        )
            .into_bound_py_any(py)?,
    )?;
    view.call_method1(
        "setCursor",
        (module.getattr("Qt")?.getattr("BlankCursor")?,),
    )?;
    Ok(none(py))
}

pub(super) fn register(cls: &Obj, module: &Bound<PyModule>) -> PyResult<()> {
    let ns = module.dict();
    install(
        cls,
        &ns,
        &[(
            "_begin_look",
            Mode::Method,
            view_navigation_begin_look,
            &["self", "ev"],
            "None",
            "",
        )],
    )?;
    let registrar = Bindings::new(cls.py())?
        .with_plugin_method(cls.py().import("views")?.getattr("_plugin_method")?)
        .with_module_annotations(&ns)?;
    registrar.install(
        cls,
        &module.getattr("ingetrazo_render")?,
        Mode::Method,
        CAMERA_KERNELS,
    )
}

#[rustfmt::skip]
const CAMERA_KERNELS: &[built_python_bindings::Entry] = &[
    ("_pixel_to_ray", "view_ray", &["self", "x", "y"], "", ""),
    ("_world_to_pixel", "view_pixel", &["self", "world", "mvp=None"], "", ""),
    ("_zoom_to_box", "zoom_box", &["self", "start:QPointF", "end:QPointF"], "None", ""),
    ("_project_to_lock_line", "lock_line", &["self", "start:QVector3D", "lock_dir:QVector3D", "pixel_x:float", "pixel_y:float"], "QVector3D", ""),
    ("_depth_of", "view_depth", &["self", "point"], "Optional[float]", ""),
    ("_recentre_on", "recentre", &["self", "x:float", "y:float"], "None", ""),
    ("_point_at_centre_depth", "centre_depth", &["self", "x:float", "y:float"], "", ""),
    ("_look_move", "look_move", &["self", "ev"], "None", ""),
    ("_world_under_cursor", "cursor_world", &["self", "x:float", "y:float"], "Optional[QVector3D]", ""),
    ("_zoom_at", "cursor_zoom", &["self", "steps:float", "pos"], "", ""),
    ("_zoom_out_step", "retreat_step", &["self"], "float", ""),
];

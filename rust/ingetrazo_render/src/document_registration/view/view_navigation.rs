// SPDX-License-Identifier: GPL-3.0-or-later
//! Host bindings for view_navigation.
use super::*;
fn view_navigation_pixel_to_ray<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let view = a.get_item(0)?;
    let x = a.get_item(1)?;
    let y = a.get_item(2)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("view_ray", (&view, &x, &y))
}

fn view_navigation_world_to_pixel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let mvp = a.get_item(2)?;
    let view = a.get_item(0)?;
    let world = a.get_item(1)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("view_pixel", (&view, &world, &mvp))
}

fn view_navigation_zoom_to_box<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let end = a.get_item(2)?;
    let view = a.get_item(0)?;
    let start = a.get_item(1)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("zoom_box", (&view, &start, &end))
}

fn view_navigation_project_to_lock_line<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let lock_dir = a.get_item(2)?;
    let pixel_x = a.get_item(3)?;
    let pixel_y = a.get_item(4)?;
    let view = a.get_item(0)?;
    let start = a.get_item(1)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("lock_line", (&view, &start, &lock_dir, &pixel_x, &pixel_y))
}

fn view_navigation_depth_of<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let point = a.get_item(1)?;
    let view = a.get_item(0)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("view_depth", (&view, &point))
}

fn view_navigation_recentre_on<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let view = a.get_item(0)?;
    let x = a.get_item(1)?;
    let y = a.get_item(2)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("recentre", (&view, &x, &y))
}

fn view_navigation_point_at_centre_depth<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let view = a.get_item(0)?;
    let x = a.get_item(1)?;
    let y = a.get_item(2)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("centre_depth", (&view, &x, &y))
}

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

fn view_navigation_look_move<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let ev = a.get_item(1)?;
    let view = a.get_item(0)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("look_move", (&view, &ev))
}

fn view_navigation_world_under_cursor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let view = a.get_item(0)?;
    let x = a.get_item(1)?;
    let y = a.get_item(2)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("cursor_world", (&view, &x, &y))
}

fn view_navigation_zoom_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let pos = a.get_item(2)?;
    let view = a.get_item(0)?;
    let steps = a.get_item(1)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("cursor_zoom", (&view, &steps, &pos))
}

fn view_navigation_zoom_out_step<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("views.view_navigation")?;
    let view = a.get_item(0)?;
    module
        .getattr("ingetrazo_render")?
        .call_method1("retreat_step", (&view,))
}

#[rustfmt::skip]
pub(super) const VIEW_NAVIGATION: &[Method] = &[
    ("_pixel_to_ray", Mode::Method, view_navigation_pixel_to_ray, &["self", "x", "y"], "", ""),
    ("_world_to_pixel", Mode::Method, view_navigation_world_to_pixel, &["self", "world", "mvp=None"], "", ""),
    ("_zoom_to_box", Mode::Method, view_navigation_zoom_to_box, &["self", "start:QPointF", "end:QPointF"], "None", ""),
    ("_project_to_lock_line", Mode::Method, view_navigation_project_to_lock_line, &["self", "start:QVector3D", "lock_dir:QVector3D", "pixel_x:float", "pixel_y:float"], "QVector3D", ""),
    ("_depth_of", Mode::Method, view_navigation_depth_of, &["self", "point"], "Optional[float]", ""),
    ("_recentre_on", Mode::Method, view_navigation_recentre_on, &["self", "x:float", "y:float"], "None", ""),
    ("_point_at_centre_depth", Mode::Method, view_navigation_point_at_centre_depth, &["self", "x:float", "y:float"], "", ""),
    ("_begin_look", Mode::Method, view_navigation_begin_look, &["self", "ev"], "None", ""),
    ("_look_move", Mode::Method, view_navigation_look_move, &["self", "ev"], "None", ""),
    ("_world_under_cursor", Mode::Method, view_navigation_world_under_cursor, &["self", "x:float", "y:float"], "Optional[QVector3D]", ""),
    ("_zoom_at", Mode::Method, view_navigation_zoom_at, &["self", "steps:float", "pos"], "", ""),
    ("_zoom_out_step", Mode::Method, view_navigation_zoom_out_step, &["self"], "float", ""),
];

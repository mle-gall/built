// SPDX-License-Identifier: GPL-3.0-or-later
//! `MetalView`: the pointer, the keys and the wheel as the tools hear them,
//! the picks a rectangle or a ray asks, and the close.
use super::metal_view::{attr, base, camera_frame, ctrl_tap, tool_frame, truthy};
use super::*;

fn input<'py>(py: Python<'py>, name: &str) -> Reply<'py> {
    py.import("built_core")?.getattr("Input")?.getattr(name)
}
fn is_input(value: &Obj<'_>, name: &str) -> PyResult<bool> {
    value.eq(input(value.py(), name)?)
}
fn has_input(value: &Obj<'_>, name: &str) -> PyResult<bool> {
    value.bitand(input(value.py(), name)?)?.is_truthy()
}
fn one_of(value: &Obj<'_>, names: &[&str]) -> PyResult<bool> {
    for name in names {
        if value.eq(*name)? {
            return Ok(true);
        }
    }
    Ok(false)
}
const DRAWING: &[&str] = &["tape", "move", "mesh"];
fn float<'py>(value: Obj<'py>) -> Reply<'py> {
    Ok(value.py().get_type::<pyo3::types::PyFloat>().call1((value,))?)
}
/// `(point.x(), point.y())`.
fn xy<'py>(point: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    Ok((point.call_method0("x")?, point.call_method0("y")?))
}
fn nav_mode<'py>(view: &Obj<'py>) -> Reply<'py> {
    attr(&attr(view, "source_viewport")?, "nav_mode")
}
fn empty<'py>(py: Python<'py>) -> Bound<'py, PyList> {
    PyList::empty(py)
}
fn framed<'py>(a: &Args<'py>, name: &str, body: fn(&Obj<'py>, &Obj<'py>) -> Reply<'py>) -> Reply<'py> {
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    tool_frame(&view, name, Some(&event), || body(&view, &event))
}
fn hover_at<'py>(view: &Obj<'py>, point: &Obj<'py>, event: &Obj<'py>) -> PyResult<()> {
    let (x, y) = xy(point)?;
    view.call_method1("hover", (x, y, event.call_method0("modifiers")?))?;
    Ok(())
}
fn notify_scene<'py>(view: &Obj<'py>) -> PyResult<()> {
    let source = view.getattr("source_viewport")?;
    if !source.is_none() {
        source.call_method0("notify_scene_changed")?;
    }
    Ok(())
}

fn mouse_press<'py>(a: &Args<'py>) -> Reply<'py> {
    framed(a, "mousePressEvent", |view, event| {
        let py = view.py();
        view.call_method0("setFocus")?;
        let press = event.call_method0("position")?;
        view.setattr("last", &press)?;
        view.setattr("press", &press)?;
        let button = event.call_method0("button")?;
        let drag_nav = is_input(&button, "MiddleButton")? || nav_mode(view)?.is_truthy()?;
        view.setattr("_drag_nav", drag_nav)?;
        let tools = view.getattr("tools")?;
        let mode = tools.call_method0("sync")?;
        let left = is_input(&button, "LeftButton")?;
        let (x, y) = xy(&press)?;
        // A junction review's badges take their clicks before any tool.
        if !drag_nav && left {
            let alt = has_input(&event.call_method0("modifiers")?, "AltModifier")?;
            if crate::junction_review::press(view, x.extract()?, y.extract()?, alt)? {
                view.setattr("last", py.None())?;
                view.call_method0("update")?;
                return Ok(none(py));
            }
        }
        if !drag_nav && mode.eq("select")? && left {
            // The Select machine owns the gesture: a press in place right
            // after a double click is the triple click, taken at once.
            let tool = tools.call_method0("select_tool")?;
            tool.call_method1("press", (&tools, &x, &y, event.call_method0("modifiers")?, float(event.call_method0("timestamp")?)?))?;
            if !tool.call_method0("pressed")?.is_truthy()? {
                view.setattr("last", py.None())?; // the release that follows is no click
                notify_scene(view)?;
                view.call_method0("sync_selection")?;
                view.call_method0("update")?;
                return Ok(none(py));
            }
        }
        if !drag_nav && attr(&tools.getattr("tool")?, "on_look")?.is_callable() && (left || is_input(&button, "RightButton")?) {
            view.call_method1("_begin_look", (event,))?;
            return Ok(none(py));
        }
        if !drag_nav && left && one_of(&mode, DRAWING)? {
            hover_at(view, &press, event)?;
            tools.call_method1("click", (event,))?;
        }
        if view.getattr("_drag_nav")?.is_truthy()? {
            view.setattr("_pivot", view.call_method1("_orbit_pivot_at", (x, y))?)?;
        }
        Ok(none(py))
    })
}

fn mouse_move<'py>(a: &Args<'py>) -> Reply<'py> {
    framed(a, "mouseMoveEvent", |view, event| {
        let py = view.py();
        let tools = view.getattr("tools")?;
        if !attr(view, "_look_drag")?.is_none() {
            if event.call_method0("buttons")?.is_truthy()? {
                view.call_method1("_look_move", (event,))?;
                return Ok(none(py));
            }
            view.setattr("_look_drag", py.None())?;
            tools.call_method0("_apply_tool_cursor")?;
        }
        let position = event.call_method0("position")?;
        view.setattr("_cursor_pos", &position)?;
        let buttons = event.call_method0("buttons")?;
        if !buttons.is_truthy()? {
            view.setattr("last", py.None())?;
            hover_at(view, &position, event)?;
            tools.call_method1("hover", (event,))?;
            return Ok(none(py));
        }
        let last = view.getattr("last")?;
        if last.is_none() {
            return Ok(none(py));
        }
        let delta = position.call_method0("toPoint")?.sub(last.call_method0("toPoint")?)?;
        view.setattr("last", &position)?;
        let mode = nav_mode(view)?;
        let zoom_window = mode.eq("zoom_window")?;
        let overlay = || py.import("views.wgpu_overlay");
        if view.getattr("_drag_nav")?.is_truthy()? && !zoom_window {
            let shift = has_input(&event.call_method0("modifiers")?, "ShiftModifier")?;
            py.import("built_render")?.call_method1("camera_drag", (view, &mode, delta, shift))?;
        } else if one_of(&tools.call_method0("sync")?, DRAWING)? {
            hover_at(view, &position, event)?;
            tools.call_method1("hover", (event,))?;
        } else if has_input(&buttons, "LeftButton")? && zoom_window {
            let rectangle = overlay()?.call_method("selection_rectangle", (xy(&view.getattr("press")?)?, xy(&position)?), Some(&fields!(py; "zoom" => true)))?;
            view.setattr("overlay_primitives", PyList::new(py, [rectangle])?)?;
        } else if has_input(&buttons, "LeftButton")? && tools.getattr("mode")?.eq("select")? {
            // The machine shows a box only past its drag threshold (a click
            // that jitters a pixel is no box).
            let (x, y) = xy(&position)?;
            let marquee = tools.call_method0("select_tool")?.call_method1("drag", (&tools, x, y, event.call_method0("modifiers")?))?;
            let shown = empty(py);
            if !marquee.is_none() {
                let ends = marquee.get_item(0)?;
                let (from, to) = (ends.get_item(pyo3::types::PySlice::new(py, 0, 2, 1))?, ends.get_item(pyo3::types::PySlice::new(py, 2, isize::MAX, 1))?);
                shown.append(overlay()?.call_method("selection_rectangle", (from, to), Some(&fields!(py; "zoom" => false)))?)?;
            }
            view.setattr("overlay_primitives", shown)?;
        }
        view.call_method0("update")?;
        Ok(none(py))
    })
}

fn active_tool<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.getattr("tools")?.getattr("tool")
}

fn focus_out<'py>(a: &Args<'py>) -> Reply<'py> {
    framed(a, "focusOutEvent", |view, event| {
        // As GL's: the walking keys' releases will land elsewhere, but the
        // tool goes on. Cancelling it here reset a line whose first point
        // was down as soon as a tray, a field or a dialog took the focus.
        view.setattr("_look_drag", view.py().None())?;
        let tools = view.getattr("tools")?;
        let hook = attr(&tools.getattr("tool")?, "on_focus_out")?;
        if !hook.is_none() {
            hook.call1((&tools,))?;
        }
        base(view)?.call_method1("focusOutEvent", (event,))?;
        Ok(none(view.py()))
    })
}

/// Tab is a tool key here (reserved for the tools, views.shortcuts): the
/// focus chain took it and left the keyboard on a tray button.
fn focus_next_prev_child<'py>(a: &Args<'py>) -> Reply<'py> {
    false.into_bound_py_any(a.py())
}

fn leave<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = a.get_item(0)?;
    a.py().import("views.wgpu_ui")?.call_method1("pointer", (&view, "leave"))?;
    base(&view)?.call_method1("leaveEvent", (a.get_item(1)?,))?;
    Ok(none(a.py()))
}

fn mouse_release<'py>(a: &Args<'py>) -> Reply<'py> {
    framed(a, "mouseReleaseEvent", |view, event| {
        let py = view.py();
        let tools = view.getattr("tools")?;
        if !attr(view, "_look_drag")?.is_none() {
            view.setattr("_look_drag", py.None())?;
            tools.call_method0("_apply_tool_cursor")?;
            view.setattr("last", py.None())?;
            return Ok(none(py));
        }
        if view.getattr("last")?.is_none() {
            return Ok(none(py));
        }
        let mode = nav_mode(view)?;
        let zoom_window = mode.eq("zoom_window")?;
        let left = is_input(&event.call_method0("button")?, "LeftButton")?;
        if view.getattr("_drag_nav")?.is_truthy()? {
            if zoom_window {
                view.call_method1("_zoom_to_box", (view.getattr("press")?, event.call_method0("position")?))?;
            }
        } else if left && tools.call_method0("sync")?.eq("select")? {
            // A release in place is a click, else the box (crossing when
            // dragged right to left): the machine decides.
            let (x, y) = xy(&event.call_method0("position")?)?;
            tools.call_method0("select_tool")?.call_method1("release", (&tools, x, y, event.call_method0("modifiers")?))?;
            notify_scene(view)?;
            view.call_method0("sync_selection")?;
        }
        if left && !view.getattr("_drag_nav")?.is_truthy()? {
            let tool=tools.getattr("tool")?;
            if tool.get_type().name()?.eq("CutTool") && tool.call_method1("_state",("editing_grip",))?.is_truthy()? {
                let (a,b)=xy(&view.getattr("press")?)?;
                let (x,y)=xy(&event.call_method0("position")?)?;
                if (a.extract::<f64>()?-x.extract::<f64>()?).hypot(b.extract::<f64>()?-y.extract::<f64>()?)>4. {tools.call_method1("click",(event,))?;}
            }
        }
        if tools.getattr("mode")?.eq("mesh")? && left && !view.getattr("_drag_nav")?.is_truthy()? {
            // GL's: a left release only. A wheel-button orbit's release
            // committed a Push/Pull in progress (a release far from its
            // press is a stroke) and set Rotate's axis along the orbit.
            tools.getattr("tool")?.call_method1("on_release", (&tools,))?;
            tools.call_method0("update")?;
            view.setattr("overlay_primitives", empty(py))?;
            tools.call_method0("mesh_feedback")?;
        }
        view.setattr("last", py.None())?;
        if tools.getattr("mode")?.eq("select")? || zoom_window {
            view.setattr("overlay_primitives", empty(py))?;
        }
        view.call_method0("update")?;
        Ok(none(py))
    })
}

fn context_menu<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    // The sheet's own menus.
    if py.import("built_render.sheet_workspace")?.call_method1("context_menu", (&view, &event))?.is_truthy()? {
        return Ok(none(py));
    }
    let point = event.call_method0("pos")?;
    let (x, y) = xy(&point)?;
    let chrome = py.import("built_render")?.call_method1("context_menu_available", (&view,))?.is_truthy()?;
    if chrome && view.getattr("renderer")?.call_method1("ui_over_chrome", (float(x.clone())?, float(y.clone())?))?.is_truthy()? {
        return Ok(none(py));
    }
    let tools = view.getattr("tools")?;
    tools.call_method0("sync")?;
    let menu = attr(&tools.getattr("tool")?, "context_menu")?;
    if !menu.is_none() && menu.call1((&tools, event.call_method0("globalPos")?))?.is_truthy()? {
        return Ok(none(py));
    }
    let source = view.getattr("source_viewport")?;
    if source.is_none() {
        return Ok(none(py));
    }
    // With the chrome on (the native app, or the Qt host), the menu is the
    // chrome's: only the verbs for what was hit (docs/context-menu.md). A
    // right-click on the chrome itself is the chrome's own.
    if !chrome {
        source.call_method1("flash_status", ("Built could not draw the context menu.",))?;
        return Ok(none(py));
    }
    py.import("plugins._context_menu")?.call_method1("context_menu_open_at", (&view, x, y))?;
    Ok(none(py))
}

fn mouse_double_click<'py>(a: &Args<'py>) -> Reply<'py> {
    framed(a, "mouseDoubleClickEvent", |view, event| {
        let py = view.py();
        let button = event.call_method0("button")?;
        let position = event.call_method0("position")?;
        // A double click of the wheel recentres the view on the point under
        // the cursor (on sky: at the depth of the view's centre); the tool
        // in use never hears of it and no orbit starts, as in GL (#979,
        // #1154).
        if is_input(&button, "MiddleButton")? {
            view.setattr("last", py.None())?;
            view.call_method1("_recentre_on", xy(&position)?)?;
            return Ok(none(py));
        }
        if is_input(&button, "LeftButton")? && nav_mode(view)?.is_truthy()? {
            return view.call_method1("mousePressEvent", (event,));
        }
        let tools = view.getattr("tools")?;
        let mode = tools.call_method0("sync")?;
        if one_of(&mode, DRAWING)? {
            hover_at(view, &position, event)?;
            if mode.eq("mesh")? {
                let context = tools.call_method1("context", (event,))?;
                if !context.is_none() {
                    tools.getattr("tool")?.call_method1("on_double_click", (context,))?;
                    tools.call_method0("update")?;
                }
            } else {
                tools.call_method1("click", (event,))?;
            }
            return Ok(none(py));
        }
        if mode.ne("select")? {
            return Ok(none(py));
        }
        let (x, y) = xy(&position)?;
        tools.call_method0("select_tool")?.call_method1("double", (&tools, x, y, event.call_method0("modifiers")?, float(event.call_method0("timestamp")?)?))?;
        view.call_method0("update")?;
        Ok(none(py))
    })
}

fn event<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    let kinds = py.import("built_core")?.getattr("Type")?;
    let kind = event.call_method0("type")?;
    let taken = || -> Reply<'py> {
        event.call_method0("accept")?;
        true.into_bound_py_any(py)
    };
    if kind.eq(kinds.getattr("NativeGesture")?)? {
        // Pinch on the sheet; then pinch and smart zoom: a trackpad's, or
        // the touch screen's (#248).
        if py.import("built_render.sheet_workspace")?.call_method1("gesture", (&view, &event))?.is_truthy()?
            || camera_frame(&view, "event", None, || {
                py.import("built_render")?.getattr("trackpad_nav")?.call_method1("native_gesture", (&view, &event))
            })?.is_truthy()?
        {
            return true.into_bound_py_any(py);
        }
    }
    if kind.eq(kinds.getattr("ShortcutOverride")?)? {
        ctrl_tap(&view, &event)?;
        if direct(py, super::chrome_input::claims_key, (&view, &event))?.is_truthy()? {
            return taken();
        }
        if view.hasattr("tools")? {
            let tools = view.getattr("tools")?;
            let mesh = tools.call_method0("sync")?.eq("mesh")?;
            let target = if mesh { tools.clone() } else { view.getattr("source_viewport")? };
            if !target.is_none() {
                let buffer = if mesh { tools.getattr("value_buffer")? } else { target.getattr("_value_buffer")? };
                let interaction = py.import("views.document_interaction")?.getattr("DocumentInteraction")?;
                if interaction.call_method1("_measurement_claims_key", (&target, &event, buffer))?.is_truthy()? {
                    return taken();
                }
            }
            let claims = attr(&tools.getattr("tool")?, "claims_key")?;
            if !claims.is_none() && claims.call1((event.call_method0("key")?, event.call_method0("modifiers")?))?.is_truthy()? {
                return taken();
            }
        }
    }
    base(&view)?.call_method1("event", (&event,))
}

fn key_press<'py>(a: &Args<'py>) -> Reply<'py> {
    framed(a, "keyPressEvent", |view, event| {
        let py = view.py();
        ctrl_tap(view, event)?;
        let tools = view.getattr("tools")?;
        let mode = tools.call_method0("sync")?;
        let key = event.call_method0("key")?;
        let source = view.getattr("source_viewport")?;
        // An open junction review: Enter applies the ticked, Esc leaves.
        let enter = is_input(&key, "Key_Return")? || is_input(&key, "Key_Enter")?;
        let escape = is_input(&key, "Key_Escape")?;
        if (enter && !tools.getattr("value_buffer")?.is_truthy()?)
            || escape
        {
            if crate::junction_review::key(view, enter, escape)? {
                view.call_method0("update")?;
                return Ok(none(py));
            }
        }
        if is_input(&key, "Key_Escape")? {
            let render = py.import("built_render")?;
            let interaction = py.import("views.document_interaction")?.getattr("DocumentInteraction")?;
            let scene = tools.getattr("scene")?;
            let busy = truthy(view, "_look_drag")?
                || !tools.getattr("start_point")?.is_none()
                || interaction.call_method1("_tool_busy_for_delete", (tools.getattr("tool")?,))?.is_truthy()?;
            let chrome = view.getattr_opt("_shared")?.unwrap_or_else(|| view.clone());
            let renderer = if truthy(&chrome, "_ui_attached")? {
                render.getattr("shell_views")?.call_method1("raw_renderer", (chrome.getattr("renderer")?,))?
            } else {
                none(py)
            };
            let owner = render.call_method1(
                "model_escape",
                (
                    busy,
                    !scene.is_none() && scene.getattr("selection")?.is_truthy()?,
                    !source.is_none() && !scene.getattr("edit_group")?.is_none(),
                    renderer,
                    tools.call_method0("_array_pending")?,
                    tools.getattr("value_buffer")?.is_truthy()?,
                    !attr(&source, "axis_lock")?.is_none() || !attr(&source, "reference_mode")?.is_none(),
                ),
            )?;
            view.setattr("_look_drag", py.None())?;
            let window = if view.hasattr("window")? { view.call_method0("window")? } else { none(py) };
            if !source.is_none() && attr(&window, "viewport")?.is(&source) && render.call_method1("leave_navigation", (&window,))?.is_truthy()? {
                return Ok(none(py));
            }
            tools.call_method0("_apply_tool_cursor")?;
            if owner.eq("value")? {
                tools.call_method1("_set_value_buffer", ("",))?;
            } else if owner.eq("constraint")? {
                source.call_method0("release_constraints")?;
            } else if owner.eq("tool")? {
                tools.call_method0("cancel")?;
            } else if owner.eq("array")? {
                tools.getattr("tool")?.call_method0("close_array_window")?;
            } else if owner.eq("edit")? {
                source.call_method0("end_one_group_edit")?;
            } else if owner.eq("selection")? {
                scene.call_method0("clear_selection")?;
                if !source.is_none() {
                    source.call_method0("notify_scene_changed")?;
                }
            }
            if one_of(&owner, &["edit", "selection"])? {
                view.setattr("last", py.None())?;
                view.setattr("overlay_primitives", empty(py))?;
                view.call_method0("sync_selection")?;
            }
        } else if tools.call_method1("key", (event,))?.is_truthy()? {
        } else if is_input(&key, "Key_Down")? && one_of(&mode, &["tape", "mesh"])? {
            view.call_method1("_down_reference", (event,))?;
        } else if mode.eq("inert")? {
            event.call_method0("ignore")?;
        } else if !source.is_none() {
            source.call_method1("keyPressEvent", (event,))?;
        }
        view.call_method0("update")?;
        Ok(none(py))
    })
}

/// The Down arrow no tool took: GL's parallel → perpendicular → free cycle
/// on the edge under the cursor, picked here (GL's hover edge is fed by
/// GL's own picks only, so Down found nothing on this view), else on the
/// last edge the line referenced.
fn down_reference<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    let source = view.getattr("source_viewport")?;
    if source.is_none() || event.call_method0("isAutoRepeat")?.is_truthy()? {
        return Ok(none(py));
    }
    let tools = view.getattr("tools")?;
    let position = attr(&view, "_cursor_pos")?;
    let mut edge = none(py);
    if !position.is_none() && source.getattr("reference_mode")?.is_none() {
        edge = tools.call_method1("pick_edge_any", xy(&position)?)?;
    }
    // GL's Down press: what it locks outlives a held Shift's plane mark.
    source.setattr("_shift_plane_mark", py.None())?;
    source.call_method1("_cycle_reference_mode", (edge,))?;
    if !position.is_none() {
        // The lock shows at once, the hand still (GL's _refresh_snap).
        let held = py.import("built_core")?.getattr("window_input")?.call_method1("modifiers", (&view,))?;
        let (x, y) = xy(&position)?;
        view.call_method1("hover", (&x, &y, &held))?;
        let bits = held.getattr_opt("value")?.unwrap_or_else(|| held.clone()).call_method0("__int__")?;
        let moved = py.import("built_render")?.getattr("MouseEvent")?.call(("move", x, y), Some(&fields!(py; "modifiers" => bits)))?;
        tools.call_method1("hover", (moved,))?;
    }
    Ok(none(py))
}

fn key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    framed(a, "keyReleaseEvent", |view, event| {
        let py = view.py();
        let tools = view.getattr("tools")?;
        let mode = tools.call_method0("sync")?;
        let tool = tools.getattr("tool")?;
        let release = attr(&tool, "on_key_release")?;
        let mesh = mode.eq("mesh")?;
        if mesh && event.call_method0("isAutoRepeat")?.is_truthy()? && attr(&tool, "claims_key")?.is_callable() {
            return Ok(none(py));
        }
        if mesh && !release.is_none() && release.call1((&tools, event.call_method0("key")?))?.is_truthy()? {
            tools.call_method0("update")?;
            return Ok(none(py));
        }
        if mode.eq("inert")? {
            event.call_method0("ignore")?;
            return Ok(none(py));
        }
        let source = view.getattr("source_viewport")?;
        if !source.is_none() {
            source.call_method1("keyReleaseEvent", (event,))?;
        }
        Ok(none(py))
    })
}

fn depth_world_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, x, y) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let miss = || ("miss", py.None()).into_bound_py_any(py);
    let row = view.call_method1("pick_row", (&x, &y))?;
    if row.is_none() {
        return miss();
    }
    let inference = py.import("built_core")?;
    let [origin, direction] = unpack::<2>(&view.call_method1("_pixel_to_ray", (&x, &y))?)?;
    let mut point = none(py);
    let index = view.getattr("inference_index")?;
    if !index.is_none() {
        point = inference.call_method1("surface", (index, &row, &origin, &direction))?;
    }
    if point.is_none() {
        // No snap index yet (worker still building it, or dropped by an
        // edit): the picked placement's triangles answer on their own.
        point = inference.call_method1("packed_surface", (view.getattr("packed")?, &row, &origin, &direction))?;
    }
    if point.is_none() {
        return miss();
    }
    ("hit", point).into_bound_py_any(py)
}

/// The model point under pixel `(x, y)` from the Metal pick, or `None` on
/// sky or ground (GL's `DocumentInteraction._model_point_at`).
fn model_point_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let [seen, point] = unpack::<2>(&a.get_item(0)?.call_method1("_depth_world_at", (a.get_item(1)?, a.get_item(2)?))?)?;
    Ok(if seen.eq("hit")? { point } else { none(a.py()) })
}

fn orbit_pivot_at<'py>(a: &Args<'py>) -> Reply<'py> {
    a.py().import("built_render")?.call_method1("metal_pivot", (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?))
}

fn wheel<'py>(a: &Args<'py>) -> Reply<'py> {
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    camera_frame(&view, "wheelEvent", Some(&event), || {
        let (view, event) = (&view, &event);
        let py = view.py();
        view.setattr("_invert_wheel", truthy(&view.getattr("source_viewport")?, "_invert_wheel")?)?;
        py.import("views.document_interaction")?.getattr("DocumentInteraction")?.call_method1("wheelEvent", (view, event))?;
        view.call_method0("update")?;
        Ok(none(py))
    })
}

fn set_highlight<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let renderer = view.getattr("renderer")?;
    if renderer.is_none() {
        return Ok(none(py));
    }
    let uid_rows = view.getattr("uid_rows")?;
    let rows_of = |uid: Obj<'py>| uid_rows.call_method1("get", (uid, PyTuple::empty(py)));
    let indices = empty(py);
    for uid in a.get_item(1)?.try_iter()? {
        indices.call_method1("extend", (rows_of(uid?)?,))?;
    }
    renderer.call_method1("set_highlight", (indices, py.get_type::<PyList>().call1((rows_of(a.get_item(2)?)?,))?))?;
    view.call_method0("update")?;
    Ok(none(py))
}

fn rect_rows<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let rows = empty(py);
    if view.getattr("renderer")?.is_none() {
        return Ok(rows.into_any());
    }
    let scale: f64 = view.call_method0("devicePixelRatioF")?.extract()?;
    let corner = |at: usize| -> PyResult<f64> { a.get_item(at)?.extract() };
    let (x0, y0, x1, y1) = (corner(1)?, corner(2)?, corner(3)?, corner(4)?);
    let pixel = |value: f64| ((value * scale) as i64).max(0);
    view.call_method0("_prepare_pick_frame")?;
    let hit = view.getattr("renderer")?.call_method1("pick_rect", (pixel(x0.min(x1)), pixel(y0.min(y1)), pixel(x0.max(x1)), pixel(y0.max(y1))))?;
    for row in hit.try_iter()? {
        let row = row?;
        if !view.call_method1("resolve", (&row,))?.is_none() {
            rows.append(row)?;
        }
    }
    Ok(rows.into_any())
}

fn pick_rect<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let uids = view.getattr("packed")?.getattr("uids")?;
    let picked = empty(py);
    for row in view.call_method1("rect_rows", (a.get_item(1)?, a.get_item(2)?, a.get_item(3)?, a.get_item(4)?))?.try_iter()? {
        picked.append(uids.get_item(row?)?)?;
    }
    Ok(picked.into_any())
}

fn close<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, event) = (a.get_item(0)?, a.get_item(1)?);
    view.setattr("_closed", true)?;
    for name in ["_refresh_timer", "_repaint_timer", "_resize_timer", "_settle_timer"] {
        let timer = attr(&view, name)?;
        if !timer.is_none() {
            timer.call_method0("stop")?;
        }
    }
    let tracker = attr(&view, "_instance_updates")?;
    if !tracker.is_none() {
        let unsubscribe = tracker.getattr("unsubscribe")?;
        if !unsubscribe.is_none() {
            unsubscribe.call0()?;
        }
    }
    view.setattr("renderer", py.None())?;
    if let Some(worker) = view.getattr_opt("_inference_worker")? {
        worker.call_method("shutdown", (), Some(&fields!(py; "wait" => false, "cancel_futures" => true)))?;
    }
    // A host's own close, when its base has one (a widget shim has none).
    if let Some(close) = base(&view)?.getattr_opt("closeEvent")? {
        if !close.is_none() {
            close.call1((event,))?;
        }
    }
    for name in [
        "_instance_updates", "_inference_future", "inference_index", "source_viewport", "_tool_source", "scene", "packed",
        "_billboards", "_reference_cache",
    ] {
        view.setattr(name, py.None())?;
    }
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("mousePressEvent", Mode::Method, mouse_press, &["self", "event"], "", ""),
    ("mouseMoveEvent", Mode::Method, mouse_move, &["self", "event"], "", ""),
    ("active_tool", Mode::Property, active_tool, &["self"], "", ""),
    ("focusOutEvent", Mode::Method, focus_out, &["self", "event"], "", ""),
    ("focusNextPrevChild", Mode::Method, focus_next_prev_child, &["self", "next"], "", ""),
    ("leaveEvent", Mode::Method, leave, &["self", "event"], "", ""),
    ("mouseReleaseEvent", Mode::Method, mouse_release, &["self", "event"], "", ""),
    ("contextMenuEvent", Mode::Method, context_menu, &["self", "event"], "", ""),
    ("mouseDoubleClickEvent", Mode::Method, mouse_double_click, &["self", "event"], "", ""),
    ("event", Mode::Method, event, &["self", "event"], "", ""),
    ("keyPressEvent", Mode::Method, key_press, &["self", "event"], "", ""),
    ("_down_reference", Mode::Method, down_reference, &["self", "event"], "", ""),
    ("keyReleaseEvent", Mode::Method, key_release, &["self", "event"], "", ""),
    ("_depth_world_at", Mode::Method, depth_world_at, &["self", "x", "y"], "", ""),
    ("_model_point_at", Mode::Method, model_point_at, &["self", "x", "y"], "", ""),
    ("_orbit_pivot_at", Mode::Method, orbit_pivot_at, &["self", "x", "y"], "", ""),
    ("wheelEvent", Mode::Method, wheel, &["self", "event"], "", ""),
    ("set_highlight", Mode::Method, set_highlight, &["self", "selected_ids", "hovered_id=None"], "", ""),
    ("rect_rows", Mode::Method, rect_rows, &["self", "x0", "y0", "x1", "y1"], "", ""),
    ("pick_rect", Mode::Method, pick_rect, &["self", "x0", "y0", "x1", "y1"], "", ""),
    ("closeEvent", Mode::Method, close, &["self", "event"], "", ""),
];

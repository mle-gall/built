// SPDX-License-Identifier: GPL-3.0-or-later
//! EyeHost callbacks on the live document.
use super::*;

fn drag_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let viewport = a.get_item(1)?;
    (
        module.call_method1("Vec3", (viewport.getattr("camera")?.getattr("target")?,))?,
        viewport.getattr("camera")?.call_method0("forward")?.neg()?,
    )
        .into_bound_py_any(py)
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let tool = a.get_item(0)?;
    if !tool.getattr("_viewport")?.is_none() {
        return (
            module.call_method1(
                "fmt_len",
                (tool
                    .getattr("_viewport")?
                    .getattr("camera")?
                    .call_method0("eye")?
                    .call_method0("z")?,),
            )?,
            py.None(),
        )
            .into_bound_py_any(py);
    }
    Ok(none(py))
}
fn host_eye_height<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let walkthrough = imported(py, "tools.walkthrough")?.into_any();
    walkthrough.call_method0("eye_height")
}
fn host_ray_distance<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let arguments = a.get_item(2)?;
    let at: Obj<'py>;
    let direction: Obj<'py>;
    let reach: Obj<'py>;
    let viewport = a.get_item(1)?;
    let ray = optional(&viewport, "ray_distance")?;
    if ray.is_none() {
        return Ok(none(py));
    }
    [at, direction, reach] = unpack::<3>(&arguments)?;
    if reach.is_none() || module.getattr("_NO_WALK_REACH")?.is_truthy()? {
        return ray.call1((
            module.call_method1("Vec3", positional(&at)?)?,
            module.call_method1("Vec3", positional(&direction)?)?,
        ));
    }
    ray.call1((
        module.call_method1("Vec3", positional(&at)?)?,
        module.call_method1("Vec3", positional(&direction)?)?,
        &reach,
    ))
}
fn tick<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let tool = a.get_item(0)?;
    let input_state = imported(py, "built_core.window_input")?.into_any();
    let vp = tool.getattr("_viewport")?;
    if vp.is_none() {
        return Ok(none(py));
    }
    let reply = tool.call_method1("_send", (&vp, fields!(py; "type" => "camera_tick", "dt" => tool.getattr("TICK_MS")?.div(1000)?, "mods" => module.call_method1("_mods", (input_state.call_method1("modifiers", (&vp,))?,))?).into_any(),))?;
    if reply.get_item("redraw")?.is_truthy()? {
        if vp.hasattr("measurementChanged")? && vp.hasattr("_measurement_text")? {
            vp.getattr("measurementChanged")?
                .call_method1("emit", (vp.call_method0("_measurement_text")?,))?;
        }
        vp.call_method0("update")?;
    }
    Ok(none(py))
}

fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr("_viewport", &viewport)?;
    imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("EyeHost")?, &tool))?
        .call_method1("on_activate", (&viewport,))?;
    Ok(none(py))
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("EyeHost")?, &tool))?
        .call_method1("on_deactivate", (&viewport,))?;
    tool.setattr("_viewport", none(py))?;
    Ok(none(py))
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    if !(imported(py, "builtins")?
        .getattr("isinstance")?
        .call1((
            &value,
            (
                py.get_type::<pyo3::types::PyInt>().into_any(),
                py.get_type::<pyo3::types::PyFloat>().into_any(),
            )
                .into_bound_py_any(py)?,
        ))?
        .is_truthy()?)
    {
        return false.into_bound_py_any(py);
    }
    let accepted = imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("EyeHost")?, &tool))?
        .call_method1("on_value", (&viewport, &value))?;
    if accepted.is_truthy()? {
        viewport.call_method0("update")?;
    }
    Ok(accepted.clone())
}
fn on_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let activate: Obj<'py>;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let window: Obj<'py>;
    tool.setattr("_handoff", none(py))?;
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => "release").into_any()),
    )?;
    if tool.getattr("_handoff")?.is_truthy()? {
        window = if let Some(call) = viewport.getattr_opt("window")? {
            call.call0()?
        } else {
            none(py)
        };
        activate = optional(&window, "_activate_tool")?;
        if activate.is_truthy()? {
            activate.call1((tool.getattr("_handoff")?,))?;
        }
    }
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn host_set_camera<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let arguments = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    let set_camera = imported(py, "core.agent_inspection")?.getattr("set_camera")?;
    set_camera.call(
        (viewport.getattr("camera")?,),
        Some(arguments.cast::<PyDict>()?),
    )?;
    Ok(none(py))
}
fn host_timer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let active = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !(active.is_truthy()?) {
        if !tool.getattr("_timer")?.is_none() {
            tool.getattr("_timer")?.call_method0("stop")?;
        }
        return Ok(none(py));
    }
    if tool.getattr("_timer")?.is_none() {
        let timer = imported(py, "built_core.timers")?.getattr("timer")?;
        tool.setattr(
            "_timer",
            timer.call(
                (
                    viewport.getattr_opt("view")?.unwrap_or(viewport.clone()),
                    tool.getattr("_tick")?,
                    tool.getattr("TICK_MS")?,
                ),
                Some(&fields!(py; "repeat" => true)),
            )?,
        )?;
    }
    if !(tool
        .getattr("_timer")?
        .call_method0("isActive")?
        .is_truthy()?)
    {
        tool.getattr("_timer")?.call_method0("start")?;
    }
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "", ""),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "", ""),
    ("on_release", Mode::Method, on_release, &["self", "viewport"], "", ""),
    ("drag_plane", Mode::Method, drag_plane, &["self", "viewport"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("host_set_camera", Mode::Method, host_set_camera, &["self", "viewport", "args"], "", ""),
    ("host_eye_height", Mode::Method, host_eye_height, &["self", "viewport", "_"], "", ""),
    ("host_ray_distance", Mode::Method, host_ray_distance, &["self", "viewport", "args"], "", ""),
    ("host_timer", Mode::Method, host_timer, &["self", "viewport", "active"], "", ""),
    ("_tick", Mode::Method, tick, &["self"], "", ""),
];

// SPDX-License-Identifier: GPL-3.0-or-later
//! Commands shared by the Tools menu and the viewport axes menu.
use crate::document_host::{vec3, xyz};
use crate::settings_rows::{func, none, tr};
use glam::{DMat3, DQuat, DVec3, EulerRot};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

type Obj<'p> = Bound<'p, PyAny>;
fn frame(scene: &Obj<'_>) -> PyResult<[DVec3; 4]> {
    let basis = scene
        .py()
        .import("built_core.group")?
        .call_method1("frame_axes", (scene.getattr("drawing_frame")?,))?;
    Ok([
        xyz(&basis.get_item(0)?)?,
        xyz(&basis.get_item(1)?)?,
        xyz(&basis.get_item(2)?)?,
        xyz(&basis.get_item(3)?)?,
    ])
}
fn apply(vp: &Obj<'_>, origin: DVec3, x: DVec3, y: DVec3) -> PyResult<()> {
    let py = vp.py();
    let args = PyDict::new(py);
    args.set_item("placement", py.None())?;
    for (key, point) in [
        ("origin", origin),
        ("red", origin + x),
        ("green", origin + y),
    ] {
        args.set_item(key, point.to_array())?;
    }
    py.import("built_core.command_protocol")?.call_method(
        "run",
        (vp.getattr("history")?, "set_group_axes"),
        Some(&args),
    )?;
    vp.call_method0("notify_scene_changed")?;
    Ok(())
}

pub(crate) fn action(win: &Obj<'_>, name: &str) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let [o, x, y, z] = frame(&scene)?;
    match name {
        "_axes_reset" => apply(&vp, DVec3::ZERO, DVec3::X, DVec3::Y),
        "_axes_align_view" => {
            let auto = crate::native_view_actions::auto_perspective(py)?;
            crate::native_view_actions::glide_after(win, |view| {
                let cam = view.getattr("camera")?;
                let yaw = cam.getattr("yaw")?.extract()?;
                let pitch: f64 = cam.getattr("pitch")?.extract()?;
                if let Some((a, b)) = crate::context_edits::look_angles(z, &[o, o + x], yaw) {
                    cam.setattr("target", vec3(py, o)?)?;
                    cam.setattr("yaw", a)?;
                    cam.setattr("pitch", b)?;
                    cam.call_method1("align_current", (yaw, pitch, auto))?;
                    cam.setattr("roll_deg", 0.)?;
                    let up = xyz(&cam.call_method0("up_vector")?)?;
                    let forward = xyz(&cam.call_method0("forward")?)?;
                    let roll = up.cross(y).dot(forward).atan2(up.dot(y));
                    cam.setattr("roll_deg", roll.to_degrees())?;
                }
                Ok(())
            })?;
            Ok(())
        }
        "_axes_move" | "_axes_face" => {
            win.call_method1("_activate_tool", ("change_axes",))?;
            let tool = win.getattr("_tools")?.get_item("change_axes")?;
            tool.setattr("target", py.None())?;
            tool.setattr("align_face", name == "_axes_face")?;
            if name == "_axes_face" {
                vp.call_method1(
                    "flash_status",
                    (tr(py, "Pick a face to align the drawing axes")?,),
                )?;
            }
            Ok(())
        }
        _ => {
            let forms = py.import("built_render.chrome_forms")?;
            let fields = PyList::empty(py);
            let (rx, ry, rz) = DQuat::from_mat3(&DMat3::from_cols(x, y, z)).to_euler(EulerRot::XYZ);
            for (id, label, value, unit) in [
                ("x", "Origin X", o.x, "m"),
                ("y", "Origin Y", o.y, "m"),
                ("z", "Origin Z", o.z, "m"),
                ("rx", "Rotation X", rx.to_degrees(), "°"),
                ("ry", "Rotation Y", ry.to_degrees(), "°"),
                ("rz", "Rotation Z", rz.to_degrees(), "°"),
            ] {
                let kw = PyDict::new(py);
                kw.set_item("lo", -1e12)?;
                kw.set_item("hi", 1e12)?;
                kw.set_item("decimals", 6)?;
                kw.set_item("suffix", unit)?;
                fields.append(
                    forms
                        .getattr("Field")?
                        .call((id, "number", tr(py, label)?, value), Some(&kw))?,
                )?;
            }
            let (vp, scene, group) = (
                vp.unbind(),
                scene.clone().unbind(),
                scene.getattr("edit_group")?.unbind(),
            );
            let done = func(py, move |py, a| {
                let v = vp.bind(py);
                if a.get_item(0)?.eq("ok")?
                    && v.getattr("scene")?.is(scene.bind(py))
                    && scene.bind(py).getattr("edit_group")?.is(group.bind(py))
                {
                    let values = a.get_item(1)?;
                    let n = |key| values.get_item(key)?.extract::<f64>();
                    let q = DQuat::from_euler(
                        EulerRot::XYZ,
                        n("rx")?.to_radians(),
                        n("ry")?.to_radians(),
                        n("rz")?.to_radians(),
                    );
                    apply(
                        v,
                        DVec3::new(n("x")?, n("y")?, n("z")?),
                        q * DVec3::X,
                        q * DVec3::Y,
                    )?;
                }
                Ok(none(py))
            })?;
            let kw = PyDict::new(py);
            kw.set_item("on_done", done)?;
            let form = forms
                .getattr("Form")?
                .call((tr(py, "Place Axes")?, fields), Some(&kw))?;
            form.setattr("placement", "drawer")?;
            forms.call_method1("show", (win, form))?;
            Ok(())
        }
    }
}

pub(crate) fn hit(view: &Obj<'_>, scene: &Obj<'_>, x: f64, y: f64) -> PyResult<bool> {
    if !scene.hasattr("drawing_frame")? || !view.hasattr("_world_to_pixel")? {
        return Ok(false);
    }
    if !scene.getattr("show_axes")?.is_truthy()? && scene.getattr("edit_group")?.is_none() {
        return Ok(false);
    }
    let [o, i, j, k] = frame(scene)?;
    let shift = view
        .getattr_opt("packed")?
        .filter(|p| !p.is_none())
        .map(|p| p.getattr("origin")?.extract::<[f64; 3]>())
        .transpose()?
        .unwrap_or_default();
    let o = o - DVec3::from(shift);
    let project = |p| {
        view.call_method1("_world_to_pixel", (vec3(view.py(), p)?,))?
            .extract::<Option<(f64, f64)>>()
    };
    let Some((ox, oy)) = project(o)? else {
        return Ok(false);
    };
    for dir in [i, j, k] {
        if let Some((ex, ey)) = project(o + dir)? {
            let (dx, dy) = (ex - ox, ey - oy);
            let length = dx.hypot(dy);
            if length > 1e-6 && ((x - ox) * dy - (y - oy) * dx).abs() <= 5. * length {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

//! Apply view commands to the active pane of a native model window.
use super::model_edit::translated;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

/// How long a view command's camera move takes (UX pass 2, M03): 250 ms,
/// or none in a scripted run (`built --script`), whose frames must come out
/// the same every time.
pub(crate) fn camera_motion_seconds(scripted: bool) -> f64 {
    if scripted { 0. } else { 0.25 }
}

/// Is this a scripted run? The app sets `BUILT_SCRIPTED` for `--script`.
pub(crate) fn scripted() -> bool {
    std::env::var_os("BUILT_SCRIPTED").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Settings > Navigation > Auto perspective (on by default): an aligned view
/// shows in parallel until the next orbit.
pub(crate) fn auto_perspective(py: Python<'_>) -> PyResult<bool> {
    Ok(!matches!(
        crate::navigation::setting(py, "nav/auto_perspective", "1")?.to_ascii_lowercase().as_str(),
        "0" | "false"
    ))
}

/// Run `change` on the window's active camera, then ease the camera there
/// from where it was, when the window draws and the run is not scripted.
pub(crate) fn glide_after(
    win: &Bound<'_, PyAny>,
    change: impl FnOnce(&Bound<'_, PyAny>) -> PyResult<()>,
) -> PyResult<()> {
    let py = win.py();
    let vp = win.call_method0("_active_view")?;
    let camera = vp.getattr("camera")?;
    let pose = py.import("built_core")?.getattr("Pose")?;
    let before = pose.call_method1("of_camera", (&camera,))?;
    change(&vp)?;
    let seconds = camera_motion_seconds(scripted());
    let drawn = win
        .getattr_opt("view")?
        .and_then(|v| v.getattr_opt("renderer").ok().flatten())
        .is_some_and(|r| !r.is_none());
    if seconds > 0. && drawn {
        if let Some(player) = win.getattr_opt("scene_player")?.filter(|p| !p.is_none()) {
            player.call_method1("glide_camera", (before, seconds))?;
        }
    }
    Ok(())
}

/// Preserve the depth and framing of the detail under the centre of the active pane.
pub(crate) fn alignment_focus(vp: &Bound<'_, PyAny>) -> PyResult<()> {
    use crate::document_host::{vec3, xyz};
    let camera = vp.getattr("camera")?;
    let display = vp.getattr_opt("keys_stand_in")?.filter(|v| !v.is_none());
    let renderer = display.as_ref().map(|v| v.getattr_opt("renderer")).transpose()?.flatten()
        .filter(|r| r.hasattr("pick").unwrap_or(false));
    let model_point = |view: &Bound<'_, PyAny>, x, y| -> PyResult<Option<glam::DVec3>> {
        let point = view.call_method1("_model_point_at", (x, y))?;
        if point.is_none() { return Ok(None); }
        let mut world = xyz(&point)?;
        // The picking camera and packed triangles share a floating origin.
        if !view.getattr("camera")?.is(&camera) {
            let origin = view.getattr("packed")?.getattr("origin")?.extract()?;
            world += glam::DVec3::from_array(origin);
        }
        Ok(Some(world))
    };
    let mut focus = None;
    if let (Some(view), Some(_)) = (&display, &renderer) {
        let w: f64 = view.call_method0("width")?.extract()?;
        let h: f64 = view.call_method0("height")?.extract()?;
        focus = model_point(view, w * 0.5, h * 0.5)?;
    }
    if focus.is_none() {
        let bounds = vp.getattr("scene")?.call_method0("selection_bounds")?;
        let lo = bounds.get_item(0)?;
        if !lo.is_none() {
            focus = Some((xyz(&lo)? + xyz(&bounds.get_item(1)?)?) * 0.5);
        }
    }
    if focus.is_none() {
        if let (Some(view), Some(renderer)) = (&display, &renderer) {
            if let Some(read) = renderer.getattr_opt("visible_focus_pixel")? {
                if let Some((x, y)) = read.call0()?.extract::<Option<(f64, f64)>>()? {
                    let scale: f64 = view.call_method0("devicePixelRatioF")?.extract()?;
                    focus = model_point(view, (x + 0.5) / scale, (y + 0.5) / scale)?;
                }
            }
        }
    }
    if let Some(point) = focus.filter(|p| p.is_finite()) {
        if camera.getattr("perspective")?.is_truthy()? {
            // Both projections use distance * tan(vertical FOV / 2) as their
            // half-height at the focal plane. Rebase distance to the picked depth.
            let eye = xyz(&camera.call_method0("eye")?)?;
            let depth = (point - eye).dot(xyz(&camera.call_method0("forward")?)?);
            if depth.is_finite() && depth > 0. {
                camera.setattr("distance", depth)?;
            }
        }
        camera.setattr("target", vec3(vp.py(), point)?)?;
    }
    Ok(())
}

/// Top, Front, … from the gizmo, ⌘1–7 or the View menu.
#[pyfunction]
fn standard_view(win: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
    let auto = auto_perspective(win.py())?;
    glide_after(win, |vp| {
        alignment_focus(vp)?;
        vp.getattr("camera")?.call_method1("align_to", (key, auto))?;
        Ok(())
    })?;
    let vp = win.call_method0("_active_view")?;
    if vp.hasattr("_sync_projection")? {
        vp.call_method0("_sync_projection")?;
    }
    vp.call_method0("update")?;
    Ok(())
}

#[pyfunction]
fn display_style(win: &Bound<'_, PyAny>, style: &Bound<'_, PyAny>) -> PyResult<()> {
    win.call_method1("set_active_style", (style.call_method0("copy")?,))?;
    let py = win.py();
    let vars = PyDict::new(py);
    vars.set_item(
        "name",
        translated(py, &style.getattr("name")?.extract::<String>()?, None)?,
    )?;
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (translated(py, "Style: {name}", Some(&vars))?, 2000),
    )?;
    Ok(())
}

#[pyfunction]
fn toggle_xray(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let view = win.call_method0("_active_view")?;
    let scene = view.getattr("scene")?;
    let style = win.call_method0("active_style")?;
    let styles = py.import("built_core.style")?;
    let restore = view
        .getattr("_style_before_xray")
        .ok()
        .filter(|v| !v.is_none());
    let target = if !style.is_none() && style.getattr("face_mode")?.eq("xray")? {
        view.setattr("_style_before_xray", py.None())?;
        if let Some(saved) = restore.filter(|v| {
            v.get_item(0).is_ok_and(|s| s.is(&scene)) && v.get_item(1).is_ok_and(|s| !s.is_none())
        }) {
            saved.get_item(1)?
        } else {
            styles.call_method1("style_by_name", ("Default",))?
        }
    } else {
        let saved = if style.is_none() {
            None
        } else {
            Some(style.call_method0("copy")?)
        };
        view.setattr("_style_before_xray", (scene, saved))?;
        styles.call_method1("style_by_name", ("X-ray",))?
    };
    display_style(win, &target)
}

#[pyfunction]
fn hidden_view(vp: &Bound<'_, PyAny>, attr: &str, on: bool) -> PyResult<()> {
    let scene = vp.getattr("scene")?;
    if scene.getattr(attr)?.is_truthy()? == on {
        return Ok(());
    }
    scene.setattr(attr, on)?;
    if !on {
        let selection = scene.getattr("selection")?;
        for ent in PyList::new(
            vp.py(),
            selection.try_iter()?.collect::<PyResult<Vec<_>>>()?,
        )?
        .iter()
        {
            if scene.call_method1("entity_hidden", (&ent,))?.is_truthy()?
                || ent
                    .getattr("hidden")
                    .is_ok_and(|v| v.is_truthy().unwrap_or(false))
            {
                selection.call_method1("discard", (ent,))?;
            }
        }
    }
    scene.call_method0("bump_view")?;
    vp.call_method0("update")?;
    Ok(())
}

#[pyfunction]
fn style_field(win: &Bound<'_, PyAny>, field: &str, on: bool) -> PyResult<()> {
    let style = win.call_method0("active_style")?;
    if !style.is_none() && style.getattr(field)?.is_truthy()? != on {
        win.call_method0("own_style")?.setattr(field, on)?;
        win.call_method0("repaint_styled")?;
    }
    Ok(())
}

#[pyfunction]
fn set_shadows(vp: &Bound<'_, PyAny>, enabled: bool) -> PyResult<()> {
    let scene = vp.getattr("scene")?;
    let sh = scene.getattr("shadows")?;
    if !sh.is_none() && sh.getattr("enabled")?.is_truthy()? != enabled {
        sh.setattr("enabled", enabled)?;
        scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
        vp.call_method0("update")?;
    }
    Ok(())
}

#[pyfunction]
fn smart_zoom(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let view = win.getattr("viewport")?;
    let before = win.call_method0("_camera_pose")?;
    if let Ok(back) = view.getattr("_smart_zoom_back") {
        if !back.is_none() && back.get_item(1)?.eq(&before)? {
            let pose = back.get_item(0)?;
            let cam = view.getattr("camera")?;
            cam.setattr(
                "target",
                (pose.get_item(0)?, pose.get_item(1)?, pose.get_item(2)?),
            )?;
            for (i, key) in ["distance", "yaw", "pitch"].iter().enumerate() {
                cam.setattr(*key, pose.get_item(i + 3)?)?;
            }
            view.setattr("_smart_zoom_back", view.py().None())?;
            view.call_method0("update")?;
            return Ok(());
        }
    }
    win.call_method0("_on_zoom_extents")?;
    let after = win.call_method0("_camera_pose")?;
    view.setattr(
        "_smart_zoom_back",
        (!before.eq(&after)?).then_some((before, after)),
    )?;
    Ok(())
}

pub(crate) fn activate_tool_hint(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let bar = win.call_method0("statusBar")?;
    bar.call_method0("clearMessage")?;
    bar.setattr("_tool_hint", win.py().None())?;
    tool_hint(win)
}

#[pyfunction]
pub(crate) fn tool_hint(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let tool = vp.getattr("active_tool")?;
    let nav = vp.getattr("nav_mode")?;
    let key: Option<String> = win.call_method1("_tool_key", (&tool,))?.extract()?;
    let nav_mode: Option<String> = nav.extract()?;
    let mode = vp.getattr_opt("linear_inference_mode")?
        .map(|v| v.extract::<Option<String>>()).transpose()?.flatten();
    let key = key.as_deref();
    let array_pending = key == Some("move") && nav.is_none()
        && tool.call_method1("array_pending", (&vp,))?.is_truthy()?;
    let hint = if nav.is_none() && matches!(key, Some("rectangle" | "line" | "pushpull" | "offset" | "rotate"))
        && vp.getattr_opt("_value_buffer")?.is_some_and(|v| v.is_truthy().unwrap_or(false)) {
        crate::settings_rows::tr(py, "Press Enter to apply.")?
    } else if array_pending {
        crate::settings_rows::tr(py, "Type 3x or /3, then Enter.")?
    } else {
        crate::status_hints::hint_for(py, key, &tool, nav_mode.as_deref(), mode.as_deref())?
    };
    let hint = crate::shortcut_text::modifier_text(&hint);
    let name = if !nav.is_none() {
        nav.call_method1("replace", ("_", " "))?
            .call_method0("capitalize")?
            .extract()?
    } else if !tool.is_none() {
        tool.getattr("name")?.extract()?
    } else {
        String::new()
    };
    let label = translated(py, &name, None)?.extract::<String>()?;
    let text = [label, hint]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" — ");
    let bar = win.call_method0("statusBar")?;
    if !bar.getattr_opt("_tool_hint")?.is_some_and(|previous| previous.eq(&text).unwrap_or(false)) {
        if array_pending {
            bar.call_method0("clearMessage")?;
        }
        bar.setattr("_tool_hint", &text)?;
        bar.setattr("_base", text)?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(standard_view, m)?)?;
    m.add_function(wrap_pyfunction!(display_style, m)?)?;
    m.add_function(wrap_pyfunction!(toggle_xray, m)?)?;
    m.add_function(wrap_pyfunction!(hidden_view, m)?)?;
    m.add_function(wrap_pyfunction!(style_field, m)?)?;
    m.add_function(wrap_pyfunction!(set_shadows, m)?)?;
    m.add_function(wrap_pyfunction!(smart_zoom, m)?)?;
    m.add_function(wrap_pyfunction!(tool_hint, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_scripted_run_jumps_and_a_live_one_glides_a_quarter_second() {
        assert_eq!(super::camera_motion_seconds(true), 0.);
        assert_eq!(super::camera_motion_seconds(false), 0.25);
    }
}

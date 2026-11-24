//! Apply view commands to the active pane of a native model window.
use super::model_edit::translated;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

#[pyfunction]
fn standard_view(vp: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
    let camera = vp.getattr("camera")?;
    if key == "iso" {
        camera.call_method0("snap_iso")?;
    } else {
        camera.call_method1("set_view", (key,))?;
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
    let styles = py.import("core.style")?;
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

#[pyfunction]
fn tool_hint(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let tool = vp.getattr("active_tool")?;
    let nav = vp.getattr("nav_mode")?;
    let mode = vp.getattr("linear_inference_mode").ok();
    let hint = py
        .import("views.status_hints")?
        .call_method1(
            "hint_for",
            (win.call_method1("_tool_key", (&tool,))?, &tool, &nav, mode),
        )?
        .extract::<String>()?;
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
    if !bar.getattr("_base")?.eq(&text)? {
        bar.call_method1("showMessage", (text,))?;
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

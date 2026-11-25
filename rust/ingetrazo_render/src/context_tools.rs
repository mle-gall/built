//! Resolve texture and text edits against the selection that opened them.
use super::model_edit::{matching, translated};
use pyo3::{prelude::*, types::PyDict};

#[pyfunction]
fn textured_face<'py>(
    vp: &Bound<'py, PyAny>,
) -> PyResult<Option<(Bound<'py, PyAny>, &'static str)>> {
    let faces = matching(
        &vp.getattr("scene")?.getattr("selection")?,
        "core.mesh",
        "Face",
    )?;
    if faces.len() != 1 {
        return Ok(None);
    }
    let py = vp.py();
    let face = faces.get_item(0)?;
    let px = vp.getattr("_context_pixel").ok().filter(|v| !v.is_none());
    let back = if let Some(px) = px {
        py.import("tools.paint")?
            .call_method1(
                "clicked_back_side",
                (vp, &face, py.None(), px.get_item(0)?, px.get_item(1)?),
            )?
            .is_truthy()?
    } else {
        false
    };
    let tool = py
        .import("tools.texture_position")?
        .getattr("TexturePositionTool")?;
    for side in if back {
        ["back", "front"]
    } else {
        ["front", "back"]
    } {
        if !tool.call_method1("side_texture", (&face, side))?.is_none() {
            return Ok(Some((face, side)));
        }
    }
    Ok(None)
}

#[pyfunction]
fn edit_texture(win: &Bound<'_, PyAny>, reset: bool) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    let py = win.py();
    let Some((face, side)) = textured_face(&vp)? else {
        return Ok(());
    };
    let tool = win.getattr("_tools")?.get_item("texture_position")?;
    if reset {
        let texture = tool
            .call_method1("side_texture", (&face, side))?
            .cast_into::<PyDict>()?;
        if !texture.contains("uvw")? && !texture.contains("rot")? {
            return Ok(());
        }
        let flat = texture.copy()?;
        for key in ["uvw", "rot"] {
            if flat.contains(key)? {
                flat.del_item(key)?;
            }
        }
        let command = tool.call_method1("side_command", (face, side, flat))?;
        vp.getattr("history")?.call_method1("execute", (command,))?;
        vp.call_method0("update")?;
    } else {
        let mut at = py.None().into_bound(py);
        if let Some(pixel) = vp.getattr("_context_pixel").ok().filter(|p| !p.is_none()) {
            let (origin, dir): (Bound<'_, PyAny>, Bound<'_, PyAny>) = vp
                .call_method1("_pixel_to_ray", (pixel.get_item(0)?, pixel.get_item(1)?))?
                .extract()?;
            if !origin.is_none() {
                let (p, n): (Bound<'_, PyAny>, Bound<'_, PyAny>) = py
                    .import("core.snap")?
                    .call_method1("face_plane_world", (&face, py.None()))?
                    .extract()?;
                at = vp.call_method1("_ray_plane", (origin, dir, p, n))?;
            }
        }
        win.call_method1("_activate_tool", ("texture_position",))?;
        let kw = PyDict::new(py);
        kw.set_item("side", side)?;
        if !tool
            .getattr("begin")?
            .call((vp, face, at), Some(&kw))?
            .is_truthy()?
        {
            win.call_method1("_activate_tool", ("select",))?;
        }
    }
    Ok(())
}

#[pyfunction]
fn selected_text<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let gs = matching(&scene.getattr("selection")?, "core.group", "Group")?;
    if gs.len() != 1 {
        return Ok(None);
    }
    text_target(scene, &gs.get_item(0)?)
}

fn text_target<'py>(
    scene: &Bound<'py, PyAny>,
    g: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    if g.getattr("text3d")?.is_truthy()? {
        return Ok(Some(g.clone()));
    }
    let ctx = scene.getattr("edit_group")?;
    if !ctx.is_none() && ctx.getattr("text3d")?.is_truthy()? {
        let kids = ctx.getattr("children")?;
        if !kids.is_none()
            && (kids.contains(g)?
                || g.getattr("owner")
                    .ok()
                    .map(|owner| kids.contains(owner))
                    .transpose()?
                    .unwrap_or(false))
        {
            return Ok(Some(ctx));
        }
    }
    Ok(None)
}

/// The menu and the editor resolve the same text, including a letter in an open group.
#[pyfunction]
pub(crate) fn text_editable(scene: &Bound<'_, PyAny>, group: &Bound<'_, PyAny>) -> PyResult<Option<bool>> {
    let Some(text) = text_target(scene, group)? else {
        return Ok(None);
    };
    if group.getattr("locked")?.is_truthy()? || text.getattr("locked")?.is_truthy()? {
        return Ok(None);
    }
    Ok(Some(
        scene
            .py()
            .import("core.text3d")?
            .call_method1("text_is_pristine", (text,))?
            .is_truthy()?,
    ))
}

#[pyfunction]
#[pyo3(signature=(win,group=None))]
fn edit_text(win: &Bound<'_, PyAny>, group: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let Some(group) = (match group {
        Some(g) => text_target(&scene, &g)?,
        None => selected_text(&scene)?,
    }) else {
        return Ok(());
    };
    let params = group.getattr("text3d")?;
    if !params.is_truthy()? {
        return Ok(());
    }
    if !py
        .import("core.text3d")?
        .call_method1("text_is_pristine", (&group,))?
        .is_truthy()?
    {
        vp.call_method1("flash_status",(translated(py,"This text's letters were edited by hand, so it can no longer be regenerated as text.",None)?,5000))?;
        return Ok(());
    }
    let done = py.import("functools")?.call_method1(
        "partial",
        (wrap_pyfunction!(apply_text, py)?, win, scene, group),
    )?;
    py.import("ingetrazo_render.font_picker")?
        .call_method1("text3d", (win, params, done))?;
    Ok(())
}

#[pyfunction]
fn apply_text(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    group: &Bound<'_, PyAny>,
    params: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    if !vp.getattr("scene")?.is(scene) || params.is_none() {
        return Ok(());
    }
    let backend = py.import("core.text3d")?;
    let before = PyDict::new(py);
    let old = group.getattr("text3d")?;
    for key in backend.getattr("TEXT_KEYS")?.try_iter()? {
        let k = key?;
        before.set_item(&k, old.call_method1("get", (&k,))?)?;
    }
    if params.eq(before)? {
        return Ok(());
    }
    if backend
        .call_method1("make_text_group", (params,))?
        .is_none()
    {
        py.import("ingetrazo_render.chrome_dialogs")?.call_method1(
            "warning",
            (
                win,
                translated(py, "3D Text", None)?,
                translated(py, "Could not build geometry for that text.", None)?,
            ),
        )?;
        return Ok(());
    }
    if scene.getattr("edit_group")?.is(group) {
        vp.call_method0("end_group_edit")?;
    }
    let cmd = py
        .import("core.history")?
        .call_method1("EditText3DCommand", (group, params))?;
    vp.getattr("history")?.call_method1("execute", (cmd,))?;
    scene.call_method1("select", (vec![group],))?;
    vp.call_method0("update")?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(textured_face, m)?)?;
    m.add_function(wrap_pyfunction!(edit_texture, m)?)?;
    m.add_function(wrap_pyfunction!(text_editable, m)?)?;
    m.add_function(wrap_pyfunction!(selected_text, m)?)?;
    m.add_function(wrap_pyfunction!(edit_text, m)?)?;
    m.add_function(wrap_pyfunction!(apply_text, m)?)?;
    Ok(())
}

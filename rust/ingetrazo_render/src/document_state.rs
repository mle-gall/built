// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-document tool state, selection feedback and edit-context preferences.
use crate::document_host::*;
use pyo3::{prelude::*, types::PyDict};

fn setting<'py, T: IntoPyObject<'py>>(
    py: Python<'py>,
    key: &str,
    default: T,
) -> PyResult<Obj<'py>> {
    py.import("core.settings")?
        .call_method0("settings")?
        .call_method1("value", (key, default))
}
#[pyfunction]
fn load_edit_rest_mode(py: Python) -> PyResult<String> {
    let mode = setting(py, "display/edit_rest_mode", "fade")?
        .str()?
        .to_string();
    Ok(if host(py)?.getattr("EDIT_REST_MODES")?.contains(&mode)? {
        mode
    } else {
        "fade".into()
    })
}
#[pyfunction]
fn load_edit_rest_fade(py: Python) -> PyResult<f64> {
    let fallback = host(py)?.getattr("EDIT_REST_FADE")?.extract::<f64>()?;
    let val = setting(py, "display/edit_rest_fade", fallback)?;
    let x = py
        .import("builtins")?
        .call_method1("float", (val,))
        .and_then(|n| n.extract::<f64>())
        .unwrap_or(fallback);
    Ok(x.min(1.).max(0.))
}
#[pyfunction]
fn load_hide_similar(py: Python) -> PyResult<bool> {
    Ok(setting(py, "display/hide_similar_components", "0")?
        .str()?
        .to_string()
        == "1")
}
#[pyfunction]
fn load_soft_shadows(py: Python) -> PyResult<i64> {
    let val = setting(py, "display/soft_shadows", 0)?;
    let x = py
        .import("builtins")?
        .call_method1("int", (val,))
        .and_then(|x| x.extract::<i64>())
        .unwrap_or(0);
    Ok(if host(py)?.getattr("SOFT_SHADOW_SPREAD")?.contains(x)? {
        x
    } else {
        0
    })
}

#[pyfunction]
fn init_document_interaction(v: &Obj) -> PyResult<()> {
    let py = v.py();
    let h = host(py)?;
    for key in [
        "_last_mouse_pos",
        "_hover_entity",
        "_hover_placement",
        "_last_double",
        "_export_size",
        "plano_style",
        "style_override",
        "view_style",
        "_overlay_rubber",
        "_box_start",
        "_box_cur",
        "_tile_fetcher",
        "_hover_geo_node",
        "_route_marker",
    ] {
        v.setattr(key, py.None())?;
    }
    for key in [
        "_alt_tap",
        "_alt_down",
        "_box_active",
        "_tile_requests_scheduled",
    ] {
        v.setattr(key, false)?;
    }
    v.setattr("linear_inference_mode", "all")?;
    v.setattr("_value_buffer", "")?;
    v.setattr("_last_coordinate", "")?;
    for (key, val) in [
        ("snap_threshold_px", 9.),
        ("edge_snap_threshold_px", 14.),
        ("pick_threshold_px", 8.),
        ("inference_angle_deg", 3.),
    ] {
        v.setattr(key, val)?;
    }
    v.setattr("_edit_rest_mode", h.call_method0("_load_edit_rest_mode")?)?;
    v.setattr(
        "_edit_rest_fade_strength",
        h.call_method0("_load_edit_rest_fade")?,
    )?;
    v.getattr("scene")?.setattr(
        "hide_similar_components",
        h.call_method0("_load_hide_similar")?,
    )?;
    v.setattr("_soft_shadows", h.call_method0("_load_soft_shadows")?)?;
    v.setattr("_msaa", 4)?;
    v.setattr("_suppressed_faces", h.call_method0("ElementSet")?)?;
    v.setattr("_export_edge_px", 1)?;
    v.setattr("_export_profile_px", 1)?;
    v.setattr("_fbo_size", (0, 0))
}
#[pyfunction]
fn set_hover(v: &Obj, e: &Obj, p: &Obj) -> PyResult<()> {
    if e.eq(v.getattr("_hover_entity")?)? && p.is(&v.getattr("_hover_placement")?) {
        return Ok(());
    }
    v.setattr("_hover_entity", e)?;
    v.setattr("_hover_placement", p)?;
    v.call_method0("update")?;
    Ok(())
}
#[pyfunction]
fn set_suppressed_faces(v: &Obj, faces: &Obj) -> PyResult<()> {
    let s = host(v.py())?.call_method1("ElementSet", (faces,))?;
    if !s.eq(v.getattr("_suppressed_faces")?)? {
        v.setattr("_suppressed_faces", s)?;
        v.setattr("_edges_version", -1)?;
        v.call_method0("update")?;
    }
    Ok(())
}
#[pyfunction]
fn edit_rest_fade<'py>(v: &Obj<'py>, g: &Obj<'py>) -> PyResult<Option<f64>> {
    if v.getattr("scene")?.getattr("edit_group")?.is_none()
        || (!g.is_none()
            && !v
                .call_method1("_draws_in_edit_context", (g,))?
                .is_truthy()?)
    {
        return Ok(Some(0.));
    }
    Ok(
        match v.getattr("_edit_rest_mode")?.extract::<String>()?.as_str() {
            "hide" => None,
            "fade" => Some(v.getattr("edit_rest_fade")?.extract()?),
            _ => Some(0.),
        },
    )
}
#[pyfunction]
fn draws_in_edit_context(v: &Obj, g: &Obj) -> PyResult<bool> {
    let ctx = v.getattr("scene")?.getattr("edit_group")?;
    Ok(!ctx.is_none()
        && !ctx.is(g)
        && !ctx.is(&opt(g, "context")?)
        && !ctx.is(&v.call_method1("_owner_of", (g,))?))
}
#[pyfunction]
fn tool_busy(t: &Obj) -> PyResult<bool> {
    for k in ["start_point", "grab"] {
        if !opt(t, k)?.is_none() {
            return Ok(true);
        }
    }
    for k in ["dragging", "_drag", "_stroke", "nodes"] {
        if opt(t, k)?.is_truthy()? {
            return Ok(true);
        }
    }
    Ok(false)
}
#[pyfunction]
fn tool_busy_for_delete(cls: &Obj, t: &Obj) -> PyResult<bool> {
    if cls.call_method1("_tool_busy", (t,))?.is_truthy()? {
        return Ok(true);
    }
    for k in ["base_face", "_profile", "sizing", "edges"] {
        if opt(t, k)?.is_truthy()? {
            return Ok(true);
        }
    }
    Ok(false)
}
#[pyfunction]
fn apply_tool_cursor(v: &Obj) -> PyResult<()> {
    let py = v.py();
    let t = v.getattr("active_tool")?;
    let mut icon = opt(&t, "icon")?;
    if icon.eq("paint")?
        && (py
            .import("tools.paint")?
            .getattr("PaintTool")?
            .getattr("sample_armed")?
            .is_truthy()?
            || opt(v, "_alt_down")?.is_truthy()?)
    {
        icon = "eyedropper".into_pyobject(py)?.into_any();
    }
    let mut cur = if t.is_none() {
        none(py)
    } else {
        v.call_method1(
            "_pointer_for_tool",
            (icon, opt(&t, "cursor_plus")?.is_truthy()?),
        )?
    };
    if cur.is_none() && !t.is_none() {
        let solid = opt(&t, "solid_cursor_key")?;
        if solid.is_truthy()? && opt(&opt(v, "_host")?, "tool_cursor")?.is_truthy()? {
            v.call_method1(
                "setCursor",
                (format!("tool:{}", solid.extract::<String>()?),),
            )?;
            return Ok(());
        }
        cur = opt(&t, "qt_cursor")?;
        if !cur.is_none() && !host(py)?.getattr("NATIVE")?.is_truthy()? {
            cur = py
                .import("PySide6.QtGui")?
                .call_method1("QCursor", (cur,))?;
        }
    }
    if cur.is_none() {
        v.call_method0("unsetCursor")?;
    } else {
        v.call_method1("setCursor", (cur,))?;
    }
    Ok(())
}
#[pyfunction]
fn effective_style<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let style = py.import("core.style")?.getattr("Style")?;
    let mode = v.getattr("plano_style")?;
    if mode.eq("tecnico")? || mode.eq("lineas")? {
        let k = PyDict::new(py);
        k.set_item(
            "face_mode",
            if mode.eq("tecnico")? {
                "hidden_line"
            } else {
                "wireframe"
            },
        )?;
        k.set_item("name", mode)?;
        k.set_item("sky", false)?;
        k.set_item("background", (1., 1., 1.))?;
        return style.call((), Some(&k));
    }
    let over = v.getattr("style_override")?;
    if !over.is_none() {
        return Ok(over);
    }
    let s = v.call_method0("_view_style")?;
    if s.is_truthy()? {
        Ok(s)
    } else {
        style.call0()
    }
}
#[pyfunction]
fn coplanar_eps(v: &Obj, o: &Obj, d: &Obj, t: f64) -> PyResult<f64> {
    let cam = opt(v, "camera")?;
    let depth = if cam.is_none() {
        t
    } else {
        (t - (xyz(&cam.call_method0("eye")?)? - xyz(o)?).dot(xyz(d)?)).abs()
    };
    Ok((depth * 1e-4).max(1e-4))
}
#[pyfunction]
fn pick_axis(v: &Obj, x: f64, y: f64) -> PyResult<Option<&'static str>> {
    let py = v.py();
    let axes = py.import("core.axes")?;
    let half = py
        .import("core.guide")?
        .getattr("GUIDE_HALF_LEN")?
        .extract::<f64>()?;
    let origin = axes.call_method0("origin")?;
    let mut limit = v.getattr("pick_threshold_px")?.extract::<f64>()?;
    let mut best = None;
    for k in ["x", "y", "z"] {
        let d = axes.call_method1("axis", (k,))?.mul(half)?;
        let s = v.call_method1("_clip_segment_front", (origin.sub(&d)?, origin.add(d)?))?;
        if s.is_none() {
            continue;
        }
        let a = v.call_method1("_world_to_pixel", (s.get_item(0)?,))?;
        let b = v.call_method1("_world_to_pixel", (s.get_item(1)?,))?;
        if a.is_none() || b.is_none() {
            continue;
        }
        let dist = host(py)?
            .call_method1("_point_to_segment_distance_2d", ((x, y), a, b))?
            .extract::<f64>()?;
        if dist < limit {
            limit = dist;
            best = Some(k);
        }
    }
    Ok(best)
}
#[pyfunction]
fn axis_source_visible(v: &Obj, name: &str, p: &Obj) -> PyResult<bool> {
    let d = v
        .py()
        .import("core.axes")?
        .call_method1("axis", (name,))?
        .mul(0.25)?;
    let proj = |p: Obj| -> PyResult<Option<[f64; 2]>> {
        v.call_method1("_world_to_pixel", (p,))?.extract()
    };
    Ok(match (proj(p.sub(&d)?)?, proj(p.add(&d)?)?) {
        (Some(a), Some(b)) => (a[0] - b[0]).hypot(a[1] - b[1]) >= 0.5,
        _ => false,
    })
}
#[pyfunction]
fn scalar_ray_triangle(o: &Obj, d: &Obj, a: &Obj, b: &Obj, c: &Obj) -> PyResult<Option<f64>> {
    let (o, d, a, b, c) = (xyz(o)?, xyz(d)?, xyz(a)?, xyz(b)?, xyz(c)?);
    let e = b - a;
    let f = c - a;
    let h = d.cross(f);
    let det = e.dot(h);
    if det.abs() < 1e-6 && (det == 0. || det.abs() <= 1e-6 * e.cross(f).length() * d.length()) {
        return Ok(None);
    }
    let inv = 1. / det;
    let s = o - a;
    let u = inv * s.dot(h);
    let q = s.cross(e);
    let v = inv * d.dot(q);
    let t = inv * f.dot(q);
    Ok(
        (u >= -1e-14 && u <= 1. + 1e-14 && v >= -1e-14 && u + v <= 1. + 1e-14 && t >= 1e-6)
            .then_some(t),
    )
}
#[pyfunction]
fn cache_ver<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let x = opt(v, "_frozen_cache_version")?;
    if x.is_none() {
        v.getattr("scene")?.getattr("version")
    } else {
        Ok(x)
    }
}
#[pyfunction]
fn content_ver<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let x = opt(v, "_frozen_cache_version")?;
    if !x.is_none() {
        return Ok(("frozen", x).into_pyobject(v.py())?.into_any());
    }
    let sc = v.getattr("scene")?;
    for k in ["geometry_version", "content_version"] {
        if let Some(x) = sc.getattr_opt(k)? {
            return Ok(x);
        }
    }
    sc.getattr("version")
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(init_document_interaction, m)?)?;
    m.add_function(wrap_pyfunction!(cache_ver, m)?)?;
    m.add_function(wrap_pyfunction!(content_ver, m)?)?;
    m.add_function(wrap_pyfunction!(scalar_ray_triangle, m)?)?;
    m.add_function(wrap_pyfunction!(load_edit_rest_mode, m)?)?;
    m.add_function(wrap_pyfunction!(load_edit_rest_fade, m)?)?;
    m.add_function(wrap_pyfunction!(load_hide_similar, m)?)?;
    m.add_function(wrap_pyfunction!(load_soft_shadows, m)?)?;

    m.add_function(wrap_pyfunction!(set_hover, m)?)?;
    m.add_function(wrap_pyfunction!(set_suppressed_faces, m)?)?;
    m.add_function(wrap_pyfunction!(edit_rest_fade, m)?)?;
    m.add_function(wrap_pyfunction!(draws_in_edit_context, m)?)?;
    m.add_function(wrap_pyfunction!(tool_busy, m)?)?;
    m.add_function(wrap_pyfunction!(tool_busy_for_delete, m)?)?;
    m.add_function(wrap_pyfunction!(apply_tool_cursor, m)?)?;
    m.add_function(wrap_pyfunction!(effective_style, m)?)?;
    m.add_function(wrap_pyfunction!(coplanar_eps, m)?)?;
    m.add_function(wrap_pyfunction!(pick_axis, m)?)?;
    m.add_function(wrap_pyfunction!(axis_source_visible, m)?)?;
    Ok(())
}

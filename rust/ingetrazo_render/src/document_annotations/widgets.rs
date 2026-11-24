// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
fn planes<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    if scene
        .getattr_opt("show_section_planes")?
        .map(|a| a.is_truthy())
        .transpose()?
        .unwrap_or(true)
    {
        opt(scene, "section_planes")
    } else {
        Ok(none(scene.py()))
    }
}
#[pyfunction]
fn draw_section_planes(v: &Obj, p: &Obj) -> PyResult<()> {
    let scene = v.getattr("scene")?;
    let planes = planes(&scene)?;
    if !planes.is_truthy()? {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    let selection = scene.getattr("selection")?;
    for plane in planes.try_iter()? {
        let plane = plane?;
        if truth(&plane, "hidden")?
            || !scene
                .call_method1("entity_visible", (&plane,))?
                .is_truthy()?
        {
            continue;
        }
        let corners = v.call_method1("section_plane_frame", (&plane,))?;
        let mut outline = Vec::with_capacity(4);
        for i in 0..4 {
            if let Some(s) = c.segment(&corners.get_item(i)?, &corners.get_item((i + 1) % 4)?)? {
                outline.push(s)
            } else {
                break;
            }
        }
        if outline.len() != 4 {
            continue;
        }
        let selected = selection.contains(&plane)?;
        let active = truth(&plane, "active")?;
        let ink = c.rgb(if active {
            [243, 115, 41]
        } else {
            [178, 182, 188]
        })?;
        let width = if active { 2. } else { 1.4 } + if selected { 1.2 } else { 0. };
        let pen = c.make_pen(
            &ink,
            width,
            Some(if selected { "SolidLine" } else { "DashLine" }),
        )?;
        c.set_pen(&pen)?;
        for (a, b) in outline {
            c.line(a, b)?
        }
        c.pen(&ink, width)?;
        let mut projected = [None; 4];
        for (i, q) in projected.iter_mut().enumerate() {
            *q = c.project(&corners.get_item(i)?)?
        }
        for i in 0..4 {
            let (Some((x, y)), Some(a), Some(b)) =
                (projected[i], projected[(i + 1) % 4], projected[(i + 3) % 4])
            else {
                continue;
            };
            for (px, py) in [a, b] {
                let (dx, dy) = (px - x, py - y);
                let len = dx.hypot(dy);
                let k = if len == 0. {
                    0.18
                } else {
                    (14.0f64).min(len * 0.18) / len
                };
                c.line((x, y), (x + dx * k, y + dy * k))?
            }
        }
        let symbol = plane
            .getattr("symbol")?
            .extract::<Option<String>>()?
            .unwrap_or_default();
        let symbol = symbol.trim().chars().take(3).collect::<String>();
        if !symbol.is_empty() {
            p.call_method0("save")?;
            let font = p.call_method0("font")?;
            font.call_method1("setPointSizeF", (8.5,))?;
            font.call_method1("setBold", (true,))?;
            p.call_method1("setFont", (font,))?;
            for (x, y) in projected.into_iter().flatten() {
                c.brush(&c.rgba([255, 255, 255, 220])?)?;
                c.pen(&ink, width)?;
                c.circle((x, y), 9.)?;
                c.pen(&ink, 1.)?;
                p.call_method1(
                    "drawText",
                    (
                        c.rect(x - 9., y - 9., 18., 18.)?,
                        c.flag("AlignCenter")?,
                        &symbol,
                    ),
                )?;
            }
            p.call_method0("restore")?;
        }
    }
    Ok(())
}
#[pyfunction]
fn pick_section_plane<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let scene = v.getattr("scene")?;
    let planes = planes(&scene)?;
    let mut best = none(v.py());
    if !planes.is_truthy()? {
        return Ok(best);
    }
    let c = Paint::new(v, &best)?;
    let mut distance = v.getattr("pick_threshold_px")?.extract::<f64>()?;
    for plane in planes.try_iter()? {
        let plane = plane?;
        if truth(&plane, "hidden")?
            || !scene
                .call_method1("entity_selectable", (&plane,))?
                .is_truthy()?
        {
            continue;
        }
        let corners = v.call_method1("section_plane_frame", (&plane,))?;
        for i in 0..4 {
            if let Some((a, b)) =
                c.segment(&corners.get_item(i)?, &corners.get_item((i + 1) % 4)?)?
            {
                let d = c
                    .styles
                    .call_method1("_point_to_segment_distance_2d", ((x, y), a, b))?
                    .extract::<f64>()?;
                if d < distance {
                    distance = d;
                    best = plane.clone()
                }
            }
        }
    }
    Ok(best)
}
#[pyfunction]
fn draw_scale_box(v: &Obj, p: &Obj) -> PyResult<()> {
    let state = call_opt(&v.getattr("active_tool")?, "scale_box_state")?;
    if state.is_none() {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    let box_pen = c.make_pen(&c.rgb([214, 174, 0])?, 1., None)?;
    c.set_pen(&box_pen)?;
    for ab in state.get_item("segments")?.try_iter()? {
        let (a, b): (Obj, Obj) = ab?.extract()?;
        if let Some((a, b)) = c.segment(&a, &b)? {
            c.line(a, b)?
        }
    }
    let camera = v.getattr("camera")?;
    let eye = xyz(&camera.call_method0("eye")?)?;
    let fwd = xyz(&camera.getattr("target")?)? - eye;
    let mut grips = Vec::new();
    for grip in state.get_item("grips")?.try_iter()? {
        let (pos, role): (Obj, String) = grip?.extract()?;
        grips.push((-(xyz(&pos)? - eye).dot(fwd), pos, role))
    }
    grips.sort_by(|a, b| a.0.total_cmp(&b.0));
    let green = c.rgb([60, 160, 60])?;
    let red = c.rgb([220, 40, 40])?;
    let edge = c.make_pen(&c.rgb([30, 30, 30])?, 1., None)?;
    for (_, pos, role) in grips {
        if let Some((x, y)) = c.project(&pos)? {
            let h = if matches!(role.as_str(), "hover" | "active" | "anchor") {
                5.
            } else {
                4.
            };
            let rect = c.rect(x - h, y - h, h * 2., h * 2.)?;
            p.call_method1(
                "fillRect",
                (&rect, if role == "idle" { &green } else { &red }),
            )?;
            c.set_pen(&edge)?;
            p.call_method1("drawRect", (rect,))?;
        }
    }
    c.set_pen(&box_pen)
}
#[pyfunction]
fn draw_guides(v: &Obj, p: &Obj) -> PyResult<()> {
    let scene = v.getattr("scene")?;
    let c = Paint::new(v, p)?;
    let guides = c.styles.call_method1("shown_guides", (&scene,))?;
    if !guides.is_truthy()? {
        return Ok(());
    }
    let pen = c.make_pen(&c.rgb([70, 90, 120])?, 1., Some("DashLine"))?;
    let selected = c.make_pen(&c.rgb([243, 115, 41])?, 2., Some("DashLine"))?;
    for b in [&pen, &selected] {
        b.call_method1("setDashPattern", ([14., 10.],))?;
    }
    let sel = scene.getattr("selection")?;
    for g in guides.try_iter()? {
        let g = g?;
        if truth(&g, "is_line")? || truth(&g, "is_circle")? {
            continue;
        }
        c.set_pen(if sel.contains(&g)? { &selected } else { &pen })?;
        if let Some(at) = c.project(&g.getattr("point")?)? {
            c.cross(at, 5.)?
        }
    }
    Ok(())
}
fn strokes(c: &Paint, segments: &Obj) -> PyResult<()> {
    for ab in segments.try_iter()? {
        let (a, b): (Obj, Obj) = ab?.extract()?;
        if let Some([a, b]) =
            c.v.call_method1("_segment_to_pixels", (a, b))?
                .extract::<Option<[Point; 2]>>()?
        {
            c.line(a, b)?
        }
    }
    Ok(())
}
#[pyfunction]
fn draw_guide_preview(v: &Obj, p: &Obj) -> PyResult<()> {
    let lines = call_opt(&v.getattr("active_tool")?, "guide_preview_lines")?;
    if !lines.is_truthy()? {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    let pen = c.make_pen(&c.rgb([70, 90, 120])?, 1.5, Some("DashLine"))?;
    pen.call_method1("setDashPattern", ([10., 7.],))?;
    c.set_pen(&pen)?;
    strokes(&c, &lines)
}
#[pyfunction]
fn draw_rubber_band_overlay(v: &Obj, p: &Obj) -> PyResult<()> {
    let band = v.getattr("_overlay_rubber")?;
    if band.is_none() {
        return Ok(());
    }
    let (segments, col, w): (Obj, Vec<f64>, f64) = band.extract()?;
    let c = Paint::new(v, p)?;
    let rgb = col
        .get(..3)
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("expected RGB colour"))?;
    let pen = c.make_pen(&c.tint(rgb, col.get(3).copied().unwrap_or(1.))?, w, None)?;
    pen.call_method1("setCapStyle", (c.flag("RoundCap")?,))?;
    pen.call_method1("setJoinStyle", (c.flag("RoundJoin")?,))?;
    c.set_pen(&pen)?;
    strokes(&c, &segments)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(draw_section_planes, m)?)?;
    m.add_function(wrap_pyfunction!(pick_section_plane, m)?)?;
    m.add_function(wrap_pyfunction!(draw_scale_box, m)?)?;
    m.add_function(wrap_pyfunction!(draw_guides, m)?)?;
    m.add_function(wrap_pyfunction!(draw_guide_preview, m)?)?;
    m.add_function(wrap_pyfunction!(draw_rubber_band_overlay, m)?)?;
    Ok(())
}

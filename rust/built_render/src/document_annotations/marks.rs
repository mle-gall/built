// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn snap_context_suffix(v: &Obj, context: &Obj) -> PyResult<String> {
    let word = match context.extract::<Option<String>>()?.as_deref() {
        Some("component") => "in component",
        Some("group") => "in group",
        Some("outside") => {
            let scene = v.getattr("scene")?;
            if !scene.getattr("edit_group")?.is_none()
                && !scene.call_method0("edit_definition")?.is_none()
            {
                "outside the component"
            } else {
                "outside the group"
            }
        }
        _ => return Ok(String::new()),
    };
    Ok(format!(" {}", tr(v.py(), word)?))
}
fn diamond(c: &Paint, (x, y): Point) -> PyResult<()> {
    c.polygon(&[(x, y - 9.), (x + 9., y), (x, y + 9.), (x - 9., y)])
}
fn square(c: &Paint, (x, y): Point, r: f64) -> PyResult<()> {
    c.p.call_method1("drawRect", (c.rect(x - r, y - r, 2. * r, 2. * r)?,))?;
    Ok(())
}
fn context_label(c: &Paint, snap: &Obj, kind: &str) -> PyResult<Option<String>> {
    let key = c.v.getattr("_SNAP_LABELS")?.call_method1("get", (kind,))?;
    let word = if kind == "aligned" && snap.getattr("axis")?.eq("z")? {
        Some("Level with point".to_owned())
    } else {
        key.extract::<Option<String>>()?
    };
    let mut label = word
        .filter(|s| !s.is_empty())
        .map(|s| tr(c.v.py(), &s))
        .transpose()?
        .unwrap_or_default();
    let custom = opt(snap, "label")?;
    if custom.is_truthy()? {
        label = custom.extract()?
    }
    if label.is_empty() {
        return Ok(None);
    }
    label.push_str(
        &c.v.call_method1("_snap_context_suffix", (opt(snap, "context")?,))?
            .extract::<String>()?,
    );
    Ok(Some(label))
}
#[pyfunction]
fn draw_snap_indicator(v: &Obj, p: &Obj, snap: &Obj) -> PyResult<()> {
    let kind = snap.getattr("kind")?.extract::<String>()?;
    if matches!(kind.as_str(), "axis" | "axis_inference") {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    let Some(at) = c.project(&snap.getattr("point")?)? else {
        return Ok(());
    };
    let (x, y) = at;
    let rgb = snap.getattr("color")?.extract::<[f64; 3]>()?;
    let mut segments = Vec::new();
    let mut guide = |a: Obj, b: Obj, col: [f64; 3]| -> PyResult<()> {
        if let (Some(a), Some(b)) = (c.project(&a)?, c.project(&b)?) {
            if let Some([a, b]) = v
                .call_method1("_clip_pixel_line", (a, b))?
                .extract::<Option<[Point; 2]>>()?
            {
                segments.push((a, b, col))
            }
        }
        Ok(())
    };
    let primary = snap.getattr("guide")?;
    if !primary.is_none() {
        guide(
            primary.get_item(0)?,
            primary.get_item(1)?,
            snap.getattr("guide_color")?
                .extract::<Option<[f64; 3]>>()?
                .unwrap_or(rgb),
        )?
    }
    let more = snap.getattr("guides")?;
    if more.is_truthy()? {
        for g in more.try_iter()? {
            let g = g?;
            guide(g.get_item(0)?, g.get_item(1)?, g.get_item(2)?.extract()?)?
        }
    }
    let label = context_label(&c, snap, &kind)?;
    // The resident canvas already has these glyphs. The compatibility painter and
    // three additional document markers share the painter protocol below.
    if !matches!(kind.as_str(), "quadrant" | "face_center" | "on_circle")
        && p.cast::<crate::annotation_canvas::AnnotationCanvas>()
            .is_ok()
    {
        let channels = |[r, g, b]: [f64; 3]| (r, g, b);
        let guides: Vec<_> = segments
            .into_iter()
            .map(|(a, b, color)| (a, b, channels(color)))
            .collect();
        p.call_method1("draw_snap", (kind, at, channels(rgb), guides, label))?;
        return Ok(());
    }
    for (a, b, color) in segments {
        c.set_pen(&c.make_pen(&c.tint(color, 0.9)?, 2., Some("DashLine"))?)?;
        c.line(a, b)?
    }
    let ink = c.tint(rgb, 1.)?;
    let fill = c.tint(rgb, 0.3)?;
    let halo = c.make_pen(&c.rgba([255, 255, 255, 230])?, 4.5, None)?;
    let mark = c.make_pen(&ink, 2.6, None)?;
    c.brush(&fill)?;
    for (i, pen) in [&halo, &mark].into_iter().enumerate() {
        match kind.as_str() {
            "intersection" => {
                c.set_pen(pen)?;
                c.line((x - 8., y - 8.), (x + 8., y + 8.))?;
                c.line((x - 8., y + 8.), (x + 8., y - 8.))?
            }
            "center" | "face_center" | "endpoint" | "origin" | "component_origin" | "on_edge"
            | "on_axis" | "intersection_edge" | "intersection_face" | "on_line" | "on_circle"
            | "extension" | "from_point" | "aligned" | "tangent" | "midpoint" | "arc_midpoint"
            | "quadrant" => {
                c.set_pen(pen)?;
                if i == 0 {
                    c.no_brush()?
                } else if kind == "center" {
                    c.brush(&c.tint(rgb, 0.85)?)?
                } else {
                    c.brush(&fill)?
                }
                match kind.as_str() {
                    "center" => c.circle(at, 6.5)?,
                    "face_center" => {
                        c.circle(at, 7.)?;
                        c.cross(at, 7.)?
                    }
                    "midpoint" | "arc_midpoint" | "quadrant" => diamond(&c, at)?,
                    _ => square(&c, at, 7.)?,
                }
            }
            "on_face" | "close" | "reference" | "through_point" | "perp_face" => {
                c.set_pen(pen)?;
                c.circle(
                    at,
                    match kind.as_str() {
                        "on_face" => 5.5,
                        "close" => 9.,
                        _ => 6.5,
                    },
                )?
            }
            _ => {}
        }
    }
    if let Some(text) = label {
        c.font(9, false, false)?;
        c.shadow_text(
            (x + 10., y + 16.),
            &text,
            &ink,
            &c.rgba([255, 255, 255, 220])?,
        )?
    }
    Ok(())
}
fn palette<'py>(py: Python<'py>, kind: &str) -> PyResult<([f64; 3], &'static str)> {
    let (color, label) = match kind {
        "midpoint" => ("COLOR_MIDPOINT", "midpoint"),
        "origin" => ("COLOR_ORIGIN", "origin"),
        "edge" => ("COLOR_ON_EDGE", "on_edge"),
        "face" => ("COLOR_ON_FACE", "on_face"),
        "guide_line" => ("COLOR_ON_EDGE", "on_line"),
        "guide_point" => ("COLOR_ENDPOINT", "guide_point"),
        _ => ("COLOR_ENDPOINT", "endpoint"),
    };
    Ok((py.import("built_core.snap")?.getattr(color)?.extract()?, label))
}
#[pyfunction]
fn draw_inference_marker(v: &Obj, p: &Obj) -> PyResult<()> {
    let py = v.py();
    let tool = v.getattr("active_tool")?;
    let result = call_opt(&tool, "inference_marker")?;
    if result.is_none() {
        return Ok(());
    }
    let (world, kind): (Obj, String) = result.extract()?;
    let c = Paint::new(v, p)?;
    let (rgb, label) = palette(py, &kind)?;
    let ink = c.tint(rgb, 1.)?;
    let guides = call_opt(&tool, "inference_guide_lines")?;
    if !guides.is_none() {
        let pen = c.make_pen(&ink, 1.5, Some("DashLine"))?;
        pen.call_method1("setDashPattern", ([10., 7.],))?;
        for g in guides.try_iter()? {
            let (a, b, k): (Obj, Obj, String) = g?.extract()?;
            pen.call_method1("setColor", (c.tint(palette(py, &k)?.0, 1.)?,))?;
            c.set_pen(&pen)?;
            if let Some([a, b]) = v
                .call_method1("_segment_to_pixels", (a, b))?
                .extract::<Option<[Point; 2]>>()?
            {
                c.line(a, b)?
            }
        }
    }
    let Some(at) = c.project(&world)? else {
        return Ok(());
    };
    let halo = c.rgba([255, 255, 255, 230])?;
    for (i, (color, width)) in [(&halo, 4.), (&ink, 2.4)].into_iter().enumerate() {
        c.pen(color, width)?;
        if i == 0 {
            c.no_brush()?
        } else {
            c.brush(&c.tint(rgb, 0.3)?)?
        }
        if kind == "midpoint" {
            diamond(&c, at)?
        } else {
            square(&c, at, 6.)?
        }
    }
    let text = tr(
        py,
        &v.getattr("_SNAP_LABELS")?
            .call_method1("get", (label, ""))?
            .extract::<String>()?,
    )?;
    if !text.is_empty() {
        c.font(9, false, false)?;
        c.shadow_text(
            (at.0 + 10., at.1 + 16.),
            &text,
            &ink,
            &c.rgba([255, 255, 255, 220])?,
        )?
    }
    Ok(())
}
#[pyfunction]
fn draw_linear_mode_label(v: &Obj, p: &Obj) -> PyResult<()> {
    if truth(v, "_chrome_owns_labels")? {
        return Ok(());
    }
    let text = match v
        .getattr("linear_inference_mode")?
        .extract::<String>()?
        .as_str()
    {
        "off" => "Linear inferences: OFF (Alt)",
        "parallel_perp" => "Inferences: parallel / perpendicular only (Alt)",
        _ => return Ok(()),
    };
    let c = Paint::new(v, p)?;
    c.font(9, false, false)?;
    c.pen(&c.rgb([210, 150, 40])?, 1.)?;
    c.text(
        (14., v.call_method0("height")?.extract::<f64>()? - 16.),
        &tr(v.py(), text)?,
    )
}
fn axis_color<'py>(c: &Paint<'py>, axis: &str) -> PyResult<Obj<'py>> {
    c.rgb(match axis {
        "x" => [220, 56, 69],
        "y" => [40, 158, 92],
        "z" => [51, 102, 199],
        _ => return Err(pyo3::exceptions::PyKeyError::new_err(axis.to_string())),
    })
}
fn lock_label(c: &Paint, axis: &str, word: &str, key: &str, val: &str) -> PyResult<()> {
    let kw = PyDict::new(c.v.py());
    kw.set_item(key, val)?;
    let s = c
        .styles
        .call_method("tr", (word,), Some(&kw))?
        .extract::<String>()?;
    c.font(11, true, false)?;
    c.pen(&axis_color(c, axis)?, 1.)?;
    c.text((14., 24.), &s)
}
#[pyfunction]
fn draw_axis_lock_label(v: &Obj, p: &Obj) -> PyResult<()> {
    if truth(v, "_chrome_owns_labels")? {
        return Ok(());
    }
    let axis = v.getattr("axis_lock")?.extract::<String>()?;
    lock_label(
        &Paint::new(v, p)?,
        &axis,
        "{axis} axis locked",
        "axis",
        &axis.to_uppercase(),
    )
}
#[pyfunction]
fn draw_plane_lock_label(v: &Obj, p: &Obj) -> PyResult<()> {
    let axis = v
        .getattr("active_tool")?
        .getattr("plane_lock")?
        .extract::<String>()?;
    let plane = v
        .py()
        .import("tools.base")?
        .getattr("PLANE_LOCK_NAMES")?
        .get_item(&axis)?
        .extract::<String>()?;
    lock_label(
        &Paint::new(v, p)?,
        &axis,
        "Drawing plane locked: {plane}",
        "plane",
        &plane,
    )
}
#[pyfunction]
fn draw_reference_label(v: &Obj, p: &Obj) -> PyResult<()> {
    let mode = v.getattr("reference_mode")?;
    if mode.is_none() || v.getattr("reference_edge")?.is_none() {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    c.font(11, true, false)?;
    c.pen(&c.tint([0.85, 0.3, 0.8], 1.)?, 1.)?;
    c.text(
        (14., 24.),
        if mode.eq("parallel")? {
            "Parallel to reference edge"
        } else {
            "Perpendicular to reference edge"
        },
    )
}
#[pyfunction]
fn draw_inference_label(v: &Obj, p: &Obj) -> PyResult<()> {
    let snap = v.getattr("last_snap")?;
    if snap.is_none() {
        return Ok(());
    }
    let kind = snap.getattr("kind")?;
    let inferred = kind.eq("axis_inference")?;
    if !inferred && !kind.eq("axis")? {
        return Ok(());
    }
    let mut s = match snap
        .getattr("axis")?
        .extract::<Option<String>>()?
        .as_deref()
    {
        Some("x") => tr(v.py(), "On Red Axis")?,
        Some("y") => tr(v.py(), "On Green Axis")?,
        Some("z") => tr(v.py(), "On Blue Axis")?,
        _ => "?".to_owned(),
    };
    if inferred {
        s.push_str(&format!(" ({})", tr(v.py(), "hold Shift to lock")?))
    }
    let c = Paint::new(v, p)?;
    c.font(10, false, true)?;
    c.pen(&c.tint(snap.getattr("color")?.extract()?, 0.95)?, 1.)?;
    c.text((14., 44.), &s)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(snap_context_suffix, m)?)?;
    m.add_function(wrap_pyfunction!(draw_snap_indicator, m)?)?;
    m.add_function(wrap_pyfunction!(draw_inference_marker, m)?)?;
    m.add_function(wrap_pyfunction!(draw_linear_mode_label, m)?)?;
    m.add_function(wrap_pyfunction!(draw_axis_lock_label, m)?)?;
    m.add_function(wrap_pyfunction!(draw_plane_lock_label, m)?)?;
    m.add_function(wrap_pyfunction!(draw_inference_label, m)?)?;
    m.add_function(wrap_pyfunction!(draw_reference_label, m)?)?;
    Ok(())
}

// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::PyBytes;

#[pyfunction]
fn draw_image_outlines(v: &Obj, painter: &Obj) -> PyResult<()> {
    let sc = v.getattr("scene")?;
    let images = opt(&sc, "image_planes")?;
    if !images.is_truthy()? {
        return Ok(());
    }
    let p = Paint::new(v, painter)?;
    let pen = p.make_pen(&p.rgb([120, 140, 165])?, 1., Some("DashLine"))?;
    let picked_pen = p.make_pen(&p.rgb([243, 115, 41])?, 2., None)?;
    let sel = sc.getattr("selection")?;
    for im in images.try_iter()? {
        let im = im?;
        if !sc.call_method1("entity_visible", (&im,))?.is_truthy()? {
            continue;
        }
        let picked = sel.contains(&im)?;
        if !picked && opt(&im, "locked")?.is_truthy()? {
            continue;
        }
        p.set_pen(if picked { &picked_pen } else { &pen })?;
        for pair in im.call_method0("border_edges")?.try_iter()? {
            let pair = pair?;
            if let Some((a, b)) = p.segment(&pair.get_item(0)?, &pair.get_item(1)?)? {
                p.line(a, b)?;
            }
        }
    }
    Ok(())
}
#[pyfunction]
fn selection_dot_color<'py>(surface: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mut alt = false;
    if !surface.is_none() {
        let [r, g, b] = [
            surface.get_item(0)?.extract::<f64>()?,
            surface.get_item(1)?.extract()?,
            surface.get_item(2)?.extract()?,
        ];
        let hi = r.max(g).max(b);
        let lo = r.min(g).min(b);
        let range = hi - lo;
        if hi >= 0.35 && range / hi >= 0.35 {
            let (rc, gc, bc) = ((hi - r) / range, (hi - g) / range, (hi - b) / range);
            let h = (if r == hi {
                bc - gc
            } else if g == hi {
                2. + rc - bc
            } else {
                4. + gc - rc
            }) / 6.;
            let h = h.rem_euclid(1.);
            alt = h <= 0.14 || h >= 0.96;
        }
    }
    host(surface.py())?.getattr(if alt {
        "SELECTION_DOT_ALT_COLOR"
    } else {
        "SELECTION_DOT_COLOR"
    })
}
#[pyfunction]
fn selection_dot_colors<'py>(v: &Obj<'py>, face: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = v.py();
    let h = host(py)?;
    let sc = v.getattr("scene")?;
    let a = py.import("built_core.materials")?.call_method1(
        "effective_attrs",
        (
            face.getattr("attrs")?,
            h.call_method1("_paint_of", (v, sc.getattr("edit_group")?))?,
        ),
    )?;
    let tex = a.call_method1("get", ("texture",))?;
    let front = if !tex.is_none() && tex.call_method1("get", ("path",))?.is_truthy()? {
        none(py)
    } else {
        let col = a.call_method1("get", ("color",))?;
        if col.is_truthy()? {
            col
        } else {
            v.getattr("DEFAULT_FACE_COLOR")?
        }
    };
    let back = a.call_method1("get", ("back",))?;
    let back = if back.is(&true.into_pyobject(py)?) {
        front.clone()
    } else if back.is_instance_of::<PyDict>() {
        back.call_method1("get", ("color",))?
    } else {
        h.call_method1("effective_back_color", (v.call_method0("_view_style")?, sc))?
    };
    Ok((selection_dot_color(&front)?, selection_dot_color(&back)?))
}
#[pyfunction]
fn edit_fade_color<'py>(background: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(
        background.py(),
        background
            .try_iter()?
            .take(3)
            .map(|x| Ok(x?.extract::<f64>()? * 0.5 + 0.275))
            .collect::<PyResult<Vec<_>>>()?,
    )
}
#[pyfunction]
fn rubber_band_style<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let t = v.getattr("active_tool")?;
    if t.is_none() {
        return Ok(none(py));
    }
    let segments = t.call_method0("rubber_band_lines")?;
    if !segments.is_truthy()? {
        return Ok(none(py));
    }
    let forced = opt(&t, "wireframe_color")?;
    let snap = v.getattr("last_snap")?;
    let k = opt(&snap, "kind")?.extract::<Option<String>>()?;
    let k = k.as_deref().unwrap_or("");
    let lock = opt(&snap, "lock_color")?;
    let inference = forced.is_none()
        && matches!(
            k,
            "axis" | "axis_inference" | "reference" | "extension" | "through_point" | "perp_face"
        );
    let color = if !forced.is_none() {
        forced
    } else if !lock.is_none()
        || matches!(
            k,
            "axis" | "axis_inference" | "reference" | "through_point" | "perp_face"
        )
    {
        let alpha = if lock.is_none() && k == "axis_inference" {
            0.5
        } else {
            1.
        };
        let c = if lock.is_none() {
            snap.getattr("color")?
        } else {
            lock
        };
        (c.get_item(0)?, c.get_item(1)?, c.get_item(2)?, alpha)
            .into_pyobject(py)?
            .into_any()
    } else {
        (if k == "close" {
            (0.2, 0.4, 0.78, 0.95)
        } else {
            (0.95, 0.45, 0.16, 0.85)
        })
        .into_pyobject(py)?
        .into_any()
    };
    Ok((segments, color, if inference { 2.5 } else { 2. })
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn draw_selected_face_centers(v: &Obj, painter: &Obj) -> PyResult<()> {
    let py = v.py();
    let pts = v.call_method0("_selected_face_centers")?;
    if !pts.is_truthy()? {
        return Ok(());
    }
    let cam = v.getattr("camera")?;
    let t = xyz(&cam.getattr("target")?)?;
    let coords = PyTuple::new(
        py,
        pts.try_iter()?
            .map(|x| {
                let p = xyz(&x?)?;
                Ok((p.x, p.y, p.z))
            })
            .collect::<PyResult<Vec<_>>>()?,
    )?;
    let key = (
        v.getattr("scene")?.getattr("version")?,
        coords,
        cam.getattr("yaw")?,
        cam.getattr("pitch")?,
        cam.getattr("distance")?,
        cam.getattr("perspective")?,
        t.x,
        t.y,
        t.z,
        v.call_method0("width")?,
        v.call_method0("height")?,
    )
        .into_pyobject(py)?;
    let memo = opt(v, "_sel_face_center_dots")?;
    let dots = if !memo.is_none() && memo.get_item(0)?.eq(&key)? {
        memo.get_item(1)?
    } else {
        let dots = PyList::empty(py);
        let hidden = v.call_method1("_occluded_many", (&pts,))?;
        for (a, b) in pts.try_iter()?.zip(hidden.try_iter()?) {
            if b?.is_truthy()? {
                continue;
            }
            let p = v.call_method1("_world_to_pixel", (a?,))?;
            if !p.is_none() {
                dots.append(p)?;
            }
        }
        v.setattr("_sel_face_center_dots", (key, &dots))?;
        dots.into_any()
    };
    if !dots.is_truthy()? {
        return Ok(());
    }
    let p = Paint::new(v, painter)?;
    p.pen(&p.rgba([255, 255, 255, 230])?, 2.5)?;
    p.brush(
        &p.tint(
            py.import("built_core.snap")?
                .getattr("COLOR_ON_FACE")?
                .extract()?,
            0.85,
        )?,
    )?;
    for d in dots.try_iter()? {
        p.circle(d?.extract()?, 7.)?;
    }
    p.no_brush()
}
fn packed<'py>(py: Python<'py>, coords: &[f32]) -> PyResult<Obj<'py>> {
    let a = host(py)?.call_method1("array", ("f",))?;
    a.call_method1(
        "frombytes",
        (PyBytes::new(py, bytemuck::cast_slice(coords)),),
    )?;
    Ok(a)
}
#[pyfunction]
fn axes_vertices(py: Python, spacing: f64, pos_len: f64) -> PyResult<(Obj, Bound<PyDict>)> {
    let axes = py.import("built_core.axes")?;
    let origin = xyz(&axes.getattr("ORIGIN")?)?;
    let mut coords = Vec::with_capacity(9378);
    let spans = PyDict::new(py);
    let s = spacing.max(1e-4);
    for (i, k) in ["x", "y", "z"].into_iter().enumerate() {
        let axis = if axes.call_method0("is_world")?.is_truthy()? {
            [DVec3::X, DVec3::Y, DVec3::Z][i]
        } else {
            DVec3::from_array(axes.call_method1("as_tuple", (k,))?.extract()?)
        };
        let start = coords.len() / 3;
        let mut push = |p: DVec3| coords.extend(p.to_array().map(|x| x as f32));
        push(origin);
        push(origin + axis * pos_len);
        for n in 0..520 {
            let t = n as f64 * s;
            push(origin - axis * t);
            push(origin - axis * (t + s * 0.18));
        }
        spans.set_item(k, (start, coords.len() / 3 - start))?;
    }
    Ok((packed(py, &coords)?, spans))
}
#[pyfunction]
fn ghost_edge_points<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mut out = Vec::new();
    for e in scene.getattr("mesh")?.getattr("edges")?.try_iter()? {
        let e = e?;
        if (opt(&e, "hidden")?.is_truthy()? || opt(&e, "soft")?.is_truthy()?)
            && scene
                .call_method1("_layer_state", (&e,))?
                .get_item(0)?
                .is_truthy()?
        {
            for k in ["a", "b"] {
                out.extend(xyz(&e.getattr(k)?)?.to_array().map(|x| x as f32));
            }
        }
    }
    packed(scene.py(), &out)
}
#[pyfunction]
fn box_edges<'py>(frame: &Obj<'py>, lo: &Obj<'py>, hi: &Obj<'py>) -> PyResult<Bound<'py, PyBytes>> {
    let py = frame.py();
    let corners = py
        .import("built_core.group")?
        .call_method1("oriented_box_corners", (frame, lo, hi))?;
    let mut out = Vec::with_capacity(72);
    for i in 0..8 {
        for bit in [1, 2, 4] {
            let j = i | bit;
            if i != j {
                for idx in [i, j] {
                    out.extend(xyz(&corners.get_item(idx)?)?.to_array().map(|c| c as f32));
                }
            }
        }
    }
    Ok(PyBytes::new(py, bytemuck::cast_slice(&out)))
}
#[pyfunction]
fn shown_guides<'py>(sc: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(sc.py());
    let g = opt(sc, "guides")?;
    let visible = opt(sc, "entity_visible")?;
    if g.is_truthy()? {
        for x in g.try_iter()? {
            let x = x?;
            if visible.is_none() || visible.call1((&x,))?.is_truthy()? {
                out.append(x)?;
            }
        }
    }
    Ok(out)
}
#[pyfunction]
fn line_jitter(v: &Obj, px: i64, w: i64, h: i64) -> PyResult<Vec<(f64, f64)>> {
    if px <= 1 || v.getattr("_export_size")?.is_none() {
        return Ok(vec![(0., 0.)]);
    }
    let n = px.min(8);
    let r = (n - 1) as f64 / 2.;
    let mut out = vec![];
    for i in 0..n {
        for j in 0..n {
            let (x, y) = (i as f64 - r, j as f64 - r);
            if x * x + y * y <= r * r + 0.51 {
                out.push((x * 2. / w.max(1) as f64, y * 2. / h.max(1) as f64));
            }
        }
    }
    Ok(out)
}
#[pyfunction]
fn tex_run_spans<'py>(
    v: &Obj<'py>,
    parts: &Obj<'py>,
    planes: &Obj<'py>,
    fading: bool,
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = v.py();
    let ctx = PyList::empty(py);
    let subj = PyList::empty(py);
    for row in parts.try_iter()? {
        let r = row?;
        let into = if fading && r.get_item(3)?.is_truthy()? {
            &subj
        } else {
            &ctx
        };
        into.append((r.get_item(0)?, r.get_item(1)?, r.get_item(2)?))?;
    }
    Ok((
        v.call_method1("_visible_spans", (ctx, planes))?
            .get_item(0)?,
        if fading {
            v.call_method1("_visible_spans", (subj, planes))?
                .get_item(0)?
        } else {
            subj.into_any()
        },
    ))
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(selection_dot_color, m)?)?;
    m.add_function(wrap_pyfunction!(edit_fade_color, m)?)?;
    m.add_function(wrap_pyfunction!(axes_vertices, m)?)?;
    m.add_function(wrap_pyfunction!(ghost_edge_points, m)?)?;
    m.add_function(wrap_pyfunction!(box_edges, m)?)?;
    m.add_function(wrap_pyfunction!(shown_guides, m)?)?;

    m.add_function(wrap_pyfunction!(draw_image_outlines, m)?)?;
    m.add_function(wrap_pyfunction!(draw_selected_face_centers, m)?)?;
    m.add_function(wrap_pyfunction!(selection_dot_colors, m)?)?;
    m.add_function(wrap_pyfunction!(rubber_band_style, m)?)?;
    m.add_function(wrap_pyfunction!(line_jitter, m)?)?;
    m.add_function(wrap_pyfunction!(tex_run_spans, m)?)?;
    Ok(())
}

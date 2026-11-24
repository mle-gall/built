// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
fn selectable(scene: &Obj, e: &Obj) -> PyResult<bool> {
    scene.call_method1("entity_selectable", (e,))?.is_truthy()
}
fn threshold(v: &Obj) -> PyResult<f64> {
    v.getattr("pick_threshold_px")?.extract()
}
fn distance(v: &Obj, point: Point, a: &Obj, b: &Obj) -> PyResult<Option<f64>> {
    Ok(project(v, a)?
        .zip(project(v, b)?)
        .map(|(a, b)| point_segment_distance(point, a, b)))
}
#[pyfunction]
fn pick_dimension<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let scene = v.getattr("scene")?;
    let dims = opt(&scene, "dimensions")?;
    let mut best = none(v.py());
    if !dims.is_truthy()? {
        return Ok(best);
    }
    let mut d = threshold(v)?;
    for dim in dims.try_iter()? {
        let dim = dim?;
        if !selectable(&scene, &dim)? {
            continue;
        }
        let (a, b): (Obj, Obj) = dim.call_method0("line_points")?.extract()?;
        for (start, end) in [
            (dim.getattr("a")?, &a),
            (dim.getattr("b")?, &b),
            (a.clone(), &b),
        ] {
            if let Some(t) = distance(v, (x, y), &start, end)? {
                if t < d {
                    d = t;
                    best = dim.clone()
                }
            }
        }
    }
    Ok(best)
}
#[pyfunction]
fn pick_geopath<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let paths = opt(&v.getattr("scene")?, "geo_paths")?;
    let mut best = none(v.py());
    if !paths.is_truthy()? {
        return Ok(best);
    }
    let mut d = threshold(v)?;
    for path in paths.try_iter()? {
        let path = path?;
        for seg in path.call_method0("segments")?.try_iter()? {
            let (a, b): (Obj, Obj) = seg?.extract()?;
            if let Some(t) = distance(
                v,
                (x, y),
                &v.call_method1("drape", (a,))?,
                &v.call_method1("drape", (b,))?,
            )? {
                if t < d {
                    d = t;
                    best = path.clone()
                }
            }
        }
    }
    Ok(best)
}
#[pyfunction]
fn circle_pick_distance(v: &Obj, g: &Obj, x: f64, y: f64) -> PyResult<Option<f64>> {
    let mut pts = Vec::new();
    for p in g.call_method1("sample_points", (64,))?.try_iter()? {
        let Some(q) = project(v, &p?)? else {
            return Ok(None);
        };
        pts.push(q)
    }
    let d = pts
        .iter()
        .zip(pts.iter().cycle().skip(1))
        .map(|(&a, &b)| point_segment_distance((x, y), a, b))
        .fold(f64::INFINITY, f64::min);
    Ok(Some(d))
}
#[pyfunction]
fn pick_guide<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let scene = v.getattr("scene")?;
    let guides = opt(&scene, "guides")?;
    let mut best = none(v.py());
    if !guides.is_truthy()? {
        return Ok(best);
    }
    let mut d = threshold(v)?;
    let visible = opt(&scene, "entity_selectable")?;
    for g in guides.try_iter()? {
        let g = g?;
        if !visible.is_none() && !visible.call1((&g,))?.is_truthy()? {
            continue;
        }
        let dist = if opt(&g, "is_circle")?.is_truthy()? {
            v.call_method1("_circle_pick_distance", (&g, x, y))?
                .extract::<Option<f64>>()?
        } else {
            let (a, b): (Obj, Obj) = g.call_method0("segment")?.extract()?;
            let seg = v.call_method1("_clip_segment_front", (a, b))?;
            if seg.is_none() {
                continue;
            }
            distance(v, (x, y), &seg.get_item(0)?, &seg.get_item(1)?)?
        };
        if let Some(t) = dist {
            if t < d {
                d = t;
                best = g
            }
        }
    }
    Ok(best)
}
#[pyfunction]
fn pick_text_label<'py>(v: &Obj<'py>, x: f64, y: f64, rect_only: bool) -> PyResult<Obj<'py>> {
    let py = v.py();
    let scene = v.getattr("scene")?;
    let labels = opt(&scene, "text_labels")?;
    let mut best = none(py);
    if !labels.is_truthy()? {
        return Ok(best);
    }
    let mut d = 2. * threshold(v)?;
    let styles = host(py)?;
    let style = scene
        .getattr_opt("dimension_style")?
        .unwrap_or_else(|| PyDict::new(py).into_any());
    let font = styles.call_method1("QFont", (style.call_method1("get", ("font_family", ""))?,))?;
    font.call_method1(
        "setPointSize",
        (py.import("builtins")?
            .call_method1("int", (style.call_method1("get", ("font_size", 9))?,))?,),
    )?;
    for (method, key, default) in [
        ("setBold", "font_bold", true),
        ("setItalic", "font_italic", false),
    ] {
        font.call_method1(method, (style.call_method1("get", (key, default))?,))?;
    }
    let fm = if styles.getattr("NATIVE")?.is_truthy()? {
        font
    } else if v.is_instance(&py.import("PySide6.QtGui")?.getattr("QPaintDevice")?)? {
        styles.call_method1("QFontMetrics", (font, v))?
    } else {
        styles.call_method1("QFontMetrics", (font,))?
    };
    for lab in labels.try_iter()? {
        let lab = lab?;
        if !selectable(&scene, &lab)? {
            continue;
        }
        let Some(pp) = project(v, &lab.call_method0("position")?)? else {
            continue;
        };
        let pa = project(v, &lab.getattr("anchor")?)?;
        let mut lines = lab
            .getattr("text")?
            .call_method0("splitlines")?
            .extract::<Vec<String>>()?;
        if lines.is_empty() {
            lines.push(String::new())
        }
        let mut widths = Vec::new();
        for line in &lines {
            widths.push(
                fm.call_method1("horizontalAdvance", (line,))?
                    .extract::<f64>()?,
            )
        }
        let left = v
            .call_method1(
                "_text_block_x",
                (pp, pa, widths.iter().copied().fold(0., f64::max)),
            )?
            .extract::<f64>()?;
        let ascent = fm.call_method0("ascent")?.extract::<f64>()?;
        let descent = fm.call_method0("descent")?.extract::<f64>()?;
        let line = fm.call_method0("height")?.extract::<f64>()?;
        for (i, w) in widths.into_iter().enumerate() {
            let base = pp.1 - 4. + i as f64 * line;
            if (left - 3. ..=left + w + 3.).contains(&x)
                && (base - ascent - 3. ..=base + descent + 3.).contains(&y)
            {
                return Ok(lab);
            }
        }
        if !rect_only {
            let mut dist = (pp.0 - x).hypot(pp.1 - y);
            if let Some(a) = pa {
                dist = dist.min(point_segment_distance((x, y), a, pp))
            }
            if dist < d {
                d = dist;
                best = lab
            }
        }
    }
    Ok(best)
}
fn plane_depth(n: DVec3, p: DVec3, o: DVec3, d: DVec3) -> Option<f64> {
    let denom = n.dot(d);
    (denom.abs() >= 1e-9).then(|| n.dot(p - o) / denom)
}
fn image<'py>(v: &Obj<'py>, x: f64, y: f64, select: Option<bool>) -> PyResult<Obj<'py>> {
    let scene = v.getattr("scene")?;
    let images = opt(&scene, "image_planes")?;
    let mut best = none(v.py());
    if !images.is_truthy()? {
        return Ok(best);
    }
    let Some((o, d)) = ray(v, x, y)? else {
        return Ok(best);
    };
    let (o, d) = (xyz(&o)?, xyz(&d)?);
    let mut depth = f64::INFINITY;
    for im in images.try_iter()? {
        let im = im?;
        if !scene.call_method1("entity_visible", (&im,))?.is_truthy()? {
            continue;
        }
        if let Some(locked) = select {
            if locked != opt(&im, "locked")?.is_truthy()? || !selectable(&scene, &im)? {
                continue;
            }
        }
        let Some(t) = plane_depth(
            xyz(&im.call_method0("normal")?)?,
            xyz(&im.call_method0("center")?)?,
            o,
            d,
        ) else {
            continue;
        };
        if t < 0. || t > depth {
            continue;
        }
        let uv = im.call_method1("project", (vec3(v.py(), o + d * t)?,))?;
        if im
            .call_method1("contains_uv", (uv.get_item(0)?, uv.get_item(1)?))?
            .is_truthy()?
        {
            best = im;
            depth = t
        }
    }
    Ok(best)
}
#[pyfunction]
fn image_plane_at<'py>(v: &Obj<'py>, x: f64, y: f64, selectable_only: bool) -> PyResult<Obj<'py>> {
    image(v, x, y, selectable_only.then_some(false))
}
#[pyfunction]
fn pick_locked_image_plane<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    image(v, x, y, Some(true))
}
#[pyfunction]
fn image_in_front_of<'py>(v: &Obj<'py>, face: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let no = none(v.py());
    let im = v.call_method1("pick_image_plane", (x, y))?;
    if im.is_none() || face.is_none() {
        return Ok(no);
    }
    let Some((o, d)) = ray(v, x, y)? else {
        return Ok(no);
    };
    let verts = opt(face, "vertices")?;
    if !verts.is_truthy()? {
        return Ok(no);
    }
    let (o, d) = (xyz(&o)?, xyz(&d)?);
    let ft = plane_depth(
        xyz(&face.call_method0("normal")?)?,
        xyz(&verts.get_item(0)?)?,
        o,
        d,
    );
    let it = plane_depth(
        xyz(&im.call_method0("normal")?)?,
        xyz(&im.call_method0("center")?)?,
        o,
        d,
    );
    Ok(
        if ft
            .zip(it)
            .is_some_and(|(f, i)| i >= 0. && i < f - (1e-4f64).max(1e-5 * f.abs()))
        {
            im
        } else {
            no
        },
    )
}
#[pyfunction]
fn pick_locked_group<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let no = none(py);
    let Some((o, d)) = ray(v, x, y)? else {
        return Ok(no);
    };
    let idx = ray_index(v, x, y)?;
    if idx.is_none() {
        return Ok(no);
    }
    let c = class(py)?;
    let hits = c.call_method1("_face_hits", (v, idx, &o, &d, "vis"))?;
    py.import("ingetrazo_core")?
        .call_method1("drop_past_cut", (v, &hits, &o, &d))?;
    if hits.is_none() || hits.getattr("t")?.len()? == 0 {
        return Ok(no);
    }
    let front = c.call_method1("_smallest_at_front", (v, &hits, o, d))?;
    if front.len()? == 0 {
        return Ok(no);
    }
    let owner = hits.call_method1("owner", (front.get_item(0)?.extract::<usize>()?,))?;
    for g in [&owner, &opt(&owner, "owner")?] {
        if opt(g, "locked")?.is_truthy()? && g.hasattr("children")? {
            let scene = v.getattr("scene")?;
            let (vis, lock): (bool, bool) = scene.call_method1("_layer_state", (g,))?.extract()?;
            if vis && !lock && scene.call_method1("entity_visible", (g,))?.is_truthy()? {
                return Ok(g.clone());
            }
            break;
        }
    }
    Ok(no)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(pick_dimension, m)?)?;
    m.add_function(wrap_pyfunction!(pick_text_label, m)?)?;
    m.add_function(wrap_pyfunction!(pick_geopath, m)?)?;
    m.add_function(wrap_pyfunction!(pick_guide, m)?)?;
    m.add_function(wrap_pyfunction!(circle_pick_distance, m)?)?;
    m.add_function(wrap_pyfunction!(image_plane_at, m)?)?;
    m.add_function(wrap_pyfunction!(pick_locked_image_plane, m)?)?;
    m.add_function(wrap_pyfunction!(image_in_front_of, m)?)?;
    m.add_function(wrap_pyfunction!(pick_locked_group, m)?)?;
    Ok(())
}

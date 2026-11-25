// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn ground_height<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let scene = v.getattr("scene")?;
    for name in ["photo_mesh", "terrain"] {
        let layer = opt(&scene, name)?;
        if truth(&layer, "visible")? {
            let z = layer.call_method1("height_at", (x, y))?;
            if !z.is_none() {
                return Ok(z);
            }
        }
    }
    Ok(none(v.py()))
}
#[pyfunction]
fn ground_elevation(v: &Obj, x: f64, y: f64) -> PyResult<Option<f64>> {
    let Some(z) = v
        .call_method1("ground_height", (x, y))?
        .extract::<Option<f64>>()?
    else {
        return Ok(None);
    };
    let alt = opt(&opt(&v.getattr("scene")?, "georef")?, "alt")?;
    let dz = if alt.is_truthy()? {
        v.py()
            .import("builtins")?
            .call_method1("float", (alt,))?
            .extract()?
    } else {
        0.
    };
    Ok(Some(z + dz))
}
#[pyfunction]
fn vertical_reference<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let m = v.py().import("built_core.georef.photomesh")?;
    let s = v.getattr("scene")?;
    let photo = opt(&s, "photo_mesh")?;
    if truth(&photo, "visible")? {
        return Ok(photo
            .getattr_opt("vertical_reference")?
            .unwrap_or(m.getattr("VERTICAL_ODM")?));
    }
    m.getattr(if truth(&opt(&s, "terrain")?, "visible")? {
        "VERTICAL_DEM"
    } else {
        "VERTICAL_LOCAL"
    })
}
#[pyfunction]
fn has_ground_surface(v: &Obj) -> PyResult<bool> {
    let s = v.getattr("scene")?;
    Ok(truth(&opt(&s, "photo_mesh")?, "visible")? || truth(&opt(&s, "terrain")?, "visible")?)
}
#[pyfunction]
fn drape<'py>(v: &Obj<'py>, pt: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mut world = xyz(pt)?;
    if let Some(z) = v
        .call_method1("ground_height", (world.x, world.y))?
        .extract::<Option<f64>>()?
    {
        world.z = z;
        vec3(v.py(), world)
    } else {
        Ok(pt.clone())
    }
}
/// Sample hover state once per frame. Reattach the readout when the renderer changes.
#[pyfunction]
fn sync_status_readout(view: &Obj, name: &str, hint: &str) -> PyResult<()> {
    let vp = view.getattr("source_viewport")?;
    let text = |key: &str| -> PyResult<String> {
        Ok(vp.getattr_opt(key)?.and_then(|v| v.extract().ok()).unwrap_or_default())
    };
    let coord = text("_last_coordinate")?;
    let full = text("_last_coordinate_full")?;
    let renderer = view.getattr("renderer")?;
    let values = (name, hint, &coord, &full).into_pyobject(view.py())?;
    if view.getattr_opt("_ui_readout_sig")?.is_some_and(|v| v.eq(&values).unwrap_or(false)) {
        return Ok(());
    }
    renderer.call_method1("ui_set_status", &values)?;
    view.setattr("_ui_readout_sig", values)?;
    Ok(())
}

#[pyfunction]
fn emit_coordinate(v: &Obj, ev: &Obj) -> PyResult<()> {
    let datum = opt(&v.getattr("scene")?, "georef")?;
    if datum.is_none() {
        if truth(v, "_last_coordinate")? {
            v.setattr("_last_coordinate", "")?;
            v.setattr("_last_coordinate_full", "")?;
            v.getattr("coordinateChanged")?
                .call_method1("emit", ("",))?;
        }
        return Ok(());
    }
    let at = ev.call_method0("position")?.call_method0("toPoint")?;
    let world = v.call_method1(
        "_world_from_pixel",
        (at.call_method0("x")?, at.call_method0("y")?),
    )?;
    if world.is_none() {
        return Ok(());
    }
    let utm = datum.call_method1("local_to_utm", (&world,))?;
    let east = utm.get_item(0)?;
    let north = utm.get_item(1)?;
    let fmt = |n: &Obj, s: &str| n.call_method1("__format__", (s,))?.extract::<String>();
    let mut text = format!("E {}  N {}", fmt(&east, ".1f")?, fmt(&north, ".1f")?);
    let mut full = format!(
        "E {}  N {}  {}{}",
        fmt(&east, ",.2f")?,
        fmt(&north, ",.2f")?,
        datum.getattr("zone")?.str()?,
        datum.getattr("hemisphere")?.str()?
    );
    let world = xyz(&world)?;
    let z = v.call_method1("ground_elevation", (world.x, world.y))?;
    if !z.is_none() {
        text.push_str(&format!("  ·  {} m", fmt(&z, ".1f")?));
        full.push_str(&format!("  ·  {} m", fmt(&z, ",.2f")?))
    }
    v.setattr("_last_coordinate_full", full)?;
    if !v.getattr("_last_coordinate")?.eq(&text)? {
        v.setattr("_last_coordinate", &text)?;
        v.getattr("coordinateChanged")?
            .call_method1("emit", (text,))?;
    }
    Ok(())
}
#[pyfunction]
fn draw_geo_surfaces(v: &Obj, p: &Obj) -> PyResult<()> {
    let paths = opt(&v.getattr("scene")?, "geo_paths")?;
    if !paths.is_truthy()? {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    let eye = xyz(&v.getattr("camera")?.call_method0("eye")?)?;
    let light = DVec3::new(0.3, 0.4, 0.85).normalize();
    for path in paths.try_iter()? {
        let path = path?;
        let tris = opt(&path, "_surface_tris")?;
        if !tris.is_truthy()? {
            continue;
        }
        let flat = opt(&path, "surface")?.eq("flat")?;
        let mut ordered = Vec::new();
        for t in tris.try_iter()? {
            let pts = t?.extract::<[Obj; 3]>()?;
            let w = [xyz(&pts[0])?, xyz(&pts[1])?, xyz(&pts[2])?];
            ordered.push((-(w[0] + w[1] + w[2] - eye * 3.).length_squared(), pts, w))
        }
        ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
        c.set_pen(&c.flag("NoPen")?)?;
        for (_, pts, [a, b, d]) in ordered {
            let (Some(q), Some(r), Some(s)) = (
                c.project(&pts[0])?,
                c.project(&pts[1])?,
                c.project(&pts[2])?,
            ) else {
                continue;
            };
            let mut normal = (b - a).cross(d - a);
            if normal.length() > 1e-9 {
                normal = normal.normalize()
            }
            let shade = 0.55 + 0.45 * normal.dot(light).abs();
            let base = if flat {
                [90., 120., 200.]
            } else {
                [120., 160., 90.]
            };
            let [r1, g, b] = base.map(|c| (c * shade) as i32);
            c.brush(&c.rgba([r1, g, b, 150])?)?;
            c.polygon(&[q, r, s])?
        }
        c.no_brush()?;
    }
    Ok(())
}
#[pyfunction]
fn draw_geo_points(v: &Obj, p: &Obj) -> PyResult<()> {
    let pts = opt(&v.getattr("scene")?, "geo_points")?;
    if !pts.is_truthy()? {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    let ink = c.rgb([206, 66, 87])?;
    c.pen(&ink, 1.6)?;
    c.font(8, false, false)?;
    for gp in pts.try_iter()? {
        let gp = gp?;
        if let Some((x, y)) = c.project(&gp.getattr("position")?)? {
            c.pen(&ink, 1.6)?;
            c.cross((x, y), 5.)?;
            let name = gp.getattr("name")?;
            if name.is_truthy()? {
                c.set_pen(&c.rgb([120, 34, 48])?)?;
                c.text((x + 7., y - 4.), &name.extract::<String>()?)?
            }
        }
    }
    Ok(())
}
#[pyfunction]
fn draw_route_marker(v: &Obj, p: &Obj) -> PyResult<()> {
    let marker = opt(v, "_route_marker")?;
    if marker.is_none() {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    if let Some(q) = c.project(&v.call_method1("drape", (marker,))?)? {
        c.brush(&c.rgb([243, 115, 41])?)?;
        c.pen(&c.rgb([255; 3])?, 1.5)?;
        c.circle(q, 6.)?;
        c.no_brush()?
    }
    Ok(())
}
#[pyfunction]
fn draw_geo_paths(v: &Obj, p: &Obj) -> PyResult<()> {
    let scene = v.getattr("scene")?;
    let paths = opt(&scene, "geo_paths")?;
    if !paths.is_truthy()? {
        return Ok(());
    }
    let c = Paint::new(v, p)?;
    let sel = scene.getattr("selection")?;
    let hover = opt(v, "_hover_geo_node")?;
    let white = c.rgb([255; 3])?;
    for path in paths.try_iter()? {
        let path = path?;
        let selected = sel.contains(&path)?;
        let ink = c.rgb(if selected {
            [243, 115, 41]
        } else {
            [0, 190, 210]
        })?;
        let mut points = Vec::new();
        for pt in path.getattr("points")?.try_iter()? {
            points.push(c.project(&v.call_method1("drape", (pt?,))?)?)
        }
        c.pen(&ink, 2.5)?;
        for pair in points.windows(2) {
            if let (Some(a), Some(b)) = (pair[0], pair[1]) {
                c.line(a, b)?
            }
        }
        let closed = truth(&path, "closed")? && points.len() >= 3;
        if closed {
            if let (Some(a), Some(b)) = (points[points.len() - 1], points[0]) {
                c.line(a, b)?
            }
        }
        c.brush(&ink)?;
        c.pen(&white, 1.)?;
        for (i, pt) in points.iter().enumerate() {
            if let Some(q) = pt {
                c.circle(*q, if hover.eq((&path, i))? { 5. } else { 3.5 })?
            }
        }
        c.no_brush()?;
        if selected && closed {
            v.call_method1("_draw_geo_path_label", (p, path))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn draw_geo_path_label(v: &Obj, p: &Obj, path: &Obj) -> PyResult<()> {
    let c = Paint::new(v, p)?;
    let pts = path.getattr("points")?;
    let mut center = DVec3::ZERO;
    for pt in pts.try_iter()? {
        center += xyz(&pt?)?
    }
    let center = vec3(v.py(), center / pts.len()? as f64)?;
    let Some((cx, cy)) = c.project(&v.call_method1("drape", (center,))?)? else {
        return Ok(());
    };
    let area = path.call_method0("area")?.extract::<f64>()?;
    let perimeter = c
        .styles
        .call_method1("_fmt_len", (path.call_method0("perimeter")?,))?
        .extract::<String>()?;
    let lines = [
        format!(
            "{}: {area:.1} m²  ({:.3} ha)",
            tr(v.py(), "Area")?,
            area / 10000.
        ),
        format!("{}: {perimeter}", tr(v.py(), "Perimeter")?),
    ];
    c.font(9, true, false)?;
    let fm = p.call_method0("fontMetrics")?;
    let height = fm.call_method0("height")?.extract::<i32>()?;
    let width = fm
        .call_method1("horizontalAdvance", (&lines[0],))?
        .extract::<i32>()?
        .max(
            fm.call_method1("horizontalAdvance", (&lines[1],))?
                .extract()?,
        );
    let (x, y) = (cx - width as f64 / 2., cy - height as f64);
    p.call_method1(
        "fillRect",
        (
            x as i32 - 5,
            y as i32 - 3,
            width + 10,
            2 * height + 6,
            c.rgba([20, 24, 30, 190])?,
        ),
    )?;
    c.set_pen(&c.rgb([255; 3])?)?;
    let ascent = fm.call_method0("ascent")?.extract::<f64>()?;
    for (i, s) in lines.iter().enumerate() {
        c.text((x, y + ascent + i as f64 * height as f64), s)?
    }
    Ok(())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(draw_geo_surfaces, m)?)?;
    m.add_function(wrap_pyfunction!(ground_height, m)?)?;
    m.add_function(wrap_pyfunction!(ground_elevation, m)?)?;
    m.add_function(wrap_pyfunction!(vertical_reference, m)?)?;
    m.add_function(wrap_pyfunction!(has_ground_surface, m)?)?;
    m.add_function(wrap_pyfunction!(drape, m)?)?;
    m.add_function(wrap_pyfunction!(emit_coordinate, m)?)?;
    m.add_function(wrap_pyfunction!(sync_status_readout, m)?)?;
    m.add_function(wrap_pyfunction!(draw_geo_points, m)?)?;
    m.add_function(wrap_pyfunction!(draw_route_marker, m)?)?;
    m.add_function(wrap_pyfunction!(draw_geo_paths, m)?)?;
    m.add_function(wrap_pyfunction!(draw_geo_path_label, m)?)?;
    Ok(())
}

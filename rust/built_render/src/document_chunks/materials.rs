// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn proto_wrapper<'py>(v: &Obj<'py>, mesh: &Obj<'py>, paint: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let wrappers = memo(v, "_proto_wrappers")?;
    let key = (id(mesh), host(py)?.call_method1("_material_sig", (paint,))?).into_pyobject(py)?;
    let old = wrappers.call_method1("get", (&key,))?;
    if !old.is_none() {
        old.setattr("material", paint)?;
        return Ok(old);
    }
    let kw = PyDict::new(py);
    kw.set_item("mesh", mesh)?;
    kw.set_item("xform", py.None())?;
    kw.set_item("material", paint)?;
    let w = py
        .import("types")?
        .call_method("SimpleNamespace", (), Some(&kw))?;
    wrappers.set_item(key, &w)?;
    Ok(w)
}
#[pyfunction]
fn flags_read(mesh: &Obj, entry: &Obj) -> PyResult<()> {
    let mut gen = mesh
        .getattr_opt("_flags_gen")?
        .map(|n| n.extract::<i64>())
        .transpose()?
        .unwrap_or(0);
    if mesh
        .getattr_opt("_chunk_dirty")?
        .map(|x| x.is_truthy())
        .transpose()?
        .unwrap_or(true)
        || truth(mesh, "_attrs_dirty")?
    {
        gen += 1;
        mesh.setattr("_flags_gen", gen)?
    }
    mesh.setattr("_chunk_dirty", false)?;
    mesh.setattr("_attrs_dirty", false)?;
    entry.set_item("flags_gen", gen)
}
fn paint_mode(style: &Obj) -> PyResult<bool> {
    Ok(matches!(
        opt(style, "face_mode")?
            .extract::<Option<String>>()?
            .as_deref(),
        Some("textures" | "shaded" | "xray")
    ))
}
#[pyfunction]
fn front_tint<'py>(style: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = style.py();
    if !paint_mode(style)? {
        return Ok(none(py));
    }
    let color = opt(style, "front_color")?;
    if !color.is_truthy()? {
        return Ok(none(py));
    }
    let rgb = color.extract::<Vec<f64>>()?;
    if rgb.iter().take(3).all(|&c| c >= 0.999) {
        return Ok(none(py));
    }
    let base = py
        .import("built_core.style")?
        .getattr("DEFAULT_FACE_COLOR")?
        .extract::<[f64; 3]>()?;
    Ok(PyTuple::new(
        py,
        base.iter()
            .zip(rgb)
            .map(|(d, c)| rounded(py, d * c, 4))
            .collect::<PyResult<Vec<_>>>()?,
    )?
    .into_any())
}
#[pyfunction]
fn with_front_tint<'py>(p: &Obj<'py>, style: &Obj<'py>) -> PyResult<Obj<'py>> {
    if p.is_truthy()? {
        return Ok(p.clone());
    }
    let tint = front_tint(style)?;
    if tint.is_none() {
        return Ok(p.clone());
    }
    let d = PyDict::new(p.py());
    d.set_item("color", tint)?;
    d.set_item("front_only", true)?;
    Ok(d.into_any())
}
#[pyfunction]
fn layer_look_sig<'py>(scene: &Obj<'py>, style: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let style = if style.is_none() {
        opt(scene, "display_style")?
    } else {
        style.clone()
    };
    let by = truth(&style, "color_by_layer")?;
    let layers = opt(scene, "layers")?;
    let mut looks = Vec::new();
    if !layers.is_none() {
        for ly in layers.try_iter()? {
            let ly = ly?;
            looks.push((
                ly.getattr("key")?,
                ly.getattr("color")?,
                ly.getattr("opacity")?,
            ))
        }
    }
    Ok((
        by,
        if by {
            opt(&style, "face_mode")?
        } else {
            none(py)
        },
        PyTuple::new(py, looks)?,
        front_tint(&style)?,
    )
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn paint_of<'py>(v: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let m = host(py)?;
    let mut worn = m.call_method1("effective_material", (group,))?;
    if !group.is_none() && !group.is_instance(&m.getattr("Group")?)? {
        return Ok(worn);
    }
    let scene = opt(v, "scene")?;
    if !group.is_none() && group.is(&opt(&scene, "edit_group")?) {
        worn = scene.call_method0("open_group_paint")?
    }
    if scene.is_none() || !opt(v, "plano_style")?.is_none() {
        return Ok(worn);
    }
    let mut style = opt(v, "style_override")?;
    if !style.is_truthy()? {
        style = opt(v, "view_style")?
    }
    if !style.is_truthy()? {
        style = opt(&scene, "display_style")?
    }
    let tints = opt(v, "view_tint")?;
    let enabled = paint_mode(&style)?;
    if tints.is_truthy()? && enabled {
        let tint = tints.call_method1("get", (opt(group, "uid")?,))?;
        if !tint.is_none() {
            let d = PyDict::new(py);
            d.set_item("by_layer", PyDict::new(py))?;
            d.set_item("color", tint)?;
            return Ok(d.into_any());
        }
    }
    let by = enabled && truth(&style, "color_by_layer")?;
    let layers = m.getattr("_layers")?;
    let rev = scene.getattr("version")?;
    let mut slot = opt(v, "_layer_paints")?;
    if slot.is_none() || !slot.get_item(0)?.eq(&rev)? {
        slot = (
            rev,
            PyDict::new(py),
            crate::host_context::core_module(layers.py())?.getattr("tag_styles")?.call_method1("layer_fx", (&scene,))?,
        )
            .into_pyobject(py)?
            .into_any();
        v.setattr("_layer_paints", &slot)?
    }
    let fx = slot.get_item(2)?;
    let own = if by {
        none(py)
    } else {
        with_front_tint(&worn, &style)?
    };
    if !by && !fx.is_truthy()? {
        return Ok(own);
    }
    let d = slot.get_item(1)?;
    let key = (
        by,
        crate::host_context::core_module(layers.py())?.getattr("tag_styles")?.call_method1("object_layer", (group,))?,
        m.call_method1("_material_sig", (&own,))?,
    )
        .into_pyobject(py)?;
    let hit = d.call_method1("get", (&key,))?;
    if !hit.is_none() {
        return Ok(hit);
    }
    let base = if by {
        crate::host_context::core_module(layers.py())?.getattr("tag_styles")?.call_method1("layer_paint", (&scene, group))?
    } else {
        own
    };
    let p = crate::host_context::core_module(layers.py())?.getattr("tag_styles")?.call_method1("layer_look", (&scene, group, base, fx))?;
    d.set_item(key, &p)?;
    Ok(p)
}
#[pyfunction]
fn shade_factor(v: &Obj, normal: &Obj) -> PyResult<f64> {
    let n = xyz(normal)?;
    if n.length() < 1e-9 {
        return Ok(0.84375);
    }
    let d = n.normalize().dot(xyz(&v.getattr("_LIGHT")?)?).abs();
    Ok(((0.62 + 0.38 * d) * 32.).round_ties_even() / 32.)
}
#[pyfunction]
fn shaded_color(v: &Obj, base: &Obj, normal: &Obj) -> PyResult<(f64, f64, f64)> {
    let shade = v
        .call_method1("_shade_factor", (normal,))?
        .extract::<f64>()?;
    let rgb: [f64; 3] = [
        base.get_item(0)?.extract()?,
        base.get_item(1)?.extract()?,
        base.get_item(2)?.extract()?,
    ];
    let [r, g, b] = rgb.map(|c| ((c * shade).min(1.) * 64.).round_ties_even() / 64.);
    Ok((r, g, b))
}
#[pyfunction]
fn smooth_vcol(
    face: &Obj,
    base: &Obj,
    flat_rgb: [f64; 3],
    smoother: &Obj,
    tris: &Obj,
    light: &Obj,
) -> PyResult<Vec<f64>> {
    let base: [f64; 3] = [
        base.get_item(0)?.extract()?,
        base.get_item(1)?.extract()?,
        base.get_item(2)?.extract()?,
    ];
    let corners = smoother.call_method1("corners", (face,))?;
    let flat = xyz(&smoother.call_method1("flat", (face,))?)?;
    let light = xyz(light)?;
    let mut cache = std::collections::HashMap::new();
    let mut out = Vec::new();
    for tri in tris.try_iter()? {
        for p in tri?.try_iter()? {
            let p = p?;
            let xyz = p.extract::<[f64; 3]>()?;
            let k = xyz.map(f64::to_bits);
            let rgb = if let Some(c) = cache.get(&k) {
                *c
            } else {
                let n = corners.call_method1("get", (&p,))?;
                let col = if n.is_none() {
                    flat_rgb
                } else {
                    let n = super::xyz(&n)?;
                    if n.dot(flat) > 0.99999 {
                        flat_rgb
                    } else {
                        let s = 0.62 + 0.38 * n.dot(light).abs();
                        base.map(|c| (c * s).min(1.))
                    }
                };
                cache.insert(k, col);
                col
            };
            out.extend(xyz);
            out.extend(rgb)
        }
    }
    Ok(out)
}
#[pyfunction]
fn curved_faces<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let set = pyo3::types::PySet::empty(py)?;
    let smooth = host(py)?.getattr("smooths")?;
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        if smooth.call1((&e,))?.is_truthy()? {
            let f = e.getattr("faces")?;
            if f.len()? > 1 {
                set.call_method1("update", (f,))?;
            }
        }
    }
    Ok(set.into_any())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(proto_wrapper, m)?)?;
    m.add_function(wrap_pyfunction!(flags_read, m)?)?;
    m.add_function(wrap_pyfunction!(front_tint, m)?)?;
    m.add_function(wrap_pyfunction!(with_front_tint, m)?)?;
    m.add_function(wrap_pyfunction!(layer_look_sig, m)?)?;
    m.add_function(wrap_pyfunction!(paint_of, m)?)?;
    m.add_function(wrap_pyfunction!(smooth_vcol, m)?)?;
    m.add_function(wrap_pyfunction!(curved_faces, m)?)?;

    m.add_function(wrap_pyfunction!(shade_factor, m)?)?;
    m.add_function(wrap_pyfunction!(shaded_color, m)?)?;
    Ok(())
}

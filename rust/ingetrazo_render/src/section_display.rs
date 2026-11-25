// SPDX-License-Identifier: GPL-3.0-or-later
//! Section display caches and world-space, layer-styled model linework.
use glam::DVec3;
use numpy::{AllowTypeChange, IntoPyArray, PyArray3, PyArrayLike3, PyArrayMethods};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::collections::HashMap;
type Obj<'py> = Bound<'py, PyAny>;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn cached<'py>(view: &Obj<'py>, name: &str) -> PyResult<Option<Obj<'py>>> {
    Ok(view.getattr_opt(name)?.filter(|x| !x.is_none()))
}
fn hit<'py>(view: &Obj<'py>, name: &str, key: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    if let Some(old) = cached(view, name)? {
        if old.get_item(0)?.eq(key)? {
            return Ok(Some(old.get_item(1)?));
        }
    }
    Ok(None)
}

#[pyfunction]
fn cut_layer_tags<'py>(idx: &Obj<'py>, rows: Vec<usize>) -> PyResult<Obj<'py>> {
    let py = idx.py();
    let layers = py.import("core.layers")?;
    let default = layers.getattr("DEFAULT_LAYER")?;
    let layer_of = layers.getattr("layer_of")?;
    let entities = idx.getattr("entities")?;
    let fast = entities.getattr_opt("layer_at")?.filter(|f| !f.is_none());
    let mut memo = HashMap::new();
    let out = PyList::empty(py);
    for i in rows {
        if let std::collections::hash_map::Entry::Vacant(v) = memo.entry(i) {
            let (mut tag, owner) = if let Some(f) = &fast {
                let (own, owner): (Obj, Obj) = f.call1((i,))?.extract()?;
                (
                    if own.is_truthy()? {
                        own
                    } else {
                        default.clone()
                    },
                    owner,
                )
            } else {
                let (face, owner): (Obj, Obj) = entities.get_item(i)?.extract()?;
                (layer_of.call1((face,))?, owner)
            };
            if !owner.is_none() && tag.eq(&default)? {
                tag = layer_of.call1((owner,))?
            }
            v.insert(tag);
        }
        out.append(&memo[&i])?;
    }
    py.import("numpy")?.call_method1("asarray", (out, "object"))
}

#[pyfunction]
fn section_cut<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let scene = view.getattr("scene")?;
    let plane = scene.call_method0("active_section")?;
    if plane.is_none() {
        return Ok(none(py));
    }
    let n = plane.getattr("normal")?;
    let p = plane.getattr("point")?;
    let mut key = vec![scene.getattr("version")?];
    for (v, places) in [(&n, 9), (&p, 6)] {
        for axis in ["x", "y", "z"] {
            key.push(v.call_method0(axis)?.call_method1("__round__", (places,))?)
        }
    }
    let key = PyTuple::new(py, key)?.into_any();
    if let Some(segs) = hit(view, "_section_cut_cache", &key)? {
        return Ok(segs);
    }
    let kw = PyDict::new(py);
    kw.set_item("near", "all")?;
    let idx = view.call_method1(
        "_pick_flat",
        (view.call_method("_pick_index", (), Some(&kw))?,),
    )?;
    let v0 = idx.getattr("tri_v0")?;
    let (mut segs, mut tags) = (none(py), none(py));
    if !v0.is_none() && v0.len()? > 0 {
        let sec = py.import("core.section")?;
        let e1 = idx.getattr("tri_e1")?;
        let e2 = idx.getattr("tri_e2")?;
        let reach = match cached(view, "_section_reach")? {
            Some(r) if r.call_method1("fits", (&v0, &n, &e1, &e2))?.is_truthy()? => r,
            _ => {
                let r = crate::host_context::core_module(sec.py())?.getattr("PlaneReach")?.call1((&v0, &e1, &e2, &n))?;
                view.setattr("_section_reach", &r)?;
                r
            }
        };
        let ent = idx.getattr("tri_ent")?;
        let (cut, rows): (Obj, Obj) = reach
            .call_method1("cut", (p, idx.getattr("ent_vis")?.get_item(&ent)?, true))?
            .extract()?;
        if !cut.is_none() {
            tags = cut_layer_tags(&idx, ent.get_item(rows)?.extract()?)?
        }
        segs = cut;
    }
    view.setattr("_section_cut_cache", (key, &segs, tags))?;
    Ok(segs)
}

fn normal<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    view.getattr("scene")?
        .call_method0("active_section")?
        .getattr("normal")
}

#[pyfunction]
fn section_fill<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let segs = view.call_method0("_section_cut_segments")?;
    let old = cached(view, "_section_fill_cache")?;
    if let Some(cache) = &old {
        let prev = cache.get_item(0)?;
        if prev.is(&segs)
            || (!prev.is_none()
                && !segs.is_none()
                && py
                    .import("numpy")?
                    .call_method1("array_equal", (prev, &segs))?
                    .is_truthy()?)
        {
            let tris = cache.get_item(1)?;
            view.setattr("_section_fill_cache", (segs, &tris))?;
            return Ok(tris);
        }
    }
    let now: f64 = py.import("time")?.call_method0("monotonic")?.extract()?;
    let last = cached(view, "_section_fill_changed")?.map_or(Ok(0.), |v| v.extract::<f64>())?;
    view.setattr("_section_fill_changed", now)?;
    if old.is_some() && now - last < 0.15 {
        py.import("core.timers")?
            .call_method1("later", (view, 200, view.getattr("update")?))?;
        return Ok(none(py));
    }
    let tris = crate::host_context::core_module(py)?
        .call_method1("section_fill_triangles", (&segs, normal(view)?))?;
    view.setattr("_section_fill_cache", (segs, &tris))?;
    view.setattr("_section_hatch_cache", PyDict::new(py))?;
    Ok(tris)
}

#[pyfunction]
fn section_layer_fills<'py>(view: &Obj<'py>, color: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    if view.call_method0("_section_fill_tris")?.is_none() {
        return Ok(none(py));
    }
    let Some(cut) = cached(view, "_section_cut_cache")? else {
        return Ok(none(py));
    };
    let segs = cut.get_item(1)?;
    if segs.is_none() {
        return Ok(none(py));
    }
    let scene = view.getattr("scene")?;
    let layers = PyList::empty(py);
    for layer in scene.getattr("layers")?.try_iter()? {
        layers.append(layer?.call_method0("to_dict")?)?
    }
    let rgb = PyTuple::new(py, color.try_iter()?.take(3).collect::<PyResult<Vec<_>>>()?)?;
    let key = (
        segs.as_ptr() as usize,
        rgb,
        layers.repr()?,
        scene
            .getattr_opt("bricks")?
            .unwrap_or_else(|| none(py))
            .repr()?,
    )
        .into_pyobject(py)?
        .into_any();
    if let Some(out) = hit(view, "_section_layer_cache", &key)? {
        return Ok(out);
    }
    let out = py.import("core.section")?.call_method1(
        "layer_fills",
        (segs, cut.get_item(2)?, normal(view)?, scene, color),
    )?;
    view.setattr("_section_layer_cache", (key, &out))?;
    Ok(out)
}

#[pyfunction]
fn section_hatch<'py>(view: &Obj<'py>, spacing: &Obj<'py>, cross: bool) -> PyResult<Obj<'py>> {
    let py = view.py();
    let Some(cache) = cached(view, "_section_fill_cache")? else {
        return Ok(none(py));
    };
    let memo = match cached(view, "_section_hatch_cache")? {
        Some(d) => d,
        None => {
            let d = PyDict::new(py);
            view.setattr("_section_hatch_cache", &d)?;
            d.into_any()
        }
    };
    let key = (spacing.call_method1("__round__", (9,))?, cross);
    if memo.contains(key.clone())? {
        return memo.get_item(key);
    }
    let lines = crate::host_context::core_module(py)?.call_method1(
        "section_hatch_segments",
        (cache.get_item(0)?, normal(view)?, spacing, cross),
    )?;
    memo.set_item(key, &lines)?;
    Ok(lines)
}

#[pyfunction]
fn cut_weights<'py>(view: &Obj<'py>, plane: &Obj<'py>, keep: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let n: usize = keep.call_method0("sum")?.extract()?;
    let style = crate::host_context::core_module(py)?
        .call_method1("section_effective_fill", (plane, view.getattr("_frame_style")?))?;
    let mut output = vec![1.; n];
    if style.get_item(0)?.eq("layers")? {
        if let Some(cut) = cached(view, "_section_cut_cache")? {
            let tags = cut.get_item(2)?;
            if !tags.is_none() && tags.len()? == keep.len()? {
                let mut pens = HashMap::new();
                let unit: f64 = view.getattr("CUT_PEN_MM")?.extract()?;
                for layer in view.getattr("scene")?.getattr("layers")?.try_iter()? {
                    let layer = layer?;
                    if let Some(w) = cached(&layer, "cut_weight")? {
                        pens.insert(
                            layer.getattr("key")?.extract::<String>()?,
                            (w.extract::<f64>()? / unit).clamp(0.4, 2.4),
                        );
                    }
                }
                if !pens.is_empty() {
                    for (weight, tag) in output.iter_mut().zip(tags.get_item(keep)?.try_iter()?) {
                        if let Ok(key) = tag?.extract::<String>() {
                            *weight = *pens.get(&key).unwrap_or(&1.);
                        }
                    }
                }
            }
        }
    }
    Ok(output.into_pyarray(py).into_any())
}

#[pyfunction]
fn model_line_words<'py>(
    py: Python<'py>,
    segs: &Obj<'py>,
    spec: &Obj<'py>,
) -> PyResult<Bound<'py, PyArray3<f32>>> {
    let mut words = Vec::<f32>::new();
    if !spec.is_none() && segs.len()? > 0 {
        let strokes = crate::text_outlines::line_text_strokes(
            py,
            &spec.get_item(0)?.extract::<String>()?,
            spec.get_item(1)?.extract()?,
        );
        let layers = py.import("core.layers")?;
        let unit: f64 = layers.getattr("MODEL_DASH_SCALE")?.extract()?;
        let marks = layers.getattr("text_marks")?;
        let shape = py
            .import("numpy")?
            .call_method1("asarray", (segs,))?
            .call_method1("reshape", (-1, 2, 3))?;
        let arr = shape.extract::<PyArrayLike3<f64, AllowTypeChange>>()?;
        let arr = arr.as_array();
        for edge in arr.outer_iter() {
            if strokes.is_empty() {
                break;
            }
            let at = |i: usize| DVec3::new(edge[[i, 0]], edge[[i, 1]], edge[[i, 2]]);
            let (mut a, b) = (at(0), at(1));
            let d = b - a;
            let len = d.length();
            if len <= 1e-9 {
                continue;
            }
            let mut u = d / len;
            if u.x < -1e-9 || (u.x.abs() <= 1e-9 && u.y < 0.) {
                u = -u;
                a = b
            }
            let side = DVec3::Z.cross(u);
            let side = if side.length() < 1e-6 {
                DVec3::Y
            } else {
                side.normalize()
            };
            let centres: Vec<(f64, f64, f64)> =
                marks.call1(((0., 0.), (len / unit, 0.), spec))?.extract()?;
            if centres.len() * strokes.len() > 4000 {
                continue;
            }
            for (x, _, _) in centres {
                let centre = a + u * (x * unit);
                for poly in &strokes {
                    for pair in poly.windows(2) {
                        for &(x, y) in pair {
                            words.extend(
                                (centre + u * (x * unit) + side * (y * unit))
                                    .as_vec3()
                                    .to_array(),
                            );
                        }
                    }
                }
            }
        }
    }
    let n = words.len() / 6;
    Ok(words.into_pyarray(py).reshape([n, 2, 3])?)
}

#[pyfunction]
fn model_layer_lines<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let layers = py.import("core.layers")?;
    let scene = view.getattr("scene")?;
    let styles = crate::host_context::core_module(layers.py())?.getattr("tag_styles")?.call_method1("model_line_styles", (&scene,))?;
    if styles.len()? == 0 {
        return Ok(PyList::empty(py).into_any());
    }
    let entries = py
        .import("builtins")?
        .call_method1("sorted", (styles.call_method0("items")?,))?;
    let key = (
        scene.as_ptr() as usize,
        scene.getattr("content_version")?,
        entries,
    )
        .into_pyobject(py)?
        .into_any();
    if let Some(lines) = hit(view, "_model_lines_cache", &key)? {
        return Ok(lines);
    }
    let kw = PyDict::new(py);
    kw.set_item("tags", true)?;
    let geometry = view.call_method("hlr_geometry", (), Some(&kw))?;
    let groups = PyDict::new(py);
    for (seg, tag) in geometry
        .get_item(1)?
        .try_iter()?
        .zip(geometry.get_item(5)?.try_iter()?)
    {
        let tag = tag?;
        if styles.contains(&tag)? {
            let list = match groups.get_item(&tag)? {
                Some(list) => list,
                None => {
                    let list = PyList::empty(py);
                    groups.set_item(&tag, &list)?;
                    list.into_any()
                }
            };
            list.call_method1("append", (seg?,))?;
        }
    }
    let names = PyDict::new(py);
    for l in scene.getattr("layers")?.try_iter()? {
        let l = l?;
        names.set_item(l.getattr("key")?, l.getattr("linetype")?)?
    }
    let lines = PyList::empty(py);
    let np = py.import("numpy")?;
    for (tag, edges) in groups.iter() {
        let (color, pattern): (Obj, Obj) = styles.get_item(&tag)?.extract()?;
        let segs = np.call_method1("asarray", (edges,))?;
        let (dash, gap): (Obj, Obj) = layers
            .call_method1("dash_segments", (&segs, pattern))?
            .extract()?;
        let spec = crate::host_context::core_module(layers.py())?.getattr("tag_styles")?.call_method1("linetype_text", (names.get_item(tag)?, &scene))?;
        lines.append((color, dash, gap, model_line_words(py, &segs, &spec)?))?;
    }
    view.setattr("_model_lines_cache", (key, &lines))?;
    Ok(lines.into_any())
}

pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(cut_layer_tags, m)?)?;
    m.add_function(wrap_pyfunction!(section_cut, m)?)?;
    m.add_function(wrap_pyfunction!(section_fill, m)?)?;
    m.add_function(wrap_pyfunction!(section_layer_fills, m)?)?;
    m.add_function(wrap_pyfunction!(section_hatch, m)?)?;
    m.add_function(wrap_pyfunction!(cut_weights, m)?)?;
    m.add_function(wrap_pyfunction!(model_line_words, m)?)?;
    m.add_function(wrap_pyfunction!(model_layer_lines, m)?)?;
    Ok(())
}

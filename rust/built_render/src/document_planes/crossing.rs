// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use numpy::PyReadonlyArray1;
use std::collections::BTreeSet;

fn pick_index<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let near = host(v.py())?
        .getattr("DocumentInteraction")?
        .call_method1("_ray_near", (v, x, y))?;
    let kw = PyDict::new(v.py());
    kw.set_item("near", near)?;
    v.call_method("_pick_index", (), Some(&kw))
}
#[pyfunction]
fn front_face_at<'py>(v: &Obj<'py>, x: f64, y: f64, which: &str) -> PyResult<Obj<'py>> {
    let py = v.py();
    let (o, d): (Obj, Obj) = v.call_method1("_pixel_to_ray", (x, y))?.extract()?;
    if o.is_none() || d.is_none() {
        return Ok(none(py));
    }
    let index = pick_index(v, x, y)?;
    let native = call_opt(v, "_lazy_native")?;
    let module = host(py)?;
    if !native.is_none() && module.getattr("_FRONT_LEAN")?.is_truthy()? {
        return v.call_method1("_front_face_lean", (index, native, o, d, which));
    }
    let hits = module
        .getattr("DocumentInteraction")?
        .call_method1("_face_hits", (v, index, o, d, which))?;
    if hits.is_none() {
        return Ok(none(py));
    }
    let depths = hits.getattr("t")?;
    if depths.len()? == 0 {
        return Ok(none(py));
    }
    let i = depths.call_method0("argmin")?.extract::<usize>()?;
    Ok((
        hits.call_method1("ent", (i,))?.get_item(0)?,
        hits.call_method1("place", (i,))?,
    )
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn front_face_lean<'py>(
    v: &Obj<'py>,
    idx: &Obj<'py>,
    nat: &Obj<'py>,
    o: &Obj<'py>,
    d: &Obj<'py>,
    which: &str,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let module = host(py)?;
    let mut best = none(py);
    let mut best_t = f64::INFINITY;
    let entities = idx.getattr("entities")?;
    if entities.len()? > 0 {
        let mask = match which {
            "infer" => idx
                .getattr_opt("ent_infer")?
                .unwrap_or(idx.getattr("ent_sel")?),
            "sel" => idx.getattr("ent_sel")?,
            _ => idx.getattr("ent_vis")?,
        };
        let kw = PyDict::new(py);
        kw.set_item("own", false)?;
        let ts = v.call_method("_ray_hits", (idx, o, d, mask), Some(&kw))?;
        if !ts.is_none() && ts.len()? > 0 {
            let i = ts.call_method0("argmin")?.extract::<usize>()?;
            let t = ts.get_item(i)?.extract::<f64>()?;
            if t.is_finite() {
                best_t = t;
                let pidx = opt(idx, "ent_place_idx")?;
                let pi = if !pidx.is_none() && i < pidx.len()? {
                    pidx.get_item(i)?.extract::<i64>()?
                } else {
                    -1
                };
                let place = if pi >= 0 {
                    idx.getattr("ent_placements")?.get_item(pi)?
                } else {
                    module.call_method1("_entity_owner", (&entities, i))?
                };
                best = (entities.get_item(i)?.get_item(0)?, place)
                    .into_pyobject(py)?
                    .into_any()
            }
        }
    }
    let rows = match which {
        "sel" => module
            .getattr("DocumentInteraction")?
            .call_method1("_lazy_sel_rows", (v, nat))?,
        "infer" => nat.get_item(1)?,
        _ => nat.get_item(2)?,
    };
    if rows.len()? == 0 {
        return Ok(best);
    }
    let np = py.import("numpy")?;
    let rows = np.call_method1("ascontiguousarray", (rows, np.getattr("int64")?))?;
    let oo = o.call_method0("toTuple")?;
    let dd = d.call_method0("toTuple")?;
    let (pl, tr, v0, e1, e2): (Obj, Obj, Obj, Obj, Obj) = nat
        .get_item(0)?
        .call_method1("ray_cands", (rows, &oo, &dd, best_t * 1.001 + 1e-6))?
        .extract()?;
    if pl.len()? == 0 {
        return Ok(best);
    }
    let (ts, hit): (PyReadonlyArray1<f64>, PyReadonlyArray1<bool>) = py
        .import("built_core")?
        .call_method1("ray_triangles", (oo, dd, v0, e1, e2))?
        .extract()?;
    let ts = ts.as_array();
    let hit = hit.as_array();
    let nearest = ts
        .iter()
        .zip(hit.iter())
        .filter(|(t, h)| **h && **t < best_t)
        .map(|(t, _)| *t)
        .reduce(f64::min);
    let Some(tmin) = nearest else { return Ok(best) };
    let groups = v.call_method0("_pick_lazy_table")?.get_item(0)?;
    let pos = v.getattr("_pick_cands_memo")?.get_item(3)?;
    let mut key = None;
    for (k, (&t, &valid)) in ts.iter().zip(hit.iter()).enumerate() {
        if !valid || t != tmin {
            continue;
        }
        let g = groups.get_item(pl.get_item(k)?)?;
        let base = v.call_method1(
            "_proto_base_chunk",
            (
                g.getattr("mesh")?,
                module.call_method1("_paint_of", (v, &g))?,
            ),
        )?;
        let j = base
            .get_item("tri_ent")?
            .get_item(tr.get_item(k)?)?
            .extract::<usize>()?;
        let rank = (pos.get_item(id(&g))?.get_item(0)?.extract::<usize>()?, j);
        if key.is_none_or(|old| rank < old) {
            key = Some(rank);
            best = (base.get_item("faces")?.get_item(j)?, g)
                .into_pyobject(py)?
                .into_any()
        }
    }
    Ok(best)
}
#[pyfunction]
fn face_crossing_point<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let module = host(py)?;
    if !module
        .call_method1("_active_cut", (v.getattr("scene")?,))?
        .is_none()
    {
        return Ok(none(py));
    }
    let (f, fp): (Obj, Obj) = v.call_method1("pick_face_inference", (x, y))?.extract()?;
    if f.is_none() {
        return Ok(none(py));
    }
    let (o, d): (Obj, Obj) = v.call_method1("_pixel_to_ray", (x, y))?.extract()?;
    if o.is_none() || d.is_none() {
        return Ok(none(py));
    }
    let hits = v.call_method1("_infer_hits", (pick_index(v, x, y)?, &o, &d))?;
    if hits.is_none() {
        return Ok(none(py));
    }
    let depths = hits.getattr("t")?.extract::<PyReadonlyArray1<f64>>()?;
    let ts = depths.as_array();
    let Some(t) = ts.iter().copied().reduce(|a, b| {
        if a.is_nan() || b.is_nan() {
            f64::NAN
        } else {
            a.min(b)
        }
    }) else {
        return Ok(none(py));
    };
    if !t.is_finite() {
        return Ok(none(py));
    }
    let r = v.getattr("_CROSSING_RING_PX")?.extract::<f64>()?;
    let (o2, d2): (Obj, Obj) = v.call_method1("_pixel_to_ray", (x + r, y))?.extract()?;
    if o2.is_none() || d2.is_none() {
        return Ok(none(py));
    }
    let reach = ((xyz(&o2)? + xyz(&d2)? * t) - (xyz(&o)? + xyz(&d)? * t)).length() * 50.;
    let mut cand: Vec<_> = ts
        .iter()
        .enumerate()
        .filter(|(_, d)| **d >= t && **d <= t + reach)
        .map(|(i, &d)| (i, d))
        .collect();
    if cand.is_empty() {
        return Ok(none(py));
    }
    cand.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (pf, nf): (Obj, Obj) = plane_of(&f, &fp)?.extract()?;
    let front = (&f, &fp, pf, &nf).into_pyobject(py)?;
    let mut pairs = Vec::new();
    for (i, _) in cand.into_iter().take(7) {
        let g = hits.call_method1("ent", (i,))?.get_item(0)?;
        let gp = hits.call_method1("place", (i,))?;
        if g.eq(&f)? && gp.is(&fp) {
            continue;
        }
        let (pg, ng): (Obj, Obj) = plane_of(&g, &gp)?.extract()?;
        let tangent = xyz(&nf)?.cross(xyz(&ng)?);
        if tangent.length() < 0.02 {
            continue;
        }
        let back = (g, gp, pg, ng, vec3(py, tangent)?).into_pyobject(py)?;
        let sides = v.call_method1("_crossing_sides", (x, y, &o, &d, &front, &back))?;
        if !sides.is_none() {
            pairs.push((back, sides))
        }
    }
    let mat = opt(v, "_pick_materialize")?;
    if !mat.is_none()
        && v.hasattr("_pick_lazy_table")?
        && !module
            .getattr("DocumentInteraction")?
            .call_method1("_ray_near", (v, x, y))?
            .eq("skip")?
    {
        for (_, s) in &pairs {
            for q in s.get_item(1)?.try_iter()? {
                let q = q?;
                mat.call1((("px", q.get_item(0)?, q.get_item(1)?),))?;
            }
        }
    }
    for (back, sides) in pairs {
        let p = v.call_method1("_crossing_on_screen", (&front, back, sides))?;
        if !p.is_none() {
            return Ok(p);
        }
    }
    Ok(none(py))
}
#[pyfunction]
fn crossing_sides<'py>(
    v: &Obj<'py>,
    x: f64,
    y: f64,
    o: &Obj<'py>,
    d: &Obj<'py>,
    f: &Obj<'py>,
    g: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let radius = v
        .getattr("edge_snap_threshold_px")?
        .extract::<f64>()?
        .max(v.getattr("_CROSSING_RING_PX")?.extract()?);
    v.py().import("core.snap")?.call_method1(
        "face_crossing_near_ray",
        (
            x,
            y,
            o,
            d,
            (f.get_item(2)?, f.get_item(3)?),
            (g.get_item(2)?, g.get_item(3)?),
            v.getattr("_world_to_pixel")?,
            radius,
            v.getattr("_CROSSING_SIDE_PX")?,
        ),
    )
}
#[pyfunction]
fn crossing_on_screen<'py>(
    v: &Obj<'py>,
    f: &Obj<'py>,
    g: &Obj<'py>,
    found: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let module = host(py)?;
    let identity = module.getattr("element_identity")?;
    let faces = [
        identity.call1((f.get_item(0)?,))?,
        identity.call1((g.get_item(0)?,))?,
    ];
    let places = [f.get_item(1)?, g.get_item(1)?];
    let mut seen = BTreeSet::new();
    for q in found.get_item(1)?.try_iter()? {
        let q = q?;
        let h = v.call_method1("_front_face_at", (q.get_item(0)?, q.get_item(1)?, "infer"))?;
        if h.is_none() {
            return Ok(none(py));
        }
        let face = identity.call1((h.get_item(0)?,))?;
        let place = h.get_item(1)?;
        let mut matched = false;
        for i in 0..2 {
            if place.is(&places[i]) && face.eq(&faces[i])? {
                seen.insert(i);
                matched = true
            }
        }
        if !matched {
            return Ok(none(py));
        }
    }
    if seen.len() != 2 {
        return Ok(none(py));
    }
    let mut context = none(py);
    for place in places {
        if place.is_none() {
            continue;
        }
        let word = module.call_method1("_snap_context_of", (v, place))?;
        if context.is_none() || word.eq("outside")? {
            context = word
        }
    }
    Ok((found.get_item(0)?, context).into_pyobject(py)?.into_any())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(front_face_at, m)?)?;
    m.add_function(wrap_pyfunction!(front_face_lean, m)?)?;
    m.add_function(wrap_pyfunction!(face_crossing_point, m)?)?;
    m.add_function(wrap_pyfunction!(crossing_sides, m)?)?;
    m.add_function(wrap_pyfunction!(crossing_on_screen, m)?)?;
    Ok(())
}

// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use numpy::{IntoPyArray, PyArrayMethods};
#[pyfunction]
fn billboard_placements<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let groups = view.call_method0("_placements")?;
    let epoch = optional_call(view, "_placements_epoch")?;
    let key = (&epoch, id(&groups), groups.len()?).into_pyobject(py)?;
    let memo = opt(view, "_billboard_memo")?;
    if !epoch.is_none()
        && !memo.is_none()
        && memo.get_item(0)?.eq(&key)?
        && memo.get_item(1)?.is(&groups)
    {
        return memo.get_item(2);
    }
    let found = PyList::empty(py);
    for g in groups.try_iter()? {
        let g = g?;
        if opt(&g, "billboard")?.is_truthy()? {
            found.append(g)?
        }
    }
    if !epoch.is_none() {
        view.setattr("_billboard_memo", (key, groups, &found))?
    }
    Ok(found.into_any())
}
#[pyfunction]
fn billboard_snap_edges<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let scene = view.getattr("scene")?;
    let moving = opt(&view.getattr("active_tool")?, "_group")?;
    let out = PyList::empty(py);
    for g in view.call_method0("_billboard_placements")?.try_iter()? {
        let g = g?;
        if g.is(&moving) || !scene.call_method1("group_snappable", (&g,))?.is_truthy()? {
            continue;
        }
        let quad = view.call_method1("_billboard_quad", (g,))?;
        if !quad.is_none() {
            let c = quad.get_item(0)?;
            let feet = (xyz(&c.get_item(0)?)? + xyz(&c.get_item(1)?)?) * 0.5;
            out.append(point(
                &Bound::new(py, Vec3::from(feet))?.into_any(),
                Some("figure"),
                None,
            )?)?
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn component_origin_points<'py>(view: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = view.py();
    let h = host(py)?;
    let scene = view.getattr("scene")?;
    let key = (
        h.call_method1("_content_ver", (view,))?,
        id(&scene.getattr("edit_group")?),
    )
        .into_pyobject(py)?;
    let old = opt(view, "_corigin_cache")?;
    let (groups, points) = if !old.is_none() && old.get_item(0)?.eq(&key)? {
        (old.get_item(1)?, old.get_item(2)?)
    } else {
        let groups = PyList::empty(py);
        let mut coords = Vec::new();
        for g in view.call_method0("_placements")?.try_iter()? {
            let g = g?;
            let xf = opt(&g, "xform")?;
            if xf.is_none() || opt(&g, "billboard")?.is_truthy()? {
                continue;
            }
            coords.extend(
                xyz(&xf.call_method1("map", (Vec3::from(glam::DVec3::ZERO),))?)?.to_array(),
            );
            groups.append(g)?;
        }
        let points = coords
            .into_pyarray(py)
            .reshape([groups.len(), 3])?
            .into_any();
        view.setattr("_corigin_cache", (key, &groups, &points))?;
        (groups.into_any(), points)
    };
    let out = PyList::empty(py);
    if groups.len()? == 0 {
        return Ok(out.into_any());
    }
    let projected = view.call_method1("_project_px", (&points,))?;
    let (sx, sy, ok): (
        numpy::PyReadonlyArray1<f64>,
        numpy::PyReadonlyArray1<f64>,
        numpy::PyReadonlyArray1<bool>,
    ) = projected.extract()?;
    let th: f64 = view.getattr("POINT_INFERENCE_PX")?.extract()?;
    let cap: usize = h
        .getattr("DocumentInteraction")?
        .getattr("GEDGE_ENOUGH")?
        .extract()?;
    let moving = opt(&view.getattr("active_tool")?, "_group")?;
    let mut rows = Vec::new();
    for (i, ((&sx, &sy), &ok)) in sx
        .as_array()
        .iter()
        .zip(sy.as_array())
        .zip(ok.as_array())
        .enumerate()
    {
        let d = (sx - x).hypot(sy - y);
        if ok && d <= th {
            rows.push((i, d))
        }
    }
    if rows.len() > cap {
        rows.sort_by(|a, b| a.1.total_cmp(&b.1));
        rows.truncate(cap + 1)
    }
    for (i, _) in rows {
        let g = groups.get_item(i)?;
        if !g.is(&moving) && out.len() < cap {
            out.append(point(
                &points.get_item(i)?,
                Some("component_origin"),
                Some(&context(view, &g)?),
            )?)?
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn mesh_arc_midpoints<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let h = host(py)?;
    let plan = py
        .import("ingetrazo_core")?
        .call_method1("plan_of", (mesh,))?;
    let curves = PyDict::new(py);
    let identity = h.getattr("element_identity")?;
    let edge = |curve: &Obj, keys: [Obj; 2], positions: [Obj; 2]| -> PyResult<()> {
        let vertices = match curves.get_item(curve)? {
            Some(v) => v,
            None => {
                let v = PyDict::new(py);
                curves.set_item(curve, &v)?;
                v.into_any()
            }
        };
        for (k, p) in keys.into_iter().zip(positions) {
            let n = vertices.call_method1("get", (&k,))?;
            let count = if n.is_none() {
                0
            } else {
                n.get_item(0)?.extract::<usize>()?
            };
            vertices.set_item(k, (count + 1, p))?
        }
        Ok(())
    };
    if plan.is_none() {
        for e in mesh.getattr("edges")?.try_iter()? {
            let e = e?;
            let curve = opt(&e, "curve")?;
            if curve.is_none() {
                continue;
            }
            let a = e.getattr("v0")?;
            let b = e.getattr("v1")?;
            edge(
                &curve,
                [identity.call1((&a,))?, identity.call1((&b,))?],
                [a.getattr("position")?, b.getattr("position")?],
            )?
        }
    } else {
        let rows = h.call_method1("_plan_curved_edges", (&plan,))?;
        let edges = plan
            .getattr("edges")?
            .get_item(&rows)?
            .call_method0("tolist")?;
        let pos = plan.getattr("pos")?;
        let curves = plan.getattr("curve")?;
        for (i, ab) in rows
            .call_method0("tolist")?
            .try_iter()?
            .zip(edges.try_iter()?)
        {
            let [a, b]: [usize; 2] = ab?.extract()?;
            edge(
                &curves.get_item(i?)?,
                [
                    a.into_pyobject(py)?.into_any(),
                    b.into_pyobject(py)?.into_any(),
                ],
                [
                    Bound::new(py, Vec3::from(xyz(&pos.get_item(a)?)?))?.into_any(),
                    Bound::new(py, Vec3::from(xyz(&pos.get_item(b)?)?))?.into_any(),
                ],
            )?
        }
    }
    let out = PyList::empty(py);
    let fit = py.import("core.snap")?.getattr("arc_sweep_midpoint")?;
    for (_, verts) in curves.iter() {
        if verts.len()? < 3 {
            continue;
        }
        let points = PyList::empty(py);
        let mut ends = 0;
        for item in verts.call_method0("values")?.try_iter()? {
            let item = item?;
            if item.get_item(0)?.extract::<usize>()? == 1 {
                ends += 1
            }
            points.append(item.get_item(1)?)?
        }
        if ends == 2 {
            let mid = fit.call1((points,))?;
            if !mid.is_none() {
                out.append(mid)?
            }
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn arc_midpoints<'py>(view: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = view.py();
    let scene = view.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let key = (
        host(py)?.call_method1("_content_ver", (view,))?,
        id(&mesh),
        id(&scene.getattr("edit_group")?),
        optional_call(view, "_rest_is_hidden")?.is_truthy()?,
    )
        .into_pyobject(py)?
        .into_any();
    let mids = if let Some(m) = cache(view, "_arc_mid_cache", &key)? {
        m
    } else {
        let old = dict_slot(view, "_arc_mid_by_mesh")?;
        let seen = PyDict::new(py);
        let mids = PyList::empty(py);
        let local = |m: &Obj<'py>| -> PyResult<Obj<'py>> {
            let k = id(m);
            let serial = opt(m, "_mut_serial")?;
            let mut hit = seen.get_item(k)?.unwrap_or(old.call_method1("get", (k,))?);
            if hit.is_none() || !hit.get_item(0)?.eq(&serial)? || !hit.get_item(2)?.is(m) {
                hit = (serial, view.call_method1("_mesh_arc_midpoints", (m,))?, m)
                    .into_pyobject(py)?
                    .into_any()
            }
            seen.set_item(k, &hit)?;
            hit.get_item(1)
        };
        for m in local(&mesh)?.try_iter()? {
            mids.append((m?, py.None()))?
        }
        let outer = optional_call(view, "_outer_loose_meshes")?;
        if !outer.is_none() {
            for om in outer.try_iter()? {
                for m in local(&om?)?.try_iter()? {
                    mids.append((m?, "outside"))?
                }
            }
        }
        for g in view.call_method0("_placements")?.try_iter()? {
            let g = g?;
            let mesh = opt(&g, "mesh")?;
            if mesh.is_none() || opt(&g, "billboard")?.is_truthy()? {
                continue;
            }
            let local = local(&mesh)?;
            if local.len()? == 0 {
                continue;
            }
            let ctx = context(view, &g)?;
            let xf = opt(&g, "xform")?;
            for p in local.try_iter()? {
                let p = p?;
                let world = if xf.is_none() {
                    Bound::new(py, Vec3::from(xyz(&p)?))?.into_any()
                } else {
                    xf.call_method1("map", (p,))?
                };
                mids.append((world, &ctx))?
            }
        }
        view.setattr("_arc_mid_by_mesh", seen)?;
        view.setattr("_arc_mid_cache", (key, &mids))?;
        mids.into_any()
    };
    let out = PyList::empty(py);
    for item in mids.try_iter()? {
        let (m, ctx): (Obj, Obj) = item?.extract()?;
        if let Some((sx, sy)) = view
            .call_method1("_world_to_pixel", (&m,))?
            .extract::<Option<(f64, f64)>>()?
        {
            if (sx - x).hypot(sy - y) <= view.getattr("POINT_INFERENCE_PX")?.extract::<f64>()? {
                out.append(point(&m, Some("arc_midpoint"), Some(&ctx))?)?
            }
        }
    }
    Ok(out.into_any())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(component_origin_points, m)?)?;
    m.add_function(wrap_pyfunction!(billboard_placements, m)?)?;
    m.add_function(wrap_pyfunction!(billboard_snap_edges, m)?)?;
    m.add_function(wrap_pyfunction!(mesh_arc_midpoints, m)?)?;
    m.add_function(wrap_pyfunction!(arc_midpoints, m)?)?;
    Ok(())
}

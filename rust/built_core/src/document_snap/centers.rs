// SPDX-License-Identifier: GPL-3.0-or-later
//! Acquire curve centers and refresh the acquired handle after an edit.
use super::*;
use pyo3::types::PyFrozenSet;

fn version<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.getattr("scene")?.getattr("version")
}
fn placed<'py>(found: &Obj<'py>, xf: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    if found.is_none() {
        return Ok(found.clone());
    }
    let mut c = found.get_item(0)?;
    if !xf.is_none() {
        c = xf.call_method1("map", (c,))?
    }
    Ok((
        c,
        found.get_item(1)?,
        found.get_item(2)?,
        found.get_item(3)?,
        found.get_item(4)?,
        found.get_item(5)?,
        group,
    )
        .into_pyobject(found.py())?
        .into_any())
}
#[pyfunction]
fn update_center_ref<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let edge = v.getattr("_hover_edge")?;
    let scene = v.getattr("scene")?;
    let mut found = none(py);
    if !opt(&edge, "curve")?.is_none() {
        found = v.call_method1("_center_of_edge", (&edge, scene.getattr("mesh")?))?
    } else if opt(&edge, "in_group")?.is_truthy()? && !opt(&edge, "group")?.is_none() {
        found = v.call_method1("_center_of_group_edge", (edge,))?
    }
    if found.is_none() {
        let mut pick = opt(v, "pick_face_inference")?;
        if !pick.is_truthy()? {
            pick = opt(v, "pick_face_placement")?
        }
        if !pick.is_truthy()? {
            pick = v.getattr("pick_face_any")?
        }
        let (face, group): (Obj, Obj) = pick.call1((x, y))?.extract()?;
        if !face.is_none() {
            let mut mesh = opt(&group, "mesh")?;
            if mesh.is_none() {
                mesh = scene.getattr("mesh")?
            }
            found = v.call_method1("_center_of_face", (face, group, mesh, x, y))?
        }
    }
    if found.is_none() {
        v.call_method0("_valid_center_ref")?;
    } else {
        v.setattr("_center_ref", &found)?
    }
    Ok(found)
}
fn edge_key<'py>(a: &Obj<'py>, b: &Obj<'py>) -> PyResult<Bound<'py, PyFrozenSet>> {
    let py = a.py();
    let round = py.import("builtins")?.getattr("round")?;
    let vertex = |p: &Obj<'py>| -> PyResult<Obj<'py>> {
        let values = xyz(p)?
            .to_array()
            .map(|f| round.call1((f, 4)))
            .into_iter()
            .collect::<PyResult<Vec<_>>>()?;
        Ok(PyTuple::new(py, values)?.into_any())
    };
    PyFrozenSet::new(py, [vertex(a)?, vertex(b)?])
}
#[pyfunction]
fn center_of_group_edge<'py>(v: &Obj<'py>, pseudo: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let group = pseudo.getattr("group")?;
    let mesh = opt(&group, "mesh")?;
    let xf = opt(&group, "xform")?;
    if mesh.is_none() {
        return Ok(none(py));
    }
    let (mut a, mut b) = (pseudo.getattr("a")?, pseudo.getattr("b")?);
    if !xf.is_none() {
        let (inv, ok): (Obj, bool) = xf.call_method0("inverted")?.extract()?;
        if !ok {
            return Ok(none(py));
        }
        a = inv.call_method1("map", (a,))?;
        b = inv.call_method1("map", (b,))?
    }
    let h = host(py)?;
    if !h
        .call_method1("_lazy_edge_may_be_curve", (&mesh, &a, &b))?
        .is_truthy()?
    {
        return Ok(none(py));
    }
    let edges = mesh.getattr("edges")?;
    if !edges.is_truthy()? {
        return Ok(none(py));
    }
    let mut edge = none(py);
    if !h.getattr("_NO_SNAP_FAST")?.is_truthy()? {
        let va = mesh.call_method1("vertex_at", (&a,))?;
        let vb = mesh.call_method1("vertex_at", (&b,))?;
        if !va.is_none() && !vb.is_none() {
            edge = mesh.call_method1("find_edge", (va, vb))?
        }
    }
    if edge.is_none() {
        let ver = version(v)?;
        let old = opt(v, "_group_edge_tables")?;
        let tables = if !old.is_none() && old.get_item(0)?.eq(&ver)? {
            old.get_item(1)?
        } else {
            let t = PyDict::new(py);
            v.setattr("_group_edge_tables", (ver, &t))?;
            t.into_any()
        };
        let mut table = tables.call_method1("get", (id(&mesh),))?;
        if table.is_none() {
            table = PyDict::new(py).into_any();
            for e in edges.try_iter()? {
                let e = e?;
                table.set_item(edge_key(&e.getattr("a")?, &e.getattr("b")?)?, e)?
            }
            tables.set_item(id(&mesh), &table)?
        }
        edge = table.call_method1("get", (edge_key(&a, &b)?,))?;
    }
    if opt(&edge, "curve")?.is_none() {
        return Ok(none(py));
    }
    placed(
        &v.call_method1("_center_of_edge", (edge, mesh))?,
        &xf,
        &group,
    )
}
#[pyfunction]
fn center_of_edge<'py>(v: &Obj<'py>, edge: &Obj<'py>, mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let pts = PyDict::new(py);
    let identity = host(py)?.getattr("element_identity")?;
    for e in mesh.call_method1("curve_edges", (edge,))?.try_iter()? {
        let e = e?;
        for name in ["v0", "v1"] {
            let vertex = e.getattr(name)?;
            pts.set_item(identity.call1((&vertex,))?, vertex.getattr("position")?)?
        }
    }
    if pts.len() < 3 {
        return Ok(none(py));
    }
    let fit = py
        .import("built_core.snap")?
        .call_method1("fit_circle", (pts.values(),))?;
    if fit.is_none() {
        return Ok(fit);
    }
    Ok((
        fit.get_item(0)?,
        fit.get_item(1)?,
        ("edge", edge.getattr("curve")?),
        version(v)?,
        edge,
        mesh,
        py.None(),
    )
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn center_of_face<'py>(
    v: &Obj<'py>,
    face: &Obj<'py>,
    group: &Obj<'py>,
    mesh: &Obj<'py>,
    x: f64,
    y: f64,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let centers = py
        .import("built_core.snap")?
        .call_method1("curve_centers_of_face", (face, opt(group, "xform")?))?;
    let mut best = none(py);
    let mut distance = f64::INFINITY;
    for c in centers.try_iter()? {
        let c = c?;
        if centers.len()? == 1 {
            best = c;
            break;
        }
        let pc = v.call_method1("_world_to_pixel", (c.get_item(0)?,))?;
        if let Some([cx, cy]) = pc.extract::<Option<[f64; 2]>>()? {
            let d = (cx - x).hypot(cy - y);
            if best.is_none() || d < distance {
                distance = d;
                best = c
            }
        }
    }
    if best.is_none() {
        return Ok(best);
    }
    Ok((
        best.get_item(0)?,
        best.get_item(1)?,
        (
            host(py)?.call_method1("element_identity", (face,))?,
            best.get_item(2)?,
        ),
        version(v)?,
        face,
        mesh,
        group,
    )
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn center_hint_px(v: &Obj, reference: &Obj, pc: [f64; 2]) -> PyResult<f64> {
    let center = xyz(&reference.get_item(0)?)?;
    let r: f64 = reference.get_item(1)?.extract()?;
    let mut size: f64 = 0.;
    for offset in [glam::DVec3::X, glam::DVec3::Y] {
        if let Some([x, y]) = v
            .call_method1("_world_to_pixel", (Vec3::from(center + offset * r),))?
            .extract::<Option<[f64; 2]>>()?
        {
            size = size.max((x - pc[0]).hypot(y - pc[1]))
        }
    }
    Ok(if size < v.getattr("CENTER_MIN_RADIUS_PX")?.extract()? {
        0.
    } else {
        (size * 0.5).min(v.getattr("CENTER_HINT_PX")?.extract()?)
    })
}
#[pyfunction]
fn valid_center_ref<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let old = opt(v, "_center_ref")?;
    if old.is_none() {
        return Ok(old);
    }
    let ver = version(v)?;
    if old.get_item(3)?.eq(&ver)? {
        return Ok(old);
    }
    let mut src = old.get_item(4)?;
    let mesh = old.get_item(5)?;
    let group = old.get_item(6)?;
    let real = opt(&src, "real")?;
    let lazy = !real.is_none()
        && mesh
            .get_type()
            .is(&py.import("built_core.mesh")?.getattr("DeferredMesh")?);
    if !real.is_none() && !lazy {
        src = real.call0()?
    }
    let mut fresh = none(py);
    if src.is_instance(&host(py)?.getattr("Face")?)? {
        if lazy || mesh.getattr("faces")?.contains(&src)? {
            let key = old.get_item(2)?;
            let arc = key.get_item(1)?;
            for c in py
                .import("built_core.snap")?
                .call_method1("curve_centers_of_face", (&src, opt(&group, "xform")?))?
                .try_iter()?
            {
                let c = c?;
                if c.get_item(2)?.eq(&arc)? {
                    fresh = (c.get_item(0)?, c.get_item(1)?, key, ver, src, mesh, group)
                        .into_pyobject(py)?
                        .into_any();
                    break;
                }
            }
        }
    } else if mesh.getattr("edges")?.contains(&src)? {
        fresh = v.call_method1("_center_of_edge", (src, mesh))?;
        let xf = opt(&group, "xform")?;
        if !xf.is_none() {
            fresh = placed(&fresh, &xf, &group)?
        }
    }
    v.setattr("_center_ref", &fresh)?;
    Ok(fresh)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(update_center_ref, m)?)?;
    m.add_function(wrap_pyfunction!(center_of_group_edge, m)?)?;
    m.add_function(wrap_pyfunction!(center_of_edge, m)?)?;
    m.add_function(wrap_pyfunction!(center_of_face, m)?)?;
    m.add_function(wrap_pyfunction!(center_hint_px, m)?)?;
    m.add_function(wrap_pyfunction!(valid_center_ref, m)?)?;
    Ok(())
}

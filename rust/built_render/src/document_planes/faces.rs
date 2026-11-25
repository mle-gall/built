// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::PyFrozenSet;

fn moving(v: &Obj, face: &Obj) -> PyResult<bool> {
    let excluded = call_opt(&v.getattr("active_tool")?, "moving_face_ids")?;
    Ok(!excluded.is_none()
        && excluded.contains(host(v.py())?.call_method1("element_identity", (face,))?)?)
}
fn near(v: &Obj, p: &Obj, x: f64, y: f64) -> PyResult<bool> {
    let screen = v
        .call_method1("_world_to_pixel", (p,))?
        .extract::<Option<[f64; 2]>>()?;
    match screen {
        Some([px, py]) => {
            Ok((px - x).hypot(py - y) <= v.getattr("POINT_INFERENCE_PX")?.extract::<f64>()?)
        }
        None => Ok(false),
    }
}
#[pyfunction]
fn face_outline_world<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let (face, place): (Obj, Obj) = v.call_method1("pick_face_inference", (x, y))?.extract()?;
    if face.is_none() {
        return Ok(none(py));
    }
    let xf = opt(&place, "xform")?;
    let ring = |points: Obj<'py>| -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        for p in points.try_iter()? {
            let p = copied(&p?)?;
            out.append(if xf.is_none() {
                p
            } else {
                xf.call_method1("map", (p,))?
            })?
        }
        Ok(out)
    };
    let outer = ring(face.getattr("vertices")?)?;
    let holes = PyList::empty(py);
    for h in face.getattr("holes")?.try_iter()? {
        holes.append(ring(h?)?)?
    }
    let mut ctx = none(py);
    if !place.is_none() {
        let owner = v.call_method1("_owner_of", (&place,))?;
        let ic = opt(&owner, "is_instance")?;
        if !ic.is_none() {
            ctx = if ic.call0()?.is_truthy()? {
                "component"
            } else {
                "group"
            }
            .into_pyobject(py)?
            .into_any()
        }
    }
    Ok((outer, holes, ctx).into_pyobject(py)?.into_any())
}
#[pyfunction]
fn still_face_under(v: &Obj, x: f64, y: f64) -> PyResult<bool> {
    let (f, g): (Obj, Obj) = v.call_method1("pick_face_inference", (x, y))?.extract()?;
    Ok(!f.is_none() && (!g.is_none() || !moving(v, &f)?))
}
#[pyfunction]
fn face_center_under<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let (face, place): (Obj, Obj) = v.call_method1("pick_face_placement", (x, y))?.extract()?;
    if face.is_none() {
        return Ok(none(py));
    }
    let ctx = if place.is_none() {
        if moving(v, &face)? {
            return Ok(none(py));
        }
        none(py)
    } else {
        let owner = v.call_method1("_owner_of", (&place,))?;
        let excl = call_opt(&v.getattr("active_tool")?, "snap_excluded")?;
        if !excl.is_none() {
            let groups = excl.get_item(1)?;
            if groups.contains(id(&owner))? || groups.contains(id(&place))? {
                return Ok(none(py));
            }
        }
        if call_opt(&owner, "is_instance")?.is_truthy()? {
            "component"
        } else {
            "group"
        }
        .into_pyobject(py)?
        .into_any()
    };
    let c = py
        .import("core.snap")?
        .call_method1("face_center_of_face", (face, opt(&place, "xform")?))?;
    if !c.is_none() && near(v, &c, x, y)? {
        Ok((c, ctx).into_pyobject(py)?.into_any())
    } else {
        Ok(none(py))
    }
}
#[pyfunction]
fn selected_face_centers<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let module = host(py)?;
    let typ = module.getattr("Face")?;
    let scene = v.getattr("scene")?;
    let mut faces = Vec::new();
    for f in scene.getattr("selection")?.try_iter()? {
        let f = f?;
        if f.is_instance(&typ)? {
            faces.push(f)
        }
    }
    let pts = PyList::empty(py);
    if faces.is_empty()
        || faces.len() > v.getattr("FACE_CENTER_SELECTION_MAX")?.extract::<usize>()?
    {
        return Ok(pts.into_any());
    }
    let excluded = call_opt(&v.getattr("active_tool")?, "moving_face_ids")?;
    let moving = if excluded.is_none() {
        PyFrozenSet::empty(py)?
    } else {
        PyFrozenSet::new(py, excluded.try_iter()?.collect::<PyResult<Vec<_>>>()?)?
    };
    let identity = module.getattr("element_identity")?;
    let ids = faces
        .iter()
        .map(|f| identity.call1((f,)))
        .collect::<PyResult<Vec<_>>>()?;
    let key = (scene.getattr("version")?, PyTuple::new(py, &ids)?, &moving).into_pyobject(py)?;
    let cached = opt(v, "_sel_face_centers")?;
    if !cached.is_none() && cached.get_item(0)?.eq(&key)? {
        return cached.get_item(1);
    }
    let center = py.import("core.snap")?.getattr("face_center_of_face")?;
    for (f, i) in faces.iter().zip(ids) {
        if !moving.contains(i)? {
            let c = center.call1((f,))?;
            if !c.is_none() {
                pts.append(c)?
            }
        }
    }
    v.setattr("_sel_face_centers", (key, &pts))?;
    Ok(pts.into_any())
}
#[pyfunction]
fn face_centers_near<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let out = PyList::empty(py);
    let under = v.call_method1("_face_center_under", (x, y))?;
    if !under.is_none() {
        out.append(under)?
    }
    for c in v.call_method0("_selected_face_centers")?.try_iter()? {
        let c = c?;
        if near(v, &c, x, y)? {
            out.append((c, py.None()))?
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn face_context_under<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let (f, g): (Obj, Obj) = v.call_method1("pick_face_inference", (x, y))?.extract()?;
    if f.is_none() {
        Ok(none(v.py()))
    } else {
        host(v.py())?.call_method1("_snap_context_of", (v, g))
    }
}
#[pyfunction]
fn hover_face_plane<'py>(v: &Obj<'py>, cursor: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let (f, g): (Obj, Obj) = pixel(v, "pick_face_inference", cursor)?.extract()?;
    if f.is_none() {
        return Ok(none(py));
    }
    let sel = v.getattr("scene")?.getattr("selection")?;
    let owner = if g.is_none() {
        none(py)
    } else {
        v.call_method1("_owner_of", (&g,))?
    };
    if !locking(&v.getattr("active_tool")?)?
        && (!owner.is_none() && sel.contains(&owner)? || g.is_none() && sel.contains(&f)?)
        || g.is_none() && moving(v, &f)?
    {
        return Ok(none(py));
    }
    plane_of(&f, &g)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(face_outline_world, m)?)?;
    m.add_function(wrap_pyfunction!(still_face_under, m)?)?;
    m.add_function(wrap_pyfunction!(face_center_under, m)?)?;
    m.add_function(wrap_pyfunction!(selected_face_centers, m)?)?;
    m.add_function(wrap_pyfunction!(face_centers_near, m)?)?;
    m.add_function(wrap_pyfunction!(face_context_under, m)?)?;
    m.add_function(wrap_pyfunction!(hover_face_plane, m)?)?;
    Ok(())
}

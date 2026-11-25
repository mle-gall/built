// SPDX-License-Identifier: GPL-3.0-or-later
//! Acquire edge, rim and face references for interactive drawing tools.
use super::context::{foot, interaction, matrix, nearby_edges, reference_edge, Obj};
use crate::math::xyz;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};

pub(super) fn record<'p>(py: Python<'p>, fields: &Bound<'p, PyDict>) -> PyResult<Obj<'p>> {
    py.import("types")?
        .getattr("SimpleNamespace")?
        .call((), Some(fields))
}
#[pyfunction]
fn acquire_rim<'p>(view: &Obj<'p>, index: &Obj<'p>, x: f64, y: f64) -> PyResult<Option<Obj<'p>>> {
    let py = view.py();
    let Some(edge) = reference_edge(index, view, x, y, None)? else {
        return Ok(None);
    };
    let picked = view
        .getattr("tools")?
        .call_method1("_mesh_for_row", (edge.getattr("row")?,))?;
    if picked.is_none() {
        return Ok(None);
    }
    let (mesh, group, m) = (
        picked.get_item(0)?,
        picked.get_item(1)?,
        picked.get_item(2)?,
    );
    let inv = m.call_method0("inverted")?;
    if !inv.get_item(1)?.is_truthy()? {
        return Ok(None);
    }
    let inv = inv.get_item(0)?;
    let vertex = |end: &str| {
        mesh.call_method1(
            "vertex_at",
            (inv.call_method1("map", (edge.getattr(end)?,))?,),
        )
    };
    let (a, b) = (vertex("a")?, vertex("b")?);
    if a.is_none() || b.is_none() {
        return Ok(None);
    }
    let e = mesh.call_method1("find_edge", (a, b))?;
    if e.getattr_opt("curve")?.is_none_or(|v| v.is_none()) {
        return Ok(None);
    }
    let center = view
        .getattr("source_viewport")?
        .call_method1("_center_of_edge", (e, mesh))?;
    if center.is_none() {
        return Ok(None);
    }
    let mut out = vec![m.call_method1("map", (center.get_item(0)?,))?];
    for i in 1..6.min(center.len()?) {
        out.push(center.get_item(i)?);
    }
    let place = if group.is_none() {
        py.None().into_bound(py)
    } else {
        let kw = PyDict::new(py);
        kw.set_item("xform", m)?;
        kw.set_item("source_occurrence", group)?;
        record(py, &kw)?
    };
    out.push(place);
    Ok(Some(PyTuple::new(py, out)?.into_any()))
}

#[pyfunction(signature=(index,view,start,rows))]
fn start_edges<'p>(
    index: &Obj<'p>,
    view: &Obj<'p>,
    start: &Obj<'p>,
    rows: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = view.py();
    let m = matrix(view)?;
    let (w, h) = (
        view.call_method0("width")?.extract::<f64>()?,
        view.call_method0("height")?.extract::<f64>()?,
    );
    let group = view.getattr("scene")?.getattr("edit_group")?;
    let key = (
        start.call_method0("toTuple")?,
        m.call_method0("data")?,
        w,
        h,
        index.getattr("epoch")?,
        group.as_ptr() as usize,
    )
        .into_pyobject(py)?;
    if let Some(c) = index.getattr_opt("start_edges_cache")? {
        if !c.is_none() && c.get_item(0)?.eq(&key)? {
            return c.get_item(1);
        }
    }
    let p = interaction(py)?.call_method1("_world_to_pixel", (view, start, &m))?;
    let out = PyList::empty(py);
    if !p.is_none() {
        let fits = py.import("core.snap")?.getattr("_point_on_segment_world")?;
        for e in nearby_edges(
            index,
            &view.getattr("packed")?,
            &view.getattr("camera")?,
            w,
            h,
            p.get_item(0)?.extract()?,
            p.get_item(1)?.extract()?,
            2.,
            16,
            rows,
            Some(&group),
        )?
        .iter()
        {
            if fits
                .call1((start, e.getattr("a")?, e.getattr("b")?))?
                .is_truthy()?
            {
                out.append(e)?;
            }
        }
    }
    index.setattr("start_edges_cache", (key, &out))?;
    Ok(out.into_any())
}
#[pyfunction(signature=(engine,view,edges,on_face,point,face_normal,options,x,y,modifiers=None))]
fn acquire(
    engine: &Obj<'_>,
    view: &Obj<'_>,
    edges: &Obj<'_>,
    on_face: bool,
    point: &Obj<'_>,
    face_normal: &Obj<'_>,
    options: &Bound<'_, PyDict>,
    x: f64,
    y: f64,
    modifiers: Option<&Obj<'_>>,
) -> PyResult<()> {
    let py = view.py();
    let none = py.None().into_bound(py);
    let tool = view
        .getattr_opt("tools")?
        .unwrap_or(none.clone())
        .getattr_opt("tool")?
        .unwrap_or(none.clone());
    let drawing = tool
        .getattr_opt("uses_snap")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false))
        && tool
            .getattr_opt("start_point")?
            .is_some_and(|v| !v.is_none());
    let mut hover = none.clone();
    if drawing {
        let reach = options
            .get_item("edge_threshold_px")?
            .filter(|v| v.is_truthy().unwrap_or(false))
            .map(|v| v.extract::<f64>())
            .transpose()?
            .unwrap_or(14.);
        let occluded = options.get_item("is_occluded")?.filter(|v| !v.is_none());
        let mut candidates = Vec::new();
        if edges.is_instance_of::<super::edges::ResidentEdges>() {
            let offset = edges.getattr("prefix")?.len()?;
            let ds = edges
                .getattr("distances")?
                .extract::<numpy::PyReadonlyArray1<f64>>()?;
            let mut sorted: Vec<_> = ds
                .as_array()
                .iter()
                .copied()
                .enumerate()
                .filter(|v| v.1 <= reach)
                .collect();
            sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
            for (i, _) in sorted {
                let e = edges.get_item(offset + i)?;
                if (xyz(&e.getattr("b")?)? - xyz(&e.getattr("a")?)?).length_squared() > 1e-18 {
                    candidates.push(e);
                    if candidates.len() == 3 {
                        break;
                    }
                }
            }
        } else {
            let mut near = Vec::new();
            for e in edges.try_iter()? {
                let e = e?;
                if let Some(d) = e.getattr_opt("distance")? {
                    let d = d.extract::<f64>()?;
                    if d <= reach
                        && (xyz(&e.getattr("b")?)? - xyz(&e.getattr("a")?)?).length_squared()
                            > 1e-18
                    {
                        near.push((d, e));
                    }
                }
            }
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            candidates.extend(near.into_iter().take(3).map(|v| v.1));
        }
        for e in candidates {
            if let Some(f) = &occluded {
                if f.call1((foot(view, &e, x, y, 0.)?,))?.is_truthy()? {
                    continue;
                }
            }
            hover = e;
            break;
        }
    }
    let kw = PyDict::new(py);
    kw.set_item("edge", &hover)?;
    kw.set_item("face_normal", if on_face { face_normal } else { &none })?;
    engine.call_method("note_drawing_refs", (&tool,), Some(&kw))?;
    options.set_item("acquired_edge", engine.getattr("_acquired_edge")?)?;
    options.set_item(
        "acquired_face_normal",
        engine.getattr("_acquired_face_normal")?,
    )?;
    if drawing {
        let modifiers = match modifiers {
            Some(m) => m.clone(),
            None => py
                .import("core.input_codes")?
                .getattr("Input")?
                .getattr("NoModifier")?,
        };
        if engine
            .call_method1("direction_locked", (modifiers,))?
            .is_truthy()?
        {
            if !hover.is_none() {
                options.call_method1("setdefault", ("hover_lock_edge", hover))?;
            } else if on_face && !face_normal.is_none() {
                options.call_method1(
                    "setdefault",
                    ("hover_lock_plane", (point, face_normal, point)),
                )?;
            }
        }
    }
    Ok(())
}

#[pyfunction(signature=(view,x,y,on_face,project,*,acquire=false))]
fn face_centers_near<'p>(
    view: &Obj<'p>,
    x: f64,
    y: f64,
    on_face: bool,
    project: &Obj<'p>,
    acquire: bool,
) -> PyResult<Obj<'p>> {
    let py = view.py();
    let none = py.None().into_bound(py);
    let doc = interaction(py)?;
    let reach = doc.getattr("POINT_INFERENCE_PX")?.extract::<f64>()?;
    let tools = view.getattr_opt("tools")?.unwrap_or(none.clone());
    let moving = tools
        .getattr_opt("tool")?
        .unwrap_or(none.clone())
        .getattr_opt("moving_face_ids")?
        .filter(|v| v.is_callable())
        .map(|v| v.call0())
        .transpose()?;
    let moving = |face: &Obj<'p>| -> PyResult<bool> {
        match &moving {
            Some(m) => m.contains(face.getattr("key")?),
            None => Ok(false),
        }
    };
    let mesh = py.import("core.mesh")?;
    let center = py.import("core.snap")?.getattr("face_center_of_face")?;
    let out = PyList::empty(py);
    let append = |c: Obj<'p>, ctx: Option<&str>| -> PyResult<()> {
        if !c.is_none() {
            let p = project.call1((&c,))?;
            if !p.is_none()
                && (p.get_item(0)?.extract::<f64>()? - x)
                    .hypot(p.get_item(1)?.extract::<f64>()? - y)
                    <= reach
            {
                out.append((c, ctx))?;
            }
        }
        Ok(())
    };
    if on_face {
        if let Some(pick) = tools.getattr_opt("_picked_mesh")? {
            let picked = pick.call1((x, y))?;
            if !picked.is_none()
                && mesh
                    .call_method1("face_count", (picked.get_item(0)?,))?
                    .extract::<usize>()?
                    <= 5000
            {
                let pair = tools.call_method1("pick_face_placement", (x, y))?;
                let (f, place) = (pair.get_item(0)?, pair.get_item(1)?);
                if !f.is_none() && !moving(&f)? {
                    let g = picked.get_item(1)?;
                    let ctx = if place.is_none() || g.is_none() {
                        None
                    } else if g.call_method0("is_instance")?.is_truthy()? {
                        Some("component")
                    } else {
                        Some("group")
                    };
                    if acquire {
                        let host = view.getattr("source_viewport")?;
                        let r = host.call_method1(
                            "_center_of_face",
                            (&f, &place, picked.get_item(0)?, x, y),
                        )?;
                        if !r.is_none() {
                            host.setattr("_center_ref", r)?;
                        }
                    }
                    append(center.call1((&f, place.getattr_opt("xform")?))?, ctx)?;
                }
            }
        }
    }
    let ty = mesh.getattr("Face")?;
    let mut faces = Vec::new();
    for f in view.getattr("scene")?.getattr("selection")?.try_iter()? {
        let f = f?;
        if f.is_instance(&ty)? {
            faces.push(f);
        }
    }
    if faces.len() <= doc.getattr("FACE_CENTER_SELECTION_MAX")?.extract()? {
        for f in faces {
            if !moving(&f)? {
                append(center.call1((f,))?, None)?;
            }
        }
    }
    Ok(out.into_any())
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(acquire_rim, m)?)?;
    m.add_function(wrap_pyfunction!(start_edges, m)?)?;
    m.add_function(wrap_pyfunction!(acquire, m)?)?;
    m.add_function(wrap_pyfunction!(face_centers_near, m)?)
}

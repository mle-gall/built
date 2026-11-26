// SPDX-License-Identifier: GPL-3.0-or-later
//! Resident placement context, reference edges and visibility queries.
use crate::math::{xyz, Vec3};
use numpy::{IntoPyArray, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::{prelude::*, types::PyList};
pub(super) type Obj<'p> = Bound<'p, PyAny>;

pub(super) fn interaction(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("views.document_interaction")?
        .getattr("DocumentInteraction")
}
pub(super) fn matrix<'p>(view: &Obj<'p>) -> PyResult<Obj<'p>> {
    let c = view.getattr("camera")?;
    c.call_method0("projection_matrix")?
        .mul(c.call_method0("view_matrix")?)
}
pub(super) fn near_rows<'p>(
    index: &Obj<'p>,
    view: &Obj<'p>,
    m: &Obj<'p>,
    x: f64,
    y: f64,
    r: f64,
    rows: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    super::screen::screen_rows(
        index,
        &view.getattr("packed")?,
        m,
        view.call_method0("width")?.extract()?,
        view.call_method0("height")?.extract()?,
        x,
        y,
        r,
        rows,
    )
}
fn group<'p>(view: &Obj<'p>, row: &Obj<'p>) -> PyResult<Obj<'p>> {
    if let Some(groups) = view.getattr("packed")?.getattr_opt("groups")? {
        if !groups.is_none() && !row.is_none() {
            return groups.get_item(row);
        }
    }
    Ok(view.py().None().into_bound(view.py()))
}
fn kind(group: &Obj<'_>, context: &Obj<'_>) -> PyResult<Option<&'static str>> {
    Ok(if group.is_none() || group.is(context) {
        None
    } else if group.call_method0("is_instance")?.is_truthy()? {
        Some("component")
    } else {
        Some("group")
    })
}
#[pyfunction]
pub(super) fn placement_context(
    view: &Obj<'_>,
    row: &Obj<'_>,
    context: &Obj<'_>,
) -> PyResult<Option<&'static str>> {
    kind(&group(view, row)?, context)
}
#[pyfunction]
pub(super) fn snap_context(
    view: &Obj<'_>,
    row: &Obj<'_>,
    context: &Obj<'_>,
) -> PyResult<Option<&'static str>> {
    let g = group(view, row)?;
    if g.is(context) {
        return Ok(None);
    }
    if !context.is_none()
        && (g.is_none()
            || match view.getattr_opt("context_placement")? {
                Some(f) if !f.is_none() => f.call1((&g,))?.is_none(),
                _ => false,
            })
    {
        return Ok(Some("outside"));
    }
    kind(&g, context)
}
#[pyfunction(signature=(index,view,row,point,project=None))]
pub(super) fn on_surface(
    index: &Obj<'_>,
    view: &Obj<'_>,
    row: &Obj<'_>,
    point: &Obj<'_>,
    project: Option<&Obj<'_>>,
) -> PyResult<bool> {
    let pixel = match project {
        Some(f) => f.call1((point,))?,
        None => view.call_method1("_world_to_pixel", (point,))?,
    };
    if pixel.is_none() {
        return Ok(false);
    }
    let ray = view.call_method1("_pixel_to_ray", (pixel.get_item(0)?, pixel.get_item(1)?))?;
    let (o, d) = (ray.get_item(0)?, ray.get_item(1)?);
    if o.is_none() || d.is_none() {
        return Ok(false);
    }
    let Some(hit) = super::picking::surface(index, row, &o, &d, f64::INFINITY)? else {
        return Ok(false);
    };
    let p = xyz(point)?;
    Ok((xyz(&hit)? - p).length() <= 1e-6 + 1e-5 * (p - xyz(&o)?).length())
}

fn see_through(view: &Obj<'_>) -> PyResult<bool> {
    let Some(host) = view.getattr_opt("source_viewport")? else {
        return Ok(false);
    };
    let Some(style) = host.getattr_opt("_effective_style")? else {
        return Ok(false);
    };
    Ok(style
        .call0()
        .and_then(|s| s.getattr("face_mode")?.extract::<String>())
        .is_ok_and(|s| s == "xray" || s == "wireframe"))
}

#[pyfunction(signature=(index,view,candidate,rows=None))]
pub(super) fn occluded(
    index: &Obj<'_>,
    view: &Obj<'_>,
    candidate: &Obj<'_>,
    rows: Option<&Obj<'_>>,
) -> PyResult<bool> {
    if see_through(view)? {
        return Ok(false);
    }
    let pixel = view.call_method1("_world_to_pixel", (candidate,))?;
    if pixel.is_none() {
        return Ok(true);
    }
    let (x, y) = (pixel.get_item(0)?.extract()?, pixel.get_item(1)?.extract()?);
    let ray = view.call_method1("_pixel_to_ray", (x, y))?;
    let (o, d) = (ray.get_item(0)?, ray.get_item(1)?);
    if o.is_none() || d.is_none() {
        return Ok(true);
    }
    let all = match rows {
        Some(r) => r.clone(),
        None => near_rows(index, view, &matrix(view)?, x, y, 1., None)?,
    };
    let py = index.py();
    let flags = index
        .getattr("replaced")?
        .extract::<PyReadonlyArray1<bool>>()?;
    let flags = flags.as_array();
    let ids = all.extract::<PyReadonlyArray1<i64>>()?;
    let mut fixed = Vec::new();
    let mut changed = Vec::new();
    for &i in ids.as_array() {
        if *flags
            .get(i as usize)
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(i))?
        {
            changed.push(i);
        } else {
            fixed.push(i);
        }
    }
    let reach = (xyz(candidate)? - xyz(&o)?).length() - 1e-4;
    let base = index
        .getattr("base")?
        .extract::<PyRef<crate::InstanceSet>>()?;
    base.check(&fixed)?;
    fixed.sort_unstable();
    let (hidden, doubt) = base.occluded_where(
        |i| fixed.binary_search(&(i as i64)).is_ok(),
        xyz(&o)?.to_array(),
        xyz(&d)?.to_array(),
        reach,
    );
    if !doubt && hidden {
        return Ok(true);
    }
    let check = if doubt {
        all
    } else {
        changed.into_pyarray(py).into_any()
    };
    if check.is_empty()? {
        return Ok(false);
    }
    match super::picking::surface(index, &check, &o, &d, reach)? {
        Some(hit) => Ok((xyz(&hit)? - xyz(&o)?).length() < reach),
        None => Ok(false),
    }
}

#[pyfunction(name="resident_occluded_many", signature=(index,view,points,rows=None))]
pub(super) fn occluded_many(
    index: &Obj<'_>,
    view: &Obj<'_>,
    points: &Obj<'_>,
    rows: Option<&Obj<'_>>,
) -> PyResult<Vec<bool>> {
    let n = points.len()?;
    if n == 0 || see_through(view)? {
        return Ok(vec![false; n]);
    }
    let cam = view.getattr("camera")?;
    if !cam
        .getattr_opt("perspective")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false))
        || !index.getattr("rows")?.is_empty()?
        || !index.getattr("changed")?.is_empty()?
    {
        return points
            .try_iter()?
            .map(|p| occluded(index, view, &p?, rows))
            .collect();
    }
    let eye = xyz(&cam.call_method0("eye")?)?;
    let base = index
        .getattr("base")?
        .extract::<PyRef<crate::InstanceSet>>()?;
    let base = &*base;
    let info = view
        .getattr("packed")?
        .getattr("instances")?
        .get_item("info")?
        .extract::<PyReadonlyArray2<u32>>()?;
    let info = info.as_array();
    let mut subset = rows
        .map(|r| {
            r.extract::<PyReadonlyArray1<i64>>()
                .map(|v| v.as_array().to_vec())
        })
        .transpose()?;
    if let Some(ids) = &mut subset {
        base.check(ids)?;
        ids.sort_unstable();
    }
    // The BVH visits only the placements along each ray. Read current visibility
    // at those leaves instead of building a scene-sized list and mask per hover.
    let visible = |i: usize| match &subset {
        Some(ids) => ids.binary_search(&(i as i64)).is_ok(),
        None => info.get([i, 2]).is_some_and(|f| f & 129 == 0),
    };
    let mut rays = Vec::with_capacity(n);
    for p in points.try_iter()? {
        let delta = xyz(&p?)? - eye;
        let len = delta.length();
        rays.push(((delta / len.max(1e-30)).to_array(), (len - 1e-4).max(0.)));
    }
    let test = |(d, r): &([f64; 3], f64)| base.occluded_where(visible, eye.to_array(), *d, *r);
    let answers: Vec<_> = if n <= 8 {
        rays.iter().map(test).collect()
    } else {
        use rayon::prelude::*;
        rays.par_iter().map(test).collect()
    };
    let mut out = Vec::with_capacity(n);
    let mut fallback = None;
    for (i, (hidden, doubt)) in answers.into_iter().enumerate() {
        if doubt && rows.is_none() && fallback.is_none() {
            fallback = Some(super::screen::eligible_rows(
                index,
                Some(&view.getattr("packed")?),
            )?);
        }
        out.push(if doubt {
            occluded(
                index,
                view,
                &points.get_item(i)?,
                rows.or(fallback.as_ref()),
            )?
        } else {
            hidden
        });
    }
    Ok(out)
}

#[pyfunction(signature=(index,packed,camera,width,height,x,y,*,radius_px=48.,cap=48,rows=None,edit_group=None))]
pub(super) fn nearby_edges<'p>(
    index: &Obj<'p>,
    packed: &Obj<'p>,
    camera: &Obj<'p>,
    width: f64,
    height: f64,
    x: f64,
    y: f64,
    radius_px: f64,
    cap: isize,
    rows: Option<&Obj<'p>>,
    edit_group: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyList>> {
    let py = index.py();
    let result = PyList::empty(py);
    if cap <= 0 {
        return Ok(result);
    }
    let m = camera
        .call_method0("projection_matrix")?
        .mul(camera.call_method0("view_matrix")?)?;
    let valid = if let Some(r) = rows {
        let n = packed.getattr("instances")?.len()?;
        let ids: Vec<i64> = r.extract()?;
        Some(
            ids.into_iter()
                .filter(|&i| i >= 0 && (i as usize) < n)
                .collect::<Vec<_>>()
                .into_pyarray(py),
        )
    } else {
        None
    };
    let near = super::screen::screen_rows(
        index,
        packed,
        &m,
        width,
        height,
        x,
        y,
        radius_px,
        valid.as_ref().map(|r| r.as_any()),
    )?;
    let cols = index.call_method1(
        "edges_near",
        (
            near,
            m.call_method0("data")?,
            width,
            height,
            x,
            y,
            radius_px,
            cap,
        ),
    )?;
    let edges = py.get_type::<super::edges::ResidentEdges>().call1((
        cols,
        packed.getattr_opt("groups")?,
        edit_group,
    ))?;
    for e in edges.try_iter()? {
        result.append(e?)?;
    }
    Ok(result)
}
pub(super) fn foot<'p>(
    view: &Obj<'p>,
    e: &Obj<'p>,
    x: f64,
    y: f64,
    min_length_squared: f64,
) -> PyResult<Obj<'p>> {
    let a = e.getattr("a")?;
    let delta = e.getattr("b")?.sub(&a)?;
    let p =
        interaction(view.py())?.call_method1("_project_to_lock_line", (view, &a, &delta, x, y))?;
    let v = xyz(&delta)?;
    let len = v.length_squared();
    let t = if len > min_length_squared {
        (xyz(&p)? - xyz(&a)?).dot(v) / len
    } else {
        0.
    };
    a.add(delta.mul(t.clamp(0., 1.))?)
}
#[pyfunction(signature=(index,view,x,y,radius_px=None))]
pub(super) fn reference_edge<'p>(
    index: &Obj<'p>,
    view: &Obj<'p>,
    x: f64,
    y: f64,
    radius_px: Option<f64>,
) -> PyResult<Option<Obj<'p>>> {
    let r = match radius_px {
        Some(r) => r,
        None => view
            .getattr_opt("pick_threshold_px")?
            .map(|v| v.extract())
            .transpose()?
            .unwrap_or(8.),
    };
    let edges = nearby_edges(
        index,
        &view.getattr("packed")?,
        &view.getattr("camera")?,
        view.call_method0("width")?.extract()?,
        view.call_method0("height")?.extract()?,
        x,
        y,
        r,
        48,
        None,
        Some(&view.getattr("scene")?.getattr("edit_group")?),
    )?;
    // An edge with an end behind the eye counts too (`nearby_edges` gives
    // the part in front): close up to a long edge that is the usual case.
    for e in edges.iter() {
        if !occluded(index, view, &foot(view, &e, x, y, 1e-12)?, None)? {
            return Ok(Some(e));
        }
    }
    Ok(None)
}
#[pyfunction]
pub(super) fn scene_edges<'p>(view: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = view.py();
    let out = PyList::empty(py);
    let doc = py.import("views.document_interaction")?;
    let interaction = doc.getattr("DocumentInteraction")?;
    let scene = view.getattr("scene")?;
    let none = py.None().into_bound(py);
    let tool = view
        .getattr_opt("tools")?
        .unwrap_or(none.clone())
        .getattr_opt("tool")?
        .unwrap_or(none);
    let moving = tool
        .getattr_opt("moving_guide_ids")?
        .map(|v| v.call0())
        .transpose()?;
    let append = |a: Obj<'p>, b: Obj<'p>, source: &Obj<'p>, guide: Option<bool>| -> PyResult<()> {
        let fields = pyo3::types::PyDict::new(py);
        fields.set_item("a", a)?;
        fields.set_item("b", b)?;
        fields.set_item("source", source)?;
        if let Some(g) = guide {
            fields.set_item("guide", g)?;
        }
        out.append(super::acquisition::record(py, &fields)?)
    };
    for g in doc.call_method1("shown_guides", (&scene,))?.try_iter()? {
        let g = g?;
        if let Some(m) = &moving {
            if m.contains(g.as_ptr() as usize)? {
                continue;
            }
        }
        let line = g.getattr("is_line")?.is_truthy()?;
        if line {
            let s = interaction.call_method1("_guide_snap_segment", (view, &g))?;
            if !s.is_none() {
                append(s.get_item(0)?, s.get_item(1)?, &g, Some(true))?;
            }
        } else {
            let p = xyz(&g.getattr("point")?)?;
            append(
                Vec3::from(p).into_pyobject(py)?.into_any(),
                Vec3::from(p).into_pyobject(py)?.into_any(),
                &g,
                Some(false),
            )?;
        }
    }
    let selectable = scene.getattr_opt("entity_selectable")?;
    if let Some(planes) = scene.getattr_opt("image_planes")?.filter(|p| !p.is_none()) {
        for plane in planes.try_iter()? {
            let plane = plane?;
            if plane
                .getattr_opt("locked")?
                .is_some_and(|l| l.is_truthy().unwrap_or(false))
            {
                continue;
            }
            if let Some(check) = &selectable {
                if !check.call1((&plane,))?.is_truthy()? {
                    continue;
                }
            }
            for ab in plane.call_method0("border_edges")?.try_iter()? {
                let ab = ab?;
                let s = interaction.call_method1(
                    "_clip_segment_front",
                    (view, ab.get_item(0)?, ab.get_item(1)?),
                )?;
                if !s.is_none() {
                    append(s.get_item(0)?, s.get_item(1)?, &plane, None)?;
                }
            }
        }
    }
    Ok(out)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(placement_context, m)?)?;
    m.add_function(wrap_pyfunction!(snap_context, m)?)?;
    m.add_function(wrap_pyfunction!(on_surface, m)?)?;
    m.add_function(wrap_pyfunction!(occluded, m)?)?;
    m.add_function(wrap_pyfunction!(occluded_many, m)?)?;
    m.add_function(wrap_pyfunction!(scene_edges, m)?)?;
    m.add_function(wrap_pyfunction!(nearby_edges, m)?)?;
    m.add_function(wrap_pyfunction!(reference_edge, m)?)
}

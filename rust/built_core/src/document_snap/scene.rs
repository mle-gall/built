// SPDX-License-Identifier: GPL-3.0-or-later
//! Candidate order matters for coincident guide, dimension and model points.
use super::*;
use pyo3::types::PySet;
#[pyfunction]
fn document_snap_scene<'py>(
    view: &Obj<'py>,
    px: Option<f64>,
    py_: Option<f64>,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let h = host(py)?;
    let scene = view.getattr("scene")?;
    let tool = view.getattr("active_tool")?;
    let lines = PyList::empty(py);
    let circles = PyList::empty(py);
    let moving = optional_call(&tool, "moving_guide_ids")?;
    for guide in h.call_method1("shown_guides", (&scene,))?.try_iter()? {
        let g = guide?;
        if !moving.is_none() && moving.contains(id(&g))? {
            continue;
        }
        if opt(&g, "is_circle")?.is_truthy()? {
            circles.append(g)?
        } else if g.getattr("is_line")?.is_truthy()? {
            let seg = view.call_method1("_guide_snap_segment", (g,))?;
            if !seg.is_none() {
                lines.append(snap_edge(
                    py,
                    &seg.get_item(0)?,
                    &seg.get_item(1)?,
                    Some("guide"),
                    None,
                    None,
                )?)?
            }
        } else {
            lines.append(point(&g.getattr("point")?, None, None)?)?
        }
    }
    for image in collection(&scene, "image_planes")?.try_iter()? {
        let im = image?;
        if opt(&im, "locked")?.is_truthy()?
            || !scene
                .call_method1("entity_selectable", (&im,))?
                .is_truthy()?
        {
            continue;
        }
        for e in im.call_method0("border_edges")?.try_iter()? {
            let e = e?;
            let seg = view.call_method1("_clip_segment_front", (e.get_item(0)?, e.get_item(1)?))?;
            if !seg.is_none() {
                lines.append(snap_edge(
                    py,
                    &seg.get_item(0)?,
                    &seg.get_item(1)?,
                    None,
                    None,
                    None,
                )?)?
            }
        }
    }
    let near = PyList::empty(py);
    let grouped = if px.is_some() {
        view.call_method1("_nearby_group_edges", (px, py_))?
    } else {
        PyTuple::empty(py).into_any()
    };
    let excl = optional_call(&tool, "snap_excluded")?;
    let skipped = if excl.is_none() {
        none(py)
    } else {
        excl.get_item(1)?
    };
    for e in grouped.try_iter()? {
        let e = e?;
        if !skipped.is_truthy()? || !skipped.contains(id(&opt(&e, "group")?))? {
            near.append(e)?
        }
    }
    if px.is_some() {
        near.call_method1("extend", (view.call_method0("_billboard_snap_edges")?,))?;
        for name in ["_component_origin_points", "_arc_midpoints"] {
            if let Some(f) = view.getattr_opt(name)? {
                near.call_method1("extend", (f.call1((px, py_))?,))?;
            }
        }
    }
    let mut box_pts = view.call_method0("_selection_box_points")?;
    if skipped.is_truthy()? {
        box_pts = view.call_method(
            "_selection_box_points",
            (),
            Some(&kw(py, "skip", &skipped)?),
        )?
    }
    near.call_method1("extend", (box_pts,))?;
    let center = optional_call(view, "_valid_center_ref")?;
    if !center.is_none() {
        let c = center.get_item(0)?;
        let pc = view.call_method1("_world_to_pixel", (&c,))?;
        if !pc.is_none()
            && match view.getattr_opt("_center_hint_px")? {
                Some(f) => f.call1((&center, pc))?.extract::<f64>()? > 0.,
                None => true,
            }
        {
            near.append(point(&c, Some("center"), None)?)?
        }
    }
    for dim in collection(&scene, "dimensions")?.try_iter()? {
        let dim = dim?;
        if scene
            .call_method1("entity_selectable", (&dim,))?
            .is_truthy()?
        {
            let pair = dim.call_method0("line_points")?;
            near.append(snap_edge(
                py,
                &pair.get_item(0)?,
                &pair.get_item(1)?,
                None,
                None,
                None,
            )?)?
        }
    }
    for c in circles.iter() {
        near.append(point(&c.getattr("center")?, Some("center"), None)?)?
    }
    let plane = h.call_method1("_active_cut", (&scene,))?;
    let kept = |e: &Obj| -> PyResult<bool> {
        Ok(plane.is_none()
            || !(plane
                .call_method1("side", (e.getattr("a")?,))?
                .extract::<f64>()?
                > 1e-6
                && plane
                    .call_method1("side", (e.getattr("b")?,))?
                    .extract::<f64>()?
                    > 1e-6))
    };
    let refs = PyList::empty(py);
    for e in near.iter() {
        if kept(&e)? {
            refs.append(e)?
        }
    }
    for gp in collection(&scene, "geo_points")?.try_iter()? {
        refs.append(point(&gp?.getattr("position")?, None, None)?)?
    }
    let outer = PyList::empty(py);
    let meshes = optional_call(view, "_outer_loose_meshes")?;
    if !meshes.is_none() {
        for om in meshes.try_iter()? {
            let om = om?;
            let edges = PyList::new(py, om.getattr("edges")?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
            let (_, (_, snaps, _)) = crate::model::scene_mesh_states::scene_mesh_states(
                py, &scene, &om, PyList::empty(py).as_any(), edges.as_any())?;
            for (e, snappable) in edges.iter().zip(snaps) {
                if snappable {
                    outer.append(e)?
                }
            }
        }
    }
    let big = px.is_some()
        && scene.getattr("edges")?.len()? + outer.len()
            > h.getattr("_LOOSE_SNAP_CAP")?.extract::<usize>()?;
    let drops = if !big {
        h.call_method1("_snap_dropped_loose", (view,))?
    } else {
        none(py)
    };
    if lines.is_empty()
        && refs.is_empty()
        && circles.is_empty()
        && plane.is_none()
        && !big
        && outer.is_empty()
        && excl.is_none()
        && !drops.is_truthy()?
    {
        return Ok(scene);
    }
    let identity = h.getattr("element_identity")?;
    let outer_ids = PySet::empty(py)?;
    for e in outer.iter() {
        outer_ids.add(identity.call1((e,))?)?
    }
    let loose = if big {
        view.call_method1("_nearby_loose_edges", (px, py_))?
    } else {
        let rows = PyList::new(
            py,
            scene
                .getattr("edges")?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?,
        )?;
        rows.call_method1("extend", (&outer,))?;
        rows.into_any()
    };
    let exclude = if excl.is_none() {
        none(py)
    } else {
        excl.get_item(0)?
    };
    let out = PyList::empty(py);
    let check_id = drops.is_truthy()? || exclude.is_truthy()? || !outer_ids.is_empty();
    let outside = "outside".into_pyobject(py)?.into_any();
    for e in loose.try_iter()? {
        let e = e?;
        if big && !scene.call_method1("edge_snappable", (&e,))?.is_truthy()? {
            continue;
        }
        let k = if check_id {
            identity.call1((&e,))?
        } else {
            none(py)
        };
        if (drops.is_truthy()? && drops.contains(&k)?)
            || (exclude.is_truthy()? && exclude.contains(&k)?)
            || !kept(&e)?
        {
            continue;
        }
        if !outer_ids.is_empty() && outer_ids.contains(&k)? {
            out.append(snap_edge(
                py,
                &e.getattr("a")?,
                &e.getattr("b")?,
                None,
                Some(&outside),
                None,
            )?)?
        } else {
            out.append(e)?
        }
    }
    out.call_method1("extend", (lines,))?;
    out.call_method1("extend", (refs,))?;
    let args = PyDict::new(py);
    args.set_item("edges", out)?;
    args.set_item("circles", circles)?;
    py.import("types")?
        .getattr("SimpleNamespace")?
        .call((), Some(&args))
}
#[pyfunction]
fn selection_box_points<'py>(view: &Obj<'py>, skip: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let group = host(py)?.getattr("Group")?;
    let corners = py.import("built_core.group")?.getattr("oriented_box_corners")?;
    let out = PyList::empty(py);
    for e in view.getattr("scene")?.getattr("selection")?.try_iter()? {
        let e = e?;
        if !e.is_instance(&group)? || opt(&e, "billboard")?.is_truthy()? || skip.contains(id(&e))? {
            continue;
        }
        let bounds = view.call_method1("_group_obb", (e,))?;
        for p in corners.call1(bounds.cast::<PyTuple>()?)?.try_iter()? {
            let p = p?;
            out.append(snap_edge(
                py,
                &p,
                &Bound::new(py, Vec3::from(xyz(&p)?))?.into_any(),
                None,
                None,
                None,
            )?)?
        }
    }
    Ok(out.into_any())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(document_snap_scene, m)?)?;
    m.add_function(wrap_pyfunction!(selection_box_points, m)?)?;
    Ok(())
}

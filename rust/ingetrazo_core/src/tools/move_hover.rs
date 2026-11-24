// SPDX-License-Identifier: GPL-3.0-or-later
//! Move grip tracking and transform snap masks share the native camera and
//! disk/radial incidence with the pick kernels.
use super::{
    host_geometry::{move_box_contains, move_grip_hit},
    move_grips, P3,
};
use crate::{
    math::{xyz, Vec3},
    store::{keys::Kind, py::PyMeshStore},
};
use pyo3::{prelude::*, types::PyList};
use std::collections::{HashMap, HashSet};

type Obb = ([P3; 3], P3, P3);
fn box_data(obb: &Bound<'_, PyAny>) -> PyResult<Obb> {
    let axes = obb.get_item(0)?;
    let axis = |i: usize| xyz(&axes.get_item(i)?).map(|v| v.to_array());
    Ok((
        [axis(0)?, axis(1)?, axis(2)?],
        xyz(&obb.get_item(1)?)?.to_array(),
        xyz(&obb.get_item(2)?)?.to_array(),
    ))
}
#[pyfunction]
pub fn rotation_handles(
    obb: &Bound<'_, PyAny>,
    eye: &Bound<'_, PyAny>,
) -> PyResult<Vec<(Vec3, Vec3, Vec3)>> {
    if obb.is_none() {
        return Ok(vec![]);
    }
    let (axes, lo, hi) = box_data(obb)?;
    Ok(
        move_grips::rotation_grips(axes, lo, hi, xyz(eye)?.to_array())
            .into_iter()
            .map(|(a, b, c)| {
                (
                    Vec3::from(glam::DVec3::from(a)),
                    Vec3::from(glam::DVec3::from(b)),
                    Vec3::from(glam::DVec3::from(c)),
                )
            })
            .collect(),
    )
}

#[pyfunction]
pub fn track_move_grips(
    tool: &Bound<'_, PyAny>,
    ctx: &Bound<'_, PyAny>,
    margin: f64,
) -> PyResult<()> {
    let view = ctx.getattr("viewport")?;
    let Some(obb) = view.getattr_opt("_group_obb")?.filter(|f| f.is_callable()) else {
        return Ok(());
    };
    let Some(cam) = view.getattr_opt("camera")?.filter(|c| !c.is_none()) else {
        return Ok(());
    };
    if !view.hasattr("_world_to_pixel")? {
        return Ok(());
    }
    let screen = ctx.getattr("screen")?;
    let x: f64 = screen.call_method0("x")?.extract()?;
    let y: f64 = screen.call_method0("y")?.extract()?;
    let selected = view.getattr("scene")?.getattr("selection")?;
    let eligible = |g: &Bound<'_, PyAny>| -> PyResult<bool> {
        Ok(selected.is_empty()? || (selected.len()? == 1 && selected.contains(g)?))
    };
    let mut group = super::view_queries::hit(&view, "pick_group", (x, y))?;
    if let Some(g) = &group {
        if !eligible(g)?
            || g.getattr_opt("billboard")?
                .is_some_and(|v| v.is_truthy().unwrap_or(false))
        {
            group = None;
        }
    }
    let mut bounds = None;
    if group.is_none() {
        let grips = tool.getattr("_grips")?;
        if !grips.is_empty()? {
            if let Some(i) = move_grip_hit(&view, &grips, [x, y], margin)? {
                return tool.setattr("_hot_grip", i);
            }
            let old = tool.getattr("_grip_group")?;
            if !old.is_none() && eligible(&old)? {
                let box_ = obb.call1((&old,))?;
                if !box_.is_none() {
                    let (axes, lo, hi) = box_data(&box_)?;
                    if move_box_contains(&view, axes, lo, hi, [x, y], margin)? {
                        bounds = Some(box_);
                        group = Some(old);
                    }
                }
            }
        }
    }
    let grips = if let Some(g) = &group {
        let b = match bounds {
            Some(b) => b,
            None => obb.call1((g,))?,
        };
        rotation_handles(&b, &cam.call_method0("eye")?)?
    } else {
        vec![]
    };
    let grips = PyList::new(tool.py(), grips)?;
    let hot = if grips.is_empty() {
        None
    } else {
        move_grip_hit(&view, &grips, [x, y], margin)?
    };
    tool.setattr("_grip_group", group)?;
    tool.setattr("_grips", grips)?;
    tool.setattr("_hot_grip", hot)
}

fn incident(tool: &Bound<'_, PyAny>, faces: bool) -> PyResult<HashSet<u128>> {
    let mut keys = HashSet::new();
    let mut stores = HashMap::new();
    for v in tool.getattr("_verts")?.try_iter()? {
        let v = v?;
        if let Some(mesh) = v.getattr_opt("_m")? {
            if let Some(store) = mesh
                .getattr_opt("_store")?
                .filter(|s| s.is_instance_of::<PyMeshStore>())
            {
                stores
                    .entry(store.as_ptr() as usize)
                    .or_insert_with(|| (store, vec![]))
                    .1
                    .push(v.getattr("key")?.extract::<u128>()?);
                continue;
            }
        }
        if let Some(edges) = v.getattr_opt("edges")? {
            for e in edges.try_iter()? {
                let e = e?;
                if faces {
                    for f in e.getattr("faces")?.try_iter()? {
                        keys.insert(f?.getattr("key")?.extract()?);
                    }
                } else {
                    keys.insert(e.getattr("key")?.extract()?);
                }
            }
        }
    }
    for (_, (store, vertices)) in stores {
        let s = PyMeshStore::read(&store)?;
        for v in vertices
            .into_iter()
            .filter_map(|k| s.inner.slot(Kind::Vertex, k))
        {
            for e in s.inner.edges_at(v) {
                if faces {
                    keys.extend(
                        s.inner
                            .edge_faces(e)
                            .iter()
                            .map(|&f| s.inner.key(Kind::Face, f)),
                    );
                } else {
                    keys.insert(s.inner.key(Kind::Edge, e));
                }
            }
        }
    }
    Ok(keys)
}
#[pyfunction]
pub fn moving_faces(tool: &Bound<'_, PyAny>) -> PyResult<HashSet<u128>> {
    incident(tool, true)
}
#[pyfunction]
pub fn snap_exclusions(
    tool: &Bound<'_, PyAny>,
    include_empty: bool,
) -> PyResult<Option<(HashSet<u128>, HashSet<usize>)>> {
    let edges = incident(tool, false)?;
    let groups = tool
        .getattr("_groups")?
        .try_iter()?
        .map(|g| Ok(g?.as_ptr() as usize))
        .collect::<PyResult<HashSet<_>>>()?;
    Ok((include_empty || !edges.is_empty() || !groups.is_empty()).then_some((edges, groups)))
}

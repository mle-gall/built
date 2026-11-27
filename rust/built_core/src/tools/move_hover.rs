// SPDX-License-Identifier: GPL-3.0-or-later
//! Move grip tracking and transform snap masks share the native camera and
//! disk/radial incidence with the pick kernels.
use super::{
    host_geometry::{move_box_contains, Projection},
    move_grips, P3,
};
use crate::{
    math::{xyz, Vec3},
    store::{keys::Kind, py::PyMeshStore},
};
use pyo3::{prelude::*, types::PyList};
use std::collections::{HashMap, HashSet};

type Obb = move_grips::Obb;
fn box_data(obb: &Bound<'_, PyAny>) -> PyResult<Obb> {
    let axes = obb.get_item(0)?;
    let axis = |i: usize| xyz(&axes.get_item(i)?).map(|v| v.to_array());
    Ok((
        [axis(0)?, axis(1)?, axis(2)?],
        xyz(&obb.get_item(1)?)?.to_array(),
        xyz(&obb.get_item(2)?)?.to_array(),
    ))
}
fn v3(p: P3) -> Vec3 {
    Vec3::from(glam::DVec3::from(p))
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
            .map(|(a, b, c)| (v3(a), v3(b), v3(c)))
            .collect(),
    )
}

/// The preference «Rotation grips in Move» (Settings › Modelling).
pub const ROTATION_GRIPS_KEY: &str = "modelling/move_rotation_grips";
fn rotation_grips_on(py: Python<'_>) -> PyResult<bool> {
    let value = py
        .import("built_core.settings")?
        .call_method0("settings")?
        .call_method1("value", (ROTATION_GRIPS_KEY, "1"))?;
    let text = value.str()?.to_string().to_ascii_lowercase();
    Ok(!matches!(text.as_str(), "" | "0" | "false"))
}

/// The bounding-box grips of the current set, as world points.
#[pyfunction]
#[pyo3(signature=(obb, set=0))]
pub fn box_grip_points(obb: &Bound<'_, PyAny>, set: usize) -> PyResult<Vec<Vec3>> {
    if obb.is_none() {
        return Ok(vec![]);
    }
    Ok(move_grips::box_grips(&box_data(obb)?, set).into_iter().map(v3).collect())
}

/// Forget the grips on show (the grip set chosen stays).
#[pyfunction]
#[pyo3(name = "clear_move_grips")]
pub fn clear(tool: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = tool.py();
    for name in ["_grip_group", "_grip_box", "_hot_grip", "_hot_box", "_grip_key", "_grip_pick"] {
        tool.setattr(name, py.None())?;
    }
    tool.setattr("_grip_groups", PyList::empty(py))?;
    tool.setattr("_grips", PyList::empty(py))?;
    tool.setattr("_box_grips", PyList::empty(py))
}

/// What grips on a hovered group act on: the group alone with nothing
/// selected, or the whole selection when the group is part of one made of
/// groups and components only. Anything else selected: no grips.
fn grip_targets<'py>(
    selection: &Bound<'py, PyAny>,
    g: &Bound<'py, PyAny>,
) -> PyResult<Option<Vec<Bound<'py, PyAny>>>> {
    if selection.is_empty()? {
        return Ok(Some(vec![g.clone()]));
    }
    if !selection.contains(g)? {
        return Ok(None);
    }
    let mut out = vec![];
    for item in selection.try_iter()? {
        let item = item?;
        if super::view_queries::classify(&item)? != "group" {
            return Ok(None);
        }
        out.push(item);
    }
    Ok(Some(out))
}

fn grip_box(
    obb: &Bound<'_, PyAny>,
    targets: &[Bound<'_, PyAny>],
) -> PyResult<Option<Obb>> {
    let mut boxes = Vec::with_capacity(targets.len());
    for t in targets {
        let b = obb.call1((t,))?;
        if !b.is_none() {
            boxes.push(box_data(&b)?);
        }
    }
    Ok(move_grips::union_box(&boxes))
}

fn obb_py(py: Python<'_>, b: &Obb) -> PyResult<Py<PyAny>> {
    Ok(((v3(b.0[0]), v3(b.0[1]), v3(b.0[2])), v3(b.1), v3(b.2))
        .into_pyobject(py)?
        .into_any()
        .unbind())
}

/// Hover tracking for the box grips (Move and Rotate) and the red rotation
/// handles (Move, unless the preference turns them off). A pick on a group
/// shows them; they stay while the cursor is inside the box as projected
/// (``margin`` px round it), so a grip out in the air can be reached. A box
/// grip within ``margin`` px wins; the handles near box grips step aside.
#[pyfunction]
#[pyo3(signature=(tool, ctx, margin, rotation=None))]
pub fn track_move_grips(
    tool: &Bound<'_, PyAny>,
    ctx: &Bound<'_, PyAny>,
    margin: f64,
    rotation: Option<bool>,
) -> PyResult<()> {
    let py = tool.py();
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
    let cursor = [
        screen.call_method0("x")?.extract::<f64>()?,
        screen.call_method0("y")?.extract::<f64>()?,
    ];
    let selection = view.getattr("scene")?.getattr("selection")?;
    let proj = Projection::from_view(&view)?;
    let pixels = |points: &Bound<'_, PyAny>, pick: usize| -> PyResult<Vec<Option<[f64; 2]>>> {
        points
            .try_iter()?
            .map(|p| {
                let p = p?;
                let p = if pick == 0 { p } else { p.get_item(0)? };
                proj.pixel(xyz(&p)?)
            })
            .collect()
    };

    let mut group = super::view_queries::hit(&view, "pick_group", (cursor[0], cursor[1]))?;
    let mut targets = None;
    if let Some(g) = &group {
        let billboard = g
            .getattr_opt("billboard")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false));
        targets = if billboard { None } else { grip_targets(&selection, g)? };
        if targets.is_none() {
            group = None;
        }
    }
    let old = tool.getattr_opt("_grip_group")?.filter(|g| !g.is_none());
    let old_box = tool.getattr_opt("_grip_box")?.filter(|b| !b.is_none());
    if group.is_none() {
        let (Some(old), Some(old_box)) = (old.as_ref(), old_box.as_ref()) else {
            return clear(tool);
        };
        // On the way out to a grip: the grips stay while over one of them
        // or inside the box.
        let boxes = pixels(&tool.getattr("_box_grips")?, 0)?;
        if let Some(i) = move_grips::nearest(&boxes, cursor, margin) {
            tool.setattr("_hot_box", i)?;
            return tool.setattr("_hot_grip", py.None());
        }
        let handles = pixels(&tool.getattr("_grips")?, 1)?;
        if let Some(i) = move_grips::nearest(&handles, cursor, margin) {
            tool.setattr("_hot_box", py.None())?;
            return tool.setattr("_hot_grip", i);
        }
        let (axes, lo, hi) = box_data(old_box)?;
        if !move_box_contains(&view, axes, lo, hi, cursor, margin)? {
            return clear(tool);
        }
        match grip_targets(&selection, old)? {
            Some(t) => {
                targets = Some(t);
                group = Some(old.clone());
            }
            None => return clear(tool),
        }
    }
    let (Some(group), Some(targets)) = (group, targets) else {
        return clear(tool);
    };
    // The box is measured when the hovered object (or the selection it
    // stands for) changes; while the cursor stays on it the grips reuse it.
    let key = (group.as_ptr() as usize, targets.len());
    let same = old.as_ref().is_some_and(|o| o.is(&group))
        && tool
            .getattr_opt("_grip_key")?
            .is_some_and(|k| k.extract::<(usize, usize)>().ok() == Some(key));
    let bounds = match (same, old_box) {
        (true, Some(b)) => box_data(&b)?,
        _ => {
            let Some(b) = grip_box(&obb, &targets)? else {
                return clear(tool);
            };
            tool.setattr("_grip_rotation", match rotation {
                Some(on) => on,
                None => rotation_grips_on(py)?,
            })?;
            b
        }
    };
    let set: usize = tool
        .getattr_opt("_grip_set")?
        .and_then(|s| s.extract().ok())
        .unwrap_or(0);
    let corners: Vec<Vec3> = move_grips::box_grips(&bounds, set).into_iter().map(v3).collect();
    let corners = PyList::new(py, corners)?;
    let box_px = pixels(corners.as_any(), 0)?;
    let rotation = rotation.unwrap_or(true)
        && tool
            .getattr_opt("_grip_rotation")?
            .is_none_or(|v| v.is_truthy().unwrap_or(true));
    let handles: Vec<(Vec3, Vec3, Vec3)> = if rotation {
        let all = move_grips::rotation_grips(bounds.0, bounds.1, bounds.2, xyz(&cam.call_method0("eye")?)?.to_array());
        let px = all
            .iter()
            .map(|g| proj.pixel(glam::DVec3::from(g.0)))
            .collect::<PyResult<Vec<_>>>()?;
        let keep = move_grips::yield_to_box_grips(&px, &box_px, 2. * margin);
        all.into_iter()
            .zip(keep)
            .filter(|(_, k)| *k)
            .map(|((a, b, c), _)| (v3(a), v3(b), v3(c)))
            .collect()
    } else {
        vec![]
    };
    let handles = PyList::new(py, handles)?;
    let hot_box = move_grips::nearest(&box_px, cursor, margin);
    let hot = if hot_box.is_some() || handles.is_empty() {
        None
    } else {
        move_grips::nearest(&pixels(handles.as_any(), 1)?, cursor, margin)
    };
    tool.setattr("_grip_key", key)?;
    tool.setattr("_grip_box", obb_py(py, &bounds)?)?;
    tool.setattr("_grip_group", &group)?;
    tool.setattr("_grip_groups", PyList::new(py, targets)?)?;
    tool.setattr("_box_grips", corners)?;
    tool.setattr("_hot_box", hot_box)?;
    tool.setattr("_grips", handles)?;
    tool.setattr("_hot_grip", hot)
}

/// The modifier key steps the box grips to the next set (corners → edge
/// midpoints → side centres → centre), with the object selected and no grip
/// under the cursor. True when it did.
#[pyfunction]
pub fn cycle_box_grips(tool: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = tool.py();
    let Some(group) = tool.getattr_opt("_grip_group")?.filter(|g| !g.is_none()) else {
        return Ok(false);
    };
    let Some(b) = tool.getattr_opt("_grip_box")?.filter(|b| !b.is_none()) else {
        return Ok(false);
    };
    let selection = view.getattr("scene")?.getattr("selection")?;
    if !selection.contains(&group)?
        || !tool.getattr("_hot_box")?.is_none()
        || !tool.getattr("_hot_grip")?.is_none()
    {
        return Ok(false);
    }
    let set: usize = tool
        .getattr_opt("_grip_set")?
        .and_then(|s| s.extract().ok())
        .unwrap_or(0);
    let next = (set + 1) % move_grips::GRIP_SETS.len();
    tool.setattr("_grip_set", next)?;
    let points: Vec<Vec3> = move_grips::box_grips(&box_data(&b)?, next).into_iter().map(v3).collect();
    tool.setattr("_box_grips", PyList::new(py, points)?)?;
    if let Some(update) = view.getattr_opt("update")?.filter(|u| u.is_callable()) {
        update.call0()?;
    }
    Ok(true)
}

/// The tool context with its point moved onto the hot box grip: the click
/// (or hover) lands exactly on the grip, whatever the snap found under it.
#[pyfunction]
pub fn grip_context<'py>(tool: &Bound<'py, PyAny>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = tool.py();
    let Some(i) = tool.getattr_opt("_hot_box")?.and_then(|i| i.extract::<usize>().ok()) else {
        return Ok(ctx.clone());
    };
    let point = tool.getattr("_box_grips")?.get_item(i)?;
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("world", point)?;
    py.import("dataclasses")?.call_method("replace", (ctx,), Some(&kw))
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

/// Move borrows Rotate for one box grip and its immediately following angle
/// retype. Both tools keep their normal history and modifier handling.
#[derive(Default)]
enum Turn {
    #[default]
    Idle,
    Active(Py<PyAny>),
    Retype(Py<PyAny>),
}
#[pyclass(module = "built_core")]
#[derive(Default)]
pub struct GripTurn {
    phase: Turn,
}
impl GripTurn {
    fn end_if_committed(
        slf: &Bound<'_, Self>,
        tool: &Bound<'_, PyAny>,
        rotate: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        if rotate.getattr("start_point")?.is_none() {
            slf.borrow_mut().phase = Turn::Retype(rotate.clone().unbind());
            tool.call_method0("_clear_grips")?;
        }
        Ok(())
    }
}
#[pymethods]
impl GripTurn {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    #[getter]
    fn active(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        if let Turn::Active(r) = &self.phase {
            Some(r.clone_ref(py))
        } else {
            None
        }
    }
    #[getter]
    fn takes_angle(&self) -> bool {
        !matches!(self.phase, Turn::Idle)
    }
    fn clear(&mut self) {
        self.phase = Turn::Idle;
    }
    fn cancel(slf: &Bound<'_, Self>, view: &Bound<'_, PyAny>) -> PyResult<bool> {
        let phase = std::mem::take(&mut slf.borrow_mut().phase);
        if let Turn::Active(r) = phase {
            r.call_method1(view.py(), "on_cancel", (view,))?;
            return Ok(true);
        }
        Ok(false)
    }
    fn click(
        slf: &Bound<'_, Self>,
        tool: &Bound<'_, PyAny>,
        ctx: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let py = ctx.py();
        let active = slf.borrow().active(py);
        if let Some(r) = active {
            r.call_method1(py, "on_click", (ctx,))?;
            Self::end_if_committed(slf, tool, r.bind(py))?;
            return Ok(true);
        }
        slf.borrow_mut().clear();
        let grip = tool.getattr("_hot_grip")?;
        if grip.is_none() || !tool.getattr("start_point")?.is_none() {
            return Ok(false);
        }
        let (pos, center, n): (Bound<'_, PyAny>, Bound<'_, PyAny>, Bound<'_, PyAny>) =
            tool.getattr("_grips")?.get_item(grip)?.extract()?;
        let view = ctx.getattr("viewport")?;
        let rot = py.import("tools.rotate")?.call_method0("RotateTool")?;
        rot.call_method1("on_activate", (&view,))?;
        rot.setattr("_groups", PyList::new(py, tool.getattr("_grip_groups")?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?)?;
        rot.getattr("_gesture")?
            .call_method1("begin", (&rot, &view, false))?;
        super::transform_queries::CopyWire::capture(&rot, ctx)?;
        rot.setattr("_custom_axis", Vec3::from(xyz(&n)?.normalize_or_zero()))?;
        rot.setattr("start_point", center)?;
        rot.setattr("ref_point", &pos)?;
        rot.setattr("hover_point", pos)?;
        slf.borrow_mut().phase = Turn::Active(rot.unbind());
        view.call_method0("update")?;
        Ok(true)
    }
    fn value(
        slf: &Bound<'_, Self>,
        tool: &Bound<'_, PyAny>,
        view: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Option<bool>> {
        let py = view.py();
        let r = match &slf.borrow().phase {
            Turn::Idle => return Ok(None),
            Turn::Active(r) | Turn::Retype(r) => r.clone_ref(py),
        };
        if !tool.getattr("start_point")?.is_none() {
            return Ok(None);
        }
        let done = r
            .call_method1(py, "on_value", (view, value))?
            .is_truthy(py)?;
        if slf.borrow().active(py).is_some() {
            Self::end_if_committed(slf, tool, r.bind(py))?;
        }
        Ok(Some(done))
    }
}

/// A click on a box grip picks up what the grips stand for (the hovered
/// group or the selection) by that point, whatever lies under the cursor:
/// a corner out in the air has no geometry under it to pick.
pub(crate) fn grip_grab(tool: &Bound<'_, PyAny>, mode: &str) -> PyResult<bool> {
    let py = tool.py();
    let Some(groups) = tool.getattr_opt("_grip_pick")?.filter(|g| g.is_truthy().unwrap_or(false)) else {
        return Ok(false);
    };
    tool.setattr("_grip_pick", py.None())?;
    let groups = PyList::new(py, groups.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
    tool.setattr("_groups", groups)?;
    for name in ["_positions", "_verts", "_moved_guides", "_splanes", "_images"] {
        tool.setattr(name, PyList::empty(py))?;
    }
    if mode == "move" {
        tool.setattr("_labels", PyList::empty(py))?;
        tool.setattr("_dims", pyo3::types::PyDict::new(py))?;
    }
    Ok(true)
}

/// One box round what a carry moves, measured where they stand (the view's
/// cached boxes): the see-through pass asks it whether they hide the target.
#[pyfunction]
pub fn carried_box(view: &Bound<'_, PyAny>, groups: &Bound<'_, PyAny>) -> PyResult<Option<Py<PyAny>>> {
    let Some(obb) = view.getattr_opt("_group_obb")?.filter(|f| f.is_callable()) else {
        return Ok(None);
    };
    let groups = groups.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    match grip_box(&obb, &groups)? {
        Some(b) => Ok(Some(obb_py(view.py(), &b)?)),
        None => Ok(None),
    }
}

/// Whether the carried box, posed by `pose` (a Matrix4's column-major data),
/// stands between the eye and `target`.
#[pyfunction]
pub fn carry_hides(
    eye: &Bound<'_, PyAny>,
    target: &Bound<'_, PyAny>,
    obb: &Bound<'_, PyAny>,
    pose: [f64; 16],
) -> PyResult<bool> {
    if obb.is_none() || target.is_none() {
        return Ok(false);
    }
    Ok(move_grips::box_hides(
        xyz(eye)?.to_array(),
        xyz(target)?.to_array(),
        &box_data(obb)?,
        &pose,
    ))
}

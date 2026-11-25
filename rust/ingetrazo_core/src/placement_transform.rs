// SPDX-License-Identifier: GPL-3.0-or-later
//! Placement transforms and definition-axis changes. The editor supplies the
//! active occurrence frame. Materialised meshes retain exact snapshots for
//! direct replay, while enlisted stores replay their transaction.
use crate::math::{xyz, Matrix4, Vec3};
use pyo3::{prelude::*, types::{PyDict, PyList}};
use std::collections::HashSet;
type Obj<'py> = Bound<'py, PyAny>;
fn matrix(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.get_type::<Matrix4>().call0()
}
fn copy_matrix<'py>(value: &Obj<'py>) -> PyResult<Obj<'py>> {
    value.py().get_type::<Matrix4>().call1((value,))
}
fn rows<'py>(items: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    items.try_iter()?.collect()
}
fn apply_pose(scene: &Obj<'_>, g: &Obj<'_>, m: &Obj<'_>, pose: &Obj<'_>) -> PyResult<()> {
    let local = crate::doc::history_support::edit_local_transform(scene,g,m)?;
    g.setattr("xform", local.mul(pose)?)
}
fn move_mesh(mesh: &Obj<'_>, m: &Obj<'_>, delta: Option<&Obj<'_>>) -> PyResult<()> {
    for v in rows(&mesh.getattr("vertices")?)? {
        if let Some(delta) = delta {
            mesh.call_method1("move_vertex", (v, delta))?;
        } else {
            let p = v.getattr("position")?;
            let step = m.call_method1("map", (&p,))?.sub(v.getattr("position")?)?;
            mesh.call_method1("move_vertex", (v, step))?;
        }
    }
    Ok(())
}
fn finish_transform(g: &Obj<'_>, m: &Obj<'_>, texture: bool) -> PyResult<()> {
    let api = g.py().import("core.group")?;
    if texture {
        api.call_method1("_remap_uvws", (g.getattr("mesh")?, m))?;
    }
    api.call_method1("carry_axes", (g, m))?;
    Ok(())
}
#[pyfunction]
pub(crate) fn shift_group(scene: &Obj<'_>, g: &Obj<'_>, delta: &Obj<'_>) -> PyResult<()> {
    let pose = crate::doc::history_support::member_placement(scene,g)?;
    let m = matrix(scene.py())?;
    m.call_method1("translate", (delta,))?;
    if pose.is_none() {
        move_mesh(&g.getattr("mesh")?, &m, Some(delta))?;
        finish_transform(g, &m, true)
    } else {
        apply_pose(scene, g, &m, &pose)
    }
}
#[pyfunction]
pub(crate) fn flip_groups(scene: &Obj<'_>, groups: Vec<Obj<'_>>, m: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let store = py.import("ingetrazo_core.store_mesh")?.getattr("StoreMesh")?;
    for g in groups {
        let pose = crate::doc::history_support::member_placement(scene,&g)?;
        if !pose.is_none() {
            apply_pose(scene, &g, m, &pose)?;
            continue;
        }
        let mesh = g.getattr("mesh")?;
        move_mesh(&mesh, m, None)?;
        finish_transform(&g, m, false)?;
        if mesh.is_instance(&store)? {
            let st = mesh.getattr("_store")?;
            st.call_method1("reverse_faces", (st.call_method1("keys", ("face",))?,))?;
        } else {
            crate::doc::history_support::reverse_faces(&mesh.getattr("faces")?)?;
        }
    }
    Ok(())
}

type State = (Py<PyAny>, Py<PyAny>);
#[pyclass(module = "ingetrazo_core")]
struct GroupTransform {
    before: Option<State>,
    after: Option<State>,
}
#[pymethods]
impl GroupTransform {
    #[new]
    fn new() -> Self {
        Self {
            before: None,
            after: None,
        }
    }
    fn apply(
        &mut self,
        scene: &Obj<'_>,
        g: &Obj<'_>,
        make_matrix: &Obj<'_>,
        undo: bool,
        journal: bool,
    ) -> PyResult<bool> {
        let py = scene.py();
        let pose = if undo {
            g.getattr("xform")?
        } else {
            crate::doc::history_support::member_placement(scene,g)?
        };
        if !pose.is_none() {
            apply_pose(scene, g, &make_matrix.call1((undo,))?, &pose)?;
            return Ok(true);
        }
        let mesh = g.getattr("mesh")?;
        if let Some((state, axes)) = if undo { &self.before } else { &self.after } {
            mesh.call_method1("restore_state", (state.bind(py),))?;
            g.setattr("axes", axes.bind(py))?;
            return Ok(true);
        }
        if undo {
            return Ok(false);
        }
        let capture = || -> PyResult<State> {
            Ok((
                mesh.call_method0("capture_state")?.unbind(),
                g.getattr("axes")?.unbind(),
            ))
        };
        if !journal {
            self.before = Some(capture()?);
        }
        let m = make_matrix.call1((false,))?;
        move_mesh(&mesh, &m, None)?;
        finish_transform(g, &m, true)?;
        if !journal {
            self.after = Some(capture()?);
        }
        Ok(true)
    }
}

fn reexpress(g: &Obj<'_>, delta: &Obj<'_>) -> PyResult<()> {
    let proto = g.getattr("mesh")?;
    let moved = g
        .py()
        .import("core.group")?
        .call_method1("transformed_mesh", (&proto, delta))?;
    proto.call_method1("restore_state", (moved.call_method0("capture_state")?,))?;
    let kids = g.getattr("children")?;
    if kids.is_truthy()? {
        for child in kids.try_iter()? {
            let child = child?;
            let m = child.getattr("xform")?;
            if !m.is_none() {
                child.setattr("xform", delta.mul(m)?)?;
            }
        }
    }
    Ok(())
}

#[pyclass(module = "ingetrazo_core")]
struct AxesEdit {
    delta: Option<Py<PyAny>>,
    axes: Option<Py<PyAny>>,
    instances: Vec<(Py<PyAny>, Py<PyAny>, Py<PyAny>)>,
}
#[pymethods]
impl AxesEdit {
    #[new]
    fn new() -> Self {
        Self {
            delta: None,
            axes: None,
            instances: Vec::new(),
        }
    }
    fn fields_only(&self) -> bool {
        self.delta.is_none()
    }
    fn apply(&mut self, scene: &Obj<'_>, g: &Obj<'_>, frame: &Obj<'_>) -> PyResult<bool> {
        let py = g.py();
        let pose = g.getattr("xform")?;
        if pose.is_none() {
            self.axes = Some(g.getattr("axes")?.unbind());
            g.setattr("axes", copy_matrix(frame)?)?;
            return Ok(true);
        }
        let inv = frame.call_method0("inverted")?;
        if !inv.get_item(1)?.is_truthy()? {
            return Ok(false);
        }
        let delta = inv.get_item(0)?.mul(pose)?;
        let inverted = delta.call_method0("inverted")?;
        if !inverted.get_item(1)?.is_truthy()? {
            self.delta = None;
            return Ok(false);
        }
        let inverse = inverted.get_item(0)?;
        let mesh = g.getattr("mesh")?;
        let mut seen = HashSet::new();
        let mut instances = Vec::new();
        let walk = py.import("core.group")?.getattr("iter_placements")?;
        for root in scene.getattr("groups")?.try_iter()? {
            for entry in walk.call1((root?,))?.try_iter()? {
                let part = entry?.get_item(0)?;
                if part.getattr("mesh")?.is(&mesh)
                    && !part.getattr("xform")?.is_none()
                    && seen.insert(part.as_ptr())
                {
                    instances.push(part);
                }
            }
        }
        if !seen.contains(&g.as_ptr()) {
            instances.push(g.clone());
        }
        reexpress(g, &delta)?;
        self.delta = Some(delta.unbind());
        self.instances.clear();
        for part in &instances {
            self.instances.push((
                part.clone().unbind(),
                part.getattr("axes")?.unbind(),
                copy_matrix(&part.getattr("xform")?)?.unbind(),
            ));
        }
        for part in instances {
            part.setattr("xform", part.getattr("xform")?.mul(&inverse)?)?;
            part.setattr("axes", py.None())?;
        }
        Ok(true)
    }
    fn undo(&self, g: &Obj<'_>) -> PyResult<bool> {
        let py = g.py();
        let Some(delta) = &self.delta else {
            if g.getattr("xform")?.is_none() {
                g.setattr("axes", self.axes.as_ref().map(|v| v.bind(py)))?;
                return Ok(true);
            }
            return Ok(false);
        };
        let inverse = delta.bind(py).call_method0("inverted")?.get_item(0)?;
        reexpress(g, &inverse)?;
        for (part, axes, pose) in &self.instances {
            let part = part.bind(py);
            part.setattr("xform", copy_matrix(pose.bind(py))?)?;
            part.setattr("axes", axes.bind(py))?;
        }
        Ok(true)
    }
}

pub(crate) fn offset(kid: &Obj<'_>) -> PyResult<glam::DVec3> {
    let old = kid.getattr("explode_offset")?;
    if old.is_truthy()? {
        xyz(&old)
    } else {
        Ok(glam::DVec3::ZERO)
    }
}

#[pyfunction]
pub(crate) fn assembled_centres(container: &Obj<'_>) -> PyResult<Vec<Vec3>> {
    let points = container
        .py()
        .import("ingetrazo_core.parts")?
        .getattr("part_points")?;
    let mut result = Vec::new();
    for kid in container.getattr("children")?.try_iter()? {
        let kid = kid?;
        let pts = points.call1((&kid,))?;
        let mid = if pts.is_empty()? {
            glam::DVec3::ZERO
        } else {
            let lo = pts.call_method1("min", (0,))?;
            let hi = pts.call_method1("max", (0,))?;
            xyz(&lo.add(hi)?.div(2.)?.call_method0("tolist")?)?
        };
        result.push(Vec3::from(mid - offset(&kid)?));
    }
    Ok(result)
}
#[pyfunction]
pub(crate) fn explode_offsets(centres: Vec<Obj<'_>>, factor: f64, mode: &str) -> PyResult<Vec<Vec3>> {
    let centres = centres.iter().map(xyz).collect::<PyResult<Vec<_>>>()?;
    let Some(&first) = centres.first() else {
        return Ok(Vec::new());
    };
    let (lo, hi) = centres
        .iter()
        .fold((first, first), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    let mid = (lo + hi) * 0.5;
    let axes = match mode {
        "x" => glam::DVec3::X,
        "y" => glam::DVec3::Y,
        "z" => glam::DVec3::Z,
        _ => glam::DVec3::ONE,
    };
    Ok(centres
        .into_iter()
        .map(|p| Vec3::from((p - mid) * factor * axes))
        .collect())
}
#[pyfunction]
pub(crate) fn apply_explode(container: &Obj<'_>, factor: f64, mode: &str, centres: &Obj<'_>) -> PyResult<()> {
    let py = container.py();
    let children = container.getattr("children")?;
    if !children.is_truthy()? {
        return Ok(());
    }
    let kids = rows(&children)?;
    let factor = factor.max(0.);

    // Even a reassembly measures the assembled centres when none were supplied.
    let centres = if centres.is_none() {
        PyList::new(py, assembled_centres(container)?)?.into_any()
    } else {
        centres.clone()
    };
    let targets = if factor > 0. {
        explode_offsets(rows(&centres)?, factor, mode)?
            .into_iter()
            .map(|v| Some(glam::DVec3::from_array(v.coords())))
            .collect::<Vec<_>>()
    } else {
        vec![None; kids.len()]
    };
    for (kid, target) in kids.into_iter().zip(targets) {
        let old = offset(&kid)?;
        let step = matrix(py)?;
        step.call_method1("translate", (Vec3::from(target.unwrap_or_default() - old),))?;
        let mut pose = kid.getattr("xform")?;
        if pose.is_none() {
            kid.setattr("component", false)?;
            pose = matrix(py)?;
        }
        kid.setattr("xform", step.mul(pose)?)?;
        kid.setattr(
            "explode_offset",
            target
                .filter(|p| *p != glam::DVec3::ZERO)
                .map(|p| (p.x, p.y, p.z)),
        )?;
    }
    if factor > 0. {
        let value = PyDict::new(py);
        value.set_item("factor", factor)?;
        value.set_item("mode", mode)?;
        container.setattr("exploded", value)?;
    } else {
        container.setattr("exploded", py.None())?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn rotate_offsets(container: &Obj<'_>, transform: &Obj<'_>) -> PyResult<()> {
    let Some(children) = container.getattr_opt("children")?.filter(|v| !v.is_none()) else {
        return Ok(());
    };
    for kid in children.try_iter()? {
        let kid = kid?;
        if let Some(offset) = kid.getattr_opt("explode_offset")?.filter(|v| !v.is_none()) {
            let rotated = transform.call_method1("mapVector", (Vec3::from(xyz(&offset)?),))?;
            kid.setattr("explode_offset", rotated.call_method0("toTuple")?)?;
        }
    }
    Ok(())
}

#[pyfunction]
fn exploded_snapshot<'py>(
    container: &Obj<'py>,
) -> PyResult<(Obj<'py>, Vec<(Obj<'py>, Obj<'py>, Obj<'py>)>)> {
    let py = container.py();
    let state = container.getattr("exploded")?;
    let state = if state.is_truthy()? {
        py.get_type::<PyDict>().call1((state,))?
    } else {
        py.None().into_bound(py)
    };
    let mut kids = Vec::new();
    for k in container.getattr("children")?.try_iter()? {
        let k = k?;
        let pose = k.getattr("xform")?;
        let pose = if pose.is_none() {
            pose
        } else {
            copy_matrix(&pose)?
        };
        let offset = k.getattr("explode_offset")?;
        kids.push((k, pose, offset));
    }
    Ok((state, kids))
}
#[pyfunction]
fn restore_exploded(container: &Obj<'_>, snap: &Obj<'_>) -> PyResult<()> {
    let py = container.py();
    let state = snap.get_item(0)?;
    container.setattr(
        "exploded",
        if state.is_truthy()? {
            py.get_type::<PyDict>().call1((state,))?
        } else {
            py.None().into_bound(py)
        },
    )?;
    for row in snap.get_item(1)?.try_iter()? {
        let row = row?;
        let kid = row.get_item(0)?;
        kid.setattr("xform", row.get_item(1)?)?;
        kid.setattr("explode_offset", row.get_item(2)?)?;
    }
    Ok(())
}
// A world reframe also moves loose or classic meshes. Its survey datum is
// owned by the host, so keep both exact mesh states for mixed-command replay.
fn reframe_mesh(
    mesh: &Obj<'_>,
    m: &Obj<'_>,
    redo: bool,
    snaps: &Bound<'_, PyDict>,
) -> PyResult<()> {
    if !mesh.getattr("vertices")?.is_truthy()? {
        return Ok(());
    }
    let key = mesh.as_ptr() as usize;
    if let Some(saved) = snaps.get_item(key)? {
        mesh.call_method1("restore_state", (saved.get_item(usize::from(redo))?,))?;
    } else {
        let before = mesh.call_method0("capture_state")?;
        move_mesh(mesh, m, None)?;
        mesh.py()
            .import("core.group")?
            .call_method1("_remap_uvws", (mesh, m))?;
        snaps.set_item(key, (before, mesh.call_method0("capture_state")?))?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn reframe_world(
    scene: &Obj<'_>,
    m: &Obj<'_>,
    degrees: f64,
    redo: bool,
    snaps: &Bound<'_, PyDict>,
) -> PyResult<()> {
    for group in scene.getattr("groups")?.try_iter()? {
        let group = group?;
        let pose = group.getattr("xform")?;
        if pose.is_none() {
            reframe_mesh(&group.getattr("mesh")?, m, redo, snaps)?;
        } else {
            group.setattr("xform", m.mul(pose)?)?;
        }
    }
    reframe_mesh(&scene.getattr("mesh")?, m, redo, snaps)?;
    // Evaluate all right-hand sides before assigning a record, as two
    // document fields may expose live vectors referring to the same record.
    let records: &[(&str, &[(&str, &str)])] = &[
        (
            "dimensions",
            &[("a", "map"), ("b", "map"), ("offset", "mapVector")],
        ),
        ("text_labels", &[("anchor", "map"), ("offset", "mapVector")]),
        ("guides", &[("point", "map")]),
        (
            "section_planes",
            &[("point", "map"), ("normal", "mapVector")],
        ),
        (
            "image_planes",
            &[("origin", "map"), ("u", "mapVector"), ("v", "mapVector")],
        ),
    ];
    for &(collection, fields) in records {
        for item in scene.getattr(collection)?.try_iter()? {
            let item = item?;
            let mut values = Vec::with_capacity(fields.len());
            for &(field, map) in fields {
                values.push((field, m.call_method1(map, (item.getattr(field)?,))?));
            }
            for (field, value) in values {
                item.setattr(field, value)?;
            }
            if collection == "guides" && item.hasattr("direction")? {
                let d = item.getattr("direction")?;
                if !d.is_none() {
                    item.setattr("direction", m.call_method1("mapVector", (d,))?)?;
                }
            }
        }
    }
    for path in scene.getattr("geo_paths")?.try_iter()? {
        let path = path?;
        let points = path
            .getattr("points")?
            .try_iter()?
            .map(|p| m.call_method1("map", (p?,)))
            .collect::<PyResult<Vec<_>>>()?;
        path.setattr("points", points)?;
        if path.hasattr("_surface_tris")? && !path.getattr("_surface_tris")?.is_none() {
            path.setattr("_surface_tris", scene.py().None())?;
        }
    }
    for point in scene.getattr("geo_points")?.try_iter()? {
        let point = point?;
        point.setattr(
            "position",
            m.call_method1("map", (point.getattr("position")?,))?,
        )?;
    }
    let turn = degrees.to_radians();
    for view in scene.getattr("saved_views")?.try_iter()? {
        let view = view?;
        let target = m.call_method1("map", (view.getattr("target")?,))?;
        view.setattr("target", target.call_method0("toTuple")?)?;
        view.setattr("yaw", view.getattr("yaw")?.add(turn)?)?;
    }
    for sheet in scene.getattr("compositions")?.try_iter()? {
        for item in sheet?.call_method0("all_items")?.try_iter()? {
            let item = item?;
            for field in ["a_world", "b_world", "cam_target"] {
                if !item.hasattr(field)? {
                    continue;
                }
                let old = item.getattr(field)?;
                if !old.is_truthy()? {
                    continue;
                }
                item.setattr(
                    field,
                    xyz(&m.call_method1("map", (old,))?)?.to_array().to_vec(),
                )?;
                if field == "cam_target" && item.hasattr("cam_yaw")? {
                    let yaw = item.getattr("cam_yaw")?;
                    if !yaw.is_none() {
                        item.setattr("cam_yaw", yaw.add(turn)?)?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "placement_transform")?;
    sub.add_class::<GroupTransform>()?;
    sub.add_class::<AxesEdit>()?;
    sub.add_function(wrap_pyfunction!(reframe_world, &sub)?)?;
    sub.add_function(wrap_pyfunction!(shift_group, &sub)?)?;
    sub.add_function(wrap_pyfunction!(flip_groups, &sub)?)?;
    sub.add_function(wrap_pyfunction!(rotate_offsets, &sub)?)?;
    sub.add_function(wrap_pyfunction!(assembled_centres, &sub)?)?;
    sub.add_function(wrap_pyfunction!(explode_offsets, &sub)?)?;
    sub.add_function(wrap_pyfunction!(apply_explode, &sub)?)?;
    sub.add_function(wrap_pyfunction!(exploded_snapshot, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_exploded, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}

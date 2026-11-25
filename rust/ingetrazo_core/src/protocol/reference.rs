// SPDX-License-Identifier: GPL-3.0-or-later
//! Entry placement frames for world-coordinate reference edits.
use super::*;
fn copy_matrix<'py>(py: Python<'py>, m: &Obj<'py>) -> PyResult<Obj<'py>> {
    if m.is_none() {
        return Ok(m.clone());
    }
    py.get_type::<crate::math::Matrix4>().call1((m,))
}
#[pyfunction]
fn parts_record<'py>(group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = group.py();
    let axes = group
        .getattr_opt("axes")?
        .unwrap_or_else(|| py.None().into_bound(py));
    let parts = PyList::empty(py);
    for child in group.getattr("children")?.try_iter()? {
        let child = child?;
        parts.append((
            &child,
            copy_matrix(py, &child.getattr("xform")?)?,
            child.getattr("explode_offset")?,
        ))?;
    }
    Ok((copy_matrix(py, &axes)?, parts)
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn put_parts(group: &Obj<'_>, record: &Obj<'_>) -> PyResult<()> {
    let py = group.py();
    for part in record.get_item(1)?.try_iter()? {
        let part = part?;
        let child = part.get_item(0)?;
        child.setattr("xform", copy_matrix(py, &part.get_item(1)?)?)?;
        child.setattr("explode_offset", part.get_item(2)?)?;
    }
    group.setattr("axes", copy_matrix(py, &record.get_item(0)?)?)
}
#[pyfunction]
fn bake_container_xform(group: &Obj<'_>) -> PyResult<()> {
    let py = group.py();
    let Some(matrix) = group.getattr_opt("xform")?.filter(|v| !v.is_none()) else {
        return Ok(());
    };
    let identity = py.get_type::<crate::math::Matrix4>().call0()?;
    if matrix.eq(&identity)? {
        return Ok(());
    }
    for child in group.getattr("children")?.try_iter()? {
        let child = child?;
        let m = child.getattr("xform")?;
        child.setattr(
            "xform",
            matrix.mul(if m.is_none() { &identity } else { &m })?,
        )?;
    }
    py.import("core.explode")?
        .call_method1("rotate_offsets", (group, &matrix))?;
    let mesh = group.getattr("mesh")?;
    if mesh.getattr("vertices")?.is_truthy()? {
        group.setattr(
            "mesh",
            py.import("core.group")?
                .call_method1("transformed_mesh", (mesh, &matrix))?,
        )?;
    }
    group.setattr("xform", identity)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ReshareStep>()?;
    m.add_function(wrap_pyfunction!(enter_reference, m)?)?;
    m.add_function(wrap_pyfunction!(finish_reference, m)?)?;
    m.add_function(wrap_pyfunction!(restore_sharing, m)?)?;
    m.add_function(wrap_pyfunction!(share_back, m)?)?;
    m.add_function(wrap_pyfunction!(parts_record, m)?)?;
    m.add_function(wrap_pyfunction!(put_parts, m)?)?;
    Ok(())
}

fn null(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn placed_elsewhere(scene: &Obj<'_>, group: &Obj<'_>, mesh: &Obj<'_>) -> PyResult<bool> {
    for g in crate::model::module(scene.py(), "core.purge")?
        .call_method1("iter_groups", (scene.getattr("groups")?,))?
        .try_iter()?
    {
        let g = g?;
        if !g.is(group) && g.getattr("mesh")?.is(mesh) {
            return Ok(true);
        }
    }
    Ok(false)
}
#[pyfunction]
fn enter_reference<'py>(
    scene: &Obj<'py>,
    group: &Obj<'py>,
    frame: Option<&Obj<'py>>,
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = scene.py();
    let mut parts = null(py);
    let mut share = null(py);
    let proto = group.getattr("mesh")?;
    let xform = group.getattr_opt("xform")?.unwrap_or_else(|| null(py));
    let world = |m: &Obj<'py>| {
        crate::model::module(py, "core.group")?.call_method1("transformed_mesh", (&proto, m))
    };
    if group
        .getattr_opt("children")?
        .map(|v| v.is_truthy())
        .transpose()?
        .unwrap_or(false)
    {
        let component = group.getattr_opt("is_component")?;
        let instance = match component {
            Some(c) if c.is_callable() => {
                c.call0()?.is_truthy()? && placed_elsewhere(scene, group, &proto)?
            }
            _ => false,
        };
        if instance {
            parts = parts_record(group)?
        }
        bake_container_xform(group)?;
        if instance {
            if group.getattr("mesh")?.is(&proto) {
                group.setattr(
                    "mesh",
                    world(&py.get_type::<crate::math::Matrix4>().call0()?)?,
                )?;
            }
            share = (
                group,
                proto,
                py.get_type::<crate::math::Matrix4>().call1((&xform,))?,
                group.getattr("mesh")?.call_method0("capture_state")?,
            )
                .into_pyobject(py)?
                .into_any();
        }
        if let Some(f) = frame {
            group.setattr("axes", f)?;
        }
    } else if !xform.is_none() {
        let edited = world(&xform)?;
        group.setattr("mesh", &edited)?;
        group.setattr("xform", py.None())?;
        share = (group, proto, xform, edited.call_method0("capture_state")?)
            .into_pyobject(py)?
            .into_any();
    }
    Ok((share, parts))
}
fn turned<'py>(matrix: &Obj<'py>, offset: &Obj<'py>) -> PyResult<Obj<'py>> {
    let p = crate::math::xyz(offset)?;
    matrix
        .call_method1("mapVector", (crate::math::Vec3::from(p),))?
        .call_method0("toTuple")
}
fn unbake(group: &Obj<'_>, matrix: &Obj<'_>, parts: Option<&Obj<'_>>) -> PyResult<()> {
    let py = group.py();
    let Some(children) = group.getattr_opt("children")? else {
        return Ok(());
    };
    if !children.is_truthy()? || matrix.is_none() || group.getattr("xform")?.eq(matrix)? {
        return Ok(());
    }
    let inv = matrix.call_method0("inverted")?;
    if !inv.get_item(1)?.is_truthy()? {
        return Ok(());
    }
    let inv = inv.get_item(0)?;
    let id = py.get_type::<crate::math::Matrix4>().call0()?;
    let mut entry = std::collections::HashMap::new();
    let mut axes = null(py);
    if let Some(parts) = parts {
        axes = parts.get_item(0)?;
        for r in parts.get_item(1)?.try_iter()? {
            let r = r?;
            entry.insert(r.get_item(0)?.as_ptr(), (r.get_item(1)?, r.get_item(2)?));
        }
    }
    for child in children.try_iter()? {
        let child = child?;
        let m = child.getattr("xform")?;
        let now = if m.is_none() { &id } else { &m };
        let o = child.getattr("explode_offset")?;
        if let Some((m0, o0)) = entry.get(&child.as_ptr()) {
            let same = matrix.mul(if m0.is_none() { &id } else { m0 })?.eq(now)?;
            child.setattr(
                "xform",
                if same {
                    copy_matrix(py, m0)?
                } else {
                    inv.mul(now)?
                },
            )?;
            let offset = if o.is_truthy()? {
                if o0.is_truthy()?
                    && turned(matrix, o0)?.eq(py.get_type::<PyTuple>().call1((&o,))?)?
                {
                    o0.clone()
                } else {
                    turned(&inv, &o)?
                }
            } else {
                o
            };
            child.setattr("explode_offset", offset)?;
        } else {
            child.setattr("xform", inv.mul(now)?)?;
            if o.is_truthy()? {
                child.setattr("explode_offset", turned(&inv, &o)?)?;
            }
        }
    }
    group.setattr("axes", copy_matrix(py, &axes)?)
}
#[pyfunction]
#[pyo3(signature=(group,proto,xform,parts=None))]
fn restore_sharing(
    group: &Obj<'_>,
    proto: &Obj<'_>,
    xform: &Obj<'_>,
    parts: Option<&Obj<'_>>,
) -> PyResult<()> {
    unbake(group, xform, parts)?;
    group.setattr("mesh", proto)?;
    group.setattr("xform", xform)
}
#[pyfunction]
#[pyo3(signature=(group,proto,xform,edited,parts=None))]
fn share_back(
    group: &Obj<'_>,
    proto: &Obj<'_>,
    xform: &Obj<'_>,
    edited: &Obj<'_>,
    parts: Option<&Obj<'_>>,
) -> PyResult<()> {
    let inv = xform.call_method0("inverted")?;
    let tr = if inv.get_item(1)?.is_truthy()? {
        inv.get_item(0)?
    } else {
        xform.clone()
    };
    let local = crate::model::module(group.py(), "core.group")?
        .call_method1("transformed_mesh", (edited, tr))?;
    proto.call_method1("restore_state", (local.call_method0("capture_state")?,))?;
    proto.call_method0("note_rewound")?;
    restore_sharing(group, proto, xform, parts)
}
#[pyfunction]
fn finish_reference(share: &Obj<'_>, parts: Option<&Obj<'_>>) -> PyResult<bool> {
    let g = share.get_item(0)?;
    let proto = share.get_item(1)?;
    let m = share.get_item(2)?;
    let edited = g.getattr("mesh")?;
    let same = edited
        .call_method0("capture_state")?
        .eq(share.get_item(3)?)?;
    if same {
        restore_sharing(&g, &proto, &m, parts)?;
    } else {
        share_back(&g, &proto, &m, &edited, parts)?;
    }
    Ok(same)
}

#[pyclass(subclass, module = "ingetrazo_core.command_protocol")]
struct ReshareStep;
fn version(scene: &Obj<'_>) -> PyResult<()> {
    scene.setattr("version", scene.getattr("version")?.add(1)?)
}
fn replay_part(cmd: &Obj<'_>, scene: &Obj<'_>, method: &str) -> PyResult<()> {
    let current = scene.getattr("mesh")?;
    let target = cmd
        .getattr_opt("_history_mesh")?
        .filter(|m| !m.is_none() && !m.is(&current));
    if let Some(target) = target {
        target.setattr("_mut_serial", target.getattr("_mut_serial")?.add(1)?)?;
        scene.setattr("mesh", target)?;
        let result = cmd.call_method1(method, (scene,));
        scene.setattr("mesh", current)?;
        result?;
    } else {
        cmd.call_method1(method, (scene,))?;
    }
    Ok(())
}
fn restore_snapshot(proto: &Obj<'_>, state: &Obj<'_>) -> PyResult<()> {
    proto.call_method1("restore_state", (state,))?;
    proto.call_method0("note_rewound")?;
    Ok(())
}
fn world_frame(step: &Obj<'_>, parts: &str, state: &str) -> PyResult<()> {
    let py = step.py();
    let edited = step.getattr("edited")?;
    let g = step.getattr("group")?;
    edited.call_method1("restore_state", (step.getattr(state)?,))?;
    g.setattr("mesh", edited)?;
    g.setattr("xform", py.get_type::<crate::math::Matrix4>().call0()?)?;
    put_parts(&g, &step.getattr(parts)?)
}
fn local_frame(step: &Obj<'_>, parts: Option<&str>) -> PyResult<()> {
    let group = step.getattr("group")?;
    group.setattr("mesh", step.getattr("proto")?)?;
    let matrix = step.getattr("xform")?;
    if let Some(parts) = parts {
        group.setattr("xform", copy_matrix(step.py(), &matrix)?)?;
        put_parts(&group, &step.getattr(parts)?)?;
    } else {
        group.setattr("xform", matrix)?;
    }
    Ok(())
}
fn refresh(scene: &Obj<'_>) -> PyResult<()> {
    if let Some(f) = scene
        .getattr_opt("_refresh_dimension_holds")?
        .filter(|v| !v.is_none())
    {
        f.call0()?;
    }
    Ok(())
}
#[pymethods]
impl ReshareStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn r#do(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let step = slf.as_any();
        let group = step.getattr("group")?;
        let proto = step.getattr("proto")?;
        let xform = step.getattr("xform")?;
        let edited = step.getattr("edited")?;
        if step
            .getattr_opt("reference_assembly")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false))
        {
            refresh(scene)?;
            if step.getattr("_framed")?.is_none() {
                step.setattr("_framed", parts_record(&group)?)?;
                let state = edited.call_method0("capture_state")?;
                step.setattr("_edited", &state)?;
                step.setattr("_before", proto.call_method0("capture_state")?)?;
                let shared = !state.eq(step.getattr("state0")?)?;
                step.setattr("shared", shared)?;
                let parts = step.getattr("parts")?;
                if shared {
                    share_back(&group, &proto, &xform, &edited, Some(&parts))?
                } else {
                    restore_sharing(&group, &proto, &xform, Some(&parts))?
                }
                step.setattr("_after", proto.call_method0("capture_state")?)?;
                step.setattr("_left", parts_record(&group)?)?;
            } else {
                world_frame(step, "_entered", "state0")?;
                for cmd in step.getattr("inner")?.try_iter()? {
                    replay_part(&cmd?, scene, "do")?;
                }
                restore_snapshot(&proto, &step.getattr("_after")?)?;
                local_frame(step, Some("_left"))?;
            }
        } else if step.getattr("_before")?.is_none() {
            step.setattr("_before", proto.call_method0("capture_state")?)?;
            share_back(&group, &proto, &xform, &edited, None)?;
            step.setattr("_after", proto.call_method0("capture_state")?)?;
        } else {
            restore_snapshot(&proto, &step.getattr("_after")?)?;
            local_frame(step, None)?;
        }
        version(scene)
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let step = slf.as_any();
        let assembly = step
            .getattr_opt("reference_assembly")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false));
        if assembly {
            refresh(scene)?;
            world_frame(step, "_framed", "_edited")?;
        }
        restore_snapshot(&step.getattr("proto")?, &step.getattr("_before")?)?;
        if assembly {
            let cmds = step
                .getattr("inner")?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?;
            for cmd in cmds.into_iter().rev() {
                replay_part(&cmd, scene, "undo")?;
            }
            step.setattr("_entered", parts_record(&step.getattr("group")?)?)?;
        }
        local_frame(step, assembly.then_some("parts"))?;
        version(scene)
    }
}

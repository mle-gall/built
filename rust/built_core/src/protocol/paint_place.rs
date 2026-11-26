// SPDX-License-Identifier: GPL-3.0-or-later
//! Prepare keyed face paint and stamp placement tools at the document boundary.
use super::annotations::vec3;
use super::placements::{one, resolve_placements};
use super::transforms::{apply, empty};
use super::*;
#[pyfunction]
#[pyo3(signature=(history,placements,paint,material=None))]
fn paint_placements<'py>(
    history: &Obj<'py>,
    placements: &Obj<'py>,
    paint: &Obj<'py>,
    material: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let parts = PyList::empty(py);
    let gs = PyList::empty(py);
    let expected = if paint.is_truthy()? {
        paint.clone()
    } else {
        py.None().into_bound(py)
    };
    for g in resolve_placements(history, placements)?.iter() {
        if g.getattr("material")?.ne(&expected)? {
            gs.append(g)?;
        }
    }
    if let Some(m) = material {
        let name = m.getattr("name")?;
        if scene
            .getattr("materials")?
            .call_method1("get", (&name,))?
            .is_none()
        {
            parts.append(history_command(
                py,
                "SetFaceMaterialTagCommand",
                (PyList::empty(py), name, m),
            )?)?;
        }
    }
    if !gs.is_empty() {
        parts.append(
            py.import("built_core")?
                .getattr("NativeCommand")?
                .call_method1("paint", (gs, paint))?,
        )?;
    }
    apply(history, &parts)
}
#[pyfunction]
#[pyo3(signature=(history,*,placement=None,mesh=None,faces=None,seed=None,material=None,color=None,texture=None,opacity=None,default=false,back=false,plane=None,_picked_placement=None))]
fn paint_material<'py>(
    history: &Obj<'py>,
    placement: Option<&Obj<'py>>,
    mesh: Option<&Obj<'py>>,
    faces: Option<&Obj<'py>>,
    seed: Option<&Obj<'py>>,
    material: Option<&Obj<'py>>,
    color: Option<&Obj<'py>>,
    texture: Option<&Obj<'py>>,
    opacity: Option<&Obj<'py>>,
    default: bool,
    back: bool,
    plane: Option<&Obj<'py>>,
    _picked_placement: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let faces = empty(py, faces);
    if placement.is_some() == faces.is_truthy()? {
        return Err(PyValueError::new_err(
            "paint_material needs a placement or faces",
        ));
    }
    let (group, target, targets, clicked) = if let Some(id) = placement {
        let group = match _picked_placement {
            Some(g) if g.getattr("uid")?.eq(id)? => g.clone(),
            _ => one(history, id)?,
        };
        (Some(group), None, PyList::empty(py), None)
    } else {
        let target = py
            .import("built_core.command_protocol")?
            .call_method1("resolve_mesh", (&scene, mesh))?;
        let targets = need(&target, "face", &faces)?;
        let clicked = if let Some(seed) = seed {
            need(&target, "face", PyList::new(py, [seed])?.as_any())?.get_item(0)?
        } else {
            targets.get_item(0)?
        };
        (None, Some(target), targets, Some(clicked))
    };
    let mut color = color.cloned();
    let mut texture = texture.cloned();
    let mut opacity = opacity.cloned();
    let mut mat = if let Some(m) = material {
        if m.is_instance_of::<pyo3::types::PyString>() {
            let mat = py
                .import("built_core.command_protocol")?
                .call_method1("material", (history, m))?;
            if opacity.is_none() {
                opacity = mat.getattr_opt("opacity")?.filter(|v| !v.is_none());
            }
            Some(mat)
        } else {
            Some(
                py.import("built_core.materials")?
                    .getattr("Material")?
                    .call_method1("from_dict", (m,))?,
            )
        }
    } else {
        None
    };
    if let Some(m) = &mat {
        if color.is_none() && texture.is_none() {
            color = m.getattr_opt("color")?.filter(|v| !v.is_none());
            texture = m.getattr_opt("texture")?.filter(|v| !v.is_none());
            if opacity.is_none() {
                opacity = m.getattr_opt("opacity")?.filter(|v| !v.is_none());
            }
        }
    }
    if let Some(c) = &color {
        let vals = c
            .try_iter()?
            .map(|v| v?.extract::<f64>())
            .collect::<PyResult<Vec<_>>>()?;
        if vals.len() != 3 || vals.iter().any(|v| !(0.0..=1.0).contains(v)) {
            return Err(PyValueError::new_err(
                "paint colour needs three components from zero to one",
            ));
        }
    }
    if let Some(o) = &opacity {
        if !(0.0..=1.0).contains(&o.extract::<f64>()?) {
            return Err(PyValueError::new_err(
                "paint opacity must be between zero and one",
            ));
        }
    }
    if color.is_none() && texture.is_none() && !default {
        return Err(PyValueError::new_err(
            "paint_material needs a colour, texture, or default",
        ));
    }
    if mat.is_none() && !default {
        let kw = PyDict::new(py);
        kw.set_item("color", &color)?;
        kw.set_item("texture", &texture)?;
        kw.set_item("opacity", &opacity)?;
        mat = Some(py.import("built_core.material_commands")?.call_method(
            "material_for_paint",
            (scene,),
            Some(&kw),
        )?);
    }
    let plane = plane
        .map(|p| -> PyResult<_> { Ok((vec3(&p.get_item(0)?)?, p.get_item(1)?.extract::<f64>()?)) })
        .transpose()?;
    let cmd = py.import("built_core")?.call_method1(
        "build_paint",
        (
            target, targets, clicked, group, back, default, color, texture, opacity, mat, plane,
        ),
    )?;
    execute(history, &cmd)
}
#[pyfunction]
#[pyo3(signature=(history,template,basis,offset,point=None,attachment=None,_pending=None,wall_host=None))]
fn place_group<'py>(
    history: &Obj<'py>,
    template: &Obj<'py>,
    basis: &Obj<'py>,
    offset: &Obj<'py>,
    point: Option<&Obj<'py>>,
    attachment: Option<&Obj<'py>>,
    _pending: Option<&Obj<'py>>,
    wall_host: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let groups = py.import("built_core.group")?;
    let scene = history.getattr("scene")?;
    let g = match _pending {
        Some(g) if g.getattr("uid")?.ne(template)? => {
            return Err(PyKeyError::new_err("unknown placement template"))
        }
        Some(g) => g.clone(),
        None => groups.call_method1("copy_group", (one(history, template)?,))?,
    };
    let axes = basis.extract::<[[f64; 3]; 3]>()?;
    let shift = offset.extract::<[f64; 3]>()?;
    let mut rows = Vec::with_capacity(16);
    for i in 0..3 {
        rows.extend([axes[0][i], axes[1][i], axes[2][i], shift[i]]);
    }
    rows.extend([0., 0., 0., 1.]);
    let m = py
        .import("built_core")?
        .getattr("Matrix4")?
        .call(PyTuple::new(py, rows)?, None)?;
    let anchor = match attachment {
        Some(a) if a.is_instance_of::<PyDict>() => Some(
            py.import("built_core.annotation_anchor")?
                .getattr("Attachment")?
                .call_method1("restore", (&scene, a, vec3(point.unwrap_or(offset))?))?,
        ),
        _ => attachment.cloned(),
    };
    let pose = g.getattr("xform")?;
    let old_axes = g.getattr("axes")?;
    let mesh = g.getattr("mesh")?;
    let state = if pose.is_none() {
        Some(mesh.call_method0("capture_state")?)
    } else {
        None
    };
    let link = if let Some(host) = wall_host {
        let link = py.get_type::<PyDict>().call1((host,))?;
        let wall = one(history, &host.get_item("uid")?)?;
        if wall.getattr("locked")?.is_truthy()?
            || !matches!(
                wall.getattr("ifc")?
                    .get_item("class")?
                    .extract::<String>()?
                    .as_str(),
                "IfcWall" | "IfcWallStandardCase"
            )
        {
            return Err(PyValueError::new_err("Select an unlocked wall"));
        }
        let flipped = host.get_item("flipped")?.is_truthy()?;
        let n = match host.get_item("normal") {
            Ok(n) => glam::DVec3::from_array(n.extract()?),
            Err(_) => glam::DVec3::from_array(axes[1]) * if flipped { 1. } else { -1. },
        };
        let reveal = host.get_item("offset")?.extract::<f64>()?;
        let matrix = crate::math::mat(&m)?;
        let profile = match host.get_item("outline") {
            Ok(outline) => outline
                .extract::<[[f64; 3]; 4]>()?
                .map(glam::DVec3::from_array),
            Err(_) => {
                use crate::tools::host_geometry::ToolBounds;
                let anchor = ToolBounds::placement_bounds(&g)?.anchor(true);
                let [w, h] = ToolBounds::opening_size(&g)?;
                let a = matrix.transform_point3(anchor) + n * reveal;
                let right = glam::DVec3::Z.cross(n) * w * 0.5;
                [
                    a - right,
                    a + right,
                    a + right + glam::DVec3::Z * h,
                    a - right + glam::DVec3::Z * h,
                ]
            }
        };
        let placed = if pose.is_none() {
            matrix
        } else {
            matrix * crate::math::mat(&pose)?
        };
        let inv = placed.inverse();
        link.set_item("normal", n.to_array())?;
        link.set_item(
            "profile",
            profile.map(|p| inv.transform_point3(p).to_array()),
        )?;
        link.set_item(
            "facing",
            inv.transform_vector3(if flipped { -n } else { n })
                .to_array(),
        )?;
        link.set_item(
            "anchor",
            inv.transform_point3(profile[0] - n * reveal).to_array(),
        )?;
        link.call_method1("pop", ("outline", py.None()))?;
        Some(link)
    } else {
        None
    };
    let before = history.getattr("revision")?;
    let run = (|| -> PyResult<()> {
        if let Some(link) = &link {
            if g.getattr("ext")?.is_none() {
                g.setattr("ext", PyDict::new(py))?;
            }
            g.getattr("ext")?.set_item("wall_host", link)?;
        }
        if link.is_some() && pose.is_none() {
            g.setattr("xform", &m)?;
        } else if !pose.is_none() {
            g.setattr("xform", m.mul(&pose)?)?;
        } else {
            let verts = py
                .get_type::<PyList>()
                .call1((mesh.getattr("vertices")?,))?;
            for v in verts.try_iter()? {
                let v = v?;
                let p = v.getattr("position")?;
                let delta = m.call_method1("map", (&p,))?.sub(&p)?;
                if delta.call_method0("length")?.extract::<f64>()? > 1e-9 {
                    mesh.call_method1("move_vertex", (v, delta))?;
                }
            }
            crate::model::module(groups.py(), "built_core")?
                .call_method1("group_remap_uvws", (&mesh, &m))?;
            groups.call_method1("carry_axes", (&g, &m))?;
        }
        let kw = PyDict::new(py);
        kw.set_item("attachment", anchor)?;
        let cmd =
            py.import("built_core.history")?
                .call_method("InsertGroupCommand", (&g,), Some(&kw))?;
        let api = py.import("built_core.history")?;
        let mut steps = Vec::new();
        if let Some(key) = g
            .getattr_opt("ext")?
            .and_then(|ext| ext.get_item("library_family_type").ok())
        {
            let reg = py
                .import("core.bim_registry")?
                .call_method1("get", (&scene,))?;
            let types = reg.get_item("types")?;
            let record = g.getattr("ifc")?;
            if !types.contains(&key)? {
                let ty = PyDict::new(py);
                ty.set_item("name", record.get_item("name")?)?;
                ty.set_item("ifc_class", record.get_item("class")?)?;
                ty.set_item("psets", record.get_item("psets")?)?;
                types.set_item(&key, ty)?;
                steps.push(api.call_method1("SetRegistryCommand", (reg,))?);
            }
            record.set_item("type", key)?;
        }
        steps.push(cmd);
        let step = if steps.len() == 1 {
            steps.pop().unwrap()
        } else {
            api.call_method1("CompoundCommand", (steps,))?
        };
        history.call_method1("execute", (step,))?;
        Ok(())
    })();
    if history.getattr("revision")?.eq(&before)? {
        g.setattr("xform", pose)?;
        g.setattr("axes", old_axes)?;
        if let Some(state) = state {
            mesh.call_method1("restore_state", (state,))?;
        }
    }
    run?;
    answer(history, &before, true)
}
/// Adjust the fill along the wall normal and vertical in one history step.
#[pyfunction]
fn set_opening_position<'py>(
    history: &Obj<'py>,
    placement: &Obj<'py>,
    sill: f64,
    offset: f64,
) -> Reply<'py> {
    if !sill.is_finite() || sill < 0. || !offset.is_finite() {
        return Err(PyValueError::new_err("Invalid opening position"));
    }
    let g = one(history, placement)?;
    let link = g.getattr("ext")?.get_item("wall_host")?;
    let normal = glam::DVec3::from_array(link.get_item("normal")?.extract()?);
    let delta = glam::DVec3::Z * (sill - link.get_item("sill")?.extract::<f64>()?)
        - normal * (offset - link.get_item("offset")?.extract::<f64>()?);
    if delta.length_squared() < 1e-16 {
        return answer(history, &history.getattr("revision")?, false);
    }
    execute(
        history,
        &history_command(
            history.py(),
            "MoveGroupCommand",
            (g, crate::math::Vec3::from(delta)),
        )?,
    )
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(paint_placements, m)?)?;
    m.add_function(wrap_pyfunction!(paint_material, m)?)?;
    m.add_function(wrap_pyfunction!(place_group, m)?)?;
    m.add_function(wrap_pyfunction!(set_opening_position, m)?)?;
    Ok(())
}

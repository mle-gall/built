// SPDX-License-Identifier: GPL-3.0-or-later
//! Build one reversible stamp, including its new boundary edges and tag/material definitions.
use super::{transforms::vector, Obj};
use crate::math::xyz;
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::collections::HashMap;

#[pyclass(dict, module = "ingetrazo_core")]
struct PruneStamp {
    gone: Vec<Py<PyAny>>,
}

#[pymethods]
impl PruneStamp {
    #[classattr]
    #[pyo3(name = "keeps_groups")]
    const KEEPS_GROUPS: bool = true;

    fn r#do(&mut self, scene: &Obj<'_>) -> PyResult<()> {
        if !self.gone.is_empty() {
            scene
                .getattr("mesh")?
                .call_method1("relink_vertices", (PyList::new(scene.py(), &self.gone)?,))?;
            self.gone.clear();
        }
        Ok(())
    }

    fn undo(&mut self, scene: &Obj<'_>) -> PyResult<()> {
        let py = scene.py();
        let mesh = scene.getattr("mesh")?;
        let vertices = mesh
            .getattr("vertices")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?;
        mesh.call_method0("prune_orphan_vertices")?;
        let keys = |attr| -> PyResult<Bound<'_, PySet>> {
            let keys = PySet::empty(py)?;
            for item in mesh.getattr(attr)?.try_iter()? {
                keys.add(item?.getattr("key")?)?;
            }
            Ok(keys)
        };
        let kept = keys("vertices")?;
        self.gone.clear();
        for v in vertices {
            if !kept.contains(v.getattr("key")?)? {
                self.gone.push(v.unbind());
            }
        }
        let faces = keys("faces")?;
        let face = py.import("ingetrazo_core.mesh")?.getattr("Face")?;
        let sel = scene.getattr("selection")?;
        for ent in sel.try_iter()?.collect::<PyResult<Vec<_>>>()? {
            if ent.is_instance(&face)? && !faces.contains(ent.getattr("key")?)? {
                sel.call_method1("discard", (ent,))?;
            }
        }
        scene.setattr("version", scene.getattr("version")?.add(1)?)?;
        Ok(())
    }
}

fn points(list: &Obj<'_>) -> PyResult<Vec<DVec3>> {
    list.try_iter()?.map(|p| xyz(&p?)).collect()
}

fn shifted<'py>(py: Python<'py>, coords: &[DVec3], off: DVec3) -> PyResult<Bound<'py, PyList>> {
    let result = PyList::empty(py);
    for &p in coords {
        result.append(vector(py, p + off)?)?;
    }
    Ok(result)
}

struct Plane {
    origin: DVec3,
    normal: DVec3,
    min: DVec3,
    max: DVec3,
}

// Keep first-seen order: a command rebuilding one plane can affect its neighbours.
fn plane_merges(
    steps: &Bound<PyList>,
    api: &Bound<PyModule>,
    mesh: &Obj<'_>,
    loops: &[Vec<DVec3>],
) -> PyResult<()> {
    let mut planes = Vec::<Plane>::new();
    let mut index = HashMap::new();
    for poly in loops.iter().filter(|p| p.len() >= 3) {
        let raw = crate::polygon::raw_newell(poly);
        let mut n = if raw.length() < 1e-9 {
            DVec3::Z
        } else {
            raw.normalize()
        };
        if n.z < 0. || (n.z == 0. && (n.y < 0. || (n.y == 0. && n.x < 0.))) {
            n = -n;
        }
        let key = [n.x, n.y, n.z, n.dot(poly[0])].map(|v| (v * 1e4).round_ties_even() as i64);
        let i = *index.entry(key).or_insert_with(|| {
            planes.push(Plane {
                origin: poly[0],
                normal: n,
                min: poly[0],
                max: poly[0],
            });
            planes.len() - 1
        });
        if planes.len() > 24 {
            return Ok(());
        }
        let plane = &mut planes[i];
        for &p in poly {
            plane.min = plane.min.min(p);
            plane.max = plane.max.max(p);
        }
    }
    let faces = mesh.getattr("faces")?;
    if planes.is_empty() || !faces.is_truthy()? {
        return Ok(());
    }
    let vertices = mesh
        .getattr("vertices")?
        .try_iter()?
        .map(|v| xyz(&v?.getattr("position")?))
        .collect::<PyResult<Vec<_>>>()?;
    let tol = 1e-4;
    for plane in planes {
        let distance = |p: DVec3| plane.normal.dot(p - plane.origin).abs();
        if vertices
            .iter()
            .filter(|&&p| distance(p) < tol)
            .take(3)
            .count()
            < 3
        {
            continue;
        }
        for face in faces.try_iter()? {
            let ps = points(&face?.getattr("vertices")?)?;
            if ps.is_empty() || ps.iter().any(|&p| distance(p) > tol) {
                continue;
            }
            let (lo, hi) = ps
                .iter()
                .fold((ps[0], ps[0]), |(lo, hi), &p| (lo.min(p), hi.max(p)));
            if (0..3).all(|i| {
                (lo[i] < plane.max[i] - tol && plane.min[i] < hi[i] - tol)
                    || (hi[i] - lo[i] < tol && plane.max[i] - plane.min[i] < tol)
            }) {
                let py = steps.py();
                steps.append(api.call_method1(
                    "RebuildPlaneFacesCommand",
                    (vector(py, plane.origin)?, vector(py, plane.normal)?),
                )?)?;
                break;
            }
        }
    }
    Ok(())
}

fn remapped<'py>(names: &Bound<'py, PyDict>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(names.get_item(key)?.unwrap_or_else(|| key.clone()))
}

fn groups<'py>(
    clip: &Obj<'py>,
    off: &Obj<'py>,
    names: &Bound<'py, PyDict>,
    renamed: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyList>> {
    let py = clip.py();
    let groups = PyList::empty(py);
    let src = clip.call_method1("get", ("groups", &groups))?;
    let api = py.import("core.group")?;
    let native = py.import("ingetrazo_core")?;
    let mut remap = !renamed.is_empty();
    for (a, b) in names.iter() {
        remap |= a.ne(b)?;
    }
    for root in src.try_iter()? {
        let copy = api.call_method1("copy_group", (root?, off))?;
        if remap {
            for placement in api.call_method1("iter_placements", (&copy,))?.try_iter()? {
                let g = placement?.get_item(0)?;
                g.setattr("layer", remapped(names, &g.getattr("layer")?)?)?;
                let mesh = crate::model::module(api.py(), "ingetrazo_core")?.call_method1("group_transformed_mesh",
                    (g.getattr("mesh")?, native.call_method0("Matrix4")?),
                )?;
                g.setattr("mesh", &mesh)?;
                if !renamed.is_empty() {
                    native.call_method1("link_restamp_materials", (&mesh, renamed))?;
                    let mat = g.getattr("material")?;
                    if mat.is_truthy()? {
                        g.setattr(
                            "material",
                            native.call_method1("link_renamed_paint", (mat, renamed))?,
                        )?;
                    }
                }
                for face in mesh.getattr("faces")?.try_iter()? {
                    let attrs = face?.getattr("attrs")?;
                    let layer = attrs.call_method1("get", ("layer",))?;
                    if let Some(to) = names.get_item(layer)? {
                        attrs.set_item("layer", to)?;
                    }
                }
                for edge in mesh.getattr("edges")?.try_iter()? {
                    let e = edge?;
                    e.setattr("layer", remapped(names, &e.getattr("layer")?)?)?;
                }
            }
        }
        groups.append(copy)?;
    }
    Ok(groups)
}

pub(super) fn stamp(history: &Obj<'_>, clip: &Obj<'_>, offset: DVec3) -> PyResult<bool> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let api = py.import("ingetrazo_core.history")?;
    let native = py.import("ingetrazo_core")?;
    let off = vector(py, offset)?;
    let copy_attrs = py
        .import("core.ifc_identity")?
        .call_method1("copied_face_attrs", (&scene,))?;
    let layers = clip.call_method1("get", ("layers",))?;
    let templates = if layers.is_truthy()? {
        layers.clone()
    } else {
        PyDict::new(py).into_any()
    };
    let names = native
        .getattr("tag_edits")?
        .call_method1("template_name_map", (&scene, templates))?
        .cast_into::<PyDict>()?;
    let steps = PyList::empty(py);
    steps.append(Py::new(py, PruneStamp { gone: vec![] })?)?;
    let renamed = PyDict::new(py);
    let mats = clip.call_method1("get", ("materials",))?;
    if mats.is_truthy()? {
        let current = scene.getattr("materials")?;
        let reg = py.import("builtins")?.call_method1("dict", (&current,))?;
        let materials = py.import("core.materials")?;
        for (old, mat) in mats.cast::<PyDict>()?.iter() {
            let name = materials.call_method1("register", (&reg, mat))?;
            if old.ne(&name)? {
                renamed.set_item(old, &name)?;
            }
            if !current.contains(&name)? {
                steps.append(api.call_method1(
                    "SetFaceMaterialTagCommand",
                    (PyList::empty(py), &name, reg.get_item(&name)?),
                )?)?;
            }
        }
    }
    if layers.is_truthy()? {
        steps.append(api.call_method1("EnsureLayersCommand", (layers,))?)?;
    }
    let guards = steps.len();
    let faces = clip.get_item("faces")?;
    let mut planned = Vec::new();
    for row in faces.try_iter()? {
        let row = row?;
        let outer = points(&row.get_item(0)?)?;
        let holes = row
            .get_item(1)?
            .try_iter()?
            .map(|h| points(&h?))
            .collect::<PyResult<Vec<_>>>()?;
        for ring in std::iter::once(&outer).chain(&holes) {
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                steps.append(api.call_method1(
                    "AddEdgeCommand",
                    (vector(py, *a + offset)?, vector(py, *b + offset)?),
                )?)?;
            }
        }
        planned.push((row, outer, holes));
    }
    let mut face_steps = Vec::new();
    let mut loops = Vec::new();
    let translate = py.import("ingetrazo_core")?.getattr("group_translated_attrs")?;
    for (row, outer, holes) in planned {
        let attrs = if row.len()? > 2 && row.get_item(2)?.is_truthy()? {
            translate.call1((row.get_item(2)?, &off))?
        } else {
            py.None().into_bound(py)
        };
        let attrs = copy_attrs.call1((attrs,))?;
        let tag = attrs.call_method1("get", ("layer",))?;
        if let Some(name) = names.get_item(tag)? {
            attrs.set_item("layer", name)?;
        }
        let kw = PyDict::new(py);
        kw.set_item("auto", false)?;
        kw.set_item(
            "attrs",
            native.call_method1("link_renamed_paint", (attrs, &renamed))?,
        )?;
        if !holes.is_empty() {
            let hs = PyList::empty(py);
            for hole in holes {
                hs.append(shifted(py, &hole, offset)?)?;
            }
            kw.set_item("holes", hs)?;
        }
        let cmd = api.call_method("AddFaceCommand", (shifted(py, &outer, offset)?,), Some(&kw))?;
        steps.append(&cmd)?;
        face_steps.push(cmd);
        loops.push(outer.into_iter().map(|p| p + offset).collect());
    }
    plane_merges(&steps, &api, &mesh, &loops)?;
    let edge_layers = clip.call_method1("get", ("edge_layers",))?;
    let ids = PyDict::new(py);
    let mesh_type = py.import("ingetrazo_core.mesh")?.getattr("Mesh")?;
    let mut copied_edges = Vec::new();
    for (i, row) in clip.get_item("edges")?.try_iter()?.enumerate() {
        let row = row?;
        let a = vector(py, xyz(&row.get_item(0)?)? + offset)?;
        let b = vector(py, xyz(&row.get_item(1)?)? + offset)?;
        let curve = row.get_item(3)?;
        let kw = PyDict::new(py);
        if !curve.is_none() {
            if !ids.contains(&curve)? {
                ids.set_item(&curve, mesh_type.call_method0("next_curve_id")?)?;
            }
            kw.set_item("curve", ids.get_item(curve)?)?;
        }
        let soft = row.get_item(2)?;
        if soft.is_truthy()? {
            kw.set_item("soft", soft)?;
        }
        if !edge_layers.is_none() && i < edge_layers.len()? {
            kw.set_item("layer", remapped(&names, &edge_layers.get_item(i)?)?)?;
        }
        steps.append(api.call_method("AddEdgeCommand", (&a, &b), Some(&kw))?)?;
        copied_edges.push((a, b));
    }
    let picked = groups(clip, &off, &names, &renamed)?;
    for group in picked.iter() {
        steps.append(api.call_method1("InsertGroupCommand", (group,))?)?;
    }
    if steps.len() == guards {
        return Ok(false);
    }
    let before = history.getattr("revision")?;
    history.call_method1("execute", (api.call_method1("CompoundCommand", (steps,))?,))?;
    if history.getattr("revision")?.eq(before)? {
        return Ok(false);
    }
    let live = PySet::empty(py)?;
    for f in mesh.getattr("faces")?.try_iter()? {
        live.add(f?.getattr("key")?)?;
    }
    for cmd in face_steps {
        let f = cmd.getattr("face")?;
        if !f.is_none() && live.contains(f.getattr("key")?)? {
            picked.append(f)?;
        }
    }
    for (a, b) in copied_edges {
        let v0 = mesh.call_method1("vertex_at", (a,))?;
        let v1 = mesh.call_method1("vertex_at", (b,))?;
        if !v0.is_none() && !v1.is_none() {
            let edge = mesh.call_method1("find_edge", (v0, v1))?;
            if !edge.is_none() {
                picked.append(edge)?;
            }
        }
    }
    scene.call_method1("select", (picked,))?;
    Ok(true)
}

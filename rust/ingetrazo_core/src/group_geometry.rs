// SPDX-License-Identifier: GPL-3.0-or-later
//! Move captured loose geometry into a new placement, retaining all paint
//! and edge flags. The facade supplies geometry owners and the active frame.
use crate::{
    math::{xyz, Vec3},
    store::commands::cell,
};
use glam::DVec3;
use pyo3::{prelude::*, types::PyDict};
use std::collections::{BTreeSet, HashSet};
type Obj<'py> = Bound<'py, PyAny>;
type Ring = Vec<DVec3>;

fn positions(points: &Obj<'_>) -> PyResult<Ring> {
    points.try_iter()?.map(|p| xyz(&p?)).collect()
}
fn ends(e: &Obj<'_>) -> PyResult<[DVec3; 2]> {
    Ok([xyz(&e.getattr("a")?)?, xyz(&e.getattr("b")?)?])
}
fn rows<'py>(items: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    items.try_iter()?.collect()
}
fn key(p: DVec3) -> [u64; 3] {
    cell(p.to_array())
}

struct FaceSpec {
    outer: Ring,
    holes: Vec<Ring>,
    attrs: Py<PyAny>,
}
#[pyclass(module = "ingetrazo_core")]
pub(crate) struct GroupGeometry {
    faces: Vec<FaceSpec>,
    edges: Vec<[DVec3; 2]>,
    flags: Vec<([DVec3; 2], Py<PyAny>)>,
}

#[pymethods]
impl GroupGeometry {
    #[new]
    fn new(py: Python<'_>, faces: &Obj<'_>, edges: &Obj<'_>) -> PyResult<Self> {
        let facade = py.import("ingetrazo_core.mesh")?;
        let read = facade.getattr("edge_flags")?;
        let plain = facade.getattr("edge_is_plain")?;
        let mut captured = Vec::new();
        let mut flags = Vec::new();
        let mut note = |e: &Obj<'_>| -> PyResult<()> {
            if !plain.call1((e,))?.is_truthy()? {
                flags.push((ends(e)?, read.call1((e,))?.unbind()));
            }
            Ok(())
        };
        let faces = rows(faces)?;
        let mut loose = Vec::new();
        for e in edges.try_iter()? {
            loose.push(ends(&e?)?);
        }
        for f in faces {
            captured.push(FaceSpec {
                outer: positions(&f.getattr("vertices")?)?,
                holes: f
                    .getattr("holes")?
                    .try_iter()?
                    .map(|r| positions(&r?))
                    .collect::<PyResult<_>>()?,
                attrs: py
                    .get_type::<PyDict>()
                    .call1((f.getattr("attrs")?,))?
                    .unbind(),
            });
            for r in crate::object_weld::rings(&f)? {
                let n = r.len()?;
                for i in 0..n {
                    let a = r.get_item(i)?;
                    let b = r.get_item((i + 1) % n)?;
                    for e in a.getattr("edges")?.try_iter()? {
                        let e = e?;
                        if e.call_method1("other", (&a,))?.eq(&b)? {
                            note(&e)?;
                            break;
                        }
                    }
                }
            }
        }
        for e in edges.try_iter()? {
            note(&e?)?;
        }
        Ok(Self {
            faces: captured,
            edges: loose,
            flags,
        })
    }

    fn build<'py>(&self, scene: &Obj<'py>, component: bool, name: &Obj<'py>) -> PyResult<Obj<'py>> {
        let py = scene.py();
        let facade = py.import("core.group")?;
        let mesh_api = py.import("ingetrazo_core.mesh")?;
        let frame = scene.getattr("drawing_frame").ok();
        let mut origin = DVec3::ZERO;
        let mut axes = None;
        let mut corner = DVec3::ZERO;
        let points = self
            .faces
            .iter()
            .flat_map(|f| std::iter::once(&f.outer).chain(&f.holes).flatten())
            .chain(self.edges.iter().flatten());
        if let Some(first) = points.clone().next() {
            if let Some(frame) = frame.filter(|f| !f.is_none()) {
                let basis = facade.call_method1("frame_axes", (frame,))?;
                let at = xyz(&basis.get_item(0)?)?;
                let dirs = [
                    xyz(&basis.get_item(1)?)?,
                    xyz(&basis.get_item(2)?)?,
                    xyz(&basis.get_item(3)?)?,
                ];
                let lo = points
                    .clone()
                    .map(|p| DVec3::from_array(dirs.map(|v| (*p - at).dot(v))))
                    .fold(DVec3::splat(f64::INFINITY), DVec3::min);
                corner = at + dirs[0] * lo.x + dirs[1] * lo.y + dirs[2] * lo.z;
                axes = Some(dirs);
            }
            if component {
                origin = if axes.is_some() {
                    corner
                } else {
                    points.copied().fold(*first, DVec3::min)
                };
            }
        }
        let local = |p: DVec3| {
            let d = p - origin;
            if let Some(dirs) = axes.filter(|_| component) {
                DVec3::from_array(dirs.map(|v| d.dot(v)))
            } else {
                d
            }
        };
        let mesh = mesh_api.getattr("Mesh")?.call0()?;
        let ring = |r: &Ring| r.iter().map(|&p| Vec3::from(local(p))).collect::<Vec<_>>();
        for f in &self.faces {
            let holes = (!f.holes.is_empty()).then(|| f.holes.iter().map(ring).collect::<Vec<_>>());
            let nf = mesh.call_method1("add_face", (ring(&f.outer), holes))?;
            if f.attrs.bind(py).is_truthy()? {
                nf.getattr("attrs")?
                    .call_method1("update", (f.attrs.bind(py),))?;
            }
        }
        for [a, b] in &self.edges {
            let (a, b) = (local(*a), local(*b));
            if key(a) != key(b) {
                mesh.call_method1("add_edge", (Vec3::from(a), Vec3::from(b)))?;
            }
        }
        let stamp = mesh_api.getattr("stamp_edge_flags")?;
        for ([a, b], flags) in &self.flags {
            let a = mesh.call_method1("vertex_at", (Vec3::from(local(*a)),))?;
            let b = mesh.call_method1("vertex_at", (Vec3::from(local(*b)),))?;
            if !a.is_none() && !b.is_none() {
                let e = mesh.call_method1("find_edge", (a, b))?;
                if !e.is_none() {
                    stamp.call1((e, flags.bind(py)))?;
                }
            }
        }
        let group = facade.getattr("Group")?.call1((&mesh, name))?;
        if let Some([x, y, z]) = axes {
            let m = py.import("ingetrazo_core.axes")?.call_method1(
                "frame_matrix",
                (
                    Vec3::from(corner),
                    Vec3::from(x),
                    Vec3::from(y),
                    Vec3::from(z),
                ),
            )?;
            group.setattr(if component { "xform" } else { "axes" }, m)?;
        } else if component {
            let matrix = py.get_type::<crate::math::Matrix4>().call0()?;
            matrix.call_method1("translate", (Vec3::from(origin),))?;
            group.setattr("xform", matrix)?;
        }
        let owner = scene
            .getattr("__dict__")?
            .call_method1("get", ("_groups_owner",))?;
        if !owner.is_none() {
            let parent = scene.getattr("mesh")?;
            if parent
                .getattr("_instance_definition")
                .is_ok_and(|h| !h.is_none())
            {
                let inverse = parent
                    .getattr("_store")?
                    .call_method0("tool_placement")?
                    .call_method0("inverted")?
                    .get_item(0)?;
                let m = group.getattr("xform")?;
                if m.is_none() {
                    group.setattr(
                        "mesh",
                        facade.call_method1("transformed_mesh", (&mesh, &inverse))?,
                    )?;
                    facade.call_method1("carry_axes", (&group, inverse))?;
                } else {
                    group.setattr("xform", inverse.mul(m)?)?;
                }
            }
        }
        Ok(group)
    }

    fn take_off(&self, mesh: &Obj<'_>, discard: &Obj<'_>) -> PyResult<()> {
        // Capture list order before removing anything: deletion retires keys
        // and can materialise a store facade midway through a transaction.

        let signature = |r: &Ring| r.iter().copied().map(key).collect::<BTreeSet<_>>();
        let faces: HashSet<_> = self.faces.iter().map(|f| signature(&f.outer)).collect();
        let grouped: HashSet<_> = self
            .faces
            .iter()
            .flat_map(|f| {
                std::iter::once(&f.outer)
                    .chain(&f.holes)
                    .flatten()
                    .copied()
                    .map(key)
            })
            .collect();
        let edge_key = |pair: [DVec3; 2]| {
            let mut k = pair.map(key);
            k.sort();
            k
        };
        let edges: HashSet<_> = self.edges.iter().copied().map(edge_key).collect();
        for f in rows(&mesh.getattr("faces")?)? {
            if faces.contains(&signature(&positions(&f.getattr("vertices")?)?)) {
                mesh.call_method1("remove_face", (&f,))?;
                discard.call1((f,))?;
            }
        }
        for e in rows(&mesh.getattr("edges")?)? {
            if !e.getattr("faces")?.is_empty()? {
                continue;
            }
            let k = edge_key(ends(&e)?);
            if edges.contains(&k) || (grouped.contains(&k[0]) && grouped.contains(&k[1])) {
                mesh.call_method1("remove_edge", (&e,))?;
                discard.call1((e,))?;
            }
        }
        mesh.call_method0("purge_debris")?;
        Ok(())
    }
}

fn copy_tree<'py>(api: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let copy = api.call_method1("copy_group", (group,))?;
    copy.setattr("uid", group.getattr("uid")?)?;
    if api.call_method1("_is_link", (group,))?.is_truthy()? {
        let children = crate::model::links::copy_parts(group, &copy)?;
        copy.setattr("children", children)?;
    } else {
        if !group.getattr("xform")?.is_none() {
            copy.setattr(
                "mesh",
                api.call_method1("_copy_definition_mesh", (group.getattr("mesh")?,))?,
            )?;
        }
        copy.setattr("children", copy_children(api, group)?)?;
    }
    Ok(copy)
}

fn copy_children<'py>(api: &Obj<'py>, group: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let children = group.getattr("children")?;
    if children.is_none() {
        return Ok(Vec::new());
    }
    children.try_iter()?.map(|c| copy_tree(api, &c?)).collect()
}

#[pyfunction]
pub(crate) fn unique_placement(group: &Obj<'_>) -> PyResult<()> {
    let api = group.py().import("core.group")?.into_any();
    if api.call_method1("_is_link", (group,))?.is_truthy()? {
        return Ok(());
    }
    let component = group.call_method0("is_component")?.is_truthy()?;
    if !component && !group.getattr("children")?.is_truthy()? {
        group.call_method0("materialize")?;
    } else {
        group.setattr(
            "mesh",
            api.call_method1("_copy_definition_mesh", (group.getattr("mesh")?,))?,
        )?;
        group.setattr("children", copy_children(&api, group)?)?;
        if !component {
            group.setattr("component", false)?;
        }
    }
    Ok(())
}

// Capture only for direct or reference-history callers. Normal replay reads
// the definition handles and placement fields from the document transaction.
const UNIQUE_FIELDS: [&str; 6] = ["mesh", "xform", "children", "axes", "component", "name"];
#[pyfunction]
pub(crate) fn make_placement_unique<'py>(
    scene: &Obj<'py>,
    group: &Obj<'py>,
    rename: bool,
    capture: bool,
) -> PyResult<Option<Vec<Obj<'py>>>> {
    let ext = group.getattr("ext")?;
    if ext.is_truthy()? && ext.call_method1("get", ("link",))?.is_truthy()? {
        return Ok(None);
    }
    let prior = if capture {
        UNIQUE_FIELDS
            .iter()
            .map(|&field| group.getattr(field))
            .collect::<PyResult<_>>()?
    } else {
        Vec::new()
    };
    if rename {
        let name = group.getattr("name")?;
        if name.is_truthy()? {
            let api = group.py().import("core.kept_components")?;
            let kwargs = PyDict::new(group.py());
            kwargs.set_item("skip", group)?;
            let taken = api.getattr("names_in_use")?.call((scene,), Some(&kwargs))?;
            if taken.contains(&name)? {
                group.setattr("name", api.call_method1("numbered_name", (name, taken))?)?;
            }
        }
    }
    group.call_method0("make_unique")?;
    Ok(Some(prior))
}
#[pyfunction]
pub(crate) fn restore_unique_placement(group: &Obj<'_>, prior: Vec<Obj<'_>>) -> PyResult<()> {
    for (field, value) in UNIQUE_FIELDS.into_iter().zip(prior) {
        if field == "children" && !value.is_truthy()? {
            group.setattr(field, pyo3::types::PyList::empty(group.py()))?;
        } else {
            group.setattr(field, value)?;
        }
    }
    Ok(())
}

fn paint_copy<'py>(paint: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(paint.py());
    for pair in paint.call_method0("items")?.try_iter()? {
        let pair = pair?;
        let value = pair.get_item(1)?;
        let value = if let Ok(dict) = value.cast::<PyDict>() {
            dict.copy()?.into_any()
        } else {
            value
        };
        result.set_item(pair.get_item(0)?, value)?;
    }
    Ok(result)
}

type Lifted<'py> = (Obj<'py>, Obj<'py>, Obj<'py>, Obj<'py>, Obj<'py>, Obj<'py>);

#[pyfunction]
pub(crate) fn lift_placements<'py>(
    groups: &Obj<'py>,
    group: &Obj<'py>,
    index: isize,
) -> PyResult<Vec<Lifted<'py>>> {
    use pyo3::types::{PyList, PySlice};
    let py = groups.py();
    let frame = group.getattr("xform")?;
    let paint = group.getattr("material")?;
    let children = group.getattr("children")?;
    let children = if children.is_none() {
        Vec::new()
    } else {
        rows(&children)?
    };
    let api = py.import("core.group")?;
    let mut lifted = Vec::with_capacity(children.len());
    for c in &children {
        lifted.push((
            c.clone(),
            c.getattr("xform")?,
            c.getattr("mesh")?,
            c.getattr("material")?,
            c.getattr("axes")?,
            c.getattr("explode_offset")?,
        ));
    }
    for (child, xform, mesh, material, _, _) in &lifted {
        child.setattr("explode_offset", py.None())?;
        if !frame.is_none() {
            if xform.is_none() {
                child.setattr(
                    "mesh",
                    api.call_method1("transformed_mesh", (mesh, &frame))?,
                )?;
                api.call_method1("carry_axes", (child, &frame))?;
            } else {
                child.setattr("xform", frame.mul(xform)?)?;
            }
        }
        if material.is_none() && paint.is_truthy()? {
            child.setattr("material", paint_copy(&paint)?)?;
        }
    }
    groups.call_method1("remove", (group,))?;
    group.setattr("children", PyList::empty(py))?;
    groups.set_item(PySlice::new(py, index, index, 1), children)?;
    Ok(lifted)
}

#[pyfunction]
pub(crate) fn lower_placements(groups: &Obj<'_>, lifted: Vec<Lifted<'_>>) -> PyResult<()> {
    for (child, xform, mesh, material, axes, offset) in lifted {
        if groups.contains(&child)? {
            groups.call_method1("remove", (&child,))?;
        }
        for (field, value) in [
            ("xform", xform),
            ("mesh", mesh),
            ("material", material),
            ("axes", axes),
            ("explode_offset", offset),
        ] {
            child.setattr(field, value)?;
        }
    }
    Ok(())
}

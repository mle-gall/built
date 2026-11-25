// SPDX-License-Identifier: GPL-3.0-or-later
//! Select the participating faces and their modelling contexts. The same
//! polygon clipper is used for resident faces and transformed instances.
use super::intersect::{PyPolygon, Segment, WorldRings};
use crate::{
    boxes::touching,
    math::{xyz, Vec3},
};
use glam::DVec3;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PySet},
    IntoPyObjectExt,
};
use std::collections::HashSet;
type Obj<'p> = Bound<'p, PyAny>;
type Polys<'p> = Vec<Bound<'p, PyPolygon>>;
fn coords(face: &Obj, xf: Option<&Obj>) -> PyResult<WorldRings> {
    let mut rings = vec![face.getattr("vertices")?];
    rings.extend(
        face.getattr("holes")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?,
    );
    rings
        .into_iter()
        .map(|r| {
            r.try_iter()?
                .map(|p| {
                    let p = p?;
                    let p = match xf {
                        Some(x) if !x.is_none() => x.call_method1("map", (p,))?,
                        _ => p,
                    };
                    Ok(xyz(&p)?.to_array())
                })
                .collect()
        })
        .collect()
}
fn h(py: Python) -> PyResult<String> {
    py.import("core.store_mesh")?
        .call_method0("_hypot")?
        .extract()
}
fn pysegs(s: Vec<Segment>) -> Vec<(Vec3, Vec3)> {
    s.into_iter()
        .map(|(a, b)| (DVec3::from_array(a).into(), DVec3::from_array(b).into()))
        .collect()
}
#[pyfunction(signature=(face,xform=None))]
pub(super) fn world_polys<'p>(
    face: &Obj<'p>,
    xform: Option<&Obj>,
) -> PyResult<Option<Bound<'p, PyPolygon>>> {
    let py = face.py();
    PyPolygon::from_rings(coords(face, xform)?, &h(py)?)?
        .map(|p| Bound::new(py, p))
        .transpose()
}
#[pyfunction]
fn cross(a: &PyPolygon, b: &PyPolygon) -> Vec<(Vec3, Vec3)> {
    pysegs(a.cross(b))
}
fn unique(segs: &mut Vec<Segment>) {
    let mut seen = HashSet::new();
    segs.retain(|&(a, b)| {
        let key = |p: [f64; 3]| p.map(|v| (v / 1e-6).round_ties_even() as i128);
        let (a, b) = (key(a), key(b));
        seen.insert(if a < b { (a, b) } else { (b, a) })
    });
}
fn hits(a: &Polys, b: &Polys, same: bool) -> Vec<Segment> {
    if a.is_empty() || b.is_empty() {
        return vec![];
    }
    let boxes = |seq: &Polys| seq.iter().map(|p| p.borrow().bounds()).collect::<Vec<_>>();
    let aa = boxes(a);
    let bb = if same { vec![] } else { boxes(b) };
    let bb = if same { &aa } else { &bb };
    let (ii, jj) = touching(&aa, bb, same, 1e-6);
    let mut out = vec![];
    for (i, j) in ii.into_iter().zip(jj) {
        let p = &a[i as usize];
        let q = &b[j as usize];
        if !p.is(q) {
            out.extend(p.borrow().cross(&q.borrow()));
        }
    }
    unique(&mut out);
    out
}
fn placements<'p>(group: &Obj<'p>) -> PyResult<Vec<(Obj<'p>, Obj<'p>)>> {
    group
        .py()
        .import("core.group")?
        .call_method1("iter_placements", (group,))?
        .try_iter()?
        .map(|r| r?.extract())
        .collect()
}
fn placed_faces<'p>(group: &Obj<'p>, xf: &Obj) -> PyResult<Polys<'p>> {
    let mut out = vec![];
    for f in group.getattr("mesh")?.getattr("faces")?.try_iter()? {
        if let Some(p) = world_polys(&f?, Some(xf))? {
            out.push(p);
        }
    }
    Ok(out)
}
fn polys<'p>(entities: &[Obj<'p>], group_class: &Obj) -> PyResult<Polys<'p>> {
    let mut out = vec![];
    for e in entities {
        if e.is_instance(group_class)? {
            for (g, xf) in placements(e)? {
                out.extend(placed_faces(&g, &xf)?);
            }
        } else if let Some(p) = world_polys(e, None)? {
            out.push(p);
        }
    }
    Ok(out)
}
fn placed_rings(group: &Obj) -> PyResult<Vec<WorldRings>> {
    let mut out = vec![];
    for (p, xf) in placements(group)? {
        for f in p.getattr("mesh")?.getattr("faces")?.try_iter()? {
            out.push(coords(&f?, Some(&xf))?);
        }
    }
    Ok(out)
}
#[pyfunction(signature=(scene,mode,selection=None))]
fn segments_for(scene: &Obj, mode: &str, selection: Option<&Obj>) -> PyResult<Vec<(Vec3, Vec3)>> {
    let py = scene.py();
    let mesh = scene.getattr("mesh")?;
    let group = py.import("core.group")?.getattr("Group")?;
    let face = py.import("core.mesh")?.getattr("Face")?;
    let entities = match selection {
        Some(v) => v.clone(),
        None => scene.getattr("selection")?,
    };
    let mut sel = vec![];
    let selected = PySet::empty(py)?;
    for e in entities.try_iter()? {
        let e = e?;
        if e.is_instance(&group)? || e.is_instance(&face)? {
            selected.add(&e)?;
            sel.push(e);
        }
    }
    if sel.is_empty() {
        return Ok(vec![]);
    }
    let mut others = vec![];
    if mode == "model" {
        let owner = scene.getattr("edit_group")?;
        let kids = if owner.is_none() {
            scene.getattr("groups")?
        } else {
            owner.getattr("children")?
        };
        for g in kids.try_iter()? {
            let g = g?;
            if !selected.contains(&g)? {
                others.push(g);
            }
        }
    }
    if mesh.hasattr("_store")? {
        let mut keys = Vec::<u128>::new();
        let mut placed = vec![];
        for e in &sel {
            if e.is_instance(&group)? {
                for rings in placed_rings(e)? {
                    placed.push((keys.len(), rings));
                }
            } else {
                keys.push(e.getattr("key")?.extract()?);
            }
        }
        let mut world = vec![];
        for e in others {
            world.extend(placed_rings(&e)?);
        }
        let segs = mesh
            .getattr("_store")?
            .call_method1("intersection_segments", (keys, mode, h(py)?, placed, world))?
            .extract()?;
        return Ok(pysegs(segs));
    }
    let mine = polys(&sel, &group)?;
    if mode == "selection" {
        return Ok(pysegs(hits(&mine, &mine, true)));
    }
    let mut rest = vec![];
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        if !selected.contains(&f)? {
            rest.push(f);
        }
    }
    rest.extend(others);
    let mut out = hits(&mine, &polys(&rest, &group)?, false);
    if mode == "model" {
        out.extend(hits(&mine, &mine, true));
        unique(&mut out);
    }
    Ok(pysegs(out))
}
#[pyclass(frozen)]
struct Mutation {
    mesh: Py<PyAny>,
    segments: Py<PyAny>,
}
#[pymethods]
impl Mutation {
    fn __call__(&self, scene: &Obj) -> PyResult<()> {
        let py = scene.py();
        py.import("core.intersect")?.call_method1(
            "_partition_faces",
            (self.mesh.bind(py), self.segments.bind(py)),
        )?;
        Ok(())
    }
}
fn mutation<'p>(mesh: &Obj<'p>, segments: &Obj) -> PyResult<Obj<'p>> {
    let py = mesh.py();
    let (module, ty) = if mesh.hasattr("_store")? {
        ("ingetrazo_core", "resident_face_mutation")
    } else {
        ("core.history", "SnapshotMutation")
    };
    let kw = PyDict::new(py);
    kw.set_item("mesh", mesh)?;
    py.import(module)?.getattr(ty)?.call(
        (Mutation {
            mesh: mesh.clone().unbind(),
            segments: segments.clone().unbind(),
        },),
        Some(&kw),
    )
}
#[pyfunction]
fn build_intersection_edit<'p>(scene: &Obj<'p>, segments: &Obj) -> PyResult<Obj<'p>> {
    mutation(&scene.getattr("mesh")?, segments)
}
#[pyfunction]
fn placement_do(cmd: &Obj, scene: &Obj) -> PyResult<()> {
    let steps = cmd
        .getattr("commands")?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?;
    for (i, c) in steps.iter().enumerate() {
        if let Err(e) = c.call_method1("do", (scene,)) {
            if !cmd.getattr("_journal_geometry")?.is_truthy()? {
                for step in steps[..=i].iter().rev() {
                    step.call_method1("undo", (scene,))?;
                }
            }
            return Err(e);
        }
    }
    Ok(())
}
fn length_floor(py: Python) -> PyResult<f64> {
    Ok(1.
        / py.import("core.mesh")?
            .getattr("_KEY_SCALE")?
            .extract::<f64>()?)
}
fn placement_cuts<'p>(py: Python<'p>, groups: &[Obj<'p>]) -> PyResult<(usize, Option<Obj<'p>>)> {
    let mut owners = vec![];
    for root in groups {
        for (g, xf) in placements(root)? {
            let inv = if xf.is_none() {
                None
            } else {
                let (inv, ok): (Obj, bool) = xf.call_method0("inverted")?.extract()?;
                if !ok {
                    return Err(PyValueError::new_err("singular placement transform"));
                }
                Some(inv)
            };
            owners.push((g.getattr("mesh")?, inv, placed_faces(&g, &xf)?));
        }
    }
    let mut world = vec![];
    let floor = length_floor(py)?;
    let mut by_mesh = std::collections::HashMap::new();
    let mut cuts = Vec::<(Obj, Vec<Segment>)>::new();
    for (i, left) in owners.iter().enumerate() {
        for right in &owners[i + 1..] {
            let ss = hits(&left.2, &right.2, false);
            world.extend(&ss);
            for (mesh, inv, _) in [left, right] {
                let k = *by_mesh.entry(mesh.as_ptr()).or_insert_with(|| {
                    cuts.push((mesh.clone(), vec![]));
                    cuts.len() - 1
                });
                let local = |p: [f64; 3]| -> PyResult<DVec3> {
                    let p = DVec3::from(p);
                    match inv {
                        None => Ok(p),
                        Some(x) => xyz(&x.call_method1("map", (Vec3::from(p),))?),
                    }
                };
                for &(a, b) in &ss {
                    let (a, b) = (local(a)?, local(b)?);
                    if a.distance(b) >= floor {
                        cuts[k].1.push((a.to_array(), b.to_array()));
                    }
                }
            }
        }
    }
    unique(&mut world);
    let commands = PyList::empty(py);
    for (m, mut ss) in cuts {
        if !ss.is_empty() {
            unique(&mut ss);
            commands.append(mutation(&m, &pysegs(ss).into_bound_py_any(py)?)?)?;
        }
    }
    let cmd = if commands.is_empty() {
        None
    } else {
        Some(
            py.import("core.intersect")?
                .call_method1("_PlacementCuts", (commands,))?,
        )
    };
    Ok((world.len(), cmd))
}
#[pyfunction(signature=(scene,mode,face_keys=None,placements=None))]
fn build_intersect_faces<'p>(
    scene: &Obj<'p>,
    mode: &str,
    face_keys: Option<&Obj<'p>>,
    placements: Option<&Obj<'p>>,
) -> PyResult<(usize, Option<Obj<'p>>)> {
    let py = scene.py();
    let sel = match placements {
        Some(s) => s.clone(),
        None => scene.getattr("selection")?,
    };
    let sel = sel.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    let group = py.import("core.group")?.getattr("Group")?;
    let mut groups = true;
    for e in &sel {
        groups &= e.is_instance(&group)?;
    }
    if mode == "placements"
        || mode == "selection" && face_keys.is_none() && !sel.is_empty() && groups
    {
        if face_keys.is_some() || !groups {
            return Err(PyValueError::new_err(
                "placements mode requires groups or components",
            ));
        }
        return placement_cuts(py, &sel);
    }
    let mesh = scene.getattr("mesh")?;
    let api = py.import("core.intersect")?;
    let segs = if let Some(keys) = face_keys {
        let found = py
            .import("core.protocol.keys")?
            .call_method1("elements", (&mesh, "face", keys))?;
        let faces = keys
            .try_iter()?
            .map(|k| found.get_item(k?))
            .collect::<PyResult<Vec<_>>>()?;
        api.call_method1("segments_for", (scene, mode, faces))?
    } else {
        api.call_method1("segments_for", (scene, mode))?
    };
    let saved = PyList::empty(py);
    let floor = length_floor(py)?;
    for row in segs.try_iter()? {
        let (a, b): (Obj, Obj) = row?.extract()?;
        if xyz(&a)?.distance(xyz(&b)?) >= floor {
            saved.append((a, b))?;
        }
    }
    Ok((
        saved.len(),
        if saved.is_empty() {
            None
        } else {
            Some(mutation(&mesh, saved.as_any())?)
        },
    ))
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let ns = PyDict::new(py);
    ns.set_item("__module__", "core.intersect")?;
    ns.set_item("journal_parts", true)?;
    ns.set_item(
        "do",
        py.import("functools")?
            .call_method1("partialmethod", (wrap_pyfunction!(placement_do, py)?,))?,
    )?;
    let base = py.import("core.history")?.getattr("CompoundCommand")?;
    m.add(
        "_PlacementCuts",
        py.import("builtins")?
            .call_method1("type", ("_PlacementCuts", (base,), ns))?,
    )?;
    for (name, val) in [
        ("WITH_SELECTION", "selection"),
        ("WITH_MODEL", "model"),
        ("WITH_CONTEXT", "context"),
    ] {
        m.add(name, val)?;
    }
    for f in [
        wrap_pyfunction!(world_polys, m)?,
        wrap_pyfunction!(cross, m)?,
        wrap_pyfunction!(segments_for, m)?,
        wrap_pyfunction!(build_intersection_edit, m)?,
        wrap_pyfunction!(build_intersect_faces, m)?,
        wrap_pyfunction!(super::intersection_edit::_partition_faces, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}

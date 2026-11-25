// SPDX-License-Identifier: GPL-3.0-or-later
//! Route indexed store views and object meshes through the same piece finder.
//! The transient object topology is keyed by vertex identity, never re-welded.
use super::{edit::Change, keys::Kind, Store, Weld};
use crate::math::xyz;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySlice},
    IntoPyObjectExt,
};
use std::collections::{HashMap, HashSet};
type Obj<'p> = Bound<'p, PyAny>;
fn items<'p>(v: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    v.try_iter()?.collect()
}
struct Topology {
    store: Store,
    vertices: HashMap<u128, u32>,
    change: Change,
}
impl Topology {
    fn read(faces: &[Obj], edges: &[Obj]) -> PyResult<(Self, Vec<u32>)> {
        let mut t = Self {
            store: Store::empty(0, Weld::default()),
            vertices: HashMap::new(),
            change: Change::default(),
        };
        let mut edge_slots = vec![];
        for e in edges {
            let a = t.vertex(&e.getattr("v0")?)?;
            let b = t.vertex(&e.getattr("v1")?)?;
            edge_slots.push(
                t.store
                    .link_edge(a, b, &mut t.change)
                    .map_err(super::py::op_err)?,
            );
        }
        for f in faces {
            let mut face = vec![];
            let mut loops = vec![f.getattr("loop")?];
            loops.extend(items(&f.getattr("hole_loops")?)?);
            for ring in loops {
                face.push(
                    ring.try_iter()?
                        .map(|v| t.vertex(&v?))
                        .collect::<PyResult<_>>()?,
                );
            }
            t.store
                .add_welded_face(&face, &mut t.change)
                .map_err(super::py::op_err)?;
        }
        Ok((t, edge_slots))
    }
    fn vertex(&mut self, v: &Obj) -> PyResult<u32> {
        let k = v.getattr("key")?.extract()?;
        if let Some(s) = self.vertices.get(&k) {
            return Ok(*s);
        }
        let s = self
            .store
            .new_slot(Kind::Vertex)
            .map_err(super::py::op_err)?;
        self.store
            .state
            .vpos
            .set(s as usize, xyz(&v.getattr("position")?)?.to_array());
        self.vertices.insert(k, s);
        Ok(s)
    }
}
fn parts<'p>(mesh: &Obj<'p>, solid: bool) -> PyResult<Obj<'p>> {
    let py = mesh.py();
    if let Some(found) = crate::kernel::store_views::pieces::mesh_parts(mesh, solid)? {
        return Ok(found.into_any());
    }
    let faces = items(&mesh.getattr("faces")?)?;
    let edges = items(&mesh.getattr("edges")?)?;
    let (top, slots) = Topology::read(&faces, &edges)?;
    let parts = if solid {
        let h = py
            .import("core._native")?
            .call_method0("qt_hypot")?
            .extract::<String>()?;
        top.store.solid_parts(crate::hypot_of(&h)?)
    } else {
        top.store.connected_parts()
    };
    let mut originals = HashMap::new();
    for (i, slot) in slots.into_iter().enumerate() {
        originals
            .entry(slot)
            .or_insert_with(Vec::new)
            .push(edges[i].clone());
    }
    let out = PyList::empty(py);
    for p in parts {
        out.append((
            p.faces
                .iter()
                .map(|i| faces[*i as usize].clone())
                .collect::<Vec<_>>(),
            p.edges
                .into_iter()
                .filter_map(|slot| originals.get(&slot))
                .flatten()
                .collect::<Vec<_>>(),
        ))?;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn connected_parts<'p>(mesh: &Obj<'p>) -> PyResult<Obj<'p>> {
    parts(mesh, false)
}
#[pyfunction]
fn solid_parts<'p>(mesh: &Obj<'p>) -> PyResult<Obj<'p>> {
    parts(mesh, true)
}
#[pyfunction]
fn _is_solid(faces: &Obj) -> PyResult<bool> {
    let fs = items(faces)?;
    let (t, _) = Topology::read(&fs, &[])?;
    Ok(t.store.is_solid(&(0..fs.len() as u32).collect::<Vec<_>>()))
}
#[pyfunction]
fn copy_part<'p>(faces: &Obj<'p>, edges: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = faces.py();
    if let Some(made) = crate::kernel::store_views::pieces::copy_mesh_part(faces, edges)? {
        return Ok(made);
    }
    let api = py.import("built_core.mesh")?;
    let mesh = api.call_method0("Mesh")?;
    let mut flat = Vec::new();
    let edges = items(edges)?;
    let mut flags = vec![];
    for e in &edges {
        for key in ["a", "b"] {
            flat.push(xyz(&e.getattr(key)?)?.to_array());
        }
        flags.push((e.getattr("soft")?, e.getattr("curve")?, e.getattr("layer")?));
    }
    let ends = flat.len();
    let mut sizes = vec![];
    let mut counts = vec![];
    let mut attrs = vec![];
    for f in faces.try_iter()? {
        let f = f?;
        let mut holes = f.getattr("holes")?;
        if holes.len()? > 1 {
            holes = py
                .import("built_core.topology")?
                .call_method1("_maximal_holes", (holes,))?;
        }
        counts.push(1 + holes.len()?);
        let mut rs = vec![f.getattr("vertices")?];
        rs.extend(items(&holes)?);
        for r in rs {
            sizes.push(r.len()?);
            for p in r.try_iter()? {
                flat.push(xyz(&p?)?.to_array());
            }
        }
        let a = f.getattr("attrs")?;
        attrs.push(if a.is_truthy()? {
            Some(py.get_type::<PyDict>().call1((a,))?)
        } else {
            None
        });
    }
    if flat.is_empty() {
        return Ok(mesh);
    }
    let pos = py
        .import("numpy")?
        .call_method1("asarray", (flat, "float64"))?;
    let (vs, inverse): (Obj, Obj) = mesh.call_method1("bulk_weld", (pos,))?.extract()?;
    let mut emap = None;
    if !edges.is_empty() {
        let a = inverse.get_item(PySlice::new(py, 0, ends as isize, 2))?;
        let b = inverse.get_item(PySlice::new(py, 1, ends as isize, 2))?;
        let pairs = mesh.call_method1("add_edges_welded", (&vs, &a, &b, flags))?;
        let n = vs.len()?;
        for ((e, i), j) in edges
            .iter()
            .zip(a.extract::<Vec<usize>>()?)
            .zip(b.extract::<Vec<usize>>()?)
        {
            let new = pairs.call_method1("get", (i.min(j) * n + i.max(j),))?;
            if !new.is_none() {
                api.call_method1(
                    "stamp_edge_flags",
                    (new, api.call_method1("edge_flags", (e,))?),
                )?;
            }
        }
        emap = Some(pairs);
    }
    if !counts.is_empty() {
        let kw = PyDict::new(py);
        kw.set_item("edge_map", emap)?;
        let ids = inverse.get_item(PySlice::new(py, ends as isize, inverse.len()? as isize, 1))?;
        mesh.call_method(
            "add_faces_welded",
            (vs, ids, sizes, counts, attrs),
            Some(&kw),
        )?;
    }
    mesh.call_method0("resplit_curves")?;
    Ok(mesh)
}
fn name_key(p: [f64; 3]) -> [u64; 3] {
    super::commands::rounded_cell::<5>(p)
}
fn generic(name: &str) -> bool {
    let Some((base, i)) = name.rsplit_once(' ') else {
        return false;
    };
    !i.is_empty()
        && i.chars().all(|x| x.is_numeric())
        && matches!(
            base.to_lowercase().as_str(),
            "piece" | "part" | "pieza" | "parte"
        )
}
fn inherited(children: &[Obj], parts: &[(Obj, Obj)]) -> PyResult<Vec<Option<String>>> {
    if children.is_empty() {
        return Ok(vec![None; parts.len()]);
    }
    let points = children[0]
        .py()
        .import("built_core.parts")?
        .getattr("part_points")?;
    let mut owners = HashMap::<_, HashSet<usize>>::new();
    for (i, g) in children.iter().enumerate() {
        let coords = points
            .call1((g,))?
            .call_method0("tolist")?
            .extract::<Vec<[f64; 3]>>()?;
        for p in coords {
            owners.entry(name_key(p)).or_default().insert(i);
        }
    }
    let mut result = vec![];
    let mut taken = HashMap::<String, usize>::new();
    for (fs, _) in parts {
        let mut possible: Option<HashSet<usize>> = None;
        'faces: for f in fs.try_iter()? {
            for v in f?.getattr("loop")?.try_iter()? {
                let p = name_key(xyz(&v?.getattr("position")?)?.to_array());
                let found = owners.get(&p);
                if let Some(ref mut ps) = possible {
                    ps.retain(|i| found.is_some_and(|f| f.contains(i)));
                } else {
                    possible = Some(found.cloned().unwrap_or_default());
                }
                if possible.as_ref().is_some_and(HashSet::is_empty) {
                    break 'faces;
                }
            }
        }
        let mut name = None;
        if let Some(ids) = possible.filter(|ids| ids.len() == 1) {
            let i = *ids.iter().next().unwrap();
            let full = children[i].getattr("name")?.extract::<String>()?;
            let base = full.trim();
            if !base.is_empty() && !generic(base) {
                let count = taken.entry(base.to_owned()).or_default();
                *count += 1;
                name = Some(if *count == 1 {
                    base.to_owned()
                } else {
                    format!("{base} {count}")
                });
            }
        }
        result.push(name);
    }
    Ok(result)
}
#[pyfunction]
fn split_into_pieces<'p>(group: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let py = group.py();
    let api = py.import("built_core.group")?;
    let mesh = group.getattr("mesh")?;
    let children = items(&group.getattr("children")?)?;
    let frame = api.call_method1("PlacementProxy", (&mesh,))?;
    frame.setattr("children", &children)?;
    let local = crate::model::module(api.py(), "built_core")?.call_method1("group_world_mesh", (frame,))?;
    let parts = solid_parts(&local)?.extract::<Vec<(Obj, Obj)>>()?;
    if parts.len() <= 1 && children.is_empty() {
        return Ok(vec![]);
    }
    if !mesh.getattr("faces")?.is_truthy()? && parts.len() == children.len() {
        let mut separate = true;
        for k in &children {
            if k.getattr("children")?.is_truthy()? || solid_parts(&k.getattr("mesh")?)?.len()? != 1
            {
                separate = false;
                break;
            }
        }
        if separate {
            return Ok(vec![]);
        }
    }
    let mut groups = vec![];
    let names = inherited(&children, &parts)?;
    for (i, ((fs, es), name)) in parts.into_iter().zip(names).enumerate() {
        let kw = PyDict::new(py);
        let name = match name {
            Some(name) => name.into_bound_py_any(py)?,
            None => {
                kw.set_item("n", i + 1)?;
                py.import("built_core.i18n")?
                    .call_method("tr", ("Piece {n}",), Some(&kw))?
            }
        };
        kw.clear();
        kw.set_item("name", name)?;
        let g = api
            .getattr("Group")?
            .call((copy_part(&fs, &es)?,), Some(&kw))?;
        g.setattr(
            "xform",
            py.import("built_core")?.call_method0("Matrix4")?,
        )?;
        g.setattr("component", false)?;
        groups.push(g);
    }
    Ok(groups)
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(connected_parts, m)?,
        wrap_pyfunction!(solid_parts, m)?,
        wrap_pyfunction!(copy_part, m)?,
        wrap_pyfunction!(split_into_pieces, m)?,
        wrap_pyfunction!(_is_solid, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}

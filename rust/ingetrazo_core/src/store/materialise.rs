// SPDX-License-Identifier: GPL-3.0-or-later
//! Populate the legacy mesh handles directly from the live and retired slots.
//! History can still hold a handle across several store/object transitions.
use super::{
    keys::{Kind, NONE},
    Store, FLAT, HIDDEN, INTERIOR, SOFT,
};
use crate::math::Vec3;
use glam::DVec3;
use pyo3::{
    intern,
    prelude::*,
    types::{PyDict, PyList, PySet},
};

type Obj<'py> = Bound<'py, PyAny>;

fn objects<'py>(
    s: &Store,
    kind: Kind,
    class: Obj<'py>,
    map: &Bound<'py, PyDict>,
) -> PyResult<Vec<Obj<'py>>> {
    let py = class.py();
    let new = class.getattr("__new__")?;
    (0..s.state.slots[kind as usize].len() as u32)
        .map(|slot| {
            let key = s.key(kind, slot);
            if let Some(obj) = map.get_item(key)? {
                return Ok(obj);
            }
            let obj = new.call1((&class,))?;
            obj.setattr(intern!(py, "key"), key)?;
            map.set_item(key, &obj)?;
            Ok(obj)
        })
        .collect()
}

pub(super) fn materialise(s: &Store, mesh: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    let api = py.import("core.store_mesh")?;
    let d = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    // A freshly copied definition has never issued object handles.
    let map = match d.get_item("_objmap")? {
        Some(map) => map.cast_into::<PyDict>()?,
        None => {
            let map = PyDict::new(py);
            d.set_item("_objmap", &map)?;
            map
        }
    };
    let own = mesh.getattr("_attr_own")?.cast_into::<PyDict>()?;
    let table = mesh.getattr("_attr_table")?;
    let copy = api.getattr("_copy_attrs")?;
    let verts = objects(s, Kind::Vertex, api.getattr("_MVertex")?, &map)?;
    let edges = objects(s, Kind::Edge, api.getattr("_MEdge")?, &map)?;
    let faces = objects(s, Kind::Face, api.getattr("_MFace")?, &map)?;
    let st = &s.state;
    let mut disks = Vec::with_capacity(verts.len());
    for (i, v) in verts.iter().enumerate() {
        v.setattr(
            intern!(py, "position"),
            Vec3::from(DVec3::from_array(*st.vpos.get(i))),
        )?;
        let disk = PySet::empty(py)?;
        v.setattr(intern!(py, "edges"), &disk)?;
        disks.push(disk);
    }
    for (i, f) in faces.iter().enumerate() {
        let mut rings = s
            .face_rings(i as u32)
            .into_iter()
            .map(|ring| PyList::new(py, ring.iter().map(|&v| &verts[v as usize])));
        let outer = rings
            .next()
            .transpose()?
            .unwrap_or_else(|| PyList::empty(py));
        f.setattr(intern!(py, "loop"), outer)?;
        f.setattr(
            intern!(py, "hole_loops"),
            PyList::new(py, rings.collect::<PyResult<Vec<_>>>()?)?,
        )?;
        let attrs = match own.get_item(s.key(Kind::Face, i as u32))? {
            Some(o) if !o.is_none() => o,
            _ => match st.fattr.get(i).checked_sub(1) {
                Some(row) => copy.call1((table.get_item(row)?,))?,
                None => PyDict::new(py).into_any(),
            },
        };
        f.setattr(intern!(py, "attrs"), attrs)?;
        f.setattr(intern!(py, "interior"), st.fflags.get(i) & INTERIOR != 0)?;
    }
    for (i, e) in edges.iter().enumerate() {
        let [a, b] = st.ends.get(i).map(|v| &verts[v as usize]);
        e.setattr(intern!(py, "v0"), a)?;
        e.setattr(intern!(py, "v1"), b)?;
        let flags = st.eflags.get(i);
        e.setattr(intern!(py, "soft"), flags & SOFT != 0)?;
        e.setattr(intern!(py, "hidden"), flags & HIDDEN != 0)?;
        e.setattr(intern!(py, "flat"), flags & FLAT != 0)?;
        let curve = *st.ecurve.get(i);
        e.setattr(intern!(py, "curve"), (curve >= 0).then_some(curve))?;
        let layer = *st.elayer.get(i);
        e.setattr(
            intern!(py, "layer"),
            (layer != NONE).then(|| &st.layers[layer as usize]),
        )?;
        e.setattr(
            intern!(py, "faces"),
            PyList::new(
                py,
                s.edge_faces(i as u32).iter().map(|&f| &faces[f as usize]),
            )?,
        )?;
    }
    let live_edges = s.live_slots(Kind::Edge);
    for &i in &live_edges {
        for &v in st.ends.get(i as usize) {
            disks[v as usize].add(&edges[i as usize])?;
        }
    }
    let registry = PyDict::new(py);
    let mut ordered: Vec<_> = st.registry.iter().collect();
    ordered.sort_unstable_by_key(|&(c, _)| *c);
    for (&c, &v) in ordered {
        registry.set_item((c.0, c.1, c.2), &verts[v as usize])?;
    }
    let displaced = PyDict::new(py);
    for (c, q) in st.displaced.iter() {
        displaced.set_item(
            (c.0, c.1, c.2),
            PyList::new(py, q.iter().map(|&v| &verts[v as usize]))?,
        )?;
    }
    for (name, slots, handles) in [
        ("vertices", s.live_slots(Kind::Vertex), &verts),
        ("edges", live_edges, &edges),
        ("faces", s.live_slots(Kind::Face), &faces),
    ] {
        d.set_item(
            name,
            PyList::new(py, slots.iter().map(|&i| &handles[i as usize]))?,
        )?;
    }
    d.set_item("_registry", registry)?;
    d.set_item("_displaced", displaced)?;
    d.set_item("_attr_own", PyDict::new(py))?;
    for name in [
        "_bulk",
        "_store",
        "_pos_cache",
        "_face_cache",
        "_attr_table",
    ] {
        if d.contains(name)? {
            d.del_item(name)?;
        }
    }
    mesh.setattr(intern!(py, "__class__"), api.getattr("MaterialisedMesh")?)
}

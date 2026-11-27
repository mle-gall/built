// SPDX-License-Identifier: GPL-3.0-or-later
//! Retain curved supports when a delivery selects a subset of a definition.
use super::*;
use crate::store::keys::Kind;
use std::collections::HashSet;

pub(crate) fn subset<'p>(
    mesh: &Bound<'p, PyAny>,
    keep: &Bound<'p, PyAny>,
) -> PyResult<Bound<'p, PyAny>> {
    let py = mesh.py();
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let held = store.try_borrow()?;
    let s = &held.inner;
    let mut faces = HashSet::new();
    let mut edges = HashSet::new();
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        if keep.call1((&f,))?.is_truthy()? {
            let id = s
                .slot(Kind::Face, f.getattr("key")?.extract()?)
                .ok_or_else(|| bad("stale delivery face"))?;
            faces.insert(id);
            edges.extend(
                s.boundary_edges(id)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|(e, _)| e),
            );
        }
    }
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        let id = s
            .slot(Kind::Edge, e.getattr("key")?.extract()?)
            .ok_or_else(|| bad("stale delivery edge"))?;
        if !edges.contains(&id) && keep.call1((&e,))?.is_truthy()? && s.edge_faces(id).is_empty() {
            return Err(bad("STEP scope contains an independent wire"));
        }
    }
    if faces.len() == s.n(Kind::Face) {
        return Ok(mesh.clone());
    }
    let out = py.import("built_core.mesh")?.call_method0("Mesh")?;
    if faces.is_empty() {
        return Ok(out);
    }
    if let Some(native) = &s.state.exact.native {
        let mut rows = native.rows.clone();
        let t = rows.topology.as_mut().unwrap();
        t.faces.retain(|f| faces.contains(&(f.id as u32)));
        t.edges.retain(|e| edges.contains(&(e.id as u32)));
        let mut vertices: HashSet<_> = t.edges.iter().flat_map(|e| e.vertices).collect();
        vertices.extend(
            t.faces
                .iter()
                .flat_map(|f| f.loops.iter().flatten().filter_map(|c| c.pole)),
        );
        t.vertices.retain(|k, _| vertices.contains(k));
        t.void_starts.clear();
        return super::import::native_mesh(py, rows);
    }
    let handle = out.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let mut borrow = handle.try_borrow_mut()?;
    let target = &mut borrow.inner;
    let mut vertices = HashMap::new();
    let mut change = crate::store::edit::Change::default();
    for f in s
        .live_slots(Kind::Face)
        .into_iter()
        .filter(|f| faces.contains(f))
    {
        let mut rings = s.face_rings(f);
        for v in rings.iter_mut().flatten() {
            let old = *v;
            let pos = s
                .state
                .frame
                .definition_point(*s.state.vpos.get(old as usize));
            let id = target
                .vertex(pos, &mut change)
                .map_err(|e| bad(format!("{e:?}")))?;
            vertices.insert(old, id);
            *v = id;
        }
        target
            .add_welded_face(&rings, &mut change)
            .map_err(|e| bad(format!("{e:?}")))?;
    }
    for e in edges {
        let [a, b] = s.state.ends.get(e as usize).map(|v| vertices[&v]);
        let new = target
            .find_edge(a, b)
            .ok_or_else(|| bad("missing scoped edge"))?;
        target
            .state
            .eflags
            .set(new as usize, *s.state.eflags.get(e as usize));
        target.set_edge_curve(new, *s.state.ecurve.get(e as usize));
        if let Some(arc) = s.state.exact.arcs.as_ref().and_then(|v| v.get(&e)) {
            let mut arc = arc.clone();
            if target.state.ends.get(new as usize)[0] != a {
                arc.interval.swap(0, 1);
            }
            target.bind_source_arc(new, arc).map_err(bad)?;
        }
    }
    target.finish(change);
    drop(borrow);
    out.call_method0("_bump")?;
    Ok(out)
}

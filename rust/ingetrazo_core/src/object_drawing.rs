// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered edits through materialised element handles. Mesh primitives own
//! incidence; edits only remove, reattach and query through those primitives.
use crate::math::{xyz, Vec3};
use crate::store::commands::{cell, Cell};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{HashMap, HashSet, VecDeque};
type Obj<'py> = Bound<'py, PyAny>;
type Segment = [Cell; 2];
fn rows<'py>(items: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    items.try_iter()?.collect()
}
fn cells(ring: &Obj<'_>) -> PyResult<Vec<Cell>> {
    ring.try_iter()?
        .map(|p| Ok(cell(xyz(&p?)?.to_array())))
        .collect()
}
pub(crate) fn outline_key(ring: &Obj<'_>) -> PyResult<Vec<Cell>> {
    let mut k = cells(ring)?;
    k.sort_unstable();
    k.dedup();
    Ok(k)
}
fn segment(a: Cell, b: Cell) -> Segment {
    if a < b {
        [a, b]
    } else {
        [b, a]
    }
}
fn segments(r: &[Cell]) -> impl Iterator<Item = Segment> + '_ {
    r.iter()
        .zip(r.iter().cycle().skip(1))
        .map(|(&a, &b)| segment(a, b))
}
fn find_edge<'py>(mesh: &Obj<'py>, ends: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let v = mesh.call_method1("vertex_at", (ends.get_item(0)?,))?;
    let w = mesh.call_method1("vertex_at", (ends.get_item(1)?,))?;
    if v.is_none() || w.is_none() {
        return Ok(None);
    }
    let e = mesh.call_method1("find_edge", (v, w))?;
    Ok((!e.is_none()).then_some(e))
}
fn discard(mesh: &Obj<'_>, selection: &Obj<'_>, method: &str, item: &Obj<'_>) -> PyResult<()> {
    mesh.call_method1(method, (item,))?;
    selection.call_method1("discard", (item,))?;
    Ok(())
}
#[pyfunction]
fn delete_edges<'py>(
    mesh: &Obj<'py>,
    ends: &Obj<'py>,
    cascade: bool,
    selection: &Obj<'py>,
) -> PyResult<(Vec<Obj<'py>>, Vec<Obj<'py>>)> {
    let mut edges = Vec::new();
    let mut keys = HashSet::new();
    for pair in ends.try_iter()? {
        let pair = pair?;
        if let Some(e) = find_edge(mesh, &pair)? {
            edges.push(e);
        }
        if cascade {
            let k = cells(&pair)?;
            keys.insert(segment(k[0], k[1]));
        }
    }
    let mut faces = Vec::new();
    if cascade {
        let api = mesh.py().import("core.history_commands")?;
        for f in api
            .call_method1("_faces_by_segments", (mesh, ends))?
            .try_iter()?
        {
            let f = f?;
            if segments(&cells(&f.getattr("vertices")?)?).any(|k| keys.contains(&k)) {
                faces.push(f);
            }
        }
        for f in &faces {
            discard(mesh, selection, "remove_face", f)?;
        }
    }
    for edge in &edges {
        discard(mesh, selection, "remove_edge", edge)?;
    }
    Ok((edges, faces))
}

#[pyfunction]
fn delete_face<'py>(
    mesh: &Obj<'py>,
    ring: &Obj<'py>,
    selection: &Obj<'py>,
) -> PyResult<Option<Obj<'py>>> {
    let target = outline_key(ring)?;
    let api = mesh.py().import("core.history_commands")?;
    for f in api
        .call_method1("_faces_keyed_like", (mesh, ring))?
        .try_iter()?
    {
        let f = f?;
        if outline_key(&f.getattr("vertices")?)? == target {
            discard(mesh, selection, "remove_face", &f)?;
            return Ok(Some(f));
        }
    }
    Ok(None)
}

fn split_holes<'py>(
    api: &Bound<'py, PyModule>,
    other: &Obj<'py>,
    pieces: &[Obj<'py>],
) -> PyResult<Option<Vec<Vec<Obj<'py>>>>> {
    let py = other.py();
    let mesh_types = py.import("core.mesh")?;
    let face = mesh_types.getattr("Face")?;
    let vertex = mesh_types.getattr("Vertex")?;
    let mut result = vec![Vec::new(); pieces.len()];
    for hole in other.getattr("holes")?.try_iter()? {
        let hole = hole?;
        let mut owner = None;
        for (i, piece) in pieces.iter().enumerate() {
            let vertices = piece
                .try_iter()?
                .map(|p| vertex.call1((p?,)))
                .collect::<PyResult<Vec<_>>>()?;
            if api
                .call_method1("loop_inside_face", (face.call1((vertices,))?, &hole))?
                .is_truthy()?
            {
                owner = Some(i);
                break;
            }
        }
        let Some(i) = owner else {
            return Ok(None);
        };
        let copy = hole
            .try_iter()?
            .map(|p| Ok(Vec3::from(xyz(&p?)?)))
            .collect::<PyResult<Vec<_>>>()?;
        result[i].push(PyList::new(py, copy)?.into_any());
    }
    Ok(Some(result))
}
#[pyfunction]
fn divide_face(mesh: &Obj<'_>, face: &Obj<'_>) -> PyResult<bool> {
    let py = mesh.py();
    let api = py.import("core.topology")?;
    let cmd = py.import("core.history_commands")?;
    let ring = face.getattr("vertices")?;
    let holding = || cmd.call_method1("_faces_holding", (mesh, &ring));
    let mother = api.call_method1("find_containing_face", (holding()?, &ring, face))?;
    let mut punched = false;
    let mut swapped = false;
    if !mother.is_none() {
        let grown = api.call_method1("hole_merged_with_loop", (&mother, &ring))?;
        if grown.is_none() {
            mesh.call_method1("add_hole", (&mother, &ring))?;
            punched = true;
        } else {
            let old = mother.getattr("hole_loops")?.get_item(grown.get_item(0)?)?;
            mesh.call_method1("remove_hole", (&mother, old))?;
            mesh.call_method1("add_hole", (&mother, grown.get_item(1)?))?;
            swapped = true;
        }
        cmd.call_method1("_inherit_paint", (face, &mother))?;
    }
    let normal = xyz(&face.call_method0("normal")?)?;
    let d = if ring.is_truthy()? {
        Some(normal.dot(xyz(&ring.get_item(0)?)?))
    } else {
        None
    };
    for other in cmd
        .call_method1("_faces_inside", (mesh, face))?
        .try_iter()?
    {
        let other = other?;
        if other.eq(face)? {
            continue;
        }
        let outer = other.getattr("vertices")?;
        if let Some(d) = d {
            let mut off = false;
            for p in outer.try_iter()? {
                if (normal.dot(xyz(&p?)?) - d).abs() > 0.002 {
                    off = true;
                    break;
                }
            }
            if off {
                continue;
            }
        }
        if api
            .call_method1("loop_inside_face", (face, &outer))?
            .is_truthy()?
        {
            mesh.call_method1("add_hole", (face, outer))?;
            punched = true;
        }
    }
    if !punched && !swapped {
        for other in holding()?.try_iter()? {
            let other = other?;
            if other.eq(face)? {
                continue;
            }
            let outer = api.call_method1("subtract_loop_from_face", (&other, &ring))?;
            let pieces = if !outer.is_none() {
                vec![outer]
            } else {
                let p = api.call_method1("carve_loop_by_chords", (&other, &ring))?;
                if !p.is_truthy()? {
                    continue;
                }
                rows(&p)?
            };
            let Some(holes) = split_holes(&api, &other, &pieces)? else {
                continue;
            };
            mesh.call_method1("remove_face", (&other,))?;
            for (piece, holes) in pieces.into_iter().zip(holes) {
                let f = if holes.is_empty() {
                    mesh.call_method1("add_face", (piece, py.None()))?
                } else {
                    mesh.call_method1("add_face", (piece, holes))?
                };
                f.setattr(
                    "attrs",
                    py.get_type::<PyDict>().call1((other.getattr("attrs")?,))?,
                )?;
            }
            cmd.call_method1("_inherit_paint", (face, other))?;
            break;
        }
    }
    Ok(swapped)
}

fn remove_drawn_faces<'py>(
    mesh: &Obj<'py>,
    rings: &Obj<'py>,
    selection: &Obj<'py>,
) -> PyResult<()> {
    let api = mesh.py().import("core.history_commands")?;
    let scan = mesh.py().import("core.scan_index")?;
    if rings.is_empty()? {
        return Ok(());
    }
    let sc = if rings.len()? <= 256 {
        Some(scan.call_method1("scan", (mesh,))?).filter(|s| !s.is_none())
    } else {
        None
    };
    let mut by_loop: HashMap<Vec<Cell>, VecDeque<Obj<'py>>> = HashMap::new();
    if let Some(sc) = sc {
        for ring in rings.try_iter()? {
            let ring = ring?;
            let target = outline_key(&ring)?;
            if by_loop.contains_key(&target) {
                continue;
            }
            let mut bucket = VecDeque::new();
            for f in api
                .call_method1("_faces_keyed_like", (mesh, ring, &sc))?
                .try_iter()?
            {
                let f = f?;
                if outline_key(&f.getattr("vertices")?)? == target {
                    bucket.push_back(f);
                }
            }
            by_loop.insert(target, bucket);
        }
    } else {
        for f in mesh.getattr("faces")?.try_iter()? {
            let f = f?;
            by_loop
                .entry(outline_key(&f.getattr("vertices")?)?)
                .or_default()
                .push_back(f);
        }
    }
    for ring in rings.try_iter()? {
        if let Some(f) = by_loop
            .get_mut(&outline_key(&ring?)?)
            .and_then(VecDeque::pop_front)
        {
            discard(mesh, selection, "remove_face", &f)?;
        }
    }
    Ok(())
}

type FaceIndex<'py> = HashMap<Segment, Vec<Obj<'py>>>;
fn index_face<'py>(index: &mut FaceIndex<'py>, f: &Obj<'py>) -> PyResult<()> {
    for key in segments(&cells(&f.getattr("vertices")?)?) {
        index.entry(key).or_default().push(f.clone());
    }
    Ok(())
}
fn single_pair<'py>(faces: &[Obj<'py>]) -> PyResult<Option<Vec<Obj<'py>>>> {
    let normals = faces
        .iter()
        .map(|f| xyz(&f.call_method0("normal")?))
        .collect::<PyResult<Vec<_>>>()?;
    let mut pair = None;
    for (i, n) in normals.iter().enumerate() {
        for j in i + 1..normals.len() {
            if n.dot(normals[j]) > 0.999 {
                if pair.is_some() {
                    return Ok(None);
                }
                pair = Some((i, j));
            }
        }
    }
    let Some((a, b)) = pair else {
        return Ok(None);
    };
    if normals
        .iter()
        .enumerate()
        .any(|(i, n)| i != a && i != b && n.dot(normals[a]).abs() > 0.999)
    {
        return Ok(None);
    }
    Ok(Some(vec![faces[a].clone(), faces[b].clone()]))
}
fn has_edge(r: &Obj<'_>, v: &Obj<'_>, w: &Obj<'_>) -> PyResult<bool> {
    let r = rows(r)?;
    for (a, b) in r.iter().zip(r.iter().cycle().skip(1)) {
        if (a.eq(v)? && b.eq(w)?) || (a.eq(w)? && b.eq(v)?) {
            return Ok(true);
        }
    }
    Ok(false)
}
#[pyfunction]
fn erase_selection<'py>(
    mesh: &Obj<'py>,
    rings: &Obj<'py>,
    ends: &Obj<'py>,
    selection: &Obj<'py>,
) -> PyResult<(Vec<(Vec3, Obj<'py>)>, Vec<Obj<'py>>)> {
    remove_drawn_faces(mesh, rings, selection)?;
    let mut slits = Vec::new();
    let mut across = Vec::new();
    if ends.is_empty()? {
        return Ok((slits, across));
    }
    // Query the scan from immediately after face deletion. Its loops and
    // element handles keep their original order as later merges change them.
    let scan_api = mesh.py().import("core.scan_index")?;
    let scan = if ends.len()? <= 256 {
        Some(scan_api.call_method1("scan", (mesh,))?).filter(|s| !s.is_none())
    } else {
        None
    };
    let mut index = FaceIndex::new();
    let mut extra = FaceIndex::new();
    let mut dead = HashSet::<u128>::new();
    if scan.is_none() {
        for f in mesh.getattr("faces")?.try_iter()? {
            index_face(&mut index, &f?)?;
        }
    }
    for pair in ends.try_iter()? {
        let Some(edge) = find_edge(mesh, &pair?)? else {
            continue;
        };
        let faces = rows(&edge.getattr("faces")?)?;
        if faces.len() == 2 && faces[0].eq(&faces[1])? {
            slits.push((
                Vec3::from(xyz(&faces[0].getattr("vertices")?.get_item(0)?)?),
                faces[0].call_method0("normal")?,
            ));
            discard(mesh, selection, "remove_edge", &edge)?;
            continue;
        }
        let pair = if faces.len() >= 2 {
            single_pair(&faces)?
        } else {
            None
        };
        if let Some(pair) = pair {
            let merged = mesh.call_method1("dissolve_coplanar_region", (&pair,))?;
            if !merged.is_none() {
                for f in &pair {
                    dead.insert(f.getattr("key")?.extract()?);
                }
                index_face(&mut extra, &merged)?;
                if faces.len() > 2 {
                    for f in faces {
                        if !pair[0].eq(&f)? && !pair[1].eq(&f)? {
                            across.push(f);
                        }
                    }
                    mesh.call_method1("remove_edge", (&edge,))?;
                }
                selection.call_method1("discard", (edge,))?;
                continue;
            }
        }
        let v = edge.getattr("v0")?;
        let w = edge.getattr("v1")?;
        let a = v.getattr("position")?;
        let b = w.getattr("position")?;
        let key = segment(cell(xyz(&a)?.to_array()), cell(xyz(&b)?.to_array()));
        if let Some(scan) = &scan {
            if let std::collections::hash_map::Entry::Vacant(entry) = index.entry(key) {
                let bounds = scan_api.call_method1("box_of", ((a, b), 0.0004))?;
                let hit = scan.call_method1("faces_meeting", (bounds,))?;
                let loops = scan
                    .getattr("native")?
                    .call_method1("face_loops", (&hit,))?;
                let mut faces = Vec::new();
                for (i, r) in rows(&hit)?.into_iter().zip(rows(&loops)?) {
                    let cells = rows(&r)?
                        .iter()
                        .map(|v| {
                            let p = scan.call_method1("tkey", (v,))?.extract()?;
                            Ok(crate::store::commands::supplied_cell(p))
                        })
                        .collect::<PyResult<Vec<_>>>()?;
                    faces.extend(segments(&cells).filter(|&k| k == key).map(|_| i.clone()));
                }
                entry.insert(rows(&scan.call_method1("faces", (faces,))?)?);
            }
        }
        let mut dying = Vec::new();
        for f in index.get(&key).into_iter().chain(extra.get(&key)).flatten() {
            if !dead.contains(&f.getattr("key")?.extract::<u128>()?) {
                dying.push(f);
            }
        }
        for f in &faces {
            if dead.contains(&f.getattr("key")?.extract::<u128>()?)
                || dying.iter().any(|other| other.eq(f).unwrap_or(false))
            {
                continue;
            }
            for h in rows(&f.getattr("hole_loops")?)? {
                if has_edge(&h, &v, &w)? && !has_edge(&f.getattr("loop")?, &v, &w)? {
                    mesh.call_method1("remove_hole", (f, h))?;
                }
            }
        }
        for f in dying {
            dead.insert(f.getattr("key")?.extract()?);
            discard(mesh, selection, "remove_face", f)?;
        }
        discard(mesh, selection, "remove_edge", &edge)?;
    }
    Ok((slits, across))
}
#[pyfunction]
fn prune_soft_edges(mesh: &Obj<'_>) -> PyResult<()> {
    let scan = mesh
        .py()
        .import("core.scan_index")?
        .call_method1("scan", (mesh,))?;
    let edges = if scan.is_none() {
        mesh.getattr("edges")?
    } else {
        scan.call_method1(
            "edges",
            (scan
                .getattr("native")?
                .call_method1("edges_faceless", (true,))?,),
        )?
    };
    for e in rows(&edges)? {
        if e.getattr("soft")?.is_truthy()? && !e.getattr("faces")?.is_truthy()? {
            mesh.call_method1("remove_edge", (e,))?;
        }
    }
    Ok(())
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "object_drawing")?;
    sub.add_function(wrap_pyfunction!(crate::object_planes::coplanar_plane, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_planes::has_plane_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_planes::face_rebuild_plane, &sub)?)?;
    sub.add_class::<crate::draw_overlay::ScanOverlay>()?;
    sub.add_class::<crate::drawing_queries::Nearby>()?;
    sub.add_function(wrap_pyfunction!(crate::drawing_queries::touching_segments, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::drawing_queries::find_duplicate_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_curves::point_on_segment, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_curves::tag_curve, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_curves::resplit_curves, &sub)?)?;
    sub.add_function(wrap_pyfunction!(delete_edges, &sub)?)?;
    sub.add_function(wrap_pyfunction!(delete_face, &sub)?)?;
    sub.add_function(wrap_pyfunction!(divide_face, &sub)?)?;
    sub.add_function(wrap_pyfunction!(erase_selection, &sub)?)?;
    sub.add_function(wrap_pyfunction!(prune_soft_edges, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::order_chain, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::divide_edges, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::join_edges, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::interior_vertex, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::collapsible, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::collapse_vertex, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::revert_collapse, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_edge_edits::dedupe_faces, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::plan_segments, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::graph_adjacency, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::graph_insert, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::graph_remove, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::draw_plan::smallest_cycle, &sub)?)?;
    crate::kernel::object_boundary::register(&sub)?;
    crate::kernel::object_repair::register(&sub)?;
    m.add_submodule(&sub)
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Editable object-mesh form of the store sweep. Collect input edge runs
//! before removing the profile so circles and rails survive the stitch.
use super::*;
use std::collections::BTreeMap;
type Curve = Option<i64>;
fn edge_at<'p>(mesh: &Obj<'p>, a: [f64; 3], b: [f64; 3]) -> PyResult<Obj<'p>> {
    let vertex = |p| mesh.call_method1("vertex_at", (Vec3::from(DVec3::from_array(p)),));
    let (a, b) = (vertex(a)?, vertex(b)?);
    if a.is_none() || b.is_none() || a.eq(&b)? {
        return Ok(mesh.py().None().into_bound(mesh.py()));
    }
    mesh.call_method1("find_edge", (a, b))
}
fn curve(edge: &Obj) -> PyResult<Curve> {
    if edge.is_none() {
        Ok(None)
    } else {
        edge.getattr("curve")?.extract()
    }
}
fn tag(edges: &mut Vec<Obj>) -> PyResult<()> {
    let mut id = None;
    for e in edges.drain(..) {
        if e.is_none() || curve(&e)?.is_some() || e.getattr("soft")?.is_truthy()? {
            continue;
        }
        let cid = match &id {
            Some(id) => id,
            None => id.insert(
                e.py()
                    .import("core.mesh")?
                    .getattr("Mesh")?
                    .call_method0("next_curve_id")?,
            ),
        };
        e.setattr("curve", cid)?;
    }
    Ok(())
}
fn tag_runs(
    mesh: &Obj,
    ss: &[Station],
    closed: bool,
    profile: &[Vec<Curve>],
    spans: &[Curve],
) -> PyResult<()> {
    if !closed {
        for end in [&ss[0], &ss[ss.len() - 1]] {
            for (ring, old) in end.iter().zip(profile) {
                // Keep first-seen order when allocating new curve IDs.
                let mut groups = Vec::<Vec<Obj>>::new();
                let mut index = BTreeMap::new();
                for (j, cid) in old.iter().enumerate().take(ring.len()) {
                    if let Some(c) = cid {
                        let k = *index.entry(c).or_insert_with(|| {
                            groups.push(vec![]);
                            groups.len() - 1
                        });
                        groups[k].push(edge_at(mesh, ring[j], ring[(j + 1) % ring.len()])?);
                    }
                }
                for run in &mut groups {
                    tag(run)?;
                }
            }
        }
    }
    if spans.iter().all(Option::is_none) {
        return Ok(());
    }
    let n = spans.len();
    let first = if closed {
        (0..n)
            .find(|i| spans[*i] != spans[(*i + n - 1) % n])
            .unwrap_or(0)
    } else {
        0
    };
    for (i, r) in ss[0].iter().enumerate() {
        for j in 0..r.len() {
            let mut run = vec![];
            let mut old = None;
            for k in 0..n {
                let s = (first + k) % n;
                if spans[s] != old {
                    tag(&mut run)?;
                    old = spans[s];
                }
                if old.is_some() {
                    run.push(edge_at(mesh, ss[s][i][j], ss[(s + 1) % ss.len()][i][j])?);
                }
            }
            tag(&mut run)?;
        }
    }
    Ok(())
}
fn edge_between<'p>(mesh: &Obj<'p>, ring: &[Obj<'p>], j: usize) -> PyResult<Obj<'p>> {
    mesh.call_method1("find_edge", (&ring[j], &ring[(j + 1) % ring.len()]))
}

pub(super) fn object_sweep(mesh: &Obj, face: &Obj, path: &Obj, closed: bool) -> PyResult<bool> {
    let Some((ss, dirs)) = stations(face, path, closed)? else {
        return Ok(false);
    };
    let py = mesh.py();
    let before = py.get_type::<PySet>().call1((mesh.getattr("edges")?,))?;
    let old_faces = PySet::empty(py)?;
    for f in mesh.getattr("faces")?.try_iter()? {
        old_faces.add(f?.getattr("key")?)?;
    }
    let mut loops = vec![face
        .getattr("loop")?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?];
    for hole in face.getattr("hole_loops")?.try_iter()? {
        loops.push(hole?.try_iter()?.collect::<PyResult<_>>()?);
    }
    let mut profile = vec![];
    for ring in &loops {
        profile.push(
            (0..ring.len())
                .map(|j| curve(&edge_between(mesh, ring, j)?))
                .collect::<PyResult<_>>()?,
        );
    }
    let path = coords(path)?;
    let spans = (0..dirs.len())
        .map(|i| curve(&edge_at(mesh, path[i], path[(i + 1) % path.len()])?))
        .collect::<PyResult<Vec<_>>>()?;
    let attrs = face.getattr("attrs")?;
    let copy = py.import("copy")?.getattr("deepcopy")?;
    mesh.call_method1("remove_face", (face,))?;
    for p in polygons(py, &ss, closed, true)? {
        let mut r = vectors(&p);
        let outer = r.remove(0);
        let holes = (!r.is_empty()).then_some(r);
        mesh.call_method1("add_face", (outer, holes))?
            .setattr("attrs", copy.call1((&attrs,))?)?;
    }
    for ring in &loops {
        for j in 0..ring.len() {
            let e = edge_between(mesh, ring, j)?;
            if !e.is_none() && !e.getattr("faces")?.is_truthy()? {
                mesh.call_method1("remove_edge", (e,))?;
            }
        }
    }
    stitch(mesh, &ss)?;
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        let fs = e.getattr("faces")?;
        if e.getattr("soft")?.is_truthy()? || fs.len()? != 2 {
            continue;
        }
        let a = fs.get_item(0)?;
        let b = fs.get_item(1)?;
        if before.contains(&e)?
            && (old_faces.contains(a.getattr("key")?)? || old_faces.contains(b.getattr("key")?)?)
        {
            continue;
        }
        let normal = |f: &Obj| -> PyResult<DVec3> {
            Ok(xyz(&f.call_method0("normal")?)?.normalize_or_zero())
        };
        let dot = normal(&a)?.dot(normal(&b)?);
        if FACET_COS < dot && dot < 0.99995 {
            e.setattr("soft", true)?;
        }
    }
    tag_runs(mesh, &ss, closed, &profile, &spans)?;
    Ok(true)
}

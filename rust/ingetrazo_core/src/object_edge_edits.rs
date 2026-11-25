// SPDX-License-Identifier: GPL-3.0-or-later
//! Divide and Join on materialised mesh incidence. Mutations go through the
//! owning mesh so callers retaining vertices, flags and undo snapshots see them.
use crate::{
    facegeom::{py_sum, Hypot},
    math::{xyz, Vec3},
};
use glam::DVec3;
use pyo3::{prelude::*, types::PyDict};
use std::collections::{BTreeMap, HashMap, HashSet};
type Obj<'py> = Bound<'py, PyAny>;
type Step<'py> = (Obj<'py>, Obj<'py>, Obj<'py>);

fn key(item: &Obj<'_>) -> PyResult<u128> {
    let k = item.getattr("key")?;
    k.extract::<u128>()
        .or_else(|_| k.extract::<i128>().map(|v| v as u128))
}
fn position(v: &Obj<'_>) -> PyResult<DVec3> {
    xyz(&v.getattr("position")?)
}
fn ends<'py>(e: &Obj<'py>) -> PyResult<[Obj<'py>; 2]> {
    Ok([e.getattr("v0")?, e.getattr("v1")?])
}
fn rows<'py>(it: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    it.try_iter()?.collect()
}

fn walk<'py>(chain: Vec<Obj<'py>>, seed: &Obj<'py>) -> PyResult<Vec<Step<'py>>> {
    if let [e] = chain.as_slice() {
        let [a, b] = ends(e)?;
        return Ok(vec![(e.clone(), a, b)]);
    }
    if chain.is_empty() {
        return Err(pyo3::exceptions::PyIndexError::new_err("empty curve"));
    }
    let mut vertices = Vec::new();
    let mut incident = HashMap::<u128, Vec<usize>>::new();
    let mut endpoints = Vec::new();
    for (i, e) in chain.iter().enumerate() {
        let pair = ends(e)?;
        for v in &pair {
            let k = key(v)?;
            let edges = incident.entry(k).or_default();
            if edges.is_empty() {
                vertices.push((k, v.clone()));
            }
            edges.push(i);
        }
        endpoints.push(pair);
    }
    let mut from = vertices
        .into_iter()
        .find(|(k, _)| incident[k].len() == 1)
        .map(|(_, v)| v)
        .unwrap_or(seed.getattr("v0")?);
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    while let Some(&i) = incident
        .get(&key(&from)?)
        .and_then(|list| list.iter().find(|i| !seen.contains(*i)))
    {
        seen.insert(i);
        let [a, b] = &endpoints[i];
        let to = if key(a)? == key(&from)? { b } else { a };
        result.push((chain[i].clone(), from, to.clone()));
        from = to.clone();
    }
    Ok(result)
}
#[pyfunction]
pub(crate) fn order_chain<'py>(chain: &Obj<'py>, seed: &Obj<'py>) -> PyResult<Vec<Step<'py>>> {
    walk(rows(chain)?, seed)
}

#[pyfunction]
pub(crate) fn divide_edges(mesh: &Obj<'_>, edges: &Obj<'_>, n: usize) -> PyResult<usize> {
    if n < 2 {
        return Ok(0);
    }
    let py = mesh.py();
    let api = py.import("ingetrazo_core.mesh")?;
    let flags = api.getattr("edge_flags")?;
    let stamp = api.getattr("stamp_edge_flags")?;
    let mut cuts = 0;
    let mut done = HashSet::new();
    for edge in rows(edges)? {
        if done.contains(&key(&edge)?) {
            continue;
        }
        let curve = !edge.getattr("curve")?.is_none();
        let walk = if curve {
            walk(rows(&mesh.call_method1("curve_edges", (&edge,))?)?, &edge)?
        } else {
            let [a, b] = ends(&edge)?;
            vec![(edge, a, b)]
        };
        let mut lengths = Vec::new();
        for (e, a, b) in &walk {
            done.insert(key(e)?);
            lengths.push((position(b)? - position(a)?).length());
        }
        let total = py_sum(lengths.iter().copied(), py.version_info().minor >= 12);
        if total < 1e-9 {
            continue;
        }
        let mut walked = 0.;
        let step = total / n as f64;
        let mut pieces = BTreeMap::<usize, Vec<Obj<'_>>>::new();
        let mut save = |lo: f64, hi: f64, e| {
            if curve {
                let mid = (lo + hi) * 0.5;
                let k = ((mid - mid % step) / step).round() as usize;
                pieces.entry(k.min(n - 1)).or_default().push(e);
            }
        };
        for ((mut rest, mut from, to), len) in walk.into_iter().zip(lengths) {
            if len < 1e-12 {
                continue;
            }
            let a = position(&from)?;
            let d = position(&to)? - a;
            let keep = flags.call1((&rest,))?;
            let mut start = walked;
            for k in 1..n {
                let at = step * k as f64;
                if at <= walked + 1e-9 || at >= walked + len - 1e-9 {
                    continue;
                }
                let pos = Vec3::from(a + d * ((at - walked) / len));
                let pair = mesh.call_method1("split_edge", (&rest, pos))?;
                let (mut first, mut second) = (pair.get_item(0)?, pair.get_item(1)?);
                if first.eq(&second)? {
                    break;
                }
                let [v, w] = ends(&first)?;
                if !v.eq(&from)? && !w.eq(&from)? {
                    std::mem::swap(&mut first, &mut second);
                }
                stamp.call1((&first, &keep))?;
                stamp.call1((&second, &keep))?;
                from = first.call_method1("other", (&from,))?;
                save(start, at, first);
                rest = second;
                start = at;
                cuts += 1;
            }
            save(start, walked + len, rest);
            walked += len;
        }
        for facets in pieces.into_values() {
            let id = api.getattr("Mesh")?.call_method0("next_curve_id")?;
            for facet in facets {
                facet.setattr("curve", &id)?;
            }
        }
    }
    Ok(cuts)
}

#[pyfunction]
#[pyo3(signature=(mesh,live,verdicts=None))]
pub(crate) fn join_edges<'py>(
    mesh: &Obj<'py>,
    mut live: Vec<Obj<'py>>,
    verdicts: Option<&Bound<'py, PyDict>>,
) -> PyResult<Vec<Obj<'py>>> {
    let py = mesh.py();
    let api = py.import("ingetrazo_core.mesh")?;
    let flags = api.getattr("edge_flags")?;
    let mut picked = HashSet::new();
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for edge in &live {
        picked.insert(key(edge)?);
        for v in ends(edge)? {
            if seen.insert(key(&v)?) {
                candidates.push(v);
            }
        }
    }
    for v in candidates {
        let incident = rows(&v.getattr("edges")?)?;
        if incident.len() < 2 {
            continue;
        }
        let p = position(&v)?.to_array();
        let geometry = |a: &Obj<'_>, b: &Obj<'_>| {
            Ok::<_, PyErr>(crate::store::drawing_edges::join_geometry(
                [p, position(a)?.to_array(), position(b)?.to_array()],
                Hypot::LibStdCxx,
                py.version_info().minor >= 12,
            ))
        };
        let keep = |reason: &str| -> PyResult<()> {
            if let Some(verdicts) = verdicts {
                verdicts.set_item(&v, reason)?;
            }
            Ok(())
        };
        if incident.len() > 2 {
            if verdicts.is_some() {
                let mut ends = Vec::new();
                for e in &incident {
                    if picked.contains(&key(e)?) {
                        ends.push(e.call_method1("other", (&v,))?);
                    }
                }
                'pairs: for (i, a) in ends.iter().enumerate() {
                    for b in &ends[i + 1..] {
                        let (straight, kink) = geometry(a, b)?;
                        if straight && kink < 90. {
                            keep("branch")?;
                            break 'pairs;
                        }
                    }
                }
            }
            continue;
        }
        let (e, f) = (&incident[0], &incident[1]);
        let (ek, fk) = (key(e)?, key(f)?);
        if !picked.contains(&ek) || !picked.contains(&fk) {
            continue;
        }
        let a = e.call_method1("other", (&v,))?;
        let b = f.call_method1("other", (&v,))?;
        let (straight, kink) = geometry(&a, &b)?;
        if !straight {
            if kink < 1. {
                keep("kink")?;
            }
            continue;
        }
        let saved = flags.call1((e,))?;
        if !saved.eq(flags.call1((f,))?)? {
            keep("flags")?;
            continue;
        }
        if !e.getattr("curve")?.is_none() || !f.getattr("curve")?.is_none() {
            keep("curve")?;
            continue;
        }
        if !mesh
            .call_method1("collapsible_vertex", (&v,))?
            .is_truthy()?
            || !mesh.call_method1("find_edge", (&a, &b))?.is_none()
        {
            continue;
        }
        let joined = mesh
            .call_method1("collapse_vertex", (&v,))?
            .get_item("new_edge")?;
        api.call_method1("stamp_edge_flags", (&joined, &saved))?;
        keep("join")?;
        picked.remove(&ek);
        picked.remove(&fk);
        picked.insert(key(&joined)?);
        live.push(joined);
    }
    let edges = mesh.getattr("edges")?;
    let mut answer = Vec::new();
    for e in live {
        if picked.contains(&key(&e)?) && edges.contains(&e)? {
            answer.push(e);
        }
    }
    Ok(answer)
}

#[pyfunction]
pub(crate) fn interior_vertex<'py>(
    mesh: &Obj<'py>,
    edge: &Obj<'py>,
    tol: f64,
) -> PyResult<Option<Obj<'py>>> {
    let [start, end] = ends(edge)?;
    let a = position(&start)?;
    let ab = position(&end)? - a;
    let len = ab.length();
    if len < tol {
        return Ok(None);
    }
    for v in mesh.getattr("vertices")?.try_iter()? {
        let v = v?;
        if v.eq(&start)? || v.eq(&end)? {
            continue;
        }
        let p = position(&v)?;
        let t = (p - a).dot(ab) / (len * len);
        if t > tol / len && t < 1. - tol / len && (p - (a + ab * t)).length() < tol {
            return Ok(Some(v));
        }
    }
    Ok(None)
}

#[pyfunction]
pub(crate) fn collapsible(v: &Obj<'_>, tol: f64) -> PyResult<bool> {
    let disk = v.getattr("edges")?;
    if disk.len()? != 2 {
        return Ok(false);
    }
    let edges = rows(&disk)?;
    let (e, f) = (&edges[0], &edges[1]);
    let p = position(v)?;
    let d1 = p - position(&e.call_method1("other", (v,))?)?;
    let d2 = position(&f.call_method1("other", (v,))?)? - p;
    if d1.length() < tol || d2.length() < tol || d1.normalize().dot(d2.normalize()) < 0.999 {
        return Ok(false);
    }
    let faces = rows(&e.getattr("faces")?)?;
    let ids = |items: &[Obj<'_>]| items.iter().map(key).collect::<PyResult<HashSet<_>>>();
    if ids(&faces)? != ids(&rows(&f.getattr("faces")?)?)? {
        return Ok(false);
    }
    for face in &faces {
        for ring in crate::object_weld::rings(face)? {
            if ring.len()? <= 3 && ring.contains(v)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

#[pyfunction]
pub(crate) fn collapse_vertex<'py>(
    mesh: &Obj<'py>,
    v: &Obj<'py>,
    cell: &Obj<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let mut edges = rows(&v.getattr("edges")?)?;
    if edges.len() != 2 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "collapse requires two incident edges",
        ));
    }
    let mut ends = [
        edges[0].call_method1("other", (v,))?,
        edges[1].call_method1("other", (v,))?,
    ];
    if position(&ends[1])?.to_array() < position(&ends[0])?.to_array() {
        ends.swap(0, 1);
        edges.swap(0, 1);
    }
    let [a, b] = &ends;
    let (e, f) = (&edges[0], &edges[1]);
    let mut faces = Vec::new();
    let mut seen = HashSet::new();
    for face in rows(&e.getattr("faces")?)? {
        if seen.insert(key(&face)?) {
            faces.push(face);
        }
    }
    mesh.call_method1("remove_edge", (e,))?;
    mesh.call_method1("remove_edge", (f,))?;
    let new = mesh.call_method1("find_edge", (a, b))?.is_none();
    let joined = mesh.call_method1("_link_edge", (a, b))?;
    let users = joined.getattr("faces")?;
    for face in &faces {
        for ring in crate::object_weld::rings(face)? {
            if ring.contains(v)? {
                ring.call_method1("remove", (v,))?;
            }
        }
        if !users.contains(face)? {
            users.call_method1("append", (face,))?;
        }
    }
    let verts = mesh.getattr("vertices")?;
    if verts.contains(v)? {
        verts.call_method1("remove", (v,))?;
    }
    let registry = mesh.getattr("_registry")?;
    let k = cell.call1((v.getattr("position")?,))?;
    if registry.call_method1("get", (&k,))?.is(v) {
        registry.del_item(k)?;
    }
    let rec = PyDict::new(mesh.py());
    rec.set_item("kind", "collapse")?;
    rec.set_item("v", v)?;
    rec.set_item("n1", a)?;
    rec.set_item("n2", b)?;
    rec.set_item("e1", e)?;
    rec.set_item("e2", f)?;
    rec.set_item("new_edge", joined)?;
    rec.set_item("new_edge_new", new)?;
    rec.set_item("faces", faces)?;
    Ok(rec)
}

#[pyfunction]
pub(crate) fn revert_collapse(mesh: &Obj<'_>, rec: &Obj<'_>, cell: &Obj<'_>) -> PyResult<()> {
    let v = rec.get_item("v")?;
    let verts = mesh.getattr("vertices")?;
    if !verts.contains(&v)? {
        verts.call_method1("append", (&v,))?;
    }
    mesh.getattr("_registry")?
        .call_method1("setdefault", (cell.call1((v.getattr("position")?,))?, &v))?;
    let edge = rec.get_item("new_edge")?;
    let faces = rows(&rec.get_item("faces")?)?;
    if rec.get_item("new_edge_new")?.is_truthy()? {
        mesh.call_method1("remove_edge", (&edge,))?;
    } else {
        let linked = edge.getattr("faces")?;
        for face in &faces {
            if linked.contains(face)? {
                linked.call_method1("remove", (face,))?;
            }
        }
    }
    let old = [rec.get_item("e1")?, rec.get_item("e2")?];
    for e in &old {
        mesh.call_method1("relink_edge", (e,))?;
    }
    let (a, b) = (rec.get_item("n1")?, rec.get_item("n2")?);
    for face in faces {
        mesh.call_method1("_insert_into_face_loops", (&face, &a, &b, &v))?;
        for e in &old {
            let linked = e.getattr("faces")?;
            if !linked.contains(&face)? {
                linked.call_method1("append", (&face,))?;
            }
        }
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn dedupe_faces(mesh: &Obj<'_>) -> PyResult<usize> {
    let mut seen = HashMap::<Vec<[u128; 2]>, Obj<'_>>::new();
    let mut count = 0;
    for f in rows(&mesh.getattr("faces")?)? {
        let verts = rows(&f.getattr("loop")?)?
            .iter()
            .map(key)
            .collect::<PyResult<Vec<_>>>()?;
        let mut sig = verts
            .iter()
            .zip(verts.iter().cycle().skip(1))
            .map(|(&a, &b)| [a.min(b), a.max(b)])
            .collect::<Vec<_>>();
        sig.sort_unstable();
        sig.dedup();
        if let Some(other) = seen.get_mut(&sig) {
            let (drop, keep) =
                if f.getattr("hole_loops")?.len()? > other.getattr("hole_loops")?.len()? {
                    (other.clone(), f)
                } else {
                    (f, other.clone())
                };
            let attrs = drop.getattr("attrs")?;
            if attrs.is_truthy()? && !keep.getattr("attrs")?.is_truthy()? {
                keep.setattr("attrs", mesh.py().get_type::<PyDict>().call1((attrs,))?)?;
            }
            mesh.call_method1("remove_face", (drop,))?;
            *other = keep;
            count += 1;
        } else {
            seen.insert(sig, f);
        }
    }
    Ok(count)
}

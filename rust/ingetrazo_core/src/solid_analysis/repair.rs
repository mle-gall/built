// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use glam::DVec3;
pub(super) fn skin_count(edge: &Obj<'_>) -> PyResult<usize> {
    let mut n = 0;
    for f in edge.getattr("faces")?.try_iter()? {
        n += usize::from(_live(&f?)?);
    }
    Ok(n)
}
#[pyfunction]
#[pyo3(signature=(mesh,edges=None))]
fn _border_loops<'p>(mesh: &Obj<'p>, edges: Option<&Obj<'p>>) -> PyResult<Vec<Vec<Obj<'p>>>> {
    let edges = edges
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| mesh.getattr("edges"))?;
    let mut at = HashMap::<Key, Vec<usize>>::new();
    let mut open = vec![];
    for edge in edges.try_iter()? {
        let edge = edge?;
        if skin_count(&edge)? != 1 {
            continue;
        }
        let a = edge.getattr("v0")?;
        let b = edge.getattr("v1")?;
        for v in [&a, &b] {
            at.entry(key(v)?).or_default().push(open.len());
        }
        open.push((a, b));
    }
    let mut used = vec![false; open.len()];
    let mut loops = vec![];
    for (i, (a, b)) in open.iter().enumerate() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let start = key(a)?;
        let mut cur = b;
        let mut ring = vec![a.clone()];
        let mut ok = at[&start].len() == 2;
        while ok && key(cur)? != start {
            let ways = &at[&key(cur)?];
            let next = ways.iter().copied().find(|&j| !used[j]);
            if ways.len() != 2 || next.is_none() {
                ok = false;
                break;
            }
            let j = next.unwrap();
            used[j] = true;
            ring.push(cur.clone());
            let (left, right) = &open[j];
            cur = if key(left)? == key(cur)? { right } else { left };
        }
        if ok && ring.len() >= 3 {
            loops.push(ring);
        }
    }
    Ok(loops)
}
fn flat(points: &[Obj<'_>], tol: f64) -> PyResult<bool> {
    let pts = points.iter().map(xyz).collect::<PyResult<Vec<_>>>()?;
    let normal = crate::polygon::raw_newell(&pts);
    if normal.length() < 1e-12 {
        return Ok(false);
    }
    let unit = normal.normalize();
    let center = pts.iter().copied().sum::<DVec3>() / (pts.len() as f64);
    Ok(pts.iter().all(|p| (*p - center).dot(unit).abs() <= tol))
}
#[pyfunction]
fn _flat(points: &Obj<'_>, tolerance: f64) -> PyResult<bool> {
    flat(&rows(points)?, tolerance)
}
#[pyfunction]
fn _agree_with_neighbour(mesh: &Obj<'_>, face: &Obj<'_>) -> PyResult<()> {
    let outer = face.getattr("loop")?;
    let a = outer.get_item(0)?;
    let b = outer.get_item(1)?;
    let edge = mesh.call_method1("find_edge", (&a, &b))?;
    if !edge.is_none() {
        for other in edge.getattr("faces")?.try_iter()? {
            let other = other?;
            if other.eq(face)? || !_live(&other)? {
                continue;
            }
            for (x, y) in _steps(&other)? {
                if x.eq(&a)? && y.eq(&b)? {
                    face.call_method0("reverse_rings")?;
                    break;
                }
            }
            break;
        }
    }
    Ok(())
}
fn tolerance(py: Python<'_>) -> PyResult<f64> {
    py.import("core.topology")?
        .getattr("_PLANAR_TOLERANCE")?
        .extract()
}
#[pyfunction]
fn _cap<'p>(mesh: &Obj<'p>, r#loop: Vec<Obj<'p>>) -> PyResult<Option<Obj<'p>>> {
    let py = mesh.py();
    let pts = positions(&r#loop)?;
    if !flat(&pts, tolerance(py)?)? {
        return Ok(None);
    }
    let copy = py.import("ingetrazo_core")?.getattr("Vec3")?;
    let pts = pts
        .into_iter()
        .map(|p| copy.call1((p,)))
        .collect::<PyResult<Vec<_>>>()?;
    let cap = mesh.call_method1("add_face", (pts,))?;
    _agree_with_neighbour(mesh, &cap)?;
    Ok(Some(cap))
}
#[pyfunction]
fn why_not_repairable<'p>(mesh: &Obj<'p>, item: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let py = mesh.py();
    let msg = match item.get_item("kind")?.extract::<String>()?.as_str() {
        "open" => {
            let ents = item.get_item("ents")?;
            let rims = _border_loops(mesh, Some(&ents))?;
            if rims.len() != 1 || rims[0].len() != ents.len()? {
                "This hole branches: close it by drawing its faces"
            } else if !flat(&positions(&rims[0])?, tolerance(py)?)? {
                "This hole is not flat: close it by drawing its faces"
            } else {
                return Ok(None);
            }
        }
        "over" => {
            "Edges shared by more than two faces: separate the solids or erase a face by hand"
        }
        _ => return Ok(None),
    };
    Ok(Some(py.import("core.i18n")?.call_method1("tr", (msg,))?))
}

pub(super) fn winding(mesh: &Obj<'_>) -> PyResult<()> {
    let rev = mesh
        .getattr_opt("_winding_rev")?
        .map(|v| v.extract::<u64>())
        .transpose()?
        .unwrap_or(0);
    mesh.setattr("_winding_rev", rev + 1)
}
fn counts() -> HashMap<&'static str, usize> {
    ["stray", "duplicates", "holes", "partitions", "reversed"]
        .map(|s| (s, 0))
        .into()
}
#[pyfunction]
fn repair_item(mesh: &Obj<'_>, item: &Obj<'_>) -> PyResult<HashMap<&'static str, usize>> {
    let mut done = counts();
    if why_not_repairable(mesh, item)?.is_some() {
        return Ok(done);
    }
    let ents = item.get_item("ents")?;
    match item.get_item("kind")?.extract::<String>()?.as_str() {
        "open" => {
            if let Some(rim) = _border_loops(mesh, Some(&ents))?.into_iter().next() {
                done.insert("holes", usize::from(_cap(mesh, rim)?.is_some()));
            }
        }
        "stray" => {
            let mut removed = 0;
            for e in ents.try_iter()? {
                let e = e?;
                if !e.getattr("faces")?.is_truthy()? && mesh.getattr("edges")?.contains(&e)? {
                    mesh.call_method1("remove_edge", (e,))?;
                    removed += 1;
                }
            }
            mesh.call_method0("prune_orphan_vertices")?;
            done.insert("stray", removed);
        }
        "internal" => {
            let face = ents.get_item(0)?;
            let key = face_key(&face)?;
            let mut duplicate = false;
            for f in mesh.getattr("faces")?.try_iter()? {
                let f = f?;
                if f.ne(&face)? && face_key(&f)? == key {
                    duplicate = true;
                    break;
                }
            }
            if duplicate {
                mesh.call_method1("remove_face", (face,))?;
                done.insert("duplicates", 1);
            } else {
                face.setattr("interior", true)?;
                done.insert("partitions", 1);
            }
        }
        "reversed" => {
            mesh.py()
                .import("ingetrazo_core.mesh")?
                .call_method1("turn_face_over", (ents.get_item(0)?,))?;
            winding(mesh)?;
            done.insert("reversed", 1);
        }
        _ => (),
    }
    Ok(done)
}
#[pyfunction]
fn repair_mesh(mesh: &Obj<'_>) -> PyResult<HashMap<&'static str, usize>> {
    let mut done = counts();
    for edge in rows(&mesh.getattr("edges")?)? {
        if !edge.getattr("faces")?.is_truthy()? {
            mesh.call_method1("remove_edge", (edge,))?;
            *done.get_mut("stray").unwrap() += 1;
        }
    }
    let mut seen = HashSet::new();
    for f in rows(&mesh.getattr("faces")?)? {
        if !seen.insert(face_key(&f)?) {
            mesh.call_method1("remove_face", (f,))?;
            *done.get_mut("duplicates").unwrap() += 1;
        }
    }
    for rim in _border_loops(mesh, None)? {
        if _cap(mesh, rim)?.is_some() {
            *done.get_mut("holes").unwrap() += 1;
        }
    }
    let mut before = HashSet::new();
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        if !_live(&f)? {
            before.insert(key(&f)?);
        }
    }
    let turned = mesh
        .py()
        .import("core.orient")?
        .call_method1("orient_outward", (mesh,))?
        .len()?;
    done.insert("reversed", turned);
    if turned > 0 {
        winding(mesh)?;
    }
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        if !_live(&f)? && !before.contains(&key(&f)?) {
            *done.get_mut("partitions").unwrap() += 1;
        }
    }
    mesh.call_method0("prune_orphan_vertices")?;
    Ok(done)
}
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(_border_loops, m)?,
        wrap_pyfunction!(_flat, m)?,
        wrap_pyfunction!(_agree_with_neighbour, m)?,
        wrap_pyfunction!(_cap, m)?,
        wrap_pyfunction!(why_not_repairable, m)?,
        wrap_pyfunction!(repair_item, m)?,
        wrap_pyfunction!(repair_mesh, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}

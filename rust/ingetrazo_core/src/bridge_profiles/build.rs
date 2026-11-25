// SPDX-License-Identifier: GPL-3.0-or-later
//! Stitch ruled sides into the store and orient only the newly added faces.
use super::{bridge_plan, xyz, PointList, EPS};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::collections::{HashSet, VecDeque};
type Obj<'py> = PointList<'py>;
fn rings<'py>(f: &Obj<'py>) -> PyResult<Vec<Vec<Obj<'py>>>> {
    let mut rings = vec![f.getattr("loop")?.try_iter()?.collect::<PyResult<_>>()?];
    for h in f.getattr("hole_loops")?.try_iter()? {
        rings.push(h?.try_iter()?.collect::<PyResult<_>>()?);
    }
    Ok(rings)
}
fn edges<'py>(mesh: &Obj<'py>, f: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mut out = vec![];
    for ring in rings(f)? {
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let e = mesh.call_method1("find_edge", (a, b))?;
            if !e.is_none() {
                out.push(e)
            }
        }
    }
    Ok(out)
}
fn direction(f: &Obj<'_>, a: &Obj<'_>, b: &Obj<'_>) -> PyResult<i8> {
    for ring in rings(f)? {
        for (p, q) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            if p.eq(a)? && q.eq(b)? {
                return Ok(1);
            }
            if p.eq(b)? && q.eq(a)? {
                return Ok(-1);
            }
        }
    }
    Ok(0)
}
fn keys(seq: &Obj<'_>) -> PyResult<HashSet<u128>> {
    seq.try_iter()?
        .map(|f| f?.getattr("key")?.extract())
        .collect()
}
#[pyfunction]
#[pyo3(signature=(mesh,new,caps=None))]
pub(super) fn follow_neighbours<'py>(
    mesh: &Obj<'py>,
    new: &Obj<'py>,
    caps: Option<&Obj<'py>>,
) -> PyResult<Bound<'py, PyList>> {
    let py = mesh.py();
    let new_ids = keys(new)?;
    let mut settled = HashSet::new();
    let turned = PyList::empty(py);
    let turn = crate::model::module(py, "ingetrazo_core.mesh")?.getattr("turn_face_over")?;
    let spread = |queue: &mut VecDeque<Obj<'py>>, settled: &mut HashSet<u128>| -> PyResult<()> {
        while let Some(g) = queue.pop_front() {
            for edge in edges(mesh, &g)? {
                let a = edge.getattr("v0")?;
                let b = edge.getattr("v1")?;
                let dg = direction(&g, &a, &b)?;
                if dg == 0 {
                    continue;
                }
                for f in edge.getattr("faces")?.try_iter()? {
                    let f = f?;
                    let key = f.getattr("key")?.extract::<u128>()?;
                    if new_ids.contains(&key) && settled.insert(key) {
                        if direction(&f, &a, &b)? == dg {
                            turn.call1((&f,))?;
                            turned.append(&f)?;
                        }
                        queue.push_back(f);
                    }
                }
            }
        }
        Ok(())
    };
    let mut queue = VecDeque::new();
    if let Some(caps) = caps {
        for f in caps.try_iter()? {
            queue.push_back(f?);
        }
    }
    spread(&mut queue, &mut settled)?;
    if settled.len() < new_ids.len() {
        let mut seen = HashSet::new();
        for f in new.try_iter()? {
            let f = f?;
            if settled.contains(&f.getattr("key")?.extract::<u128>()?) {
                continue;
            }
            for e in edges(mesh, &f)? {
                for g in e.getattr("faces")?.try_iter()? {
                    let g = g?;
                    let key = g.getattr("key")?.extract::<u128>()?;
                    if !new_ids.contains(&key) && seen.insert(key) {
                        queue.push_back(g);
                    }
                }
            }
        }
        spread(&mut queue, &mut settled)?;
    }
    Ok(turned)
}
#[pyfunction]
pub(super) fn bridge<'py>(mesh: &Obj<'py>, profiles: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = mesh.py();
    let polys = bridge_plan(profiles)?;
    let before = keys(&mesh.getattr("faces")?)?;
    let mut diagonals = vec![];
    let seed = PySet::empty(py)?;
    let key = crate::model::module(py, "core.topology")?.getattr("_key")?;
    for poly in polys.iter() {
        let mut points = vec![];
        let mut coords: Vec<glam::DVec3> = vec![];
        for pt in poly.try_iter()? {
            let pt = pt?;
            seed.add(key.call1((&pt,))?)?;
            let p = xyz(&pt)?;
            if coords.last().is_none_or(|q| p.distance(*q) >= EPS) {
                points.push(pt);
                coords.push(p);
            }
        }
        if coords.len() > 1 && coords[0].distance(*coords.last().unwrap()) < EPS {
            coords.pop();
            points.pop();
        }
        if coords.len() < 3 {
            continue;
        }
        if let [a, b, c, d] = coords.as_slice() {
            let mut n = (*b - *a).cross(*c - *a);
            if n.length() < EPS {
                n = (*c - *a).cross(*d - *a)
            }
            let size = coords.iter().map(|p| p.distance(*a)).fold(0., f64::max);
            if n.length() >= EPS && (*d - *a).dot(n.normalize()).abs() > 1e-5 * size {
                let (tris, diag) = if a.distance(*c) <= b.distance(*d) {
                    ([[0, 1, 2], [0, 2, 3]], [0, 2])
                } else {
                    ([[0, 1, 3], [1, 2, 3]], [1, 3])
                };
                for tri in tris {
                    mesh.call_method1("add_face", (PyList::new(py, tri.map(|i| &points[i]))?,))?;
                }
                diagonals.push((points[diag[0]].clone(), points[diag[1]].clone()));
                continue;
            }
        }
        mesh.call_method1("add_face", (PyList::new(py, points)?,))?;
    }
    let fresh = || -> PyResult<Bound<'py, PyList>> {
        let l = PyList::empty(py);
        for f in mesh.getattr("faces")?.try_iter()? {
            let f = f?;
            if !before.contains(&f.getattr("key")?.extract::<u128>()?) {
                l.append(f)?;
            }
        }
        Ok(l)
    };
    let new = PySet::new(py, fresh()?.iter())?;
    crate::doc::history_support::run_stitch(mesh, seed.as_any(), Some(new.as_any()), true, true)?;
    let new = fresh()?;
    for (a, b) in diagonals {
        let a = mesh.call_method1("vertex_at", (a,))?;
        let b = mesh.call_method1("vertex_at", (b,))?;
        if !a.is_none() && !b.is_none() {
            let e = mesh.call_method1("find_edge", (a, b))?;
            if !e.is_none() {
                e.setattr("soft", true)?;
            }
        }
    }
    let ids = keys(new.as_any())?;
    let facet = crate::model::module(py, "core.sweep")?
        .getattr("_CURVE_FACET_COS")?
        .extract::<f64>()?;
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        if e.getattr("soft")?.is_truthy()? {
            continue;
        }
        let faces = e.getattr("faces")?;
        if faces.len()? != 2 {
            continue;
        }
        let a = faces.get_item(0)?;
        let b = faces.get_item(1)?;
        if ids.contains(&a.getattr("key")?.extract::<u128>()?)
            && ids.contains(&b.getattr("key")?.extract::<u128>()?)
        {
            let d = xyz(&a.call_method0("normal")?)?
                .normalize_or_zero()
                .dot(xyz(&b.call_method0("normal")?)?.normalize_or_zero());
            if d > facet && d < 0.99995 {
                e.setattr("soft", true)?;
            }
        }
    }
    if !new.is_empty() {
        let caps = PyList::empty(py);
        let live = mesh.getattr("faces")?;
        for pr in profiles.try_iter()? {
            let f = pr?.getattr("face")?;
            if !f.is_none() && live.contains(&f)? {
                caps.append(f)?;
            }
        }
        follow_neighbours(mesh, new.as_any(), Some(caps.as_any()))?;
        let kw = PyDict::new(py);
        kw.set_item("only", &new)?;
        crate::model::module(py, "core.orient")?.call_method(
            "orient_outward",
            (mesh,),
            Some(&kw),
        )?;
    }
    Ok(new)
}

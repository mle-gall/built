// SPDX-License-Identifier: GPL-3.0-or-later
//! Topological diagnosis for the Solid Inspector and its scripting API.
use crate::math::xyz;
use pyo3::{
    prelude::*,
    types::{PyDict, PyFrozenSet, PyList, PyTuple},
};
use std::collections::{HashMap, HashSet};
mod command;
mod problems;
mod repair;
type Obj<'p> = Bound<'p, PyAny>;

// Standalone vertices have negative keys. Store elements carry unsigned document keys.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Key {
    Local(i64),
    Store(u128),
}
fn key(obj: &Obj<'_>) -> PyResult<Key> {
    let value = obj.getattr("key")?;
    value
        .extract()
        .map(Key::Store)
        .or_else(|_| value.extract().map(Key::Local))
}
fn rows<'p>(seq: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    seq.try_iter()?.collect()
}
fn corners<'p>(face: &Obj<'p>) -> PyResult<Vec<Vec<Obj<'p>>>> {
    let mut rings = vec![rows(&face.getattr("loop")?)?];
    for hole in face.getattr("hole_loops")?.try_iter()? {
        rings.push(rows(&hole?)?);
    }
    Ok(rings)
}
fn sides<T>(ring: &[T]) -> impl Iterator<Item = (&T, &T)> {
    ring.iter().zip(ring.iter().cycle().skip(1))
}
type Pair = (Key, Key);
fn edge_pair(a: &Obj<'_>, b: &Obj<'_>) -> PyResult<Pair> {
    let (a, b) = (key(a)?, key(b)?);
    Ok(if a < b { (a, b) } else { (b, a) })
}
fn face_pairs(f: &Obj<'_>) -> PyResult<Vec<Pair>> {
    let mut steps = Vec::new();
    for ring in corners(f)? {
        for (a, b) in sides(&ring) {
            steps.push(edge_pair(a, b)?);
        }
    }
    Ok(steps)
}
fn face_key(f: &Obj<'_>) -> PyResult<Vec<Vec<Key>>> {
    let mut rings = corners(f)?
        .into_iter()
        .map(|r| {
            let mut vs = r.iter().map(key).collect::<PyResult<Vec<_>>>()?;
            vs.sort_unstable();
            vs.dedup();
            Ok(vs)
        })
        .collect::<PyResult<Vec<_>>>()?;
    let outer = rings.remove(0);
    rings.sort_unstable();
    rings.dedup();
    rings.insert(0, outer);
    Ok(rings)
}
#[pyfunction]
fn _live(face: &Obj<'_>) -> PyResult<bool> {
    Ok(!face
        .getattr_opt("interior")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false)))
}
#[pyfunction]
fn _steps<'p>(face: &Obj<'p>) -> PyResult<Vec<(Obj<'p>, Obj<'p>)>> {
    let mut steps = vec![];
    for ring in corners(face)? {
        steps.extend(sides(&ring).map(|(a, b)| (a.clone(), b.clone())));
    }
    Ok(steps)
}
#[pyfunction]
fn _step_key<'p>(a: &Obj<'p>, b: &Obj<'p>) -> PyResult<Bound<'p, PyFrozenSet>> {
    PyFrozenSet::new(a.py(), [a.getattr("key")?, b.getattr("key")?])
}
#[pyfunction]
fn _face_key<'p>(face: &Obj<'p>) -> PyResult<(Bound<'p, PyFrozenSet>, Bound<'p, PyFrozenSet>)> {
    let py = face.py();
    let mut rings = corners(face)?
        .into_iter()
        .map(|ring| {
            PyFrozenSet::new(
                py,
                ring.iter()
                    .map(|v| v.getattr("key"))
                    .collect::<PyResult<Vec<_>>>()?,
            )
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok((rings.remove(0), PyFrozenSet::new(py, rings)?))
}
#[pyfunction]
fn internal_faces<'p>(mesh: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let mut seen = HashSet::new();
    let mut count = HashMap::<Pair, usize>::new();
    let mut internal = vec![];
    let mut skin = vec![];
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        if !_live(&f)? {
            continue;
        }
        if !seen.insert(face_key(&f)?) {
            internal.push(f);
            continue;
        }
        let pairs = face_pairs(&f)?;
        for pair in &pairs {
            *count.entry(*pair).or_default() += 1;
        }
        skin.push((f, pairs));
    }
    internal.extend(
        skin.into_iter()
            .filter(|(_, pairs)| pairs.iter().all(|p| count[p] > 2))
            .map(|(f, _)| f),
    );
    Ok(internal)
}
fn positions<'p>(vs: &[Obj<'p>]) -> PyResult<Vec<Obj<'p>>> {
    vs.iter().map(|v| v.getattr("position")).collect()
}
#[pyfunction]
fn _copy_faces<'p>(faces: &Obj<'p>) -> PyResult<(Obj<'p>, Bound<'p, PyDict>)> {
    let py = faces.py();
    let mesh = py.import("built_core.mesh")?.call_method0("Mesh")?;
    let orig = PyDict::new(py);
    for f in faces.try_iter()? {
        let f = f?;
        let mut rings = corners(&f)?
            .iter()
            .map(|r| positions(r))
            .collect::<PyResult<Vec<_>>>()?;
        let outer = rings.remove(0);
        let g = mesh.call_method1("add_face", (outer, rings))?;
        g.setattr("interior", f.getattr("interior")?)?;
        orig.set_item(g.getattr("key")?, f)?;
    }
    Ok((mesh, orig))
}
#[pyfunction]
fn _signed_volume(faces: &Obj<'_>) -> PyResult<f64> {
    let mut vol = 0.;
    for f in faces.try_iter()? {
        let f = f?;
        if !_live(&f)? {
            continue;
        }
        for tri in f.call_method0("triangulate")?.try_iter()? {
            let tri = tri?;
            vol +=
                xyz(&tri.get_item(0)?)?.dot(xyz(&tri.get_item(1)?)?.cross(xyz(&tri.get_item(2)?)?));
        }
    }
    Ok(vol / 6.)
}
#[pyfunction]
fn reversed_faces<'p>(mesh: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let py = mesh.py();
    let orient = py.import("built_core.orient")?;
    if !orient.call_method1("is_closed", (mesh,))?.is_truthy()? {
        return Ok(vec![]);
    }
    let api = py.import("built_core")?;
    let all = rows(&mesh.getattr("faces")?)?;
    let labels = api
        .call_method1("solid_face_components", (mesh, &all))?
        .extract::<Vec<usize>>()?;
    let mut pieces = Vec::<Vec<Obj<'_>>>::new();
    let mut indexes = HashMap::new();
    for (label, f) in labels.into_iter().zip(all) {
        let i = *indexes.entry(label).or_insert_with(|| {
            pieces.push(vec![]);
            pieces.len() - 1
        });
        pieces[i].push(f);
    }
    let suspect = PyList::empty(py);
    for piece in pieces {
        let skin = PyList::empty(py);
        for f in &piece {
            if _live(f)? {
                skin.append(f)?;
            }
        }
        if api
            .call_method1("solid_incoherent_edges", (mesh, &skin))?
            .is_truthy()?
            || _signed_volume(skin.as_any())? <= 0.
        {
            for f in piece {
                suspect.append(f)?;
            }
        }
    }
    if suspect.is_empty() {
        return Ok(vec![]);
    }
    let (copy, origin) = _copy_faces(suspect.as_any())?;
    orient
        .call_method1("orient_outward", (copy,))?
        .try_iter()?
        .map(|g| origin.as_any().get_item(g?.getattr("key")?))
        .collect()
}
#[pyfunction]
fn inspect_mesh<'p>(mesh: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let py = mesh.py();
    let edges = mesh.getattr("edges")?;
    let faces = mesh.getattr("faces")?;
    let store = py.import("built_core.store_mesh")?;
    let mut defects: [Vec<Obj<'_>>; 3] = Default::default();
    if mesh.is_instance(&store.getattr("StoreMesh")?)? {
        let keys = mesh
            .getattr("_store")?
            .call_method0("snapshot")?
            .call_method0("edge_defects")?;
        let edge = store.getattr("EdgeView")?;
        for (i, out) in defects.iter_mut().enumerate() {
            for k in keys.get_item(i)?.try_iter()? {
                out.push(edge.call1((mesh, k?))?);
            }
        }
    } else {
        for e in edges.try_iter()? {
            let e = e?;
            let n = repair::skin_count(&e)?;
            match n {
                0 => defects[0].push(e),
                1 => defects[1].push(e),
                2 => (),
                _ => defects[2].push(e),
            }
        }
    }
    let [stray, open, over] = defects;
    let watertight = stray.is_empty() && open.is_empty() && over.is_empty() && faces.is_truthy()?;
    let turned = reversed_faces(mesh)?;
    let r = PyDict::new(py);
    let volume = if watertight && turned.is_empty() {
        let skin = PyList::empty(py);
        for f in faces.try_iter()? {
            let f = f?;
            if _live(&f)? {
                skin.append(f)?;
            }
        }
        py.import("core.bim")?
            .call_method1("face_set_volume", (skin,))?
    } else {
        py.None().into_bound(py)
    };
    r.set_item("volume", volume)?;
    r.set_item("watertight", watertight)?;
    r.set_item(
        "internal",
        if over.is_empty() {
            vec![]
        } else {
            internal_faces(mesh)?
        },
    )?;
    r.set_item("reversed", turned)?;
    r.set_item("faces", faces.len()?)?;
    r.set_item("edges", edges.len()?)?;
    r.set_item("open", open)?;
    r.set_item("stray", stray)?;
    r.set_item("over", over)?;
    Ok(r)
}
#[pyfunction]
fn is_sound(report: &Obj<'_>) -> PyResult<bool> {
    Ok(report.get_item("watertight")?.is_truthy()? && !report.get_item("reversed")?.is_truthy()?)
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(_live, m)?,
        wrap_pyfunction!(_steps, m)?,
        wrap_pyfunction!(_step_key, m)?,
        wrap_pyfunction!(_face_key, m)?,
        wrap_pyfunction!(internal_faces, m)?,
        wrap_pyfunction!(_copy_faces, m)?,
        wrap_pyfunction!(reversed_faces, m)?,
        wrap_pyfunction!(_signed_volume, m)?,
        wrap_pyfunction!(inspect_mesh, m)?,
        wrap_pyfunction!(is_sound, m)?,
    ] {
        m.add_function(f)?;
    }
    problems::install(m)?;
    repair::install(m)?;
    command::install(m)
}

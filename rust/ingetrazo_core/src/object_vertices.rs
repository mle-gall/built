// SPDX-License-Identifier: GPL-3.0-or-later
//! Weld object-backed batches in the same creation order as store imports.
//! Registry entries can alias the same vertex or name a retired handle.
use crate::store::cow::{FxBuild, FxHashMap};
use crate::{
    math::{xyz, Vec3},
    store::py::PyMeshStore,
};
use glam::DVec3;
use numpy::{IntoPyArray, PyArray1, PyReadonlyArray2};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type FxHashSet<K> = std::collections::HashSet<K, FxBuild>;

type Obj<'py> = Bound<'py, PyAny>;
type Cell = (i64, i64, i64);

fn neighbours((x, y, z): Cell) -> impl Iterator<Item = Cell> {
    (-1..=1).flat_map(move |dx| {
        (-1..=1).flat_map(move |dy| {
            (-1..=1).filter_map(move |dz| {
                if (dx, dy, dz) == (0, 0, 0) {
                    return None;
                }
                Some((x.checked_add(dx)?, y.checked_add(dy)?, z.checked_add(dz)?))
            })
        })
    })
}

#[pyfunction]
pub(crate) fn weld_candidates<'py>(
    py: Python<'py>,
    keys: PyReadonlyArray2<'py, i64>,
) -> PyResult<Bound<'py, PyArray1<bool>>> {
    let rows = keys.as_array();
    if rows.ncols() != 3 {
        return Err(PyValueError::new_err("expected three cell coordinates"));
    }
    let cells: Vec<_> = rows
        .rows()
        .into_iter()
        .map(|r| (r[0], r[1], r[2]))
        .collect();
    let set: FxHashSet<_> = cells.iter().copied().collect();
    Ok(cells
        .iter()
        .map(|&c| neighbours(c).any(|n| set.contains(&n)))
        .collect::<Vec<_>>()
        .into_pyarray(py))
}

#[pyfunction]
pub(crate) fn probe_vertex<'py>(
    reg: &Bound<'py, PyDict>,
    cell: &Obj<'py>,
    point: &Obj<'py>,
    tol2: f64,
) -> PyResult<Option<Obj<'py>>> {
    let p = xyz(point)?;
    let hit = |v: &Obj<'py>| -> PyResult<bool> {
        Ok(xyz(&v.getattr("position")?)?.distance_squared(p) < tol2)
    };
    if let Ok(c) = cell.extract::<Cell>() {
        if ![c.0, c.1, c.2]
            .iter()
            .any(|&x| x == i64::MAX || x == i64::MIN)
        {
            for near in neighbours(c) {
                if let Some(v) = reg.get_item(near)? {
                    if hit(&v)? {
                        return Ok(Some(v));
                    }
                }
            }
            return Ok(None);
        }
    }
    // Scalar cells are Python integers, including coordinates outside int64.
    let axes: Vec<_> = (0..3)
        .map(|i| {
            let k = cell.get_item(i)?;
            Ok([k.sub(1)?, k.clone(), k.add(1)?])
        })
        .collect::<PyResult<_>>()?;
    for (i, x) in axes[0].iter().enumerate() {
        for (j, y) in axes[1].iter().enumerate() {
            for (k, z) in axes[2].iter().enumerate() {
                if (i, j, k) == (1, 1, 1) {
                    continue;
                }
                if let Some(v) = reg.get_item(PyTuple::new(reg.py(), [x, y, z])?)? {
                    if hit(&v)? {
                        return Ok(Some(v));
                    }
                }
            }
        }
    }
    Ok(None)
}

fn batch_cell(x: f64, scale: f64) -> i64 {
    let n = (x * scale).round_ties_even();
    // Match numpy.round(...).astype(int64), including its out-of-range sentinel.
    if !(-9223372036854775808.0..9223372036854775808.0).contains(&n) {
        i64::MIN
    } else {
        n as i64
    }
}

#[pyfunction]
#[pyo3(signature=(mesh, pos, model, store, scale, tol2))]
pub(crate) fn bulk_weld<'py>(
    mesh: &Obj<'py>,
    pos: PyReadonlyArray2<'py, f64>,
    model: PyReadonlyArray2<'py, f64>,
    store: Option<&Obj<'py>>,
    scale: f64,
    tol2: f64,
) -> PyResult<(Bound<'py, PyList>, Bound<'py, PyArray1<i64>>)> {
    let py = mesh.py();
    let p = pos.as_array();
    let def = model.as_array();
    if p.ncols() != 3 || def.dim() != p.dim() {
        return Err(PyValueError::new_err(
            "expected matching (P,3) position arrays",
        ));
    }
    let obj_reg = if store.is_none() {
        Some(mesh.getattr("_registry")?.cast_into::<PyDict>()?)
    } else {
        None
    };
    let mut first = FxHashMap::default();
    let cells: Vec<Cell> = def
        .rows()
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let c = (
                batch_cell(r[0], scale),
                batch_cell(r[1], scale),
                batch_cell(r[2], scale),
            );
            first.entry(c).or_insert(i);
            c
        })
        .collect();
    let mut candidate = FxHashSet::default();
    {
        let st = store.map(PyMeshStore::read).transpose()?;
        let occupied = |c: Cell| -> PyResult<bool> {
            match (&st, &obj_reg) {
                (Some(s), _) => Ok(s.inner.state.registry.contains(&c)),
                (_, Some(reg)) if !reg.is_empty() => reg.contains(c),
                (_, Some(_)) => Ok(false),
                _ => unreachable!(),
            }
        };
        for &c in first.keys() {
            if occupied(c)? {
                candidate.insert(c);
                continue;
            }
            for n in neighbours(c) {
                if first.contains_key(&n) || occupied(n)? {
                    candidate.insert(c);
                    break;
                }
            }
        }
    }
    let vertices = mesh.getattr("vertices")?;
    let new = py.import("core.mesh")?.getattr("Vertex")?;
    let keys = mesh.getattr("_keys")?;
    let out = PyList::empty(py);
    let mut by_key = FxHashMap::<i128, i64>::default();
    let mut isolated = FxHashMap::<Cell, i64>::default();
    let mut ids = vec![0; p.nrows()];
    for candidates in [false, true] {
        for (i, &c) in cells.iter().enumerate() {
            if candidate.contains(&c) != candidates {
                continue;
            }
            if !candidates && first[&c] != i {
                ids[i] = isolated[&c];
                continue;
            }
            let point = [p[[i, 0]], p[[i, 1]], p[[i, 2]]];
            let v = if let Some(reg) = &obj_reg {
                let pos = Bound::new(py, Vec3::from(DVec3::from_array(point)))?.into_any();
                let existing = if candidates {
                    match reg.get_item(c)? {
                        Some(v) => Some(v),
                        None => probe_vertex(reg, &c.into_pyobject(py)?.into_any(), &pos, tol2)?,
                    }
                } else {
                    None
                };
                if let Some(v) = existing {
                    v
                } else {
                    let kw = PyDict::new(py);
                    kw.set_item("key", keys.call_method0("__next__")?)?;
                    let v = new.call((&pos,), Some(&kw))?;
                    vertices.call_method1("append", (&v,))?;
                    reg.set_item(c, &v)?;
                    v
                }
            } else {
                mesh.call_method1("vertex", ((point[0], point[1], point[2]),))?
            };
            let key = v.getattr("key")?.extract::<i128>()?;
            let id = if let Some(&id) = by_key.get(&key) {
                id
            } else {
                let id = out.len() as i64;
                out.append(&v)?;
                by_key.insert(key, id);
                id
            };
            ids[i] = id;
            if !candidates {
                isolated.insert(c, id);
            }
        }
    }
    Ok((out, ids.into_pyarray(py)))
}

#[pyfunction]
pub(crate) fn vertex<'py>(
    mesh: &Obj<'py>,
    pos: &Obj<'py>,
    cell: &Obj<'py>,
    tol2: f64,
    create: bool,
) -> PyResult<Option<Obj<'py>>> {
    let reg = mesh.getattr("_registry")?.cast_into::<PyDict>()?;
    if let Some(v) = reg.get_item(cell)? {
        return Ok(Some(v));
    }
    if let Some(v) = probe_vertex(&reg, cell, pos, tol2)? {
        return Ok(Some(v));
    }
    if !create {
        return Ok(None);
    }
    let py = mesh.py();
    let kw = PyDict::new(py);
    kw.set_item("key", mesh.getattr("_keys")?.call_method0("__next__")?)?;
    let v = py
        .import("core.mesh")?
        .getattr("Vertex")?
        .call((pos,), Some(&kw))?;
    reg.set_item(cell, &v)?;
    mesh.getattr("vertices")?.call_method1("append", (&v,))?;
    Ok(Some(v))
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Feed the box kernel directly from the packed scene. Only a prototype on
//! the rectangle's boundary needs its indexed positions expanded.
use crate::pick::{box_rows, Projector, Proto};
use numpy::{PyReadonlyArray1, PyReadonlyArray2, PyReadonlyArray3, PyUntypedArrayMethods};
use pyo3::{exceptions::PyValueError, prelude::*};
use std::collections::{HashMap, HashSet};

struct Commands {
    ranges: Vec<(usize, usize, i32)>,
}
impl Commands {
    fn read(p: &Bound<'_, PyAny>, field: &str) -> PyResult<Self> {
        let cmd = p.getattr(field)?;
        let first = cmd.get_item("first")?.extract::<PyReadonlyArray1<u32>>()?;
        let count = cmd.get_item("count")?.extract::<PyReadonlyArray1<u32>>()?;
        let base = cmd.get_item("base")?.extract::<PyReadonlyArray1<i32>>()?;
        Ok(Self {
            ranges: first
                .as_array()
                .iter()
                .zip(count.as_array())
                .zip(base.as_array())
                .map(|((&i, &n), &b)| (i as usize, n as usize, b))
                .collect(),
        })
    }
}
type Catch = (HashSet<usize>, HashSet<usize>);
#[pyfunction]
pub fn box_from_pack(
    view: &Bound<'_, PyAny>,
    candidates: Vec<Bound<'_, PyAny>>,
    rect: [f64; 4],
    crossing: bool,
) -> PyResult<Option<Catch>> {
    if candidates.is_empty()
        || view
            .getattr_opt("_group_chunk")?
            .is_some_and(|c| !c.is_none())
    {
        return Ok(None);
    }
    let Some(p) = view
        .getattr_opt("view")?
        .map(|v| v.getattr_opt("packed"))
        .transpose()?
        .flatten()
        .filter(|p| !p.is_none())
    else {
        return Ok(None);
    };
    let Some(groups) = p.getattr_opt("groups")?.filter(|g| !g.is_none()) else {
        return Ok(None);
    };
    let Some(camera) = view.getattr_opt("_camera_px")? else {
        return Ok(None);
    };
    let Some(instances) = p
        .getattr_opt("instances")?
        .filter(|i| i.hasattr("dtype").unwrap_or(false))
    else {
        return Ok(None);
    };
    let world = instances
        .get_item("world")?
        .extract::<PyReadonlyArray3<f32>>()?;
    let info = instances
        .get_item("info")?
        .extract::<PyReadonlyArray2<u32>>()?;
    let bounds = p.getattr("bounds")?.extract::<PyReadonlyArray3<f32>>()?;
    let n = world.shape()[0];
    let count = bounds.shape()[0];
    if n != groups.len()? {
        return Ok(None);
    }
    if world.shape() != [n, 4, 4] || info.shape() != [n, 4] || bounds.shape() != [count, 2, 4] {
        return Err(PyValueError::new_err("invalid packed instance layout"));
    }
    let mut owners = HashMap::new();
    for (i, g) in candidates.iter().enumerate() {
        let mut stack = vec![g.clone()];
        while let Some(node) = stack.pop() {
            let id = node.as_ptr() as usize;
            if owners.insert(id, i) == Some(i) {
                continue;
            }
            if let Some(children) = node.getattr_opt("children")?.filter(|c| !c.is_none()) {
                stack.extend(children.try_iter()?.collect::<PyResult<Vec<_>>>()?);
            }
        }
    }
    let matrices = world.as_array();
    let info = info.as_array();
    let bounds = bounds.as_array();
    let (mut worlds, mut codes, mut owner, mut drawn) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut tested = HashSet::new();
    for (i, g) in groups.try_iter()?.enumerate() {
        let Some(&o) = owners.get(&(g?.as_ptr() as usize)) else {
            continue;
        };
        tested.insert(candidates[o].as_ptr() as usize);
        let code = info[[i, 0]];
        if code as usize >= count {
            return Err(PyValueError::new_err("invalid packed prototype code"));
        }
        worlds.push(std::array::from_fn(|j| matrices[[i, j / 4, j % 4]] as f64));
        codes.push(code as i64);
        owner.push(o as i64);
        drawn.push(info[[i, 2]] & 1 == 0);
    }
    let bound = |which| {
        (0..count)
            .map(|i| std::array::from_fn(|j| bounds[[i, which, j]] as f64))
            .collect::<Vec<_>>()
    };
    let pos = p
        .getattr("vertices")?
        .get_item("pos")?
        .extract::<PyReadonlyArray2<f32>>()?;
    let ix = p.getattr("indices")?.extract::<PyReadonlyArray1<u32>>()?;
    let pos = pos.as_array();
    let ix = ix.as_array();
    let faces = Commands::read(&p, "commands")?;
    let edges = Commands::read(&p, "edge_commands")?;
    if pos.ncols() != 3 || faces.ranges.len() != count || edges.ranges.len() != count {
        return Err(PyValueError::new_err("invalid prototype ranges"));
    }
    let expand = |cmd: &Commands, code: usize| -> Option<Vec<[f64; 3]>> {
        let (first, n, base) = cmd.ranges[code];
        let mut points = Vec::new();
        for j in first..first.checked_add(n)? {
            let index = (*ix.get(j)? as i64 + base as i64) as usize;
            if index >= pos.nrows() {
                return None;
            }
            points.push(std::array::from_fn(|k| pos[[index, k]] as f64))
        }
        Some(points)
    };
    let (matrix, width, height): ([f64; 16], f64, f64) = camera.call0()?.extract()?;
    let mut invalid = false;
    let mut fetch = |code| match (expand(&faces, code), expand(&edges, code)) {
        (Some(tris), Some(edges)) => Proto { tris, edges },
        _ => {
            invalid = true;
            Proto {
                tris: vec![],
                edges: vec![],
            }
        }
    };
    let hits = box_rows(
        &worlds,
        &codes,
        &owner,
        &drawn,
        &bound(0),
        &bound(1),
        &mut fetch,
        candidates.len(),
        &Projector::new(&matrix, width, height),
        &rect,
        crossing,
    );
    if invalid {
        return Err(PyValueError::new_err(
            "prototype index outside vertex buffer",
        ));
    }
    Ok(Some((
        hits.iter()
            .zip(candidates)
            .filter_map(|(hit, g)| hit.then_some(g.as_ptr() as usize))
            .collect(),
        tested,
    )))
}

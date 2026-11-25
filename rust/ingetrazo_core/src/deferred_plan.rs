// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered indexing of a deferred block, including all edge-to-face incidences.
use numpy::{IntoPyArray, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::{
    exceptions::{PyIndexError, PyValueError},
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashMap;
type Obj<'py> = Bound<'py, PyAny>;
fn index(i: i64, len: usize) -> PyResult<usize> {
    let k = if i < 0 {
        i.checked_add(len as i64).unwrap_or(-1)
    } else {
        i
    };
    usize::try_from(k)
        .ok()
        .filter(|x| *x < len)
        .ok_or_else(|| PyIndexError::new_err("deferred mesh index out of range"))
}
#[derive(Default)]
struct Edge {
    ends: [i64; 2],
    flags: i64,
    curve: Option<i64>,
    layer: Option<i64>,
    faces: Vec<i64>,
}
fn edge_id(edges: &mut Vec<Edge>, by_pair: &mut HashMap<[i64; 2], usize>, a: i64, b: i64) -> usize {
    *by_pair.entry([a.min(b), a.max(b)]).or_insert_with(|| {
        let i = edges.len();
        edges.push(Edge {
            ends: [a, b],
            ..Edge::default()
        });
        i
    })
}
#[pyfunction]
#[pyo3(signature=(inverse,edges,flags,curves,layers,layer_names,rings,sizes,counts))]
pub(crate) fn deferred_incidence<'py>(
    py: Python<'py>,
    inverse: PyReadonlyArray1<'py, i64>,
    edges: PyReadonlyArray2<'py, i64>,
    flags: Option<PyReadonlyArray1<'py, i64>>,
    curves: Option<PyReadonlyArray1<'py, i64>>,
    layers: Option<PyReadonlyArray1<'py, i64>>,
    layer_names: &Obj<'py>,
    rings: PyReadonlyArray1<'py, i64>,
    sizes: PyReadonlyArray1<'py, i64>,
    counts: PyReadonlyArray1<'py, i64>,
) -> PyResult<Obj<'py>> {
    let ids = inverse.as_slice()?;
    let input = edges.as_array();
    let flags = flags.as_ref().map(|a| a.as_slice()).transpose()?;
    let curves = curves.as_ref().map(|a| a.as_slice()).transpose()?;
    let layers = layers.as_ref().map(|a| a.as_slice()).transpose()?;
    let rings = rings.as_slice()?;
    let sizes = sizes.as_slice()?;
    let counts = counts.as_slice()?;
    if input.shape()[1] != 2 {
        return Err(PyValueError::new_err("edges must have two endpoints"));
    }
    let (all, corners) = py.detach(|| -> PyResult<_> {
        let mut all = Vec::new();
        let mut by_pair = HashMap::new();
        for (i, row) in input.outer_iter().enumerate() {
            let a = ids[index(row[0], ids.len())?];
            let b = ids[index(row[1], ids.len())?];
            if a == b {
                continue;
            }
            let e = edge_id(&mut all, &mut by_pair, a, b);
            if let Some(f) = flags {
                all[e].flags |= *f
                    .get(i)
                    .ok_or_else(|| PyIndexError::new_err("missing edge flags"))?;
                for (column, field) in [(curves, true), (layers, false)] {
                    if let Some(column) = column {
                        let v = *column
                            .get(i)
                            .ok_or_else(|| PyIndexError::new_err("missing edge metadata"))?;
                        if v >= 0 {
                            if field {
                                all[e].curve = Some(v)
                            } else {
                                all[e].layer = Some(v)
                            }
                        }
                    }
                }
            }
        }
        let corners = rings
            .iter()
            .map(|&k| Ok(ids[index(k, ids.len())?]))
            .collect::<PyResult<Vec<_>>>()?;
        let mut ring = 0usize;
        let mut corner = 0usize;
        for (face, &nr) in counts.iter().enumerate() {
            let nr =
                usize::try_from(nr).map_err(|_| PyValueError::new_err("negative ring count"))?;
            let next = ring
                .checked_add(nr)
                .filter(|n| *n <= sizes.len())
                .ok_or_else(|| PyValueError::new_err("invalid ring count"))?;
            for &n in &sizes[ring..next] {
                let n =
                    usize::try_from(n).map_err(|_| PyValueError::new_err("negative ring size"))?;
                let end = corner
                    .checked_add(n)
                    .filter(|n| *n <= corners.len())
                    .ok_or_else(|| PyValueError::new_err("invalid ring size"))?;
                let pts = &corners[corner..end];
                for j in 0..n {
                    let e = edge_id(&mut all, &mut by_pair, pts[j], pts[(j + 1) % n]);
                    all[e].faces.push(face as i64);
                }
                corner = end;
            }
            ring = next;
        }
        if ring != sizes.len() || corner != corners.len() {
            return Err(PyValueError::new_err("unconsumed ring data"));
        }
        Ok((all, corners))
    })?;
    let n = all.len();
    let mut ends = Vec::with_capacity(n * 2);
    let mut soft = Vec::with_capacity(n);
    let mut hidden = Vec::with_capacity(n);
    let mut flat = Vec::with_capacity(n);
    let curve = PyList::empty(py);
    let layer = PyList::empty(py);
    let mut start = vec![0i64];
    let mut incident = Vec::new();
    for e in all {
        ends.extend(e.ends);
        soft.push(e.flags & 1 != 0);
        hidden.push(e.flags & 2 != 0);
        flat.push(e.flags & 4 != 0);
        curve.append(e.curve)?;
        layer.append(e.layer.map(|i| layer_names.get_item(i)).transpose()?)?;
        incident.extend(e.faces);
        start.push(incident.len() as i64);
    }
    let out = PyDict::new(py);
    out.set_item("edges", ends.into_pyarray(py).reshape([n, 2])?)?;
    out.set_item("soft", soft.into_pyarray(py))?;
    out.set_item("hidden", hidden.into_pyarray(py))?;
    out.set_item("flat", flat.into_pyarray(py))?;
    out.set_item("curve", curve)?;
    out.set_item("layer", layer)?;
    out.set_item("corners", corners.into_pyarray(py))?;
    out.set_item("ef_start", start.into_pyarray(py))?;
    out.set_item("ef_face", incident.into_pyarray(py))?;
    Ok(out.into_any())
}

/// Net polygon areas without constructing a store or exposing vertex views.
#[pyfunction]
pub(crate) fn planned_face_areas<'py>(
    py: Python<'py>,
    pos: PyReadonlyArray2<'py, f64>,
    corners: PyReadonlyArray1<'py, i64>,
    sizes: PyReadonlyArray1<'py, i64>,
    counts: PyReadonlyArray1<'py, i64>,
) -> PyResult<Bound<'py, numpy::PyArray1<f64>>> {
    let pos = pos.as_array();
    let corners = corners.as_array();
    let sizes = sizes.as_array();
    let counts = counts.as_array();
    if pos.shape()[1] != 3 {
        return Err(PyValueError::new_err("positions require three coordinates"));
    }
    let mut areas = Vec::with_capacity(counts.len());
    let mut ring = 0;
    let mut corner = 0;
    let mut pts = Vec::new();
    for &count in counts.iter() {
        let count =
            usize::try_from(count).map_err(|_| PyValueError::new_err("negative ring count"))?;
        let mut area = 0.;
        let mut valid = false;
        for j in 0..count {
            let n = *sizes
                .get(ring)
                .ok_or_else(|| PyValueError::new_err("missing ring size"))?;
            let n = usize::try_from(n).map_err(|_| PyValueError::new_err("negative ring size"))?;
            ring += 1;
            pts.clear();
            for _ in 0..n {
                let ix = *corners
                    .get(corner)
                    .ok_or_else(|| PyValueError::new_err("missing ring corner"))?;
                let p = pos.row(index(ix, pos.nrows())?);
                pts.push(glam::DVec3::new(p[0], p[1], p[2]));
                corner += 1;
            }
            let value = if n >= 3 {
                0.5 * crate::polygon::raw_newell(&pts).length()
            } else {
                0.
            };
            if j == 0 {
                valid = n >= 3;
                area = value
            } else {
                area -= value
            }
        }
        areas.push(if valid {
            if area < 0. {
                0.
            } else {
                area
            }
        } else {
            0.
        });
    }
    Ok(areas.into_pyarray(py))
}

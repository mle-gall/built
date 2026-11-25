// SPDX-License-Identifier: GPL-3.0-or-later
use super::Obj;
use crate::math::Vec3;
use glam::DVec3;
use numpy::PyReadonlyArray2;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyList};

#[pyclass]
struct SurveyIndex {
    min: [f64; 2],
    cell: f64,
    nx: usize,
    ny: usize,
    starts: Vec<usize>,
    ids: Vec<usize>,
}
fn vertex(mesh: &Obj<'_>, index: usize) -> PyResult<DVec3> {
    let row = mesh.get_item(index)?;
    Ok(DVec3::from_array(row.extract()?))
}
#[pyfunction]
fn photo_index<'p>(py: Python<'p>, mesh: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    if let Some(old) = mesh.getattr_opt("_index")?.filter(|v| !v.is_none()) {
        return Ok(Some(old));
    }
    let vertices = mesh.getattr("vertices")?;
    let triangles = mesh.getattr("triangles")?;
    if vertices.len()? == 0 || triangles.len()? == 0 {
        return Ok(None);
    }
    let points = py
        .import("numpy")?
        .call_method1("asarray", (&vertices, "float64"))?
        .extract::<PyReadonlyArray2<f64>>()?;
    let faces = py
        .import("numpy")?
        .call_method1("asarray", (&triangles, "int64"))?
        .extract::<PyReadonlyArray2<i64>>()?;
    let p = points.as_array();
    let t = faces.as_array();
    if p.ncols() != 3 || t.ncols() != 3 {
        return Err(PyValueError::new_err("invalid survey geometry"));
    }
    let mut ranges = Vec::with_capacity(t.nrows());
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for r in t.rows() {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for &id in r {
            let i = usize::try_from(id)
                .ok()
                .filter(|&i| i < p.nrows())
                .ok_or_else(|| PyValueError::new_err("invalid survey triangle"))?;
            for j in 0..2 {
                let v = p[[i, j]];
                if !v.is_finite() {
                    return Err(PyValueError::new_err("non-finite survey point"));
                }
                lo[j] = lo[j].min(v);
                hi[j] = hi[j].max(v)
            }
        }
        for j in 0..2 {
            low[j] = low[j].min(lo[j]);
            high[j] = high[j].max(hi[j])
        }
        ranges.push((lo, hi));
    }
    let size = [(high[0] - low[0]).max(1e-6), (high[1] - low[1]).max(1e-6)];
    let target = (t.nrows() as f64 / 4.).sqrt().floor().clamp(1., 2048.);
    let cell = size[0].max(size[1]) / target;
    let nx = (size[0] / cell) as usize + 1;
    let ny = (size[1] / cell) as usize + 1;
    let mut spans = Vec::with_capacity(ranges.len());
    let mut starts = vec![0usize; nx * ny + 1];
    let mut total = 0;
    for (lo, hi) in ranges {
        let bounds = |p: [f64; 2]| {
            (
                ((p[0] - low[0]) / cell).floor() as usize,
                ((p[1] - low[1]) / cell).floor() as usize,
            )
        };
        let (x0, y0) = bounds(lo);
        let (x1, y1) = bounds(hi);
        let x1 = x1.min(nx - 1);
        let y1 = y1.min(ny - 1);
        total += (x1 - x0 + 1) * (y1 - y0 + 1);
        if total > 16_000_000 {
            return Ok(None);
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                starts[y * nx + x + 1] += 1
            }
        }
        spans.push((x0, y0, x1, y1));
    }
    for i in 1..starts.len() {
        starts[i] += starts[i - 1]
    }
    let mut next = starts.clone();
    let mut ids = vec![0; total];
    for (id, (x0, y0, x1, y1)) in spans.into_iter().enumerate() {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let index = y * nx + x;
                ids[next[index]] = id;
                next[index] += 1
            }
        }
    }
    let index = Py::new(
        py,
        SurveyIndex {
            min: low,
            cell,
            nx,
            ny,
            starts,
            ids,
        },
    )?
    .into_bound(py)
    .into_any();
    mesh.setattr("_index", &index)?;
    Ok(Some(index))
}
#[pyfunction]
fn photo_height(py: Python<'_>, mesh: &Obj<'_>, x: f64, y: f64) -> PyResult<Option<f64>> {
    let Some(index) = photo_index(py, mesh)? else {
        return Ok(None);
    };
    let grid = index.extract::<PyRef<SurveyIndex>>()?;
    let xx = ((x - grid.min[0]) / grid.cell).floor();
    let yy = ((y - grid.min[1]) / grid.cell).floor();
    if !(xx >= 0. && yy >= 0. && xx < grid.nx as f64 && yy < grid.ny as f64) {
        return Ok(None);
    }
    let cell = yy as usize * grid.nx + xx as usize;
    let vertices = mesh.getattr("vertices")?;
    let triangles = mesh.getattr("triangles")?;
    let coords = vertices.extract::<PyReadonlyArray2<f32>>().ok();
    let faces = triangles.extract::<PyReadonlyArray2<u32>>().ok();
    let mut highest = None;
    for &id in &grid.ids[grid.starts[cell]..grid.starts[cell + 1]] {
        let ids = if let Some(f) = &faces {
            let f = f.as_array();
            if id >= f.nrows() || f.ncols() != 3 {
                return Err(PyValueError::new_err("invalid survey triangle"));
            }
            [
                f[[id, 0]] as usize,
                f[[id, 1]] as usize,
                f[[id, 2]] as usize,
            ]
        } else {
            triangles.get_item(id)?.extract::<[usize; 3]>()?
        };
        let pt = |i: usize| -> PyResult<DVec3> {
            if let Some(p) = &coords {
                let a = p.as_array();
                if i >= a.nrows() || a.ncols() != 3 {
                    return Err(PyValueError::new_err("invalid survey vertex"));
                }
                Ok(DVec3::new(
                    a[[i, 0]] as f64,
                    a[[i, 1]] as f64,
                    a[[i, 2]] as f64,
                ))
            } else {
                vertex(&vertices, i)
            }
        };
        let a = pt(ids[0])?;
        let ab = pt(ids[1])? - a;
        let ac = pt(ids[2])? - a;
        let det = ab.x * ac.y - ac.x * ab.y;
        if det.abs() <= 1e-12 {
            continue;
        }
        let u = ((x - a.x) * ac.y - ac.x * (y - a.y)) / det;
        let v = (ab.x * (y - a.y) - (x - a.x) * ab.y) / det;
        if u >= -1e-9 && v >= -1e-9 && u + v <= 1. + 1e-9 {
            let z = a.z + u * ab.z + v * ac.z;
            highest = Some(highest.map_or(z, |old: f64| old.max(z)))
        }
    }
    Ok(highest)
}
#[pyfunction]
fn photo_bounds(mesh: &Obj<'_>) -> PyResult<(Option<Vec3>, Option<Vec3>)> {
    let v = mesh.getattr("vertices")?;
    if v.len()? == 0 {
        return Ok((None, None));
    }
    let point = |name| -> PyResult<_> {
        Ok(Some(Vec3::from(DVec3::from_array(
            v.call_method1(name, (0,))?.extract()?,
        ))))
    };
    Ok((point("min")?, point("max")?))
}
#[pyfunction]
fn vertical_origin(mesh: &Obj<'_>) -> PyResult<Option<f64>> {
    if mesh.is_none() || mesh.getattr("triangles")?.len()? == 0 {
        return Ok(None);
    }
    let py = mesh.py();
    let all = py.get_type::<pyo3::types::PySlice>().call1((py.None(),))?;
    let z = mesh.getattr("vertices")?.get_item((all, 2))?;
    Ok(Some(
        py.import("numpy")?
            .call_method1("percentile", (z, 1.))?
            .extract()?,
    ))
}
#[pyfunction]
fn photo_missing<'p>(py: Python<'p>, mesh: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let l = PyList::empty(py);
    for m in mesh.getattr("materials")?.try_iter()? {
        let p = m?.getattr("texture")?;
        if !p.is_none() && !p.call_method0("is_file")?.is_truthy()? {
            l.append(p)?
        }
    }
    Ok(l)
}
#[pyfunction]
fn photo_sample(s: &Obj<'_>, point: &Obj<'_>) -> PyResult<Option<f64>> {
    let p = crate::math::xyz(point)?;
    let z = s
        .getattr("mesh")?
        .call_method1("height_at", (p.x, p.y))?
        .extract::<Option<f64>>()?;
    let alt = s
        .getattr("datum")?
        .getattr_opt("alt")?
        .filter(|v| !v.is_none())
        .map(|v| v.extract::<f64>())
        .transpose()?
        .unwrap_or(0.);
    Ok(z.map(|v| v + alt))
}
#[pyfunction]
fn photo_geodetic(s: &Obj<'_>, lat: f64, lon: f64) -> PyResult<Option<f64>> {
    photo_sample(
        s,
        &s.getattr("datum")?
            .call_method1("geodetic_to_local", (lat, lon))?,
    )
}
#[pyfunction]
fn photo_hemisphere(anchor: &Obj<'_>) -> PyResult<&'static str> {
    Ok(if anchor.getattr("northern")?.is_truthy()? {
        "N"
    } else {
        "S"
    })
}
#[pyfunction]
fn photo_invalidate_index(mesh: &Obj<'_>) -> PyResult<()> {
    mesh.setattr("_index", mesh.py().None())
}
#[pyfunction]
fn photo_vertex_count<'p>(mesh: &Obj<'p>) -> PyResult<Obj<'p>> {
    mesh.getattr("vertices")?.getattr("shape")?.get_item(0)
}
#[pyfunction]
fn photo_triangle_count<'p>(mesh: &Obj<'p>) -> PyResult<Obj<'p>> {
    mesh.getattr("triangles")?.getattr("shape")?.get_item(0)
}
#[pyfunction]
fn photo_is_placed(mesh: &Obj<'_>) -> PyResult<bool> {
    Ok(!mesh.getattr("anchor")?.is_none())
}
/// An imported survey is already resident for every requested area.
#[pyfunction]
fn photo_ensure_area(
    sampler: &Obj<'_>,
    lat_s: &Obj<'_>,
    lon_w: &Obj<'_>,
    lat_n: &Obj<'_>,
    lon_e: &Obj<'_>,
) {
    let _ = (sampler, lat_s, lon_w, lat_n, lon_e);
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(photo_hemisphere, m)?)?;
    m.add_function(wrap_pyfunction!(photo_invalidate_index, m)?)?;
    m.add_function(wrap_pyfunction!(photo_vertex_count, m)?)?;
    m.add_function(wrap_pyfunction!(photo_triangle_count, m)?)?;
    m.add_function(wrap_pyfunction!(photo_is_placed, m)?)?;
    m.add_function(wrap_pyfunction!(photo_ensure_area, m)?)?;
    m.add_function(wrap_pyfunction!(photo_index, m)?)?;
    m.add_function(wrap_pyfunction!(photo_height, m)?)?;
    m.add_function(wrap_pyfunction!(photo_bounds, m)?)?;
    m.add_function(wrap_pyfunction!(photo_missing, m)?)?;
    m.add_function(wrap_pyfunction!(photo_sample, m)?)?;
    m.add_function(wrap_pyfunction!(photo_geodetic, m)?)?;
    m.add_function(wrap_pyfunction!(vertical_origin, m)?)?;
    Ok(())
}

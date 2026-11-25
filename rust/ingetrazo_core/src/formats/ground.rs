// SPDX-License-Identifier: GPL-3.0-or-later
//! Terrain grid generation, surface fitting, and elevation interpolation.
use super::{
    paths::points,
    tiles::{deg2num, tiles_covering},
    Obj,
};
use crate::math::{xyz, Vec3};
use glam::{DMat3, DVec2, DVec3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashMap;
fn pt(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::from(DVec3::new(x, y, z))
}
#[pyfunction]
fn ground_reference<'p>(sampler: &Obj<'p>, datum: &Obj<'p>) -> PyResult<Obj<'p>> {
    sampler.call_method1(
        "elevation_at",
        (datum.getattr("lat")?, datum.getattr("lon")?),
    )
}
fn plane(pts: &[DVec3], zs: &[f64]) -> DVec3 {
    let mut a = DMat3::ZERO;
    let mut b = DVec3::ZERO;
    for (p, &z) in pts.iter().zip(zs) {
        let p = DVec3::new(p.x, p.y, 1.);
        a += DMat3::from_cols(p * p.x, p * p.y, p);
        b += p * z
    }
    let det = a.determinant();
    if det.abs() < 1e-12 {
        DVec3::new(
            0.,
            0.,
            if pts.is_empty() {
                0.
            } else {
                b.z / pts.len() as f64
            },
        )
    } else {
        a.inverse() * b
    }
}
#[pyfunction]
fn fit_plane(pts: &Obj<'_>, zs: Vec<f64>) -> PyResult<(f64, f64, f64)> {
    let a = plane(&points(pts)?, &zs);
    Ok((a.x, a.y, a.z))
}
#[pyfunction]
fn build_flat_surface<'p>(
    path: &Obj<'p>,
    sampler: &Obj<'p>,
    ground_ref: f64,
) -> PyResult<Option<Obj<'p>>> {
    let py = path.py();
    let p = points(&path.getattr("points")?)?;
    if p.len() < 3 {
        return Ok(None);
    }
    let mut zs = vec![];
    for v in &p {
        let Some(z) = sampler
            .call_method1("elevation_at_local", (pt(v.x, v.y, 0.),))?
            .extract::<Option<f64>>()?
        else {
            return Ok(None);
        };
        zs.push(z)
    }
    let a = plane(&p, &zs);
    let verts = p
        .iter()
        .map(|v| Py::new(py, pt(v.x, v.y, a.x * v.x + a.y * v.y + a.z - ground_ref)))
        .collect::<PyResult<Vec<_>>>()?;
    let out = PyList::empty(py);
    for [i, j, k] in crate::earcut::earcut(&p.iter().map(|v| [v.x, v.y]).collect::<Vec<_>>(), &[]) {
        out.append((verts[i].bind(py), verts[j].bind(py), verts[k].bind(py)))?
    }
    Ok(Some(out.into_any()))
}
fn split([a, b, c]: [DVec2; 3], depth: u32, out: &mut Vec<[DVec2; 3]>) {
    if depth == 0 {
        out.push([a, b, c]);
        return;
    }
    let (ab, bc, ca) = ((a + b) / 2., (b + c) / 2., (c + a) / 2.);
    for t in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
        split(t, depth - 1, out)
    }
}
#[pyfunction]
fn build_draped_surface<'p>(
    path: &Obj<'p>,
    sampler: &Obj<'p>,
    ground_ref: f64,
) -> PyResult<Option<Obj<'p>>> {
    let py = path.py();
    let pts = points(&path.getattr("points")?)?;
    let ring: Vec<_> = pts.iter().map(|v| [v.x, v.y]).collect();
    if ring.len() < 3 {
        return Ok(None);
    }
    let tri = crate::earcut::earcut(&ring, &[]);
    if tri.is_empty() {
        return Ok(None);
    }
    let mut max: f64 = 0.;
    let mut base = vec![];
    for ids in tri {
        let [a, b, c] = ids.map(|i| DVec2::from_array(ring[i]));
        max = max.max(a.distance(b)).max(b.distance(c)).max(c.distance(a));
        base.push([a, b, c]);
    }
    let depth = (max / 12.).log2().ceil().clamp(0., 5.) as u32;
    let mut cache = HashMap::<(i64, i64), Py<Vec3>>::new();
    let mut tris = vec![];
    let out = PyList::empty(py);
    for t in base {
        tris.clear();
        split(t, depth, &mut tris);
        for [a, b, c] in &tris {
            let mut vs = vec![];
            for p in [a, b, c] {
                let key = (
                    (p.x * 1000.).round_ties_even() as i64,
                    (p.y * 1000.).round_ties_even() as i64,
                );
                let v = match cache.entry(key) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let Some(z) = sampler
                            .call_method1("elevation_at_local", (pt(p.x, p.y, 0.),))?
                            .extract::<Option<f64>>()?
                        else {
                            return Ok(None);
                        };
                        e.insert(Py::new(py, pt(p.x, p.y, z - ground_ref))?)
                    }
                };
                vs.push(v.clone_ref(py));
            }
            out.append(pyo3::types::PyTuple::new(py, vs)?)?
        }
    }
    Ok(Some(out.into_any()))
}
#[pyfunction]
fn build_surface<'p>(
    path: &Obj<'p>,
    sampler: &Obj<'p>,
    datum: &Obj<'p>,
) -> PyResult<Option<Obj<'p>>> {
    let mode = path.getattr("surface")?;
    if !mode.is_truthy()? || path.getattr("points")?.len()? < 3 {
        return Ok(None);
    }
    let Some(z) = ground_reference(sampler, datum)?.extract::<Option<f64>>()? else {
        return Ok(None);
    };
    match mode.extract::<&str>()? {
        "flat" => build_flat_surface(path, sampler, z),
        "draped" => build_draped_surface(path, sampler, z),
        _ => Ok(None),
    }
}
#[pyfunction]
fn terrain_bounds(terrain: &Obj<'_>) -> PyResult<(Option<Vec3>, Option<Vec3>)> {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    let vs = terrain.getattr("vertices")?;
    if vs.len()? == 0 {
        return Ok((None, None));
    }
    for v in vs.try_iter()? {
        let v = xyz(&v?)?;
        lo = lo.min(v);
        hi = hi.max(v)
    }
    Ok((Some(lo.into()), Some(hi.into())))
}
#[pyfunction]
fn terrain_height(terrain: &Obj<'_>, x: f64, y: f64) -> PyResult<Option<f64>> {
    let nx = terrain.getattr("nx")?.extract::<usize>()?;
    let ny = terrain.getattr("ny")?.extract::<usize>()?;
    let [l, b, r, t] = terrain.getattr("bbox")?.extract::<[f64; 4]>()?;
    if nx < 2 || ny < 2 || r <= l || t <= b || x < l || x > r || y < b || y > t {
        return Ok(None);
    }
    let fi = (x - l) / (r - l) * (nx - 1) as f64;
    let fj = (t - y) / (t - b) * (ny - 1) as f64;
    let i = (fi as usize).min(nx - 2);
    let j = (fj as usize).min(ny - 2);
    let u = fi - i as f64;
    let v = fj - j as f64;
    let verts = terrain.getattr("vertices")?;
    let z = |k| -> PyResult<f64> { Ok(xyz(&verts.get_item(k)?)?.z) };
    let k = j * nx + i;
    Ok(Some(
        (z(k)? * (1. - u) + z(k + 1)? * u) * (1. - v)
            + (z(k + nx)? * (1. - u) + z(k + nx + 1)? * u) * v,
    ))
}
#[pyfunction(signature=(datum,sampler,ground_ref,bbox,target_cell_m=40.,max_grid=180,zoom=15))]
fn build_terrain<'p>(
    py: Python<'p>,
    datum: &Obj<'p>,
    sampler: &Obj<'p>,
    ground_ref: f64,
    bbox: [f64; 4],
    target_cell_m: f64,
    max_grid: usize,
    zoom: i32,
) -> PyResult<Option<Obj<'p>>> {
    let [x0, y0, x1, y1] = bbox;
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0. || h <= 0. {
        return Ok(None);
    }
    if target_cell_m <= 0. {
        return Err(PyValueError::new_err("invalid terrain grid spacing"));
    }
    let count = |d: f64| {
        (((d / target_cell_m).round_ties_even() as usize).saturating_add(1))
            .min(max_grid)
            .max(2)
    };
    let (nx, ny) = (count(w), count(h));
    let (s, west, n, e) = super::tiles::bbox(datum, bbox)?;
    let tiles = tiles_covering(s, west, n, e, zoom);
    if tiles.is_empty() {
        return Ok(None);
    }
    let tx = tiles.iter().map(|t| t.0).min().unwrap();
    let ty = tiles.iter().map(|t| t.1).min().unwrap();
    let nw = tiles.iter().map(|t| t.0).max().unwrap() - tx + 1;
    let nh = tiles.iter().map(|t| t.1).max().unwrap() - ty + 1;
    let verts = PyList::empty(py);
    let mut uv = vec![];
    let mut tris = vec![];
    let sample = sampler.getattr("elevation_at_local")?;
    let geo = datum.getattr("local_to_geodetic")?;
    for j in 0..ny {
        let y = y1 - h * j as f64 / (ny - 1) as f64;
        for i in 0..nx {
            let x = x0 + w * i as f64 / (nx - 1) as f64;
            let p = Py::new(py, pt(x, y, 0.))?;
            let Some(z) = sample.call1((p.bind(py),))?.extract::<Option<f64>>()? else {
                return Ok(None);
            };
            verts.append(pt(x, y, z - ground_ref))?;
            let (lat, lon, _) = geo.call1((p,))?.extract::<(f64, f64, f64)>()?;
            let (xf, yf) = deg2num(lat, lon, zoom);
            uv.push(((xf - tx as f64) / nw as f64, (yf - ty as f64) / nh as f64));
            if i + 1 < nx && j + 1 < ny {
                let a = j * nx + i;
                tris.extend([(a, a + nx, a + 1), (a + 1, a + nx, a + nx + 1)])
            }
        }
    }
    Ok(Some(py.import("georef.terrain")?.call_method1(
        "TerrainObject",
        (
            verts,
            uv,
            tris,
            (tx, ty, nw, nh, zoom),
            nx,
            ny,
            (x0, y0, x1, y1),
        ),
    )?))
}
#[pyfunction]
fn build_mosaic<'p>(py: Python<'p>, terrain: &Obj<'p>, images: &Obj<'p>) -> PyResult<Obj<'p>> {
    let out = PyDict::new(py);
    for item in images.call_method0("items")?.try_iter()? {
        let (k, v) = item?.extract::<(Obj, Obj)>()?;
        let r = as_raster(py, &v)?;
        if !r.is_none() {
            out.set_item(k, r)?
        }
    }
    py.import("ingetrazo_render")?
        .getattr("SurveyRaster")?
        .call_method1("mosaic", (terrain.getattr("tile_range")?, out, 256))
}
#[pyfunction]
pub(super) fn as_raster<'p>(py: Python<'p>, image: &Obj<'p>) -> PyResult<Obj<'p>> {
    let cls = py.import("ingetrazo_render")?.getattr("SurveyRaster")?;
    if image.is_none() || image.is_instance(&cls)? {
        return Ok(image.clone());
    }
    if image.call_method0("isNull")?.is_truthy()? {
        return Ok(py.None().into_bound(py));
    }
    let rgba = image.call_method1(
        "convertToFormat",
        (image.getattr("Format")?.getattr("Format_RGBA8888")?,),
    )?;
    cls.call1((
        rgba.call_method0("width")?,
        rgba.call_method0("height")?,
        py.get_type::<pyo3::types::PyBytes>()
            .call1((rgba.call_method0("constBits")?,))?,
    ))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    functions!(m;ground_reference,fit_plane,build_flat_surface,build_draped_surface,build_surface,terrain_bounds,terrain_height,build_terrain,build_mosaic,as_raster);
    Ok(())
}

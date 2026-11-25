// SPDX-License-Identifier: GPL-3.0-or-later
//! ODM survey import, including separate position/texture indices and anchor placement.
mod storage;
mod surface;
use numpy::{ndarray::Array2, IntoPyArray};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_core.georef.photomesh")
}
fn path<'p>(py: Python<'p>, p: &Path) -> PyResult<Obj<'p>> {
    py.import("pathlib")?.call_method1("Path", (p,))
}
#[pyfunction]
fn parse_anchor<'p>(py: Python<'p>, text: &str) -> PyResult<Option<Obj<'p>>> {
    let mut lines = text.lines().map(str::trim).filter(|s| !s.is_empty());
    let Some(crs) = lines.next() else {
        return Ok(None);
    };
    let Some(xy) = lines.next() else {
        return Ok(None);
    };
    let mut zone = None;
    let mut north = None;
    for token in crs.replace("UTM", " ").to_uppercase().split_whitespace() {
        if let Some(n) = token
            .strip_suffix('N')
            .or_else(|| token.strip_suffix('S'))
            .and_then(|n| n.parse::<i32>().ok())
        {
            zone = Some(n);
            north = Some(token.ends_with('N'));
            break;
        }
        if let Ok(n) = token.parse::<i32>() {
            zone = Some(n)
        } else if zone.is_some() && matches!(token, "N" | "S") {
            north = Some(token == "N")
        }
    }
    let mut fields = xy.split_whitespace().map(str::parse::<f64>);
    match (zone, north, fields.next(), fields.next()) {
        (Some(z), Some(n), Some(Ok(e)), Some(Ok(y))) => {
            Ok(Some(api(py)?.call_method1("ODMAnchor", (e, y, z, n))?))
        }
        _ => Ok(None),
    }
}
#[pyfunction]
fn find_anchor(py: Python<'_>, obj_path: PathBuf) -> PyResult<Option<Obj<'_>>> {
    let dir = obj_path.parent().unwrap_or(Path::new("."));
    for rel in ["../odm_georeferencing", ".", "odm_georeferencing"] {
        let p = dir.join(rel).join("odm_georeferencing_model_geo.txt");
        if p.is_file() {
            if let Some(a) = parse_anchor(py, &String::from_utf8_lossy(&fs::read(p)?))? {
                return Ok(Some(a));
            }
        }
    }
    Ok(None)
}
#[pyfunction]
fn parse_mtl<'p>(py: Python<'p>, text: &str) -> PyResult<Bound<'p, PyDict>> {
    let out = PyDict::new(py);
    let mut name = None;
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("newmtl") => {
                if let Some(s) = words.next() {
                    name = Some(s);
                    if !out.contains(s)? {
                        out.set_item(s, py.None())?
                    }
                }
            }
            Some("map_Kd") => {
                if let Some(n) = name {
                    let s = words.collect::<Vec<_>>().join(" ");
                    if !s.is_empty() {
                        out.set_item(n, s)?
                    }
                }
            }
            _ => (),
        }
    }
    Ok(out)
}
struct Survey {
    xyz: Vec<[f64; 3]>,
    uv: Vec<[f32; 2]>,
    tris: Vec<u32>,
    runs: Vec<(String, usize, usize)>,
    mtl: Option<String>,
}
impl Survey {
    fn parse(src: &str) -> Result<Self, String> {
        let mut pos = Vec::new();
        let mut tex = Vec::new();
        let mut corners = Vec::new();
        let mut runs = Vec::<(String, usize, usize)>::new();
        let mut mtl = None;
        let mut header = true;
        for (index, line) in src.lines().enumerate() {
            if index % 4096 == 0 && crate::file_open::cancelled() {
                return Err("Opening cancelled".into());
            }
            let mut words = line.split_whitespace();
            let tag = words.next();
            let mut num = || {
                words
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
                    .ok_or("invalid survey coordinate")
            };
            match tag {
                Some("mtllib") if header && mtl.is_none() => {
                    mtl = Some(words.collect::<Vec<_>>().join(" "))
                }
                Some("v") => {
                    header = false;
                    pos.push([num()?, num()?, num()?])
                }
                Some("vt") => tex.push([num()? as f32, (1. - num()?) as f32]),
                Some("usemtl") => {
                    let name = words.collect::<Vec<_>>().join(" ");
                    match runs.last_mut() {
                        Some((n, _, 0)) => *n = name,
                        _ => runs.push((name, corners.len() / 3, 0)),
                    }
                }
                Some("f") => {
                    header = false;
                    let mut face = vec![];
                    for corner in words {
                        let mut ix = corner.split('/');
                        let v = ix
                            .next()
                            .and_then(|v| v.parse::<i64>().ok())
                            .ok_or("invalid survey face index")?;
                        let uv = match ix.next() {
                            Some(s) if !s.is_empty() => s
                                .parse::<i64>()
                                .map_err(|_| "invalid survey texture index")?,
                            _ => 0,
                        };
                        face.push((
                            if v > 0 { v - 1 } else { pos.len() as i64 + v },
                            if uv < 0 {
                                tex.len() as i64 + uv + 1
                            } else {
                                uv
                            },
                        ));
                    }
                    for i in 2..face.len() {
                        corners.extend([face[0], face[i - 1], face[i]])
                    }
                    if let Some((_, _, n)) = runs.last_mut() {
                        *n += face.len().saturating_sub(2);
                    }
                }
                _ => (),
            }
        }
        if !corners.is_empty() && pos.is_empty() {
            return Err("survey face has no vertices".into());
        }
        for (v, t) in &mut corners {
            *v = (*v).clamp(0, pos.len().saturating_sub(1) as i64);
            *t = (*t).clamp(0, tex.len() as i64)
        }
        let mut distinct = corners.clone();
        distinct.sort_unstable();
        distinct.dedup();
        let mut ids = HashMap::with_capacity(distinct.len());
        let mut xyz = Vec::with_capacity(distinct.len());
        let mut uv = Vec::with_capacity(distinct.len());
        for pair in distinct {
            ids.insert(
                pair,
                u32::try_from(xyz.len()).map_err(|_| "too many survey vertices")?,
            );
            xyz.push(pos[pair.0 as usize]);
            uv.push(if pair.1 == 0 {
                [0.; 2]
            } else {
                tex[pair.1 as usize - 1]
            });
        }
        let tris = corners.iter().map(|c| ids[c]).collect();
        Ok(Self {
            xyz,
            uv,
            tris,
            runs,
            mtl,
        })
    }
}
#[pyfunction(signature=(obj_path,datum,ground_ref=0.,anchor=None))]
fn load_odm_obj(
    py: Python<'_>,
    obj_path: PathBuf,
    datum: Py<PyAny>,
    ground_ref: f64,
    anchor: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    crate::file_open::DocumentOpen::stage(py, None, move |py, _| {
        Ok(load_inner(
            py,
            obj_path,
            datum.bind(py),
            ground_ref,
            anchor.map(|a| a.into_bound(py)),
        )?
        .unbind())
    })
}
fn load_inner<'p>(
    py: Python<'p>,
    obj_path: PathBuf,
    datum: &Obj<'p>,
    ground_ref: f64,
    anchor: Option<Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let a = match anchor {
        Some(a) => Some(a),
        None => find_anchor(py, obj_path.clone())?,
    };
    let file = py.import("built_core.formats.obj")?;
    let src = file
        .call_method1("read_obj_text", (&obj_path,))?
        .extract::<String>()?;
    let mut data = py
        .detach(move || Survey::parse(&src))
        .map_err(PyValueError::new_err)?;
    if let Some(a) = &a {
        let e = a.getattr("east")?.extract::<f64>()?;
        let n = a.getattr("north")?.extract::<f64>()?;
        let z = a.getattr("zone")?.extract::<i32>()?;
        let north = a.getattr("northern")?.is_truthy()?;
        if z == datum.getattr("zone")?.extract::<i32>()?
            && north == datum.getattr("northern")?.is_truthy()?
        {
            let v = crate::math::xyz(&datum.call_method1("utm_to_local", (e, n, 0.))?)?;
            for (index, p) in data.xyz.iter_mut().enumerate() {
                if index % 2048 == 0 {
                    crate::file_open::checkpoint(py)?;
                }
                p[0] += v.x;
                p[1] += v.y
            }
        } else {
            for (index, p) in data.xyz.iter_mut().enumerate() {
                if index % 2048 == 0 {
                    crate::file_open::checkpoint(py)?;
                }
                let (lat, lon) = crate::location::utm_inverse(e + p[0], n + p[1], z, north);
                let v =
                    crate::math::xyz(&datum.call_method1("geodetic_to_local", (lat, lon, 0.))?)?;
                p[0] = v.x;
                p[1] = v.y
            }
        }
    }
    let dir = obj_path.parent().unwrap_or(Path::new("."));
    let materials = PyList::empty(py);
    let module = api(py)?;
    if !data.runs.is_empty() {
        let mtl = data
            .mtl
            .map(|n| dir.join(n))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| obj_path.with_extension("mtl"));
        let maps = if mtl.is_file() {
            parse_mtl(
                py,
                &file
                    .call_method1("read_obj_text", (mtl,))?
                    .extract::<String>()?,
            )?
        } else {
            PyDict::new(py)
        };
        for (name, start, count) in data.runs {
            if count == 0 {
                continue;
            }
            let tex = maps
                .get_item(&name)?
                .and_then(|v| v.extract::<String>().ok())
                .map(|s| path(py, &dir.join(s)))
                .transpose()?;
            materials.append(module.call_method1("PhotoMaterial", (name, tex, start, count))?)?;
        }
    }
    let n = data.tris.len() / 3;
    if materials.is_empty() && n > 0 {
        materials.append(module.call_method1("PhotoMaterial", ("", py.None(), 0, n))?)?
    }
    let pos = Array2::from_shape_vec(
        (data.xyz.len(), 3),
        data.xyz
            .into_iter()
            .flat_map(|[x, y, z]| [x as f32, y as f32, (z - ground_ref) as f32])
            .collect(),
    )
    .unwrap()
    .into_pyarray(py);
    let uv = Array2::from_shape_vec((data.uv.len(), 2), data.uv.into_iter().flatten().collect())
        .unwrap()
        .into_pyarray(py);
    let tri = Array2::from_shape_vec((n, 3), data.tris)
        .unwrap()
        .into_pyarray(py);
    module.call_method1(
        "PhotoMesh",
        (pos, uv, tri, materials, path(py, &obj_path)?, a),
    )
}
#[pyfunction]
fn atlas_bytes(sizes: Vec<(usize, usize)>) -> u64 {
    (sizes.iter().map(|&(w, h)| w as f64 * h as f64).sum::<f64>() * 4. * (4. / 3.)) as u64
}
#[pyfunction(signature=(sizes,gl_max,budget=1073741824))]
fn plan_texture_sizes(
    sizes: Vec<(usize, usize)>,
    gl_max: usize,
    budget: u64,
) -> Vec<(usize, usize)> {
    let mut out: Vec<_> = sizes
        .into_iter()
        .map(|(w, h)| {
            if w.max(h) <= gl_max {
                (w, h)
            } else {
                let s = gl_max as f64 / w.max(h) as f64;
                (
                    ((w as f64 * s) as usize).max(1),
                    ((h as f64 * s) as usize).max(1),
                )
            }
        })
        .collect();
    while atlas_bytes(out.clone()) > budget {
        let Some((w, h)) = out
            .iter_mut()
            .rev()
            .max_by_key(|(w, h)| *w as u128 * *h as u128)
        else {
            break;
        };
        if (*w).max(*h) <= 256 {
            break;
        }
        *w = (*w / 2).max(1);
        *h = (*h / 2).max(1);
    }
    out
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse_anchor, m)?)?;
    m.add_function(wrap_pyfunction!(find_anchor, m)?)?;
    m.add_function(wrap_pyfunction!(parse_mtl, m)?)?;
    m.add_function(wrap_pyfunction!(load_odm_obj, m)?)?;
    m.add_function(wrap_pyfunction!(atlas_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(plan_texture_sizes, m)?)?;
    surface::register(m)?;
    storage::register(m)
}

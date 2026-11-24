// SPDX-License-Identifier: GPL-3.0-or-later
//! Convert glTF triangle batches to a detached editable mesh.
use glam::DVec3;
use numpy::{ndarray::Array2, IntoPyArray, PyReadonlyArray3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
type Obj<'py> = Bound<'py, PyAny>;
fn tick(progress: Option<&Obj<'_>>, fraction: f64, msg: &str) -> PyResult<()> {
    if let Some(cb) = progress {
        cb.call1((fraction, msg))?;
    }
    Ok(())
}
// Gradients dual to the two edges lie in the plane of the triangle.
fn affine(p: [DVec3; 3], uv: [[f64; 2]; 3]) -> Option<([f64; 8], f64, f64)> {
    let a = p[1] - p[0];
    let b = p[2] - p[0];
    let n = a.cross(b);
    let det = n.length_squared();
    if !(det > 1e-18) {
        return None;
    }
    let a_dual = b.cross(n) / det;
    let b_dual = n.cross(a) / det;
    let u = a_dual * (uv[1][0] - uv[0][0]) + b_dual * (uv[2][0] - uv[0][0]);
    let v = a_dual * (uv[0][1] - uv[1][1]) + b_dual * (uv[0][1] - uv[2][1]);
    let size = |v: DVec3| {
        if v.length() > 1e-9 {
            1. / v.length()
        } else {
            1.
        }
    };
    Some((
        [
            u.x,
            u.y,
            u.z,
            uv[0][0] - u.dot(p[0]),
            v.x,
            v.y,
            v.z,
            1. - uv[0][1] - v.dot(p[0]),
        ],
        size(u),
        size(v),
    ))
}
fn texture_path(face: &Obj<'_>) -> PyResult<Option<String>> {
    let attrs = face.getattr("attrs")?;
    if !attrs.is_truthy()? {
        return Ok(None);
    }
    let tex = attrs.call_method1("get", ("texture",))?;
    if !tex.is_truthy()? {
        return Ok(None);
    }
    tex.call_method1("get", ("path",))?.extract()
}
fn hide_cards(mesh: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    let mut stats = HashMap::<String, (usize, f64)>::new();
    for (i, f) in mesh.getattr("faces")?.try_iter()?.enumerate() {
        if i % 2048 == 0 {
            crate::file_open::checkpoint(py)?
        }
        let f = f?;
        if let Some(t) = texture_path(&f)?.filter(|p| !p.is_empty()) {
            let e = stats.entry(t).or_default();
            e.0 += 1;
            e.1 += f.call_method0("area")?.extract::<f64>()?;
        }
    }
    let alpha = py
        .import("ingetrazo_render")?
        .getattr("texture_has_alpha")?;
    let mut foliage = HashSet::new();
    for (path, (n, area)) in stats {
        if (n > 200 && area / (n as f64) < 0.02) || alpha.call1((&path,))?.is_truthy()? {
            foliage.insert(path);
        }
    }
    if foliage.is_empty() {
        return Ok(());
    }
    for (i, e) in mesh.getattr("edges")?.try_iter()?.enumerate() {
        if i % 2048 == 0 {
            crate::file_open::checkpoint(py)?
        }
        let e = e?;
        let faces = e.getattr("faces")?;
        if !faces.is_truthy()? {
            continue;
        }
        let mut hide = true;
        for face in faces.try_iter()? {
            if !texture_path(&face?)?.is_some_and(|p| foliage.contains(&p)) {
                hide = false;
                break;
            }
        }
        if hide {
            e.setattr("hidden", true)?
        }
    }
    Ok(())
}
fn three<'py>(py: Python<'py>, obj: &Obj<'py>) -> PyResult<PyReadonlyArray3<'py, f64>> {
    Ok(py
        .import("numpy")?
        .call_method1("asarray", (obj, "float64"))?
        .extract()?)
}
#[pyfunction]
#[pyo3(signature=(batches,progress=None))]
fn gltf_build_mesh<'py>(
    py: Python<'py>,
    batches: &Obj<'py>,
    progress: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let mesh = py.import("core.mesh")?.call_method0("Mesh")?;
    let mut flat = Vec::new();
    let attrs = PyList::empty(py);
    let total = batches
        .try_iter()?
        .try_fold(0usize, |n, b| Ok::<_, PyErr>(n + b?.get_item(0)?.len()?))?;
    if batches.len()? == 0 {
        return Err(PyValueError::new_err(
            "No triangle geometry found in the glTF file",
        ));
    }
    for batch in batches.try_iter()? {
        let batch = batch?;
        let tris = three(py, &batch.get_item(0)?)?;
        let tris = tris.as_array();
        if tris.shape()[1..] != [3, 3] {
            return Err(PyValueError::new_err("invalid glTF triangle shape"));
        }
        let count = tris.shape()[0];
        let material = batch.get_item(2)?;
        let path = material.get_item(0)?;
        let rgba = material.get_item(1)?;
        let opacity = material.get_item(2)?;
        let color = if rgba.is_truthy()? {
            rgba.try_iter()?
                .take(3)
                .map(|v| v?.extract())
                .collect::<PyResult<Vec<f64>>>()?
        } else {
            vec![1.; 3]
        };
        let extra = PyDict::new(py);
        if material.len()? > 3 {
            let recipe = material.get_item(3)?;
            extra.set_item("mat", recipe.get_item(0)?)?;
            extra.set_item("pbr", recipe.get_item(1)?)?;
            extra.set_item("back", true)?;
            extra.set_item("color", &color)?;
        }
        let uv = batch.get_item(1)?;
        if uv.is_none() || (!path.is_truthy()? && extra.is_empty()) {
            if rgba.is_truthy()? && color != [1., 1., 1.] {
                extra.set_item("color", &color)?;
                extra.set_item("back", true)?
            }
            if !opacity.is_none() {
                extra.set_item("opacity", &opacity)?
            }
            let row = (!extra.is_empty()).then_some(&extra);
            for _ in 0..count {
                attrs.append(row)?
            }
            flat.extend(tris.iter().copied());
        } else {
            let uv = three(py, &uv)?;
            let uv = uv.as_array();
            if uv.shape() != [count, 3, 2] {
                return Err(PyValueError::new_err("invalid glTF UV shape"));
            }
            for i in 0..count {
                if i % 2048 == 0 {
                    crate::file_open::checkpoint(py)?
                }
                let p = std::array::from_fn(|j| {
                    DVec3::new(tris[[i, j, 0]], tris[[i, j, 1]], tris[[i, j, 2]])
                });
                for v in p {
                    flat.extend(v.to_array())
                }
                let row = PyDict::new(py);
                if let Some((map, sw, sh)) =
                    affine(p, std::array::from_fn(|j| [uv[[i, j, 0]], uv[[i, j, 1]]]))
                {
                    let texture = PyDict::new(py);
                    texture.set_item(
                        "path",
                        path.extract::<Option<String>>()?.unwrap_or_default(),
                    )?;
                    texture.set_item("uvw", map)?;
                    texture.set_item("sw", sw)?;
                    texture.set_item("sh", sh)?;
                    row.set_item("texture", texture)?;
                    row.set_item("back", true)?
                }
                row.update(extra.as_mapping())?;
                if !row.is_empty() && !opacity.is_none() {
                    row.set_item("opacity", &opacity)?
                }
                attrs.append((!row.is_empty()).then_some(row))?;
            }
        }
        tick(
            progress,
            0.3 + 0.4 * attrs.len() as f64 / total.max(1) as f64,
            "Collecting geometry…",
        )?;
    }
    let n = flat.len() / 9;
    let array = Array2::from_shape_vec((flat.len() / 3, 3), flat)
        .unwrap()
        .into_pyarray(py);
    let (v, indices): (Obj<'_>, Obj<'_>) = mesh.call_method1("bulk_weld", (array,))?.extract()?;
    tick(progress, 0.75, "Building the model…")?;
    mesh.call_method1(
        "add_faces_welded",
        (
            v,
            indices,
            vec![3i64; n].into_pyarray(py),
            vec![1i64; n].into_pyarray(py),
            attrs,
        ),
    )?;
    if mesh.getattr("edges")?.len()? <= 500_000 {
        tick(progress, 0.9, "Smoothing edges…")?;
        py.import("formats.fuse")?
            .call_method1("soften_smooth_edges", (&mesh,))?;
    }
    hide_cards(&mesh)?;
    Ok(mesh)
}
fn bounds(batches: &Obj<'_>) -> PyResult<(DVec3, DVec3)> {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    for b in batches.try_iter()? {
        let a = three(batches.py(), &b?.get_item(0)?)?;
        let a = a.as_array();
        if a.shape()[2] != 3 {
            return Err(PyValueError::new_err("invalid glTF position shape"));
        }
        for tri in a.outer_iter() {
            for row in tri.outer_iter() {
                let p = DVec3::new(row[0], row[1], row[2]);
                lo = lo.min(p);
                hi = hi.max(p)
            }
        }
    }
    Ok((lo, hi))
}
fn scale_for(dim: f64) -> f64 {
    [1., 0.0254, 0.01, 0.001]
        .into_iter()
        .find(|s| (0.2..=40.).contains(&(s * dim)))
        .unwrap_or(1.)
}
#[pyfunction]
fn gltf_guess_scale(batches: &Obj<'_>) -> PyResult<f64> {
    let (lo, hi) = bounds(batches)?;
    Ok(scale_for((hi - lo).max_element()))
}
fn stage<'py>(
    py: Python<'py>,
    path: PathBuf,
    materials: &Obj<'py>,
    progress: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    tick(progress, 0.05, "Reading file…")?;
    let (batches, js) = crate::gltf_read::gltf_read_triangles(py, path.clone(), Some(materials))?;
    let asset = js
        .cast::<PyDict>()?
        .get_item("asset")?
        .filter(|x| !x.is_none())
        .unwrap_or_else(|| PyDict::new(py).into_any());
    let ours = asset.call_method1("get", ("generator",))?.eq("IngeTrazo")?;
    let (lo, hi) = bounds(&batches)?;
    let scale = if ours {
        1.
    } else {
        scale_for((hi - lo).max_element())
    };
    let offset = py.import("formats.recenter")?.call_method1(
        "survey_offset",
        ((lo * scale).to_array(), (hi * scale).to_array()),
    )?;
    let adjusted = PyList::empty(py);
    for b in batches.try_iter()? {
        let b = b?;
        let mut p = b.get_item(0)?;
        if scale != 1. {
            p = p.mul(scale)?
        }
        if !offset.is_none() {
            p = p.sub(&offset)?
        }
        adjusted.append((p, b.get_item(1)?, b.get_item(2)?))?
    }
    let mesh = gltf_build_mesh(py, &adjusted, progress)?;
    let extras = asset.call_method1("get", ("extras",))?;
    let title = if extras.is_truthy()? {
        extras
            .call_method1("get", ("title",))?
            .extract::<Option<String>>()?
    } else {
        None
    };
    let name = title
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .chars()
        .take(60)
        .collect::<String>();
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    let group = py
        .import("core.group")?
        .getattr("Group")?
        .call((mesh,), Some(&kw))?;
    Ok((group, offset).into_pyobject(py)?.into_any())
}
#[pyfunction]
#[pyo3(signature=(scene,path,progress=None))]
fn gltf_load<'py>(
    py: Python<'py>,
    scene: &Obj<'py>,
    path: PathBuf,
    progress: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let registry = scene.getattr("materials")?;
    let materials = py.get_type::<PyDict>().call1((&registry,))?;
    let local = materials.clone().unbind();
    let report = progress.is_some();
    let result = crate::file_open::DocumentOpen::stage(py, progress, move |py, cb| {
        Ok(stage(py, path, local.bind(py), report.then_some(&cb))?.unbind())
    })?
    .into_bound(py);
    registry.call_method1("update", (materials,))?;
    scene
        .getattr("groups")?
        .call_method1("append", (result.get_item(0)?,))?;
    result.get_item(1)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(gltf_load, m)?)?;
    m.add_function(wrap_pyfunction!(gltf_build_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(gltf_guess_scale, m)?)
}

// SPDX-License-Identifier: GPL-3.0-or-later
use super::{api, path, Obj};
use pyo3::{
    exceptions::{PyException, PyOSError, PyValueError},
    prelude::*,
    types::{PyDict, PyList},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
const GEOMETRY: &str = "survey/geometry.npz";
#[pyfunction]
fn photo_pack<'p>(
    py: Python<'p>,
    mesh: &Obj<'p>,
) -> PyResult<(Bound<'p, PyDict>, Bound<'p, PyDict>)> {
    let b = py.import("io")?.call_method0("BytesIO")?;
    let data = PyDict::new(py);
    for k in ["vertices", "uvs", "triangles"] {
        data.set_item(k, mesh.getattr(k)?)?
    }
    py.import("numpy")?
        .call_method("savez_compressed", (&b,), Some(&data))?;
    let blobs = PyDict::new(py);
    blobs.set_item(GEOMETRY, b.call_method0("getvalue")?)?;
    let mats = PyList::empty(py);
    let images = mesh.getattr("images")?;
    let convert = py.import("ingetrazo_core")?.getattr("as_raster")?;
    for (i, m) in mesh.getattr("materials")?.try_iter()?.enumerate() {
        let m = m?;
        let d = PyDict::new(py);
        for k in ["name", "start", "count"] {
            d.set_item(k, m.getattr(k)?)?
        }
        let image = convert.call1((images.call_method1("get", (i,))?,))?;
        if !image.is_none() {
            let jpeg = image.call_method0("jpeg")?;
            if jpeg.is_truthy()? {
                let member = format!("survey/atlas-{i:02}.jpg");
                blobs.set_item(&member, jpeg)?;
                d.set_item("atlas", member)?
            }
        }
        mats.append(d)?
    }
    let meta = PyDict::new(py);
    meta.set_item("geometry", GEOMETRY)?;
    meta.set_item("materials", mats)?;
    for k in ["visible", "layer", "vertical_reference"] {
        meta.set_item(k, mesh.getattr(k)?)?
    }
    let a = mesh.getattr("anchor")?;
    if !a.is_none() {
        let d = PyDict::new(py);
        for k in ["east", "north", "zone", "northern"] {
            d.set_item(k, a.getattr(k)?)?
        }
        meta.set_item("anchor", d)?
    }
    let s = mesh.getattr("source")?;
    if !s.is_none() {
        meta.set_item("source", s.str()?)?
    }
    Ok((meta, blobs))
}
#[pyfunction]
fn photo_unpack<'p>(
    py: Python<'p>,
    meta: &Obj<'p>,
    read_blob: &Obj<'p>,
) -> PyResult<Option<Obj<'p>>> {
    let arrays = (|| {
        let bytes = read_blob.call1((meta.call_method1("get", ("geometry", GEOMETRY))?,))?;
        let b = py.import("io")?.call_method1("BytesIO", (bytes,))?;
        let zip = py.import("numpy")?.call_method1("load", (b,))?;
        let data = (|| -> PyResult<_> {
            Ok((
                zip.get_item("vertices")?,
                zip.get_item("uvs")?,
                zip.get_item("triangles")?,
            ))
        })();
        zip.call_method0("close")?;
        data
    })();
    let (v, u, t) = match arrays {
        Ok(a) => a,
        Err(e) if e.is_instance_of::<PyException>(py) => return Ok(None),
        Err(e) => return Err(e),
    };
    let module = api(py)?;
    let a = meta.call_method1("get", ("anchor",))?;
    let anchor = if a.is_truthy()? {
        ((|| -> PyResult<_> {
            let f = py.get_type::<pyo3::types::PyFloat>();
            let i = py.get_type::<pyo3::types::PyInt>();
            module.call_method1(
                "ODMAnchor",
                (
                    f.call1((a.get_item("east")?,))?,
                    f.call1((a.get_item("north")?,))?,
                    i.call1((a.get_item("zone")?,))?,
                    a.get_item("northern")?.is_truthy()?,
                ),
            )
        })())
        .ok()
    } else {
        None
    };
    let mats = PyList::empty(py);
    let imgs = PyDict::new(py);
    for (i, item) in meta
        .call_method1("get", ("materials", PyList::empty(py)))?
        .try_iter()?
        .enumerate()
    {
        let m = item?;
        mats.append(module.call_method1(
            "PhotoMaterial",
            (
                m.call_method1("get", ("name", ""))?,
                py.None(),
                m.call_method1("get", ("start", 0))?,
                m.call_method1("get", ("count", 0))?,
            ),
        )?)?;
        let atlas = m.call_method1("get", ("atlas",))?;
        if atlas.is_truthy()? {
            let data = read_blob.call1((atlas,))?;
            if data.is_truthy()? {
                let raster = py
                    .import("ingetrazo_render")?
                    .getattr("SurveyRaster")?
                    .call_method1("decode", (data,))?;
                if !raster.is_none() {
                    imgs.set_item(i, raster)?
                }
            }
        }
    }
    let source = meta.call_method1("get", ("source",))?;
    let source = if source.is_truthy()? {
        Some(path(py, &source.extract::<PathBuf>()?)?)
    } else {
        None
    };
    let mesh = module.call_method1("PhotoMesh", (v, u, t, mats, source, anchor))?;
    mesh.setattr("images", imgs)?;
    mesh.setattr(
        "visible",
        meta.call_method1("get", ("visible", true))?.is_truthy()?,
    )?;
    mesh.setattr(
        "vertical_reference",
        meta.call_method1("get", ("vertical_reference", "odm"))?,
    )?;
    mesh.setattr("layer", meta.call_method1("get", ("layer",))?)?;
    Ok(Some(mesh))
}
fn cached(py: Python<'_>, p: &Path, target: (usize, usize)) -> PyResult<PathBuf> {
    let path = path(py, p)?;
    let stat = path.call_method0("stat")?;
    let key = format!(
        "{}|{}|{}|{}x{}",
        path.call_method0("resolve")?.str()?,
        stat.getattr("st_size")?,
        stat.getattr("st_mtime")?.extract::<f64>()? as i64,
        target.0,
        target.1
    );
    let digest = py
        .import("hashlib")?
        .call_method1("sha1", (pyo3::types::PyBytes::new(py, key.as_bytes()),))?
        .call_method0("hexdigest")?
        .extract::<String>()?;
    let root = py
        .import("core.texture")?
        .call_method0("texture_cache_root")?
        .extract::<PathBuf>()?;
    Ok(root.join("odm").join(format!(
        "{}-{}.jpg",
        &digest[..16],
        p.file_stem().unwrap_or_default().to_string_lossy()
    )))
}
#[pyfunction]
fn load_atlas(py: Python<'_>, path: PathBuf, target: (usize, usize)) -> PyResult<Obj<'_>> {
    let raster = py.import("ingetrazo_render")?.getattr("SurveyRaster")?;
    let cache = cached(py, &path, target).ok();
    if let Some(p) = cache.as_ref().filter(|p| p.is_file()) {
        let img = raster.call_method1("atlas", (p, target))?;
        if !img.is_none() {
            return Ok(img);
        }
    }
    let img = raster.call_method1("atlas", (path, target))?;
    if let Some(p) = cache.filter(|_| !img.is_none()) {
        let written = (|| -> PyResult<()> {
            fs::create_dir_all(p.parent().unwrap())?;
            py.import("formats.igz")?
                .call_method1("_write_atomic", (p, img.call_method0("jpeg")?))?;
            Ok(())
        })();
        if let Err(e) = written {
            if !e.is_instance_of::<PyValueError>(py) && !e.is_instance_of::<PyOSError>(py) {
                return Err(e);
            }
        }
    }
    Ok(img)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(photo_pack, m)?)?;
    m.add_function(wrap_pyfunction!(photo_unpack, m)?)?;
    m.add_function(wrap_pyfunction!(load_atlas, m)?)?;
    Ok(())
}

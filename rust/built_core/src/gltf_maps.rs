// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared-UV PBR image channel packing for glTF.
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};
use std::path::PathBuf;
type Obj<'py> = Bound<'py, PyAny>;
fn path_record<'py>(py: Python<'py>, path: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("path", path)?;
    Ok(d)
}
fn open<'py>(py: Python<'py>, src: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    if !src.is_truthy()? {
        return Ok(None);
    }
    let raster = py
        .import("built_render")?
        .getattr("SurveyRaster")?
        .call_method1("open", (src,))?;
    Ok((!raster.is_none()).then_some(raster))
}
fn dims(img: &Obj<'_>) -> PyResult<(usize, usize)> {
    Ok((
        img.getattr("width")?.extract()?,
        img.getattr("height")?.extract()?,
    ))
}
fn cache<'py>(
    py: Python<'py>,
    w: usize,
    h: usize,
    pixels: &[u8],
    name: &str,
) -> PyResult<Obj<'py>> {
    let png = py
        .import("built_render")?
        .getattr("SurveyRaster")?
        .call1((w, h, PyBytes::new(py, pixels)))?
        .call_method0("png")?;
    Ok(py
        .import("built_core.texture")?
        .call_method1("cache_image", (png, name, "pbr"))?
        .str()?
        .into_any())
}
#[pyfunction]
pub(crate) fn gltf_import_recipe<'py>(
    py: Python<'py>,
    mat: &Obj<'py>,
    textures: &Obj<'py>,
    paths: &Obj<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let mr = mat.call_method1("get", ("pbrMetallicRoughness", PyDict::new(py)))?;
    let recipe = PyDict::new(py);
    for (k, field) in [
        ("metallic", "metallicFactor"),
        ("roughness", "roughnessFactor"),
    ] {
        recipe.set_item(k, mr.call_method1("get", (field, 1.))?)?;
    }
    let path = |obj: &Obj<'py>, slot: &str| -> PyResult<Obj<'py>> {
        let info = obj.call_method1("get", (slot,))?;
        if info.is_truthy()? {
            let source = textures
                .get_item(info.get_item("index")?)?
                .call_method1("get", ("source",))?;
            if let Ok(i) = source.extract::<usize>() {
                if i < paths.len()? {
                    return paths.get_item(i);
                }
            }
        }
        Ok(py.None().into_bound(py))
    };
    let packed = path(&mr, "metallicRoughnessTexture")?;
    let ao = path(mat, "occlusionTexture")?;
    if packed.is_truthy()? && packed.eq(&ao)? {
        recipe.set_item("orm_map", path_record(py, &packed)?)?
    } else if let Some(im) = open(py, &packed)? {
        let (w, h) = dims(&im)?;
        let pixels = im.call_method0("rgba")?;
        let bytes = pixels.cast::<PyBytes>()?.as_bytes();
        for (channel, key) in [(1, "roughness_map"), (2, "metallic_map")] {
            let mut grey = bytes.to_vec();
            for px in grey.chunks_exact_mut(4) {
                let c = px[channel];
                px.copy_from_slice(&[c, c, c, 255])
            }
            let src = cache(py, w, h, &grey, &format!("{key}.png"))?;
            recipe.set_item(key, path_record(py, &src)?)?;
        }
    }
    if ao.is_truthy()? {
        recipe.set_item(
            "ao_strength",
            mat.get_item("occlusionTexture")?
                .call_method1("get", ("strength", 1.))?,
        )?;
        if !ao.eq(&packed)? {
            recipe.set_item("ao_map", path_record(py, &ao)?)?
        }
    }
    let normal = path(mat, "normalTexture")?;
    if normal.is_truthy()? {
        recipe.set_item("normal_map", path_record(py, &normal)?)?;
        recipe.set_item(
            "normal_scale",
            mat.get_item("normalTexture")?
                .call_method1("get", ("scale", 1.))?,
        )?
    }
    Ok(recipe)
}
#[pyfunction]
pub(crate) fn gltf_export_maps<'py>(
    py: Python<'py>,
    recipe: &Obj<'py>,
) -> PyResult<(Obj<'py>, Obj<'py>, Obj<'py>)> {
    let path = |slot: &str| -> PyResult<Obj<'py>> {
        let m = recipe.call_method1("get", (slot,))?;
        if m.is_truthy()? {
            m.call_method1("get", ("path",))
        } else {
            Ok(py.None().into_bound(py))
        }
    };
    let mut normal = path("normal_map")?;
    if normal.is_truthy()?
        && recipe
            .get_item("normal_map")?
            .call_method1("get", ("flip_y",))?
            .is_truthy()?
    {
        if let Some(im) = open(py, &normal)? {
            let (w, h) = dims(&im)?;
            let mut bytes = im.call_method0("rgba")?.extract::<Vec<u8>>()?;
            for px in bytes.chunks_exact_mut(4) {
                px[1] = 255 - px[1]
            }
            let src: PathBuf = normal.extract()?;
            normal = cache(
                py,
                w,
                h,
                &bytes,
                &format!(
                    "{}-opengl.png",
                    src.file_stem().unwrap_or_default().to_string_lossy()
                ),
            )?
        }
    }
    let orm = path("orm_map")?;
    let mut packed = orm.clone();
    if !packed.is_truthy()? {
        let mut layers = Vec::new();
        let (mut w, mut h) = (0, 0);
        for (c, k) in [(1, "roughness_map"), (2, "metallic_map")] {
            if let Some(img) = open(py, &path(k)?)? {
                let (iw, ih) = dims(&img)?;
                w = w.max(iw);
                h = h.max(ih);
                layers.push((c, img))
            }
        }
        if !layers.is_empty() {
            let len = w
                .checked_mul(h)
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("PBR map too large"))?;
            let mut pixels = vec![255u8; len];
            for (c, img) in layers {
                let rgba = img.call_method1("resized", (w, h))?.call_method0("rgba")?;
                for (dst, src) in pixels
                    .chunks_exact_mut(4)
                    .zip(rgba.cast::<PyBytes>()?.as_bytes().chunks_exact(4))
                {
                    dst[c] = src[0]
                }
            }
            packed = cache(py, w, h, &pixels, "metallic-roughness.png")?;
        }
    }
    let ao = if orm.is_truthy()? {
        orm
    } else {
        path("ao_map")?
    };
    Ok((normal, packed, ao))
}
pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(gltf_import_recipe, module)?)?;
    module.add_function(wrap_pyfunction!(gltf_export_maps, module)?)
}

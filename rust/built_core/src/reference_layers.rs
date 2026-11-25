// SPDX-License-Identifier: GPL-3.0-or-later
//! Reference images and survey geometry. Datum binding, selection and
//! invalidation of terrain triangulations belong to the host.
use crate::math::{xyz, Vec3};
use pyo3::prelude::*;
type Obj<'py> = Bound<'py, PyAny>;
#[pyfunction]
pub(crate) fn add_image(scene: &Obj<'_>, image: &Obj<'_>) -> PyResult<bool> {
    let name = image.getattr("layer")?;
    let make = name.is_truthy()? && scene.call_method1("layer", (&name,))?.is_none();
    if make {
        let layer = scene
            .py()
            .import("core.layers")?
            .getattr("Layer")?
            .call1((name,))?;
        scene.getattr("layers")?.call_method1("append", (layer,))?;
    }
    scene
        .getattr("image_planes")?
        .call_method1("append", (image,))?;
    Ok(make)
}
#[pyfunction]
pub(crate) fn set_image_frame(image: &Obj<'_>, vectors: &Obj<'_>) -> PyResult<(Vec3, Vec3, Vec3)> {
    let previous = (
        Vec3::from(xyz(&image.getattr("origin")?)?),
        Vec3::from(xyz(&image.getattr("u")?)?),
        Vec3::from(xyz(&image.getattr("v")?)?),
    );
    for (i, field) in ["origin", "u", "v"].into_iter().enumerate() {
        let v = vectors.get_item(i)?;
        if !v.is_none() {
            image.setattr(field, Vec3::from(xyz(&v)?))?;
        }
    }
    Ok(previous)
}
#[pyfunction]
pub(crate) fn transform_images(images: Vec<Obj<'_>>, m: &Obj<'_>) -> PyResult<()> {
    for image in images {
        image.setattr(
            "origin",
            m.call_method1("map", (image.getattr("origin")?,))?,
        )?;
        for field in ["u", "v"] {
            image.setattr(
                field,
                m.call_method1("mapVector", (image.getattr(field)?,))?,
            )?;
        }
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn toggle_paths(paths: Vec<Obj<'_>>) -> PyResult<()> {
    for path in paths {
        path.setattr("closed", !path.getattr("closed")?.is_truthy()?)?;
    }
    Ok(())
}
type Surface<'py> = (Obj<'py>, Obj<'py>, Obj<'py>);
#[pyfunction]
pub(crate) fn path_surfaces<'py>(paths: Vec<Obj<'py>>, mode: &Obj<'py>) -> PyResult<Vec<Surface<'py>>> {
    let mut before = Vec::new();
    for p in &paths {
        before.push((p.clone(), p.getattr("surface")?, p.getattr("closed")?));
    }
    for p in paths {
        p.setattr("surface", mode)?;
        if mode.is_truthy()? {
            p.setattr("closed", true)?;
        }
        p.setattr("_surface_tris", p.py().None())?;
    }
    Ok(before)
}
#[pyfunction]
pub(crate) fn move_node(path: &Obj<'_>, index: &Obj<'_>, value: &Obj<'_>) -> PyResult<Vec3> {
    let points = path.getattr("points")?;
    let old = Vec3::from(xyz(&points.get_item(index)?)?);
    points.set_item(index, Vec3::from(xyz(value)?))?;
    Ok(old)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "reference_layers")?;
    sub.add_function(wrap_pyfunction!(add_image, &sub)?)?;
    sub.add_function(wrap_pyfunction!(set_image_frame, &sub)?)?;
    sub.add_function(wrap_pyfunction!(transform_images, &sub)?)?;
    sub.add_function(wrap_pyfunction!(toggle_paths, &sub)?)?;
    sub.add_function(wrap_pyfunction!(path_surfaces, &sub)?)?;
    sub.add_function(wrap_pyfunction!(move_node, &sub)?)?;
    m.add_submodule(&sub)?;
    Ok(())
}

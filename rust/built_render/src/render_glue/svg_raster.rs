//! An SVG file drawn into a square, for scripts that rasterize artwork
//! (``scripts/gen_built_icon.py``); wrap the pixels in a ``SurveyRaster``
//! to save or encode them.
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use resvg::tiny_skia::{Pixmap, Transform};
use std::path::PathBuf;

/// ``(size, size, rgba)``: the SVG at ``path`` centred and fitted in a
/// ``size`` × ``size`` square, straight RGBA, transparent around it.
#[pyfunction]
pub fn svg_rgba<'py>(py: Python<'py>, path: PathBuf, size: u32) -> PyResult<(u32, u32, Bound<'py, PyBytes>)> {
    let data = std::fs::read(&path).map_err(|e| PyValueError::new_err(format!("cannot read {}: {e}", path.display())))?;
    let tree = crate::image_assets::svg_tree(&data, path.parent())
        .ok_or_else(|| PyValueError::new_err(format!("{} is not a valid SVG", path.display())))?;
    let mut pixmap = Pixmap::new(size, size).ok_or_else(|| PyValueError::new_err("empty size"))?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let k = size as f32 / w.max(h);
    let t = Transform::from_row(k, 0., 0., k, (size as f32 - w * k) / 2., (size as f32 - h * k) / 2.);
    resvg::render(&tree, t, &mut pixmap.as_mut());
    let mut pixels = image::RgbaImage::from_raw(size, size, pixmap.take()).ok_or_else(|| PyValueError::new_err("bad raster"))?;
    crate::image_assets::unpremultiply(&mut pixels);
    Ok((size, size, PyBytes::new(py, pixels.as_raw())))
}

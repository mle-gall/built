// SPDX-License-Identifier: GPL-3.0-or-later
use crate::document_registration::view::calls;
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};
mod geometry;
pub(super) use geometry::component_signature;
use resvg::tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

type Polygon = (f32, Vec<(f32, f32)>, [f32; 3], f32);

#[pyfunction]
pub(crate) fn component_pixels<'py>(
    definition: &Bound<'py, PyAny>,
    size: u32,
) -> PyResult<Bound<'py, PyBytes>> {
    let image = preview(definition, size, 3000)?;
    Ok(PyBytes::new(definition.py(), image.data()))
}

/// PNG export and CPU readers take straight alpha; the GPU uses premultiplied pixels.
#[pyfunction]
pub(crate) fn component_raster(
    definition: &Bound<'_, PyAny>,
    size: u32,
) -> PyResult<crate::survey_raster::SurveyRaster> {
    raster(definition, size, 3000)
}
fn raster(
    definition: &Bound<PyAny>,
    size: u32,
    budget: usize,
) -> PyResult<crate::survey_raster::SurveyRaster> {
    let image = preview(definition, size, budget)?;
    let mut pixels = image::RgbaImage::new(size, size);
    for (target, source) in pixels.pixels_mut().zip(image.pixels()) {
        let color = source.demultiply();
        *target = image::Rgba([color.red(), color.green(), color.blue(), color.alpha()]);
    }
    Ok(crate::survey_raster::SurveyRaster { pixels })
}

fn preview(definition: &Bound<'_, PyAny>, size: u32, budget: usize) -> PyResult<Pixmap> {
    let encoded = embedded_preview(definition)?;
    if let Some(mut image) = Pixmap::new(size.min(1024), size.min(1024)).filter(|_| size <= 1024) {
        if decoded_into(&mut image, encoded.as_deref()) { return Ok(image); }
    }
    draw_polygons(geometry::polygons(definition, budget)?, size)
}

fn embedded_preview(definition: &Bound<'_, PyAny>) -> PyResult<Option<Vec<u8>>> {
    let py = definition.py();
    let encoded = definition
        .getattr_opt("ext")?
        .filter(|v| !v.is_none())
        .and_then(|v| v.get_item("rfa").ok())
        .and_then(|v| v.get_item("preview_png_base64").ok());
    Ok(encoded.filter(|s| s.len().is_ok_and(|n| n > 0 && n <= 16 << 20)).and_then(|text|
        py.import("base64").and_then(|m| m.call_method1("b64decode", (text,))).and_then(|v| v.extract()).ok()))
}

fn decoded_into(image: &mut Pixmap, encoded: Option<&[u8]>) -> bool {
    let Some(source) = encoded.and_then(crate::image_io::decode) else { return false };
    let size = image.width();
    let pixels = source.thumbnail(size, size).to_rgba8();
    let (w, h) = pixels.dimensions();
    let (dx, dy) = ((size - w) / 2, (size - h) / 2);
    for (x, y, p) in pixels.enumerate_pixels() {
        let i = (((y + dy) * size + x + dx) * 4) as usize;
        let [r, g, b, a] = p.0;
        let [r, g, b] = [r, g, b].map(|v| ((v as u16 * a as u16 + 127) / 255) as u8);
        image.data_mut()[i..i + 4].copy_from_slice(&[r, g, b, a]);
    }
    true
}

fn draw_polygons(mut polys: Vec<Polygon>, size: u32) -> PyResult<Pixmap> {
    let mut image = Pixmap::new(size.min(1024), size.min(1024))
        .filter(|_| size <= 1024)
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("Invalid preview size"))?;
    polys.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    for (_, xy, _, _) in &polys {
        for &(x, y) in xy {
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
        }
    }
    let scale = (size as f32 - 4.).max(1.) / (hi[0] - lo[0]).max(hi[1] - lo[1]).max(f32::EPSILON);
    let project =
        |v: f32, axis: usize| (v - (lo[axis] + hi[axis]) * 0.5) * scale + size as f32 * 0.5;
    for (_, xy, rgb, lam) in polys {
        let mut path = PathBuilder::new();
        for (i, (x, y)) in xy.into_iter().enumerate() {
            let (x, y) = (project(x, 0), project(y, 1));
            if i == 0 {
                path.move_to(x, y);
            } else {
                path.line_to(x, y);
            }
        }
        path.close();
        let Some(path) = path.finish() else { continue };
        let [r, g, b] = rgb.map(|v| (255. * v * (0.5 + 0.5 * lam)).round() as u8);
        let mut paint = Paint::default();
        paint.set_color_rgba8(r, g, b, 255);
        image.fill_path(
            &path,
            &paint,
            FillRule::EvenOdd,
            Transform::identity(),
            None,
        );
        paint.set_color_rgba8(r - r / 3, g - g / 3, b - b / 3, 255);
        image.stroke_path(
            &path,
            &paint,
            &Stroke {
                width: 0.6,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
    Ok(image)
}

/// Cache by mesh identity and drawing inputs, without holding document references.
#[pyfunction]
#[pyo3(signature=(definition,size=40,*,max_faces=3000))]
pub(super) fn component_thumbnail<'p>(
    definition: &Bound<'p, PyAny>,
    size: u32,
    max_faces: usize,
) -> PyResult<Bound<'p, PyAny>> {
    let py = definition.py();
    let cache = py
        .import("built_render")?
        .getattr("_component_previews")?
        .cast_into::<PyDict>()?;
    let sig = component_signature(definition)?;
    let image = definition
        .getattr_opt("ext")?
        .and_then(|v| v.get_item("rfa").ok())
        .and_then(|r| r.get_item("preview_png_base64").ok());
    let key = (sig[0].0, size, max_faces);
    let signature = (sig, image).into_pyobject(py)?;
    if let Some(hit) = cache.get_item(key)? {
        if hit.get_item(0)?.eq(&signature)? {
            return hit.get_item(1);
        }
    }
    let pic = Bound::new(py, raster(definition, size, max_faces)?)?;
    if cache.len() >= 4096 {
        cache.clear();
    }
    cache.set_item(key, (signature, &pic))?;
    Ok(pic.into_any())
}

pub(crate) fn upload(ctx: &Bound<'_, PyAny>, icons: &[String]) -> PyResult<()> {
    let scene = ctx.getattr("scene")?;
    let prefix = format!(
        "component:{}:{}:",
        scene.as_ptr() as usize,
        scene.getattr("content_version")?
    );
    let view = ctx.getattr("view")?;
    let rows = calls::components_in_model(&view, &ctx.getattr("win")?)?;
    let renderer = view.getattr("renderer")?;
    let native = renderer.cast::<crate::Renderer>().ok();
    let start = std::time::Instant::now();
    let mut batch = Vec::new();
    for key in icons {
        // Snapshot document geometry on its owning thread, yielding between definitions.
        // Rows still missing their texture request it again on the next frame.
        if native.is_some() && start.elapsed() >= std::time::Duration::from_millis(2) { break; }
        let Some(i) = key
            .strip_prefix(&prefix)
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|&i| i < rows.len().unwrap_or(0))
        else {
            continue;
        };
        let row = rows.get_item(i)?;
        let groups = row.get_item(2)?;
        let definition = if groups.len()? > 0 {
            groups.get_item(0)?
        } else {
            row.get_item(3)?
        };
        if let Some(renderer) = native {
            if !renderer.borrow().ui.as_ref().is_some_and(|ui| ui.needs_component_preview(key)) { continue; }
            let encoded = embedded_preview(&definition)?;
            let polygons = geometry::polygons(&definition, 3000)?;
            if let Some(ui) = &mut renderer.borrow_mut().ui {
                ui.request_component_preview(key, move || {
                    let mut image = Pixmap::new(96, 96)?;
                    if !decoded_into(&mut image, encoded.as_deref()) {
                        image = draw_polygons(polygons, 96).ok()?;
                    }
                    Some(egui::ColorImage::from_rgba_premultiplied([96, 96], image.data()))
                });
            }
            continue;
        }
        batch.push((key, 96, 96, component_pixels(&definition, 96)?));
    }
    if !batch.is_empty() {
        view.getattr("renderer")?
            .call_method1("ui_set_icons", (batch,))?;
    }
    Ok(())
}

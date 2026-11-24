// SPDX-License-Identifier: GPL-3.0-or-later
//! Adapter to the Sheets frame-picture backend. The scene menu owns cache
//! lifetime, visibility, retries and textures; the backend owns projection.
use pyo3::prelude::*;
type Obj<'py> = Bound<'py, PyAny>;
pub type Pixels = (usize, usize, Vec<u8>);

#[pyfunction]
pub fn scene_thumbnail_icon<'py>(window: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    let py = window.py();
    let Some(vp) = window.getattr_opt("viewport")?.filter(|v| !v.is_none()) else {
        return Ok(py.None().into_bound(py));
    };
    let painter = if let Some(p) = window.getattr_opt("_composer")?.filter(|v| !v.is_none()) {
        p
    } else if let Some(p) = window
        .getattr_opt("_scene_thumbnailer")?
        .filter(|v| !v.is_none())
    {
        p
    } else {
        let p = py
            .import("views.composer")?
            .getattr("SceneThumbnailer")?
            .call1((window,))?;
        window.setattr("_scene_thumbnailer", &p)?;
        p
    };
    if let Some(cache) = painter.getattr_opt("_thumbs")? {
        cache.call_method1("pop", (name, py.None()))?;
    }
    let cam = vp.getattr("camera")?;
    let lens = cam.call_method0("lens_dict")?;
    let result = painter.call_method1("scene_thumbnail", (name,));
    cam.call_method1("apply_lens", (lens,))?;
    result
}

pub fn thumbnail_mode(py: Python<'_>) -> PyResult<bool> {
    let raw = crate::navigation::setting(py, "scenes/thumbnails", "true")?;
    Ok(!matches!(raw.trim().to_lowercase().as_str(), "false" | "0"))
}
#[pyfunction]
pub fn save_thumbnail_mode(py: Python<'_>, on: bool) -> PyResult<()> {
    py.import("core.settings")?
        .getattr("settings")?
        .call0()?
        .call_method1("setValue", ("scenes/thumbnails", on))?;
    Ok(())
}
#[pyfunction]
pub fn render_scene_photo(window: &Obj<'_>, request: &str) -> PyResult<Option<Pixels>> {
    let py = window.py();
    let (owner, revision, name): (String, String, String) = serde_json::from_str(request)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
    let scene = window.getattr("viewport")?.getattr("scene")?;
    if owner != (scene.as_ptr() as usize).to_string()
        || revision != scene.getattr("content_version")?.str()?.to_str()?
    {
        return Ok(None);
    }
    let display = py
        .import("views.chrome_dialogs")?
        .call_method1("view_of", (window,))?;
    if display.is_none() {
        return Ok(None);
    }
    let opts = pyo3::types::PyDict::new(py);
    opts.set_item("w_mm", 36.)?;
    opts.set_item("h_mm", 24.)?;
    opts.set_item("view_key", format!("scene:{name}"))?;
    opts.set_item("style", "sombreado")?;
    opts.set_item("paper_bg", true)?;
    let frame = py
        .import("core.composition")?
        .getattr("MarcoVista")?
        .call((), Some(&opts))?;
    let frames = py.import("views.wgpu_sheet_frames")?;
    let saved = frames.call_method1("saved_view", (&scene, &frame))?;
    if !saved.is_truthy()? {
        return Ok(None);
    }
    let scale = py
        .import("core.outliner")?
        .call_method1("fit_scale", (&scene, saved, 36., 24., 0.95))?;
    frame.setattr("scale_n", scale)?;
    let picture = frames.call_method1("picture", (display, frame))?;
    if picture.is_none() {
        return Ok(None);
    }
    let (w, h, rgba) = picture.extract::<(u32, u32, Vec<u8>)>()?;
    let image = image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err("invalid scene picture dimensions")
    })?;
    let photo = py.detach(|| crate::image_assets::resize(image, 96, 64));
    Ok(Some((96, 64, photo.into_raw())))
}

/// Papers and menu thumbnails have already been composited over the frame background.
#[pyfunction]
pub fn opaque_frame<'py>(
    py: Python<'py>,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
    if w == 0
        || h == 0
        || u64::from(w) * u64::from(h) != (rgba.len() / 4) as u64
        || rgba.len() % 4 != 0
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid frame dimensions",
        ));
    }
    pyo3::types::PyBytes::new_with(py, rgba.len(), |dst| {
        dst.copy_from_slice(rgba);
        for alpha in dst[3..].iter_mut().step_by(4) {
            *alpha = 255;
        }
        Ok(())
    })
}

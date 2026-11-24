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
    let icon = scene_thumbnail_icon(window, &name)?;
    if icon.is_none() {
        return Ok(None);
    }
    let fmt = py
        .import("PySide6.QtGui")?
        .getattr("QImage")?
        .getattr("Format_RGBA8888")?;
    let img = icon
        .call_method1("pixmap", (96, 64))?
        .call_method0("toImage")?
        .call_method1("convertToFormat", (fmt,))?;
    let (w, h): (usize, usize) = (
        img.call_method0("width")?.extract()?,
        img.call_method0("height")?.extract()?,
    );
    if w == 0 || h == 0 {
        return Ok(None);
    }
    let pixels: Vec<u8> = img
        .call_method0("constBits")?
        .call_method0("tobytes")?
        .extract()?;
    Ok(Some((w, h, pixels)))
}

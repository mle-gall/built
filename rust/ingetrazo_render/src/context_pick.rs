// SPDX-License-Identifier: GPL-3.0-or-later
//! Context menu selection for both viewport hosts.
use pyo3::{prelude::*, types::PyDict};

#[pyfunction]
pub fn context_pick<'py>(view: &Bound<'py, PyAny>, x: f64, y: f64) -> PyResult<Bound<'py, PyAny>> {
    let pixel = (x, y);
    let edge = view.call_method1("pick_edge", pixel)?;
    let loose = if edge.is_truthy()? {
        edge
    } else {
        view.call_method1("pick_face", pixel)?
    };
    if !loose.is_none()
        && view
            .getattr("scene")?
            .getattr("selection")?
            .contains(&loose)?
    {
        return Ok(loose);
    }

    let options = PyDict::new(view.py());
    options.set_item("rect_only", true)?;
    let mut picked = view.call_method("pick_text_label", pixel, Some(&options))?;
    for method in [
        "pick_group",
        "pick_edge",
        "pick_geopath",
        "pick_dimension",
        "pick_text_label",
        "pick_section_plane",
        "pick_guide",
        "_face_or_image_in_front",
        "pick_image_plane",
        "pick_locked_image_plane",
    ] {
        if picked.is_truthy()? {
            break;
        }
        picked = view.call_method1(method, pixel)?;
    }
    if !picked.is_none() {
        let scene = view.getattr("scene")?;
        if !scene.getattr("selection")?.contains(&picked)? {
            scene.call_method1("select", (vec![picked.clone()],))?;
            view.call_method0("update")?;
        }
    }
    Ok(picked)
}

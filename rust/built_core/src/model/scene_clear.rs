// SPDX-License-Identifier: GPL-3.0-or-later
//! `Scene.clear()` (`core/scene.py`): every open group edit left, then,
//! when anything is there, every table, flag and record reset to a new
//! document's, the version moved once; the cameras, units, plugin data and
//! links registry forgotten in any case. The checks run in the Python's
//! order and stop at the first that holds (reading a deferred mesh's
//! edges builds it). `tests/test_scene_clear_golden.py`.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use super::module;

type Obj<'py> = Bound<'py, PyAny>;

/// Whether anything is there to clear.
fn busy(scene: &Obj<'_>) -> PyResult<bool> {
    let py = scene.py();
    let mesh = scene.getattr("mesh")?;
    if mesh.getattr("edges")?.is_truthy()? || mesh.getattr("faces")?.is_truthy()? {
        return Ok(true);
    }
    for name in ["selection", "groups", "dimensions", "georef", "tile_layer", "geo_paths", "terrain", "guides",
                 "geo_points", "text_labels", "saved_views", "compositions", "scene_folders", "image_planes",
                 "plugin_data", "link_registry", "kept_components", "section_planes", "materials"] {
        if scene.getattr(name)?.is_truthy()? {
            return Ok(true);
        }
    }
    if scene.getattr("animation")?.call_method0("to_dict")?.is_truthy()?
        || !scene.getattr("show_section_planes")?.is_truthy()?
        || !scene.getattr("show_section_cuts")?.is_truthy()?
        || scene.getattr("show_hidden_objects")?.is_truthy()?
        || scene.getattr("show_hidden_geometry")?.is_truthy()?
        || scene.getattr("custom_scales")?.is_truthy()?
        || !scene.getattr("back_face_color")?.is_none()
        || scene.getattr("layers")?.len()? > 1
    {
        return Ok(true);
    }
    for name in ["layer_folders", "active_layer", "hatch_patterns", "line_types", "linetype_colors", "bricks", "north_deg"] {
        if scene.getattr(name)?.is_truthy()? {
            return Ok(true);
        }
    }
    let _ = py;
    Ok(false)
}

#[pyfunction]
pub fn scene_clear(scene: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    scene.call_method0("end_group_edit")?;
    if busy(scene)? {
        scene.getattr("mesh")?.call_method0("clear")?;
        for name in ["groups", "dimensions", "text_labels", "text_placement_anchors", "geo_paths", "geo_points",
                     "guides", "image_planes", "saved_views"] {
            scene.getattr(name)?.call_method0("clear")?;
        }
        scene.setattr("scene_folders", PyList::empty(py))?;
        scene.setattr("scene_folder_parents", PyDict::new(py))?;
        for name in ["compositions", "custom_scales", "kept_components", "materials"] {
            scene.getattr(name)?.call_method0("clear")?;
        }
        scene.setattr("plugin_data", PyDict::new(py))?;
        scene.setattr("link_registry", PyDict::new(py))?;
        scene.getattr("selection")?.call_method0("clear")?;
        let layers = module(py, "built_core.layers")?;
        let default = layers.getattr("Layer")?.call1((layers.getattr("DEFAULT_LAYER")?,))?;
        scene.setattr("layers", PyList::new(py, [default])?)?;
        scene.setattr("active_layer", py.None())?;
        scene.setattr("layer_folders", PyList::empty(py))?;
        for name in ["folder_parents", "folder_visibility", "layer_folder_uids", "layer_folder_names",
                     "hatch_patterns", "line_types", "linetype_colors", "bricks"] {
            scene.setattr(name, PyDict::new(py))?;
        }
        for name in ["georef", "tile_layer", "terrain", "back_face_color", "active_ifc"] {
            scene.setattr(name, py.None())?;
        }
        let own = module(py, "built_core.scene")?;
        scene.setattr("display_style", own.call_method0("_make_style")?)?;
        scene.setattr("shadows", own.call_method0("_make_shadows")?)?;
        scene.setattr("animation", own.call_method0("_make_animation")?)?;
        scene.getattr("section_planes")?.call_method0("clear")?;
        scene.setattr("show_section_planes", true)?;
        scene.setattr("show_section_cuts", true)?;
        scene.setattr("show_hidden_objects", false)?;
        scene.setattr("show_hidden_geometry", false)?;
        scene.setattr("show_axes", true)?;
        scene.setattr("north_deg", 0.0)?;
        scene.setattr("version", scene.getattr("version")?.add(1)?)?;
    }
    // Outside the guard: an empty document has a camera to forget too.
    scene.setattr("camera_home", py.None())?;
    scene.setattr("views_home", py.None())?;
    let units = PyDict::new(py);
    units.set_item("length", "m")?;
    units.set_item("precision", 2)?;
    scene.setattr("units", units)?;
    scene.setattr("plugin_data", PyDict::new(py))?;
    scene.setattr("link_registry", PyDict::new(py))?;
    // A new document: a new identity at its first save.
    scene.setattr("doc_id", py.None())?;
    scene.setattr("doc_ext", py.None())?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scene_clear, m)?)
}

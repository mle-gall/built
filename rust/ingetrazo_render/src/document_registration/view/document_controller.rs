// SPDX-License-Identifier: GPL-3.0-or-later
//! Host bindings for document_controller.
use super::*;
fn document_controller_set_updates_enabled<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.setattr("_updates_enabled", a.get_item(1)?)?;
    Ok(none(a.py()))
}
fn document_controller_updates_enabled<'py>(a: &Args<'py>) -> Reply<'py> {
    match a.get_item(0)?.getattr_opt("_updates_enabled")? {
        Some(value) => Ok(value),
        None => true.into_bound_py_any(a.py()),
    }
}
fn document_controller_reset_texture_cache<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    view.call_method0("reset_document_caches")?;
    view.setattr(
        "_reference_revision",
        view.getattr_opt("_reference_revision")?
            .unwrap_or((0).into_bound_py_any(py)?)
            .add(1)?,
    )?;
    view.call_method0("update")?;
    Ok(none(py))
}

fn document_controller_depth_world_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let kind: Obj<'py>;
    let point: Obj<'py>;
    let view = a.get_item(0)?;
    let x = a.get_item(1)?;
    let y = a.get_item(2)?;
    let display = view.getattr("keys_stand_in")?;
    if display.is_none() || display.getattr("renderer")?.is_none() {
        return Ok(none(py));
    }
    let painted = optional(&display, "_last_camera")?;
    let camera = display.getattr("camera")?;
    if painted.is_none()
        || painted.get_item(0)?.ne((
            py.get_type::<pyo3::types::PyTuple>()
                .into_any()
                .call1((camera.call_method0("view_matrix")?.call_method0("data")?,))?,
            py.get_type::<pyo3::types::PyTuple>()
                .into_any()
                .call1((camera
                    .call_method0("projection_matrix")?
                    .call_method0("data")?,))?,
        )
            .into_bound_py_any(py)?)?
    {
        return Ok(none(py));
    }
    if !(((0).into_bound_py_any(py)?.le(&x)? && x.lt(display.call_method0("width")?)?)
        && ((0).into_bound_py_any(py)?.le(&y)? && y.lt(display.call_method0("height")?)?))
    {
        return Ok(none(py));
    }
    {
        let [part0, part1] = unpack::<2>(&display.call_method1("_depth_world_at", (&x, &y))?)?;
        kind = part0;
        point = part1;
    }
    Ok(if kind.eq("miss")? {
        ("background", py.None()).into_bound_py_any(py)?
    } else {
        (&kind, &point).into_bound_py_any(py)?
    })
}

fn document_controller_upload_terrain<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    view.setattr(
        "_reference_revision",
        view.getattr_opt("_reference_revision")?
            .unwrap_or((0).into_bound_py_any(py)?)
            .add(1)?,
    )?;
    view.call_method0("update")?;
    Ok(none(py))
}

fn document_controller_upload_photo_mesh<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let images = a.get_item(2)?;
    let mesh = a.get_item(1)?;
    let view = a.get_item(0)?;
    if !mesh.is_none() && !images.is_none() {
        mesh.setattr("images", &images)?;
    }
    view.setattr(
        "_reference_revision",
        view.getattr_opt("_reference_revision")?
            .unwrap_or((0).into_bound_py_any(py)?)
            .add(1)?,
    )?;
    view.call_method0("update")?;
    Ok(none(py))
}

fn document_controller_release_photo_textures<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    view.setattr(
        "_reference_revision",
        view.getattr_opt("_reference_revision")?
            .unwrap_or((0).into_bound_py_any(py)?)
            .add(1)?,
    )?;
    view.call_method0("update")?;
    Ok(none(py))
}

fn document_controller_max_texture_size<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let renderer = optional(&view.getattr("keys_stand_in")?, "renderer")?;
    Ok(if !renderer.is_none() {
        renderer.call_method0("max_texture_size")?
    } else {
        (4096).into_bound_py_any(py)?
    })
}

fn document_controller_photo_showing<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let mesh = optional(&view.getattr("scene")?, "photo_mesh")?;
    (mesh.is_truthy()?
        && mesh.getattr("visible")?.is_truthy()?
        && mesh.getattr("triangle_count")?.is_truthy()?
        && view
            .getattr("scene")?
            .call_method1("entity_visible", (&mesh,))?
            .is_truthy()?)
    .into_bound_py_any(py)
}

fn document_controller_render_raster<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let annotations_only = a.get_item(4)?;
    let crop_to_frame = a.get_item(7)?;
    let height_px = a.get_item(2)?;
    let overlays = a.get_item(3)?;
    let view = a.get_item(0)?;
    let sharp_shadows = a.get_item(5)?;
    let transparent = a.get_item(6)?;
    let width_px = a.get_item(1)?;
    let survey_raster = py.import("ingetrazo_render")?.getattr("SurveyRaster")?;
    if crop_to_frame.is_truthy()? && height_px.is_none() {
        return Ok(view.call_method(
            "_render_framed_image",
            (
                &width_px,
                &overlays,
                &annotations_only,
                &sharp_shadows,
                &transparent,
            ),
            Some(&fields!(py; "raster" => true)),
        )?);
    }
    let frame = view.getattr("keys_stand_in")?.call_method("render_pixels", (&width_px, &height_px,), Some(&fields!(py; "overlays" => &overlays, "sharp_shadows" => &sharp_shadows, "transparent" => &transparent, "annotations_only" => &annotations_only)))?;
    Ok(if frame.is_truthy()? {
        survey_raster.call1(positional(&frame)?)?
    } else {
        none(py)
    })
}

fn document_controller_render_image<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let annotations_only = a.get_item(4)?;
    let crop_to_frame = a.get_item(7)?;
    let height_px = a.get_item(2)?;
    let overlays = a.get_item(3)?;
    let view = a.get_item(0)?;
    let sharp_shadows = a.get_item(5)?;
    let transparent = a.get_item(6)?;
    let width_px = a.get_item(1)?;
    if crop_to_frame.is_truthy()? && height_px.is_none() {
        return Ok(view.call_method1(
            "_render_framed_image",
            (
                &width_px,
                &overlays,
                &annotations_only,
                &sharp_shadows,
                &transparent,
            ),
        )?);
    }
    Ok(view.getattr("keys_stand_in")?.call_method("render_image", (&width_px, &height_px,), Some(&fields!(py; "overlays" => &overlays, "sharp_shadows" => &sharp_shadows, "transparent" => &transparent, "annotations_only" => &annotations_only)))?)
}

#[rustfmt::skip]
pub(super) const DOCUMENT_CONTROLLER: &[Method] = &[
    ("setUpdatesEnabled", Mode::Method, document_controller_set_updates_enabled, &["self", "enabled"], "", ""),
    ("updatesEnabled", Mode::Method, document_controller_updates_enabled, &["self"], "", ""),
    ("reset_texture_cache", Mode::Method, document_controller_reset_texture_cache, &["self"], "", ""),
    ("_depth_world_at", Mode::Method, document_controller_depth_world_at, &["self", "x", "y"], "", ""),
    ("upload_terrain", Mode::Method, document_controller_upload_terrain, &["self", "terrain"], "", ""),
    ("upload_photo_mesh", Mode::Method, document_controller_upload_photo_mesh, &["self", "mesh", "images=None"], "", ""),
    ("release_photo_textures", Mode::Method, document_controller_release_photo_textures, &["self"], "", ""),
    ("max_texture_size", Mode::Method, document_controller_max_texture_size, &["self"], "", ""),
    ("_photo_showing", Mode::Method, document_controller_photo_showing, &["self"], "", ""),
    ("render_raster", Mode::Method, document_controller_render_raster, &["self", "width_px", "height_px=None", "overlays=True", "annotations_only=False", "sharp_shadows=False", "transparent=False", "crop_to_frame=False"], "", ""),
    ("render_image", Mode::Method, document_controller_render_image, &["self", "width_px", "height_px=None", "overlays=True", "annotations_only=False", "sharp_shadows=False", "transparent=False", "crop_to_frame=False"], "", ""),
];

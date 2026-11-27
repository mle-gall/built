// SPDX-License-Identifier: GPL-3.0-or-later
//! Native format and georeferencing entry points.
#[cfg(unix)]
use crate::file_archive;
use crate::{
    alignment, atomic_document, dae_read, dae_write, dxf_read, file_annotations, file_blocks,
    file_bounds, file_clip, file_deferred, file_document, file_geometry, file_groups, file_json,
    file_links, file_members, file_mesh, file_model, file_open, file_pack, file_precision,
    file_prepare, file_read, file_save, file_scene, file_sheet, file_snapshot, file_styles,
    file_textures, fuse, gltf_maps, gltf_mesh, gltf_read, gltf_write, igz, mesh_export, obj_read,
    obj_write, reader_compat, rfa_scene, skp, stl, survey_pixels, view_dxf, view_svg,
};
macro_rules! functions {
    ($module:ident; $($name:ident),+ $(,)?) => { $( $module.add_function(pyo3::wrap_pyfunction!($name, $module)?)?; )+ };
}
type Obj<'p> = Bound<'p, PyAny>;
fn path_obj<'p>(py: Python<'p>, p: &std::path::Path) -> PyResult<Obj<'p>> {
    py.import("pathlib")?.call_method1("Path", (p,))
}
mod datum;
mod bindings;
mod codec_modules;
mod module_state;
mod document_types;
mod delivery;
mod geotiff;
mod ground;
mod paths;
mod photo;
mod services;
mod tiles;
mod survey_api;
use pyo3::prelude::*;

pub(super) fn register_formats(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(file_scene::igz_archive_scene_fields, m)?)?;
    m.add_function(wrap_pyfunction!(file_scene::igz_reset_scene, m)?)?;
    m.add_function(wrap_pyfunction!(file_scene::igz_restore_views, m)?)?;
    m.add_function(wrap_pyfunction!(file_scene::igz_finish_scene, m)?)?;
    m.add_function(wrap_pyfunction!(codec_modules::install_codec_modules,m)?)?;
    file_mesh::register(m)?;
    file_precision::register(m)?;
    gltf_read::register(m)?;
    dae_write::register(m)?;
    dae_read::register(m)?;
    dxf_read::register(m)?;
    view_svg::register(m)?;
    view_dxf::register(m)?;
    m.add_function(wrap_pyfunction!(file_model::dae_add_fused, m)?)?;
    file_save::register(m)?;
    file_clip::register(m)?;
    rfa_scene::register(m)?;
    file_snapshot::register(m)?;
    file_deferred::register(m)?;
    file_pack::register(m)?;
    file_members::register(m)?;
    file_document::register(m)?;
    file_blocks::register(m)?;
    file_prepare::register(m)?;
    reader_compat::register(m)?;
    file_links::register(m)?;
    file_read::register(m)?;
    file_bounds::register(m)?;
    fuse::register(m)?;
    obj_read::register(m)?;
    obj_write::register(m)?;
    mesh_export::register(m)?;
    gltf_maps::register(m)?;
    gltf_mesh::register(m)?;
    gltf_write::register(m)?;
    crate::step::register(m)?;
    stl::register(m)?;
    alignment::register(m)?;
    survey_pixels::register(m)?;
    m.add_class::<file_open::DocumentOpen>()?;
    m.add_function(wrap_pyfunction!(skp::skp_tag_folders, m)?)?;
    skp::register_dispatch(m)?;
    m.add_function(wrap_pyfunction!(skp::skp_apply, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_apply_folders, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_apply_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_apply_metadata, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_materials, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_read_pbr, m)?)?;
    m.add_class::<skp::SkpInsertion>()?;
    m.add_class::<skp::SkpArchiveCursor>()?;
    m.add_function(wrap_pyfunction!(skp::skp_mfc_object, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_mfc_readers, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_face_entry, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_parse, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_adapt_inner, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_collect, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_loose_segments, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_soft_segments, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_curve_segments, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_instance_identity, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_world_points, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_defn_geom, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_merge_protos, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_file_layers, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_empty_model, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_texture_dir, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_cached_colorize, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_needs_tint, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_inverted_png, m)?)?;
    m.add_class::<skp::FlatGeometry>()?;
    m.add_class::<skp::PrototypeGeometry>()?;
    m.add_function(wrap_pyfunction!(skp::image_stage_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_vff_model, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_image_faces, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_image_bounds, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_image_attached, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_mark_drapes, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_ring, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_positioned_uvs, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_instance_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_model_annotations, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_resolve_annotations, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_annotations, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_definition_spans, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_definition, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_pages, m)?)?;
    m.add_function(wrap_pyfunction!(skp::skp_geometry_nodes, m)?)?;
    #[cfg(unix)]
    m.add_function(wrap_pyfunction!(file_archive::igz_write_zip, m)?)?;
    #[cfg(unix)]
    m.add_class::<atomic_document::AtomicDocument>()?;
    #[cfg(unix)]
    m.add_function(wrap_pyfunction!(file_archive::igz_write_plain, m)?)?;
    #[cfg(unix)]
    m.add_function(wrap_pyfunction!(file_archive::igz_read_member, m)?)?;
    #[cfg(unix)]
    m.add_class::<file_archive::DocumentReader>()?;
    #[cfg(unix)]
    m.add_class::<file_archive::ArchiveProgress>()?;
    m.add_function(wrap_pyfunction!(file_annotations::igz_load_annotations, m)?)?;
    m.add_function(wrap_pyfunction!(
        file_annotations::igz_archive_annotations,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(file_annotations::highest_group_number, m)?)?;
    m.add_function(wrap_pyfunction!(file_styles::igz_archive_layer, m)?)?;
    m.add_function(wrap_pyfunction!(file_styles::igz_load_layer, m)?)?;
    m.add_function(wrap_pyfunction!(file_styles::igz_archive_material, m)?)?;
    m.add_function(wrap_pyfunction!(file_styles::igz_load_material, m)?)?;
    m.add_function(wrap_pyfunction!(file_sheet::igz_load_sheet, m)?)?;
    m.add_function(wrap_pyfunction!(file_sheet::igz_archive_sheet, m)?)?;
    m.add_function(wrap_pyfunction!(file_textures::igz_pack_textures, m)?)?;
    m.add_function(wrap_pyfunction!(file_textures::igz_unpack_textures, m)?)?;
    m.add_function(wrap_pyfunction!(file_textures::igz_texture_slots, m)?)?;
    m.add_function(wrap_pyfunction!(file_textures::igz_texture_entries, m)?)?;
    m.add_function(wrap_pyfunction!(file_json::igz_json_loads, m)?)?;
    m.add_function(wrap_pyfunction!(file_json::igz_encode_json, m)?)?;
    m.add_class::<file_json::FileMatrix>()?;
    m.add_class::<file_groups::FileGroups>()?;
    m.add_class::<file_groups::FileGroupWriter>()?;
    m.add_function(wrap_pyfunction!(file_geometry::igz_points, m)?)?;
    m.add_function(wrap_pyfunction!(igz::igz_read_geometry, m)?)?;
    geotiff::register(m)?;
    photo::register(m)?;
    datum::register(m)?;
    tiles::register(m)?;
    paths::register(m)?;
    ground::register(m)?;
    services::register(m)?;
    delivery::register(m)?;
    document_types::register(m)?;
    m.add_function(wrap_pyfunction!(survey_api::register_survey, m)?)?;
    Ok(())
}

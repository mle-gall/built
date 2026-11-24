// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Native kernels for IngeTrazo. Optional: `core/_native.py` imports this
//! module when it is built and falls back to the Python code otherwise.
//!
//! Every kernel here mirrors a Python function line by line — the same
//! arithmetic in the same order — so both give the same drawing. The only
//! thing a kernel may add is an index that finds the SAME candidates faster.

use numpy::{PyArray1, PyArray2, PyReadonlyArray2, PyReadonlyArray3, PyReadonlyArray1};
use pyo3::prelude::*;
use numpy::IntoPyArray;
use pyo3::types::PyBytes;

mod math;
mod texture_position;
mod navigation;
mod recents;
mod builtproject;
mod delivery_form;
mod command_search;
mod location;
mod boxes;
mod doc;
mod earcut;
mod facegeom;
mod render_bake;
mod scene_bake;
mod smooth;
mod facesview;
mod hlr;
mod igz;
#[cfg(unix)]
mod file_archive;
mod file_json;
mod file_open;
mod file_geometry;
mod file_textures;
mod file_sheet;
mod file_groups;
mod group_geometry;
mod structure;
mod project_tree;
mod selection_identity;
mod sidebar;
mod hatch_file;
mod style_files;
mod style_edit;
mod settings;
mod document_watch;
mod window_input;
mod window_commands;
mod scene_animation;
mod atomic_document;
mod scene_video;
mod mp4;
mod survey_pixels;
mod filing_rows;
mod readiness;
mod project_sections;
mod project_sheets;
mod link_subset;
mod presentation;
mod object_replay;
mod object_drawing;
mod planar_edits;
mod visibility_sets;
mod scene_commands;
mod material_library;
mod swatch;
mod assembly_inspector;
mod solid_inspector;
mod scene_tree;
mod structure_bootstrap;
mod placement_edits;
mod placement_transform;
mod reference_layers;
mod definition_edits;
mod file_styles;
mod file_annotations;
mod instances;
mod jsondoc;
mod occlusion;
mod parity;
mod visible;
mod weld;
mod skp;
mod snap_build;
mod snap;
mod snap_engine;
mod tools;
mod pick;
mod units;
mod value_field;
mod inference_state;
mod snap_curve;
mod snap_planes;
mod pack;
mod pack_materials;
mod store;
mod scans;
mod orient;
mod pushscan;
mod push_command;
pub mod pyset;
mod push_guard;
mod object_weld;
mod object_merge;
mod panic_log;
mod sheets;

#[pymodule]
fn ingetrazo_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    scene_animation::register(m)?;
    survey_pixels::register(m)?;
    m.add_class::<scene_video::MkvWriter>()?;
    m.add_class::<mp4::Mp4Writer>()?;
    m.add_function(wrap_pyfunction!(scene_video::video_writer, m)?)?;
    m.add_function(wrap_pyfunction!(mp4::video_format, m)?)?;
    builtproject::register(m)?;
    m.add_class::<settings::NativeSettings>()?;
    m.add_class::<document_watch::DocumentWatch>()?;
    m.add_class::<window_input::WindowInput>()?;
    m.add_class::<window_commands::WindowCommands>()?;
    m.add_class::<window_commands::WindowCommand>()?;
    m.add_class::<file_open::DocumentOpen>()?;
    m.add_function(wrap_pyfunction!(skp::skp_tag_folders, m)?)?;
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
    m.add_class::<command_search::SearchItem>()?;
    m.add_function(wrap_pyfunction!(command_search::damerau_levenshtein, m)?)?;
    m.add_function(wrap_pyfunction!(command_search::rank_commands, m)?)?;
    m.add_function(wrap_pyfunction!(command_search::command_score, m)?)?;
    m.add_function(wrap_pyfunction!(command_search::fuzzy_errors, m)?)?;
    m.add_class::<location::LocationDraft>()?;
    m.add_class::<location::LocationEdit>()?;
    m.add_function(wrap_pyfunction!(location::zone_for_lon, m)?)?;
    m.add_function(wrap_pyfunction!(location::utm_forward, m)?)?;
    m.add_function(wrap_pyfunction!(location::utm_inverse, m)?)?;
    m.add_function(wrap_pyfunction!(pyset::py_set_replay, m)?)?;
    m.add_function(wrap_pyfunction!(pyset::py_pair_set_order, m)?)?;
    m.add_function(wrap_pyfunction!(store::arrangement::arrange_plane, m)?)?;
    m.add_function(wrap_pyfunction!(store::arrangement::plane_split_graph, m)?)?;
    m.add_function(wrap_pyfunction!(store::arrangement::plane_segment_parameter, m)?)?;
    m.add_function(wrap_pyfunction!(store::arrangement::plane_intersections, m)?)?;
    m.add_function(wrap_pyfunction!(store::planar::plane_region_point, m)?)?;
    m.add_function(wrap_pyfunction!(store::planar::plane_nesting, m)?)?;
    m.add_function(wrap_pyfunction!(store::planar::plane_interior_point, m)?)?;
    m.add_function(wrap_pyfunction!(store::planar::plane_scan_point, m)?)?;
    m.add_function(wrap_pyfunction!(store::planar::union_plane_regions, m)?)?;
    m.add_function(wrap_pyfunction!(store::planar::trace_plane_outline, m)?)?;
    m.add_function(wrap_pyfunction!(store::plane_regions::rebuild_region_faces, m)?)?;
    m.add_function(wrap_pyfunction!(store::plane_repair::object_cap_coverage, m)?)?;
    m.add_function(wrap_pyfunction!(store::plane_repair::shadowed_push_faces, m)?)?;
    m.add_function(wrap_pyfunction!(store::plane_repair::plane_fingerprint, m)?)?;
    m.add_function(wrap_pyfunction!(store::plane_repair::plane_cycle, m)?)?;
    m.add_function(wrap_pyfunction!(store::plane_repair::install_rebuilt_plane, m)?)?;
    m.add_function(wrap_pyfunction!(store::plane_repair::prune_object_plane, m)?)?;
    m.add_function(wrap_pyfunction!(store::intersection_edit::partition_intersections, m)?)?;
    m.add_function(wrap_pyfunction!(object_merge::revert_recorded_union, m)?)?;
    m.add_function(wrap_pyfunction!(object_merge::merge_seeded_edges, m)?)?;
    m.add_function(wrap_pyfunction!(object_merge::revert_seeded_merges, m)?)?;
    m.add_function(wrap_pyfunction!(object_merge::merge_materialised_regions, m)?)?;
    m.add_function(wrap_pyfunction!(object_merge::trace_object_loops, m)?)?;
    m.add_function(wrap_pyfunction!(object_merge::dissolve_object_edge, m)?)?;
    m.add_function(wrap_pyfunction!(object_merge::dissolve_object_region, m)?)?;
    m.add_function(wrap_pyfunction!(object_weld::weld_object_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(object_weld::weld_object_vertex, m)?)?;
    m.add_function(wrap_pyfunction!(object_weld::split_object_at_position, m)?)?;
    m.add_function(wrap_pyfunction!(object_weld::split_object_edge, m)?)?;
    m.add_function(wrap_pyfunction!(object_weld::revert_object_split, m)?)?;
    m.add_function(wrap_pyfunction!(push_guard::guarded_push, m)?)?;
    m.add_function(wrap_pyfunction!(push_guard::translate_prism_cap, m)?)?;
    m.add_function(wrap_pyfunction!(push_command::push_top_out, m)?)?;
    m.add_function(wrap_pyfunction!(push_command::push_secondary_faces, m)?)?;
    m.add_function(wrap_pyfunction!(push_command::outline_curve_runs, m)?)?;
    navigation::register(m)?;
    recents::register(m)?;
    delivery_form::register(m)?;
    m.add_class::<math::Vec3>()?;
    m.add_class::<texture_position::TextureMap>()?;
    m.add_class::<texture_position::TexturePlacement>()?;
    m.add_class::<math::Vec4>()?;
    m.add_class::<math::Matrix4>()?;
    m.add_class::<math::Quaternion>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("SOURCE_HASH", env!("INGETRAZO_SOURCE_HASH"))?;
    m.add_function(wrap_pyfunction!(panic_log::install_panic_log, m)?)?;
    m.add_function(wrap_pyfunction!(panic_log::_panic_for_test, m)?)?;
    m.add_function(wrap_pyfunction!(pack_materials::pack_materials, m)?)?;
    m.add_function(wrap_pyfunction!(pack::pack_weld, m)?)?;
    m.add_class::<HlrOccluders>()?;
    sheets::py::register(m)?;
    m.add_function(wrap_pyfunction!(touching_boxes, m)?)?;
    m.add_class::<ParityTris>()?;
    m.add_function(wrap_pyfunction!(orient::orient_decision, m)?)?;
    m.add_function(wrap_pyfunction!(orient::scan_face_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(orient::scan_face_probes, m)?)?;
    m.add_function(wrap_pyfunction!(orient::py_random_stream, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_is_closed, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_faces_holding_points, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_parallel_overlapping, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_lateral_exit, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_inward_limit, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_classify_base, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_sweeps_free_sheet, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_surface_normal, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_cap_passes_wall, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_vertices_in_boxes, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_cracked_edges, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_faces_meeting_rings, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::store_keys_since, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_tjunction_arrays, m)?)?;
    m.add_function(wrap_pyfunction!(pushscan::scan_mesh_is_flat, m)?)?;
    m.add_function(wrap_pyfunction!(orient::scan_face_indices, m)?)?;
    m.add_function(wrap_pyfunction!(occlusion_span, m)?)?;
    m.add_function(wrap_pyfunction!(face_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(render_bake::bake_vertices, m)?)?;
    m.add_function(wrap_pyfunction!(render_bake::bake_edges, m)?)?;
    m.add_function(wrap_pyfunction!(scene_bake::bake_scene, m)?)?;
    m.add_function(wrap_pyfunction!(qt_vector_probe, m)?)?;
    m.add_function(wrap_pyfunction!(qt_dot_pairs, m)?)?;
    m.add_function(wrap_pyfunction!(smooth_corner_normals, m)?)?;
    m.add_function(wrap_pyfunction!(weld_points, m)?)?;
    m.add_function(wrap_pyfunction!(store_weld_indices, m)?)?;
    m.add_function(wrap_pyfunction!(json_mesh_blocks, m)?)?;
    m.add_function(wrap_pyfunction!(boxes_near_px, m)?)?;
    m.add_function(wrap_pyfunction!(points_near_px, m)?)?;
    m.add_function(wrap_pyfunction!(segment_distances, m)?)?;
    m.add_function(wrap_pyfunction!(nearest_within, m)?)?;
    m.add_function(wrap_pyfunction!(visible_parts, m)?)?;
    m.add_function(wrap_pyfunction!(earcut_indices, m)?)?;
    m.add_function(wrap_pyfunction!(faces_view, m)?)?;
    m.add_class::<InstanceProto>()?;
    m.add_class::<InstanceSet>()?;
    m.add_function(wrap_pyfunction!(snap::drawing_axis, m)?)?;
    m.add_function(wrap_pyfunction!(snap::screen_axis, m)?)?;
    m.add_function(wrap_pyfunction!(snap::edge_direction, m)?)?;
    m.add_function(wrap_pyfunction!(snap::reference_crossings, m)?)?;
    m.add_function(wrap_pyfunction!(snap::reference_feet, m)?)?;
    m.add_function(wrap_pyfunction!(snap::face_center, m)?)?;
    m.add_function(wrap_pyfunction!(snap_curve::circle_tangents, m)?)?;
    m.add_function(wrap_pyfunction!(snap_curve::snap_circle, m)?)?;
    m.add_class::<inference_state::InferenceMemory>()?;
    m.add_class::<tools::py::ToolMachine>()?;
    m.add_class::<tools::py::ScaleGripSpec>()?;
    m.add_function(wrap_pyfunction!(tools::host_geometry::protractor_metrics, m)?)?;
    m.add_function(wrap_pyfunction!(tools::host_geometry::move_box_contains, m)?)?;
    m.add_function(wrap_pyfunction!(tools::host_geometry::move_grip_hit, m)?)?;
    m.add_class::<tools::view_queries::ViewportToolHost>()?;
    m.add_function(wrap_pyfunction!(tools::box_records::box_records, m)?)?;
    m.add_function(wrap_pyfunction!(tools::box_records::box_placements, m)?)?;
    m.add_function(wrap_pyfunction!(tools::box_records::segment_overlaps_box, m)?)?;
    m.add_function(wrap_pyfunction!(tools::edge_runs::ordered_edge_path, m)?)?;
    m.add_function(wrap_pyfunction!(tools::edge_runs::outline_normal, m)?)?;
    m.add_class::<pick::FacePicker>()?;
    m.add_class::<tools::transform_queries::CopyWire>()?;
    m.add_function(wrap_pyfunction!(tools::transform_queries::transform_targets, m)?)?;
    m.add_function(wrap_pyfunction!(tools::transform_queries::transform_records, m)?)?;
    m.add_class::<tools::host_geometry::ToolBounds>()?;
    m.add_function(wrap_pyfunction!(tools::py::scale_box_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(tools::py::move_rotation_grips, m)?)?;
    m.add_function(wrap_pyfunction!(tools::py::move_grip_region, m)?)?;
    m.add_function(wrap_pyfunction!(tools::arc::py_last_fillet_d, m)?)?;
    m.add_function(wrap_pyfunction!(tools::py::select_mode, m)?)?;
    m.add_function(wrap_pyfunction!(tools::py::select_pick, m)?)?;
    m.add_function(wrap_pyfunction!(pick::ray_triangles, m)?)?;
    m.add_function(wrap_pyfunction!(units::units_table, m)?)?;
    m.add_function(wrap_pyfunction!(tools::plane::drawing_plane_axes, m)?)?;
    m.add_function(wrap_pyfunction!(tools::plane::py_loop_normal, m)?)?;
    m.add_function(wrap_pyfunction!(tools::plane::py_face_the_plane, m)?)?;
    m.add_class::<tools::plane::PyPlaneLock>()?;
    m.add_function(wrap_pyfunction!(units::vulgar_fractions, m)?)?;
    m.add_function(wrap_pyfunction!(units::py_number, m)?)?;
    m.add_function(wrap_pyfunction!(units::py_plain_marks, m)?)?;
    m.add_function(wrap_pyfunction!(units::py_parse, m)?)?;
    m.add_function(wrap_pyfunction!(units::py_parse_value_buffer, m)?)?;
    m.add_function(wrap_pyfunction!(value_field::value_key, m)?)?;
    m.add_function(wrap_pyfunction!(value_field::value_enter, m)?)?;
    m.add_function(wrap_pyfunction!(value_field::py_zoom_enter, m)?)?;
    m.add_function(wrap_pyfunction!(value_field::value_claims, m)?)?;
    m.add_function(wrap_pyfunction!(pick::project_px, m)?)?;
    m.add_function(wrap_pyfunction!(pick::seg_rect_mask, m)?)?;
    m.add_function(wrap_pyfunction!(pick::box_mesh_arrays, m)?)?;
    m.add_function(wrap_pyfunction!(pick::box_packed_rows, m)?)?;
    m.add_function(wrap_pyfunction!(pick::nearest_segments_px, m)?)?;
    m.add_function(wrap_pyfunction!(snap_curve::arc_midpoint, m)?)?;
    m.add_function(wrap_pyfunction!(snap_planes::plane_cut_ray, m)?)?;
    m.add_function(wrap_pyfunction!(snap_planes::plane_alignments, m)?)?;
    m.add_function(wrap_pyfunction!(snap_planes::lock_on_extension, m)?)?;
    m.add_function(wrap_pyfunction!(snap_planes::face_plane, m)?)?;
    m.add_class::<snap::SnapQuery>()?;
    m.add_function(wrap_pyfunction!(snap_engine::resolve_snap, m)?)?;
    m.add_function(wrap_pyfunction!(snap::lock_crossing, m)?)?;
    m.add_function(wrap_pyfunction!(snap_build::build_snap_index, m)?)?;
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
    m.add_function(wrap_pyfunction!(file_annotations::igz_archive_annotations, m)?)?;
    m.add_function(wrap_pyfunction!(file_annotations::highest_group_number, m)?)?;
    m.add_function(wrap_pyfunction!(file_styles::igz_archive_layer, m)?)?;
    m.add_function(wrap_pyfunction!(file_styles::igz_load_layer, m)?)?;
    m.add_function(wrap_pyfunction!(file_styles::igz_archive_material, m)?)?;
    m.add_function(wrap_pyfunction!(file_styles::igz_load_material, m)?)?;
    m.add_function(wrap_pyfunction!(file_sheet::igz_load_sheet, m)?)?;
    m.add_function(wrap_pyfunction!(file_sheet::igz_archive_sheet, m)?)?;
    m.add_function(wrap_pyfunction!(file_textures::igz_texture_references, m)?)?;
    m.add_function(wrap_pyfunction!(file_json::igz_json_loads, m)?)?;
    m.add_function(wrap_pyfunction!(file_json::igz_encode_json, m)?)?;
    m.add_class::<file_json::FileMatrix>()?;
    m.add_class::<group_geometry::GroupGeometry>()?;
    presentation::register(m)?;
    object_replay::register(m)?;
    object_drawing::register(m)?;
    scene_tree::register(m)?;
    planar_edits::register(m)?;
    m.add_function(wrap_pyfunction!(structure_bootstrap::structure_proposal, m)?)?;
    placement_edits::register(m)?;
    placement_transform::register(m)?;
    reference_layers::register(m)?;
    definition_edits::register(m)?;
    m.add_function(wrap_pyfunction!(structure::add_template_layers, m)?)?;
    m.add_function(wrap_pyfunction!(structure::structure_folder_state, m)?)?;
    m.add_function(wrap_pyfunction!(structure::apply_folder_parents, m)?)?;
    m.add_function(wrap_pyfunction!(structure::restore_structure_folders, m)?)?;
    m.add_class::<structure::StructurePlacements>()?;
    m.add_function(wrap_pyfunction!(structure::filing_choices, m)?)?;
    m.add_function(wrap_pyfunction!(structure::structure_lineage, m)?)?;
    m.add_function(wrap_pyfunction!(structure::structure_targets, m)?)?;
    m.add_function(wrap_pyfunction!(structure::plan_filing, m)?)?;
    m.add_function(wrap_pyfunction!(structure::check_filing_paths, m)?)?;
    m.add_function(wrap_pyfunction!(structure::plan_node_edit, m)?)?;
    m.add_function(wrap_pyfunction!(project_tree::reveal_branches, m)?)?;
    m.add_function(wrap_pyfunction!(project_tree::reveal_selection, m)?)?;
    m.add_function(wrap_pyfunction!(project_tree::loose_edit, m)?)?;
    m.add_function(wrap_pyfunction!(project_tree::reset_source_tags, m)?)?;
    m.add_function(wrap_pyfunction!(project_tree::source_state, m)?)?;
    m.add_function(wrap_pyfunction!(project_tree::source_direct_count, m)?)?;
    m.add_function(wrap_pyfunction!(project_tree::source_placement_rows, m)?)?;
    m.add_function(wrap_pyfunction!(project_sections::section_inspector, m)?)?;
    m.add_function(wrap_pyfunction!(project_sections::section_edit, m)?)?;
    m.add_class::<link_subset::LinkSubset>()?;
    m.add_function(wrap_pyfunction!(link_subset::link_subset_rows, m)?)?;
    m.add_function(wrap_pyfunction!(link_subset::link_selection_sources, m)?)?;
    m.add_function(wrap_pyfunction!(readiness::export_readiness, m)?)?;
    m.add_function(wrap_pyfunction!(sidebar::project_sidebar, m)?)?;
    m.add_function(wrap_pyfunction!(hatch_file::import_hatch_file, m)?)?;
    m.add_function(wrap_pyfunction!(style_files::export_style_file, m)?)?;
    m.add_function(wrap_pyfunction!(style_edit::edit_line_type, m)?)?;
    m.add_function(wrap_pyfunction!(style_edit::edit_tag_folder, m)?)?;
    m.add_function(wrap_pyfunction!(style_edit::document_piece, m)?)?;
    m.add_function(wrap_pyfunction!(style_edit::import_line_file, m)?)?;
    m.add_function(wrap_pyfunction!(sidebar::promote_tag, m)?)?;
    m.add_function(wrap_pyfunction!(filing_rows::filing_rows, m)?)?;
    m.add_function(wrap_pyfunction!(filing_rows::mark_filing_selection, m)?)?;
    m.add_function(wrap_pyfunction!(selection_identity::selection_identity, m)?)?;
    m.add_function(wrap_pyfunction!(project_tree::project_name_matches, m)?)?;
    m.add_function(wrap_pyfunction!(project_tree::project_item_matches, m)?)?;
    m.add_function(wrap_pyfunction!(visibility_sets::visibility_set_rows, m)?)?;
    m.add_function(wrap_pyfunction!(visibility_sets::visibility_set_command, m)?)?;
    m.add_function(wrap_pyfunction!(scene_commands::plan_scene_command, m)?)?;
    m.add_class::<assembly_inspector::AssemblyPreview>()?;
    m.add_function(wrap_pyfunction!(assembly_inspector::part_measures, m)?)?;
    m.add_function(wrap_pyfunction!(assembly_inspector::assembly_selection, m)?)?;
    m.add_function(wrap_pyfunction!(assembly_inspector::plan_part_edit, m)?)?;
    swatch::register(m)?;
    m.add_function(wrap_pyfunction!(material_library::bundled_materials, m)?)?;
    m.add_function(wrap_pyfunction!(material_library::pick_library_paint, m)?)?;
    m.add_function(wrap_pyfunction!(material_library::material_inspector, m)?)?;
    m.add_function(wrap_pyfunction!(material_library::refresh_library_paint, m)?)?;
    m.add_function(wrap_pyfunction!(solid_inspector::solid_inspector_sections, m)?)?;
    m.add_function(wrap_pyfunction!(solid_inspector::plan_solid_fault, m)?)?;
    m.add_function(wrap_pyfunction!(solid_inspector::solid_problem_targets, m)?)?;
    m.add_function(wrap_pyfunction!(material_library::material_surface_input, m)?)?;
    m.add_class::<project_sheets::ProjectSheets>()?;
    m.add_function(wrap_pyfunction!(project_tree::project_context_rows, m)?)?;
    m.add_function(wrap_pyfunction!(structure::file_created_placement, m)?)?;
    m.add_function(wrap_pyfunction!(structure::resync_filing, m)?)?;
    m.add_function(wrap_pyfunction!(structure::apply_structure_bootstrap, m)?)?;
    m.add_function(wrap_pyfunction!(structure::set_structure_nodes, m)?)?;
    m.add_function(wrap_pyfunction!(structure::restore_structure_nodes, m)?)?;
    m.add_function(wrap_pyfunction!(group_geometry::unique_placement, m)?)?;
    m.add_function(wrap_pyfunction!(group_geometry::make_placement_unique, m)?)?;
    m.add_function(wrap_pyfunction!(group_geometry::restore_unique_placement, m)?)?;
    m.add_function(wrap_pyfunction!(group_geometry::lift_placements, m)?)?;
    m.add_function(wrap_pyfunction!(group_geometry::lower_placements, m)?)?;
    m.add_function(wrap_pyfunction!(group_geometry::explode_object_geometry, m)?)?;
    m.add_class::<file_groups::FileGroups>()?;
    m.add_class::<file_groups::FileGroupWriter>()?;
    m.add_function(wrap_pyfunction!(file_geometry::igz_points, m)?)?;
    m.add_function(wrap_pyfunction!(igz::igz_read_geometry, m)?)?;
    // Unit tests link without libpython; this line made the test binary
    // keep the store's PyO3 glue (and its Python symbols) alive.
    #[cfg(not(test))]
    store::py::register(m)?;
    doc::py::register(m)?;
    m.add_class::<scans::MeshScan>()?;
    Ok(())
}

/// The triangles of one hidden-line pass, in camera space, with a 2D grid
/// over their bounding boxes. Built once per view by `core.hlr`.
#[pyclass(frozen)]
struct HlrOccluders {
    inner: hlr::Occluders,
}

#[pymethods]
impl HlrOccluders {
    /// ``tc``: (T,3,3) triangle vertices in camera space (x, y, depth).
    #[new]
    fn new(tc: PyReadonlyArray3<'_, f64>) -> PyResult<Self> {
        let tc = tc.as_array();
        let n = tc.shape()[0];
        let mut tris = Vec::with_capacity(n);
        for t in 0..n {
            let mut v = [[0.0f64; 3]; 3];
            for k in 0..3 {
                for c in 0..3 {
                    v[k][c] = tc[[t, k, c]];
                }
            }
            tris.push(v);
        }
        Ok(Self { inner: hlr::Occluders::new(tris) })
    }

    /// Visible t-intervals of every edge: core.hlr.visible_spans for each
    /// row, with the same candidate filter as hlr_drawing. ``a``/``b``:
    /// (E,3) endpoints in camera space. Returns ``(offsets (E+1,), spans
    /// (S,2))``: edge i owns spans[offsets[i]:offsets[i+1]].
    fn visible_spans<'py>(
        &self,
        py: Python<'py>,
        a: PyReadonlyArray2<'_, f64>,
        b: PyReadonlyArray2<'_, f64>,
        eps: f64,
    ) -> PyResult<(Bound<'py, PyArray1<i64>>, Bound<'py, PyArray2<f64>>)> {
        let a = a.as_array();
        let b = b.as_array();
        let n = a.shape()[0];
        let edges: Vec<([f64; 3], [f64; 3])> = (0..n)
            .map(|i| {
                ([a[[i, 0]], a[[i, 1]], a[[i, 2]]], [b[[i, 0]], b[[i, 1]], b[[i, 2]]])
            })
            .collect();
        let inner = &self.inner;
        let (offsets, spans) = py.detach(|| inner.visible_spans_all(&edges, eps));
        let flat: Vec<Vec<f64>> = spans.iter().map(|s| vec![s.0, s.1]).collect();
        let off = PyArray1::from_vec(py, offsets);
        let sp = if flat.is_empty() {
            PyArray2::zeros(py, [0, 2], false)
        } else {
            PyArray2::from_vec2(py, &flat)?
        };
        Ok((off, sp))
    }

    /// The Profiles test of hlr_drawing for many visible spans at once:
    /// for each row, is the background on either side of the span's
    /// midpoint? ``mid`` (N,2), ``nrm`` (N,2) the sideways probe offset,
    /// ``zm`` (N,) the span's depth at the midpoint.
    fn profile_flags<'py>(
        &self,
        py: Python<'py>,
        mid: PyReadonlyArray2<'_, f64>,
        nrm: PyReadonlyArray2<'_, f64>,
        zm: PyReadonlyArray1<'_, f64>,
        delta: f64,
        ptol: f64,
    ) -> PyResult<Bound<'py, PyArray1<bool>>> {
        let mid = mid.as_array();
        let nrm = nrm.as_array();
        let zm = zm.as_array();
        let n = mid.shape()[0];
        let probes: Vec<([f64; 2], [f64; 2], f64)> = (0..n)
            .map(|i| ([mid[[i, 0]], mid[[i, 1]]], [nrm[[i, 0]], nrm[[i, 1]]], zm[i]))
            .collect();
        let inner = &self.inner;
        let out = py.detach(|| inner.profile_flags(&probes, delta, ptol));
        Ok(PyArray1::from_vec(py, out))
    }
}

fn rows3(a: &numpy::ndarray::ArrayView2<'_, f64>) -> Vec<[f64; 3]> {
    (0..a.shape()[0]).map(|i| [a[[i, 0]], a[[i, 1]], a[[i, 2]]]).collect()
}

/// core.intersect._touching for all rows at once: the index pairs ``(i,
/// j)`` whose boxes touch (within ``eps``), sorted by i then j; ``same``:
/// j > i only. ``lo_*``/``hi_*``: (N,3) box corners.
#[pyfunction]
fn touching_boxes<'py>(
    py: Python<'py>,
    lo_a: PyReadonlyArray2<'_, f64>,
    hi_a: PyReadonlyArray2<'_, f64>,
    lo_b: PyReadonlyArray2<'_, f64>,
    hi_b: PyReadonlyArray2<'_, f64>,
    same: bool,
    eps: f64,
) -> PyResult<(Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<i64>>)> {
    let a: Vec<boxes::Box3> = rows3(&lo_a.as_array()).into_iter()
        .zip(rows3(&hi_a.as_array())).collect();
    let b: Vec<boxes::Box3> = rows3(&lo_b.as_array()).into_iter()
        .zip(rows3(&hi_b.as_array())).collect();
    let (ii, jj) = py.detach(|| boxes::touching(&a, &b, same, eps));
    Ok((PyArray1::from_vec(py, ii), PyArray1::from_vec(py, jj)))
}

/// A packed triangle set (core.orient._PackedTris) with a ray index, for
/// the parity queries of orientation and of the push/pull rebuild.
#[pyclass(frozen)]
struct ParityTris {
    inner: parity::Tris,
}

#[pymethods]
impl ParityTris {
    /// ``a``/``e1``/``e2``: (N,3) vertex A and the two edge vectors;
    /// ``face_idx``: (N,) the face of each triangle.
    #[new]
    fn new(
        a: PyReadonlyArray2<'_, f64>,
        e1: PyReadonlyArray2<'_, f64>,
        e2: PyReadonlyArray2<'_, f64>,
        face_idx: PyReadonlyArray1<'_, i64>,
    ) -> PyResult<Self> {
        let face = face_idx.as_array().to_vec();
        Ok(Self { inner: parity::Tris::new(rows3(&a.as_array()), rows3(&e1.as_array()),
                                           rows3(&e2.as_array()), face) })
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Crossings ahead of ``origin`` (3,) along each row of ``dirs`` (R,3),
    /// over the triangles with ``mask`` True (all when None) that are not of
    /// face ``skip``. Returns a list of R ints.
    #[pyo3(signature = (origin, dirs, mask=None, skip=None))]
    fn crossings(
        &self,
        py: Python<'_>,
        origin: (f64, f64, f64),
        dirs: PyReadonlyArray2<'_, f64>,
        mask: Option<PyReadonlyArray1<'_, bool>>,
        skip: Option<i64>,
    ) -> PyResult<Vec<i64>> {
        let d = rows3(&dirs.as_array());
        // The mask is read in place: copying it cost O(triangles) per query,
        // and orientation asks one query per face — O(F²) bytes moved (the
        // ride's 267k-face mesh spent ~60 s of a push click copying masks).
        let owned: Option<Vec<bool>>;
        let m: Option<&[bool]> = match mask.as_ref() {
            None => None,
            Some(m) => match m.as_slice() {
                Ok(s) => Some(s),
                Err(_) => {
                    owned = Some(m.as_array().to_vec());
                    owned.as_deref()
                }
            },
        };
        if let Some(mm) = m {
            if mm.len() != self.inner.len() {
                return Err(pyo3::exceptions::PyValueError::new_err("mask length"));
            }
        }
        let inner = &self.inner;
        let o = [origin.0, origin.1, origin.2];
        Ok(py.detach(|| inner.crossings(o, &d, m, skip)))
    }
}

fn hypot_of(name: &str) -> PyResult<facegeom::Hypot> {
    match name {
        "libstdc++" => Ok(facegeom::Hypot::LibStdCxx),
        "msvc" => Ok(facegeom::Hypot::Msvc),
        "libc++-fma" => Ok(facegeom::Hypot::LibCxxFma),
        _ => Err(pyo3::exceptions::PyValueError::new_err("unknown hypot")),
    }
}

/// views.viewport.Viewport._placements_under_px: the indices of the boxes
/// ``lo``/``hi`` (P,3) whose screen footprint through ``mvp`` (the 4×4,
/// row-major: clip = M·(p, 1)) comes within ``r`` px of ``(x, y)`` in a
/// ``w``×``h`` widget; a box crossing the eye plane, or not finite, counts.
/// The same operations, in parallel.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn boxes_near_px<'py>(py: Python<'py>, lo: PyReadonlyArray2<'py, f64>,
                      hi: PyReadonlyArray2<'py, f64>, mvp: Vec<f64>, w: f64, h: f64,
                      x: f64, y: f64, r: f64) -> PyResult<Bound<'py, PyArray1<i64>>> {
    use rayon::prelude::*;
    if mvp.len() != 16 {
        return Err(pyo3::exceptions::PyValueError::new_err("mvp"));
    }
    let (lo, hi) = (rows3(&lo.as_array()), rows3(&hi.as_array()));
    if lo.len() != hi.len() {
        return Err(pyo3::exceptions::PyValueError::new_err("boxes"));
    }
    let m = |j: usize, k: usize| mvp[4 * j + k];
    let keep: Vec<i64> = py.detach(|| (0..lo.len()).into_par_iter().filter(|&i| {
        let (a, b) = (lo[i], hi[i]);
        let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY,
                                                f64::INFINITY, f64::NEG_INFINITY);
        let (mut any_front, mut all_front) = (false, true);
        for c in 0..8 {
            let p = [if c & 1 != 0 { b[0] } else { a[0] },
                     if c & 2 != 0 { b[1] } else { a[1] },
                     if c & 4 != 0 { b[2] } else { a[2] }];
            if !p.iter().all(|v| v.is_finite()) {
                return true;
            }
            let clip = |j: usize| p[0] * m(j, 0) + p[1] * m(j, 1) + p[2] * m(j, 2) + m(j, 3);
            let cw = clip(3);
            if cw > 1e-9 {
                any_front = true;
                let sx = (clip(0) / cw * 0.5 + 0.5) * w;
                let sy = (1.0 - (clip(1) / cw * 0.5 + 0.5)) * h;
                x0 = x0.min(sx);
                x1 = x1.max(sx);
                y0 = y0.min(sy);
                y1 = y1.max(sy);
            } else {
                all_front = false;
            }
        }
        if any_front && !all_front {
            return true;
        }
        all_front && x0 - r <= x && x <= x1 + r && y0 - r <= y && y <= y1 + r
    }).map(|i| i as i64).collect());
    Ok(PyArray1::from_vec(py, keep))
}

/// The frame's limits as Python hands them: ``(eye, limits (gh, gw), x0,
/// y0, dpr, device height)`` (see ``Viewport._depth_limits``).
#[allow(clippy::type_complexity)]
fn depth_limits(s: (Vec<f64>, PyReadonlyArray2<'_, f64>, i64, i64, f64, i64))
    -> PyResult<instances::DepthLimits> {
    let (eye, g, x0, y0, dpr, hdev) = s;
    if eye.len() != 3 {
        return Err(pyo3::exceptions::PyValueError::new_err("seen"));
    }
    let ga = g.as_array();
    Ok(instances::DepthLimits {
        eye: [eye[0], eye[1], eye[2]], lim: ga.iter().copied().collect(),
        x0, y0, gw: ga.shape()[1] as i64, gh: ga.shape()[0] as i64, dpr, hdev })
}

/// The indices, in order, of the projected points ``(x, y)`` in front
/// (``ok``) whose squared distance to ``(px, py)`` is at most ``r``²: the
/// candidates of views.viewport.Viewport._px_near, which measures them
/// as NumPy does (650 000 corners a pointer move over the plant of #158).
/// With the points ``pts`` (N, 3) and the frame's ``seen`` limits (as
/// ``InstanceSet.corners_near`` takes them), those the frame shows
/// something in front of are left out too, as
/// ``Viewport._depth_hidden`` leaves them out: ~6 ms of NumPy over the
/// fixed corners near the cursor in that plant.
#[pyfunction]
#[pyo3(signature = (x, y, ok, px, py_, r, pts=None, seen=None))]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn points_near_px<'py>(py: Python<'py>, x: PyReadonlyArray1<'py, f64>,
                       y: PyReadonlyArray1<'py, f64>, ok: PyReadonlyArray1<'py, bool>,
                       px: f64, py_: f64, r: f64, pts: Option<PyReadonlyArray2<'py, f64>>,
                       seen: Option<(Vec<f64>, PyReadonlyArray2<'py, f64>, i64, i64, f64, i64)>)
    -> PyResult<Bound<'py, PyArray1<i64>>> {
    use rayon::prelude::*;
    let (x, y, ok) = (x.as_array(), y.as_array(), ok.as_array());
    if x.len() != y.len() || x.len() != ok.len() {
        return Err(pyo3::exceptions::PyValueError::new_err("lengths"));
    }
    let seen = match (seen, &pts) {
        (Some(s), Some(p)) => {
            if p.as_array().shape() != [x.len(), 3] {
                return Err(pyo3::exceptions::PyValueError::new_err("pts"));
            }
            Some(depth_limits(s)?)
        }
        (None, _) => None,
        (Some(_), None) => return Err(pyo3::exceptions::PyValueError::new_err("pts")),
    };
    let pv = pts.as_ref().map(|p| p.as_array());
    // Read in place where contiguous (the cached projections and their
    // heads are): copying them cost as much as the pass.
    let cow = |a: numpy::ndarray::ArrayView1<'_, f64>| match a.to_slice() {
        Some(_) => None,
        None => Some(a.to_vec()),
    };
    let (xc, yc) = (cow(x), cow(y));
    let okc: Vec<bool> = ok.to_vec();
    let x: &[f64] = xc.as_deref().unwrap_or_else(|| x.to_slice().unwrap());
    let y: &[f64] = yc.as_deref().unwrap_or_else(|| y.to_slice().unwrap());
    let ok: &[bool] = &okc;
    let r2 = r * r;
    let keep: Vec<i64> = py.detach(|| (0..x.len()).into_par_iter().with_min_len(8192)
        .filter(|&i| {
            let (dx, dy) = (x[i] - px, y[i] - py_);
            ok[i] && dx * dx + dy * dy <= r2
                && !seen.as_ref().is_some_and(|s| {
                    let p = pv.as_ref().unwrap();
                    s.hides([p[[i, 0]], p[[i, 1]], p[[i, 2]]], x[i], y[i])
                })
        }).map(|i| i as i64).collect());
    Ok(PyArray1::from_vec(py, keep))
}

/// The indices of the ``cap`` smallest of ``d[:n]`` under ``radius``, by
/// distance and then index — what ``np.lexsort((cand, d[cand]))[:cap]``
/// over ``np.flatnonzero(d < radius)`` gives (views.viewport's fixed group
/// edges near the cursor). Zoomed out over the plant of #158 some 475 000
/// fixed edges lay within the radius: ~4 ms of NumPy to pick 48, twice a
/// pointer move.
#[pyfunction]
#[pyo3(signature = (d, radius, cap, n=None))]
fn nearest_within<'py>(py: Python<'py>, d: PyReadonlyArray1<'py, f64>, radius: f64,
                       cap: usize, n: Option<usize>) -> PyResult<Bound<'py, PyArray1<i64>>> {
    use rayon::prelude::*;
    let d = d.as_array();
    let n = n.unwrap_or(d.len()).min(d.len());
    let own;
    let d: &[f64] = match d.to_slice() {
        Some(s) => &s[..n],
        None => {
            own = d.iter().take(n).copied().collect::<Vec<f64>>();
            &own
        }
    };
    // Under the radius there is no NaN: the order is total.
    let by = |a: &(f64, usize), b: &(f64, usize)| {
        a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1))
    };
    let keep = |mut v: Vec<(f64, usize)>| {
        if v.len() > cap {
            v.select_nth_unstable_by(cap, by);
            v.truncate(cap);
        }
        v
    };
    let mut all: Vec<(f64, usize)> = py.detach(|| {
        let parts: Vec<Vec<(f64, usize)>> = d.par_chunks(1 << 16).enumerate()
            .map(|(c, ch)| keep(ch.iter().enumerate()
                .filter(|(_, &v)| v < radius)
                .map(|(i, &v)| (v, (c << 16) + i)).collect()))
            .collect();
        keep(parts.into_iter().flatten().collect())
    });
    all.sort_unstable_by(by);
    Ok(PyArray1::from_vec(py, all.into_iter().map(|t| t.1 as i64).collect()))
}

/// ``core.edge_grid.segment_distances``: the pixel distance from ``(px,
/// py)`` to each segment ``(ax, ay)–(bx, by)`` (inf where ``ok`` is
/// False), for all of them or only the indices ``which`` — the same
/// operations in the same order, so the same bits (``hypot`` is the C
/// library's; the caller checks it is NumPy's). Zoomed out over the plant
/// of #158 some 475 000 edges lay within 8 px of the cursor: ~11 ms of
/// NumPy a pointer move.
#[pyfunction]
#[pyo3(signature = (ax, ay, bx, by, ok, px, py_, which=None))]
#[allow(clippy::too_many_arguments)]
fn segment_distances<'py>(py: Python<'py>, ax: PyReadonlyArray1<'py, f64>,
                          ay: PyReadonlyArray1<'py, f64>, bx: PyReadonlyArray1<'py, f64>,
                          by: PyReadonlyArray1<'py, f64>, ok: PyReadonlyArray1<'py, bool>,
                          px: f64, py_: f64, which: Option<PyReadonlyArray1<'py, i64>>)
    -> PyResult<Bound<'py, PyArray1<f64>>> {
    use rayon::prelude::*;
    let (ax, ay, bx, by, ok) = (ax.as_array(), ay.as_array(), bx.as_array(), by.as_array(),
                                ok.as_array());
    let n = ax.len();
    if ay.len() != n || bx.len() != n || by.len() != n || ok.len() != n {
        return Err(pyo3::exceptions::PyValueError::new_err("lengths"));
    }
    let w: Option<Vec<usize>> = match &which {
        Some(w) => {
            let w = w.as_array();
            if w.iter().any(|&i| i < 0 || i as usize >= n) {
                return Err(pyo3::exceptions::PyValueError::new_err("which"));
            }
            Some(w.iter().map(|&i| i as usize).collect())
        }
        None => None,
    };
    let m = w.as_ref().map_or(n, |w| w.len());
    let one = |i: usize| -> f64 {
        if !ok[i] {
            return f64::INFINITY;
        }
        let (dx, dy) = (bx[i] - ax[i], by[i] - ay[i]);
        let l2 = dx * dx + dy * dy;
        let safe = if l2 > 1e-12 { l2 } else { 1.0 };
        let mut t = ((px - ax[i]) * dx + (py_ - ay[i]) * dy) / safe;
        // np.clip: maximum then minimum, NaN through.
        t = if t < 0.0 { 0.0 } else { t };
        t = if t > 1.0 { 1.0 } else { t };
        (ax[i] + t * dx - px).hypot(ay[i] + t * dy - py_)
    };
    let out: Vec<f64> = py.detach(|| (0..m).into_par_iter().with_min_len(4096)
        .map(|k| one(w.as_ref().map_or(k, |w| w[k]))).collect());
    Ok(PyArray1::from_vec(py, out))
}

/// Smooth normals and lighting mask in triangle-corner order.
#[pyfunction]
fn smooth_corner_normals<'py>(py: Python<'py>, pos: PyReadonlyArray2<'_, f64>,
    normal: PyReadonlyArray2<'_, f64>, unit: PyReadonlyArray2<'_, f64>,
    corners: PyReadonlyArray1<'_, i64>, sizes: PyReadonlyArray1<'_, i64>,
    counts: PyReadonlyArray1<'_, i64>, edges: PyReadonlyArray2<'_, i64>,
    soft: PyReadonlyArray1<'_, bool>, starts: PyReadonlyArray1<'_, i64>,
    adjacent: PyReadonlyArray1<'_, i64>, tris: PyReadonlyArray2<'_, i64>,
    faces: PyReadonlyArray1<'_, i64>, hypot: &str)
    -> PyResult<(Bound<'py, PyArray2<f64>>, Bound<'py, PyArray1<bool>>)> {
    let h = hypot_of(hypot)?;
    let vectors = |a: PyReadonlyArray2<'_, f64>| -> Vec<store::geometry::V3> {
        a.as_array().outer_iter().map(|v| store::geometry::V3(v[0],v[1],v[2])).collect()
    };
    let (p, n, u) = (vectors(pos), vectors(normal), vectors(unit));
    let e: Vec<[i64;2]> = edges.as_array().outer_iter().map(|r| [r[0],r[1]]).collect();
    let fans = smooth::corners(&p, &u, corners.as_slice()?, sizes.as_slice()?,
        counts.as_slice()?, &e, soft.as_slice()?, starts.as_slice()?,
        adjacent.as_slice()?, h);
    let mut out = Vec::new();
    let mut mask = Vec::new();
    for (tri, &f) in tris.as_array().outer_iter().zip(faces.as_slice()?) {
        let f = f as usize;
        for &v in tri {
            let got = fans.get(&(v as usize, f));
            let value = got.copied().unwrap_or(n[f]);
            out.push(vec![value.0,value.1,value.2]);
            mask.push(got.map_or(false, |v| v.dot(u[f],h) <= 0.99999));
        }
    }
    Ok((PyArray2::from_vec2(py, &out)?, PyArray1::from_vec(py, mask)))
}

/// Pairwise QVector3D dot products, including Apple's fused operations.
#[pyfunction]
fn qt_dot_pairs<'py>(py: Python<'py>, a: PyReadonlyArray2<'_, f32>,
                     b: PyReadonlyArray2<'_, f32>, hypot: &str)
    -> PyResult<Bound<'py, PyArray1<f32>>> {
    let h = hypot_of(hypot)?;
    let (a, b) = (a.as_array(), b.as_array());
    if a.shape() != b.shape() || a.shape()[1] != 3 {
        return Err(pyo3::exceptions::PyValueError::new_err("expected matching (N,3) arrays"));
    }
    let mut out = Vec::with_capacity(a.shape()[0]);
    for i in 0..a.shape()[0] {
        out.push(facegeom::V3(a[[i,0]], a[[i,1]], a[[i,2]])
            .dot(facegeom::V3(b[[i,0]], b[[i,1]], b[[i,2]]), h));
    }
    Ok(PyArray1::from_vec(py, out))
}

/// ``flat`` (3 values a row) as an (N,3) array, without a Vec per row: making
/// those was most of what a big mesh held the interpreter for (#1092).
fn flat3<'py, T: numpy::Element>(py: Python<'py>, flat: Vec<T>) -> PyResult<Bound<'py, PyArray2<T>>> {
    let n = flat.len() / 3;
    use numpy::PyArrayMethods;
    PyArray1::from_vec(py, flat).reshape([n, 3])
}

/// QVector3D's own arithmetic on each row of ``v`` (N,3) float32, for the
/// start-up check that ``hypot`` is this Qt's: ``(length (N,), normalized
/// (N,3), plane u (N,3), plane v (N,3))`` — core.triangulate.plane_axes.
#[pyfunction]
fn qt_vector_probe<'py>(
    py: Python<'py>,
    v: PyReadonlyArray2<'_, f32>,
    hypot: &str,
) -> PyResult<(Bound<'py, PyArray1<f32>>, Bound<'py, PyArray2<f32>>,
               Bound<'py, PyArray2<f32>>, Bound<'py, PyArray2<f32>>)> {
    let h = hypot_of(hypot)?;
    let v = v.as_array();
    let n = v.shape()[0];
    // With the interpreter let go: the bakes of a big model run side by side,
    // one of these for each (#1092).
    let (ln, nz, pu, pv) = py.detach(|| {
        let mut ln = Vec::with_capacity(n);
        let (mut nz, mut pu, mut pv) = (Vec::with_capacity(3 * n), Vec::with_capacity(3 * n), Vec::with_capacity(3 * n));
        for i in 0..n {
            let x = facegeom::V3(v[[i, 0]], v[[i, 1]], v[[i, 2]]);
            ln.push(facegeom::length(x, h));
            let m = facegeom::normalized(x, h);
            nz.extend_from_slice(&[m.0, m.1, m.2]);
            let (a, b) = facegeom::plane_axes(x, h);
            pu.extend_from_slice(&[a.0, a.1, a.2]);
            pv.extend_from_slice(&[b.0, b.1, b.2]);
        }
        (ln, nz, pu, pv)
    });
    Ok((PyArray1::from_vec(py, ln), flat3(py, nz)?, flat3(py, pu)?, flat3(py, pv)?))
}

/// views.viewport._face_geometry for every face of a mesh given as arrays:
/// ``pos`` (V,3) float32 vertex positions, ``corners`` every ring's vertex
/// indices (rings concatenated, each face's outer ring first),
/// ``ring_sizes`` and ``ring_counts`` (rings per face). Returns ``(normal
/// (F,3) f32, ln (F,) f32 — the Newell length, area = ln / 2 — centroid
/// (F,3) f32, NaN for a face of under three corners, tris (T,3) u32 vertex
/// indices, tri_face (T,) u32, unit (F,3) f32, tris_unit, tri_face_unit)``;
/// each face's triangles in order. ``unit`` and its triangles are the
/// viewport's ``_normal_of`` / ``_tris_of`` (Qt's normalized()).
/// ``compensated_sum``: this Python's sum() of floats is Neumaier's (3.12+).
#[pyfunction]
fn face_geometry<'py>(
    py: Python<'py>,
    pos: &Bound<'py, PyAny>,
    corners: PyReadonlyArray1<'_, i64>,
    ring_sizes: PyReadonlyArray1<'_, i64>,
    ring_counts: PyReadonlyArray1<'_, i64>,
    hypot: &str,
    compensated_sum: bool,
) -> PyResult<Bound<'py, PyAny>> {
    if let Ok(p) = pos.extract::<PyReadonlyArray2<'_, f64>>() {
        let output = face_geometry_arrays(py, p, corners, ring_sizes, ring_counts, hypot, compensated_sum,
            store::geometry::V3, store::geometry::face_geometry, f64::NAN)?;
        Ok(output.into_pyobject(py)?.into_any())
    } else {
        let output = face_geometry_arrays(py, pos.extract()?, corners, ring_sizes, ring_counts, hypot, compensated_sum,
            facegeom::V3, facegeom::face_geometry, f32::NAN)?;
        Ok(output.into_pyobject(py)?.into_any())
    }
}

fn face_geometry_arrays<'py, T, V>(
    py: Python<'py>,
    pos: PyReadonlyArray2<'_, T>,
    corners: PyReadonlyArray1<'_, i64>,
    ring_sizes: PyReadonlyArray1<'_, i64>,
    ring_counts: PyReadonlyArray1<'_, i64>,
    hypot: &str,
    compensated_sum: bool,
    vector: impl Fn(T, T, T) -> V + Send + Sync,
    geometry: impl Fn(&[V], &[&[u32]], facegeom::Hypot, bool) -> facegeom::FaceGeom<V, T> + Send + Sync,
    nan: T,
) -> PyResult<(Bound<'py, PyArray2<T>>, Bound<'py, PyArray1<T>>,
               Bound<'py, PyArray2<T>>, Bound<'py, PyArray2<u32>>,
               Bound<'py, PyArray1<u32>>, Bound<'py, PyArray2<T>>,
               Bound<'py, PyArray2<u32>>, Bound<'py, PyArray1<u32>>)>
where T: numpy::Element + Copy + Send + Sync, V: Into<[T;3]> + Copy + Send + Sync
{
    use rayon::prelude::*;
    let h = hypot_of(hypot)?;
    let p = pos.as_array();
    if p.ncols() != 3 { return Err(pyo3::exceptions::PyValueError::new_err("expected three coordinates")); }
    let corners = corners.as_array();
    let sizes = ring_sizes.as_array();
    let counts = ring_counts.as_array();
    let done = py.detach(|| -> Result<_, &'static str> {
        let pts: Vec<V> = (0..p.shape()[0])
            .map(|i| vector(p[[i, 0]], p[[i, 1]], p[[i, 2]])).collect();
        let nv = pts.len() as i64;
        let corners: Vec<u32> = corners.iter().map(|&c| {
            if c < 0 || c >= nv { u32::MAX } else { c as u32 }
        }).collect();
        if corners.iter().any(|&c| c == u32::MAX) {
            return Err("corner out of range");
        }
        // Each face's first ring, and each ring's first corner.
        let mut ring_start = Vec::with_capacity(sizes.len() + 1);
        let mut acc = 0usize;
        for &s in sizes.iter() {
            ring_start.push(acc);
            if s < 0 { return Err("negative ring size"); }
            acc += s as usize;
        }
        ring_start.push(acc);
        if acc != corners.len() {
            return Err("ring sizes");
        }
        let mut face_ring = Vec::with_capacity(counts.len() + 1);
        let mut r = 0usize;
        for &c in counts.iter() {
            face_ring.push(r);
            if c <= 0 { return Err("face needs a ring"); }
            r += c as usize;
        }
        face_ring.push(r);
        if r != sizes.len() {
            return Err("ring counts");
        }
        let geoms: Vec<facegeom::FaceGeom<V, T>> = (0..counts.len()).into_par_iter().map(|f| {
            let rings: Vec<&[u32]> = (face_ring[f]..face_ring[f + 1])
                .map(|k| &corners[ring_start[k]..ring_start[k + 1]]).collect();
            geometry(&pts, &rings, h, compensated_sum)
        }).collect();
        let nf = geoms.len();
        let mut normal = Vec::with_capacity(3 * nf);
        let mut ln = Vec::with_capacity(nf);
        let mut cen = Vec::with_capacity(3 * nf);
        let mut tris = Vec::new();
        let mut tri_face = Vec::new();
        let mut unit = Vec::with_capacity(3 * nf);
        let mut tris_u = Vec::new();
        let mut tri_face_u = Vec::new();
        for (f, g) in geoms.into_iter().enumerate() {
            unit.extend_from_slice(&g.unit.into());
            for t in g.tris_unit.as_ref().unwrap_or(&g.tris) {
                tris_u.extend_from_slice(&t[..]);
                tri_face_u.push(f as u32);
            }
            normal.extend_from_slice(&g.normal.into());
            ln.push(g.ln);
            match g.centroid {
                Some(c) => cen.extend_from_slice(&c.into()),
                None => cen.extend_from_slice(&[nan; 3]),
            }
            for t in g.tris {
                tris.extend_from_slice(&t[..]);
                tri_face.push(f as u32);
            }
        }
        Ok((normal, ln, cen, tris, tri_face, unit, tris_u, tri_face_u))
    });
    let (normal, ln, cen, tris, tri_face, unit, tris_u, tri_face_u) =
        done.map_err(pyo3::exceptions::PyValueError::new_err)?;
    Ok((flat3(py, normal)?, PyArray1::from_vec(py, ln), flat3(py, cen)?, flat3(py, tris)?,
        PyArray1::from_vec(py, tri_face), flat3(py, unit)?, flat3(py, tris_u)?,
        PyArray1::from_vec(py, tri_face_u)))
}

/// core.lazy_mesh.weld (Mesh.bulk_weld on an empty mesh): ``pos`` (N,3)
/// float32 points, ``scale`` the weld cell (10**decimals), ``tol2`` the
/// probe's squared distance. Returns ``(vertex positions (V,3) float32,
/// point → vertex (N,), the point each vertex was made at (V,))``.
#[pyfunction]
fn weld_points<'py>(
    py: Python<'py>,
    pos: PyReadonlyArray2<'_, f32>,
    scale: f64,
    tol2: f64,
) -> PyResult<(Bound<'py, PyArray2<f32>>, Bound<'py, PyArray1<i64>>,
               Bound<'py, PyArray1<i64>>)> {
    let p = pos.as_array();
    let (flat, ids, origin) = py.detach(|| {
        let pts: Vec<[f32; 3]> = (0..p.shape()[0]).map(|i| [p[[i, 0]], p[[i, 1]], p[[i, 2]]]).collect();
        let (vpos, ids, origin) = weld::weld(&pts, scale, tol2);
        let flat: Vec<f32> = vpos.iter().flatten().copied().collect();
        (flat, ids, origin)
    });
    Ok((flat3(py, flat)?, PyArray1::from_vec(py, ids), PyArray1::from_vec(py, origin)))
}

/// Stable input and representative indices for a double-precision bulk weld.
#[pyfunction]
fn store_weld_indices<'py>(
    py: Python<'py>, pos: PyReadonlyArray2<'_, f64>, scale: f64, tol2: f64,
) -> PyResult<(Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<i64>>)> {
    use numpy::PyUntypedArrayMethods;
    if pos.shape()[1] != 3 {
        return Err(pyo3::exceptions::PyValueError::new_err("expected (P, 3) positions"));
    }
    let points = pos.as_slice()?.as_chunks::<3>().0;
    let (_, ids, origin) = py.detach(|| weld::weld(points, scale, tol2));
    Ok((PyArray1::from_vec(py, ids), PyArray1::from_vec(py, origin)))
}

/// `formats.igz._parse_native`'s kernel: ``(the document without its mesh
/// arrays, [(pos (N,3) float64, flags, curves, layers, layer names, ring
/// sizes, rings per face, face → attrs, attrs, faces with several holes), …])``,
/// or ``None`` when the document is one for the old parser (jsondoc.rs).
#[pyfunction]
#[allow(clippy::type_complexity)]
fn json_mesh_blocks<'py>(py: Python<'py>, doc: &Bound<'py, PyAny>) -> PyResult<Option<(Bound<'py, PyBytes>, Vec<Bound<'py, pyo3::types::PyTuple>>)>> {
    use pyo3::types::{PyByteArray, PyByteArrayMethods, PyBytesMethods};
    let d: &[u8] = if let Ok(b) = doc.cast::<PyBytes>() {
        b.as_bytes()
    } else {
        // SAFETY: the caller's own buffer, which nobody touches meanwhile.
        unsafe { doc.cast::<PyByteArray>()?.as_bytes() }
    };
    let Ok((text, blocks)) = py.detach(|| jsondoc::mesh_blocks(d)) else {
        return Ok(None);
    };
    let out = blocks.into_iter().map(|b| mesh_tuple(py, b)).collect::<PyResult<Vec<_>>>()?;
    Ok(Some((PyBytes::new(py, &text), out)))
}

fn mesh_tuple<'py>(py: Python<'py>, b: jsondoc::Block) -> PyResult<Bound<'py, pyo3::types::PyTuple>> {
    use numpy::PyArrayMethods;
    let n = b.pos.len() / 3;
    let pos = PyArray1::from_vec(py, b.pos).reshape([n, 3])?;
    let names: Vec<Bound<'py, PyBytes>> = b.layer_names.iter().map(|x| PyBytes::new(py, x)).collect();
    let attrs: Vec<Bound<'py, PyBytes>> = b.attrs.iter().map(|x| PyBytes::new(py, x)).collect();
    pyo3::types::PyTuple::new(py, [
        pos.into_any(),
        PyArray1::from_vec(py, b.flags).into_any(),
        PyArray1::from_vec(py, b.curves).into_any(),
        PyArray1::from_vec(py, b.layers).into_any(),
        pyo3::types::PyList::new(py, names)?.into_any(),
        PyArray1::from_vec(py, b.sizes).into_any(),
        PyArray1::from_vec(py, b.counts).into_any(),
        PyArray1::from_vec(py, b.face_attr).into_any(),
        pyo3::types::PyList::new(py, attrs)?.into_any(),
        PyArray1::from_vec(py, b.multi).into_any(),
    ])
}

/// One prototype for the spatial index: its pick triangles (``v0``,
/// ``e1``, ``e2``: (T,3) local, as the chunk keeps them) under a BVH, and
/// its hard edges (``edges``: (E,6) local endpoints).
#[pyclass(frozen)]
struct InstanceProto {
    inner: std::sync::Arc<instances::Proto>,
    curves: Vec<Vec<[f64; 6]>>,
}

#[pymethods]
impl InstanceProto {
    #[new]
    fn new(
        py: Python<'_>,
        v0: PyReadonlyArray2<'_, f64>,
        e1: PyReadonlyArray2<'_, f64>,
        e2: PyReadonlyArray2<'_, f64>,
        edges: PyReadonlyArray2<'_, f64>,
    ) -> PyResult<Self> {
        let (a, b, c) = (rows3(&v0.as_array()), rows3(&e1.as_array()), rows3(&e2.as_array()));
        if a.len() != b.len() || a.len() != c.len() {
            return Err(pyo3::exceptions::PyValueError::new_err("triangle arrays"));
        }
        let ev = edges.as_array();
        if ev.shape()[0] > 0 && ev.shape()[1] != 6 {
            return Err(pyo3::exceptions::PyValueError::new_err("edges (E,6)"));
        }
        let es: Vec<[[f64; 3]; 2]> = (0..ev.shape()[0]).map(|i| [
            [ev[[i, 0]], ev[[i, 1]], ev[[i, 2]]], [ev[[i, 3]], ev[[i, 4]], ev[[i, 5]]]]).collect();
        let inner = py.detach(|| instances::Proto::new(a, b, c, es));
        Ok(Self { inner: std::sync::Arc::new(inner), curves: Vec::new() })
    }

    fn curve_segments<'py>(&self, py: Python<'py>) -> Vec<Bound<'py, PyArray2<f64>>> {
        self.curves.iter().map(|curve| {
            let flat: Vec<f64> = curve.iter().flatten().copied().collect();
            numpy::ndarray::Array2::from_shape_vec((curve.len(), 6), flat).unwrap().into_pyarray(py)
        }).collect()
    }

    fn __len__(&self) -> usize {
        self.inner.n_tris()
    }
}

/// The placements of a scene state over their prototypes: ``protos`` (a
/// list of InstanceProto), ``proto_of`` (P,) which one each placement
/// shows, ``mats`` (P,16) their matrices column-major (QMatrix4x4.data()).
#[pyclass(frozen)]
struct InstanceSet {
    protos: Vec<std::sync::Arc<instances::Proto>>,
    places: Vec<instances::Placement>,
    bvh: instances::PlaceBvh,
}

#[pymethods]
impl InstanceSet {
    #[new]
    #[pyo3(signature = (protos, proto_of, mats, lo=None, hi=None))]
    fn new(
        protos: Vec<PyRef<'_, InstanceProto>>,
        proto_of: PyReadonlyArray1<'_, i64>,
        mats: PyReadonlyArray2<'_, f64>,
        lo: Option<PyReadonlyArray2<'_, f64>>,
        hi: Option<PyReadonlyArray2<'_, f64>>,
    ) -> PyResult<Self> {
        let pr: Vec<std::sync::Arc<instances::Proto>> = protos.iter().map(|p| p.inner.clone()).collect();
        let po = proto_of.as_array();
        let m = mats.as_array();
        if m.shape()[0] != po.len() || (po.len() > 0 && m.shape()[1] != 16) {
            return Err(pyo3::exceptions::PyValueError::new_err("placements"));
        }
        let boxes = match (&lo, &hi) {
            (Some(a), Some(b)) => {
                let (a, b) = (rows3(&a.as_array()), rows3(&b.as_array()));
                if a.len() != po.len() || b.len() != po.len() {
                    return Err(pyo3::exceptions::PyValueError::new_err("boxes"));
                }
                Some((a, b))
            }
            (None, None) => None,
            _ => return Err(pyo3::exceptions::PyValueError::new_err("lo and hi")),
        };
        let mut places = Vec::with_capacity(po.len());
        for i in 0..po.len() {
            let k = po[i];
            if k < 0 || k as usize >= pr.len() {
                return Err(pyo3::exceptions::PyValueError::new_err("proto index"));
            }
            let mut mm = [0.0f64; 16];
            for j in 0..16 {
                mm[j] = m[[i, j]];
            }
            let (blo, bhi) = match &boxes {
                Some((a, b)) => instances::grown(a[i], b[i]),
                None => ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]),
            };
            places.push(instances::Placement { proto: k as usize, m: mm,
                                               inv: instances::affine_inverse(&mm),
                                               lo: blo, hi: bhi });
        }
        // A ray meets nothing in a placement of no triangles (an empty
        // group, a container of others): it has no box in the pick table,
        // and every ray visited its ~1 400 of them over the plant of #158.
        let bvh = instances::PlaceBvh::new(&places, |p| pr[p.proto].n_tris() > 0);
        Ok(Self { protos: pr, places, bvh })
    }

    fn __len__(&self) -> usize {
        self.places.len()
    }

    /// What a ray can be asked about: ``(placements in the rays' BVH,
    /// placements always asked)``. Placements of no triangles are in
    /// neither (tests count them instead of timing rays).
    fn ray_members(&self) -> (usize, usize) {
        self.bvh.members()
    }

    /// Placement ``row`` as built: its prototype and its 16 matrix values
    /// in the order given (the snap index reads committed rows here, not
    /// from a copy of the instance table).
    fn placement(&self, row: usize) -> PyResult<(usize, [f64; 16])> {
        self.places.get(row).map(|p| (p.proto, p.m))
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("placement index"))
    }

    /// Of placements ``cand``, those whose triangles the world ray ``o`` +
    /// t·``d`` may meet for t in [0, ``reach``] (None: unbounded) — a
    /// placement whose matrix cannot be inverted is always kept.
    #[pyo3(signature = (cand, o, d, reach=None))]
    fn ray_filter(&self, py: Python<'_>, cand: PyReadonlyArray1<'_, i64>,
                  o: (f64, f64, f64), d: (f64, f64, f64), reach: Option<f64>) -> PyResult<Vec<i64>> {
        use rayon::prelude::*;
        let c = cand.as_array().to_vec();
        self.check(&c)?;
        let (o, d) = ([o.0, o.1, o.2], [d.0, d.1, d.2]);
        let lim = reach.unwrap_or(f64::INFINITY);
        Ok(py.detach(|| c.par_iter().copied().filter(|&i| {
            let p = &self.places[i as usize];
            match p.local_ray(o, d) {
                None => true,
                Some((lo, ld)) => self.protos[p.proto].ray_may_hit(lo, ld, lim),
            }
        }).collect()))
    }

    /// Of placements ``cand``, those with something near ``(px, py)`` on a
    /// ``w``×``h`` view through ``mvp`` (16 floats, column-major): a hard
    /// edge within ``edge_reach`` px, a triangle within ``tri_reach``.
    #[allow(clippy::too_many_arguments)]
    fn screen_filter(&self, py: Python<'_>, cand: PyReadonlyArray1<'_, i64>, mvp: Vec<f64>,
                     w: f64, h: f64, px: f64, py_: f64, edge_reach: f64,
                     tri_reach: f64) -> PyResult<Vec<i64>> {
        use rayon::prelude::*;
        if mvp.len() != 16 {
            return Err(pyo3::exceptions::PyValueError::new_err("mvp"));
        }
        let c = cand.as_array().to_vec();
        self.check(&c)?;
        let mut a = [0.0f64; 16];
        a.copy_from_slice(&mvp);
        Ok(py.detach(|| c.par_iter().copied().filter(|&i| {
            let p = &self.places[i as usize];
            let m = instances::mat_mul(&a, &p.m);
            self.protos[p.proto].screen_near(&m, w, h, px, py_, edge_reach, tri_reach)
        }).collect()))
    }

    /// The hard-edge ends of placements ``cand`` within ``reach`` px of
    /// ``(px, py)`` on a ``w``×``h`` view through ``mvp`` (16 floats,
    /// column-major): ``(place, edge, end, xyz (N,3), sx, sy)`` — each end
    /// as the bake would store it (float32) and its pixel as
    /// ``Viewport._project_px`` gives it, bit for bit; an edge counts only
    /// with both ends in front of the eye. In ``cand`` order, then edge,
    /// then end.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    #[pyo3(signature = (cand, mvp, w, h, px, py_, reach, seen=None))]
    fn corners_near<'py>(&self, py: Python<'py>, cand: PyReadonlyArray1<'py, i64>, mvp: Vec<f64>,
                         w: f64, h: f64, px: f64, py_: f64, reach: f64,
                         seen: Option<(Vec<f64>, PyReadonlyArray2<'py, f64>, i64, i64, f64, i64)>)
        -> PyResult<(Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<i64>>,
                     Bound<'py, PyArray1<i64>>, Bound<'py, PyArray2<f64>>,
                     Bound<'py, PyArray1<f64>>, Bound<'py, PyArray1<f64>>)> {
        use rayon::prelude::*;
        if mvp.len() != 16 {
            return Err(pyo3::exceptions::PyValueError::new_err("mvp"));
        }
        let c = cand.as_array().to_vec();
        self.check(&c)?;
        let mut a = [0.0f64; 16];
        a.copy_from_slice(&mvp);
        // (eye, limits (gh, gw), x0, y0, dpr, device height): corners the
        // frame shows something in front of are left out here.
        let seen = match seen {
            Some(s) => Some(depth_limits(s)?),
            None => None,
        };
        let seen = seen.as_ref();
        let found: Vec<(i64, Vec<instances::Corner>)> = py.detach(|| c.par_iter().filter_map(|&i| {
            let p = &self.places[i as usize];
            if !p.screen_may_reach(&a, w, h, px, py_, reach) {
                return None;
            }
            let mut out = Vec::new();
            self.protos[p.proto].corners_near(&p.m, &a, w, h, px, py_, reach, seen, &mut out);
            if out.is_empty() { None } else { Some((i, out)) }
        }).collect());
        let n: usize = found.iter().map(|f| f.1.len()).sum();
        let (mut pl, mut ed, mut en) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        let (mut xyz, mut sx, mut sy) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        for (i, cs) in found {
            for k in cs {
                pl.push(i);
                ed.push(k.edge as i64);
                en.push(k.end as i64);
                xyz.push(vec![k.xyz[0], k.xyz[1], k.xyz[2]]);
                sx.push(k.sx);
                sy.push(k.sy);
            }
        }
        let xyz = if xyz.is_empty() {
            PyArray2::zeros(py, [0, 3], false)
        } else {
            PyArray2::from_vec2(py, &xyz).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?
        };
        Ok((PyArray1::from_vec(py, pl), PyArray1::from_vec(py, ed), PyArray1::from_vec(py, en),
            xyz, PyArray1::from_vec(py, sx), PyArray1::from_vec(py, sy)))
    }

    /// The hard edges of placements ``cand`` within ``radius`` px of
    /// ``(px, py)`` on a ``w``×``h`` view through ``mvp`` (both ends in
    /// front), the nearest ``cap`` of them (0: all) by (distance, place in
    /// ``cand``, edge): ``(place, edge, a (N,3), b (N,3), d)`` — the ends as
    /// the bake stores them, the distance as ``segment_distances`` gives
    /// it, bit for bit.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn edges_near<'py>(&self, py: Python<'py>, cand: PyReadonlyArray1<'py, i64>, mvp: Vec<f64>,
                       w: f64, h: f64, px: f64, py_: f64, radius: f64, cap: usize)
        -> PyResult<(Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<i64>>,
                     Bound<'py, PyArray2<f64>>, Bound<'py, PyArray2<f64>>,
                     Bound<'py, PyArray1<f64>>)> {
        use rayon::prelude::*;
        if mvp.len() != 16 {
            return Err(pyo3::exceptions::PyValueError::new_err("mvp"));
        }
        let c = cand.as_array().to_vec();
        self.check(&c)?;
        let mut a = [0.0f64; 16];
        a.copy_from_slice(&mvp);
        // The candidates by how near their boxes come on screen, nearest
        // first: once the heap holds ``cap`` edges, a box farther than the
        // worst of them holds none that could enter, nor do the rest.
        let mut gaps: Vec<(f64, usize, i64)> = py.detach(|| c.par_iter().enumerate()
            .filter_map(|(k, &i)| {
                let g = self.places[i as usize].screen_gap(&a, w, h, px, py_);
                if g < radius { Some((g, k, i)) } else { None }
            }).collect());
        gaps.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        let all = py.detach(|| {
            let mut heap = instances::NearHeap::new(cap, radius);
            // In batches, nearest first, each placement of a batch on its
            // own heap in parallel, bounded by the shared heap's worst: a
            // dense corner of the plant of #158 kept ~4 000 placements
            // within reach of the cursor, walked one after another. Merged
            // in the batch's order, the shared heap ends as the walk left
            // it: (distance, candidate, edge) orders the edges wholly.
            let mut at = 0;
            while at < gaps.len() {
                if !heap.may_take(gaps[at].0) {
                    break;
                }
                let size = if at == 0 { 8 } else { 64 };
                let hi = (at + size).min(gaps.len());
                let bound = heap.worst();
                let found: Vec<Vec<instances::NearEdge>> = gaps[at..hi].par_iter()
                    .map(|&(g, k, i)| {
                        let mut own = instances::NearHeap::bounded(cap, radius, bound);
                        if own.may_take(g) {
                            let p = &self.places[i as usize];
                            self.protos[p.proto].edges_near(&p.m, &a, w, h, px, py_, k, i,
                                                            &mut own);
                        }
                        own.into_sorted()
                    }).collect();
                for e in found.into_iter().flatten() {
                    heap.offer(e);
                }
                at = hi;
            }
            heap.into_sorted()
        });
        let n = all.len();
        let (mut pl, mut ed, mut d) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        let (mut av, mut bv) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for e in all {
            pl.push(e.place);
            ed.push(e.edge as i64);
            d.push(e.d);
            av.push(e.a.to_vec());
            bv.push(e.b.to_vec());
        }
        let arr = |v: Vec<Vec<f64>>| -> PyResult<Bound<'py, PyArray2<f64>>> {
            if v.is_empty() {
                Ok(PyArray2::zeros(py, [0, 3], false))
            } else {
                PyArray2::from_vec2(py, &v).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
            }
        };
        Ok((PyArray1::from_vec(py, pl), PyArray1::from_vec(py, ed), arr(av)?, arr(bv)?,
            PyArray1::from_vec(py, d)))
    }

    /// Whether the triangles of placements ``cand`` hide the point at
    /// ``limit[r]`` along each ray from ``origin`` (``dirs`` (R,3)) — the
    /// placements as baked, without baking them. ``(hidden, doubt)``: a
    /// doubtful ray's answer could hang on the last bit (or a placement
    /// cannot be inverted); the caller decides it on ``ray_tris``.
    fn occluded<'py>(&self, py: Python<'py>, cand: PyReadonlyArray1<'py, i64>,
                     origin: (f64, f64, f64), dirs: PyReadonlyArray2<'py, f64>,
                     limit: PyReadonlyArray1<'py, f64>)
        -> PyResult<(Bound<'py, PyArray1<bool>>, Bound<'py, PyArray1<bool>>)> {
        use rayon::prelude::*;
        let c = cand.as_array().to_vec();
        self.check(&c)?;
        let o = [origin.0, origin.1, origin.2];
        let d = rows3(&dirs.as_array());
        let l = limit.as_array().to_vec();
        if l.len() != d.len() {
            return Err(pyo3::exceptions::PyValueError::new_err("limit"));
        }
        // The candidates as a mask: each ray walks the placements' BVH.
        let mut mask = vec![false; self.places.len()];
        for &i in &c {
            mask[i as usize] = true;
        }
        let out: Vec<(bool, bool)> = py.detach(|| (0..d.len()).into_par_iter()
            .map(|r| self.occluded_ray(&mask, o, d[r], l[r])).collect());
        let (hid, dbt): (Vec<bool>, Vec<bool>) = out.into_iter().unzip();
        Ok((PyArray1::from_vec(py, hid), PyArray1::from_vec(py, dbt)))
    }

    /// The first of ``pts`` (N,3), in order, that nothing hides from
    /// ``eye`` — ``Viewport._occluded_many_now`` asked of each in turn, in
    /// one call: the ray to each point built as ``_unit_rays`` builds it
    /// (float coordinates, the three-argument hypot of ``hypot``, the
    /// float division), the placements ``cand`` of this set and, when given,
    /// those ``stat_cand`` of ``stat`` (the pick index's own triangles)
    /// asked as there. ``(first, doubts)``: the index of the first point
    /// both call seen (-1: none), and before it, in order, those either
    /// could not call bit for bit — the caller asks them as before. A
    /// point on the eye is seen, as there. Over the plant of #158 the
    /// ~7000 corners a roof hides near the cursor were a dozen calls.
    #[pyo3(signature = (cand, eye, pts, stat=None, stat_cand=None, hypot="libstdc++"))]
    fn first_clear<'py>(&self, py: Python<'py>, cand: PyReadonlyArray1<'py, i64>,
                        eye: (f64, f64, f64), pts: PyReadonlyArray2<'py, f64>,
                        stat: Option<PyRef<'py, InstanceSet>>,
                        stat_cand: Option<PyReadonlyArray1<'py, i64>>,
                        hypot: &str)
        -> PyResult<(i64, Bound<'py, PyArray1<i64>>)> {
        use rayon::prelude::*;
        hypot_of(hypot)?; // Validate the legacy call signature.
        let c = cand.as_array().to_vec();
        self.check(&c)?;
        let mut mask = vec![false; self.places.len()];
        for &i in &c {
            mask[i as usize] = true;
        }
        let st: Option<&InstanceSet> = stat.as_deref();
        let smask = match (st, &stat_cand) {
            (Some(t), Some(sc)) => {
                let sc = sc.as_array().to_vec();
                t.check(&sc)?;
                let mut m = vec![false; t.places.len()];
                for &i in &sc {
                    m[i as usize] = true;
                }
                m
            }
            (None, None) => Vec::new(),
            _ => return Err(pyo3::exceptions::PyValueError::new_err("stat and stat_cand")),
        };
        let p = rows3(&pts.as_array());
        let o = [eye.0, eye.1, eye.2];
        // 0 seen, 1 hidden, 2 doubt.
        let call = |q: [f64; 3]| -> u8 {
            let Some((d, dist)) = occlusion::unit_ray(o, q) else {
                return 0;
            };
            let limit = dist - 1e-3;
            let mut doubt = false;
            if let Some(t) = st {
                let (h, db) = t.occluded_ray(&smask, o, d, limit);
                if h {
                    return 1;
                }
                doubt |= db;
            }
            let (h, db) = self.occluded_ray(&mask, o, d, limit);
            if h {
                return 1;
            }
            if doubt || db { 2 } else { 0 }
        };
        let (first, doubts) = py.detach(|| {
            // Each point once, where it first comes: a corner shared by
            // several edges comes once per end, and gives what it gave.
            let key = |q: &[f64; 3]| q.map(|v| if v == 0.0 { 0u64 } else { v.to_bits() });
            let mut seen = std::collections::HashSet::with_capacity(p.len());
            let at: Vec<usize> = (0..p.len())
                .filter(|&i| p[i].iter().any(|v| v.is_nan()) || seen.insert(key(&p[i])))
                .collect();
            let mut doubts: Vec<i64> = Vec::new();
            // The nearest alone first (usually the answer), then batches
            // growing four times over, as the caller's loop took them.
            let (mut k, mut size) = (0usize, 1usize);
            while k < at.len() {
                let hi = (k + size).min(at.len());
                let got: Vec<u8> = at[k..hi].par_iter().map(|&i| call(p[i])).collect();
                for (j, g) in got.into_iter().enumerate() {
                    match g {
                        0 => return (at[k + j] as i64, doubts),
                        2 => doubts.push(at[k + j] as i64),
                        _ => {}
                    }
                }
                k = hi;
                size = if size == 1 { 8 } else { (size * 4).min(1024) };
            }
            (-1, doubts)
        });
        Ok((first, PyArray1::from_vec(py, doubts)))
    }

    /// The world triangles (v0, e1, e2), as baked, of placements ``cand``
    /// that the ray from ``origin`` along ``d`` may meet before ``limit`` —
    /// a superset of what the exact test accepts. A placement that cannot
    /// be inverted gives all of its triangles.
    #[allow(clippy::type_complexity)]
    fn ray_tris<'py>(&self, py: Python<'py>, cand: PyReadonlyArray1<'py, i64>,
                     origin: (f64, f64, f64), d: (f64, f64, f64), limit: f64)
        -> PyResult<(Bound<'py, PyArray2<f64>>, Bound<'py, PyArray2<f64>>, Bound<'py, PyArray2<f64>>)> {
        let c = cand.as_array().to_vec();
        self.check(&c)?;
        let (o, d) = ([origin.0, origin.1, origin.2], [d.0, d.1, d.2]);
        let (mut a, mut b, mut e) = (Vec::new(), Vec::new(), Vec::new());
        for &i in &c {
            let p = &self.places[i as usize];
            if !p.ray_meets_box(o, d, limit) {
                continue;
            }
            let pr = &self.protos[p.proto];
            let mut take = |r: usize| {
                let (x, y, z) = pr.world_tri(&p.m, r);
                a.push(x.to_vec());
                b.push(y.to_vec());
                e.push(z.to_vec());
                true
            };
            match p.local_ray(o, d) {
                Some((lo, ld)) => pr.for_tris_on_ray(lo, ld, limit, &mut take),
                None => (0..pr.n_tris()).for_each(|r| { take(r); }),
            }
        }
        let arr = |v: Vec<Vec<f64>>| -> PyResult<Bound<'py, PyArray2<f64>>> {
            if v.is_empty() {
                Ok(PyArray2::zeros(py, [0, 3], false))
            } else {
                PyArray2::from_vec2(py, &v).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
            }
        };
        Ok((arr(a)?, arr(b)?, arr(e)?))
    }

    /// ``ray_tris`` with whose each triangle is: ``(place, tri, v0, e1,
    /// e2)`` for the placements ``cand`` the ray walks the placements' BVH
    /// to, ordered by placement then prototype triangle — a superset of
    /// what the exact test accepts, for the caller to decide on.
    #[allow(clippy::type_complexity)]
    fn ray_cands<'py>(&self, py: Python<'py>, cand: PyReadonlyArray1<'py, i64>,
                      origin: (f64, f64, f64), d: (f64, f64, f64), limit: f64)
        -> PyResult<(Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<i64>>,
                     Bound<'py, PyArray2<f64>>, Bound<'py, PyArray2<f64>>,
                     Bound<'py, PyArray2<f64>>)> {
        let c = cand.as_array().to_vec();
        self.check(&c)?;
        let (o, d) = ([origin.0, origin.1, origin.2], [d.0, d.1, d.2]);
        let mut mask = vec![false; self.places.len()];
        for &i in &c {
            mask[i as usize] = true;
        }
        let mut met: Vec<usize> = Vec::new();
        self.bvh.on_ray(o, d, limit, |i| {
            if mask[i] && self.places[i].ray_meets_box(o, d, limit) {
                met.push(i);
            }
            true
        });
        met.sort_unstable();
        met.dedup();
        let (mut pl, mut tr) = (Vec::new(), Vec::new());
        let (mut a, mut b, mut e) = (Vec::new(), Vec::new(), Vec::new());
        for i in met {
            let p = &self.places[i];
            let pr = &self.protos[p.proto];
            let mut rows: Vec<usize> = Vec::new();
            match p.local_ray(o, d) {
                Some((lo, ld)) => pr.for_tris_on_ray(lo, ld, limit, |r| {
                    rows.push(r);
                    true
                }),
                None => rows.extend(0..pr.n_tris()),
            }
            rows.sort_unstable();
            rows.dedup();
            for r in rows {
                let (x, y, z) = pr.world_tri(&p.m, r);
                pl.push(i as i64);
                tr.push(r as i64);
                a.push(x.to_vec());
                b.push(y.to_vec());
                e.push(z.to_vec());
            }
        }
        let arr = |v: Vec<Vec<f64>>| -> PyResult<Bound<'py, PyArray2<f64>>> {
            if v.is_empty() {
                Ok(PyArray2::zeros(py, [0, 3], false))
            } else {
                PyArray2::from_vec2(py, &v).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
            }
        };
        Ok((PyArray1::from_vec(py, pl), PyArray1::from_vec(py, tr), arr(a)?, arr(b)?, arr(e)?))
    }
}

impl InstanceSet {
    /// Whether the placements in ``mask`` hide the point at ``limit`` along
    /// the ray: ``(hidden, doubt)``. Any one that hides it settles it, so
    /// the order they are met in does not matter.
    fn occluded_ray(&self, mask: &[bool], o: [f64; 3], d: [f64; 3], limit: f64) -> (bool, bool) {
        let mut doubt = false;
        let mut found = false;
        self.bvh.on_ray(o, d, limit, |i| {
            if !mask[i] {
                return true;
            }
            let p = &self.places[i];
            if !p.ray_meets_box(o, d, limit) {
                return true;
            }
            let Some((lo, ld)) = p.local_ray(o, d) else {
                doubt = true;
                return true;
            };
            let pr = &self.protos[p.proto];
            let mut hidden = false;
            pr.for_tris_on_ray(lo, ld, limit, |r| {
                let (v0, e1, e2) = pr.world_tri(&p.m, r);
                match occlusion::tri_occludes(o, d, limit, v0, e1, e2) {
                    Some(true) => {
                        hidden = true;
                        false
                    }
                    Some(false) => true,
                    None => {
                        doubt = true;
                        true
                    }
                }
            });
            if hidden {
                found = true;
                return false;
            }
            true
        });
        if found { (true, false) } else { (false, doubt) }
    }

    fn check(&self, c: &[i64]) -> PyResult<()> {
        if c.iter().any(|&i| i < 0 || i as usize >= self.places.len()) {
            return Err(pyo3::exceptions::PyValueError::new_err("placement index"));
        }
        Ok(())
    }
}

/// ``Viewport._occluded_many``'s block for one span: rays from ``origin``
/// along the rows of ``dirs`` (R,3), against the span's triangles ``v0``,
/// ``e1``, ``e2`` (N,3). Returns ``(best, doubt)``: per ray, the minimum of
/// ``best`` (R,) and every accepted hit, and whether that answer could hang
/// on the last bit (then the caller asks NumPy), with ``limit`` (R,) the
/// distance of the point the ray goes to.
#[pyfunction]
fn occlusion_span<'py>(
    py: Python<'py>,
    origin: (f64, f64, f64),
    dirs: PyReadonlyArray2<'py, f64>,
    best: PyReadonlyArray1<'py, f64>,
    limit: PyReadonlyArray1<'py, f64>,
    v0: PyReadonlyArray2<'py, f64>,
    e1: PyReadonlyArray2<'py, f64>,
    e2: PyReadonlyArray2<'py, f64>,
) -> PyResult<(Bound<'py, PyArray1<f64>>, Bound<'py, PyArray1<bool>>)> {
    let o = [origin.0, origin.1, origin.2];
    let d = rows3(&dirs.as_array());
    let b = best.as_array().to_vec();
    let l = limit.as_array().to_vec();
    let (a, x, y) = (rows3(&v0.as_array()), rows3(&e1.as_array()), rows3(&e2.as_array()));
    let (nb, doubt) = py.detach(|| occlusion::span(o, &d, &b, &l, &a, &x, &y));
    Ok((PyArray1::from_vec(py, nb), PyArray1::from_vec(py, doubt)))
}

/// Each face of a view cut down to what nothing in front of it covers —
/// `core.vista_caras._visible_parts` (see `visible.rs`). The rings of all
/// faces come flat: `coords` (M, 2), `ring_off` (R + 1) the start of each
/// ring in `coords`, `face_off` (n + 1) the first ring of each face; `near`
/// (n, 3) the nearness planes, `opaque` (n,). Back the same way: the
/// visible rings of every face, in the faces' order. `progress(done)` is
/// called between batches of faces with how far the pass has come (0 to 1);
/// if it raises, the pass stops there and the error comes out.
#[pyfunction]
#[pyo3(signature = (coords, ring_off, face_off, near, opaque, min_area, progress=None, batch=64, grid=visible::GRID))]
#[allow(clippy::too_many_arguments)]
fn visible_parts<'py>(
    py: Python<'py>,
    coords: PyReadonlyArray2<'py, f64>,
    ring_off: PyReadonlyArray1<'py, i64>,
    face_off: PyReadonlyArray1<'py, i64>,
    near: PyReadonlyArray2<'py, f64>,
    opaque: PyReadonlyArray1<'py, bool>,
    min_area: f64,
    progress: Option<Bound<'py, PyAny>>,
    batch: usize,
    grid: f64,
) -> PyResult<(Bound<'py, PyArray2<f64>>, Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<i64>>)> {
    let c = coords.as_array();
    let ro = ring_off.as_slice()?;
    let fo = face_off.as_slice()?;
    let nr = near.as_array();
    let op = opaque.as_slice()?;
    let n = fo.len().saturating_sub(1);
    let mut faces: Vec<visible::Face> = Vec::with_capacity(n);
    for i in 0..n {
        let mut rings = Vec::new();
        for r in fo[i] as usize..fo[i + 1] as usize {
            let (a, b) = (ro[r] as usize, ro[r + 1] as usize);
            rings.push((a..b).map(|k| [c[[k, 0]], c[[k, 1]]]).collect::<Vec<[f64; 2]>>());
        }
        faces.push(visible::Face {
            rings,
            near: [nr[[i, 0]], nr[[i, 1]], nr[[i, 2]]],
            opaque: op[i],
        });
    }
    let setup = visible::Setup::new(&faces, min_area, grid);
    let batch = batch.max(1);
    let mut out: Vec<Vec<visible::Ring>> = Vec::with_capacity(n);
    let mut start = 0usize;
    while start < n {
        if let Some(p) = &progress {
            p.call1((start as f64 / n as f64,))?;
        }
        let end = (start + batch).min(n);
        let got = py.detach(|| setup.chunk(start..end));
        out.extend(got);
        start = end;
    }
    let mut flat: Vec<Vec<f64>> = Vec::new();
    let mut ring_out: Vec<i64> = vec![0];
    let mut face_out: Vec<i64> = vec![0];
    for rings in &out {
        for r in rings {
            for p in r {
                flat.push(vec![p[0], p[1]]);
            }
            ring_out.push(flat.len() as i64);
        }
        face_out.push(ring_out.len() as i64 - 1);
    }
    let arr = if flat.is_empty() { PyArray2::zeros(py, [0, 2], false) } else { PyArray2::from_vec2(py, &flat)? };
    Ok((arr, PyArray1::from_vec(py, ring_out), PyArray1::from_vec(py, face_out)))
}

/// `core.triangulate.earcut` (see `earcut.rs`): `points` (N, 2), the outer
/// ring then each hole's points, `holes` where each hole starts; the
/// triangles as an (M, 3) array of indices into `points`, the same ones
/// the Python port gives, in the same order.
#[pyfunction]
#[pyo3(name = "earcut", signature = (points, holes=Vec::new()))]
fn earcut_indices<'py>(
    py: Python<'py>,
    points: PyReadonlyArray2<'py, f64>,
    holes: Vec<usize>,
) -> PyResult<Bound<'py, PyArray2<i64>>> {
    let p = points.as_array();
    let pts: Vec<[f64; 2]> = (0..p.shape()[0]).map(|k| [p[[k, 0]], p[[k, 1]]]).collect();
    let tris = py.detach(|| earcut::earcut(&pts, &holes));
    let rows: Vec<Vec<i64>> = tris.iter().map(|t| vec![t[0] as i64, t[1] as i64, t[2] as i64]).collect();
    if rows.is_empty() {
        Ok(PyArray2::zeros(py, [0, 3], false))
    } else {
        Ok(PyArray2::from_vec2(py, &rows)?)
    }
}

/// The geometry of every face of a view (see `facesview.rs`), for
/// `core.vista_caras.view_faces`. In: the faces' rings flat in their
/// meshes' coordinates — `coords` (M, 3), `ring_off` (R + 1), `face_off`
/// (n + 1) — `xf_index` (n,) the placement of each face into `xforms`
/// (K, 16, row major; -1: none), the camera (`perspective`, and `cam`: a
/// parallel one's eye, right, up, fwd — 12 numbers — or a perspective
/// one's clip matrix row major, eye and picture scale — 21) and the
/// section's kept side `(nx, ny, nz, c)` or none. Out, per face: `kept`
/// (0: skipped, 1: a caster only, 2: drawn), `back`, `normal` (n, 3),
/// `near` (n, 3), `depth` (n,); the world rings flat (`w_coords`,
/// `w_ring_off`, `w_face_off`) and the drawn rings flat with their depths
/// (`p_coords`, `p_depth`, `p_ring_off`, `p_face_off`).
#[pyfunction]
#[pyo3(signature = (coords, ring_off, face_off, xf_index, xforms, perspective, cam, section=None))]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn faces_view<'py>(
    py: Python<'py>,
    coords: PyReadonlyArray2<'py, f64>,
    ring_off: PyReadonlyArray1<'py, i64>,
    face_off: PyReadonlyArray1<'py, i64>,
    xf_index: PyReadonlyArray1<'py, i64>,
    xforms: PyReadonlyArray2<'py, f64>,
    perspective: bool,
    cam: Vec<f64>,
    section: Option<Vec<f64>>,
) -> PyResult<(
    Bound<'py, PyArray1<i8>>, Bound<'py, PyArray1<bool>>, Bound<'py, PyArray2<f64>>,
    Bound<'py, PyArray2<f64>>, Bound<'py, PyArray1<f64>>,
    Bound<'py, PyArray2<f64>>, Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<i64>>,
    Bound<'py, PyArray2<f64>>, Bound<'py, PyArray1<f64>>, Bound<'py, PyArray1<i64>>, Bound<'py, PyArray1<i64>>,
)> {
    use pyo3::exceptions::PyValueError;
    let c = coords.as_array();
    let ro = ring_off.as_slice()?;
    let fo = face_off.as_slice()?;
    let xi = xf_index.as_slice()?;
    let xf = xforms.as_array();
    let n = fo.len().saturating_sub(1);
    let mut faces: Vec<facesview::FaceIn> = Vec::with_capacity(n);
    for i in 0..n {
        let mut rings = Vec::new();
        for r in fo[i] as usize..fo[i + 1] as usize {
            let (a, b) = (ro[r] as usize, ro[r + 1] as usize);
            rings.push((a..b).map(|k| [c[[k, 0]], c[[k, 1]], c[[k, 2]]]).collect::<Vec<[f64; 3]>>());
        }
        faces.push(facesview::FaceIn { rings, xform: if xi[i] < 0 { None } else { Some(xi[i] as usize) } });
    }
    let mats: Vec<[[f64; 4]; 4]> = (0..xf.shape()[0])
        .map(|k| {
            let mut m = [[0.0f64; 4]; 4];
            for r in 0..4 {
                for q in 0..4 {
                    m[r][q] = xf[[k, r * 4 + q]];
                }
            }
            m
        })
        .collect();
    let v3 = |at: usize| [cam[at], cam[at + 1], cam[at + 2]];
    let camera = if perspective {
        if cam.len() != 21 {
            return Err(PyValueError::new_err("perspective camera: 21 numbers"));
        }
        let mut m = [[0.0f64; 4]; 4];
        for r in 0..4 {
            for q in 0..4 {
                m[r][q] = cam[r * 4 + q];
            }
        }
        facesview::Camera::Perspective { m, eye: v3(16), scale: [cam[19], cam[20]] }
    } else {
        if cam.len() != 12 {
            return Err(PyValueError::new_err("parallel camera: 12 numbers"));
        }
        facesview::Camera::Parallel { eye: v3(0), right: v3(3), up: v3(6), fwd: v3(9) }
    };
    let sec = section.map(|s| facesview::Section { n: [s[0], s[1], s[2]], c: s[3] });
    let out = py.detach(|| facesview::faces_view(&faces, &mats, &camera, sec.as_ref()));
    let mut kept = vec![0i8; n];
    let mut back = vec![false; n];
    let mut normal = vec![vec![0.0f64; 3]; n];
    let mut near = vec![vec![f64::NAN; 3]; n];
    let mut depth = vec![f64::NAN; n];
    let mut wc: Vec<Vec<f64>> = Vec::new();
    let mut wro: Vec<i64> = vec![0];
    let mut wfo: Vec<i64> = vec![0];
    let mut pc: Vec<Vec<f64>> = Vec::new();
    let mut pd: Vec<f64> = Vec::new();
    let mut pro: Vec<i64> = vec![0];
    let mut pfo: Vec<i64> = vec![0];
    for (i, f) in out.iter().enumerate() {
        if let Some(f) = f {
            kept[i] = if f.drawn.is_empty() { 1 } else { 2 };
            back[i] = f.back;
            normal[i] = f.normal.to_vec();
            near[i] = f.near.to_vec();
            depth[i] = f.depth;
            for r in &f.world {
                for p in r {
                    wc.push(p.to_vec());
                }
                wro.push(wc.len() as i64);
            }
            for (r, d) in f.drawn.iter().zip(&f.depths) {
                for (p, &z) in r.iter().zip(d) {
                    pc.push(p.to_vec());
                    pd.push(z);
                }
                pro.push(pc.len() as i64);
            }
        }
        wfo.push(wro.len() as i64 - 1);
        pfo.push(pro.len() as i64 - 1);
    }
    let arr2 = |py: Python<'py>, v: &Vec<Vec<f64>>, w: usize| -> PyResult<Bound<'py, PyArray2<f64>>> {
        if v.is_empty() { Ok(PyArray2::zeros(py, [0, w], false)) } else { Ok(PyArray2::from_vec2(py, v)?) }
    };
    Ok((
        PyArray1::from_vec(py, kept), PyArray1::from_vec(py, back), arr2(py, &normal, 3)?,
        arr2(py, &near, 3)?, PyArray1::from_vec(py, depth),
        arr2(py, &wc, 3)?, PyArray1::from_vec(py, wro), PyArray1::from_vec(py, wfo),
        arr2(py, &pc, 2)?, PyArray1::from_vec(py, pd), PyArray1::from_vec(py, pro), PyArray1::from_vec(py, pfo),
    ))
}

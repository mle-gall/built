// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[rustfmt::skip]
const CORE_INSTANCE: &[Entry] = &[
    ("edit_group_box_contains_pixel", "edit_box_contains_pixel", &["self", "x:float", "y:float"], "bool", ""),
    ("start_pick_warmup", "start_pick_warmup", &["self"], "None", ""),
    ("ray_distance", "index_ray_distance", &["self", "origin:QVector3D", "direction:QVector3D", "reach:float | None=None"], "", ""),
    ("edge_point_under_cursor", "edge_cursor_point", &["self", "edge", "screen_x:float", "screen_y:float"], "", ""),
    ("pick_edge_any", "pick_edge_any", &["self", "screen_x:float", "screen_y:float"], "", ""),
];

#[rustfmt::skip]
const RENDER_INSTANCE: &[Entry] = &[
    ("prepare_drawing", "prepare_drawing", &["self", "progress=None"], "None", ""),
    ("image_plane_at", "image_plane_at", &["self", "screen_x:float", "screen_y:float", "selectable_only:bool=False"], "", ""),
    ("image_in_front_of", "image_in_front_of", &["self", "face", "screen_x:float", "screen_y:float"], "", ""),
    ("pick_locked_image_plane", "pick_locked_image_plane", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("pick_locked_group", "pick_locked_group", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("reset_document_caches", "reset_document_caches", &["self", "keep_bakes:bool=False"], "None", ""),
    ("reset_tiles", "reset_tiles", &["self"], "None", ""),
    ("prefetch_tiles", "prefetch_tiles", &["self", "source", "tile_list", "zoom"], "None", ""),
    ("prune_stale_selection", "prune_stale_selection", &["self"], "", ""),
    ("notify_scene_changed", "document_scene_changed", &["self"], "None", ""),
    ("begin_groups_preview", "begin_groups_preview", &["self", "groups=()", "external:bool=False"], "None", ""),
    ("end_groups_preview", "end_groups_preview", &["self"], "None", ""),
    ("set_hover", "set_hover", &["self", "entity", "placement=None"], "None", ""),
    ("flash_status", "document_status", &["self", "text:str", "msec:int=2500"], "None", ""),
    ("set_suppressed_faces", "set_suppressed_faces", &["self", "faces"], "None", ""),
    ("_draw_snap_indicator", "draw_snap_indicator", &["self", "painter:QPainter", "snap:SnapResult"], "None", ""),
    ("pick_section_plane", "pick_section_plane", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("ground_height", "ground_height", &["self", "x:float", "y:float"], "float | None", ""),
    ("ground_elevation", "ground_elevation", &["self", "x:float", "y:float"], "float | None", ""),
    ("vertical_reference", "vertical_reference", &["self"], "str", ""),
    ("has_ground_surface", "has_ground_surface", &["self"], "bool", ""),
    ("drape", "drape", &["self", "v:QVector3D"], "QVector3D", ""),
    ("_draw_extension_overlays", "document_paint_extensions", &["self", "painter:QPainter"], "None", ""),
    ("pick_dimension", "pick_dimension", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("pick_text_label", "pick_text_label", &["self", "screen_x:float", "screen_y:float", "rect_only:bool=False"], "", ""),
    ("pick_geopath", "pick_geopath", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("pick_axis", "pick_axis", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("pick_guide", "pick_guide", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("pick_vertex", "pick_vertex", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("hlr_geometry", "hlr_geometry", &["self", "tags:bool=False"], "", ""),
    ("hlr_geometry_now", "hlr_geometry_now", &["self"], "", ""),
    ("pick_face", "pick_face", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("pick_group", "pick_group", &["self", "screen_x:float", "screen_y:float"], "", ""),
    ("copy_selection", "copy_selection", &["self", "*", "publish=True"], "bool", ""),
    ("cut_selection", "cut_selection", &["self", "*", "publish=True"], "bool", ""),
    ("set_edit_rest_mode", "set_edit_rest_mode", &["self", "mode:str"], "None", ""),
    ("set_edit_rest_fade", "set_edit_rest_fade", &["self", "strength:float"], "None", ""),
    ("set_hide_similar_components", "set_hide_similar_components", &["self", "on:bool"], "None", ""),
    ("set_nav_mode", "set_nav_mode", &["self", "mode:Optional[str]"], "None", ""),
    ("wheelEvent", "document_wheel", &["self", "ev"], "None", ""),
    ("escape", "document_escape", &["self"], "None", ""),
    ("pick_extension_item", "pick_extension_item", &["self", "px:float", "py:float"], "", ""),
    ("set_extension_pick", "set_extension_pick", &["self", "hit"], "None", ""),
    ("clear_extension_pick", "clear_extension_pick", &["self", "notify:bool=True"], "None", ""),
    ("delete_extension_pick", "delete_extension_pick", &["self"], "bool", ""),
    ("release_constraints", "release_constraints", &["self"], "bool", ""),
    ("refresh_status_hint", "refresh_status_hint", &["self"], "None", ""),
    ("apply_zoom_value", "apply_zoom_value", &["self", "typed:str"], "bool", ""),
];

pub(super) fn install(r: &Registrar, owner: &Obj) -> PyResult<()> {
    r.bindings.install(owner, &r.core, Mode::Method, CORE_INSTANCE)?;
    r.bindings.install(owner, &r.render, Mode::Method, RENDER_INSTANCE)?;
    Ok(())
}

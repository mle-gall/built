# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Commands of the core protocol by key (docs/core-protocol.md §1): each
command runs through History and returns the step published by its change feed.

``run(history, name, **args)`` dispatches the names in ``COMMANDS``.
Arguments are keys, placement uids and plain values. An
unknown key raises ``KeyError`` before anything runs. The answer is
``{"ok", "step", "error", "rejected"}`` with ``step`` the feed's record
(a :class:`core.protocol.feed.Step`) or None when nothing happened."""
from __future__ import annotations

from ingetrazo_core import EditTagCommand, LinkSubsetCommand, SetLocationCommand, SetTagStateCommand

from ingetrazo_core.command_protocol import (hide, lock, make_unique, move_vertices, redo,
    rename_placement, rename_placements, set_tag, undo, unhide)
from ingetrazo_core.command_protocol import (
    move_placements, copy_placements, rotate_selection, scale_selection, set_group_scale, move_selection, flip_selection,
    erase, erase_stroke, reverse_faces, orient_faces, resegment_curve, follow_me, rebuild_cap, divide_edges, join_edges, purge_debris,
    intersect_faces, solid_operation, repair_solid, explode_group, split_into_pieces,
    explode_view, purge_unused, set_group_axes, paste_clipboard, add_guide,
    change_guide, add_guide_circle, bevel_selection, push_pull, offset_outline,
    paint_placements, paint_material, place_group,
    set_location, edit_link_subset, edit_section_plane, edit_assembly_part, edit_visibility_set,
    edit_saved_scene, set_tag_state, rename_component, set_model_units, set_dimension_style,
    set_annotation_font,
    add_edge, add_edges, add_face, add_arc,
    add_geo_path, move_geo_path_node, add_text_label, add_dimension, place_section_plane, set_north, add_image_plane,
    set_material_pbr, set_material_base, rename_material, edit_material, duplicate_material, delete_material, create_material, name_unnamed_colours,
    edit_loose_geometry, promote_tag, file_placements, edit_structure_node,
    reset_link_tags, delete_placements, unlink_placements, edit_tag,
)
from ingetrazo_core.deliveries import create_delivery, edit_delivery, delete_delivery, run_delivery
from core.protocol.records import patch_record_field, set_filing_rule


class _Headless:
    """The viewport surface the Push/Pull tool's commit uses."""

    def __init__(self, history):
        self.scene = history.scene
        self.history = history

    def update(self):
        pass

    def set_hover(self, entity):
        pass

    def set_suppressed_faces(self, faces):
        pass

    def flash_status(self, text, msec=2500):
        pass



COMMANDS = {fn.__name__: fn for fn in (
    set_location, set_model_units, set_annotation_font, set_dimension_style, patch_record_field,
    set_filing_rule, create_delivery, edit_delivery, delete_delivery, run_delivery, follow_me,
    intersect_faces, add_edge, add_edges, add_face, add_guide, change_guide, add_guide_circle,
    add_arc, move_placements, copy_placements, move_selection, scale_selection, place_group,
    erase_stroke, add_geo_path, move_geo_path_node, add_text_label, add_dimension,
    place_section_plane, set_north, add_image_plane, set_group_scale, set_group_axes,
    flip_selection, paste_clipboard, bevel_selection, rotate_selection, offset_outline,
    paint_material, divide_edges, join_edges, move_vertices, erase, reverse_faces, push_pull,
    orient_faces, rebuild_cap, solid_operation, explode_group, split_into_pieces, explode_view,
    purge_unused, purge_debris, undo, redo, hide, unhide, lock, rename_placement,
    rename_placements, paint_placements, make_unique, edit_assembly_part, edit_visibility_set,
    edit_saved_scene, edit_link_subset, reset_link_tags, unlink_placements, edit_section_plane,
    edit_loose_geometry, file_placements, promote_tag, edit_structure_node, edit_tag, set_tag,
    set_tag_state, resegment_curve, repair_solid, rename_material, edit_material,
    duplicate_material, delete_material, delete_placements, rename_component, create_material,
    set_material_base, set_material_pbr, name_unnamed_colours,
)}


def run(history, name, /, **args):
    try:
        fn = COMMANDS[name]
    except KeyError:
        raise ValueError(f"unknown command {name!r}; one of {sorted(COMMANDS)}") from None
    return fn(history, **args)

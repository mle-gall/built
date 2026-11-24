# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Import-provider compatibility hooks and thread-local parse state."""
from functools import partial
from threading import local

from ingetrazo_core import (
    reader_compat as _native,
    skp_annotations,
    skp_definition_spans as _vff_definition_spans,
    skp_read_pbr as read_pbr,
    skp_tag_folders as parse_tag_folders,
)

_applied = False
_parse_tables = local()
WORKFLOW_CLASSIC, WORKFLOW_PBR = 0, 1
NORMAL_STYLE_OPENGL, NORMAL_STYLE_DIRECTX = 0, 1
_DEFAULT_TAGS = ("Layer0", "Layer 0", "Untagged")

apply = _native.apply
group_definitions = _native.group_definitions
kinds_known = _native.kinds_known
reset_group_definitions = _native.reset_group_definitions
hidden_tags = _native.hidden_tags
tag_folder_records = _native.tag_folder_records
resolve_face_layers = _native.resolve_face_layers
resolve_vff_annotations = _native.resolve_vff_annotations
attach_pbr = _native.attach_pbr
_parse_pages = _native._parse_pages
_find_node = _native._find_node
_keep_tag_folders = _native._keep_tag_folders
_keep_pbr = _native._keep_pbr
_keep_layer_table = _native._keep_layer_table
_keep_vff_annotation_inputs = _native._keep_vff_annotation_inputs
_note_definition_kinds = _native._note_definition_kinds
_note_legacy_groups = _native._note_legacy_groups
_scan_vff_dimensions = partial(skp_annotations, True)
_scan_vff_texts = partial(skp_annotations, False)

# SPDX-License-Identifier: GPL-3.0-or-later
"""Host bindings for the Rust .skp reader and classic model provider."""
import gc
from collections import OrderedDict
from contextlib import contextmanager

from core.texture import (
    colorize_image as _colorize_image, image_has_cutout as _image_has_cutout,
    hls_to_rgb as _hls_to_rgb, rgb_to_hls as _rgb_to_hls,
    projection_basis as _plane_basis,
)
from formats.skp_consumption import (
    SoftSegments, _GeometryCache, _geometry_cache, _remember_geometry,
)
from formats.skp_coordinates import PointRows as _PointRows
from ingetrazo_core import (
    Matrix4 as QMatrix4x4, Vec3 as QVector3D,
    skp_adapt as _adapt, skp_parse_raw as _parse_raw,
    skp_adapt_inner as _adapt_inner, skp_cached_colorize as _cached_colorize,
    skp_collect as _collect, skp_curve_segments as _curve_segments,
    skp_defn_geom as _defn_geom, skp_empty_model as _is_empty_model,
    skp_face_entry as _face_entry, skp_file_layers as file_layer_records,
    skp_image_attached as _rests_on_a_face, skp_image_bounds as _image_geometry_bounds,
    skp_image_faces as _image_quad_faces, skp_instance_identity as _instance_identity,
    skp_instance_matrix as _matrix, skp_inverted_png as _inverted_png,
    skp_loose_segments as _loose_edge_segments, skp_mark_drapes as _mark_projected_faces,
    skp_parse as parse, skp_materials as _material_attrs, skp_merge_protos as _merge_equal_protos,
    skp_needs_tint as _needs_tint, skp_positioned_uvs as _positioned_uvs,
    skp_ring as _ring_raw, skp_soft_segments as _soft_edge_segments,
    skp_texture_dir as _texture_dir, skp_vff_model, skp_world_points as _world_points,
)

_INCH = 0.0254
EXACT_TREE = True
_GEOMETRY_CACHE_BYTES = 64 * 1024 * 1024
_COLORIZE_CACHE_LIMIT = 32 * 1024 * 1024
_colorize_cache = OrderedDict()

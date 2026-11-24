# SPDX-License-Identifier: GPL-3.0-or-later
"""Shared native mesh collection for interchange writers and rendering."""
from ingetrazo_core import (
    SmoothNormals, _add_face, _facing_quad, _feet, _resolve_finishes, _shown,
    _turned_toward, _worn_outright, collect_geometry, collect_geometry_split,
    corner_normals, export_names, export_triangles, faceme_figures, geolocation,
    material_key, world_faces, world_faces_painted,
)

_DEFAULT_COLOR = (0.96, 0.95, 0.925)

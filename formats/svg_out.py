# SPDX-License-Identifier: GPL-3.0-or-later
"""Layered view drawings at paper scale."""
from ingetrazo_core import (
    chain_segments,
    svg_image_uri as _image_uri,
    svg_uv_to_page as _uv_to_page,
    view_svg_save as save_svg_layers,
    view_svg_text as svg_text,
)

DEFAULT_PENS = {"cut": 0.5, "profile": 0.35, "edge": 0.18}
MAX_TEXTURE_PX = 1024
BAKE_PX_PER_MM = 8.0
MAX_BAKE_PX = 2048
_SEAM_MM = 0.15

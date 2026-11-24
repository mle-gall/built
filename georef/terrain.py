# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Native terrain sampling and display geometry."""
from dataclasses import dataclass, field
from functools import partialmethod
from ingetrazo_core import build_mosaic, build_terrain, terrain_bounds, terrain_height

TILE_PX = 256


@dataclass(eq=False)
class TerrainObject:
    vertices: list
    uvs: list
    triangles: list
    tile_range: tuple
    nx: int = 0
    ny: int = 0
    bbox: tuple = (0., 0., 0., 0.)
    visible: bool = field(default=True, init=False)
    texture_image: object = field(default=None, init=False)

    height_at = partialmethod(terrain_height)
    bounds = partialmethod(terrain_bounds)

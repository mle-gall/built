# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""DEM providers and the requesting host's elevation signal."""
from dataclasses import dataclass
from functools import partialmethod
from georef.transport import Service, Signal
from georef.tiles import TileCache
from ingetrazo_core import (
    decode_elevation, bilinear, pixel_in_tile, tile_url, dem_init, dem_fetcher, dem_tile,
    dem_field, dem_area, dem_elevation, dem_local, dem_has,
)

TILE_PX = 256

@dataclass(frozen=True)
class DEMSource:
    """A raster DEM provider: an ``{z}/{x}/{y}`` URL plus its RGB encoding.

    ``encoding`` is ``"terrarium"`` (AWS Terrain Tiles / Mapzen) or ``"mapbox"``
    (Mapbox Terrain-RGB). Duck-compatible with :class:`~georef.tiles.TileSource`
    (``id``/``url``/``max_zoom``), so it rides the same fetcher and cache.
    """

    id: str
    name: str
    url_template: str
    encoding: str = "terrarium"
    max_zoom: int = 15
    attribution: str = ""

    url = partialmethod(tile_url)


AWS_TERRAIN = DEMSource(
    id="aws_terrarium",
    name="AWS Terrain Tiles (SRTM/NED)",
    url_template="https://s3.amazonaws.com/elevation-tiles-prod/terrarium/"
                 "{z}/{x}/{y}.png",
    encoding="terrarium",
    max_zoom=15,
    attribution="Terrain Tiles hosted on AWS by Mapzen/Nextzen — sources "
                "include SRTM, NED, Copernicus. https://registry.opendata.aws/terrain-tiles/",
)


class DEMSampler(Service):
    changed = Signal()
    __init__ = partialmethod(dem_init)
    _ensure_fetcher = partialmethod(dem_fetcher)
    _on_tile = partialmethod(dem_tile)
    _field_for = partialmethod(dem_field)
    ensure_area = partialmethod(dem_area)
    elevation_at = partialmethod(dem_elevation)
    elevation_at_local = partialmethod(dem_local)
    has_tile = partialmethod(dem_has)

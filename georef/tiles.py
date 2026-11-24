# SPDX-License-Identifier: GPL-3.0-or-later
"""XYZ map source definitions and native tile capture and caching."""
from dataclasses import dataclass, field
from functools import partialmethod
from ingetrazo_core import (
    deg2num, num2deg, tile_bbox, tiles_covering, TileCache, default_cache_dir,
    tile_url, custom_source, source_slug, tile_init, tile_from_dict, tile_to_dict,
    add_patch, set_rectangle, patch_tiles, flat_tiles, cap_detail, capture_bounds, visible_tiles, quad_local,
)

@dataclass(frozen=True)
class TileSource:
    """A slippy-map provider: an ``{z}/{x}/{y}`` URL template plus metadata.

    ``url_template`` may use ``{z}``, ``{x}``, ``{y}`` in any order (ArcGIS
    serves ``{z}/{y}/{x}``) and an optional ``{s}`` subdomain placeholder that
    rotates through ``subdomains``.
    """

    id: str
    name: str
    url_template: str
    max_zoom: int = 19
    attribution: str = ""
    subdomains: tuple[str, ...] = field(default_factory=tuple)
    tile_size: int = 256

    url = partialmethod(tile_url)

PRESETS: dict[str, TileSource] = {
    "esri_imagery": TileSource(
        id="esri_imagery",
        name="Esri World Imagery (satellite)",
        url_template="https://server.arcgisonline.com/ArcGIS/rest/services/"
                     "World_Imagery/MapServer/tile/{z}/{y}/{x}",
        max_zoom=19,
        attribution="Esri, Maxar, Earthstar Geographics, and the GIS community",
    ),
    "s2cloudless": TileSource(
        id="s2cloudless",
        name="Sentinel-2 cloudless (EOX)",
        url_template="https://tiles.maps.eox.at/wmts/1.0.0/"
                     "s2cloudless-2020_3857/default/GoogleMapsCompatible/"
                     "{z}/{y}/{x}.jpg",
        max_zoom=17,
        attribution="Sentinel-2 cloudless — https://s2maps.eu by EOX IT Services "
                    "GmbH (Contains modified Copernicus Sentinel data)",
    ),
    "osm": TileSource(
        id="osm",
        name="OpenStreetMap (streets)",
        url_template="https://tile.openstreetmap.org/{z}/{x}/{y}.png",
        max_zoom=19,
        attribution="© OpenStreetMap contributors",
    ),
}

DEFAULT_SOURCE_ID = "esri_imagery"


class TileLayer:
    __init__ = partialmethod(tile_init)
    to_dict = partialmethod(tile_to_dict)
    from_dict = classmethod(tile_from_dict)
    set_rectangle = partialmethod(set_rectangle)
    add_patch = partialmethod(add_patch)
    capture_bounds = partialmethod(capture_bounds)
    cap_detail = partialmethod(cap_detail)
    _patch_tiles = partialmethod(patch_tiles)
    flat_tiles = partialmethod(flat_tiles)
    visible_tiles = partialmethod(visible_tiles)
    quad_local = partialmethod(quad_local)

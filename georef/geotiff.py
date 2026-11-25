# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Public orthomosaic metadata and native TIFF reader."""
from dataclasses import dataclass
from pathlib import Path
from typing import Optional
from functools import partialmethod

from ingetrazo_core import (
    geotiff_info as read_info, georeferenced as _placed, pixel_to_map as _pixel,
    corners_geodetic as _corners, ground_size as _size, unsupported_reason, read_rgba, describe,
)

COMPRESSION_NONE = 1
COMPRESSION_LZW = 5
COMPRESSION_JPEG = 7
COMPRESSION_DEFLATE = 8
COMPRESSION_PACKBITS = 32773
COMPRESSION_DEFLATE_OLD = 32946

class GeoTiffError(ValueError):
    """The file is not a TIFF this reader can place or decode; the message
    says which part (shown to the user as is)."""


@dataclass
class GeoTiffInfo:
    """Header of a GeoTIFF: the raster layout plus its map placement."""
    path: Path
    width: int
    height: int
    samples: int
    bits: tuple
    compression: int
    photometric: int
    planar: int
    tiled: bool
    tile_w: int
    tile_h: int
    offsets: tuple
    bytecounts: tuple
    predictor: int = 1
    jpeg_tables: bytes = b""
    extra_samples: tuple = ()
    sample_format: tuple = ()
    big: bool = False
    byte_order: str = "<"
    #: Pixel → map affine ``(a, b, c, d, e, f)``: X = a·col + b·row + c,
    #: Y = d·col + e·row + f, for the pixel's top-left CORNER (PixelIsPoint
    #: headers are shifted half a pixel here, so callers never care).
    transform: Optional[tuple] = None
    #: EPSG of the map CRS when the file states one.
    epsg: Optional[int] = None
    #: UTM zone / hemisphere the map coordinates are in, or ``None`` when
    #: the CRS is geographic (``geographic=True``: X = lon, Y = lat) or unknown.
    zone: Optional[int] = None
    northern: Optional[bool] = None
    geographic: bool = False
    citation: str = ""
    nodata: Optional[str] = None

    georeferenced = property(_placed)
    pixel_to_map = partialmethod(_pixel)
    corners_geodetic = partialmethod(_corners)
    ground_size = partialmethod(_size)

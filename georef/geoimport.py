# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Georeferenced alignments parsed by the native file readers."""
from __future__ import annotations

from dataclasses import dataclass, field

from ingetrazo_core import load_features, parse_geojson, parse_kml, parse_kmz


@dataclass
class GeoFeature:
    """A parsed feature: ordered ``(lat, lon)`` points + open/closed + name."""
    points: list[tuple[float, float]] = field(default_factory=list)
    closed: bool = False
    name: str = ""

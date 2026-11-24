# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Import georeferenced alignments (Track G) — KML/KMZ and GeoJSON.

The realistic road/GIS flow: you trace the alignment in Google Earth or QGIS
(with your own SHP layers as reference there), export it, and bring it into
IngeTrazo **correctly located** to profile / measure it. This module parses the
geometry to plain ``(lat, lon)`` features; the caller projects them through the
scene datum into local :class:`~georef.geopath.GeoPath` traces.

Pure parsing (stdlib only: ``xml.etree``, ``json``, ``zipfile``) — no GIS deps,
no CRS handling (KML and GeoJSON are WGS84 lon/lat by definition). Projected
formats (SHP/DXF, usually UTM) are a later addition that needs CRS support.
"""
from __future__ import annotations

import json
from ingetrazo_core import parse_kml, parse_kmz
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class GeoFeature:
    """A parsed feature: ordered ``(lat, lon)`` points + open/closed + name."""
    points: list[tuple[float, float]] = field(default_factory=list)
    closed: bool = False
    name: str = ""


def _drop_closing_dup(points, closed):
    """A closed ring repeats its first vertex last — drop it (GeoPath closes it)."""
    if closed and len(points) > 1:
        a, b = points[0], points[-1]
        if abs(a[0] - b[0]) < 1e-9 and abs(a[1] - b[1]) < 1e-9:
            return points[:-1]
    return points


def parse_geojson(text: str) -> list[GeoFeature]:
    """Parse GeoJSON text into features (LineString/Polygon + Multi* variants)."""
    try:
        data = json.loads(text)
    except (ValueError, TypeError):
        return []
    feats: list[GeoFeature] = []

    def add_line(coords, name):
        pts = [(c[1], c[0]) for c in coords if len(c) >= 2]
        if len(pts) >= 2:
            feats.append(GeoFeature(pts, False, name))

    def add_ring(coords, name):
        pts = _drop_closing_dup([(c[1], c[0]) for c in coords if len(c) >= 2], True)
        if len(pts) >= 3:
            feats.append(GeoFeature(pts, True, name))

    def geom(g, name):
        if not isinstance(g, dict):
            return
        t, c = g.get("type"), g.get("coordinates")
        if t == "LineString":
            add_line(c or [], name)
        elif t == "MultiLineString":
            for line in c or []:
                add_line(line, name)
        elif t == "Polygon":
            if c:
                add_ring(c[0], name)
        elif t == "MultiPolygon":
            for poly in c or []:
                if poly:
                    add_ring(poly[0], name)
        elif t == "GeometryCollection":
            for sub in g.get("geometries", []):
                geom(sub, name)

    def obj(o):
        if not isinstance(o, dict):
            return
        t = o.get("type")
        if t == "FeatureCollection":
            for f in o.get("features", []):
                obj(f)
        elif t == "Feature":
            name = (o.get("properties") or {}).get("name") or ""
            geom(o.get("geometry"), str(name))
        else:
            geom(o, "")

    obj(data)
    return feats


def load_features(path) -> list[GeoFeature]:
    """Parse a KML / KMZ / GeoJSON file at ``path`` into features."""
    path = Path(path)
    ext = path.suffix.lower()
    if ext == ".kmz":
        return parse_kmz(path)
    text = path.read_text(encoding="utf-8", errors="replace")
    if ext == ".kml":
        return parse_kml(text)
    if ext in (".geojson", ".json"):
        return parse_geojson(text)
    # Unknown extension — sniff the content.
    stripped = text.lstrip()
    if stripped.startswith("<"):
        return parse_kml(text)
    return parse_geojson(text)

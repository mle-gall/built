# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Scene datum — the bridge between geographic coordinates and the scene.

Everything the modelling engine touches lives in **local metres** relative to a
scene origin (:class:`SceneDatum`). This is non-negotiable: ``QVector3D`` is
float32, so feeding raw UTM eastings (~500 km) straight into the mesh drops
precision to centimetres. The datum anchors the scene at one geographic point
and projects every other point into a local, continuous, metre-based frame:

    X = easting  (east,  metres from anchor)
    Y = northing (north, metres from anchor)
    Z = altitude (metres from anchor altitude)

matching IngeTrazo's Z-up / X-east / Y-north convention.

Projection is UTM (WGS84 Transverse Mercator). The datum **freezes the UTM
zone** at construction and projects every point through that single zone, so
local coordinates stay continuous even if the model spills across a zone
boundary — never recompute the zone per point. The forward series is ported
from IngePresupuestos' ``latlon_a_utm``; the inverse is the standard Snyder
Transverse Mercator expansion. Both round-trip to well under a millimetre at
city scale (see ``tests/test_datum.py``).

The forward and inverse projections are shared with the native location editor
through ``ingetrazo_core``.
"""
from __future__ import annotations

import math
from dataclasses import dataclass

from ingetrazo_core import Vec3 as QVector3D, utm_forward, utm_inverse, zone_for_lon


def _wrap_deg(deg: float) -> float:
    """Fold an angle into (-180, 180] so 350° and -10° are the same north."""
    d = (deg + 180.0) % 360.0 - 180.0
    return 180.0 if d == -180.0 else d


@dataclass
class SceneDatum:
    """Anchors the scene at a geographic point; converts geodetic ↔ local metres.

    ``lat``/``lon`` (degrees, WGS84) and ``alt`` (metres) are the sole source of
    truth — the UTM zone, hemisphere and false-origin offsets are derived and
    kept in sync in :meth:`__post_init__`. Serialise only ``lat``/``lon``/``alt``.
    """

    lat: float
    lon: float
    alt: float = 0.0
    # North angle: where true north lies, in
    # degrees CLOCKWISE from the model's +Y (green) axis. 0 = the green axis
    # points north. It turns the base map, terrain and every geographic
    # import under the model instead of the model under them — so a plaza
    # drawn square to the axes keeps its standard views, axis locks and
    # rectangle tool after it is placed on a site that isn't north-aligned.
    north: float = 0.0

    def __post_init__(self) -> None:
        self.lat = float(self.lat)
        self.lon = float(self.lon)
        self.alt = float(self.alt)
        self.north = _wrap_deg(float(self.north))
        self.zone = zone_for_lon(self.lon)
        self.northern = self.lat >= 0
        # UTM coordinates of the anchor — the local-frame origin.
        self._east0, self._north0 = utm_forward(self.lat, self.lon, self.zone)
        b = math.radians(self.north)
        self._cos_n, self._sin_n = math.cos(b), math.sin(b)

    # ---- North angle --------------------------------------------------------
    def grid_to_local_xy(self, east: float, north: float) -> tuple[float, float]:
        """Rotate a metre offset from the anchor (grid east, grid north) into
        the model's frame — identity when the north angle is 0."""
        c, s = self._cos_n, self._sin_n
        return east * c + north * s, north * c - east * s

    def local_to_grid_xy(self, x: float, y: float) -> tuple[float, float]:
        """Inverse of :meth:`grid_to_local_xy`: model frame → grid offset."""
        c, s = self._cos_n, self._sin_n
        return x * c - y * s, x * s + y * c

    def north_vector(self) -> QVector3D:
        """Unit vector pointing to true north, in the model's frame."""
        return QVector3D(self._sin_n, self._cos_n, 0.0)

    @property
    def hemisphere(self) -> str:
        return "N" if self.northern else "S"

    # ---- Conversions --------------------------------------------------------
    def geodetic_to_local(self, lat: float, lon: float,
                          alt: float | None = None) -> QVector3D:
        """Geodetic (degrees + metres) → local scene metres (X east, Y north).

        Omitting ``alt`` means **on the scene's reference plane** (local Z = 0),
        not at sea level. Every caller that doesn't carry an elevation — base
        map tiles, an imported KML alignment — wants the ground plane, and a
        default of 0.0 metres absolute quietly meant "``self.alt`` below the
        scene" the moment a datum acquired an altitude. That put the base map
        1.8 km underground on the first georeferenced survey.
        """
        east, north = utm_forward(lat, lon, self.zone)
        z = 0.0 if alt is None else alt - self.alt
        x, y = self.grid_to_local_xy(east - self._east0, north - self._north0)
        return QVector3D(x, y, z)

    def local_to_geodetic(self, point: QVector3D) -> tuple[float, float, float]:
        """Local scene metres → geodetic ``(lat, lon, alt)`` (degrees + metres)."""
        de, dn = self.local_to_grid_xy(point.x(), point.y())
        lat, lon = utm_inverse(de + self._east0, dn + self._north0,
                               self.zone, self.northern)
        return lat, lon, point.z() + self.alt

    def utm_to_local(self, east: float, north: float,
                     alt: float = 0.0) -> QVector3D:
        """UTM metres (in the datum's frozen zone — survey CSVs report these
        directly) → local scene metres. Pure offsets, no reprojection."""
        x, y = self.grid_to_local_xy(east - self._east0, north - self._north0)
        return QVector3D(x, y, alt - self.alt)

    def local_to_utm(self, point: QVector3D) -> tuple[float, float, float]:
        """Local scene metres → UTM ``(east, north, alt)`` in the frozen zone."""
        de, dn = self.local_to_grid_xy(point.x(), point.y())
        return (de + self._east0, dn + self._north0, point.z() + self.alt)

    # ---- Serialisation ------------------------------------------------------
    def to_dict(self) -> dict:
        d = {"lat": self.lat, "lon": self.lon, "alt": self.alt}
        if self.north:            # older readers ignore it; 0 stays terse
            d["north"] = self.north
        return d

    @classmethod
    def from_dict(cls, data: dict) -> "SceneDatum":
        return cls(data["lat"], data["lon"], data.get("alt", 0.0),
                   north=data.get("north", 0.0))

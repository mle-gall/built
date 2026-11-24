# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Polygon geometry for plugins, renderers and object-mesh previews.

Triangulation returns triples of the original vertex objects in draw order.
The kernel projects binary64 coordinates and clips holes in the face plane.
"""
from ingetrazo_core import (
    Vec3 as QVector3D,
    drawing_plane_axes,
    polygon_earcut as earcut,
    polygon_is_convex as is_convex,
    polygon_normal,
    polygon_triangles as triangulate,
)

Point2 = tuple[float, float]
Tri3 = tuple[QVector3D, QVector3D, QVector3D]


def plane_axes(normal: QVector3D) -> tuple[QVector3D, QVector3D]:
    return drawing_plane_axes(normal)


def _newell(vertices: list[QVector3D]) -> QVector3D:
    return polygon_normal(vertices)

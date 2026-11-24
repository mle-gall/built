# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Geometry primitives: vertex, edge, face, solid.

Edges and faces use identity-based equality so each instance is a distinct
entity that can be tracked in a selection ``set``. Two edges (or two
faces) with the same coordinates are still treated as separate objects.
"""
from __future__ import annotations

from dataclasses import dataclass, field

from ingetrazo_core import Vec3 as QVector3D, polygon_centroid, polygon_newell


@dataclass(eq=False)
class Edge:
    """A line segment between two 3D points (identity-equal)."""

    a: QVector3D
    b: QVector3D

    def length(self) -> float:
        return (self.b - self.a).length()


@dataclass(eq=False)
class Face:
    """A planar polygon defined by its outer vertex loop, with optional holes.

    No internal triangulation is stored — consumers (renderer, picker) call
    :meth:`triangulate`. A face with no holes and a convex loop reduces to a
    cheap fan; a face that has been divided by an inner loop (``holes``) is a
    "donut" and is triangulated by :mod:`core.triangulate`.

    Each entry in ``holes`` is an inner vertex loop that has been subtracted
    from this face (e.g. a small rectangle drawn inside a larger one). The
    inner loop is also usually its own :class:`Face`; the hole here is what
    keeps this (mother) face from overlapping it.

    The face normal is derived with Newell's method so it works for
    arbitrary polygons (including non-planar ones, where it returns the
    best-fit plane normal).
    """

    vertices: list[QVector3D] = field(default_factory=list)
    holes: list[list[QVector3D]] = field(default_factory=list)
    #: Optional display attrs ("color", "texture") — tool PREVIEWS carry them
    #: so a paste drags the real look; the topology mesh has its own Face.
    attrs: dict | None = None

    def _newell(self) -> QVector3D:
        return polygon_newell(self.vertices)

    def normal(self) -> QVector3D:
        if len(self.vertices) < 3:
            return QVector3D(0.0, 0.0, 1.0)
        n = self._newell()
        if n.length() < 1e-9:
            return QVector3D(0.0, 0.0, 1.0)
        return n.normalized()

    def area(self) -> float:
        """Area of the outer loop (holes not subtracted). Used to disambiguate
        coplanar faces that overlap under the cursor — the smaller one is the
        inner patch the user is pointing at."""
        if len(self.vertices) < 3:
            return 0.0
        return 0.5 * self._newell().length()

    def centroid(self) -> QVector3D:
        return polygon_centroid(self.vertices)

    def triangulate(self) -> list[tuple[QVector3D, QVector3D, QVector3D]]:
        """Triangles covering this face (outer loop minus any holes).

        Delegates to :mod:`core.triangulate`, which fans a convex loop, ear-
        clips a concave one, and bridges holes when present — so a concave or
        holed face renders and picks correctly while convex faces keep the
        cheap fan path.
        """
        if len(self.vertices) < 3:
            return []
        # Imported lazily to keep geometry.py dependency-light at import time.
        from core.triangulate import triangulate

        return triangulate(self.vertices, self.holes, self.normal())

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Planar arrangement: rebuild the minimal faces of a plane from its edges.

Given a tangle of segments lying in one plane (hand-drawn walls, slivers,
crossings, T-junctions, doubled lines), recompute the *minimal bounded faces* —
the robust, deterministic replacement for the heuristic heal pass.

Classic DCEL face extraction:

1. Project the segments to the plane's 2D coordinates.
2. Split every segment at its intersections / overlaps with the others, snapping
   endpoints to a quantised grid so coincident points become one vertex.
3. Prune dangling (degree-1) chains — they border no face.
4. At each vertex, sort the outgoing half-edges by angle; trace each face by
   always taking the *next edge clockwise* (interior kept on the left), which
   yields bounded faces wound CCW and the unbounded face wound CW.
5. Keep the CCW cycles as faces; nest each CW cycle (a hole) inside the smallest
   CCW face that contains it; drop the one CW cycle that contains everything
   (the unbounded outer face).

The result is returned in 3D as ``(outer_loop, [hole_loops])`` per face, ready to
hand to ``AddFaceCommand``. Self-overlapping pathologies aside, this is exact for
the room/notch/hole geometry a floor plan is made of.
"""
from collections import namedtuple

from ingetrazo_core import (
    arrangement as _kernel, arrange_plane, object_drawing,
    plane_interior_point as _interior_point, plane_intersections as _intersections,
    plane_scan_point as scan_interior_point, plane_segment_parameter as _on_seg,
    plane_split_graph as _build_graph,
)

_TOL = 1e-4
_Seg = namedtuple("_Seg", "a b")
_key = _kernel.grid_key
_area_floor = _kernel.area_floor
plane_basis = _kernel.plane_basis
_signed_area = _kernel.signed_area
_point_in_polygon = _kernel.point_in_polygon
_classify = _kernel.classify
coplanar_plane = object_drawing.coplanar_plane
flat_drawing = _kernel.flat_drawing
busy_plane = _kernel.busy_plane
curve_extras = _kernel.curve_extras
arc_command = _kernel.arc_command


def planar_arrangement(segments_3d, origin, normal):
    """Ordered bounded faces, as outer and hole loops of model-space points."""
    return arrange_plane(segments_3d, origin, plane_basis(normal), False)[1]


def planar_rebuild(segments_3d, origin, normal):
    """Return every split edge (including spurs) and the bounded faces."""
    return arrange_plane(segments_3d, origin, plane_basis(normal), True)

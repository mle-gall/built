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
from __future__ import annotations

import math

from ingetrazo_core import (Vec3 as QVector3D, object_drawing, plane_intersections as _intersections,
                            plane_segment_parameter as _on_seg)

# Position quantisation (~0.1 mm at metre scale) — coincident points within this
# collapse to one vertex, matching the rest of the engine's welding tolerance.
_TOL = 1e-4


def _key(x: float, y: float) -> tuple[int, int]:
    return (round(x / _TOL), round(y / _TOL))


def _area_floor(xy) -> float:
    """The least area a traced loop needs to be a face rather than noise.

    A loop thinner than the weld tolerance -- an out-and-back walk, a
    rounding sliver -- has a mean width ``2·area/perimeter`` under ``_TOL``.
    The old floor was ``_TOL`` itself read as an AREA: 1 cm², so a 5 mm
    circle or a countersink's rim never became a face (tutorial hunt 2, B8).
    The cap stays at ``_TOL`` so a big loop is never asked for more than
    before."""
    n = len(xy)
    per = sum(math.hypot(xy[(i + 1) % n][0] - xy[i][0],
                         xy[(i + 1) % n][1] - xy[i][1]) for i in range(n))
    return min(_TOL, _TOL * per / 2.0)


def plane_basis(normal: QVector3D) -> tuple[QVector3D, QVector3D]:
    """Two orthonormal in-plane axes for ``normal`` (stable choice of reference)."""
    n = normal.normalized()
    ref = QVector3D(1.0, 0.0, 0.0)
    if abs(QVector3D.dotProduct(ref, n)) > 0.9:
        ref = QVector3D(0.0, 1.0, 0.0)
    u = (ref - n * QVector3D.dotProduct(ref, n)).normalized()
    v = QVector3D.crossProduct(n, u).normalized()
    return u, v


# ---- 2D geometry helpers ---------------------------------------------------

def _signed_area(loop: list) -> float:
    s = 0.0
    n = len(loop)
    for i in range(n):
        x0, y0 = loop[i]
        x1, y1 = loop[(i + 1) % n]
        s += x0 * y1 - x1 * y0
    return s * 0.5


def _point_in_polygon(p, poly: list) -> bool:
    """Ray-cast even-odd test; ``poly`` is a list of (x, y)."""
    x, y = p
    inside = False
    n = len(poly)
    j = n - 1
    for i in range(n):
        xi, yi = poly[i]
        xj, yj = poly[j]
        if ((yi > y) != (yj > y)) and (
            x < (xj - xi) * (y - yi) / (yj - yi + 1e-30) + xi
        ):
            inside = not inside
        j = i
    return inside


# ---- Arrangement -----------------------------------------------------------

def _build_graph(segments_2d):
    """Split all segments at intersections and return ``(coords, adjacency)``:
    ``coords`` maps a vertex key to its (x, y); ``adjacency`` maps a key to the
    set of neighbour keys (undirected, deduplicated, no self-loops)."""
    from ingetrazo_core import plane_split_graph
    return plane_split_graph(segments_2d)


def _classify(loops, coords) -> list:

    from ingetrazo_core import plane_nesting
    rings = [[coords[k] for k in loop] for loop in loops]
    return [(rings[i], [rings[j] for j in holes])
            for i, holes in plane_nesting(rings, True)]


def planar_arrangement(segments_3d, origin: QVector3D, normal: QVector3D) -> list:
    """Rebuild minimal faces from coplanar ``segments_3d`` (list of
    ``(QVector3D, QVector3D)``). Returns ``[(outer_loop, [hole_loops]), …]`` with
    every loop a list of ``QVector3D`` on the plane."""
    from ingetrazo_core import arrange_plane
    return arrange_plane(segments_3d, origin, plane_basis(normal), False)[1]


def planar_rebuild(segments_3d, origin: QVector3D, normal: QVector3D):
    """Full planar rebuild: returns ``(edges_3d, faces_3d)`` where ``edges_3d`` is
    the complete split edge set (so nothing is lost, including spurs) and
    ``faces_3d`` is ``[(outer, [holes])]`` — exactly what the command re-adds."""
    from ingetrazo_core import arrange_plane
    return arrange_plane(segments_3d, origin, plane_basis(normal), True)


def coplanar_plane(points, tol: float = 1e-4):
    """``(origin, normal)`` of the plane through ``points`` (a list of
    ``QVector3D``) if they're all coplanar, else ``None``. Collinear or
    degenerate input returns a horizontal plane (the arrangement then yields no
    face), so the caller can still run safely."""
    return object_drawing.coplanar_plane(points, tol)


def flat_drawing(scene, new_points) -> bool:
    """Whether the whole drawing (existing mesh + the new loop) shares one
    plane — the gate for the classic planar arrangement, which forms
    every intersection face deterministically. 3D models keep the naive path."""
    return object_drawing.coplanar_plane(new_points, vertices=scene.mesh.vertices) is not None


def busy_plane(scene, new_points):
    """``(origin, normal)`` of the plane the new loop lies on, when that plane
    already carries mesh content (edges or faces) — the gate for the scoped
    per-plane arrangement in 3D scenes. As soon as ANY solid exists, the
    whole-mesh flat gate goes dark; without this, a circle drawn over another
    circle on the ground stacks two full discs instead of splitting into three
    areas. ``None`` when the plane is virgin (naive path is enough)."""
    plane = coplanar_plane(list(new_points))
    if plane is None:
        return None
    return plane if object_drawing.has_plane_edge(scene.mesh.edges, plane, False) else None


def curve_extras(scene, pts):
    """The commands after a closed curve's edges (a circle, a polygon, a
    pie): tagged as one curve, with the faces of its plane rebuilt.

    A flat drawing rebuilds the plane's faces from the edge graph (the
    deterministic planar arrangement: a circle crossing a square splits
    both); a 3D scene whose plane already carries content runs the SCOPED
    arrangement on that plane, the disc going in first so the new regions
    count as covered; a virgin plane takes the disc as is. The outline is
    drawn either way: only a swept curve's facets hide (Push/Pull)."""
    from core.history import (AddFaceCommand, RebuildPlaneFacesCommand,
                              RebuildPlanarFacesCommand, TagCurveCommand)
    if flat_drawing(scene, pts):
        return [TagCurveCommand(list(pts), closed=True), RebuildPlanarFacesCommand()]
    plane = busy_plane(scene, pts)
    if plane is not None:
        return [AddFaceCommand(list(pts)), TagCurveCommand(list(pts), closed=True),
                RebuildPlaneFacesCommand(*plane)]
    return [AddFaceCommand(list(pts)), TagCurveCommand(list(pts), closed=True)]

def _interior_point(loop: list):
    """A point safely inside the polygon ``loop`` (average of a triangle fan
    vertex that tests inside), good enough for containment nesting."""
    from ingetrazo_core import plane_interior_point
    return plane_interior_point(loop)


def scan_interior_point(outer: list, holes=()):
    """A point well inside ``outer`` and outside every hole, or None.

    A horizontal line halfway between two consecutive vertex heights crosses
    the region in spans; the middle of the widest span (measured against
    the band's height too) is as far from every boundary as such a simple
    probe gets, whatever the shape — a C, an L, a ring."""
    from ingetrazo_core import plane_scan_point
    return plane_scan_point(outer, holes)


def arc_command(scene, pts, close_to=None, trim=None):
    """The command that draws an arc polyline (the Arc tools', Freehand's):
    the curve tagged open, with the plane rule of ``curve_extras`` (planar
    rebuild on a flat drawing, the scoped one on a busy plane in 3D, naive
    otherwise). ``close_to`` (a centre) closes the wedge with two radii,
    the Pie. ``trim``: ``(a, b)`` position pairs to erase once the arc is
    in, the corner stubs a fillet leaves, so the rounded face survives and
    the corner sliver goes with its edges."""
    from collections import namedtuple
    from core.edits import build_add_edges
    from core.history import (DeleteEdgesCommand, RebuildPlaneFacesCommand,
                              RebuildPlanarFacesCommand, TagCurveCommand)
    seg = namedtuple("_Seg", "a b")
    segments = [(pts[i], pts[i + 1]) for i in range(len(pts) - 1)]
    if close_to is not None:
        segments = [(close_to, pts[0])] + segments + [(pts[-1], close_to)]
    after = [DeleteEdgesCommand([seg(QVector3D(a), QVector3D(b)) for a, b in trim])] if trim else []
    tag = TagCurveCommand(list(pts), closed=False)
    if flat_drawing(scene, pts):
        return build_add_edges(scene, segments, detect_faces=False,
                               extra=[tag, RebuildPlanarFacesCommand()] + after)
    plane = busy_plane(scene, pts)
    if plane is not None:
        return build_add_edges(scene, segments, detect_faces=True,
                               extra=[tag, RebuildPlaneFacesCommand(*plane)] + after)
    return build_add_edges(scene, segments, detect_faces=True, extra=[tag] + after)

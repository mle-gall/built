# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Snap engine: pick the best snap target near the cursor.

Snap kinds (priority high → low):
1. ``"axis"``           — explicit axis lock from arrow keys.
2. ``"reference"``      — parallel/perpendicular lock to a reference edge.
3. ``"axis"`` via Shift — Shift held while an axis inference is active.
4. ``"close"``          — closing the current polygon chain.
5. ``"endpoint"``       — vertex of an existing edge.
6. ``"intersection"``   — where two edges / guide lines actually cross (green X).
   Under a direction lock the crossing of the lock line with an edge is
   ``"intersection_edge"`` and with a face (loose, in a group or component)
   ``"intersection_face"``.
7. ``"midpoint"``       — midpoint of an existing edge.
   ``"face_center"``    — centre of area of the face under the cursor or
                          of a selected face (with the endpoints, which
                          win a tie).
8. ``"origin"``         — world origin.
9. ``"on_edge"``        — arbitrary point along an edge (start a shape on it).
10. ``"aligned"``       — level with / in line with an encouraged point across
                          the face under the cursor (the axis plane through it).
11. ``"axis_inference"`` — soft auto-detected axis alignment (visual cue only).
12. ``"on_face"``       — the free point on the face under the cursor
                          (or, just above it, ``"intersection"`` where that
                          face meets another with no edge between them).
13. ``"none"``          — no snap.

Some kinds name a POINT of the model and some name the FACE the cursor is
over; ``POINT_SNAP_KINDS`` says which is which, and a planar shape reads it
to know whether to re-plan onto a point or to follow that face (#545).

Distance checks for point snaps are done in **screen-space pixels** so the
snap radius stays constant under zoom. The caller supplies a
``world_to_pixel`` callback so this module does not depend on Qt's
viewport directly.
"""
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Callable, Optional

import ingetrazo_core as kernel
from ingetrazo_core import Vec3 as QVector3D

from .geometry import Edge

# Colors used by the rubber band and the 2D snap indicator. RGB floats [0, 1].
COLOR_ENDPOINT = (0.16, 0.62, 0.36)
COLOR_MIDPOINT = (0.20, 0.66, 0.74)  # cyan — midpoint of an edge
COLOR_ON_EDGE = (0.86, 0.22, 0.27)   # red — arbitrary point on an edge
COLOR_ON_FACE = (0.42, 0.46, 0.92)   # blue/violet — point on a face
COLOR_ORIGIN = (0.95, 0.45, 0.16)
COLOR_CLOSE = (0.20, 0.40, 0.78)
COLOR_AXIS_X = (0.86, 0.22, 0.27)
COLOR_AXIS_Y = (0.16, 0.62, 0.36)
COLOR_AXIS_Z = (0.20, 0.40, 0.78)
COLOR_REFERENCE = (0.85, 0.30, 0.80)  # magenta — parallel / perpendicular
COLOR_EXTENSION = (0.55, 0.55, 0.58)  # grey — collinear extension of an edge
COLOR_LOCK_INTERSECT = (0.85, 0.20, 0.20)  # red — a Shift lock meeting an acquired edge / face
COLOR_IN_GROUP = (0.85, 0.30, 0.80)   # magenta — a point inside a group / component
COLOR_TANGENT = (0.20, 0.66, 0.74)    # cyan — an arc tangent to the arc it starts from
COLOR_NONE = (0.0, 0.0, 0.0)

AXIS_COLORS = {
    "x": COLOR_AXIS_X,
    "y": COLOR_AXIS_Y,
    "z": COLOR_AXIS_Z,
}

AXIS_NAMES = {"x": "X", "y": "Y", "z": "Z"}

#: The snap kinds that name a real POINT of the model — a vertex, a midpoint,
#: a crossing, a spot on an edge, the origin. When the cursor's own snap is
#: one of these, that point is what the user aimed at (it may merely lie on a
#: face, or on no face at all) and a shape re-plans onto it, through
#: ``PlaneLock.snapped_plane``.
#:
#: Every OTHER kind names the FACE the cursor is over — ``"on_face"`` itself,
#: the inferences the engine keeps ON that face (``"aligned"``,
#: ``"from_point"``, ``"extension"``), the axis/shift locks — and there the
#: face IS the answer: a planar shape follows it (``Viewport.
#: _planar_face_plane``, #545). Same test as ``RotatedRectangleTool.
#: _POINT_SNAPS``, which asks the same question of the protractor's width.
POINT_SNAP_KINDS = frozenset({
    "endpoint", "midpoint", "intersection", "intersection_edge",
    "intersection_face", "on_edge", "on_line", "on_circle", "close",
    "origin", "center", "centre", "component_origin", "arc_midpoint",
    "quadrant",
})


@dataclass
class SnapResult:
    point: QVector3D
    kind: str
    color: tuple[float, float, float] = COLOR_NONE
    axis: Optional[str] = None  # "x" / "y" / "z" when kind is "axis" or "axis_inference"
    # Two world points defining a dashed guide line to draw (the extension
    # inference shows the dashed continuation of the edge to the cursor).
    guide: Optional[tuple] = None
    # Colour for the dashed guide line when it should differ from the marker
    # colour (e.g. 'from point' draws an axis-coloured guide but a green point).
    guide_color: Optional[tuple[float, float, float]] = None
    # Extra dashed guides, ``[(a, b, rgb), ...]`` — the two-point 'from point'
    # draws one from each encouraged point.
    guides: Optional[list] = None
    # ``"group"`` / ``"component"`` when the point belongs to one — the
    # ScreenTip adds "in group" / "in component".
    context: Optional[str] = None
    #: The ScreenTip's own words, already translated — what an extension's
    #: inference says («Level PA»); wins over the kind's built-in label.
    label: Optional[str] = None
    #: The colour of the lock the point was found under (an arrow-key or
    #: Shift axis, a reference): the draw line keeps it while the point
    #: snaps to a corner on that line — locked to red, the line stays red
    #: (#711, BastelBaus: «keep line color»).
    lock_color: Optional[tuple[float, float, float]] = None
    # 17.10.26 BasteBaus: stores an edge if snapped to an edge to be used in 
    #                     dimension tool do directly create a dimension on an edge
    edge: Optional[Edge] = None


# ---- Helpers ---------------------------------------------------------------

def _axis_tuples():
    return tuple(axis.toTuple() for axis in _AXIS_VECTORS.values())


def project_to_view_plane(point: QVector3D, ref: QVector3D,
                          forward: QVector3D, threshold: float = 0.97):
    """Project *point* onto the plane through *ref* facing the camera, when
    the view is axis-aligned (a standard front/plan/side view).

    In a front elevation, a dimension should read the FRONTAL span — the
    height/width you see — not the true 3-D diagonal that a bit of depth
    between the two picked points would add. So the moving endpoint has its
    depth (the dominant view axis) pulled to the reference point's, keeping
    the measurement in the plane of the drawing. Oblique views return the
    point untouched (true 3-D distance, the classic behaviour).
    """
    plane = first_point_work_plane(forward, ref, threshold)
    if plane is None:
        return point
    a = plane[1]
    # Do the cancellation in doubles before returning the Qt point.
    offset = sum((r-p)*v for r,p,v in zip(ref.toTuple(),point.toTuple(),a.toTuple()))
    return QVector3D(*(p+offset*v for p,v in zip(point.toTuple(),a.toTuple())))


def first_point_work_plane(forward: QVector3D, scene_center: QVector3D,
                           threshold: float = 0.97):
    """Fallback work plane for a tool's FIRST point clicked in empty space.

    When the camera looks (near-)straight down a principal axis — a
    standard view (front / plan / side) — the natural plane to place an
    unsnapped point on is the one FACING the camera, through the model's
    centre, not the ground: dimensioning a front view must stay in the
    frontal plane instead of dropping the point to Z=0 at some arbitrary
    depth. Returns ``(point, normal)`` for such axis-aligned views, or
    ``None`` for oblique (orbit) views, where the caller keeps its usual
    ground/face behaviour so freehand modelling is unchanged.
    """
    from core.axes import is_world
    hit = kernel.drawing_axis(forward.normalized().toTuple(), _axis_tuples(),
                                      threshold, 1e-9, is_world())
    return (scene_center, QVector3D(*hit[1])) if hit is not None else None


from ingetrazo_core import face_plane_world


def _detect_axis_alignment(
    start: QVector3D, candidate: QVector3D, angle_deg: float
) -> Optional[str]:
    """If the start→candidate direction is within ``angle_deg`` of an axis,
    return that axis as ``"x"``/``"y"``/``"z"``. Otherwise ``None``."""
    delta = candidate - start
    hit = kernel.drawing_axis(delta.toTuple(), _axis_tuples(),
                                      math.cos(math.radians(angle_deg)), 1e-6, False)
    return "xyz"[hit[0]] if hit is not None else None


#: How far along an axis the resolved point may land, as a multiple of how
#: far the cursor itself is from the start. See ``_engaged_axis``.
_MAX_AXIS_REACH = 5.0


def _detect_axis_on_screen(
    start: QVector3D, candidate_world: QVector3D,
    candidate_pixel: tuple[float, float], world_to_pixel, threshold_px: float
) -> Optional[str]:
    """Which axis the cursor is sitting on, measured in PIXELS.

    The world-space detector above can only ever see the axes contained in
    the work plane, because that is where ``_world_from_pixel`` puts the
    candidate. Measured over the whole camera grid on 2026-09-18: every
    camera reaches exactly two axes and never three — x/y from the top,
    x/z from the front, y/z from the side. That is issue #31's second
    point, and it is geometry, not a threshold that needs widening.

    A straight line in the world projects to a straight line on screen, so
    two projected points give the axis's exact screen line and the question
    becomes the cursor's perpendicular distance to it. The reach is taken
    from how far the user is already drawing, which keeps the sampled point
    in the same region of the screen as the cursor.
    """
    reach = (candidate_world - start).length()
    if reach < 1e-6:
        return None
    origin_px = world_to_pixel(start)
    if origin_px is None:
        return None
    samples = [world_to_pixel(start + a * reach) for a in _AXIS_VECTORS.values()]
    axis = kernel.screen_axis(origin_px, samples, candidate_pixel, threshold_px)
    return "xyz"[axis] if axis is not None else None


#: How far (degrees) a draw may drift from an engaged axis before the STICKY
#: axis lets go. Generous on purpose: once a line is being drawn along an
#: axis, a hand drifting in perspective must not drop it (issue #1095) — but
#: far enough that a deliberate diagonal still breaks out.
_STICKY_AXIS_RELEASE_DEG = 20.0


#: How far out of a plane an axis may sit and still count as IN it: the same
#: 1 mm-per-metre the shapes use to tell a point off their plane
#: (``PlaneLock.OFF_PLANE_TOL``), so everything agrees on what "on the plane"
#: means.
_IN_PLANE_TOL = 1e-3


def _direction_from_edge(edge, mode: str,
                        plane_normal: Optional[QVector3D] = None
                        ) -> Optional[QVector3D]:
    """Return a unit-length direction for ``mode``.

    ``parallel``      → the edge's own direction.
    ``perpendicular`` → square to the edge, IN THE PLANE BEING DRAWN ON.

    The plane matters. Perpendicular used to be a flat 90° turn in XY,
    which is right on the ground and wrong everywhere else: on a ramp it
    locks the line to a horizontal direction that does not lie on the ramp,
    so drawing from one edge of the slope across to the other was pulled
    off the face by a magenta lock that could not be satisfied (Marco,
    2026-09-10). ``cross(plane_normal, edge)`` is the same thing on a
    horizontal plane — cross((0,0,1), (dx,dy,dz)) is (-dy, dx, 0), the old
    formula exactly — and the right thing on a tilted or vertical one.
    """
    normal = plane_normal if plane_normal is not None else _AXIS_VECTORS["z"]
    direction = kernel.edge_direction(edge.a.toTuple(), edge.b.toTuple(),
                                               mode == 'perpendicular', normal.toTuple())
    return QVector3D(*direction) if direction is not None else None


# ---- Entry point ------------------------------------------------------------

# Type alias for the projection callback the viewport provides. Given the
# chain start point and a direction in world space, it returns the closest
# point on that infinite line to the camera ray that currently passes
# through the cursor pixel. This is what makes Z-axis locks usable (and
# X/Y locks correct when the start point is off the ground plane).
ProjectOntoLine = Callable[[QVector3D, QVector3D], QVector3D]


# Unit vectors for each world axis, used by the axis lock paths.
# The SAME dict as core.axes.AXES (updated in place): the world's axes at
# the top level, the open group's own inside it (issue #44).
from core.axes import AXES as _AXIS_VECTORS  # noqa: E402


def _closest_on_segment_2d(
    p: tuple[float, float], a: tuple[float, float], b: tuple[float, float]
) -> tuple[float, float]:
    """Closest point on 2D segment ``ab`` to ``p``. Returns ``(distance, t)``
    where ``t`` in [0, 1] is the parameter along ``ab`` of that closest point."""
    ax, ay = a
    bx, by = b
    px, py = p
    dx = bx - ax
    dy = by - ay
    denom = dx * dx + dy * dy
    if denom == 0.0:
        return math.hypot(px - ax, py - ay), 0.0
    t = ((px - ax) * dx + (py - ay) * dy) / denom
    t = max(0.0, min(1.0, t))
    qx = ax + t * dx
    qy = ay + t * dy
    return math.hypot(px - qx, py - qy), t


def _line_segment_intersection(
    p: QVector3D, u: QVector3D, a: QVector3D, b: QVector3D, tol: float = 1e-3
) -> Optional[QVector3D]:
    """Where the infinite line through ``p`` (unit direction ``u``) crosses the
    segment ``a``–``b``, or ``None`` if they're parallel, skew (closest approach
    > ``tol``), or the crossing falls outside the segment."""
    hit = kernel.lock_crossing(p.toTuple(), u.toTuple(), a.toTuple(), b.toTuple(), tol)
    return QVector3D(*hit) if hit is not None else None


def _point_on_segment_world(
    p: QVector3D, a: QVector3D, b: QVector3D, tol: float = 1e-3
) -> bool:
    """Whether ``p`` lies on segment ``a``–``b`` (within ``tol`` world units)."""
    ab = b - a
    length_sq = QVector3D.dotProduct(ab, ab)
    if length_sq < 1e-12:
        return (p - a).length() < tol
    t = QVector3D.dotProduct(p - a, ab) / length_sq
    if t < -1e-6 or t > 1.0 + 1e-6:
        return False
    return (p - (a + ab * t)).length() < tol


def fit_circle(points, tol_rel: float = 0.01):
    """The circle through ``points`` (world, coplanar): ``(centre, radius)``
    or ``None`` when they do not sit on one. Least squares in the points'
    own plane; every point must lie within ``tol_rel`` of the radius (or
    2 mm), so a polyline that merely bends never passes for an arc."""
    hit = kernel.snap_circle([p.toTuple() for p in points], tol_rel)
    return (QVector3D(*hit[0]), hit[1]) if hit is not None else None


def arc_sweep_midpoint(points, fit=None) -> Optional[QVector3D]:
    """The «Arc midpoint» of an open curve chain through ``points`` (its
    distinct vertices, in any order): the point of the arc bisecting its
    sweep — not any facet's midpoint — or ``None`` (not on a circle, or a
    sweep whose vertices balance out around the centre). ``fit`` is the
    chain's :func:`fit_circle` when the caller already has it.

    The one rule both views offer: the GL viewport reads it per mesh
    (``Viewport._mesh_arc_midpoints``), the Metal view per prototype of its
    snap index (``views.wgpu_inference._curve_features``)."""
    if fit is None:
        if len(points) < 3:
            return None
        fit = fit_circle(list(points))
        if fit is None:
            return None
    c, r = fit[0], fit[1]
    point = kernel.arc_midpoint([p.toTuple() for p in points], c.toTuple(), r)
    return QVector3D(*point) if point is not None else None


_boundary_runs = kernel.boundary_runs


def curve_centers_of_face(face, xform=None) -> list:
    """Return (world center, radius, (ring, run)) for each circular boundary run."""
    from core.store_mesh import FaceView, _store_of
    if isinstance(face, FaceView):
        store = _store_of(face._m)
        if store is not None and store.is_live(face.key):
            return store.curve_centers(face.key, xform)
    return kernel.curve_centers(face, xform)


def face_center_of_face(face, xform=None) -> Optional[QVector3D]:
    """The *Face center* inference: the centre of AREA of the face, holes
    taken out, in world space (``xform`` places a component's face).

    Not the average of the corners (``Face.centroid``): a trapezoid, or a
    face with many vertices crowded on one side (a rounded corner), pulls
    that average away from the middle.

    A centre in a HOLE still counts: the middle of a frame or a ring is
    exactly the point one is after. ``None`` when it falls outside the
    outer boundary — the elbow of an L, where the point belongs to no
    part of the face — and for a degenerate face."""
    loops = [list(face.vertices)] + [list(h) for h in (face.holes or [])]
    if xform is not None:
        loops = [[xform.map(QVector3D(p)) for p in loop] for loop in loops]
    center = kernel.face_center([[p.toTuple() for p in ring] for ring in loops])
    return QVector3D(*center) if center is not None else None


def _vertex_on_line(
    vertex: QVector3D, line_start: QVector3D, line_dir: QVector3D, tol: float = 1e-4
) -> bool:
    """Whether ``vertex`` lies on the infinite line through ``line_start`` in
    ``line_dir`` (perpendicular distance below ``tol``)."""
    rel = vertex - line_start
    proj = QVector3D.dotProduct(rel, line_dir) * line_dir
    return (rel - proj).length() < tol


def _two_point_snap(
    refs, candidate, cx, cy, world_to_pixel, threshold_px, is_occluded=None,
) -> Optional[SnapResult]:
    """Where the axis lines through two encouraged points cross: for every
    pair of points and pair of DIFFERENT axes, the crossing (if the two
    lines meet — coplanar within a millimetre) within the snap radius of
    the cursor. Green point, one dotted guide from each point in its axis
    colour."""
    hit = kernel.cross_guide(refs, _axis_tuples(), (cx, cy), threshold_px, world_to_pixel, is_occluded)
    if hit is None:
        return None
    cross, p, k, q, l = hit
    ka, kb = "xyz"[k], "xyz"[l]
    return SnapResult(cross, "from_point", COLOR_ENDPOINT,
                      guide=(QVector3D(p), cross), guide_color=AXIS_COLORS[ka],
                      guides=[(QVector3D(q), cross, AXIS_COLORS[kb])])


def _first_point_from_point(
    ref, candidate, cx, cy, world_to_pixel, threshold_px, is_occluded=None,
    plane_normal=None, project_onto_line=None, free_3d=False,
) -> Optional[SnapResult]:
    """The foot of the cursor on the axis line through ``ref`` (the last
    hovered corner), when the cursor sits within the snap radius of one of
    the three axis lines — a green 'from point' with the axis-coloured
    dotted guide back to the corner. Sitting ON the corner is the endpoint
    snap's job, not this one's.

    With ``plane_normal`` — the cursor is on a face, and ``candidate`` lies
    on that face's plane — the answer stays ON that plane: an axis line
    that pierces the plane offers its piercing point, one lying in the
    plane its foot as before, and one running parallel to the plane off
    it offers nothing. It used to hand back the foot on the line in the
    air, which is a point nowhere near the wall the cursor is pointing at
    and, for a rectangle started on that wall, a corner off its plane.

    ``free_3d`` — a tool that is not a planar ``PlaneLock`` (the Line, the
    free placements) — reads the foot off the cursor RAY instead, so an
    elevated corner still offers its horizontal 'level with' line while the
    cursor sits on the ground (#1095)."""
    hit = kernel.axis_guide(ref, candidate, tuple(_AXIS_VECTORS.values()), (cx, cy),
                            threshold_px, world_to_pixel, is_occluded, plane_normal,
                            project_onto_line if free_3d else None)
    if hit is None:
        return None
    k, foot = hit
    axis = "xyz"[k]
    return SnapResult(foot, "from_point", COLOR_ENDPOINT,
                      guide=(QVector3D(ref), foot),
                      guide_color=AXIS_COLORS[axis], axis=axis)


def _in_plane_with_point_snap(
    ref, candidate, plane_normal, cx, cy, world_to_pixel, threshold_px,
    is_occluded=None,
) -> Optional[SnapResult]:
    """Level with — or in line with — an encouraged point ACROSS the face
    under the cursor: where the axis plane through ``ref`` (the horizontal
    plane at its height, or a vertical one through it) cuts the plane the
    cursor is on. That cut is a line lying on the face; when the cursor is
    within the snap radius of it, the snap is the cursor's foot on it.

    Rafael, 2026-09-16 (02:20), putting windows on a house: the corner of
    the first window gives its dotted line along its own wall «estupendamente»,
    but a window on ANOTHER wall at the same height had nothing to line up
    with — «que la línea guía se extendiera por aquí y yo pudiera fijar la
    ventana aquí… tampoco eso lo hace […]». It does not: the classic
    'from point' is the axis LINE through the corner, which meets a
    perpendicular wall in a single point and the opposite wall not at all.
    The plane through the corner meets both in a line.

    Two dotted guides, so the eye can walk from the corner to the cursor:
    from ``ref`` to where it projects onto the line (along the first
    wall), and from there along the line to the foot (along the second),
    each in the colour of the axis it runs along. ``axis`` names the
    plane's normal: ``"z"`` is 'level with', the other two 'in line with'.

    Sits BELOW the axis lines through the point (a line on the face is
    exactly one of those when the point is on the same wall, and the foot
    is then the same point) — an inference derived from a point never
    beats the point, and this one never beats the line it generalises."""
    hit = kernel.plane_guide(ref, candidate, plane_normal, _axis_tuples(), (cx, cy),
                             threshold_px, world_to_pixel, is_occluded)
    if hit is None:
        return None
    k, foot, elbow, first, second, extra = hit
    def color(index):
        return COLOR_EXTENSION if index is None else AXIS_COLORS["xyz"[index]]
    return SnapResult(foot, "aligned", COLOR_ENDPOINT, axis="xyz"[k], guide=(elbow, foot),
                      guide_color=color(second),
                      guides=[(QVector3D(ref), elbow, color(first))] if extra else None)


def _from_point_snap(
    scene, start_point, draw_dir, cx, cy, world_to_pixel, threshold_px,
    is_occluded, extra_point=None, axis_deg: float = 10.0,
    hovered_refs: bool = False, line_dir: Optional[QVector3D] = None,
    project_onto_line: Optional[ProjectOntoLine] = None,
    edge_body: bool = True, query=None,
) -> Optional[SnapResult]:
    """'From point' inference ("Desde el punto"), the single clean version.

    Without ``line_dir``, fires only when the draw runs along an axis (within
    ``axis_deg``), the way the red/green/blue axis line lights up. Under
    an explicit directional lock, ``line_dir`` supplies that locked direction
    instead. For every corner (and midpoint) it snaps to the fixed foot of that
    point on the draw line, pinning one spot instead of sliding or scattering.

    Corners → green 'from point' with an axis-coloured guide; midpoints → cyan.

    With ``hovered_refs`` (the arrow-key lock) the cursor may also sit on the
    REFERENCE itself — a corner or midpoint, or any point of an edge — far
    from the draw line, and the snap lands on that reference's foot. That is
    how the lock is used: Tape from the wall's bottom edge, ↑, hover
    the window's corner, and the guide takes the window's height (Rafael,
    04:20: «cuando pulso la flechita para subir no me hace el snap»)."""
    if start_point is None:
        return None
    adir = kernel.reference_direction(draw_dir, line_dir, _axis_tuples(), axis_deg)
    if adir is None:
        return None
    if query is None:
        from core.snap_query import make_query
        query = make_query(scene.edges, world_to_pixel, (cx, cy))
    def hidden(p):
        return is_occluded(QVector3D(*p))

    hit = query.reference_foot(start_point.toTuple(), adir.toTuple(), threshold_px,
                               hovered_refs, extra_point.toTuple() if extra_point is not None else None,
                               hidden if is_occluded is not None else None)
    if hit is not None:
        foot, ref, kind = hit
        color = {"from_point": COLOR_ENDPOINT, "midpoint": COLOR_MIDPOINT, "origin": COLOR_ORIGIN}[kind]
        return _from_point_result(QVector3D(*foot), QVector3D(*ref), kind, color)
    if not hovered_refs or not edge_body:
        return None

    def project(a, d):
        p = project_onto_line(QVector3D(*a), QVector3D(*d))
        return p.toTuple() if p is not None else None

    body = query.target(threshold_px, [], project if project_onto_line else None,
                        (lambda pts: [hidden(p) for p in pts]) if is_occluded else None,
                        True, True)
    if body is None:
        return None
    ref = QVector3D(*body[0])
    foot = kernel.body_reference(start_point, adir, ref, (cx, cy), threshold_px,
                                 world_to_pixel, is_occluded)
    if foot is None:
        return None
    return _from_point_result(foot, ref, "from_point", COLOR_ENDPOINT)


def _from_point_result(foot, ref, kind, color):
    # The guide runs perpendicular to the draw, from the corner to the foot —
    # coloured by its axis when it RUNS along one (red/green/blue), grey
    # when it does not: a guide across the axes painted blue read as «along
    # blue» (#711).
    guide_color = None
    if kind == "from_point":
        axis = kernel.reference_colour(foot, ref, _axis_tuples())
        guide_color = AXIS_COLORS['xyz'[axis]] if axis is not None else COLOR_EXTENSION
    return SnapResult(foot, kind, color, guide=(ref, foot),
                      guide_color=guide_color)


def _pairs_that_may_meet(edges):
    """The pairs (i < j, in order) of ``edges`` whose boxes come close
    enough for ``segment_intersection`` to find a meeting: it accepts a
    point up to its tolerance past either end and a gap up to
    ``max(tol, 1e-7 * length)``, so each box grows by more than that.
    Pairs apart are never asked — 1128 pairs under the cursor on the plant
    of #158, most of them edges a metre away from each other."""
    n = len(edges)
    if n < 8:
        return ((i, j) for i in range(n) for j in range(i + 1, n))
    import numpy as np
    from core.topology import _SPLIT_TOLERANCE as tol
    ends = np.array([(e.a.x(), e.a.y(), e.a.z(), e.b.x(), e.b.y(), e.b.z())
                     for e in edges], dtype=np.float64).reshape(n, 2, 3)
    lo, hi = ends.min(axis=1), ends.max(axis=1)
    length = np.linalg.norm(ends[:, 1] - ends[:, 0], axis=1)
    grow = (2.5 * tol + 2e-7 * length
            + 1e-6 * np.abs(ends).max(axis=(1, 2)))[:, None]
    lo, hi = lo - grow, hi + grow
    apart = ((lo[:, None, :] > hi[None, :, :])
             | (lo[None, :, :] > hi[:, None, :])).any(axis=2)
    ii, jj = np.nonzero(np.triu(~apart, 1))
    return zip(ii.tolist(), jj.tolist())


def _intersection_snap(
    cx, cy, scene, world_to_pixel, et, is_occluded, are_occluded=None,
    candidate_world=None,
) -> Optional[SnapResult]:
    """Green intersection point where two edges, guide lines or guide circles
    actually cross.

    Every other ``"intersection"`` in this module is *directional*: it only
    appears while a lock line is active (an axis lock, a perpendicular draw, an
    extension). Two construction guides crossing produce no such direction, so
    their meeting point was never offered — the cursor slid along whichever
    guide was nearest (``on_edge``). The X of two guides is the whole reason to
    draw them, so collect the targets whose screen span passes under the cursor
    and intersect them pairwise in 3-D (edge intersection, circle-circle and
    circle-segment for the guide circles).

    ``segment_intersection`` rejects parallel and *skew* pairs, so two edges
    that merely cross in projection do not light up a point that isn't there.
    Real model edges that cross are already split into a shared vertex by the
    topology pass (the endpoint snap wins first); this mainly catches the
    guides, which live outside the mesh, and any unsplit crossing."""

    from core.axes import plane_axes
    from core.snap_query import make_query
    query = make_query(scene.edges, world_to_pixel, (cx, cy))
    circles = []
    if candidate_world is not None:
        for circle in getattr(scene, "circles", ()):
            geometry = (circle.center.toTuple(), circle.radius, circle.normal.toTuple())
            basis = plane_axes(circle.normal)[0].toTuple()
            if query.rim(geometry, candidate_world.toTuple(), basis, et) is not None:
                circles.append(geometry)
    hits = query.crossings(et, circles=circles)
    points = [QVector3D(*p) for p, _ in hits]
    index = _first_visible(points, is_occluded, are_occluded)
    if index is not None:
        kind = "endpoint" if hits[index][1] else "intersection"
        return SnapResult(points[index], kind, COLOR_ENDPOINT)
    return None


class SurfacePatch(tuple):
    """A face known by its plane and a membership test instead of its
    loops: ``(point, normal, contains, context)``. What a view that picks
    triangles, not faces (the Metal view), hands ``compute_snap`` as its
    ``face_outline``; ``contains(world point)`` says whether the point lies
    on the face under the cursor (inside it, not in a hole)."""
    __slots__ = ()

    def __new__(cls, point, normal, contains, context=None):
        return tuple.__new__(cls, (point, normal, contains, context))


def _edge_face_snap(
    cx, cy, scene, world_to_pixel, et, is_occluded, outline, query=None
) -> Optional[SnapResult]:
    """Green intersection where an edge — a guide line, a loose line, a
    facet of an arc or circle — pierces the face under the cursor.

    ``_intersection_snap`` only pairs edges with edges and
    ``face_crossing`` only faces with faces, so a line through a face
    offered nothing. ``outline`` is ``(outer, holes, context)``: the face's
    loops in world space — or a :class:`SurfacePatch`, its plane and a
    membership test. The crossing counts only INSIDE the face (outer
    loop, outside every hole), on the edge's own span, and under the cursor.
    An edge lying in the face's plane has no single crossing and is skipped."""
    if isinstance(outline, SurfacePatch):
        origin, n, inside, context = outline
        if n is None or n.length() < 1e-12:
            return None
        n = n.normalized()
    else:
        from core.topology import _point_inside_2d
        from core.triangulate import plane_axes
        outer, holes, context = outline
        if len(outer) < 3:
            return None
        nx = ny = nz = 0.0
        for i in range(len(outer)):
            a, b = outer[i], outer[(i + 1) % len(outer)]
            nx += (a.y() - b.y()) * (a.z() + b.z())
            ny += (a.z() - b.z()) * (a.x() + b.x())
            nz += (a.x() - b.x()) * (a.y() + b.y())
        n = QVector3D(nx, ny, nz)
        if n.length() < 1e-12:
            return None
        n = n.normalized()
        origin = outer[0]
        u, w = plane_axes(n)

        def proj(p):
            rel = p - origin
            return (QVector3D.dotProduct(rel, u), QVector3D.dotProduct(rel, w))

        outer2 = [proj(p) for p in outer]
        holes2 = [[proj(p) for p in h] for h in holes if len(h) >= 3]

        def inside(p):
            pt = proj(p)
            return (_point_inside_2d(pt, outer2)
                    and not any(_point_inside_2d(pt, h) for h in holes2))

    if query is None:
        from core.snap_query import make_query
        query = make_query(scene.edges, world_to_pixel, (cx, cy))
    for coords in query.plane_crossings(et, origin.toTuple(), n.toTuple()):
        hit = QVector3D(*coords)
        if inside(hit) and (is_occluded is None or not is_occluded(hit)):
            return SnapResult(hit, "intersection", COLOR_ENDPOINT, context=context)
    return None

def face_crossing_near_ray(px, py, origin, direction, f_plane, g_plane,
                           world_to_pixel, reach_px, side_px=3.0):
    """Where plane ``f_plane`` meets plane ``g_plane`` (each ``(point,
    unit normal)``) nearest the cursor ray ``origin + t·direction``, if it
    falls within ``reach_px`` of the cursor ``(px, py)`` on screen:
    ``(world point, (pixel one side, pixel the other side))`` — the two
    probes ``side_px`` either side of the crossing line, square to it on
    screen, where one face must show and then the other — or ``None``
    (near-coplanar planes, a crossing line running along the ray, out of
    reach).

    The geometry of the modeller's face/face «Intersection» (#819) that both
    views share; each asks its own picks which face shows at the probes
    (``Viewport._crossing_on_screen``, ``views.wgpu_inference.
    face_crossing``)."""
    p_f, n_f = f_plane
    p_g, n_g = g_plane
    hit = kernel.plane_cut_ray(origin.toTuple(), direction.toTuple(),
              (p_f.toTuple(), n_f.toTuple()), (p_g.toTuple(), n_g.toTuple()))
    if hit is None:
        return None
    x, sample = (QVector3D(*p) for p in hit)
    pix = world_to_pixel(x)
    if pix is None or math.hypot(pix[0] - px, pix[1] - py) > reach_px:
        return None
    # Either side of the crossing on screen: one face, then the other.
    q = world_to_pixel(sample)
    if q is None:
        return None
    sx, sy = q[0] - pix[0], q[1] - pix[1]
    ln = math.hypot(sx, sy)
    if ln < 1e-9:
        return None
    nx, ny = -sy / ln * side_px, sx / ln * side_px
    return x, ((pix[0] + nx, pix[1] + ny), (pix[0] - nx, pix[1] - ny))


def _lock_reference_snap(
    start_point, line_dir, world_to_pixel, hover_edge, hover_plane,
) -> Optional[SnapResult]:
    """the modeller's «constrained on line / plane» under a direction lock (arrow,
    Shift or reference lock): with the cursor ON an edge or a face, the point
    where the lock line meets that edge's INFINITE line
    (``intersection_edge``) or that face's PLANE (``intersection_face``) is
    offered automatically - the cursor need not be anywhere near it. The
    dotted guide runs from the edge's near end (or from the cursor, for a
    face) to the point. An edge wins over a face. ``hover_plane`` is
    ``(point, normal, ref)``; skew and parallel cases offer nothing."""
    if hover_edge is not None:
        a, b = hover_edge.a, hover_edge.b
        pair = kernel.lock_on_extension(start_point.toTuple(), line_dir.toTuple(),
                                                  a.toTuple(), b.toTuple())
        if pair is not None:
            hit, end = (QVector3D(*p) for p in pair)
            if world_to_pixel(hit) is not None:
                return SnapResult(hit, "intersection_edge", COLOR_LOCK_INTERSECT,
                                  guide=(end, hit), guide_color=COLOR_EXTENSION)
    if hover_plane is not None:
        p0, n, ref = hover_plane
        hit = kernel.lock_plane(start_point, line_dir, p0, n)
        if hit is not None and world_to_pixel(hit) is not None:
            return SnapResult(hit, "intersection_face", COLOR_LOCK_INTERSECT,
                              guide=(ref, hit), guide_color=COLOR_EXTENSION)
    return None


_resolve_point_candidates = kernel.resolve_points


def _lock_line_snaps(
    scene, start_point, line_dir, cx, cy, world_to_pixel, threshold_px,
    is_occluded, acquired_point, chain_first_point=None,
    project_onto_line=None, hover_edge=None, hover_plane=None, are_occluded=None, query=None,
) -> Optional[SnapResult]:
    """What a directional lock still lets you fetch, in order: the chain's
    first point (closing), named points and endpoints ON the lock line,
    an edge crossing, an on-line midpoint, then projected references.
    Corners, midpoints and the origin may be hovered off the lock line too.
    Issue #27 gave these to
    the arrow lock, #31 to the Shift lock, and #34 found the third lock —
    Shift held over a soft axis cue, when the press had nothing to capture
    — with none of them: «release Shift to reposition the camera and
    reactivate it: half of the Snap points are not detected». One body
    now, three callers. ``line_dir`` must already point at the cursor."""
    # The chain's own first point closes the polyline, lock or no lock.
    # Without this the lock TRAPS you in the chain: the rule returns
    # before the close rule is ever reached, so the click that should
    # have closed the figure came back as a plain lock and the Line tool
    # went on chaining (Marco, 2026-09-18: «se cerró la figura … sigue
    # apareciendo el eje X bloqueado» — it LOOKED closed). Only when the
    # point really is on the lock line, so closing never quietly breaks
    # the lock the user asked for.
    if chain_first_point is not None and _vertex_on_line(
            chain_first_point, start_point, line_dir):
        fp_px = world_to_pixel(chain_first_point)
        if fp_px is not None and math.hypot(
                fp_px[0] - cx, fp_px[1] - cy) <= threshold_px:
            return SnapResult(QVector3D(chain_first_point), "close", COLOR_CLOSE)

    if query is None:
        from core.snap_query import make_query
        query = make_query(scene.edges, world_to_pixel, (cx, cy))
    from core.snap_query import point_result
    line = (start_point.toTuple(), line_dir.toTuple())
    hit = point_result(query, scene.edges, threshold_px, is_occluded, line=line, hidden_many=are_occluded)
    if hit is not None:
        return hit
    points = [QVector3D(*p) for p, _ in query.crossings(threshold_px, line)]
    k = _first_visible(points, is_occluded, are_occluded)
    if k is not None:
        return SnapResult(points[k], "intersection", COLOR_ENDPOINT, guide=(start_point,points[k]))
    hit = point_result(query, scene.edges, threshold_px, is_occluded, line=line, midpoints=True, hidden_many=are_occluded)
    if hit is not None:
        return hit
    # 'From point' along the lock line: line up with a corner (green) or
    # midpoint (cyan) projected onto the locked axis — or hover it.
    # The cursor ON an edge: its meeting with the lock line is the answer
    # (the modeller's «constrained on line»), so the edge's body must not offer
    # an arbitrary 'from point' instead; corners and midpoints still do.
    edge_hit = (_lock_reference_snap(start_point, line_dir, world_to_pixel,
                                     hover_edge, None)
                if hover_edge is not None else None)
    hit = _from_point_snap(
        scene, start_point, line_dir, cx, cy, world_to_pixel,
        threshold_px, is_occluded, extra_point=acquired_point,
        hovered_refs=True, line_dir=line_dir,
        project_onto_line=project_onto_line,
        edge_body=edge_hit is None, query=query,
    )
    if hit is not None:
        return hit
    if edge_hit is not None:
        return edge_hit
    # Nothing better: the cursor over a face constrains the lock line to
    # that plane.
    return _lock_reference_snap(start_point, line_dir, world_to_pixel,
                                None, hover_plane)


_first_visible = kernel.first_visible


_closest_on_circle = kernel.closest_rim
_circle_circle_intersections = kernel.circle_crossings
_segment_circle_intersections = kernel.segment_crossings
_circle_quadrants = kernel.quadrants


def _circle_tangent_points(c, r, n, p):
    """The tangent points on a circle from an external point ``p`` (projected
    onto the circle's plane): 0 (inside), 1 (on the rim) or 2 points."""
    return [QVector3D(*q) for q in kernel.circle_tangents(
        (c.toTuple(), r, n.toTuple()), p.toTuple())]


def compute_snap(
    candidate_world: QVector3D,
    candidate_pixel: tuple[float, float],
    scene,
    world_to_pixel: Callable[[QVector3D], Optional[tuple[float, float]]],
    threshold_px: float,
    project_onto_line: Optional[ProjectOntoLine] = None,
    chain_first_point: Optional[QVector3D] = None,
    start_point: Optional[QVector3D] = None,
    axis_lock: Optional[str] = None,
    shift_held: bool = False,
    reference_edge=None,
    reference_mode: Optional[str] = None,
    inference_angle_deg: float = 3.0,
    screen_axis_px: Optional[float] = None,
    is_occluded: Optional[Callable[[QVector3D], bool]] = None,
    are_occluded: Optional[Callable[[list], list]] = None,
    face_under_cursor: bool = False,
    edge_threshold_px: Optional[float] = None,
    magnetic_axis_deg: Optional[float] = None,
    acquired_edge=None,
    acquired_point=None,
    acquired_face_normal=None,
    acquired_points=None,
    shift_lock_dir=None,
    shift_lock_color=None,
    shift_lock_ref=None,
    linear_mode: str = "all",
    diagonal_span: bool = False,
    work_plane_normal: Optional[QVector3D] = None,
    radial_arm: bool = False,
    axis_in_plane: bool = False,
    free_3d: bool = False,
    sticky_axis: Optional[str] = None,
    dwell_axis_lock: bool = False,
    hover_lock_edge=None,
    hover_lock_plane: Optional[tuple] = None,
    face_crossing: Optional[Callable[[], Optional[tuple]]] = None,
    face_outline: Optional[Callable[[], Optional[tuple]]] = None,
    face_centers: Optional[list] = None,
    face_context: Optional[str] = None,
    start_edges=None,
) -> SnapResult:
    options = locals()
    from core.snap_query import make_query
    query = make_query(scene.edges, world_to_pixel, candidate_pixel)
    from core.snap_frame import SnapFrame
    frame = SnapFrame(options, query)
    return kernel.resolve_snap(query, frame, frame.state)

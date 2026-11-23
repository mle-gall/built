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

from ingetrazo_core import Vec3 as QVector3D

from core import _native
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
    if hasattr(_native.native, 'drawing_axis'):
        plane = first_point_work_plane(forward, ref, threshold)
        if plane is None:
            return point
        a = plane[1]
        # Do the cancellation in doubles before returning the Qt point.
        offset = sum((r-p)*v for r,p,v in zip(ref.toTuple(),point.toTuple(),a.toTuple()))
        return QVector3D(*(p+offset*v for p,v in zip(point.toTuple(),a.toTuple())))
    if forward.length() < 1e-9:
        return point
    f = forward.normalized()
    from core import axes as _axes
    if not _axes.is_world():
        # Inside a turned group the standard views face ITS axes (#44).
        lf = _axes.to_local(f)
        comps = [(abs(lf.x()), "x"), (abs(lf.y()), "y"), (abs(lf.z()), "z")]
        best, name = max(comps)
        if best < threshold:
            return point
        a = _axes.AXES[name]
        return point + a * QVector3D.dotProduct(ref - point, a)
    ax, ay, az = abs(f.x()), abs(f.y()), abs(f.z())
    if max(ax, ay, az) < threshold:
        return point
    if ax >= ay and ax >= az:
        return QVector3D(ref.x(), point.y(), point.z())
    if ay >= az:
        return QVector3D(point.x(), ref.y(), point.z())
    return QVector3D(point.x(), point.y(), ref.z())


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
    if hasattr(_native.native, 'drawing_axis'):
        from core.axes import is_world
        hit = _native.native.drawing_axis(forward.normalized().toTuple(), _axis_tuples(),
                                          threshold, 1e-9, is_world())
        return (scene_center, QVector3D(*hit[1])) if hit is not None else None
    if forward.length() < 1e-9:
        return None
    f = forward.normalized()
    from core import axes as _axes
    if not _axes.is_world():
        lf = _axes.to_local(f)
        comps = [(abs(lf.x()), "x"), (abs(lf.y()), "y"), (abs(lf.z()), "z")]
        best, name = max(comps)
        if best < threshold:
            return None
        a = _axes.axis(name)
        sign = 1.0 if QVector3D.dotProduct(f, a) > 0 else -1.0
        return scene_center, a * sign
    if max(abs(f.x()), abs(f.y()), abs(f.z())) < threshold:
        return None                     # oblique view — not axis-aligned
    # snap the normal to the exact dominant axis so the plane is clean
    if abs(f.x()) >= abs(f.y()) and abs(f.x()) >= abs(f.z()):
        normal = QVector3D(1.0 if f.x() > 0 else -1.0, 0.0, 0.0)
    elif abs(f.y()) >= abs(f.z()):
        normal = QVector3D(0.0, 1.0 if f.y() > 0 else -1.0, 0.0)
    else:
        normal = QVector3D(0.0, 0.0, 1.0 if f.z() > 0 else -1.0)
    return scene_center, normal


def face_plane_world(face, xform=None):
    """A face's plane as ``(point, normal)`` in WORLD coordinates.

    A component instance's face lives in its prototype's local frame, so
    its centroid and normal describe the copy at the origin — using them
    raw put the drawing plane of every instance in the wrong place (no
    reference on the far face of a placed slat: Marco, 2026-09-03). The
    normal is rebuilt from the mapped polygon, exact for any affine map."""
    if xform is None:
        return face.centroid(), face.normal()
    pts = [xform.map(QVector3D(v)) for v in face.vertices]
    if hasattr(_native.native, 'face_plane') and pts:
        center, normal = _native.native.face_plane([p.toTuple() for p in pts])
        n = QVector3D(*normal)
        if n.isNull():
            n = xform.mapVector(face.normal()).normalized()
        return QVector3D(*center), n
    n = len(pts)
    c = QVector3D(0.0, 0.0, 0.0)
    for p in pts:
        c += p
    c /= float(n)
    nx = ny = nz = 0.0
    for i in range(n):
        a, b = pts[i], pts[(i + 1) % n]
        nx += (a.y() - b.y()) * (a.z() + b.z())
        ny += (a.z() - b.z()) * (a.x() + b.x())
        nz += (a.x() - b.x()) * (a.y() + b.y())
    normal = QVector3D(nx, ny, nz)
    if normal.length() < 1e-12:
        return c, xform.mapVector(face.normal()).normalized()
    return c, normal.normalized()


def _detect_axis_alignment(
    start: QVector3D, candidate: QVector3D, angle_deg: float
) -> Optional[str]:
    """If the start→candidate direction is within ``angle_deg`` of an axis,
    return that axis as ``"x"``/``"y"``/``"z"``. Otherwise ``None``."""
    delta = candidate - start
    if hasattr(_native.native, 'drawing_axis'):
        hit = _native.native.drawing_axis(delta.toTuple(), _axis_tuples(),
                                          math.cos(math.radians(angle_deg)), 1e-6, False)
        return "xyz"[hit[0]] if hit is not None else None
    length = delta.length()
    if length < 1e-6:
        return None
    cos_thresh = math.cos(math.radians(angle_deg))
    from core import axes as _axes
    if not _axes.is_world():
        delta = _axes.to_local(delta)     # the context's own axes (#44)
    nx = abs(delta.x()) / length
    ny = abs(delta.y()) / length
    nz = abs(delta.z()) / length
    # Pick the strongest alignment if multiple pass.
    candidates = [(nx, "x"), (ny, "y"), (nz, "z")]
    candidates.sort(reverse=True)
    if candidates[0][0] >= cos_thresh:
        return candidates[0][1]
    return None


#: An axis whose screen projection is shorter than this is pointing at the
#: camera: its image is a dot, every cursor is "on" it, and the
#: perpendicular distance below means nothing. Skip it rather than let it
#: swallow the screen (looking straight down would glue everything to blue).
_MIN_AXIS_SCREEN_PX = 12.0

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
    if hasattr(_native.native, 'screen_axis'):
        samples = [world_to_pixel(start + a * reach) for a in _AXIS_VECTORS.values()]
        axis = _native.native.screen_axis(origin_px, samples, candidate_pixel, threshold_px)
        return "xyz"[axis] if axis is not None else None
    sx, sy = origin_px
    cx, cy = candidate_pixel
    best: Optional[tuple[float, str]] = None
    for name in ("x", "y", "z"):
        sample_px = world_to_pixel(start + _AXIS_VECTORS[name] * reach)
        if sample_px is None:
            # The sample lands where the camera cannot project (behind it, or
            # off the near plane) - seen in the plan view zoomed onto the
            # origin, where the z sample goes behind the camera. No line to
            # measure against, so this axis offers no answer.
            continue
        ax, ay = sample_px
        dx, dy = ax - sx, ay - sy
        span = math.hypot(dx, dy)
        if span < _MIN_AXIS_SCREEN_PX:
            continue
        # Distance from the cursor to the INFINITE line: the axis runs both
        # ways from the start point, as the arrow lock does.
        dist = abs((cx - sx) * dy - (cy - sy) * dx) / span
        if dist <= threshold_px and (best is None or dist < best[0]):
            best = (dist, name)
    return best[1] if best else None


def _still_along_axis(
    axis: str, start: QVector3D, candidate: QVector3D, angle_deg: float
) -> bool:
    """Whether start→candidate is still within ``angle_deg`` of ``axis``.

    The STICKY release test: looser than the engage cone, so a hand drifting
    in perspective keeps the line on the axis it already took."""
    d = candidate - start
    if d.length() < 1e-6:
        return True                       # too close to tell: keep holding
    a = _AXIS_VECTORS[axis]
    return abs(QVector3D.dotProduct(d.normalized(), a)) >= math.cos(
        math.radians(angle_deg))


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


def _axis_in_plane(name: Optional[str], normal: Optional[QVector3D]
                   ) -> Optional[str]:
    """``name`` when that axis lies IN the plane with this normal, else
    ``None``. A missing or degenerate normal filters nothing."""
    if name is None or normal is None:
        return name
    n = QVector3D(normal)
    if n.length() < 1e-9:
        return name
    n = n.normalized()
    if abs(QVector3D.dotProduct(_AXIS_VECTORS[name], n)) > _IN_PLANE_TOL:
        return None
    return name


def _engaged_axis(
    start_point: QVector3D,
    candidate_world: QVector3D,
    candidate_pixel: tuple[float, float],
    world_to_pixel,
    project_onto_line,
    magnetic_axis_deg: Optional[float],
    screen_axis_px: Optional[float],
    work_plane_normal: Optional[QVector3D] = None,
    axis_in_plane: bool = False,
) -> Optional[str]:
    """Which axis the drag has taken command of, or ``None``.

    A tool asks for one or both detectors: ``magnetic_axis_deg`` for the work
    plane's own two axes (world space), ``screen_axis_px`` for the axes the
    plane CANNOT offer (issue #31/#42). The **screen answer wins where it
    exists**, because it is the gesture the hand is actually making - and
    ``_detect_axis_on_screen``'s whole reason to exist is that a camera-facing
    plane leaves X and Y unreachable in the world: the Move tool dragged
    exactly along X produced ``axis:z`` for all thirty steps of the iso test
    (measured 2026-09-18) because the world detector can only see the plane's
    normal. Preferring the screen cannot change an answer the world detector
    already gave on the Line tool's grid study (zero cells of 57k where the two
    disagreed), and where they DO disagree it says what the cursor points at.

    The world detector keeps its turn when the screen is blind: an axis
    pointing at the camera projects to a dot and is skipped there. A screen
    runaway - an edge-on axis is one pixel of mouse per metre of line, so a
    stray pixel is a point 16 km out (Marco, first live test) - is NOT an
    engagement: the point may not run further from the start than
    ``_MAX_AXIS_REACH`` times where the cursor actually is, and past that the
    call is handed to the world detector exactly as before.

    Point snaps are deliberately NOT consulted here. They sit above the axis
    in :func:`compute_snap` and still win - going to fetch a vertex with the
    mouse is the one thing the caller must not switch off.
    """
    if start_point is None or project_onto_line is None:
        return None
    if screen_axis_px is not None:
        axis = _detect_axis_on_screen(
            start_point, candidate_world, candidate_pixel,
            world_to_pixel, screen_axis_px,
        )
        if axis is not None and axis_in_plane:
            # A planar shape may not follow an axis its plane cannot hold:
            # keep the screen detector's generous pixel capture for the
            # plane's own two axes and let the world detector answer for
            # the rest (#962).
            axis = _axis_in_plane(axis, work_plane_normal)
        if axis is not None:
            reach = (candidate_world - start_point).length()
            locked = project_onto_line(start_point, _AXIS_VECTORS[axis])
            if (locked - start_point).length() <= _MAX_AXIS_REACH * reach:
                return axis
    if magnetic_axis_deg is not None:
        return _detect_axis_alignment(
            start_point, candidate_world, magnetic_axis_deg
        )
    return None


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
    if hasattr(_native.native, 'edge_direction'):
        normal = plane_normal if plane_normal is not None else _AXIS_VECTORS["z"]
        direction = _native.native.edge_direction(edge.a.toTuple(), edge.b.toTuple(),
                                                   mode == 'perpendicular', normal.toTuple())
        return QVector3D(*direction) if direction is not None else None
    direction = edge.b - edge.a
    if direction.length() < 1e-6:
        return None
    if mode == "perpendicular":
        from core.axes import AXES
        normal = plane_normal if plane_normal is not None else QVector3D(AXES["z"])
        perp = QVector3D.crossProduct(normal, direction)
        if perp.length() < 1e-9:
            return None          # the edge stands square to the plane
        return perp / perp.length()
    return direction.normalized()


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


def _on_edge_point(edge, t: float,
                   project_onto_line: Optional[ProjectOntoLine]) -> QVector3D:
    """The point of ``edge`` under the cursor, given the screen parameter
    ``t`` from :func:`_closest_on_segment_2d`. Under perspective ``t`` is
    not the world parameter: on a long edge — a construction guide is tens
    of metres once clipped to the view — lerping it put the «hovered» point
    metres away and off screen, so a lock line could never take a guide's
    height (issue #166, @pacaeiro: Shift on the blue axis, hover the guide).
    Through the cursor ray instead, as rule 7 does, when the caller has it."""
    ab = edge.b - edge.a
    if project_onto_line is not None and ab.length() > 1e-9:
        proj = project_onto_line(edge.a, ab)
        if proj is not None:
            tt = QVector3D.dotProduct(proj - edge.a, ab) / QVector3D.dotProduct(ab, ab)
            return edge.a + ab * max(0.0, min(1.0, tt))
    return edge.a + ab * t


def _edge_cursor_point(edge, cursor, world_to_pixel, threshold_px,
                       project_onto_line=None):
    """Screen distance, segment parameter and world point under the cursor.

    If one endpoint is behind the camera, recover the visible point through
    the cursor ray. Keep the original endpoints and midpoint: a clipping
    boundary is neither an endpoint target nor the middle of the model edge.
    """
    pa, pb = world_to_pixel(edge.a), world_to_pixel(edge.b)
    if pa is not None and pb is not None:
        distance, t = _closest_on_segment_2d(cursor, pa, pb)
        if distance > threshold_px:
            return None
        return distance, t, _on_edge_point(edge, t, project_onto_line)
    if project_onto_line is None or (pa is None and pb is None):
        return None
    ab = edge.b - edge.a
    length_sq = ab.lengthSquared()
    if length_sq < 1e-18:
        return None
    projected = project_onto_line(edge.a, ab)
    if projected is None:
        return None
    t = max(0.0, min(1.0, QVector3D.dotProduct(projected - edge.a, ab) / length_sq))
    point = edge.a + ab * t
    pixel = world_to_pixel(point)
    if pixel is None:
        return None
    distance = math.hypot(pixel[0] - cursor[0], pixel[1] - cursor[1])
    return (distance, t, point) if distance <= threshold_px else None


def _line_segment_intersection(
    p: QVector3D, u: QVector3D, a: QVector3D, b: QVector3D, tol: float = 1e-3
) -> Optional[QVector3D]:
    """Where the infinite line through ``p`` (unit direction ``u``) crosses the
    segment ``a``–``b``, or ``None`` if they're parallel, skew (closest approach
    > ``tol``), or the crossing falls outside the segment."""
    if hasattr(_native.native, 'lock_crossing'):
        hit = _native.native.lock_crossing(p.toTuple(), u.toTuple(), a.toTuple(), b.toTuple(), tol)
        return QVector3D(*hit) if hit is not None else None
    # Normalized directions and double-precision products avoid cancellation
    # on long edges, as in topology.segment_intersection. Keep the line
    # infinite, the segment bounded, and the caller's absolute skew tolerance.
    pc, ac = p.toTuple(), a.toTuple()
    uc = u.toTuple()
    v = tuple(bc - av for bc, av in zip(b.toTuple(), ac))
    lu, lv = math.hypot(*uc), math.hypot(*v)
    if lu < 1e-9 or lv < 1e-9:
        return None
    uc = tuple(c / lu for c in uc)
    v = tuple(c / lv for c in v)
    w = tuple(pv - av for pv, av in zip(pc, ac))

    def dot(x, y):
        return sum(a * b for a, b in zip(x, y))

    aa, bb, cc = dot(uc, uc), dot(uc, v), dot(v, v)
    dd, ee = dot(uc, w), dot(v, w)
    denom = aa * cc - bb * bb
    if denom < 1e-12:
        return None
    sc = (bb * ee - cc * dd) / denom
    tc = (aa * ee - bb * dd) / denom
    if tc < -1e-6 * lv or tc > (1.0 + 1e-6) * lv:
        return None
    on_line = tuple(pv + uv * sc for pv, uv in zip(pc, uc))
    on_seg = tuple(av + vv * tc for av, vv in zip(ac, v))
    if math.dist(on_line, on_seg) > tol:
        return None
    return QVector3D(*on_seg)


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
    if hasattr(_native.native, 'snap_circle'):
        hit = _native.native.snap_circle([p.toTuple() for p in points], tol_rel)
        return (QVector3D(*hit[0]), hit[1]) if hit is not None else None
    import numpy as np
    pts = np.array([[p.x(), p.y(), p.z()] for p in points], dtype=np.float64)
    if len(pts) < 3:
        return None
    c0 = pts.mean(axis=0)
    q = pts - c0
    # Plane basis: the two dominant directions of the point cloud.
    _w, v = np.linalg.eigh(q.T @ q)
    u, w = v[:, 2], v[:, 1]
    x, y = q @ u, q @ w
    a = np.c_[2.0 * x, 2.0 * y, np.ones(len(pts))]
    b = x * x + y * y
    try:
        sol, *_ = np.linalg.lstsq(a, b, rcond=None)
    except np.linalg.LinAlgError:
        return None
    cx, cy = sol[0], sol[1]
    r2 = sol[2] + cx * cx + cy * cy
    if r2 <= 0.0:
        return None
    r = float(math.sqrt(r2))
    dev = np.abs(np.hypot(x - cx, y - cy) - r)
    if dev.max() > max(0.002, tol_rel * r):
        return None
    c = c0 + u * cx + w * cy
    return QVector3D(float(c[0]), float(c[1]), float(c[2])), r


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
    if hasattr(_native.native, 'arc_midpoint'):
        point = _native.native.arc_midpoint([p.toTuple() for p in points], c.toTuple(), r)
        return QVector3D(*point) if point is not None else None
    m = QVector3D(0.0, 0.0, 0.0)
    for pt in points:
        m += (pt - c)
    if m.length() < 1e-9:
        return None
    return c + m.normalized() * r


def _boundary_runs(loop) -> list:
    """Split a boundary loop into runs of consecutive segments that belong
    to one circle or arc, as ``[[vertex, ...], ...]`` (each run's vertices
    in order). A drawn curve says so with its ``curve`` id; an imported
    one usually only with ``soft`` segments; a bare polyline gives itself
    away by equal segments turning by a constant angle — the shape of every
    circle and arc a CAD program ever wrote."""
    n = len(loop)
    if n < 3:
        return []
    segs = []
    for i in range(n):
        v0, v1 = loop[i], loop[(i + 1) % n]
        edge = next((e for e in getattr(v0, "edges", ()) if e.other(v0) == v1),
                    None)
        cid = getattr(edge, "curve", None) if edge is not None else None
        soft = bool(getattr(edge, "soft", False)) if edge is not None else False
        segs.append((cid, soft))

    def turn(i):
        a = loop[i - 1].position if i > 0 else loop[n - 1].position
        b = loop[i].position
        c = loop[(i + 1) % n].position
        u, w = (b - a), (c - b)
        lu, lw = u.length(), w.length()
        if lu < 1e-9 or lw < 1e-9:
            return None, lu, lw
        cosang = max(-1.0, min(1.0, QVector3D.dotProduct(u, w) / (lu * lw)))
        return math.degrees(math.acos(cosang)), lu, lw

    # One key per segment: the curve id, else "soft", else the geometric
    # signature (turn at its start vertex, rounded), else None.
    keys: list = []
    for i in range(n):
        cid, soft = segs[i]
        if cid is not None:
            keys.append(("id", cid))
        elif soft:
            keys.append(("soft",))
        else:
            t, lu, lw = turn(i)
            tn, lu2, lw2 = turn((i + 1) % n)
            same = (t is not None and tn is not None and 2.0 < t < 60.0
                    and abs(t - tn) < 1.5
                    and abs(lu2 - lw) < 0.05 * max(lw, 1e-9))
            keys.append(("geo",) if same else None)
    # Runs of equal keys; "geo" and "soft" runs may wrap around the loop.
    runs: list = []
    if all(k is not None and k == keys[0] for k in keys):
        return [(list(loop), keys[0][0] == "id")]   # the whole loop is one curve
    # rotate so the loop does not start mid-run
    start = 0
    for j in range(n):
        if keys[j] != keys[j - 1]:
            start = j
            break
    order = [(start + j) % n for j in range(n)]
    cur: list = []
    cur_key = None
    for j in order:
        k = keys[j]
        if k is not None and k == cur_key:
            cur.append(j)
        else:
            if cur_key is not None and len(cur) >= 2:
                runs.append(cur)
            cur, cur_key = ([j] if k is not None else []), k
    if cur_key is not None and len(cur) >= 2:
        runs.append(cur)
    out = []
    for r in runs:
        verts = [loop[j] for j in r] + [loop[(r[-1] + 1) % n]]
        out.append((verts, keys[r[0]][0] == "id"))
    return out


def curve_centers_of_face(face, xform=None) -> list:
    """The *Center* inference: the centre of every circle or arc on
    the face's boundary — a circle face gives one, a rounded corner one
    per corner, a circular hole its own. ``[(centre, radius, key)]`` in
    world space (``xform`` places a component's face). ``key`` tells the
    arcs of one face apart.

    Drawn curves carry a ``curve`` id; imported ones usually only ``soft``
    segments; and a plain polyline is read by its shape (see
    :func:`_boundary_runs`) — the plaza's circle came from a file with
    neither."""
    out: list = []
    for li, loop in enumerate([face.loop]
                              + list(getattr(face, "hole_loops", []) or [])):
        for ri, (verts, drawn) in enumerate(_boundary_runs(loop)):
            # Three points always sit on SOME circle, and four often do
            # (every isosceles trapezoid of a revolved surface is cyclic):
            # a run read from its shape or its soft flags needs five to
            # mean anything. A drawn curve says what it is: three suffice.
            if len(verts) < (3 if drawn else 5):
                continue
            pts = [v.position for v in verts]
            if xform is not None:
                pts = [xform.map(QVector3D(p)) for p in pts]
            fit = fit_circle(pts)
            if fit is not None:
                out.append((fit[0], fit[1], (li, ri)))
    return out


def _inside_2d(p, poly) -> bool:
    """Even-odd ray cast: whether ``p`` lies inside ``poly`` [(x, y)]."""
    x, y = p
    inside = False
    j = len(poly) - 1
    for i in range(len(poly)):
        xi, yi = poly[i]
        xj, yj = poly[j]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
            inside = not inside
        j = i
    return inside


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
    if hasattr(_native.native, 'face_center'):
        center = _native.native.face_center([[p.toTuple() for p in ring] for ring in loops])
        return QVector3D(*center) if center is not None else None
    outer = loops[0]
    if len(outer) < 3:
        return None
    # Newell's normal of the outer loop, in doubles.
    nx = ny = nz = 0.0
    for i in range(len(outer)):
        a, b = outer[i], outer[(i + 1) % len(outer)]
        nx += (a.y() - b.y()) * (a.z() + b.z())
        ny += (a.z() - b.z()) * (a.x() + b.x())
        nz += (a.x() - b.x()) * (a.y() + b.y())
    nlen = math.sqrt(nx * nx + ny * ny + nz * nz)
    if nlen < 1e-12:
        return None
    n = (nx / nlen, ny / nlen, nz / nlen)
    # An orthonormal basis (u, v) of the face's plane, origin at a corner.
    seed = (1.0, 0.0, 0.0) if abs(n[0]) < 0.9 else (0.0, 1.0, 0.0)
    u = (n[1] * seed[2] - n[2] * seed[1],
         n[2] * seed[0] - n[0] * seed[2],
         n[0] * seed[1] - n[1] * seed[0])
    ul = math.sqrt(u[0] ** 2 + u[1] ** 2 + u[2] ** 2)
    u = (u[0] / ul, u[1] / ul, u[2] / ul)
    v = (n[1] * u[2] - n[2] * u[1],
         n[2] * u[0] - n[0] * u[2],
         n[0] * u[1] - n[1] * u[0])
    o = outer[0]
    ox, oy, oz = o.x(), o.y(), o.z()

    def flat(loop):
        out = []
        for p in loop:
            dx, dy, dz = p.x() - ox, p.y() - oy, p.z() - oz
            out.append((dx * u[0] + dy * u[1] + dz * u[2],
                        dx * v[0] + dy * v[1] + dz * v[2]))
        return out

    flats = [flat(loop) for loop in loops if len(loop) >= 3]
    area = sx = sy = 0.0
    for k, poly in enumerate(flats):
        a = cx = cy = 0.0
        for i in range(len(poly)):
            x0, y0 = poly[i]
            x1, y1 = poly[(i + 1) % len(poly)]
            cr = x0 * y1 - x1 * y0
            a += cr
            cx += (x0 + x1) * cr
            cy += (y0 + y1) * cr
        if abs(a) < 1e-18:
            continue
        # Each loop's area-weighted centre, whatever its winding: the
        # outer loop adds, the holes take away.
        sign = 1.0 if k == 0 else -1.0
        w = abs(a) * 0.5 * sign
        area += w
        sx += w * cx / (3.0 * a)
        sy += w * cy / (3.0 * a)
    if area <= 1e-12:
        return None
    c = (sx / area, sy / area)
    if not _inside_2d(c, flats[0]):
        return None
    return QVector3D(ox + c[0] * u[0] + c[1] * v[0],
                     oy + c[0] * u[1] + c[1] * v[1],
                     oz + c[0] * u[2] + c[1] * v[2])


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
    pts = [p for p in refs if p is not None]
    if len(pts) < 2:
        return None
    best = None
    axes = list(_AXIS_VECTORS.items())
    if hasattr(_native.native, 'reference_crossings'):
        hits = _native.native.reference_crossings([p.toTuple() for p in pts], _axis_tuples())
        for point, i, k, j, l in hits:
            cross = QVector3D(*point)
            pixel = world_to_pixel(cross)
            if pixel is None:
                continue
            d = math.hypot(pixel[0]-cx, pixel[1]-cy)
            if d <= threshold_px and (best is None or d < best[0]):
                if is_occluded is None or not is_occluded(cross):
                    best = (d, cross, pts[i], "xyz"[k], pts[j], "xyz"[l])
    else:
        for i in range(len(pts)):
            for j in range(i + 1, len(pts)):
                p, q = pts[i], pts[j]
                for ka, a in axes:
                    for kb, b in axes:
                        if ka == kb:
                            continue
                        # Closest points of the two lines p + a·s and q + b·t.
                        w = p - q
                        ab = QVector3D.dotProduct(a, b)
                        denom = 1.0 - ab * ab
                        if denom < 1e-9:
                            continue
                        wa = QVector3D.dotProduct(w, a)
                        wb = QVector3D.dotProduct(w, b)
                        s_ = (ab * wb - wa) / denom
                        t_ = (wb - ab * wa) / denom
                        pa = p + a * s_
                        pb = q + b * t_
                        if (pa - pb).length() > 1e-3:
                            continue                    # skew: no crossing
                        if abs(s_) < 1e-3 or abs(t_) < 1e-3:
                            continue                    # on a point itself
                        px = world_to_pixel(pa)
                        if px is None:
                            continue
                        d = math.hypot(px[0] - cx, px[1] - cy)
                        if d > threshold_px:
                            continue
                        if is_occluded is not None and is_occluded(pa):
                            continue
                        if best is None or d < best[0]:
                            best = (d, pa, p, ka, q, kb)
    if best is None:
        return None
    _, cross, p, ka, q, kb = best
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
    if (free_3d and project_onto_line is not None) or hasattr(_native.native, 'reference_feet'):
        if free_3d and project_onto_line is not None:
            feet = ((axis, project_onto_line(ref, a)) for axis, a in _AXIS_VECTORS.items())
        else:
            feet = (("xyz"[i], QVector3D(*p)) for i, p in _native.native.reference_feet(
                ref.toTuple(), candidate.toTuple(), _axis_tuples(),
                plane_normal.toTuple() if plane_normal is not None else None))
        best = None
        for axis, foot in feet:
            fp = world_to_pixel(foot)
            if fp is None:
                continue
            d = math.hypot(fp[0] - cx, fp[1] - cy)
            if d > threshold_px:
                continue
            if is_occluded is not None and is_occluded(foot):
                continue
            if best is None or d < best[0]:
                best = (d, foot, axis)
        if best is None:
            return None
        _, foot, axis = best
        return SnapResult(foot, "from_point", COLOR_ENDPOINT,
                          guide=(QVector3D(ref), foot),
                          guide_color=AXIS_COLORS[axis], axis=axis)
    best = None
    n = None
    if plane_normal is not None and plane_normal.length() > 1e-9:
        n = plane_normal.normalized()
    for axis, a in _AXIS_VECTORS.items():
        an = QVector3D.dotProduct(a, n) if n is not None else 0.0
        if abs(an) < 1e-6:
            # The axis line runs parallel to the plane (or there is none):
            # its foot — which lies in the plane exactly when the line does.
            s = QVector3D.dotProduct(candidate - ref, a)
            if abs(s) < 1e-3:
                continue                    # that is ``ref`` itself
            if n is not None and abs(QVector3D.dotProduct(ref - candidate, n)) > 1e-4:
                continue                    # …and here it does not
            foot = ref + a * s
        else:
            # The line pierces the plane: the one point of it on the face.
            t = QVector3D.dotProduct(candidate - ref, n) / an
            if abs(t) < 1e-3:
                continue                    # that is ``ref`` itself
            foot = ref + a * t
        fp = world_to_pixel(foot)
        if fp is None:
            continue
        d = math.hypot(fp[0] - cx, fp[1] - cy)
        if d > threshold_px:
            continue
        if is_occluded is not None and is_occluded(foot):
            continue
        if best is None or d < best[0]:
            best = (d, foot, axis)
    if best is None:
        return None
    _, foot, axis = best
    return SnapResult(foot, "from_point", COLOR_ENDPOINT,
                      guide=(QVector3D(ref), foot),
                      guide_color=AXIS_COLORS[axis], axis=axis)


def _edge_from_point_crossing(
    edge, ref, cx, cy, world_to_pixel, threshold_px, is_occluded=None,
) -> Optional[SnapResult]:
    """Where an axis line through ``ref`` (the acquired point) crosses
    ``edge``, when that crossing is within the snap radius of the cursor —
    a green 'from point' on the edge with the axis-coloured guide back to
    ``ref``. ``None`` when no axis line meets the edge near the cursor."""
    best = None
    for axis, a in _AXIS_VECTORS.items():
        hit = _line_segment_intersection(ref, a, edge.a, edge.b)
        if hit is None or (hit - ref).length() < 1e-4:
            continue
        hp = world_to_pixel(hit)
        if hp is None:
            continue
        d = math.hypot(hp[0] - cx, hp[1] - cy)
        if d > threshold_px:
            continue
        if is_occluded is not None and is_occluded(hit):
            continue
        if best is None or d < best[0]:
            best = (d, hit, axis)
    if best is None:
        return None
    _, hit, axis = best
    return SnapResult(hit, "from_point", COLOR_ENDPOINT,
                      guide=(QVector3D(ref), hit),
                      guide_color=AXIS_COLORS[axis],
                      context=getattr(edge, "context", None))


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
    if plane_normal is None or plane_normal.length() < 1e-9:
        return None
    n = plane_normal.normalized()
    best = None
    def feet():
        if hasattr(_native.native, 'plane_alignments'):
            for k, foot, d in _native.native.plane_alignments(
                    ref.toTuple(), candidate.toTuple(), plane_normal.toTuple(), _axis_tuples()):
                yield "xyz"[k], QVector3D(*foot), QVector3D(*d)
            return
        for axis, a in _AXIS_VECTORS.items():
            d = QVector3D.crossProduct(a, n)
            if d.length() < 1e-5:     # normalized() gives the NULL vector below this
                continue      # the axis plane is parallel to the face: no line
            d = d.normalized()
            # In the face, square to the line: the way the cursor has to move
            # to reach the plane through ``ref``.
            m = QVector3D.crossProduct(n, d)
            ma = QVector3D.dotProduct(m, a)                       # = |a × n|, > 0
            s = QVector3D.dotProduct(candidate - ref, a)
            foot = candidate - m * (s / ma)
            yield axis, foot, d

    for axis, foot, d in feet():
        fp = world_to_pixel(foot)
        if fp is None:
            continue
        dist = math.hypot(fp[0] - cx, fp[1] - cy)
        if dist > threshold_px:
            continue
        if is_occluded is not None and is_occluded(foot):
            continue
        if best is None or dist < best[0]:
            best = (dist, foot, axis, d)
    if best is None:
        return None
    _, foot, axis, d = best
    # The reference's own projection onto the line: the guide's elbow.
    elbow = foot + d * QVector3D.dotProduct(ref - foot, d)

    def _colour_along(v):
        if v.length() < 1e-6:
            return COLOR_EXTENSION
        u = v.normalized()
        k = max(_AXIS_VECTORS,
                key=lambda k: abs(QVector3D.dotProduct(u, _AXIS_VECTORS[k])))
        return AXIS_COLORS[k]

    guides = []
    if (elbow - QVector3D(ref)).length() > 1e-4:
        guides.append((QVector3D(ref), elbow, _colour_along(elbow - ref)))
    return SnapResult(foot, "aligned", COLOR_ENDPOINT, axis=axis,
                      guide=(elbow, foot), guide_color=_colour_along(foot - elbow),
                      guides=guides or None)


def _extension_snap(
    candidate_world, cx, cy, scene, world_to_pixel, et, start_point, is_occluded,
    project_onto_line=None, plane_normal=None,
) -> Optional[SnapResult]:
    """Extension / intersection inference: when the draw direction is collinear
    with an edge and the cursor is on that edge's *continuation* (beyond its
    ends), snap along it — and onto where it crosses another edge (a green
    connection point), so a line extends exactly onto a perpendicular one.

    With ``plane_normal`` — the cursor is over a face — the extension is CUT
    by that face: the point offered is where the edge's line crosses the
    plane, and a crossing that lands far from the cursor is no offer at all.
    An edge already lying IN the plane cuts nothing (its line is parallel to
    it) and keeps the plain projection, which is in the plane too.

    ``None`` unless the draw is collinear with an edge near the cursor, which is
    what keeps every edge's infinite line from becoming snap noise."""
    if start_point is None:
        return None
    if plane_normal is not None:
        n = QVector3D(plane_normal)
        plane_normal = n.normalized() if n.length() > 1e-9 else None
    draw = candidate_world - start_point
    if draw.length() < 1e-6:
        return None
    draw = draw.normalized()
    # The draw direction above comes from where the cursor ray meets the
    # scene. Over empty sky that is on the draw; over a face it is a point
    # on the wall BEHIND, and the direction is nonsense — the extension of
    # a sloped roof edge showed only with nothing behind it («a veces te
    # bloquea y a veces no, depende de cómo te orientes», Rafael, revision
    # 4, 03:56). So when the 3D test fails the same question is asked on
    # screen, where the cursor really is, and the point is taken on the
    # edge's line under the cursor ray instead of on that far wall.
    sp = world_to_pixel(start_point) if project_onto_line is not None else None
    sdx = sdy = 0.0
    if sp is not None:
        sdx, sdy = cx - sp[0], cy - sp[1]
    sdl = math.hypot(sdx, sdy)
    best_ext = None  # (dist, proj, from_end, edge, dir)
    for edge in scene.edges:
        # One chord of a circle, arc or smoothed surface has no line worth
        # extending — a sphere's soft edges and the next circle's segments
        # threw dashed guides across the model (issue #140) — and a hidden
        # edge is not there to be followed.
        if (getattr(edge, "soft", False) or getattr(edge, "hidden", False)
                or getattr(edge, "curve", None) is not None):
            continue
        ab = edge.b - edge.a
        if ab.length() < 1e-9:
            continue
        u = ab.normalized()
        at = candidate_world
        if abs(QVector3D.dotProduct(draw, u)) < 0.966:  # ~15°: drawing along it
            # The on-screen reading is only for EXTENDING this edge from its
            # own line (the start sits on it, as Rafael's did at the eave):
            # a merely parallel edge, or one that runs toward the camera and
            # shows as a stub, has no trustworthy screen direction — letting
            # those in handed a midpoint over to an «extension» 5.8 m deep
            # (snap matrix, front camera).
            if sdl < 4.0:
                continue
            off = start_point - edge.a
            if (off - u * QVector3D.dotProduct(off, u)).length() > 1e-3:
                continue
            pa, pb = world_to_pixel(edge.a), world_to_pixel(edge.b)
            if pa is None or pb is None:
                continue
            edx, edy = pb[0] - pa[0], pb[1] - pa[1]
            edl = math.hypot(edx, edy)
            if edl < 20.0 or abs(sdx * edx + sdy * edy) < 0.966 * sdl * edl:
                continue
            at = project_onto_line(edge.a, u)
        t = QVector3D.dotProduct(at - edge.a, u)
        if plane_normal is not None:
            # The far end of the extension is where the edge's line meets
            # the plane the cursor is on, not where the cursor ray happens
            # to fall on the edge's line (#545). ``candidate_world`` is the
            # cursor's own point on that plane, so it gives the offset.
            denom = QVector3D.dotProduct(plane_normal, u)
            if abs(denom) > 1e-6:
                t = (QVector3D.dotProduct(plane_normal,
                                          candidate_world - edge.a) / denom)
        if -1e-6 <= t <= ab.length() + 1e-6:
            continue  # on the segment itself
        proj = edge.a + u * t
        pp = world_to_pixel(proj)
        if pp is None:
            continue
        d = math.hypot(pp[0] - cx, pp[1] - cy)
        if d > et:
            continue
        if is_occluded is not None and is_occluded(proj):
            continue
        if best_ext is None or d < best_ext[0]:
            from_end = edge.a if t < 0 else edge.b
            best_ext = (d, proj, from_end, edge, u)
    if best_ext is None:
        return None
    _, proj, from_end, src, u = best_ext
    best_hit = None
    for other in scene.edges:
        if other == src:
            continue
        hit = _line_segment_intersection(proj, u, other.a, other.b)
        if hit is None:
            continue
        hp = world_to_pixel(hit)
        if hp is None:
            continue
        dh = math.hypot(hp[0] - cx, hp[1] - cy)
        if dh > et:
            continue
        if is_occluded is not None and is_occluded(hit):
            continue
        if best_hit is None or dh < best_hit[0]:
            best_hit = (dh, hit)
    if best_hit is not None:
        if (best_hit[1] - from_end).lengthSquared() < 1e-12:
            # The extension «crosses» its neighbour at its own end: that
            # point is the Endpoint, not an Intersection (#357).
            return SnapResult(best_hit[1], "endpoint", COLOR_ENDPOINT)
        return SnapResult(best_hit[1], "intersection", COLOR_ENDPOINT,
                          guide=(from_end, best_hit[1]))
    return SnapResult(proj, "extension", COLOR_EXTENSION, guide=(from_end, proj))


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
    if start_point is None or draw_dir.length() < 1e-6:
        return None
    if line_dir is not None:
        if line_dir.length() < 1e-6:
            return None
        adir = QVector3D(line_dir).normalized()
    else:
        u = draw_dir.normalized()
        # The draw must be along an axis; orient that axis along the draw direction.
        adir = None
        cos_axis = math.cos(math.radians(axis_deg))
        for a in _AXIS_VECTORS.values():
            dp = QVector3D.dotProduct(u, a)
            if abs(dp) >= cos_axis:
                adir = a if dp > 0 else -a
                break
        if adir is None:
            return None  # diagonal draw — no clean 'from point'

    def hovered_edge_point():
        best_edge = None
        for edge in scene.edges:
            pa, pb = world_to_pixel(edge.a), world_to_pixel(edge.b)
            if pa is None or pb is None:
                continue
            d, t = _closest_on_segment_2d((cx, cy), pa, pb)
            if d <= threshold_px and (best_edge is None or d < best_edge[0]):
                point = _on_edge_point(edge, t, project_onto_line)
                # Only an edge you can see. An edge the model hides cannot
                # be the one the cursor is on: its screen trace crossing
                # the cursor is a coincidence of projection. Dragging up
                # with Z locked from a box's top corner, the FAR vertical
                # edge projects onto the cursor's screen line and lent the
                # lock its lower height (upstream #1085). In X-ray nothing
                # hides anything (no ``is_occluded``), so it still counts.
                if is_occluded is not None and is_occluded(point):
                    continue
                best_edge = (d, point)
        return best_edge[1] if best_edge is not None else None

    if query is not None:
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
        refs = [(QVector3D(*body[0]), "from_point", COLOR_ENDPOINT)] if body is not None else []
    else:
        refs = []
        if extra_point is not None:
            refs.append((extra_point, "from_point", COLOR_ENDPOINT))
        for edge in scene.edges:
            if getattr(edge, "figure", False):
                # A face-me figure is scenery, not drawing: its feet snap when you
                # point at them, but no line of the drawing lines up with it
                # (Rafael, revision 4, 04:08: the scale figure pulled «from
                # point» guides metres away from the roof he was drawing).
                continue
            refs.append((edge.a, "from_point", COLOR_ENDPOINT))
            refs.append((edge.b, "from_point", COLOR_ENDPOINT))
            refs.append(((edge.a + edge.b) * 0.5, "midpoint", COLOR_MIDPOINT))
        # The world origin is a reference like any corner. It is a named snap of
        # its own (rule 6) but lives in no edge, so under an axis lock — which
        # returns from this function and never reaches rule 6 — it existed only
        # when a piece of geometry happened to touch it. Measured with the red
        # axis locked and the cursor four pixels off each point: a corner gave
        # an exact foot, the origin gave the cursor's own projection, i.e.
        # nothing (issue #27, @pacaeiro).
        refs.append((QVector3D(0.0, 0.0, 0.0), "origin", COLOR_ORIGIN))

        if hovered_refs and edge_body:
            # The point of an edge under the cursor is a reference too (the
            # window's sill, not just its corners): the closest point of the
            # nearest edge, as a plain 'from point'. Corners and midpoints come
            # first — a sub-pixel miss on a corner must not hand the snap to
            # the edge's body a few millimetres away — so the edge point only
            # competes when no point reference is within reach.
            body = hovered_edge_point()
            if body is not None:
                refs.append((body, "from_point", COLOR_ENDPOINT, True))
    best = None  # (dist, foot, ref, kind, color)
    for entry in refs:
        ref, kind, color = entry[0], entry[1], entry[2]
        if len(entry) > 3 and best is not None:
            continue                      # the edge body yields to any point
        s = QVector3D.dotProduct(ref - start_point, adir)
        if not hovered_refs and s <= 1e-6:
            continue  # at or behind the start along the draw
        # Under a lock (``hovered_refs``) the line runs BOTH ways and the
        # reference itself is a target, so no side is "behind" — and a
        # reference level with the start (s = 0) is worth showing: its foot
        # is the start, the guide says «level with it», and a click there
        # draws nothing. The side used to be decided by where the cursor
        # RAY met the lock line, which with the cursor ON a reference far
        # from the line is anywhere: measured with six dashes and five
        # cameras, each camera lost a different dash (issue #34,
        # @pacaeiro: «not all of them are detected… Origin not always
        # detected… if the position of the Origin is negative»).
        foot = start_point + adir * s
        if (foot - ref).length() < 1e-4:
            continue  # ref already on the draw line (collinear, not a crossing)
        qp = world_to_pixel(foot)
        if qp is None:
            continue
        d = math.hypot(qp[0] - cx, qp[1] - cy)
        if hovered_refs:
            rp = world_to_pixel(ref)
            if rp is not None:
                d_ref = math.hypot(rp[0] - cx, rp[1] - cy)
                # Pointing AT a reference counts only when it can be seen:
                # with Z locked from a box's top corner in an Iso view, the
                # hidden back edge's midpoint sat right above the corner on
                # screen, and the point went DOWN to its height while the
                # cursor went up (upstream #1085). Its foot on the lock line
                # still counts the usual way.
                if d_ref < d and not (is_occluded is not None
                                      and is_occluded(ref)):
                    d = d_ref
        if d > threshold_px:
            continue
        if is_occluded is not None and is_occluded(foot):
            continue
        if best is None or d < best[0]:
            best = (d, foot, ref, kind, color)
    if best is None:
        return None
    _, foot, ref, kind, color = best
    return _from_point_result(foot, ref, kind, color)


def _from_point_result(foot, ref, kind, color):
    # The guide runs perpendicular to the draw, from the corner to the foot —
    # coloured by its axis when it RUNS along one (red/green/blue), grey
    # when it does not: a guide across the axes painted blue read as «along
    # blue» (#711).
    guide_color = None
    if kind == "from_point":
        gdn = (foot - ref).normalized()
        gaxis = max(_AXIS_VECTORS,
                    key=lambda k: abs(QVector3D.dotProduct(gdn, _AXIS_VECTORS[k])))
        along = abs(QVector3D.dotProduct(gdn, _AXIS_VECTORS[gaxis]))
        guide_color = AXIS_COLORS[gaxis] if along > 0.999 else COLOR_EXTENSION
    return SnapResult(foot, kind, color, guide=(ref, foot),
                      guide_color=guide_color)


def _through_point_snap(
    start_point, through_point, draw_dir, project_onto_line, inference_deg,
) -> Optional[SnapResult]:
    """'Through point' inference ("A través del punto"): when the draw heads
    along the line from the start *through* an encouraged point, lock onto that
    line so the segment passes exactly through it. A magenta directional lock,
    like parallel/perpendicular but toward an arbitrary point."""
    if (
        start_point is None or through_point is None
        or project_onto_line is None or draw_dir.length() < 1e-6
    ):
        return None
    dirv = through_point - start_point
    if dirv.length() < 1e-6:
        return None
    dir_u = dirv.normalized()
    if abs(QVector3D.dotProduct(draw_dir.normalized(), dir_u)) < \
            math.cos(math.radians(inference_deg)):
        return None
    locked = project_onto_line(start_point, dir_u)
    return SnapResult(locked, "through_point", COLOR_REFERENCE,
                      guide=(through_point, locked))


def _perpendicular_face_snap(
    start_point, face_normal, draw_dir, project_onto_line, inference_deg,
) -> Optional[SnapResult]:
    """'Perpendicular to face' inference ("Perpendicular a la cara"): when the
    draw runs along an encouraged face's normal, lock to it so the line leaves
    the face square-on. Magenta directional lock."""
    if (
        start_point is None or face_normal is None
        or project_onto_line is None or draw_dir.length() < 1e-6
        or face_normal.length() < 1e-6
    ):
        return None
    n_u = face_normal.normalized()
    if abs(QVector3D.dotProduct(draw_dir.normalized(), n_u)) < \
            math.cos(math.radians(inference_deg)):
        return None
    locked = project_onto_line(start_point, n_u)
    return SnapResult(locked, "perp_face", COLOR_REFERENCE)


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

    # Local import: keep the snap engine free of a module-load dependency on
    # the topology package (and any import cycle it might grow into).
    from core.topology import segment_intersection

    # Only edges whose screen span passes under the cursor can contribute:
    # that keeps the pairwise test tiny and stops a far-away crossing (whose
    # pixel happens to land near the cursor by coincidence of depth) from
    # firing.
    near_edges = []
    for edge in scene.edges:
        if getattr(edge, "center", False):
            continue  # the degenerate pseudo-edge that carries a circle centre
        ab = edge.b - edge.a
        if QVector3D.dotProduct(ab, ab) < 1e-12:
            continue
        pa = world_to_pixel(edge.a)
        pb = world_to_pixel(edge.b)
        if pa is None or pb is None:
            continue
        d, _t = _closest_on_segment_2d((cx, cy), pa, pb)
        if d <= et:
            near_edges.append(edge)
    # Guide circles whose rim passes under the cursor.
    near_circles = []
    if candidate_world is not None:
        for circ in getattr(scene, "circles", ()):
            d, _rim = _closest_on_circle(
                circ, candidate_world, cx, cy, world_to_pixel, et)
            if d is not None:
                near_circles.append(circ)
    if not near_circles and len(near_edges) < 2:
        return None

    # The crossings under the cursor, in pair order; the nearest one in
    # sight wins (the first of equals). Occlusion is asked once a point,
    # nearest first, in batches (_first_visible): under the plant of #158
    # a cursor on a hidden corner had 56 crossings, each one a ray.
    found = []

    def _offer(hit, o1, o2):
        hp = world_to_pixel(hit)
        if hp is None:
            return
        dh = math.hypot(hp[0] - cx, hp[1] - cy)
        if dh <= et:
            found.append((dh, hit, o1, o2))

    for i, j in _pairs_that_may_meet(near_edges):
        e1, e2 = near_edges[i], near_edges[j]
        hit = segment_intersection(e1.a, e1.b, e2.a, e2.b)
        if hit is not None:
            _offer(hit, e1, e2)
    for circ in near_circles:
        for e in near_edges:
            for p in _segment_circle_intersections(
                    e.a, e.b, circ.center, circ.radius, circ.normal):
                _offer(p, e, circ)
    for i in range(len(near_circles)):
        for j in range(i + 1, len(near_circles)):
            c1, c2 = near_circles[i], near_circles[j]
            for p in _circle_circle_intersections(
                    c1.center, c1.radius, c1.normal,
                    c2.center, c2.radius, c2.normal):
                _offer(p, c1, c2)
    if not found:
        return None
    found.sort(key=lambda f: f[0])
    first = {}
    for k, f in enumerate(found):
        first.setdefault((f[1].x(), f[1].y(), f[1].z()), k)
    order = sorted(first.values())
    k = _first_visible([found[i][1] for i in order], is_occluded,
                       are_occluded)
    if k is None:
        return None
    best = found[order[k]]
    hit = best[1]
    corners = [c for c in (getattr(best[2], "a", None),
                           getattr(best[2], "b", None),
                           getattr(best[3], "a", None),
                           getattr(best[3], "b", None))
               if c is not None]
    if any((hit - c).lengthSquared() < 1e-12 for c in corners):
        # Two edges meeting at a shared corner «intersect» there, and
        # between the endpoint rule's radius and this one the corner came
        # back labelled Intersection. Same point, so it keeps the snap —
        # under its true name (#357, @kellytait).
        return SnapResult(hit, "endpoint", COLOR_ENDPOINT)
    return SnapResult(hit, "intersection", COLOR_ENDPOINT)


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

    if query is not None:
        for coords in query.plane_crossings(et, origin.toTuple(), n.toTuple()):
            hit = QVector3D(*coords)
            if inside(hit) and (is_occluded is None or not is_occluded(hit)):
                return SnapResult(hit, "intersection", COLOR_ENDPOINT, context=context)
        return None

    found = []
    for edge in scene.edges:
        if getattr(edge, "center", False):
            continue
        ab = edge.b - edge.a
        if QVector3D.dotProduct(ab, ab) < 1e-12:
            continue
        den = QVector3D.dotProduct(n, ab)
        if abs(den) < 1e-9 * ab.length():
            continue                      # parallel / in the plane
        t = QVector3D.dotProduct(n, origin - edge.a) / den
        # Strictly inside the span: an edge that merely ENDS on the face (a
        # wall rising from a slab) has an endpoint there, with its own radius.
        margin = 1e-4 / ab.length()
        if t <= margin or t >= 1.0 - margin:
            continue                      # stops short of the plane, or on it
        hit = edge.a + ab * t
        hp = world_to_pixel(hit)
        if hp is None:
            continue
        dh = math.hypot(hp[0] - cx, hp[1] - cy)
        if dh > et:
            continue
        found.append((dh, hit))
    # Nearest first; the membership test (a ray for a SurfacePatch) and the
    # occlusion are asked only of the candidates, in that order.
    found.sort(key=lambda f: f[0])
    for _dh, hit in found:
        if not inside(hit):
            continue
        if is_occluded is None or not is_occluded(hit):
            return SnapResult(hit, "intersection", COLOR_ENDPOINT,
                              context=context)
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
    if hasattr(_native.native, 'plane_cut_ray'):
        hit = _native.native.plane_cut_ray(origin.toTuple(), direction.toTuple(),
                  (p_f.toTuple(), n_f.toTuple()), (p_g.toTuple(), n_g.toTuple()))
        if hit is None:
            return None
        x, sample = (QVector3D(*p) for p in hit)
    else:
        u = QVector3D.crossProduct(n_f, n_g)
        if u.length() < 0.02:                   # (near) coplanar: no crossing
            return None
        o, d = origin, direction
        # The crossing line: the point of both planes nearest the origin…
        d_f = QVector3D.dotProduct(n_f, p_f)
        d_g = QVector3D.dotProduct(n_g, p_g)
        uu = QVector3D.dotProduct(u, u)
        l0 = (QVector3D.crossProduct(n_g, u) * d_f
              + QVector3D.crossProduct(u, n_f) * d_g) / uu
        # …then its point nearest the cursor ray.
        un = u / math.sqrt(uu)
        w0 = l0 - o
        b = QVector3D.dotProduct(un, d)
        den = 1.0 - b * b
        if den < 1e-9:                          # the line runs along the ray
            return None
        s = (b * QVector3D.dotProduct(w0, d) - QVector3D.dotProduct(w0, un)) / den
        x = l0 + un * s
        sample = x + un * max(1e-3, (x - o).length() * 1e-2)
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
        if hasattr(_native.native, 'lock_on_extension'):
            pair = _native.native.lock_on_extension(start_point.toTuple(), line_dir.toTuple(),
                                                      a.toTuple(), b.toTuple())
            if pair is not None:
                hit, end = (QVector3D(*p) for p in pair)
                if world_to_pixel(hit) is not None:
                    return SnapResult(hit, "intersection_edge", COLOR_LOCK_INTERSECT,
                                      guide=(end, hit), guide_color=COLOR_EXTENSION)
        else:
            v = b - a
            if v.lengthSquared() > 1e-12:
                w0 = start_point - a
                bb = QVector3D.dotProduct(line_dir, v)
                cc = QVector3D.dotProduct(v, v)
                dd = QVector3D.dotProduct(line_dir, w0)
                ee = QVector3D.dotProduct(v, w0)
                uu = QVector3D.dotProduct(line_dir, line_dir)
                den = cc * uu - bb * bb
                if abs(den) > 1e-9 * cc * uu:
                    s = (bb * ee - cc * dd) / den
                    t = (ee * uu - bb * dd) / den
                    on_line = start_point + line_dir * s
                    on_edge = a + v * t
                    if ((on_line - on_edge).length() <= 1e-3
                            and (on_edge - start_point).length() > 1e-6
                            and world_to_pixel(on_edge) is not None):
                        # Beyond an end of the edge that stretch is the dotted
                        # extension; inside it, the crossing rule answered.
                        near_end = a if t < 0.5 else b
                        return SnapResult(on_edge, "intersection_edge",
                                          COLOR_LOCK_INTERSECT,
                                          guide=(near_end, on_edge),
                                          guide_color=COLOR_EXTENSION)
    if hover_plane is not None:
        p0, n, ref = hover_plane
        den = QVector3D.dotProduct(n, line_dir)
        if abs(den) > 1e-6:
            s = QVector3D.dotProduct(n, p0 - start_point) / den
            hit = start_point + line_dir * s
            # A plane through the start meets the lock line AT the start:
            # nothing to offer, the line leaves the plane there. Offering
            # it pinned a Z lock from a box's top corner to that corner
            # while the cursor, over the top face, went up (upstream #1085,
            # seen on the Metal view, whose hover edge is visible-only).
            if abs(s) > 1e-6 and world_to_pixel(hit) is not None:
                return SnapResult(hit, "intersection_face",
                                  COLOR_LOCK_INTERSECT, guide=(ref, hit),
                                  guide_color=COLOR_EXTENSION)
    return None


def _point_snap_candidates(edges, named=()):
    """Named points, world origin and endpoints, with their original context.

    The order is the precedence on a tie: the pseudo-edges' named points (a
    circle's centre, a component's origin, an arc's midpoint), then the
    caller's extra ``named`` points (a guide circle's quadrants), then the
    world origin, then the plain edges' endpoints. A construction guide's
    ends are clipped to the view: not endpoints."""
    plain = []
    for edge in edges:
        context = getattr(edge, "context", None)
        if getattr(edge, "center", False):
            yield edge.a, "center", COLOR_ENDPOINT, context
        elif getattr(edge, "component_origin", False):
            yield edge.a, "component_origin", COLOR_ORIGIN, context
        elif getattr(edge, "arc_midpoint", False):
            yield edge.a, "arc_midpoint", COLOR_MIDPOINT, context
        elif not getattr(edge, "guide", False):
            plain.append(edge)
    for point in named:
        yield point, "quadrant", COLOR_MIDPOINT, None
    yield QVector3D(0.0, 0.0, 0.0), "origin", COLOR_ORIGIN, None
    for edge in plain:
        context = getattr(edge, "context", None)
        yield edge.a, "endpoint", COLOR_ENDPOINT, context
        yield edge.b, "endpoint", COLOR_ENDPOINT, context


def _midpoint_snap_candidates(edges):
    """Actual edge midpoints; clipped guides and pseudo-points have none."""
    for edge in edges:
        if not getattr(edge, "guide", False) and (edge.b - edge.a).length() >= 1e-9:
            yield ((edge.a + edge.b) * 0.5, "midpoint", COLOR_MIDPOINT,
                   getattr(edge, "context", None))


def _resolve_point_candidates(candidates, is_occluded, are_occluded=None):
    """Nearest visible point, preferring the drawing context in a near-tie.

    Candidates are (distance, point, kind, color, context, occludable).
    Anchor the tie window to the nearest VISIBLE candidate, not absolute
    half-pixel buckets or a hidden point. Original order breaks remaining
    ties so named points keep precedence. Cache visibility across both passes.
    """
    if not candidates:
        return None
    ordered = sorted(enumerate(candidates), key=lambda c: c[1][0])
    hidden = {}

    def key(candidate):
        return candidate[1].toTuple()

    def one(candidate):
        if not candidate[5] or is_occluded is None:
            return False
        point_key = key(candidate)
        if point_key not in hidden:
            hidden[point_key] = is_occluded(candidate[1])
        return hidden[point_key]

    def many(batch):
        unseen = {}
        for candidate in batch:
            if candidate[5] and key(candidate) not in hidden:
                unseen.setdefault(key(candidate), candidate[1])
        if unseen:
            hidden.update(zip(unseen, are_occluded(list(unseen.values()))))
        return [one(candidate) for candidate in batch]

    nearest = _first_visible([c for _i, c in ordered],
                             one if is_occluded is not None else None,
                             many if are_occluded is not None else None)
    if nearest is None:
        return None
    limit = ordered[nearest][1][0] + _TIE_PX
    ties = [(i, c) for i, c in ordered[nearest:] if c[0] < limit]
    ties.sort(key=lambda c: (c[1][4] is not None, c[0]))
    for _i, candidate in ties:
        if not one(candidate):
            return candidate
    return None


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

    def point_on_lock(points):
        candidates = []
        for point, kind, color, context in points:
            if not _vertex_on_line(point, start_point, line_dir):
                continue
            pixel = world_to_pixel(point)
            if pixel is None:
                continue
            distance = math.hypot(pixel[0] - cx, pixel[1] - cy)
            if distance <= threshold_px:
                candidates.append((distance, point, kind, color, context, True))
        chosen = _resolve_point_candidates(candidates, is_occluded, are_occluded)
        if chosen is not None:
            return SnapResult(chosen[1], chosen[2], chosen[3], context=chosen[4])
        return None

    if query is not None:
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
    else:
        hit = point_on_lock(_point_snap_candidates(scene.edges))
        if hit is not None:
            return hit
        # Where the lock line crosses another edge → intersection (green
        # point), so a locked line landing on a crossing wall offers the exact
        # junction without breaking the lock.
        best_hit: Optional[tuple[float, QVector3D]] = None
        for edge in scene.edges:
            hit = _line_segment_intersection(start_point, line_dir, edge.a, edge.b)
            if hit is None:
                continue
            hp = world_to_pixel(hit)
            if hp is None:
                continue
            dh = math.hypot(hp[0] - cx, hp[1] - cy)
            if dh > threshold_px:
                continue
            if is_occluded is not None and is_occluded(hit):
                continue
            if best_hit is None or dh < best_hit[0]:
                best_hit = (dh, hit)
        if best_hit is not None:
            return SnapResult(best_hit[1], "intersection", COLOR_ENDPOINT,
                              guide=(start_point, best_hit[1]))
        hit = point_on_lock(_midpoint_snap_candidates(scene.edges))
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


#: Two point candidates closer than this on screen are a tie, and the tie
#: goes to the drawing context (see ``_resolve_point_candidates``).
_TIE_PX = 0.5


def _first_visible(points, is_occluded, are_occluded=None):
    """Index of the first of ``points`` that is not occluded, or ``None``.

    The first is asked alone (it is usually the answer); past it the rest go
    in batches of 8, 16, 32… through ``are_occluded`` when there is one, as
    ``compute_snap``'s ``_resolve`` asks its candidates: a dense model hides
    dozens behind the first wall, and one ray each was the hover's cost."""
    if not points:
        return None
    if is_occluded is None:
        return 0
    if are_occluded is None:
        return next((i for i, p in enumerate(points) if not is_occluded(p)),
                    None)
    if not is_occluded(points[0]):
        return 0
    k, size = 1, 8
    while k < len(points):
        batch = points[k:k + size]
        for j, h in enumerate(are_occluded(batch)):
            if not h:
                return k + j
        k += size
        size *= 2
    return None


def _closest_on_circle(circ, candidate_world, cx, cy, world_to_pixel, et):
    """The nearest point on a guide circle's rim to the cursor, as
    ``(screen_distance_px, world_point)`` when within ``et``, else
    ``(None, None)``.

    The circle is analytic: the candidate is projected onto the circle's
    plane and then radially onto the rim, so the snap is exact, not a
    polygon's vertex.
    """
    c = circ.center
    n = circ.normal
    r = float(circ.radius)
    if r <= 0.0:
        return None, None
    proj = candidate_world - n * QVector3D.dotProduct(candidate_world - c, n)
    v = proj - c
    if v.length() < 1e-9:
        # On the circle's axis every rim point is equally near: pick the
        # circle's own x-axis as a stable representative.
        from core.axes import plane_axes
        u, _w = plane_axes(n)
        v = u * r
    rim = c + v.normalized() * r
    px = world_to_pixel(rim)
    if px is None:
        return None, None
    d = math.hypot(px[0] - cx, px[1] - cy)
    return (d, rim) if d <= et else (None, None)


def _circle_circle_intersections(c1, r1, n1, c2, r2, n2):
    """The points where two circles meet: a list of 0, 1 or 2 ``QVector3D``.

    The circles must be coplanar (parallel normals, same plane); otherwise
    they do not meet in a point. Exact — no polygon vertices involved.
    """
    if r1 <= 0.0 or r2 <= 0.0:
        return []
    if abs(abs(QVector3D.dotProduct(n1, n2)) - 1.0) > 1e-6:
        return []                    # not coplanar
    d = c2 - c1
    dist = d.length()
    if dist < 1e-12:
        return []                    # concentric: no two points to offer
    if abs(QVector3D.dotProduct(d, n1)) > 1e-6 * max(1.0, dist):
        return []                    # parallel planes, offset apart
    dd = d / dist
    a = (dist * dist + r1 * r1 - r2 * r2) / (2.0 * dist)
    h2 = r1 * r1 - a * a
    if h2 < -1e-12:
        return []                    # separate, or one inside the other
    h = math.sqrt(max(h2, 0.0))
    mid = c1 + dd * a
    perp = QVector3D.crossProduct(n1, dd)
    if h < 1e-9:
        return [mid]                 # tangent
    return [mid + perp * h, mid - perp * h]


def _segment_circle_intersections(a, b, c, r, n):
    """Where a finite segment ``a→b`` meets a circle (centre ``c``, radius
    ``r``, unit normal ``n``): 0, 1 or 2 points, clamped to the segment."""
    if r <= 0.0:
        return []
    ab = b - a
    lab2 = QVector3D.dotProduct(ab, ab)
    if lab2 < 1e-12:
        return []
    denom = QVector3D.dotProduct(ab, n)
    if abs(denom) > 1e-9 * math.sqrt(lab2):
        # The segment's line pierces the circle's plane once.
        t = QVector3D.dotProduct(c - a, n) / denom
        if t < 0.0 or t > 1.0:
            return []
        p = a + ab * t
        return [p] if abs((p - c).length() - r) < 1e-6 else []
    # The segment lies in (or parallel to) the circle's plane.
    if abs(QVector3D.dotProduct(c - a, n)) > 1e-6:
        return []                    # parallel, not in the plane
    f = a - c
    dd = QVector3D.dotProduct(ab, ab)
    df = QVector3D.dotProduct(ab, f)
    ff = QVector3D.dotProduct(f, f)
    disc = df * df - dd * (ff - r * r)
    if disc < 0.0:
        return []
    sq = math.sqrt(max(disc, 0.0))
    out = []
    for t in ((-df - sq) / dd, (-df + sq) / dd):
        if 0.0 <= t <= 1.0:
            p = a + ab * t
            if not out or (p - out[-1]).length() > 1e-6:
                out.append(p)
    return out


def _circle_quadrants(c, r, n):
    """The four cardinal points of a circle (centre ``c``, radius ``r``,
    plane normal ``n``): N/E/S/W in the circle's own basis."""
    if r <= 0.0:
        return []
    from core.axes import plane_axes
    u, v = plane_axes(n)
    return [c + u * r, c - u * r, c + v * r, c - v * r]


def _circle_tangent_points(c, r, n, p):
    """The tangent points on a circle from an external point ``p`` (projected
    onto the circle's plane): 0 (inside), 1 (on the rim) or 2 points."""
    if hasattr(_native.native, 'circle_tangents'):
        return [QVector3D(*q) for q in _native.native.circle_tangents(
            (c.toTuple(), r, n.toTuple()), p.toTuple())]
    if r <= 0.0:
        return []
    proj = p - n * QVector3D.dotProduct(p - c, n)
    dvec = proj - c
    d = dvec.length()
    if d < r:
        return []                     # inside: no real tangent
    if d <= r:
        return [QVector3D(proj)]      # on the rim: the point itself
    dd = dvec / d
    w = QVector3D.crossProduct(n, dd)  # unit, in the plane, ⊥ dd
    cos_b = r / d
    sin_b = math.sqrt(1.0 - cos_b * cos_b)
    return [c + (dd * cos_b + w * sin_b) * r,
            c + (dd * cos_b - w * sin_b) * r]


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
    if query is not None:
        from core.snap_frame import SnapFrame
        frame = SnapFrame(options, query)
        return _native.native.resolve_snap(query, frame)
    return _compute_snap_python(**options)


def _compute_snap_python(**options):
    from core._snap_python import compute_snap
    return compute_snap(**options)

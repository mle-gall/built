# SPDX-License-Identifier: GPL-3.0-or-later
"""Reference implementation for the engine preference and arbitrary projection callbacks."""
from __future__ import annotations

import math

from ingetrazo_core import Vec3 as QVector3D
from typing import Callable, Optional

from core.snap import (
    AXIS_COLORS,
    COLOR_CLOSE,
    COLOR_ENDPOINT,
    COLOR_NONE,
    COLOR_ON_EDGE,
    COLOR_ON_FACE,
    COLOR_ORIGIN,
    COLOR_REFERENCE,
    COLOR_TANGENT,
    ProjectOntoLine,
    SnapResult,
    _AXIS_VECTORS,
    _STICKY_AXIS_RELEASE_DEG,
    _axis_in_plane,
    _circle_quadrants,
    _circle_tangent_points,
    _closest_on_circle,
    _detect_axis_alignment,
    _direction_from_edge,
    _edge_cursor_point,
    _edge_face_snap,
    _edge_from_point_crossing,
    _engaged_axis,
    _extension_snap,
    _first_point_from_point,
    _first_visible,
    _from_point_snap,
    _in_plane_with_point_snap,
    _intersection_snap,
    _line_segment_intersection,
    _lock_line_snaps,
    _midpoint_snap_candidates,
    _perpendicular_face_snap,
    _point_on_segment_world,
    _point_snap_candidates,
    _resolve_point_candidates,
    _still_along_axis,
    _through_point_snap,
    _two_point_snap,
    _vertex_on_line,
)


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
    # Linear-inference toggle (Alt): "all" = every inference, "off" =
    # point snaps only, "parallel_perp" = keep only parallel/perpendicular. The
    # explicit locks (arrow keys, Down-arrow reference) always work regardless.
    allow_axis = linear_mode == "all"          # axis / from-point / extension
    allow_parperp = linear_mode != "off"       # parallel / perpendicular
    # A protractor ARM (Rotate, Protractor) is a direction from the centre,
    # not a line being drawn: 'through point', 'extension' and 'from point'
    # are line-drawing inferences. On an arm they only fought the axis
    # magnet — a diameter 1.7° off X held the arm on its own direction
    # through the reference just clicked, and a chord of the next circle
    # turned the angle with the cursor's distance (issue #140, @pacaeiro).
    line_inferences = allow_axis and not radial_arm
    # On a face, the LINE inferences stay ON that face: rule 5 (extension)
    # and rule 8d ('level with a point across the face'). A line is drawn in
    # ONE plane, and the face under the cursor is the one the badge names —
    # an inference that answered off it left the point, the plane and the
    # readout describing three different things (#545, @pacaeiro). The point
    # inferences (endpoint, midpoint, on-edge, origin) are deliberately NOT
    # constrained: a vertex is a vertex, and the shape re-plans onto it (see
    # ``PlaneLock.snapped_plane``).
    on_plane = work_plane_normal if face_under_cursor else None
    # A point acquired where the operation STARTED adds nothing: every line
    # through the start along an axis is the axis inference itself. Left in,
    # its derived «From point» outranked the axis cue it derives from, so
    # dragging an edge along red read «From point» all the way and even
    # nudged the point off the cursor (#368, @pacaeiro — the family where a
    # derived inference must never beat what it comes from). It still counts
    # as one of TWO acquired points: their crossing is a real inference.
    if (acquired_point is not None and start_point is not None
            and (acquired_point - start_point).lengthSquared() < 1e-12):
        acquired_point = None

    # A rectangle's second click is its OPPOSITE corner: the span from the
    # start to the cursor is a DIAGONAL, not a line being drawn (#389,
    # @jonson7777: «snaps to some purpel line and turns it into a line»). A
    # thin rectangle's diagonal runs a fraction of a degree off an axis, and
    # every rule that reads start→cursor as a line's direction — the axis,
    # perpendicular, parallel, through point, extension, 'from point' along
    # the draw — held the far corner on a line through the first one: width
    # zero. Per side those inferences have nothing to do, because the sides
    # run along the drawing plane's own axes by construction. What a side
    # CAN take from the model is its length, lined up with a point — the
    # far corner level with a visited corner (8d, 8e) — and those rules
    # start at the cursor, so they stay, as do the point snaps. The
    # explicit locks (arrows, Down, a captured Shift) keep ``start_point``.
    line_start = None if diagonal_span else start_point
    if diagonal_span and start_point is not None and acquired_points:
        # The first corner, encouraged on the way to clicking it, would cross
        # its own axis lines with another point's: a corner on its own row.
        acquired_points = [p for p in acquired_points if p is not None
                           and (p - start_point).lengthSquared() >= 1e-12]

    # The axis that has taken command of the drag, settled once here and used
    # twice below: it silences the DERIVED linear inferences (rules 4b, 5, 5b,
    # 5c, 7, 8b, 8c, 8d) so nothing can steal the point off the line, and rule 9
    # returns it as the lock. The POINT inferences are deliberately NOT silenced
    # - the endpoint/close snaps, 'through point' and the origin still outrank
        # it, because fetching a vertex with the mouse is the one thing that must
    # keep winning (issue #140 pins 'through point' above the magnet on the Line
    # tool). Explicit locks always have the last word. See
    # :func:`_engaged_axis`.
    axis_engaged = None
    if allow_axis and project_onto_line is not None:
        axis_engaged = _engaged_axis(
            line_start, candidate_world, candidate_pixel, world_to_pixel,
            project_onto_line, magnetic_axis_deg, screen_axis_px,
            work_plane_normal, axis_in_plane,
        )
        if axis_in_plane:
            # A shape that lives in ONE plane can only be magnetised along
            # the axes that plane HOLDS. An axis it cannot hold would carry
            # the point off the plane, and a circle's rim — measured IN the
            # plane — collapses to nothing while the badge still reads «On
            # face» (#545, @pacaeiro: «em determinadas faces o círculo já
            # não segue a posição do rato, fica a metade ou menos ainda»).
            # Over an axis-aligned face the plane holds two world axes and
            # the magnet keeps working; over a sloped one it holds none of
            # them and the cursor's own point on the face stands.
            axis_engaged = _axis_in_plane(axis_engaged, work_plane_normal)
        if (axis_engaged is None and sticky_axis is not None
                and line_start is not None):
            # Sticky axis: the magnet engaged ``sticky_axis`` on an earlier
            # hover. Hold it while the draw keeps heading along that axis, so
            # a line started along X stays on X in perspective even as the
            # hand drifts (issue #1095). A deliberate diagonal breaks out once
            # it turns past ``_STICKY_AXIS_RELEASE_DEG``.
            if _still_along_axis(sticky_axis, line_start, candidate_world,
                                 _STICKY_AXIS_RELEASE_DEG):
                axis_engaged = sticky_axis
        if (axis_engaged is not None and line_start is not None
                and project_onto_line is not None):
            # The magnet must not take the point BEHIND the face the cursor
            # is over. On a sloped face, the red or green line through a
            # start on the face runs under it: the magnet engaged, silenced
            # «On edge» / «On face», and the line ended in the air under the
            # roof — a loop drawn on it made a floating face, a line across
            # it cut nothing. An axis lying IN the face (the eave) or rising
            # in front of it (a chimney) is visible and still engages.
            # Asked whether or not a face was hit under the cursor: on a
            # face's border edge the ray grazes past it, and the axis point
            # hidden under the face is just as wrong there. Cost: one
            # occlusion ray, only while an axis engages and its point is off
            # the plane the cursor's own point lies on.
            #
            # And a line drawn ON the face (its start on the face's plane)
            # keeps to the face while the cursor is over it: an axis that
            # runs at a shallow angle to a sloped face (under 45°) is not the
            # way off it — the cursor on the face and the axis point lie on
            # the same pixel, and the face is what the badge names. The
            # axis along the normal, or steeper than 45° (a post rising from
            # a sloped roof), still engages. No ray for this one.
            axis_dir = _AXIS_VECTORS[axis_engaged]
            ax_pt = project_onto_line(line_start, axis_dir)
            n = work_plane_normal
            n = n.normalized() if n is not None and n.length() > 1e-9 else None
            off_plane = (n is None or abs(
                QVector3D.dotProduct(ax_pt - candidate_world, n)) > 1e-3)
            if off_plane and n is not None and face_under_cursor:
                rise = abs(QVector3D.dotProduct(axis_dir.normalized(), n))
                start_on_face = abs(QVector3D.dotProduct(
                    line_start - candidate_world, n)) <= 1e-3
                if start_on_face and 0.0087 < rise < 0.7071:
                    axis_engaged = None
            if (axis_engaged is not None and off_plane
                    and is_occluded is not None and is_occluded(ax_pt)):
                axis_engaged = None

    # 0.5 Axis lock BEFORE the first click (issue #1095): with no start point
    #     yet, an arrow key constrains the first click to the encouraged
    #     point's own axis line, so a line starts 'level with' the dwell. The
    #     point snaps a lock normally offers stay available on that line.
    if (dwell_axis_lock and axis_lock and start_point is None
            and acquired_point is not None and project_onto_line is not None):
        axis_dir = _AXIS_VECTORS[axis_lock]
        cx, cy = candidate_pixel
        hit = _lock_line_snaps(
            scene, acquired_point, axis_dir, cx, cy, world_to_pixel,
            threshold_px, is_occluded, acquired_point, chain_first_point,
            project_onto_line, hover_lock_edge, hover_lock_plane, are_occluded,
        )
        guide = (QVector3D(acquired_point),
                 project_onto_line(acquired_point, _AXIS_VECTORS[axis_lock]))
        guide_color = AXIS_COLORS[axis_lock]
        if hit is not None:
            hit.lock_color = AXIS_COLORS[axis_lock]
            if hit.guide is None:
                hit.guide = guide
                hit.guide_color = guide_color
            return hit
        locked = project_onto_line(acquired_point, axis_dir)
        return SnapResult(locked, "axis", AXIS_COLORS[axis_lock], axis=axis_lock,
                          guide=guide, guide_color=guide_color)

    # 1. Explicit axis lock (arrow keys). Use the viewport's camera-aware
    #    projection so locks to Z (vertical) actually move along Z. Existing
    #    vertices that fall on the lock line still get an endpoint snap, so
    #    you can land exactly on them without leaving the lock.
    if axis_lock and start_point is not None and project_onto_line is not None:
        axis_dir = _AXIS_VECTORS[axis_lock]
        locked = project_onto_line(start_point, axis_dir)
        cx, cy = candidate_pixel
        # A lock line runs BOTH ways from the start, so point it at the
        # cursor. 'From point' below drops any reference behind the draw
        # direction — sound when the draw direction IS the cursor, but here
        # the axis was handed over as its positive vector whichever way the
        # user was going, so HALF THE AXIS offered nothing: the same corner,
        # the same lock, snapped when approached from the left and not from
        # the right (measured 2026-09-17; issue #27, @pacaeiro: «almost none
        # of them are detected»). The two rules above work on the infinite
        # line and do not care about the sign.
        if QVector3D.dotProduct(locked - start_point, axis_dir) < 0:
            axis_dir = -axis_dir
        # 1a–1d: closing, a vertex on the line, a crossing, 'from point'
        #        (``_lock_line_snaps``; issues #27, #34).
        hit = _lock_line_snaps(
            scene, start_point, axis_dir, cx, cy, world_to_pixel,
            threshold_px, is_occluded, acquired_point, chain_first_point,
            project_onto_line, hover_lock_edge, hover_lock_plane, are_occluded,
        )
        # A hard lock also keeps the 'level with' reference to an encouraged
        # point visible: its dashed axis line, so the cursor can go fetch
        # another point on the locked axis without losing the reference
        # (issue #1095).
        guide = guide_color = None
        if acquired_point is not None:
            guide = (QVector3D(acquired_point),
                     project_onto_line(acquired_point, _AXIS_VECTORS[axis_lock]))
            guide_color = AXIS_COLORS[axis_lock]
        if hit is not None:
            hit.lock_color = AXIS_COLORS[axis_lock]
            if hit.guide is None and guide is not None:
                hit.guide = guide
                hit.guide_color = guide_color
            return hit
        return SnapResult(locked, "axis", AXIS_COLORS[axis_lock], axis=axis_lock,
                          guide=guide, guide_color=guide_color)

    # 1.4 Shift lock BEFORE the first click (issue #1095): the lock was captured
    #     from a 'from point' reference, so it anchors on that point's own axis
    #     line until the line starts. The point snaps a lock normally offers
    #     stay available on that line.
    if (shift_lock_dir is not None and shift_lock_ref is not None
            and start_point is None and project_onto_line is not None):
        ref_pt, ref_axis = shift_lock_ref
        if ref_axis in _AXIS_VECTORS:
            axis_dir = _AXIS_VECTORS[ref_axis]
            cx, cy = candidate_pixel
            hit = _lock_line_snaps(
                scene, ref_pt, axis_dir, cx, cy, world_to_pixel,
                threshold_px, is_occluded, ref_pt, chain_first_point,
                project_onto_line, hover_lock_edge, hover_lock_plane, are_occluded,
            )
            color = shift_lock_color if shift_lock_color is not None else COLOR_REFERENCE
            guide = (QVector3D(ref_pt), project_onto_line(ref_pt, axis_dir))
            guide_color = AXIS_COLORS[ref_axis]
            if hit is not None:
                hit.lock_color = color
                if hit.guide is None:
                    hit.guide = guide
                    hit.guide_color = guide_color
                return hit
            locked = project_onto_line(ref_pt, axis_dir)
            return SnapResult(locked, "reference", color,
                              guide=guide, guide_color=guide_color)

    # 1.5 Sticky inference lock (Shift captured an active inference): hold that
    #     direction regardless of cursor, the way Shift classically locks whatever
    #     inference was showing. Vertices on the lock line still snap so you can
    #     land exactly on a corner without leaving the lock.
    if (
        shift_lock_dir is not None
        and start_point is not None
        and project_onto_line is not None
    ):
        cx, cy = candidate_pixel
        lock_dir = QVector3D(shift_lock_dir)
        locked = project_onto_line(start_point, lock_dir)
        # The lock line runs BOTH ways from the start: point it at the cursor,
        # exactly as rule 1 does since issue #27, or half of it offers nothing.
        if QVector3D.dotProduct(locked - start_point, lock_dir) < 0:
            lock_dir = -lock_dir
        # Everything rule 1 gained in issue #27 and this one never did.
        # @pacaeiro on 0.4.4: «when I get an Axis and locked it with Shift, I
        # do not have any Snaps… P.S. If I use hard lock (arrows) I have the
        # Snaps working». He is right, and he also says why it matters: Shift
        # is the lock you use precisely to go and fetch a point from another
        # object. Held to a direction with no points to fetch, it is half a
        # tool. Same sub-rules, same order (``_lock_line_snaps``).
        hit = _lock_line_snaps(
            scene, start_point, lock_dir, cx, cy, world_to_pixel,
            threshold_px, is_occluded, acquired_point, chain_first_point,
            project_onto_line, hover_lock_edge, hover_lock_plane, are_occluded,
        )
        color = shift_lock_color if shift_lock_color is not None else COLOR_REFERENCE
        # A lock captured from a 'from point' keeps its dashed, axis-coloured
        # guide (the dwell point's own axis line) while the cursor is free to
        # fetch another point on the locked line (issue #1095).
        guide = guide_color = None
        if shift_lock_ref is not None:
            ref_pt, ref_axis = shift_lock_ref
            if ref_axis in _AXIS_VECTORS:
                guide = (QVector3D(ref_pt),
                         project_onto_line(ref_pt, _AXIS_VECTORS[ref_axis]))
                guide_color = AXIS_COLORS[ref_axis]
        if hit is not None:
            hit.lock_color = color
            if hit.guide is None and guide is not None:
                hit.guide = guide
                hit.guide_color = guide_color
            return hit
        return SnapResult(locked, "reference", color,
                          guide=guide, guide_color=guide_color)

    # 2. Reference edge lock (Down arrow + edge under cursor).
    if (
        reference_edge is not None
        and reference_mode
        and start_point is not None
        and project_onto_line is not None
    ):
        direction = _direction_from_edge(reference_edge, reference_mode,
                                         work_plane_normal)
        if direction is not None:
            locked = project_onto_line(start_point, direction)
            if QVector3D.dotProduct(locked - start_point, direction) < 0:
                direction = -direction
            hit = _lock_line_snaps(
                scene, start_point, direction, candidate_pixel[0],
                candidate_pixel[1], world_to_pixel, threshold_px,
                is_occluded, acquired_point, chain_first_point,
                project_onto_line, hover_lock_edge, hover_lock_plane, are_occluded,
            )
            if hit is not None:
                hit.lock_color = COLOR_REFERENCE
                return hit
            return SnapResult(locked, "reference", COLOR_REFERENCE)

    # 3. Shift held + auto axis inference → lock to that axis. This is the
    #    lock you get when the Shift press found nothing to capture (after
    #    orbiting away, say) and the cursor then lines up with an axis. It
    #    had NO snaps — the third such lock (issue #34, @pacaeiro: «release
    #    Shift to reposition the camera… reactivate the Shift, half of the
    #    Snap points are not detected»). Same sub-rules as the other two.
    if allow_axis and shift_held and line_start is not None and project_onto_line is not None:
        inferred = _detect_axis_alignment(
            line_start, candidate_world, inference_angle_deg
        )
        if axis_in_plane:
            inferred = _axis_in_plane(inferred, work_plane_normal)
        if inferred is not None:
            axis_dir = QVector3D(_AXIS_VECTORS[inferred])
            locked = project_onto_line(line_start, axis_dir)
            if QVector3D.dotProduct(locked - line_start, axis_dir) < 0:
                axis_dir = -axis_dir
            cx3, cy3 = candidate_pixel
            hit = _lock_line_snaps(
                scene, line_start, axis_dir, cx3, cy3, world_to_pixel,
                threshold_px, is_occluded, acquired_point, chain_first_point,
                project_onto_line, hover_lock_edge, hover_lock_plane, are_occluded,
            )
            if hit is not None:
                hit.lock_color = AXIS_COLORS[inferred]
                return hit
            return SnapResult(locked, "axis", AXIS_COLORS[inferred], axis=inferred)

    cx, cy = candidate_pixel
    et = edge_threshold_px if edge_threshold_px is not None else threshold_px
    best: Optional[tuple[float, QVector3D, str, tuple[float, float, float]]] = None

    # Candidates within the snap radius, resolved LAZILY: sorted by screen
    # distance and occlusion-tested in that order until the first visible
    # one. Testing every candidate up front cast a ray per point — dozens
    # per hover next to dense geometry (the plaza's pergola: 70 ms a move,
    # measured 2026-09-14) for the same answer the nearest visible gives.
    pending: list = []

    def _consider(
        world: QVector3D,
        kind: str,
        color: tuple[float, float, float],
        occludable: bool = True,
        context: Optional[str] = None,
    ) -> None:
        px = world_to_pixel(world)
        if px is None:
            return
        d = math.hypot(px[0] - cx, px[1] - cy)
        if d > threshold_px:
            return
        pending.append((d, world, kind, color, context, occludable))

    def _resolve():
        """The nearest visible candidate, or ``None``; clears the list.

        The same visibility and tie policy as a direction lock's points
        (``_resolve_point_candidates``). Near-ties go to the point of the
        context being drawn in — a loose vertex beats a component's corner
        at the same spot. The two really do coincide when the line was
        started ON that corner, and only the loose vertex can be welded to
        (issue #36, @pacaeiro: «Endpoint in component and the vertice
        endpoint share the same coordinate, but no face created»). Hidden
        candidates are asked in doubling batches (``_first_visible``): a
        dense model hides dozens behind the first wall."""
        nonlocal best
        chosen = _resolve_point_candidates(pending, is_occluded, are_occluded)
        pending.clear()
        best = chosen[:5] if chosen is not None else None
        return best

    # 4. Vertex snaps (close, endpoint) — the highest-priority discrete points.
    if (
        chain_first_point is not None
        and start_point is not None
        and chain_first_point is not start_point
    ):
        # The point being chained to is part of the live drawing, not hidden
        # scene geometry — never occlusion-cull it.
        _consider(chain_first_point, "close", COLOR_CLOSE, occludable=False)
        _resolve()
    if best is None or best[2] != "close":
        # The named points first (a tie goes to the first considered): an
        # arc's midpoint that happens to fall on one of its facet vertices
        # reads "Arc midpoint", the usual name, not "Endpoint".
        # Named points (centre, component origin, arc midpoint), then a
        # guide circle's four quadrants (the cardinal N/E/S/W of the circle,
        # exact), then the world origin, then plain endpoints: the same
        # candidates, in the same order, a direction lock fetches
        # (``_point_snap_candidates``). The world origin is a point inference
        # like a corner, so it beats the LINEAR inferences of rule 5 (Marco,
        # 2026-09-15: «me quiero poner en el origen y no se pone»). Points
        # inside a component keep their colours; the tip says "in component".
        quadrants = [q for circ in getattr(scene, "circles", ())
                     for q in _circle_quadrants(circ.center, circ.radius,
                                                circ.normal)]
        for point, kind, color, context in _point_snap_candidates(
                scene.edges, quadrants):
            _consider(point, kind, color, context=context)
        for point, context in face_centers or ():
            # The centres of the face under the cursor and of the selected
            # faces, ``(point, context)``. Handed in on their own, not as
            # degenerate pseudo-edges: the line rules (on edge, extension,
            # the lock's 'endpoint on the line') would read such a point as
            # a scrap of edge. After the endpoints, so a corner wins a tie.
            #
            # Not the point the operation started on: a circle begun at a
            # face's centre found that same centre again under its rim, and
            # the radius snapped back to nothing — no disc was built. A
            # corner is geometry the drawing welds to; this point is not,
            # and landing on it again only gives a zero-length step.
            if start_point is not None and (point - start_point).length() < 1e-6:
                continue
            _consider(point, "face_center", COLOR_ON_FACE, context=context)
        # A "close" already chosen stands; otherwise the nearest visible of
        # the named points and endpoints (named ones first on a tie — they
        # were appended first and the sort is stable).
        if best is None:
            _resolve()
    if best is not None:
        return SnapResult(best[1], best[2], best[3], context=best[4])

    def _edge_under_cursor(exclude_at: QVector3D) -> bool:
        """Whether a visible edge not passing through ``exclude_at`` lies within
        the on-edge reach of the cursor (rule 7's target)."""
        for edge in scene.edges:
            if (edge.b - edge.a).length() < 1e-9:
                continue                      # a pseudo-edge: centre, origin…
            if _point_on_segment_world(exclude_at, edge.a, edge.b):
                continue
            hit = _edge_cursor_point(edge, (cx, cy), world_to_pixel, et,
                                     project_onto_line)
            if hit is None:
                continue
            on_pt = hit[2]
            if is_occluded is None or not is_occluded(on_pt):
                return True
        return False

    # 4b. Perpendicular to a wall you started on: drawing square to it locks the
    #     exact perpendicular (magenta) and predicts the connection — where that
    #     perpendicular line crosses another edge near the cursor (the parallel
    #     wall) as a green point. Gated on having started on the edge and drawing
    #     square to it, so it's high priority (beats midpoint/on-edge) without
    #     fighting free-angle drawing. Yields to an engaged axis (see above).
    if (
        allow_parperp
        and axis_engaged is None
        and line_start is not None
        and project_onto_line is not None
    ):
        draw = candidate_world - line_start
        if draw.length() > 1e-6:
            draw_u = draw.normalized()
            cos_tol = math.cos(math.radians(8.0))  # forgiving: it snaps to exact
            # ``scene.edges`` are the edges near the CURSOR; the edge the line
            # started on is far from it once the line is longer than the
            # search radius, and the lock went with it (the Metal view only
            # hands in nearby edges). ``start_edges`` are the edges through
            # the start, found once per start by the caller.
            for edge in list(scene.edges) + list(start_edges or ()):
                if not _point_on_segment_world(line_start, edge.a, edge.b):
                    continue
                perp = _direction_from_edge(edge, "perpendicular",
                                            work_plane_normal)
                if perp is None:
                    continue
                if abs(QVector3D.dotProduct(draw_u, perp)) < cos_tol:
                    continue
                best_hit = None
                for other in scene.edges:
                    if _point_on_segment_world(line_start, other.a, other.b):
                        continue
                    hit = _line_segment_intersection(
                        line_start, perp, other.a, other.b)
                    if hit is None:
                        continue
                    if QVector3D.dotProduct(hit - line_start, draw_u) <= 0:
                        continue  # behind the cursor
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
                    return SnapResult(best_hit[1], "intersection", COLOR_ENDPOINT,
                                      guide=(line_start, best_hit[1]))
                # Before settling for a bare perpendicular lock, let a corner's
                # projection win: landing where this perpendicular lines up with
                # a corner is the exact point the user is after, and the generic
                # lock would otherwise shadow it.

                if line_inferences and axis_engaged is None:

                    fp = _from_point_snap(
                        scene, line_start, candidate_world - line_start,
                        cx, cy, world_to_pixel, threshold_px, is_occluded,
                        extra_point=acquired_point,
                    )
                    if fp is not None:
                        return fp
                # The lock is chosen by ANGLE (an 8 degree cone), so from a
                # corner it also catches a line meant for a point of another
                # edge: a tapered leg ends on the far edge about 8 degrees off
                # the perpendicular, and the lock moved it 27 px onto the wall
                # it started on (#544). An edge under the cursor is a target
                # the user pointed at: it wins. The edges the line starts on
                # are not such targets - the perpendicular may be one of them.
                if _edge_under_cursor(line_start):
                    break
                locked = project_onto_line(line_start, perp)
                return SnapResult(locked, "reference", COLOR_REFERENCE)

    # 4e. 'Through point': heading along the line from the start through an
    #     encouraged corner locks onto it (magenta), so the segment passes
    #     exactly through that point even past it.
    #
    #     …but not when the cursor is ON that point. Then the user is not
    #     heading through it, they are landing on it, and this rule kept
    #     them on the ray SHORT of it — pausing over a point quietly made
    #     that point unsnappable. Marco, 2026-09-18: he hooked the midpoint
    #     of a rectangle's edge, saw its marker, clicked once, and the line
    #     stopped 3.9 cm short without even splitting the edge. Measured
    #     with the cursor exactly on the midpoint:
    #
    #         nothing acquired            midpoint        0.00 cm
    #         that midpoint acquired      through_point   0.73 cm
    #
    #     The offset ran precisely back along the draw direction, which is
    #     what gave it away. Same shape as the from-point fix an hour
    #     earlier: an inference DERIVED from a point must not beat the
    #     point it came from.
    if line_inferences and line_start is not None:
        sobre_el_punto = False
        if acquired_point is not None:
            ap = world_to_pixel(acquired_point)
            if ap is not None:
                sobre_el_punto = math.hypot(ap[0] - cx, ap[1] - cy) <= threshold_px
        if not sobre_el_punto:
            tp = _through_point_snap(
                line_start, acquired_point, candidate_world - line_start,
                project_onto_line, inference_angle_deg,
            )
            if tp is not None:
                return tp

    # 5. Extension: when drawing collinear with an edge, snap along its dashed
    #     continuation — and to where that extension crosses another edge (a
    #     definite green connection point), so you can extend a line exactly onto
    #     a perpendicular one. Gated on the draw direction being collinear with
    #     the edge, so it only fires when you mean to extend (no line noise).
    #     Runs before 'from point' so extending a line wins over a corner line-up.

    if line_inferences and axis_engaged is None:

        ext = _extension_snap(
            candidate_world, cx, cy, scene, world_to_pixel, et, line_start,
            is_occluded, project_onto_line=project_onto_line,
            plane_normal=on_plane,
        )
        if ext is not None:
            return ext

    # 5b. 'From point' ("Desde el punto"): drawing along an axis, line up with a
    #     corner (green) or midpoint (cyan) — the fixed foot of that point on the
    #     axis-aligned draw line. Only fires on-axis, so free-angle draws stay
    #     quiet and the point never scatters or slides.
    if line_inferences and line_start is not None and axis_engaged is None:
        fp = _from_point_snap(
            scene, line_start, candidate_world - line_start,
            cx, cy, world_to_pixel, threshold_px, is_occluded,
            extra_point=acquired_point,
        )
        if fp is not None:
            return fp

    # 5c. Edge / guide intersection (the green X): where two edges or
    #     guide lines actually cross. Only the directional locks above build an
    #     intersection, so crossing guides never offered their meeting point —
    #     the cursor slid along the nearest guide. Runs before midpoint/on-edge
    #     so the exact crossing wins, but below every endpoint and lock.
    inter = _intersection_snap(cx, cy, scene, world_to_pixel, et, is_occluded,
                               are_occluded, candidate_world)
    if inter is not None:
        if axis_engaged is None:
            return inter
        # The intersection is a POINT snap and must keep beating the engaged
        # axis (as endpoint/close do) — but only while it lies ON that axis,
        # so it never pulls the point off the lock line. Two guide circles
        # side by side cross twice on the line through their centres: that
        # chord lines up with an axis, and without this the axis magnet stole
        # the second crossing from a line started at the first (#238).
        if start_point is not None and _vertex_on_line(
                inter.point, start_point, _AXIS_VECTORS[axis_engaged]):
            return inter

    # 5d. An edge — guide, line, arc or circle facet — piercing the face under
    #     the cursor. Asked after the edge/edge crossing and before the
    #     midpoint / on-edge / on-line rules, which would otherwise slide
    #     along the edge past the crossing.
    if face_outline is not None and face_under_cursor and axis_engaged is None:
        outline = face_outline()
        if outline is not None:
            pierce = _edge_face_snap(cx, cy, scene, world_to_pixel, et,
                                     is_occluded, outline)
            if pierce is not None:
                return pierce

    # 6. Midpoint + origin.
    best = None
    for point, kind, color, context in _midpoint_snap_candidates(scene.edges):
        _consider(point, kind, color, context=context)
    _consider(QVector3D(0.0, 0.0, 0.0), "origin", COLOR_ORIGIN)
    _resolve()
    if best is not None:
        return SnapResult(best[1], best[2], best[3], context=best[4])

    # 6b. Tangent to a guide circle: while drawing from a fixed start, snap to
    #     the point on the circle where the segment is tangent (the classic CAD
    #     tangent). Two exist; the nearest one wins. Only meaningful with a
    #     start point — a line's, not a shape's diagonal span.
    if line_start is not None:
        for circ in getattr(scene, "circles", ()):
            for tp in _circle_tangent_points(
                    circ.center, circ.radius, circ.normal, line_start):
                _consider(tp, "tangent", COLOR_TANGENT)
        _resolve()
        if best is not None:
            return SnapResult(best[1], best[2], best[3], context=best[4])

    # 7. On-edge / on-circle: an arbitrary point along an edge or a guide
    #    circle's rim. A linear target is big, so it gets a more generous
    #    radius than the point snaps — landing a corner on it (a door on the
    #    floor line) should be forgiving.
    on_targets = []
    for edge in scene.edges:
        # The point under the cursor through the camera ray (the screen
        # parameter is not the world one under perspective), and the visible
        # part of an edge whose other end is behind the camera.
        hit = _edge_cursor_point(edge, (cx, cy), world_to_pixel, et,
                                 project_onto_line)
        if hit is None:
            continue
        d, t, on_pt = hit
        if t <= 0.0 or t >= 1.0:
            # Past either end the nearest point is the endpoint itself: an
            # «On edge» there named a point the endpoint snap had already
            # turned down (#587, between its 9 px and the edge's 14 px).
            continue
        kind = "on_line" if getattr(edge, "guide", False) else "on_edge"
        on_targets.append((d, on_pt, kind, edge))
    # A guide circle's rim is the same big target, resolved exactly: the
    # nearest point on the analytic circle to the cursor.
    for circ in getattr(scene, "circles", ()):
        d, rim_pt = _closest_on_circle(
            circ, candidate_world, cx, cy, world_to_pixel, et)
        if d is not None:
            on_targets.append((d, rim_pt, "on_circle", circ))
    # The nearest VISIBLE one, asked nearest first: a ray per target in reach
    # was most of a hover over a dense model (14 000 rays in 60 moves over
    # the SPR of #158, when only the first visible one is kept). The sort is
    # stable, so a tie goes to the target listed first, as it always did.
    on_targets.sort(key=lambda c: c[0])
    k = _first_visible([c[1] for c in on_targets], is_occluded, are_occluded)
    best_target = on_targets[k] if k is not None else None
    # 7a. With an axis engaged the on-edge point is silenced (it would pull
    #     the point off the line), but the edge under the cursor may CROSS
    #     the axis line: that crossing is on the line AND on the edge, the
    #     junction the arrow lock already offers (1c). Without it a line or
    #     arc drawn along red from one edge of a face to the opposite one
    #     landed 1–2 mm inside or outside the edge depending on the pixel,
    #     and left loose stubs (0.5.7 landed exactly; the chair backrest of
    #     the tutorial hunt, 2026-10-12). A circle rim has no such straight
    #     crossing, so it stays silenced under a lock.
    if best_target is not None and axis_engaged is not None:
        _d, _on_pt, kind, target = best_target
        if kind != "on_circle":
            hit = _line_segment_intersection(
                start_point, _AXIS_VECTORS[axis_engaged], target.a, target.b)
            if hit is not None:
                hp = world_to_pixel(hit)
                if (hp is not None
                        and math.hypot(hp[0] - cx, hp[1] - cy) <= et
                        and (is_occluded is None or not is_occluded(hit))):
                    return SnapResult(hit, "intersection", COLOR_ENDPOINT,
                                      guide=(start_point, hit))
            elif (kind == "on_edge" and line_start is not None
                    and work_plane_normal is not None
                    and work_plane_normal.length() > 1e-9):
                # …and when it does NOT cross it — the edge lies in the plane
                # the line is drawn on (a sloped face's border) and the axis
                # leaves that plane at a shallow angle — the axis line passes
                # under or over the edge, and the edge the cursor is on is
                # the target. Locked to the axis, a line across a sloped face
                # ended in the air beside its border and cut nothing.
                n = work_plane_normal.normalized()
                rise = abs(QVector3D.dotProduct(
                    _AXIS_VECTORS[axis_engaged].normalized(), n))
                in_plane = all(abs(QVector3D.dotProduct(p - line_start, n))
                               <= 1e-3 for p in (target.a, target.b))
                if in_plane and 0.0087 < rise < 0.7071:
                    axis_engaged = None
    if best_target is not None and axis_engaged is None:
        _d, on_pt, kind, target = best_target
        if kind == "on_edge" and line_inferences and acquired_point is not None:
            # On an edge AND lined up with the acquired point: the one point
            # of the edge that is both. The edge used to win outright, the
            # dotted «from point» line vanished as the cursor reached the
            # wall and the length jumped with every pixel — «hasta el final
            # no me llega» (Rafael, revision 4, 01:30).
            cross = _edge_from_point_crossing(
                target, acquired_point, cx, cy, world_to_pixel, threshold_px,
                is_occluded)
            if cross is not None:
                return cross
        if kind == "on_circle":
            return SnapResult(on_pt, "on_circle", COLOR_ON_EDGE)
        if kind == "on_line":
            return SnapResult(on_pt, "on_line", COLOR_ON_EDGE)   # a guide line
        return SnapResult(on_pt, "on_edge", COLOR_ON_EDGE,
                          context=getattr(target, "context", None),
                          edge=target)
                          # 17.10.26 Bastelbaus: added edge which is used on dimension tool
                          #                      to directly create a dimension by clickling
                          #                      on an edge

    # 7c. Where the face under the cursor meets another face with no edge
    #     between them — a box sunk into the floor, a beam through a wall
    #     (#819, the modeller's «Intersection»). Asked only here, once nothing
    #     with a name is nearer: the caller's search casts a few rays.
    if face_crossing is not None and face_under_cursor and axis_engaged is None:
        crossing = face_crossing()
        if crossing is not None:
            return SnapResult(crossing[0], "intersection", COLOR_ENDPOINT,
                              context=crossing[1])

    # 8b. Acquired-edge parallel inference. An edge the cursor hovered while
    #     drawing is held as a reference; when the draw runs parallel to it the
    #     line locks parallel (magenta), so you can align to an off-axis wall
    #     without pressing Down. Axis-aligned edges are left to the axis
    #     inference below so they keep their red/green/blue cue.
    if (
        allow_parperp
        and axis_engaged is None
        and acquired_edge is not None
        and line_start is not None
        and project_onto_line is not None
    ):
        ev = acquired_edge.b - acquired_edge.a
        draw = candidate_world - line_start
        if (
            ev.length() > 1e-9
            and draw.length() > 1e-6
            and _detect_axis_alignment(acquired_edge.a, acquired_edge.b,
                                       inference_angle_deg) is None
        ):
            ev_u = ev.normalized()
            par_cos = math.cos(math.radians(inference_angle_deg))
            if abs(QVector3D.dotProduct(draw.normalized(), ev_u)) >= par_cos:
                locked = project_onto_line(line_start, ev_u)
                return SnapResult(locked, "reference", COLOR_REFERENCE)
        # …and PERPENDICULAR to it, in the plane being drawn on: the other
        # half of the hovered-edge reference (only parallel was offered).
        # A perpendicular that runs along an axis is left to the axis cue,
        # as the parallel above leaves axis-aligned edges.
        perp = (_direction_from_edge(acquired_edge, "perpendicular",
                                     work_plane_normal)
                if ev.length() > 1e-9 and draw.length() > 1e-6 else None)
        if (perp is not None
                and _detect_axis_alignment(QVector3D(), perp,
                                           inference_angle_deg) is None
                and abs(QVector3D.dotProduct(draw.normalized(), perp))
                >= math.cos(math.radians(inference_angle_deg))):
            locked = project_onto_line(line_start, perp)
            return SnapResult(locked, "reference", COLOR_REFERENCE)

    # 8c. Perpendicular to an encouraged face: heading along its normal locks
    #     the line square-out of the face (magenta).
    if (
        allow_parperp
        and axis_engaged is None
        and line_start is not None
        and project_onto_line is not None
    ):
        pf = _perpendicular_face_snap(
            line_start, acquired_face_normal, candidate_world - line_start,
            project_onto_line, inference_angle_deg,
        )
        if pf is not None:
            return pf

    # 8d. 'From point' from an encouraged point — the dotted line
    #     from the corner you last hovered. This is how a window's first
    #     corner lands level with the door's top (Rafael, 2026-09-10: «te
    #     salía una línea de extensión para poder dibujar aquí la ventana»).
    #
    #     It used to live ABOVE the named points and only before the first
    #     click. Both halves of that were wrong, and Marco found them in one
    #     afternoon. Measured with a clean 4 m line and one end acquired,
    #     the cursor sitting exactly on its midpoint:
    #
    #         nothing acquired      midpoint     0.00 cm
    #         one end acquired      from_point   0.65 cm
    #
    #     An alignment LINE was beating a named POINT, so clicking the
    #     midpoint of a wall quietly landed centimetres off and the drawing
    #     came out skewed — reported twice before it was caught («del medio
    #     de la línea quiero dibujar una línea, dibujo sale desfasada»; «sí
    #     agarra, solo cuando termina el dibujo está desfasado»). His own
    #     case was 2.8 cm on a 4 m wall.
    #
    #     Down here it sits below endpoint, midpoint, intersection and
    #     origin, and above the soft axis cue — and it no longer cares
    #     whether an operation is under way, which is the other half: the
    #     reference used to die at the first click, exactly when he needed
    #     it to hold a rectangle's length to a corner.
    #
    #     Marco, 2026-09-18, drawing a step along a wall: «en la segunda
    #     esquina está la referencia pero al momento de jalar el rectángulo
    #     se pierde la referencia». Measured with the wall's far corner
    #     acquired and the cursor pulled out to give the step its depth:
    #
    #         before the first click   from_point   x = 6.430   (the corner)
    #         rectangle under way      none         x = 6.370   (drifted)
    #
    #     The machinery was built and working; it was locked away at exactly
    #     the moment he needed it. Push/pull modellers offer it mid-operation too.
    #
    #     It is a SECOND call rather than an unlocked condition on 5d,
    #     because 5d sits above the named points: opening it in place would
    #     have let an alignment line outrank a midpoint or the origin, and
    #     that precedence is not ours to spend. Down here it competes only
    #     with the soft axis cue below — the weakest rule there is.

    if line_inferences and axis_engaged is None:

        if acquired_points:
            tp = _two_point_snap(
                acquired_points, candidate_world, cx, cy, world_to_pixel,
                threshold_px, is_occluded,
            )
            if tp is not None:
                return tp
        if acquired_point is not None:
            # On a face, the point stays on the face (see the function and
            # ``on_plane`` up top) — except for a tool that is not a planar
            # shape, whose 'level with' reads the cursor ray in 3D (#1095).
            fp = _first_point_from_point(
                acquired_point, candidate_world, cx, cy, world_to_pixel,
                threshold_px, is_occluded, plane_normal=on_plane,
                project_onto_line=project_onto_line, free_3d=free_3d,
            )
            if fp is not None:
                return fp
            # 8e. …and level with / in line with it ACROSS this face: the
            #     axis PLANE through the point cut by the face (Rafael's
            #     window on the next wall, 2026-09-16, 02:20). Only on a
            #     face — the bare ground is not a wall to run a line along.
            if on_plane is not None:
                ip = _in_plane_with_point_snap(
                    acquired_point, candidate_world, on_plane, cx, cy,
                    world_to_pixel, threshold_px, is_occluded,
                )
                if ip is not None:
                    return ip

                # 9. The axis that took command, then the soft cue. The *magnetic* axis
    #    (Move, Line, Tape, issue #31) used to be resolved only here, under
    #    every derived inference - which is exactly what issue #42 was: the axis
    #    could not take command because it only ever spoke last, and an off-line
    #    intersection eight pixels away outranked it. The derived linear
    #    inferences are silenced while an axis is engaged (the guards above), so
    #    arriving here with one engaged means it LOCKS - projected onto the axis
    #    as the arrow lock does. Point inferences were left untouched, so a
    #    vertex, a 'through point' or the origin fetched with the mouse still
    #    wins.
    if axis_engaged is not None:
        locked = project_onto_line(start_point, _AXIS_VECTORS[axis_engaged])
        # 'From point' ALONG the engaged axis is the one derived inference
        # the magnet keeps: its foot is ON the axis line, so it cannot pull
        # the point off it (what #42 and #140 silenced the others for).
        # Without it a line drawn along green toward the red axis slid past
        # the origin's line-up, and could not end exactly on the red axis
        # unless the arrow lock was on (upstream #1418).
        if line_inferences:
            run = locked - start_point
            if run.length() > 1e-6:
                fp = _from_point_snap(
                    scene, start_point, run, cx, cy, world_to_pixel,
                    threshold_px, is_occluded, extra_point=acquired_point,
                    line_dir=run.normalized(),
                )
                if fp is not None:
                    fp.lock_color = AXIS_COLORS[axis_engaged]
                    return fp
        return SnapResult(
            locked, "axis", AXIS_COLORS[axis_engaged], axis=axis_engaged,
        )
    #    The cue kept below is the visual-only fallback: the cursor is near an
    #    axis and the marker says so, but nothing is projected and nothing is
    #    locked.
    if line_start is not None and allow_axis:
        inferred = _detect_axis_alignment(
            line_start, candidate_world, inference_angle_deg
        )
        if axis_in_plane:
            # …and the cue itself obeys the plane: left unfiltered it lit
            # the blue axis on a face whose plane cannot hold it, and the
            # Shift lock captures whatever cue is showing (#545).
            inferred = _axis_in_plane(inferred, work_plane_normal)
        if inferred is not None:
            return SnapResult(
                candidate_world, "axis_inference", AXIS_COLORS[inferred],
                axis=inferred,
            )

    # 10. On-face: the cursor hovers a face with nothing closer to snap to.
    #     The candidate already lies on that face's plane (the work plane).
    #     ``face_context`` is what the ScreenTip says about it — the rest of
    #     the model while a group / component is open ("outside").
    if face_under_cursor:
        return SnapResult(candidate_world, "on_face", COLOR_ON_FACE,
                          context=face_context)

    return SnapResult(candidate_world, "none", COLOR_NONE)

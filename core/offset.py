# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Robust polygon offset: slide the edges, let the arrangement sort out the
mess, keep the regions that are really there.

:func:`core.topology.offset_loop` slides every edge and re-intersects the
neighbours. That is exact while the result stays a polygon with the same
corners — and gives up the moment one of them disappears, because it has no
way to DROP a segment. On a real drawing that happens constantly: Marco's
paved slab, whose boundary carries the segmented bite the round plaza takes
out of it, refused any inward offset past 2.65 cm (Plaza Yanque,
2026-09-10). The classic offset does it, because it removes what
collapses.

The recipe here is the standard one, and it costs little because the hard
part already exists: :mod:`core.arrangement` computes every intersection,
traces the minimal faces and tells outer loops from holes.

1. Slide each edge by ``d``.
2. Join the corners with a MITRE, bevelling when the spike runs past the
   mitre limit (a very sharp corner would otherwise throw its offset corner
   arbitrarily far).
3. Hand the segments to the arrangement, which resolves the crossings the
   collapses create.
4. Keep the regions whose interior really sits ``d`` inside the original —
   the pockets that closed produce regions that fail this and drop out.

Step 4 is the whole trick, and it is one signed distance per region.
"""
from __future__ import annotations

from typing import Optional

from core import doc_model

from ingetrazo_core import Vec3 as QVector3D

#: How far past the offset corner a mitre may reach before it is bevelled,
#: as a multiple of the distance. Clipper's default is 2; the same value
#: keeps a 20° spike honest without chamfering ordinary corners.
MITRE_LIMIT = 2.0

#: The kept region must sit this fraction of ``d`` inside — not the full ``d``,
#: because the arrangement's interior point is a triangle centroid, not the
#: deepest point, and a thin valid sliver would fail an exact test.
_KEEP = 0.75


def _dist_to_boundary(p: QVector3D, loop: list) -> float:
    best = float("inf")
    count = len(loop)
    for i in range(count):
        a = loop[i]
        ab = loop[(i + 1) % count] - a
        length2 = ab.lengthSquared()
        if length2 < 1e-18:
            best = min(best, (p - a).length())
            continue
        t = QVector3D.dotProduct(p - a, ab) / length2
        t = 0.0 if t < 0.0 else (1.0 if t > 1.0 else t)
        best = min(best, (p - (a + ab * t)).length())
    return best


def _inside(p: QVector3D, loop: list, normal: QVector3D) -> bool:
    from core.arrangement import _point_in_polygon, plane_basis
    u, w = plane_basis(normal)
    origin = loop[0]

    def proj(q):
        rel = q - origin
        return (QVector3D.dotProduct(rel, u), QVector3D.dotProduct(rel, w))

    return _point_in_polygon(proj(p), [proj(q) for q in loop])


def interior_point(loop: list, normal: QVector3D) -> Optional[QVector3D]:
    """A point genuinely inside ``loop``: the centroid of its biggest
    triangle. The average of the vertices is not one — for a concave loop it
    can land outside the loop itself."""
    if len(loop) < 3:
        return None
    from core.triangulate import triangulate
    try:
        tris = triangulate(list(loop), [], normal)
    except Exception:                    # noqa: BLE001 — geometry, not flow
        return None
    best = None
    best_area = 0.0
    for a, b, c in tris:
        area = QVector3D.crossProduct(b - a, c - a).lengthSquared()
        if area > best_area:
            best_area = area
            best = (a + b + c) / 3.0
    return best


def _offset_segments(loop: list, normal: QVector3D, d: float) -> list:
    """The slid edges, joined corner to corner (mitre, or bevel past the
    limit). Returns ``[(a, b), …]`` ready for the arrangement."""
    from core.topology import _offset_line_intersection
    lines = []
    count = len(loop)
    for i in range(count):
        a = loop[i]
        b = loop[(i + 1) % count]
        edge = b - a
        if edge.length() < 1e-9:
            continue                     # a zero-length edge is not an edge
        direction = edge.normalized()
        push = QVector3D.crossProduct(normal, direction).normalized() * d
        lines.append([a + push, b + push, direction])
    total = len(lines)
    if total < 3:
        return []
    # Two passes on purpose. Mitring as we emit would fix the LAST corner
    # onto the first line after its segment had already gone out with the
    # un-mitred start, leaving the loop open by one corner — the arrangement
    # then prunes the spur and traces nothing at all.
    reach = MITRE_LIMIT * abs(d) + 1e-9
    corners: list = []
    for i in range(total):
        _start, end, direction = lines[i]
        nxt = lines[(i + 1) % total]
        corner = _offset_line_intersection(end, direction, nxt[0], nxt[2],
                                           normal)
        corners.append(corner if corner is not None
                       and (corner - end).length() <= reach else None)
    segments = []
    for i in range(total):
        start, end, _direction = lines[i]
        back = corners[(i - 1) % total]
        here = corners[i]
        a = QVector3D(back) if back is not None else QVector3D(start)
        b = QVector3D(here) if here is not None else QVector3D(end)
        segments.append((a, b))
        if here is None:
            # Bevel across the gap to the next line's own start.
            nxt_start = lines[(i + 1) % total][0]
            segments.append((QVector3D(end), QVector3D(nxt_start)))
    return segments


def offset_regions(loop, normal, d):
    if not doc_model.ENABLED:
        return _reference_offset_regions(loop, normal, d)
    return _native_regions(loop, normal, d)


def _native_regions(loop, normal, distance):
    from core.arrangement import planar_arrangement
    from core.store_mesh import _hypot
    from ingetrazo_core import MeshStore
    unit = normal.normalized()
    if len(loop) < 3 or abs(distance) < 1e-9 or unit.lengthSquared() < .5:
        return []
    points = [p.toTuple() for p in loop]
    pairs = MeshStore.offset_segments(points, normal.toTuple(), distance, _hypot(), True)
    segments = [(QVector3D(*a), QVector3D(*b)) for a, b in pairs]
    try:
        regions = planar_arrangement(segments, loop[0], unit)
    except Exception:
        return []
    keep = MeshStore.keep_offset_outlines(points, [[p.toTuple() for p in outer] for outer, _ in regions],
                                          normal.toTuple(), distance, _hypot())
    return [region for region, kept in zip(regions, keep) if kept]


def _reference_offset_regions(loop: list, normal: QVector3D, d: float) -> list:
    """Offset ``loop`` by ``d`` (positive inward) and return the regions that
    survive, as ``[(outer, [holes]), …]``.

    Empty when the offset consumes the whole polygon — which is a real
    answer, not a failure: 10 cm inward of a 20 cm kerb is nothing.
    """
    if len(loop) < 3 or abs(d) < 1e-9:
        return []
    from core.arrangement import planar_arrangement
    unit = normal.normalized()
    if unit.lengthSquared() < 0.5:
        return []
    segments = _offset_segments(loop, unit, d)
    if not segments:
        return []
    try:
        faces = planar_arrangement(segments, loop[0], unit)
    except Exception:                    # noqa: BLE001 — geometry, not flow
        return []
    kept = []
    for outer, holes in faces:
        point = interior_point(outer, unit)
        if point is None:
            continue
        # Signed depth: positive inside the ORIGINAL, negative outside. One
        # test covers both directions — inward wants d of clearance, outward
        # allows straying |d| past the boundary.
        depth = _dist_to_boundary(point, loop)
        if not _inside(point, loop, unit):
            depth = -depth
        if depth < d * _KEEP if d > 0 else depth < d:
            continue
        kept.append((outer, holes))
    return kept


def store_offset_regions(store, face_key, loop, normal, distance, hypot):
    """Slide and classify the store face using the drawing kernel's planar arrangement."""
    from core.arrangement import planar_arrangement
    if len(loop) < 3 or abs(distance) < 1e-9:
        return []
    pairs = store.offset_outline(face_key, normal.toTuple(), distance, hypot)
    segments = [(QVector3D(*a), QVector3D(*b)) for a, b in pairs]
    try:
        faces = planar_arrangement(segments, loop[0], normal.normalized())
    except Exception:
        return []
    keep = store.keep_offset_regions(face_key, [[p.toTuple() for p in outer]
                                                for outer, _ in faces],
                                     normal.toTuple(), distance, hypot)
    return [face for face, ok in zip(faces, keep) if ok]


def offset_chain(points, normal, d):
    if not doc_model.ENABLED:
        return _reference_offset_chain(points, normal, d)
    from core.store_mesh import _hypot
    from ingetrazo_core import MeshStore
    segments = MeshStore.offset_segments([p.toTuple() for p in points], normal.toTuple(), d, _hypot(), False)
    if segments:
        return [QVector3D(*segments[0][0]), *(QVector3D(*end) for _, end in segments)]
    return None


def _reference_offset_chain(points: list, normal: QVector3D, d: float) -> Optional[list]:
    """Offset an OPEN polyline ``points`` (coplanar, ``normal`` its plane) by
    ``d`` — the signed distance along ``cross(normal, direction)`` of each
    segment, so the sign picks the side. Consecutive slid segments are
    re-intersected (mitre), or bevelled past :data:`MITRE_LIMIT`; the two
    free ends are not extended. Returns the new points, or ``None`` when
    fewer than two distinct points remain (@pacaeiro, issue #40: «Click a
    face, or connected edges, to offset» promised edges too)."""
    from core.topology import _offset_line_intersection
    n = normal.normalized()
    lines = []
    for a, b in zip(points, points[1:]):
        edge = b - a
        if edge.length() < 1e-9:
            continue
        direction = edge.normalized()
        push = QVector3D.crossProduct(n, direction).normalized() * d
        lines.append([a + push, b + push, direction])
    if not lines:
        return None
    reach = MITRE_LIMIT * abs(d) + 1e-9
    out = [QVector3D(lines[0][0])]
    for i in range(len(lines) - 1):
        _start, end, direction = lines[i]
        nxt = lines[i + 1]
        corner = _offset_line_intersection(end, direction, nxt[0], nxt[2], n)
        if corner is not None and (corner - end).length() <= reach:
            out.append(QVector3D(corner))
        else:
            out.append(QVector3D(end))          # bevel across the gap
            out.append(QVector3D(nxt[0]))
    out.append(QVector3D(lines[-1][1]))
    return out if len(out) >= 2 else None


class _OffsetFace:
    """The topology step inside the tool's snapshot compound."""

    def __init__(self, face, faces, attrs):
        self.face = face
        self.faces = faces
        self.attrs = attrs

    def do(self, scene):
        from core.store_mesh import FaceView
        from core.topology import _maximal_holes
        regions = []
        for outer, *holes in self.faces:
            if len(holes) > 1:
                holes = _maximal_holes(holes)
            regions.append([[p.toTuple() for p in loop] for loop in [outer, *holes]])
        mesh = scene.mesh
        keys, self.change = mesh._store.offset_face(self.face.key, regions)
        mesh._dirty()
        for k in keys:
            FaceView(mesh, k).attrs.update(self.attrs)
        scene.selection.discard(self.face)
        scene.version += 1


class OffsetHoleCollision(ValueError):
    """An inward offset crosses a pre-existing opening."""


def build_offset_face(mesh, face, distance):
    """Plan one face offset by view or store face key. None means the outline collapsed."""
    from core.history import AddFaceCommand, DeleteFaceCommand, SnapshotCompound
    from core.store_face_edit import StoreFaceCompound
    from core.store_mesh import FaceView, _store_of, _hypot
    from core.topology import offset_loop
    store = _store_of(mesh)
    if isinstance(face, int):
        if store is None or not store.is_live(face):
            raise KeyError(face)
        face = FaceView(mesh, face)
    outline = face.vertices
    normal = face.normal()
    if store is not None and (h := _hypot()):
        regions = store_offset_regions(store, face.key, outline,
                                       normal, distance, h)
    else:
        regions = offset_regions(outline, normal, distance)
    if not regions:
        # Safety net: the exact slide-and-intersect path still runs when
        # the arrangement finds nothing, so a bug here can only lose the
        # new cases, never the ones that already worked.
        fallback = offset_loop(outline, normal, distance)
        if fallback is not None:
            regions = [(fallback, [])]
    if not regions:
        return None
    # Inward: the original boundary is the outer ring and each surviving
    # region is a hole in it — plural, because a shape pinched at the
    # waist really does come apart (a dumbbell with a 60 cm neck splits
    # in two at 40 cm). Outward: the biggest region is the ring and the
    # original is its single hole.
    attrs = dict(getattr(face, "attrs", None) or {})
    # The face's own holes go back with it (#537): a ring on top of
    # four walls came back as a full square over the room otherwise.
    own_holes = [list(h) for h in
                 (getattr(face, "holes", None) or [])]
    if doc_model.ENABLED:
        from ingetrazo_core import MeshStore
        def rows(ring):
            return [p.toTuple() for p in ring]
        layout = MeshStore.offset_layout(rows(outline), [rows(hole) for hole in own_holes],
                                         [rows(loop) for loop, _ in regions], normal.toTuple(), distance, _hypot())
        if layout is None:
            raise OffsetHoleCollision
        faces = [[[QVector3D(*p) for p in ring] for ring in region] for region in layout]
    else:
        faces = _reference_layout(face, store, outline, own_holes, regions, normal, distance, _hypot())
    if store is not None:
        commands = [_OffsetFace(face, faces, attrs)]
    else:
        commands = [DeleteFaceCommand(face)]
        commands.extend(AddFaceCommand(rings[0], auto=False, holes=rings[1:],
                                       attrs=dict(attrs)) for rings in faces)
    # Curve ids only when the offset kept the source's shape one-to-one:
    # with the boundary re-arranged there is no run to mirror.
    if len(regions) == 1 and len(regions[0][0]) == len(outline):
        commands.extend(_curve_tags(mesh, outline, regions[0][0]))
    # Snapshot undo: Delete/Add compose fine forward, but the hole edges
    # the ring creates don't reverse cleanly command-by-command (they
    # leaked on undo). One snapshot reverses exactly.
    return StoreFaceCompound(commands) if store is not None else SnapshotCompound(commands)


def _reference_layout(face, store, outline, own_holes, regions, normal, distance, h):
    if distance > 0:
        outer = outline
        inners = [loop for loop, _holes in regions]
        if store is not None and h:
            targets = store.offset_hole_targets(face.key, [[p.toTuple() for p in r]
                                                           for r in inners], normal.toTuple(), h)
            if targets is None:
                raise OffsetHoleCollision
            inner_holes, strip_holes = [[] for _ in inners], []
            for hole, target in zip(own_holes, targets):
                (strip_holes if target < 0 else inner_holes[target]).append(hole)
        else:
            placed = _place_holes(own_holes, inners, normal)
            if placed is None:
                raise OffsetHoleCollision
            inner_holes, strip_holes = placed
    else:
        outer = max((loop for loop, _holes in regions),
                    key=lambda lp: _loop_extent(lp))
        inners = [outline]
        inner_holes, strip_holes = [own_holes], []
    faces = [[list(outer), *inners, *strip_holes]]
    faces.extend([list(loop), *holes] for loop, holes in zip(inners, inner_holes))
    return faces


def _curve_tags(mesh, loop, off: list[QVector3D]) -> list:
    from core.history import TagCurveCommand

    n = len(loop)
    ids: list = []
    for i in range(n):
        va = mesh.vertex_at(loop[i])
        vb = mesh.vertex_at(loop[(i + 1) % n])
        e = mesh.find_edge(va, vb) if va is not None and vb is not None \
            else None
        ids.append(e.curve if e is not None else None)
    if doc_model.ENABLED:
        from ingetrazo_core import outline_curve_runs
        return [TagCurveCommand([off[j] for j in run], closed=closed)
                for run, closed in outline_curve_runs(ids, False)]
    if ids[0] is not None and all(c == ids[0] for c in ids):
        return [TagCurveCommand(list(off), closed=True)]  # full circle
    tags = []
    for k in range(n):
        if ids[k] is None or ids[(k - 1) % n] == ids[k]:
            continue                                       # not a run start
        pts, j = [off[k]], k
        while ids[j] == ids[k]:
            pts.append(off[(j + 1) % n])
            j = (j + 1) % n
        tags.append(TagCurveCommand(pts, closed=False))
    return tags


def _place_holes(holes: list, regions: list, normal: QVector3D):
    """Where each of a face's ``holes`` goes after an inward offset:
    ``(per_region, strip)`` — the holes inside each of ``regions`` (in
    order), and those left in the strip between the outline and the
    regions. ``None`` when a hole crosses or touches a region's outline:
    the offset reaches into the hole, and no face drawn from these loops
    would be right (#537)."""
    from core.arrangement import _point_in_polygon, plane_basis
    u, w = plane_basis(normal)
    origin = regions[0][0] if regions else QVector3D()

    def flat(loop):
        return [(QVector3D.dotProduct(p - origin, u),
                 QVector3D.dotProduct(p - origin, w)) for p in loop]

    flat_regions = [flat(r) for r in regions]
    per_region = [[] for _ in regions]
    strip = []
    for hole in holes:
        fh = flat(hole)
        if any(_outlines_meet(fh, fr) for fr in flat_regions):
            return None
        # Outlines that do not meet are nested or apart: one vertex says
        # which. A region inside the hole means the offset went past it.
        for i, fr in enumerate(flat_regions):
            if _point_in_polygon(fh[0], fr):
                per_region[i].append(list(hole))
                break
            if _point_in_polygon(fr[0], fh):
                return None
        else:
            strip.append(list(hole))
    return per_region, strip


def _outlines_meet(a: list, b: list, eps: float = 1e-6) -> bool:
    """Whether two closed 2D outlines cross, touch or overlap anywhere."""
    def orient(p, q, r):
        v = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
        return 0 if abs(v) < eps else (1 if v > 0 else -1)

    def on_seg(p, q, r):
        return (min(p[0], r[0]) - eps <= q[0] <= max(p[0], r[0]) + eps
                and min(p[1], r[1]) - eps <= q[1] <= max(p[1], r[1]) + eps)

    for i in range(len(a)):
        p1, p2 = a[i], a[(i + 1) % len(a)]
        for j in range(len(b)):
            q1, q2 = b[j], b[(j + 1) % len(b)]
            o1, o2 = orient(p1, p2, q1), orient(p1, p2, q2)
            o3, o4 = orient(q1, q2, p1), orient(q1, q2, p2)
            if o1 != o2 and o3 != o4:
                return True
            if (o1 == 0 and on_seg(p1, q1, p2)
                    or o2 == 0 and on_seg(p1, q2, p2)
                    or o3 == 0 and on_seg(q1, p1, q2)
                    or o4 == 0 and on_seg(q1, p2, q2)):
                return True
    return False


def _loop_extent(loop) -> float:
    """A cheap size for picking the biggest region: the bounding box's
    diagonal. Area would need the plane; this only has to rank."""
    xs = [p.x() for p in loop]
    ys = [p.y() for p in loop]
    zs = [p.z() for p in loop]
    return ((max(xs) - min(xs)) ** 2 + (max(ys) - min(ys)) ** 2
            + (max(zs) - min(zs)) ** 2)

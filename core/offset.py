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

from ingetrazo_core import Vec3 as QVector3D

#: How far past the offset corner a mitre may reach before it is bevelled,
#: as a multiple of the distance. Clipper's default is 2; the same value
#: keeps a 20° spike honest without chamfering ordinary corners.
MITRE_LIMIT = 2.0


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


def offset_regions(loop, normal, d):
    """Return surviving offset regions in arrangement order, positive inward."""
    from core.arrangement import planar_arrangement
    from core.store_mesh import _hypot
    from ingetrazo_core import MeshStore
    unit = normal.normalized()
    if len(loop) < 3 or abs(d) < 1e-9 or unit.lengthSquared() < .5:
        return []
    points = [p.toTuple() for p in loop]
    pairs = MeshStore.offset_segments(points, normal.toTuple(), d, _hypot(), True)
    segments = [(QVector3D(*a), QVector3D(*b)) for a, b in pairs]
    try:
        regions = planar_arrangement(segments, loop[0], unit)
    except Exception:
        return []
    keep = MeshStore.keep_offset_outlines(points, [[p.toTuple() for p in outer] for outer, _ in regions],
                                          normal.toTuple(), d, _hypot())
    return [region for region, kept in zip(regions, keep) if kept]


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
    from core.store_mesh import _hypot
    from ingetrazo_core import MeshStore
    segments = MeshStore.offset_segments([p.toTuple() for p in points], normal.toTuple(), d, _hypot(), False)
    if segments:
        return [QVector3D(*segments[0][0]), *(QVector3D(*end) for _, end in segments)]
    return None


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
    from ingetrazo_core import MeshStore

    def rows(ring):
        return [p.toTuple() for p in ring]

    layout = MeshStore.offset_layout(rows(outline), [rows(hole) for hole in own_holes],
                                     [rows(loop) for loop, _ in regions], normal.toTuple(), distance, _hypot())
    if layout is None:
        raise OffsetHoleCollision
    faces = [[[QVector3D(*p) for p in ring] for ring in region] for region in layout]
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
    from ingetrazo_core import outline_curve_runs
    return [TagCurveCommand([off[j] for j in run], closed=closed)
            for run, closed in outline_curve_runs(ids, False)]

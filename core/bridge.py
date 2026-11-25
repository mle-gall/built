# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Bridge (issue #614): side faces between two profiles.

A profile is a face (its outline, and its holes), a closed loop of edges or
an open chain of them. Two selected profiles are joined:

* face to face — the faces stay as the two caps, so two loose rectangles
  become a closed box;
* face to line — the line is walked there and back as a flat loop, so a
  rectangle and a ridge line become a hip roof;
* line to line — a ruled surface between the two chains.

Points are paired one to one when both profiles have as many; otherwise by
the fraction of the length walked along each, which gives quads where the
two meet and triangles in between. Closed profiles are first turned and, if
need be, reversed so the bridge does not twist. A side that is not flat is
split along its shorter diagonal, softened so it still reads as one surface.

Headless: :func:`profiles_from_selection` reads the selection and
:func:`bridge` builds the faces; the menu command is a thin shell over them.
"""
from __future__ import annotations

from ingetrazo_core import Vec3 as QVector3D

from ingetrazo_core import align_chain, align_loop, bridge_centre as _centre, bridge_pairs as _pairs, bridge_plan as plan, run_stitch, strips
from core.mesh import Edge, Face, turn_face_over
from core.orient import orient_outward
from core.sweep import _CURVE_FACET_COS, _wall, order_path_edges
from core.topology import _key

_EPS = 1e-9
#: A quad is flat when its fourth corner is off the plane of the other three
#: by less than this fraction of its size.
_FLAT = 1e-5


class BridgeError(ValueError):
    """The selection cannot be bridged; the message says why (English
    source text, translated by the caller)."""


class Profile:
    """One side of a bridge: ``loops`` of points (a face's outline then its
    holes, or a single chain), ``closed`` or not, and the ``face`` it came
    from (kept as a cap) when it is one."""

    def __init__(self, loops, closed: bool, face=None) -> None:
        self.loops = [list(lp) for lp in loops]
        self.closed = closed
        self.face = face

    @property
    def points(self) -> list:
        return [p for lp in self.loops for p in lp]


# ---------------------------------------------------------------- selection

def profiles_from_selection(mesh, selection) -> list[Profile]:
    """The two profiles a selection holds, or :class:`BridgeError`.

    Selected faces count as faces; the selected edges that are not the
    outline of a selected face (picking a face by double click selects its
    edges too) are chained into lines or closed loops."""
    # Sets of ids, not ``in mesh.faces``: those are lists, and checking each
    # selected element against them made a big selection quadratic.
    mesh_faces = {f.key for f in mesh.faces}
    mesh_edges = {e.key for e in mesh.edges}
    faces = [e for e in selection
             if isinstance(e, Face) and e.key in mesh_faces]
    if len(faces) > 2:
        raise BridgeError("Select two faces, a face and a line, or two "
                          "lines to bridge")
    face_ids = {f.key for f in faces}
    edges = [e for e in selection
             if isinstance(e, Edge) and e.key in mesh_edges
             and not any(f.key in face_ids for f in e.faces)]
    profiles = [Profile([f.vertices] + f.holes, True, f) for f in faces]
    chains = _connected(edges, most=2 - len(faces))
    if chains is None:
        raise BridgeError("Select two faces, a face and a line, or two "
                          "lines to bridge")
    for chain in chains:
        got = order_path_edges(chain)
        if got is None:
            raise BridgeError("Each line to bridge must be one chain "
                              "of edges, without branches")
        points, closed = got
        if closed and len(points) > 1 and \
                (points[0] - points[-1]).length() < _EPS:
            points = points[:-1]
        profiles.append(Profile([points], closed))
    if len(profiles) != 2:
        raise BridgeError("Select two faces, a face and a line, or two "
                          "lines to bridge")
    return profiles


def _connected(edges, most: int | None = None) -> list[list] | None:
    """The selected edges split into the pieces they form; ``None`` as soon
    as there are more than ``most`` of them (the right-click menu asks on
    every click, and a selection of thousands of loose edges must not be
    walked and sorted only to say no)."""
    left = set(edges)
    by_vertex: dict = {}
    for e in edges:
        by_vertex.setdefault(e.v0, []).append(e)
        by_vertex.setdefault(e.v1, []).append(e)
    pieces = []
    while left:
        seed = left.pop()
        piece, stack = [seed], [seed]
        while stack:
            e = stack.pop()
            for v in (e.v0, e.v1):
                for n in by_vertex[v]:
                    if n in left:
                        left.remove(n)
                        piece.append(n)
                        stack.append(n)
        pieces.append(piece)
        if most is not None and len(pieces) > most:
            return None
    # Stable order: the order the user's selection set happens to give
    # must not decide which profile is "first".
    pieces.sort(key=lambda p: min(_key(v.position) for e in p
                                  for v in (e.v0, e.v1)))
    return pieces


# ---------------------------------------------------------------- pairing

def _flat(quad) -> bool:
    a, b, c, d = quad
    n = QVector3D.crossProduct(b - a, c - a)
    if n.length() < _EPS:
        n = QVector3D.crossProduct(c - a, d - a)
    if n.length() < _EPS:
        return True
    size = max((p - a).length() for p in quad)
    return abs(QVector3D.dotProduct(d - a, n.normalized())) <= _FLAT * size


def _polygons(poly) -> tuple[list, tuple | None]:
    """The faces one side polygon makes — a non-flat quad split along its
    shorter diagonal — and that diagonal (to soften), if any."""
    poly = _wall(poly)
    if poly is None:
        return [], None
    if len(poly) < 4 or _flat(poly):
        return [poly], None
    a, b, c, d = poly
    if (c - a).length() <= (d - b).length():
        return [[a, b, c], [a, c, d]], (a, c)
    return [[a, b, d], [b, c, d]], (b, d)


def bridge(mesh, profiles) -> list[Face]:
    """Add the side faces between two profiles to ``mesh``; the faces among
    them stay, as caps. Returns the new faces. No undo bookkeeping: the
    caller snapshots."""
    polys = plan(profiles)
    before = {f.key for f in mesh.faces}
    diagonals = []
    for poly in polys:
        faces, diag = _polygons(poly)
        for f in faces:
            mesh.add_face(f)
        if diag is not None:
            diagonals.append(diag)
    seed = {_key(pt) for poly in polys for pt in poly}
    new = [f for f in mesh.faces if f.key not in before]
    run_stitch(mesh, seed, set(new), coplanar_merge=True)
    new = [f for f in mesh.faces if f.key not in before]
    for a, b in diagonals:
        va, vb = mesh.vertex_at(a), mesh.vertex_at(b)
        e = mesh.find_edge(va, vb) if va and vb else None
        if e is not None:
            e.soft = True
    # Soften the seams of a curved bridge (two circles make a smooth
    # cylinder); real corners stay hard. Same rule as Follow Me's.
    new_ids = {f.key for f in new}
    for e in mesh.edges:
        if e.soft or len(e.faces) != 2 or \
                not all(f.key in new_ids for f in e.faces):
            continue
        d = QVector3D.dotProduct(e.faces[0].normal().normalized(),
                                 e.faces[1].normal().normalized())
        if _CURVE_FACET_COS < d < 0.99995:
            e.soft = True
    if new:
        caps = [pr.face for pr in profiles
                if pr.face is not None and pr.face in mesh.faces]
        follow_neighbours(mesh, new, caps)
        orient_outward(mesh, only=new)
    return new


def _edges_of(mesh, face):
    for lp in (face.loop, *face.hole_loops):
        n = len(lp)
        for i in range(n):
            e = mesh.find_edge(lp[i], lp[(i + 1) % n])
            if e is not None:
                yield e


def _walks(face, va, vb) -> int:
    """+1 when ``face`` walks the edge from ``va`` to ``vb``, -1 the other
    way, 0 when it does not border it."""
    for lp in (face.loop, *face.hole_loops):
        n = len(lp)
        for i in range(n):
            if lp[i] == va and lp[(i + 1) % n] == vb:
                return 1
            if lp[i] == vb and lp[(i + 1) % n] == va:
                return -1
    return 0


def follow_neighbours(mesh, new, caps=()) -> list:
    """Turn the ``new`` faces so each winds like the face it was built
    against: across a shared edge two faces of one surface walk it in
    opposite directions. The bridged faces (``caps``) lead, then the new
    faces already settled, then any other face already in the mesh; a face
    that was there before is never turned. Returns the faces turned.

    Only a closed solid has an outside for :func:`orient_outward` to find;
    on an open surface (the usual model: walls, a roof, a terrain) the
    sides came out wound by the order the profiles happened to be walked,
    against their neighbours half the time (issue #1043)."""
    from collections import deque
    new_ids = {f.key for f in new}
    settled: set = set()
    turned = []

    def spread(queue) -> None:
        while queue:
            g = queue.popleft()
            for e in _edges_of(mesh, g):
                dg = _walks(g, e.v0, e.v1)
                if dg == 0:
                    continue
                for f in e.faces:
                    if f.key not in new_ids or f.key in settled:
                        continue
                    df = _walks(f, e.v0, e.v1)
                    if df == dg:              # same way: f is turned over
                        turn_face_over(f)
                        turned.append(f)
                    settled.add(f.key)
                    queue.append(f)

    spread(deque(caps))
    if len(settled) < len(new_ids):
        others = []
        seen = set()
        for f in new:
            if f.key in settled:
                continue
            for e in _edges_of(mesh, f):
                for g in e.faces:
                    if g.key not in new_ids and g.key not in seen:
                        seen.add(g.key)
                        others.append(g)
        spread(deque(others))
    return turned

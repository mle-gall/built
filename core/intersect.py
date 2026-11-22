# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Intersect Faces (Edit ▸ Intersect Faces ▸ With Model / With
Selection / With Context): new edges wherever faces cross.

Nothing has to be a solid — this is the classic way to cut shapes
out of one another: intersect, then erase what is not wanted. The edges
land in the context being edited (outside the groups when the model is
the context, which is what makes them easy to separate), and they split
the faces of that context they run across.

The geometry: two planar faces cross along the line common to their two
planes; that line is clipped by each polygon (outer ring and holes, the
even-odd rule) and what both keep is the new edge. Coplanar faces do not
cross — nothing is drawn for them.
"""
from __future__ import annotations
import math

from PySide6.QtGui import QVector3D

from core.history import CompoundCommand
from core.mesh import element_identity
from core.triangulate import plane_axes

#: Below this a crossing is a point, not an edge.
_MIN_LEN = 1e-5


class _Poly:
    """A face in world space, ready for crossing tests."""

    __slots__ = ("rings", "n", "d", "lo", "hi", "u", "v", "rings2d")

    def __init__(self, rings: list, normal: QVector3D) -> None:
        self.rings = rings
        self.n = normal.normalized()
        self.d = QVector3D.dotProduct(self.n, rings[0][0])
        xs = [p.x() for p in rings[0]]
        ys = [p.y() for p in rings[0]]
        zs = [p.z() for p in rings[0]]
        self.lo = (min(xs), min(ys), min(zs))
        self.hi = (max(xs), max(ys), max(zs))
        self.u, self.v = plane_axes(self.n)
        self.rings2d = [[(QVector3D.dotProduct(p, self.u),
                          QVector3D.dotProduct(p, self.v)) for p in ring]
                        for ring in rings]


def world_polys(face, xform=None) -> _Poly | None:
    """``face`` as a world-space polygon (through ``xform`` for a group)."""
    def w(p):
        return xform.map(p) if xform is not None else QVector3D(p)
    outer = [w(p) for p in face.vertices]
    if len(outer) < 3:
        return None
    holes = [[w(p) for p in h] for h in face.holes]
    n = _newell(outer)
    if n.length() < 1e-12:
        return None
    return _Poly([outer, *holes], n)


def _newell(pts) -> QVector3D:
    x = y = z = 0.0
    for i, a in enumerate(pts):
        b = pts[(i + 1) % len(pts)]
        x += (a.y() - b.y()) * (a.z() + b.z())
        y += (a.z() - b.z()) * (a.x() + b.x())
        z += (a.x() - b.x()) * (a.y() + b.y())
    return QVector3D(x, y, z)


def _boxes_touch(a: _Poly, b: _Poly, eps: float = 1e-6) -> bool:
    return all(a.lo[i] <= b.hi[i] + eps and b.lo[i] <= a.hi[i] + eps
               for i in range(3))


def _intervals(poly: _Poly, origin: QVector3D, direction: QVector3D) -> list:
    """Where the line ``origin + t·direction`` (lying in ``poly``'s plane)
    is inside the polygon: sorted ``(t0, t1)`` pairs, even-odd over every
    ring."""
    ox = QVector3D.dotProduct(origin, poly.u)
    oy = QVector3D.dotProduct(origin, poly.v)
    dx = QVector3D.dotProduct(direction, poly.u)
    dy = QVector3D.dotProduct(direction, poly.v)
    ts = []
    for ring in poly.rings2d:
        n = len(ring)
        for i in range(n):
            ax, ay = ring[i]
            bx, by = ring[(i + 1) % n]
            # which side of the line each end is on (half-open: a vertex ON
            # the line counts as the positive side, so it is crossed once)
            sa = (ax - ox) * dy - (ay - oy) * dx
            sb = (bx - ox) * dy - (by - oy) * dx
            if (sa >= 0) == (sb >= 0):
                continue
            k = sa / (sa - sb)
            px, py = ax + (bx - ax) * k, ay + (by - ay) * k
            den = dx * dx + dy * dy
            ts.append(((px - ox) * dx + (py - oy) * dy) / den)
    ts.sort()
    return [(ts[i], ts[i + 1]) for i in range(0, len(ts) - 1, 2)]


def _overlap(xs: list, ys: list) -> list:
    out, i, j = [], 0, 0
    while i < len(xs) and j < len(ys):
        lo = max(xs[i][0], ys[j][0])
        hi = min(xs[i][1], ys[j][1])
        if hi > lo:
            out.append((lo, hi))
        if xs[i][1] < ys[j][1]:
            i += 1
        else:
            j += 1
    return out


def cross(a: _Poly, b: _Poly) -> list:
    """The segments where faces ``a`` and ``b`` cross (none when their
    planes are parallel)."""
    direction = QVector3D.crossProduct(a.n, b.n)
    if direction.length() < 1e-9:
        return []
    direction = direction.normalized()
    # A point on both planes: the one nearest the origin.
    n1, n2 = a.n, b.n
    d1, d2 = a.d, b.d
    c = QVector3D.dotProduct(n1, n2)
    den = 1.0 - c * c
    if den < 1e-12:
        # Nearly parallel: QVector3D works in float32, so c rounded to
        # exactly 1 and this divided by zero (seen on a real model while
        # testing #541). No well-defined crossing line there.
        return []
    k1 = (d1 - d2 * c) / den
    k2 = (d2 - d1 * c) / den
    origin = n1 * k1 + n2 * k2
    segs = []
    for t0, t1 in _overlap(_intervals(a, origin, direction),
                           _intervals(b, origin, direction)):
        p, q = origin + direction * t0, origin + direction * t1
        if (q - p).length() > _MIN_LEN:
            segs.append((p, q))
    return segs


def _boxes(polys: list):
    import numpy as np
    lo = np.array([p.lo for p in polys], dtype=np.float64).reshape(-1, 3)
    hi = np.array([p.hi for p in polys], dtype=np.float64).reshape(-1, 3)
    return lo, hi


#: Box pairs tested per NumPy block (rows × columns), to bound the memory.
_PAIR_BLOCK = 2_000_000


def _touching(lo_a, hi_a, lo_b, hi_b, same: bool, eps: float = 1e-6):
    """Index pairs ``(i, j)`` whose boxes touch — ``_boxes_touch`` for every
    pair at once, in blocks of rows (``same``: j > i only)."""
    import numpy as np
    # The Rust engine (core._native) answers the same pairs in the same
    # order: the same test, with a grid that skips the far boxes.
    from core._native import native
    if native is not None:
        ii, jj = native.touching_boxes(
            *(np.ascontiguousarray(x, dtype=np.float64)
              for x in (lo_a, hi_a, lo_b, hi_b)), bool(same), float(eps))
        yield from zip(ii.tolist(), jj.tolist())
        return
    rows = max(1, _PAIR_BLOCK // max(1, len(lo_b)))
    for r0 in range(0, len(lo_a), rows):
        r1 = min(len(lo_a), r0 + rows)
        ok = np.all((lo_a[r0:r1, None, :] <= hi_b[None, :, :] + eps)
                    & (lo_b[None, :, :] <= hi_a[r0:r1, None, :] + eps), axis=2)
        if same:
            ok &= np.arange(len(lo_b))[None, :] > np.arange(r0, r1)[:, None]
        ii, jj = np.nonzero(ok)
        yield from zip((ii + r0).tolist(), jj.tolist())


def intersect(polys_a: list, polys_b: list, same: bool = False,
              boxes_a=None, boxes_b=None) -> list:
    """Every crossing between a face of ``polys_a`` and one of ``polys_b``
    (``same``: the two lists are one, each pair once). The box test runs
    in NumPy blocks: one Python test per pair took minutes over a whole
    model. ``boxes_a``/``boxes_b``: the ``_boxes`` of each list, when the
    caller already has them."""
    out = []
    if not polys_a or not polys_b:
        return out
    lo_a, hi_a = boxes_a if boxes_a is not None else _boxes(polys_a)
    if same:
        lo_b, hi_b = lo_a, hi_a
        ia = ib = None
    else:
        import numpy as np
        lo_b, hi_b = boxes_b if boxes_b is not None else _boxes(polys_b)
        # Only the faces that reach the OTHER side's whole box can cross
        # it: a big object (a pavement under a thousand others) is cut
        # down to the few faces near each neighbour before the pair test.
        eps = 1e-6
        ia = np.nonzero(np.all((lo_a <= hi_b.max(axis=0) + eps)
                               & (hi_a >= lo_b.min(axis=0) - eps), axis=1))[0]
        if not len(ia):
            return out
        lo_a, hi_a = lo_a[ia], hi_a[ia]
        ib = np.nonzero(np.all((lo_b <= hi_a.max(axis=0) + eps)
                               & (hi_b >= lo_a.min(axis=0) - eps), axis=1))[0]
        if not len(ib):
            return out
        lo_b, hi_b = lo_b[ib], hi_b[ib]
    for i, j in _touching(lo_a, hi_a, lo_b, hi_b, same):
        if ia is not None:
            i, j = int(ia[i]), int(ib[j])
        a, b = polys_a[i], polys_b[j]
        if a is not b:
            out.extend(cross(a, b))
    return _dedupe(out)


def _dedupe(segs: list, tol: float = 1e-6) -> list:
    seen = set()
    out = []
    for p, q in segs:
        k1 = tuple(round(c / tol) for c in (p.x(), p.y(), p.z()))
        k2 = tuple(round(c / tol) for c in (q.x(), q.y(), q.z()))
        key = (k1, k2) if k1 <= k2 else (k2, k1)
        if key in seen:
            continue
        seen.add(key)
        out.append((p, q))
    return out


# ---- What takes part ----------------------------------------------------------

def polys_of(entities) -> list:
    """World polygons for a mix of loose faces and groups/components."""
    from core.group import Group, iter_placements
    from core.mesh import Face
    out = []
    for e in entities:
        if isinstance(e, Face):
            p = world_polys(e)
            if p is not None:
                out.append(p)
        elif isinstance(e, Group):
            for g, xf in iter_placements(e):
                for f in g.mesh.faces:
                    p = world_polys(f, xf)
                    if p is not None:
                        out.append(p)
    return out


WITH_MODEL, WITH_SELECTION, WITH_CONTEXT = "model", "selection", "context"


def _placement_rings(group):
    """Apply the scene graph's composed Qt transform to each prototype face."""
    from core.group import iter_placements
    for placement, matrix in iter_placements(group):
        for face in placement.mesh.faces:
            yield [[(matrix.map(p) if matrix is not None else p).toTuple() for p in ring]
                   for ring in [face.vertices, *face.holes]]


def segments_for(scene, mode: str, selection=None) -> list:
    """The crossing segments Intersect Faces would add, for ``mode``:

    * **model** — the selection against everything it touches (the whole
      context and every group in it),
    * **selection** — only the selected things against each other,
    * **context** — the selection against the rest of the context being
      edited (inside a group: its own geometry)."""
    from core.group import Group
    from core.mesh import Face
    entities = scene.selection if selection is None else selection
    sel = [e for e in entities if isinstance(e, (Face, Group))]
    if not sel:
        return []
    mesh = scene.mesh
    if hasattr(mesh, "_store"):
        from core.store_mesh import _hypot
        h = _hypot()
        if h:
            keys, placements = [], []
            for entity in sel:
                if isinstance(entity, Group):
                    placements.extend((len(keys), rings) for rings in _placement_rings(entity))
                else:
                    keys.append(entity.key)
            others = []
            if mode == WITH_MODEL:
                kids = scene.edit_group.children if scene.edit_group is not None else scene.groups
                selected_groups = {e for e in sel if isinstance(e, Group)}
                others = [rings for g in kids if g not in selected_groups
                          for rings in _placement_rings(g)]
            pairs = mesh._store.intersection_segments(keys, mode, h, placements, others)
            return [(QVector3D(*a), QVector3D(*b)) for a, b in pairs]
    mine = polys_of(sel)
    if mode == WITH_SELECTION:
        return intersect(mine, mine, same=True)
    selected = set(map(element_identity, sel))
    rest = [f for f in scene.mesh.faces if element_identity(f) not in selected]
    if mode == WITH_MODEL:
        owner = scene.edit_group
        kids = (getattr(owner, "children", None) or []) if owner is not None \
            else scene.groups
        rest += [g for g in kids if element_identity(g) not in selected]
    others = polys_of(rest)
    if mode == WITH_MODEL:
        # The selection is part of the model: Select All + With Model is the
        # usual way to cut everything against everything, and taking the
        # selection out of «the rest» left nothing to cut against (#541).
        return _dedupe(intersect(mine, others)
                       + intersect(mine, mine, same=True))
    return intersect(mine, others)


def _partition_faces(mesh, segments):
    """Weld the lines, then partition each original face in its own plane."""
    from core import scan_index
    from core.edits import _ScanEdges
    from core.mesh import edge_flags, stamp_edge_flags
    from core.topology import plan_edge_split, _strictly_inside_2d

    lines = {}
    for a, b in segments:
        sc = scan_index.scan(mesh)
        pieces, cuts = (_ScanEdges(sc).split_plan(a, b) if sc is not None
                        else plan_edge_split(mesh.edges, a, b))
        for edge, point in cuts.items():
            flags = edge_flags(edge)
            was_cut_line = lines.pop(edge.key, None) is not None
            for half in mesh.split_edge(edge, point):
                stamp_edge_flags(half, flags)
                if was_cut_line:
                    lines[half.key] = half
        for p, q in pieces:
            edge = mesh.add_edge(p, q)
            lines[edge.key] = edge

    # Collect all lines before replacing any face. One cut can be a chord
    # on one operand and part of a closed interior contour on the other.
    sc = scan_index.scan(mesh)
    cuts_by_face = {}
    polys = {}
    for edge in lines.values():
        a, b = edge.a, edge.b
        length = (b - a).length()
        direction = (b - a).normalized()
        bounds = list(zip(a.toTuple(), b.toTuple()))
        box = (tuple(min(v) - 1e-4 for v in bounds), tuple(max(v) + 1e-4 for v in bounds))
        faces = sc.faces(sc.faces_meeting(box)) if sc is not None else mesh.faces
        for face in faces:
            if face.key not in polys:
                polys[face.key] = world_polys(face)
            poly = polys[face.key]
            if poly is None or any(abs(QVector3D.dotProduct(p - poly.rings[0][0], poly.n))
                                   > 1e-4 for p in (a, b)):
                continue
            for lo, hi in _overlap([(0., length)], _intervals(poly, a, direction)):
                mid = a + direction * ((lo + hi) / 2)
                xy = (QVector3D.dotProduct(mid, poly.u), QVector3D.dotProduct(mid, poly.v))
                if hi - lo < _MIN_LEN or not _strictly_inside_2d(xy, poly.rings2d[0]):
                    continue
                cut = (a if lo == 0 else a + direction * lo,
                       b if hi == length else a + direction * hi)
                cuts_by_face.setdefault(face, []).append(cut)

    for face, cuts in cuts_by_face.items():
        _replace_regions(mesh, face, polys[face.key], cuts)


def _replace_regions(mesh, face, poly, cuts):
    from copy import deepcopy
    from core.arrangement import planar_rebuild
    from core.make_faces import _probe
    from core.topology import _strictly_inside_2d
    from core.triangulate import _newell

    boundary = [(a, b) for ring in poly.rings for a, b in zip(ring, ring[1:] + ring[:1])]
    _, regions = planar_rebuild(boundary + cuts, poly.rings[0][0], poly.n)
    # Arrangement also returns a cell for each existing opening.
    covered = []
    for outer, holes in regions:
        if poly.rings2d[1:]:
            mid = _probe(outer, holes, poly.n)
            if mid is None:
                continue
            xy = (QVector3D.dotProduct(mid, poly.u), QVector3D.dotProduct(mid, poly.v))
            if any(_strictly_inside_2d(xy, h) for h in poly.rings2d[1:]):
                continue
        if QVector3D.dotProduct(_newell(outer), poly.n) < 0:
            outer.reverse()
            for hole in holes:
                hole.reverse()
        covered.append((outer, holes))
    if len(covered) > 1:
        attrs, interior = face.attrs, face.interior
        mesh.remove_face(face)
        for outer, holes in covered:
            new = mesh.add_face(outer, holes)
            new.attrs = deepcopy(attrs)
            new.interior = interior


def _intersection_mutation(mesh, segments):
    from core.history import SnapshotMutation
    from core.store_face_edit import StoreFaceMutation

    def apply(scene):
        _partition_faces(mesh, segments)

    cls = StoreFaceMutation if hasattr(mesh, "_store") else SnapshotMutation
    return cls(apply, mesh=mesh)


def build_intersection_edit(scene, segments):
    return _intersection_mutation(scene.mesh, segments)


class _PlacementCuts(CompoundCommand):
    # History protects the open mesh. Each child snapshot protects its
    # own prototype, including a child that fails halfway through.
    def do(self, scene):
        for i, command in enumerate(self.commands):
            try:
                command.do(scene)
            except Exception:
                for attempted in reversed(self.commands[:i + 1]):
                    attempted.undo(scene)
                raise


def _placement_intersections(groups):
    """Intersect different selected placements, keeping cuts in local space."""
    from itertools import combinations
    from core.group import iter_placements
    from core.mesh import _KEY_SCALE

    owners = []
    for root in groups:
        for group, xf in iter_placements(root):
            inverse = None
            if xf is not None:
                inverse, ok = xf.inverted()
                if not ok:
                    raise ValueError("singular placement transform")
            polys = [p for f in group.mesh.faces if (p := world_polys(f, xf)) is not None]
            owners.append((group.mesh, inverse, polys))
    local = {}
    world = []
    for left, right in combinations(owners, 2):
        segments = intersect(left[2], right[2])
        world.extend(segments)
        for mesh, inverse, _ in (left, right):
            cuts = local.setdefault(mesh, [])
            for p, q in segments:
                if inverse is not None:
                    p, q = inverse.map(p), inverse.map(q)
                if (q - p).length() >= 1.0 / _KEY_SCALE:
                    cuts.append((p, q))
    commands = [_intersection_mutation(mesh, _dedupe(cuts))
                for mesh, cuts in local.items() if cuts]
    return len(_dedupe(world)), _PlacementCuts(commands) if commands else None


def build_intersect_faces(scene, mode, face_keys=None, placements=None):
    """Plan an intersection by face keys, or from the current selection including groups.

    Return (segment count, snapshot command). A sub-weld segment collapses
    to one vertex and cannot be installed as an edge.
    """
    from core.mesh import _KEY_SCALE
    from core.group import Group
    selection = list(scene.selection) if placements is None else placements
    if mode == "placements" or (mode == WITH_SELECTION and face_keys is None
                                and selection and all(isinstance(g, Group) for g in selection)):
        if face_keys is not None or not all(isinstance(g, Group) for g in selection):
            raise ValueError("placements mode requires groups or components")
        return _placement_intersections(selection)
    if face_keys is None:
        pairs = segments_for(scene, mode)
    else:
        from core.protocol.keys import elements
        by_key = elements(scene.mesh, "face", face_keys)
        faces = [by_key[k] for k in face_keys]
        pairs = segments_for(scene, mode, faces)
    segments = [(p, q) for p, q in pairs if (q - p).length() >= 1.0 / _KEY_SCALE]
    return len(segments), build_intersection_edit(scene, segments) if segments else None

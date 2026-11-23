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

from ingetrazo_core import Vec3 as QVector3D

from core.history import CompoundCommand
from core.mesh import element_identity


class _Poly:
    """A face in world space, ready for crossing tests."""

    __slots__ = ("rings", "native", "n", "lo", "hi")

    def __init__(self, rings, native):
        self.rings = rings
        self.native = native
        self.n = QVector3D(*native.normal)
        self.lo, self.hi = native.bounds


def world_polys(face, xform=None) -> _Poly | None:
    """``face`` as a world-space polygon (through ``xform`` for a group)."""
    def w(p):
        return xform.map(p) if xform is not None else QVector3D(p)
    outer = [w(p) for p in face.vertices]
    if len(outer) < 3:
        return None
    holes = [[w(p) for p in h] for h in face.holes]
    from core.store_mesh import _hypot
    from ingetrazo_core import IntersectionPolygon
    rings = [outer, *holes]
    native = IntersectionPolygon.from_rings([[p.toTuple() for p in ring] for ring in rings], _hypot())
    return _Poly(rings, native) if native is not None else None


def _boxes_touch(a: _Poly, b: _Poly, eps: float = 1e-6) -> bool:
    return all(a.lo[i] <= b.hi[i] + eps and b.lo[i] <= a.hi[i] + eps
               for i in range(3))


def cross(a: _Poly, b: _Poly) -> list:
    """The segments where faces ``a`` and ``b`` cross (none when their
    planes are parallel)."""
    return [(QVector3D(*p), QVector3D(*q)) for p, q in a.native.cross(b.native)]


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
    """Install the split graph, then replace its crossed faces in mesh order."""
    from core import scan_index
    from ingetrazo_core import partition_intersections
    from core.edits import _ScanEdges
    from core.mesh import edge_flags, stamp_edge_flags
    from core.topology import plan_edge_split

    def split_plan(a, b):
        sc = scan_index.scan(mesh)
        return (_ScanEdges(sc).split_plan(a, b) if sc is not None
                else plan_edge_split(mesh.edges, a, b))

    partition_intersections(mesh, segments, split_plan, edge_flags, stamp_edge_flags,
                            scan_index.scan, world_polys, _replace_regions)


def _replace_regions(mesh, face, poly, cuts):
    from copy import deepcopy
    from core.arrangement import planar_arrangement

    boundary = [(a, b) for ring in poly.rings for a, b in zip(ring, ring[1:] + ring[:1])]
    regions = planar_arrangement(boundary + cuts, poly.rings[0][0], poly.n)
    poly.native.replace_regions(mesh, face, regions, deepcopy)


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
    journal_parts = True

    # History protects the open mesh. Each child snapshot protects its
    # own prototype, including a child that fails halfway through.
    def do(self, scene):
        for i, command in enumerate(self.commands):
            try:
                command.do(scene)
            except Exception:
                if not self._journal_geometry:
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

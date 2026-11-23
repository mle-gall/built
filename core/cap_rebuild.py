# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Deterministic plane rebuild for push/pull (the root-fix, "path C").

This is what replaced the fragile post-extrude heal chain — the old
``_extrude_commands`` case tree (extend-wall / notch / strip), the
``cap_boundary_loops`` crack patcher, and the winding-tolerant coplanar merge
on solids — with one deterministic recompute per touched plane:

1. Gather the mesh edges lying on the plane.
2. Run the planar arrangement (:mod:`core.arrangement`) to get every minimal
   bounded region.
3. Classify each region by **which side of the plane holds material**,
   volumetrically: parity ray-casting (the same primitive as
   :func:`core.orient.orient_outward`) from a sample point just off the
   region's interior on each side. Reading the volume keeps the answer
   independent of the order planes are rebuilt in — the overlapping coplanar
   faces a naive extrude leaves mid-cleanup cancel in crossing-parity pairs.
   Material on exactly **one** side → a boundary face (wound toward the empty
   side); on neither → a phantom outside the solid; on both → solid interior
   (e.g. the mouth ring where a pushed-out pane meets its wall — facing it
   would create an internal partition and swallow a window hole).
4. Union each side's regions (dropping the edges interior to the union) into
   the final face loops — outer boundary plus holes.

The headline win: a push that touched a plane just rebuilds that plane's faces
from its edges, instead of patching geometry with a growing tree of special
cases.
"""
from __future__ import annotations

import math
import random
import sys
from typing import Optional

from ingetrazo_core import Vec3 as QVector3D, plane_cycle as _canon_loop

from core.arrangement import (
    _TOL,
    planar_arrangement,
    plane_basis,
)
from core.orient import (
    _face_components,
    _face_triangles,
    _pack_lists,
    ray_parity_outside,
)

# A point this close to the plane (along its normal) counts as on it.
_ON_PLANE = 1e-4


def _snapshot_triangles(mesh, keys):
    from core._native import qt_hypot
    from core.orient import _PackedTris
    positions, tris, owners, _ = mesh._store.snapshot().indexed_surface(qt_hypot(), keys)
    a = positions[tris[:, 0]]
    return _PackedTris(a, positions[tris[:, 1]] - a, positions[tris[:, 2]] - a, owners)


class RebuildCache:
    """Per-face memos shared across one push's plane rebuilds.

    Every plane rebuild scans the whole mesh three ways — "which faces are
    coplanar here", "triangulate everything for the parity set" — and each
    scan recomputed Newell normals and earcut triangulations from scratch. On
    an imported 3k-face barbecue that was 76k normal computations per drag
    frame, dwarfing the parity work it was feeding.

    Safe because the fixpoint loop never mutates a face in place: a rebuild
    *removes* faces and *adds* new ones, so an identity key is either still
    the same geometry or gone. (The trailing ``run_stitch``, which does mutate
    loops, runs after the loop and takes no cache.) Stable face keys cannot be reused by later geometry, so a dead face
    cannot alias a live cache entry."""

    __slots__ = ("tris", "norm", "cent", "token", "_packed", "_comp",
                 "_comp_token", "seed_faces", "_scope", "_scope_token",
                 "dropped")

    def __init__(self) -> None:
        #: Outlines (3D segment pairs) of the strips a rebuild found inside
        #: the solid and dropped (issue #94): the planes beside them must be
        #: looked at again when a seam is left there — see
        #: :func:`edges_along` and ``tools.pushpull._seam_planes_along``.
        self.dropped: list = []
        self.tris: dict = {}
        self.norm: dict = {}
        self.cent: dict = {}
        #: Bumped by the caller whenever a rebuild actually changed the mesh;
        #: the packed parity set and the component map are memoised against it.
        self.token = 0
        self._packed: tuple = (None, None, None)
        self._comp: Optional[dict] = None
        self._comp_token = -1
        #: The operation's own faces. Their components are the only solids a
        #: rebuild may touch, so every whole-mesh scan narrows to them.
        self.seed_faces: set = set()
        self._scope: Optional[tuple] = None
        self._scope_token = -1

    def components(self, mesh) -> dict:
        """Face key → edge-connected component label, recomputed when the mesh
        changes."""
        if self._comp is None or self._comp_token != self.token:
            from core.store_mesh import StoreMesh, FaceView, EdgeView
            if isinstance(mesh, StoreMesh):
                seeds = [f.key for f in self.seed_faces]
                labels, edges = mesh._store.snapshot().cap_scope(seeds)
                self._comp = dict(labels)
                live = {self._comp[k] for k in seeds if k in self._comp}
                self._scope = ([FaceView(mesh, k) for k, c in labels if c in live],
                               [EdgeView(mesh, k) for k in edges]) if live else None
                self._scope_token = self.token
            else:
                self._comp = {f.key: c for f, c in _face_components(mesh).items()}
            self._comp_token = self.token
        return self._comp

    def scope(self, mesh):
        """``(faces, edges)`` of the solids the operation is working in, or
        ``None`` when it has no seed to narrow by (then callers scan the whole
        mesh, as before).

        A plane rebuild reads the mesh three ways per plane — which edges lie
        on the plane, which faces are coplanar with it, which faces cover a
        region — and each scan walked every edge and face of the group. Inside
        an imported barbecue that is nineteen welded solids re-examined to
        rebuild one brick's planes, twelve planes over. Narrowing is also more
        correct: a *different* solid that happens to be coplanar was feeding
        its edges into this plane's arrangement.

        Computed once per mesh state, not once per plane — that repetition was
        the cost, more than the scans themselves."""
        if not self.seed_faces:
            return None
        if self._scope_token == self.token:
            return self._scope
        comp = self.components(mesh)
        if self._scope_token == self.token:
            return self._scope
        live = {comp[f.key] for f in self.seed_faces if f.key in comp}
        if not live:
            self._scope, self._scope_token = None, self.token
            return None
        faces = [f for f in mesh.faces if comp.get(f.key) in live]
        fset = set(faces)
        # An edge belongs to the scope when a face of it does. A faceless edge
        # is plane debris a rebuild may still need to re-trace, so it stays.
        edges = [e for e in mesh.edges
                 if not e.faces or any(f in fset for f in e.faces)]
        self._scope = (faces, edges)
        self._scope_token = self.token
        return self._scope

    def triangles(self, f):
        t = self.tris.get(f.key)
        if t is None:
            t = f.triangulate()
            self.tris[f.key] = t
        return t

    def normal(self, f):
        n = self.norm.get(f.key)
        if n is None:
            n = f.normal().normalized()
            self.norm[f.key] = n
        return n

    def centroid(self, f):
        c = self.cent.get(f.key)
        if c is None:
            c = f.centroid()
            self.cent[f.key] = c
        return c

    def packed(self, mesh, shadowed, want=None):
        """The packed parity triangles of the counted faces (boundary, not
        shadowed, and — when ``want`` is given — in one of those components),
        memoised until the mesh changes."""
        key = (frozenset(f.key for f in shadowed) if shadowed else None,
               frozenset(want) if want is not None else None)
        tok, k, val = self._packed
        if tok == self.token and k == key and val is not None:
            return val
        comp = self.components(mesh) if want is not None else None
        from core._native import qt_hypot
        from core.store_mesh import StoreMesh
        selected = (f for f in mesh.faces if f not in shadowed
                    and (want is None or comp.get(f.key) in want))
        if isinstance(mesh, StoreMesh) and qt_hypot():
            val = _snapshot_triangles(mesh, [f.key for f in selected])
        else:
            val = _pack_lists([self.triangles(f) for f in selected if not f.interior])
        self._packed = (self.token, key, val)
        return val


def _on_plane(pos: QVector3D, origin: QVector3D, normal: QVector3D) -> bool:
    return abs(QVector3D.dotProduct(pos - origin, normal)) < _ON_PLANE


# Normal offset of the material sample points: clear of the welding tolerance
# (1e-4) yet far smaller than any feature the engine can represent.
_SAMPLE_OFF = 1e-3


def _region_test_point(outer_xy, holes_xy):
    """A point inside ``outer_xy`` but outside every hole — so the classifier
    samples the region's *material*, not a void. The plain centroid of an annular
    region (a cap with a skylight) falls in the hole, which would misread it as
    outside the solid; this nudges in from an outer edge instead when needed."""
    from ingetrazo_core import plane_region_point
    return plane_region_point(outer_xy, holes_xy)


def _union_outline(solid_regions_xy, keep_segs=None) -> list:
    """Union a set of 2D regions (each ``(outer, [holes])`` wound as the
    arrangement winds them — outer CCW, holes CW) into ``[(outer, [holes])]``,
    dropping every edge interior to the union (it appears in two solid regions,
    once in each direction, so the directions cancel).

    ``keep_segs`` (2D segments) are **creases**: edges a perpendicular face
    stands on. Union edges *lying on* one never cancel, so the union keeps a
    face boundary there — two roof slabs over a dividing wall stay two faces
    with a visible ridge, as users expect. The test is geometric (midpoint on
    segment): the arrangement splits edges at crossings, so an endpoint-pair
    match would lose the crease on the split-off pieces."""
    from ingetrazo_core import trace_plane_outline
    return _nest_outlines(trace_plane_outline(solid_regions_xy, keep_segs or ()))


def _nest_outlines(loops):
    from ingetrazo_core import plane_nesting
    return [(loops[i], [loops[j] for j in holes])
            for i, holes in plane_nesting(loops, False)]


def rebuild_plane(mesh, origin: QVector3D, normal: QVector3D,
                  fresh=(), keep_mode: bool = False,
                  removing: bool = True, op=None,
                  cache: Optional[RebuildCache] = None) -> Optional[list]:
    """Recompute the solid *boundary* faces of ``mesh`` on the plane
    ``(origin, normal)``.

    A region of the plane carries a face exactly when material sits on **one**
    side of it: none → it is outside the solid (a phantom); both → it is
    interior. A both-sides region is dropped when fresh (the mouth ring where
    a pushed-out pane meets its wall — facing it would swallow a window hole),
    **kept as a partition** when an existing interior face covers it (the slab
    a Ctrl-push keeps, a wall two rooms share — deliberate structure), and
    **declared boundary by the push** when one of this push's ``fresh`` faces
    covers it: parity can't see a void still walled in by an untrimmed
    neighbouring plane, but the naive build's deterministic winding already
    says which side emptied (the fresh normal points to the empty side) — that
    is what keeps the per-plane rebuild order-free next to partitions.

    Returns ``[(outer_loop, [hole_loops], is_partition), …]`` with every loop a
    list of 3D ``QVector3D`` on the plane, boundary loops *wound outward*
    (toward the empty side) — or ``None`` when the plane carries too little to
    face (fewer than three usable edges). Pure: it reads the mesh, it does not
    mutate it.
    """
    normal = normal.normalized()
    u, v = plane_basis(normal)
    # The solids this operation may touch (see RebuildCache.scope). Falling
    # back to the whole mesh keeps direct callers — and any op with no seed —
    # behaving exactly as before.
    scope = cache.scope(mesh) if cache is not None else None
    scope_faces = mesh.faces if scope is None else scope[0]
    scope_edges = mesh.edges if scope is None else scope[1]

    def to2d(p):
        d = p - origin
        return (QVector3D.dotProduct(d, u), QVector3D.dotProduct(d, v))

    from core._native import qt_hypot
    from core.store_mesh import EdgeView, StoreMesh
    native = isinstance(mesh, StoreMesh) and qt_hypot()
    if native:
        rows = mesh._store.snapshot().cap_segments(
            origin.toTuple(), normal.toTuple(), _ON_PLANE, qt_hypot(),
            None if scope is None else [e.key for e in scope_edges])
        plane_edges = [EdgeView(mesh, k) for k, _ in rows]
        segs = [(QVector3D(*a), QVector3D(*b)) for _, (a, b) in rows]
    else:
        plane_edges = [e for e in scope_edges if _on_plane(e.a, origin, normal)
                       and _on_plane(e.b, origin, normal)]
        segs = [(e.a, e.b) for e in plane_edges]
    if len(segs) < 3:
        return None

    regions_3d = planar_arrangement(segs, origin, normal)
    if not regions_3d:
        return None

    # Material presence per side is volumetric: parity ray-casting from a point
    # just off the region's interior, away from the plane (so the ray never
    # looks back through a crack being capped). It reads the whole mesh — the
    # one source that is consistent mid-cleanup, because the overlapping
    # coplanar faces a naive extrude leaves cancel in crossing-parity pairs.
    # This is what makes the per-plane rebuild independent of plane order.
    # Two exclusions keep the counts honest:
    # - interior partitions are not boundary: a ray crossing one would flip
    #   parity without leaving the solid;
    # - in **keep mode** (Ctrl: the push stacks/divides, it removes nothing),
    #   a fresh face shadowed by a counted old face on its own plane (an
    #   inward Ctrl-stack's tube quad lying on the boundary it overlaps)
    #   double-counts a boundary that *persists* — exclude it. Shadowed by an
    #   interior partition, the quad is itself a partition-to-be (material on
    #   both sides; keep removes nothing) — exclude it too. A consumed-base
    #   push is the opposite: its sweep empties the overlapped region, and the
    #   pair must keep cancelling (two crossings = no boundary) — the original
    #   "cancel in pairs" doctrine — while a quad over an interior partner is
    #   the only crossing left there and must count.
    fresh_set = {f for f in fresh if f in mesh.faces}
    from ingetrazo_core import shadowed_push_faces
    shadowed = (shadowed_push_faces(fresh_set, scope_faces, cache, plane_basis, _ON_PLANE)
                if keep_mode else set())
    # Pack the counted triangles ONCE per plane. Each region's two parity
    # queries used to hand ``ray_parity_outside`` the raw triangle lists,
    # which re-packed the whole mesh per call — on an imported 3k-face
    # barbecue (hundreds of regions per drag frame) the packing alone
    # dominated the push preview.
    #
    # The parity set is also restricted to the **edge-connected component(s)**
    # this plane belongs to — the same domain :func:`core.orient.orient_outward`
    # decides orientation over, so the two agree. Inside/outside is a property
    # of one shell: a disjoint solid elsewhere in the mesh (a group holding
    # nineteen welded bricks) is crossed in and back out by any ray, so it
    # cannot change the verdict — while its *unmarked* interior partitions
    # could, by adding a lone third crossing. A single-component mesh gets the
    # whole mesh, so ordinary drawn geometry is unaffected.
    if cache is None and native:
        keys = [f.key for f in mesh.faces if f not in shadowed] if shadowed else None
        packed = _snapshot_triangles(mesh, keys)
    elif cache is None:
        packed = _pack_lists([t for f, t in _face_triangles(mesh).items()
                              if not f.interior and f not in shadowed])
    else:
        comp = cache.components(mesh)
        want = {comp[f.key] for f in scope_faces
                if f.key in comp and _coplanar_on(f, origin, normal, cache)}
        # A push welding a new wall onto a neighbouring solid must see both.
        want |= {comp[f.key] for f in fresh_set if f.key in comp}
        packed = cache.packed(mesh, shadowed, want or None)
    rng = random.Random(54321)

    # A fresh face *marked interior* (the cap of an inward Ctrl-stack) is a
    # deliberate division, not a boundary declaration — it counts as existing
    # structure instead. What a fresh winding may testify depends on the push:
    # an inward carve empties regions (declarations apply where material reads
    # *both* sides — a void still walled in by an untrimmed plane); an outward
    # push creates material (declarations apply where material reads *neither*
    # side — the sweep envelops old boundary faces that still miscount); a
    # Ctrl push never declares (it removes nothing and its quads are
    # divisions).
    samples = []
    for outer, holes in regions_3d:
        outer_xy = [to2d(p) for p in outer]
        holes_xy = [[to2d(p) for p in h] for h in holes]
        samples.append((outer_xy, holes_xy, _region_test_point(outer_xy, holes_xy)))

    if native:
        coverage = mesh._store.snapshot().cap_coverage(
            origin.toTuple(), normal.toTuple(), [p for _, _, p in samples],
            [f.key for f in fresh_set], _ON_PLANE, qt_hypot(), sys.version_info >= (3, 12),
            None if scope is None else [f.key for f in scope_faces])
    else:
        from ingetrazo_core import object_cap_coverage
        coverage = object_cap_coverage(scope_faces, fresh_set, cache, origin, normal, (u, v),
                                       [p for _, _, p in samples], _ON_PLANE)
    from ingetrazo_core import rebuild_region_faces

    def region_outlines(regions, creases):
        if native:
            loops = mesh._store.snapshot().trace_cap_outlines(regions, creases, _TOL)
            if loops is not None:
                return _nest_outlines(loops)
        return _union_outline(regions, creases)

    return rebuild_region_faces(
        origin, normal, (u, v), samples, coverage, plane_edges, keep_mode, removing,
        op, cache, _SAMPLE_OFF, _ON_PLANE, _TOL / 2, ray_parity_outside, rng,
        packed, region_outlines)


def _coplanar_on(face, origin, normal, cache=None) -> bool:
    fn = face.normal().normalized() if cache is None else cache.normal(face)
    if abs(QVector3D.dotProduct(fn, normal)) <= 0.999:
        return False
    fc = face.centroid() if cache is None else cache.centroid(face)
    return _on_plane(fc, origin, normal)


def _canon_faces(faces_as_loops) -> frozenset:
    """Winding-sensitive, rotation-independent fingerprint of a rebuilt plane."""
    from ingetrazo_core import plane_fingerprint
    return plane_fingerprint(faces_as_loops)


def apply_rebuild(mesh, origin: QVector3D, normal: QVector3D,
                  fresh=(), keep_mode: bool = False,
                  removing: bool = True, prune: bool = True,
                  op=None, cache: Optional[RebuildCache] = None) -> bool:
    """Rebuild one plane of ``mesh`` in place: replace its coplanar faces with the
    deterministic solid faces from :func:`rebuild_plane`, and prune the edges left
    interior to the plane (a dissolved seam) that now border nothing. Returns
    whether anything changed — **False when the plane already matches** the
    rebuilt result, so callers can iterate rebuilds to a fixpoint and know it
    terminates. No-op when the rebuild can't trace the plane (``None``); an
    *empty* rebuild is a real answer — every region is phantom or interior
    (a zero-thickness fin left by a flush collapse) — and removes the plane's
    faces. ``fresh`` (this push's new faces) lets the rebuild resolve regions
    a partition used to cover (see :func:`rebuild_plane`).

    ``prune=False`` keeps the faceless plane edges: inside the fixpoint loop a
    region can be mis-dropped while a neighbouring plane still carries its
    dirty coincidences, and a later round can only re-trace it if its edges
    survive — the caller prunes once the loop converges
    (:func:`prune_plane_debris`).

    The caller snapshots for undo (the push wraps the whole mutation), so this
    keeps no inverse of its own."""
    normal = normal.normalized()
    rebuilt = rebuild_plane(mesh, origin, normal, fresh, keep_mode, removing,
                            op, cache)
    from ingetrazo_core import install_rebuilt_plane
    changed = install_rebuilt_plane(mesh, origin, normal, rebuilt, cache, _ON_PLANE,
                                    _replace_plane_store, _region_test_point, plane_basis)
    if changed and prune:
        prune_plane_debris(mesh, origin, normal)
    return changed


def _replace_plane_store(mesh, old, rebuilt):
    from core.store_mesh import FaceView, StoreMesh, _fallback
    if isinstance(mesh, StoreMesh):
        from core.topology import _maximal_holes
        caps = []
        for outer, holes, interior in rebuilt:
            rings = [outer, *(_maximal_holes(holes) if len(holes) > 1 else holes)]
            caps.append(([[p.toTuple() for p in ring] for ring in rings], interior))
        try:
            keys, _change = mesh._store.replace_plane_faces([f.key for f in old], caps)
        except _fallback():
            pass  # Replay through remove_face/add_face below.
        else:
            mesh._dirty()
            return [FaceView(mesh, k) for k in keys]
    return None


def prune_plane_debris(mesh, origin: QVector3D, normal: QVector3D) -> None:
    """Drop edges that lie fully on the plane and face nothing — the interior
    seams a union dissolved. Perimeter edges still border a wall, so they keep
    a face and survive. Vertices those edges leave behind with no incidence at
    all go too (they would otherwise haunt snapping)."""
    from core._native import qt_hypot
    from core.store_mesh import StoreMesh
    if isinstance(mesh, StoreMesh) and qt_hypot():
        change = mesh._store.prune_plane_debris(origin.toTuple(), normal.toTuple(),
                                                _ON_PLANE, qt_hypot())
        if any(change.removed.values()):
            mesh._dirty()
        return
    from ingetrazo_core import prune_object_plane
    prune_object_plane(mesh, origin, normal, _ON_PLANE)


def plane_key(point: QVector3D, normal: QVector3D):
    """Canonical ``(key, origin, normal)`` for the plane through ``point`` with
    ``normal``: the normal's largest-magnitude component is made positive, so a
    face and its flip map to one plane while the two sides of a slab stay
    distinct (their offsets ``d`` differ)."""
    n = normal.normalized()
    comps = (n.x(), n.y(), n.z())
    if comps[max(range(3), key=lambda i: abs(comps[i]))] < 0:
        n = -n
    d = QVector3D.dotProduct(n, point)
    return (round(n.x(), 3), round(n.y(), 3), round(n.z(), 3), round(d, 3)), point, n


def crack_planes(mesh, near=None) -> list:
    """Representative ``(origin, normal)`` for each distinct plane that carries a
    crack — a boundary edge bordering a single face. These are the planes a
    nested push can leave unclosed, the ones :func:`apply_rebuild` should
    recompute (the deterministic replacement for ``cap_boundary_loops``).

    ``near`` — a box ``(x0, y0, z0, x1, y1, z1)`` — keeps only the cracks with
    an end inside it: the ones the operation that box surrounds can have left.
    An imported model is open in many places, and every push re-rebuilt every
    plane with a crack anywhere in the group: 178 planes, minutes, for one
    face of the Plaza (09-10) — and planes far from the push recomputed."""
    seen: dict = {}

    from core.store_mesh import StoreMesh
    from core._native import qt_hypot
    if isinstance(mesh, StoreMesh) and qt_hypot():
        for p, n in mesh._store.snapshot().crack_planes(qt_hypot(), sys.version_info >= (3, 12), near):
            key, origin, normal = plane_key(QVector3D(*p), QVector3D(*n))
            seen.setdefault(key, (origin, normal))
        return list(seen.values())

    def inside(p) -> bool:
        return (near[0] <= p.x() <= near[3] and near[1] <= p.y() <= near[4]
                and near[2] <= p.z() <= near[5])

    from core import push_scans
    edges = push_scans.cracked_edges(mesh, near)   # natively: exactly these
    for e in (mesh.edges if edges is None else edges):
        if len(e.faces) != 1:
            continue
        if near is not None and not (inside(e.a) or inside(e.b)):
            continue
        f = e.faces[0]
        key, origin, n = plane_key(f.centroid(), f.normal())
        if key not in seen:
            seen[key] = (origin, n)
    return list(seen.values())


def edges_along(mesh, segments) -> list:
    """Every edge of ``mesh`` lying along one of ``segments`` — the outline
    of a strip a rebuild dropped as inside the solid (issue #94). The plane
    beside it was rebuilt BEFORE the strip went and kept that rim as a crease
    between two faces that are now coplanar (fuzz prism seed 80); the caller
    dissolves exactly those, and nothing else in the plane is recomputed —
    a full re-rebuild there misread open sheets (the rings' membranes) as
    material and capped a hole."""
    if not segments:
        return []

    from core.store_mesh import EdgeView, StoreMesh
    from core._native import qt_hypot
    if isinstance(mesh, StoreMesh) and qt_hypot():
        lines = [(a.toTuple(), b.toTuple()) for a, b in segments]
        keys = mesh._store.snapshot().edges_along(lines, _ON_PLANE, qt_hypot())
        return [EdgeView(mesh, k) for k in keys]

    def on(p, a, b) -> bool:
        ab = b - a
        L2 = ab.lengthSquared()
        if L2 < 1e-18:
            return (p - a).length() < _ON_PLANE
        t = QVector3D.dotProduct(p - a, ab) / L2
        if t < -1e-9 or t > 1 + 1e-9:
            return False
        return (a + ab * t - p).length() < _ON_PLANE

    return [e for e in mesh.edges
            if any(on(e.a, a, b) and on(e.b, a, b) for a, b in segments)]


# cos²(1e-4 rad): the most two normals may differ to be read as ONE plane
# drawn a hair out of true (#1076: a door 0.2 mm off over 2 m, 7.5e-5).
# An imported panel leaning 5e-4 next to an exact new face is two planes:
# correcting it changed which pushes a broken bench model refused.
_HAIR_COS2 = math.cos(1e-4) ** 2
# The Qt unit-vector dot rounds up to 1 above the midpoint to the preceding
# float32. Preserve that effective aperture for double-precision cap normals.
_STORE_HAIR_COS2 = (1.0 - 2.0 ** -25) ** 2


def _off_plane(face, origin, normal) -> float:
    """The farthest any of ``face``'s vertices sits from the plane."""
    return max(abs(QVector3D.dotProduct(v.position - origin, normal))
               for lp in (face.loop, *face.hole_loops) for v in lp)


def _drops_its_neighbour(f, nf, g) -> bool:
    """Whether the plane read off the op's face ``f`` would leave part of
    its bigger coplanar neighbour ``g`` off the plane, while ``g``'s own
    normal through ``f`` keeps both on it: one plane drawn a hair out of
    true, not two planes (#1076)."""
    half = _ON_PLANE / 2
    gn = g.normal().normalized()
    if QVector3D.dotProduct(nf, gn) ** 2 < _HAIR_COS2:
        return False                    # two planes, not one out of true
    oc = f.centroid()
    if _off_plane(g, oc, nf) <= half:
        return False                    # the op's plane already holds it
    return _off_plane(g, oc, gn) <= half and _off_plane(f, oc, gn) <= half


def seam_planes(mesh, new_faces) -> list:
    """Representative ``(origin, normal)`` for each plane where one of this
    operation's ``new_faces`` ended up coplanar-edge-adjacent to another face —
    the seams and overlaps a naive extrude leaves behind: a strip stacked on an
    existing wall, a phantom quad over a wall about to be notched, a fresh cap
    flush with an old roof, or two fresh quads split at a T-junction. These are
    exactly the planes :func:`apply_rebuild` must recompute so the deterministic
    union dissolves them (the replacement for the winding-tolerant coplanar
    merge of ``run_stitch`` phase 3).

    The plane is read off the op's face unless that would leave part of
    its bigger coplanar neighbour off the plane while the neighbour's own
    normal holds both; then that normal is used (the origin stays on the
    op's face). A push's new faces are small and inherit any tilt of the
    pushed face (a door drawn 1 mm out of plumb sweeps a pocket floor that
    leans 2e-5): read off that floor, the room's plane drifted past the
    on-plane tolerance a few metres away, the far half of its outline fell
    out of the arrangement, and the whole floor went as «not traced» --
    the push refused, more often the bigger the model (#1076)."""
    seen: dict = {}
    from core.store_mesh import StoreMesh
    from core._native import qt_hypot
    if isinstance(mesh, StoreMesh) and qt_hypot():
        rows = mesh._store.snapshot().seam_planes(
            [f.key for f in new_faces], _ON_PLANE, _STORE_HAIR_COS2, qt_hypot(), sys.version_info >= (3, 12))
        for point, normal, correction in rows:
            key, origin, cn = plane_key(QVector3D(*point), QVector3D(*normal))
            if correction is not None and not seen.get(key, (False,))[0]:
                _, _, cn = plane_key(origin, QVector3D(*correction))
                seen[key] = (True, origin, cn)
            else:
                seen.setdefault(key, (False, origin, cn))
        return [(o, n) for _, o, n in seen.values()]
    for f in new_faces:
        if f not in mesh.faces:
            continue
        nf = f.normal().normalized()
        for lp in (f.loop, *f.hole_loops):
            n = len(lp)
            for i in range(n):
                e = mesh.find_edge(lp[i], lp[(i + 1) % n])
                if e is None:
                    continue
                for g in e.faces:
                    if g == f:
                        continue
                    if abs(QVector3D.dotProduct(
                            nf, g.normal().normalized())) > 0.999:
                        key, origin, cn = plane_key(f.centroid(), nf)
                        if key in seen and seen[key][0]:
                            continue        # already corrected from a big face
                        if g.area() > f.area() and _drops_its_neighbour(
                                f, nf, g):
                            # Its normal, through the op's own face.
                            _, origin, cn = plane_key(f.centroid(),
                                                      g.normal())
                            seen[key] = (True, origin, cn)
                        elif key not in seen:
                            seen[key] = (False, origin, cn)
    return [(o, n) for _a, o, n in seen.values()]

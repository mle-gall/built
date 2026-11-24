# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Deterministic plane rebuild (Store::single_plane) — the root-fix "path C".

Given a plane of a solid, recompute its faces from the plane's edges: run the
arrangement, classify each minimal region inside/outside the solid by winding
against outward-oriented wall edges, and union the inside ones. These tests pin
the two things the plain arrangement could not do alone — reject phantom regions
on the *outside* of the solid, and survive concave footprints — plus holes and
the multi-region union.
"""
from __future__ import annotations

from ingetrazo_core import Vec3 as QVector3D

from core.store_mesh import plan_cap_plane as rebuild_plane
from core.mesh import Mesh
from core.orient import orient_outward
from core.scene import Scene
from core.history import History
from tools.pushpull import PushPullTool
from core.topology import classify_push_edge, refine_loop_with_points


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


def _loop_area(loop):
    n = QVector3D(0, 0, 0)
    m = len(loop)
    for i in range(m):
        a, b = loop[i], loop[(i + 1) % m]
        n += QVector3D(
            (a.y() - b.y()) * (a.z() + b.z()),
            (a.z() - b.z()) * (a.x() + b.x()),
            (a.x() - b.x()) * (a.y() + b.y()),
        )
    return 0.5 * n.length()


def _areas(faces):
    """Sorted *net* face areas of a rebuild result (outer minus holes).
    Accepts both ``(outer, holes)`` pairs and the rebuild's
    ``(outer, holes, is_partition)`` triples."""
    return sorted(
        round(_loop_area(item[0]) - sum(_loop_area(h) for h in item[1]), 2)
        for item in faces
    )


# ---- prism cap with an extruded box footprint (a closed solid) -------------

def _prism(h=3.0):
    a, b, c = (-0.2, -2.8), (2.7, -7.2), (4.1, -2.7)
    m = Mesh()
    base = [V(*a, 0), V(*b, 0), V(*c, 0)]
    top = [V(*a, h), V(*b, h), V(*c, h)]
    m.add_face(base)
    m.add_face(top)
    for i in range(3):
        j = (i + 1) % 3
        m.add_face([base[i], base[j], top[j], top[i]])
    orient_outward(m)
    return m


def test_plain_prism_cap_is_one_face():
    m = _prism()
    faces = rebuild_plane(m, V(0, 0, 3), V(0, 0, 1))
    assert len(faces) == 1
    # The triangle area (shoelace of the irregular triangle).
    assert _areas(faces) == [round(_areas([(m.faces[1].vertices, [])])[0], 2)]


# ---- concave L footprint with a phantom region in the notch ----------------

def _L_prism():
    L = [(0, 0), (4, 0), (4, 2), (2, 2), (2, 4), (0, 4)]  # concave
    m = Mesh()
    b = [V(*p, 0) for p in L]
    t = [V(*p, 3) for p in L]
    m.add_face([V(*p, 0) for p in L])
    m.add_face([V(*p, 3) for p in L])
    for i in range(len(L)):
        j = (i + 1) % len(L)
        m.add_face([b[i], b[j], t[j], t[i]])
    orient_outward(m)
    return m


def test_concave_L_cap_rejects_notch_phantom():
    m = _L_prism()
    # Add stray edges closing the missing quadrant (the notch) — they enclose a
    # phantom bounded region *outside* the L that the plain arrangement would
    # face. The classifier must drop it.
    m.add_edge(V(2, 4, 3), V(4, 4, 3))
    m.add_edge(V(4, 4, 3), V(4, 2, 3))
    faces = rebuild_plane(m, V(0, 0, 3), V(0, 0, 1))
    assert _areas(faces) == [12.0]   # the L, not the 4-area notch phantom


# ---- a hole in the cap (a window/skylight) survives -------------------------

def test_cap_with_hole_keeps_the_hole():
    # A 6×6 slab with a 2×2 opening, as a closed solid box-with-shaft is overkill;
    # build the cap plane directly with wall edges around outer + hole, oriented.
    m = Mesh()
    outer = [(0, 0), (6, 0), (6, 6), (0, 6)]
    hole = [(2, 2), (4, 2), (4, 4), (2, 4)]
    ob = [V(*p, 0) for p in outer]
    ot = [V(*p, 3) for p in outer]
    hb = [V(*p, 0) for p in hole]
    ht = [V(*p, 3) for p in hole]
    m.add_face([V(*p, 0) for p in outer], [[V(*p, 0) for p in hole]])  # bottom w/ hole
    m.add_face([V(*p, 3) for p in outer], [[V(*p, 3) for p in hole]])  # top w/ hole
    for i in range(4):
        j = (i + 1) % 4
        m.add_face([ob[i], ob[j], ot[j], ot[i]])          # outer walls
        m.add_face([hb[i], hb[j], ht[j], ht[i]])          # shaft walls
    orient_outward(m)
    faces = rebuild_plane(m, V(0, 0, 3), V(0, 0, 1))
    assert len(faces) == 1
    outer_loop, holes, _interior = faces[0]
    assert len(holes) == 1
    assert _areas(faces) == [32.0]   # 36 − 4


# ---- end to end: a real pushed prism cap matches what the engine commits ----

class _StubVP:
    def __init__(self, scene, history):
        self.scene = scene
        self.history = history

    def set_hover(self, *a):
        pass

    def set_suppressed_faces(self, *a):
        pass

    def update(self):
        pass


def _push(scene, hist, face, dist):
    t = PushPullTool()
    t.base_face = face
    t.extrusion = dist
    t._normal = face.normal()
    t._anchor = face.centroid()
    t._attached, t._prism_cap = t._classify_base(scene)
    t._cap_positions = t._cap_loop_positions(face)
    t._commit(_StubVP(scene, hist))


def _fresh_prism_scene():
    tri = [(-0.2, -2.8), (2.7, -7.2), (4.1, -2.7)]
    sc = Scene()
    h = History(sc)
    sc.mesh.add_face([V(*tri[0]), V(*tri[1]), V(*tri[2])])
    _push(sc, h, sc.mesh.faces[0], 3.0)
    return sc, h


def _walls(sc):
    w = [f for f in sc.mesh.faces if abs(f.normal().z()) < 0.1]
    return sorted(w, key=lambda f: (round(f.centroid().x(), 3),
                                    round(f.centroid().y(), 3)))


import itertools

import pytest

_DISTS = [1.5, 1.0, 2.0]


def _ground_truth_cap_areas(perm, normal):
    sc, h = _fresh_prism_scene()
    for k, wi in enumerate(perm):
        _push(sc, h, _walls(sc)[wi], _DISTS[k])
    o = V(0, 0, 3) if normal.z() > 0 else V(0, 0, 0)
    return sorted(
        round(f.area(), 2)
        for f in sc.mesh.faces
        if abs(QVector3D.dotProduct(f.normal().normalized(), normal)) > 0.999
        and abs(QVector3D.dotProduct(f.centroid() - o, normal)) < 1e-3
    )


@pytest.mark.parametrize("perm", list(itertools.permutations(range(3))))
@pytest.mark.parametrize("normal", [V(0, 0, 1), V(0, 0, -1)])
def test_pushed_prism_cap_rebuild_matches_committed_face(perm, normal):
    """For every push order and both caps: rebuilding the affected plane at the
    messy mid-operation state reproduces the area the engine commits — phantom
    dropped, solid kept, adjacent solid pieces unioned."""
    gt_total = sum(_ground_truth_cap_areas(perm, normal))

    # Mid-operation: the first two pushes commit normally, then only the third's
    # extrude commands run, leaving the pre-heal geometry the rebuild must clean.
    sc, h = _fresh_prism_scene()
    _push(sc, h, _walls(sc)[perm[0]], _DISTS[0])
    _push(sc, h, _walls(sc)[perm[1]], _DISTS[1])
    m = sc.mesh
    face = _walls(sc)[perm[2]]
    d = _DISTS[2]
    fn = face.normal()
    allv = [v.position for v in m.vertices]
    base = refine_loop_with_points(face.vertices, allv)
    top = [v + fn * d for v in base]
    count = len(base)
    kinds = [classify_push_edge(face, base[i], base[(i + 1) % count], sc.faces)
             for i in range(count)]
    attached = all(k != "free" for k, _ in kinds)
    from core.store_mesh import _hypot
    dirty, *_ = m._store.push_shell(face.key, [[p.toTuple() for p in base]],
                                    [[p.toTuple() for p in top]], (fn * d).toTuple(),
                                    attached, None, _hypot())
    m._dirty(dirty)

    o = V(0, 0, 3) if normal.z() > 0 else V(0, 0, 0)
    faces = rebuild_plane(m, o, normal)
    assert round(sum(_areas(faces)), 2) == round(gt_total, 2)


# ---- fixpoint / order independence -------------------------------------------

def test_apply_rebuild_noop_on_stable_plane():
    """A plane already in its rebuilt form reports no change — the property
    that terminates the push's rebuild fixpoint loop."""
    sc, h = _fresh_prism_scene()
    m = sc.mesh
    wall = _walls(sc)[0]
    n = wall.normal().normalized()
    before = len(m.faces)
    assert m.rebuild_cap_plane(wall.centroid(), n) is False
    assert len(m.faces) == before


def test_rebuild_order_independent_on_overhang():
    """The overhang push (cubito flush on an unsplit host) must come out clean
    for *every* seam-plane iteration order — the volumetric (parity ray-cast)
    classification reads the mesh volume, not the half-cleaned plane edges, so
    no order leaves a seam or an internal partition."""
    import itertools
    from ingetrazo_core import Vec3 as QVector3D
    from core.scene import Scene
    from core.history import History
    from tools.pushpull import PushPullTool

    def build():
        X0, X1, XC = 0.0, 4.0, 2.0
        Y0, Y1, YC = 0.0, 6.0, 1.5
        ZT, ZC = 3.0, 5.0
        scene = Scene()
        m = scene.mesh
        m.add_face([V(X0,Y0,0), V(X1,Y0,0), V(X1,Y1,0), V(X0,Y1,0)])
        m.add_face([V(X0,Y0,0), V(X1,Y0,0), V(X1,Y0,ZT), V(X0,Y0,ZT)])
        m.add_face([V(X1,Y0,0), V(X1,Y1,0), V(X1,Y1,ZT), V(X1,Y0,ZT)])
        m.add_face([V(X0,Y1,0), V(X1,Y1,0), V(X1,Y1,ZT), V(X0,Y1,ZT)])
        m.add_face([V(X0,Y0,0), V(X0,Y1,0), V(X0,Y1,ZT), V(X0,Y0,ZT)])
        m.add_face([V(XC,Y0,ZT), V(X1,Y0,ZT), V(X1,Y1,ZT), V(X0,Y1,ZT),
                    V(X0,YC,ZT), V(XC,YC,ZT)])
        m.add_face([V(X0,Y0,ZC), V(XC,Y0,ZC), V(XC,YC,ZC), V(X0,YC,ZC)])
        fw = m.add_face([V(X0,Y0,ZT), V(XC,Y0,ZT), V(XC,Y0,ZC), V(X0,Y0,ZC)])
        m.add_face([V(XC,Y0,ZT), V(XC,YC,ZT), V(XC,YC,ZC), V(XC,Y0,ZC)])
        m.add_face([V(X0,YC,ZT), V(XC,YC,ZT), V(XC,YC,ZC), V(X0,YC,ZC)])
        m.add_face([V(X0,Y0,ZT), V(X0,YC,ZT), V(X0,YC,ZC), V(X0,Y0,ZC)])
        return scene, fw

    class _VP:
        def __init__(self, s):
            self.scene = s
            self.history = History(s)
        def update(self): pass
        def set_hover(self, e): pass
        def set_suppressed_faces(self, f): pass

    # Change the real face-list order seen by the store's seam discovery.
    for perm in [(0,1,2,3), (3,2,1,0), (1,3,0,2), (2,0,3,1)]:
        scene, fwall = build()
        rim = list(scene.mesh.faces)[-4:]
        loops = [f.vertices for f in rim]
        for f in rim:
            scene.mesh.remove_face(f)
        for i in perm:
            added = scene.mesh.add_face(loops[i])
            if rim[i] == fwall:
                fwall = added
        t = PushPullTool()
        t.base_face = fwall
        t.extrusion = -2.0
        t.dragging = True
        t._anchor = fwall.centroid()
        t._normal = fwall.normal()
        t._attached, t._prism_cap = t._classify_base(scene)
        t._cap_positions = t._cap_loop_positions(fwall)
        t._commit(_VP(scene))
        m = scene.mesh
        seams = [e for e in m.edges if len(e.faces) == 2 and
                 QVector3D.dotProduct(e.faces[0].normal().normalized(),
                                      e.faces[1].normal().normalized()) > 0.999]
        assert seams == [], f"seam left with order {perm}"



def test_plane_cycle_keeps_the_first_minimum_and_unbounded_grid_coordinates():
    from ingetrazo_core import plane_cycle as _canon_loop

    # The slit returns to the smallest point twice. Reversing its winding
    # changes the shape fingerprint and choosing the second visit changes its start.
    ring = [V(2, 0), V(-0.00005, 0), V(2e16, 0), V(-0.00005, 0), V(1, 2)]
    cells = [tuple(round(x / 1e-4) for x in p.toTuple()) for p in ring]
    assert _canon_loop(ring) == tuple(cells[1:] + cells[:1])
    assert _canon_loop(ring[::-1]) != _canon_loop(ring)


def test_cap_coverage_counts_a_fresh_face_outside_the_seeded_scope():
    from core.arrangement import plane_basis
    from ingetrazo_core import object_cap_coverage

    mesh = Mesh()
    ring = [V(0, 0), V(4, 0), V(4, 4), V(0, 4)]
    hole = [V(1, 1), V(1, 3), V(3, 3), V(3, 1)]
    old = mesh.add_face(ring, [hole])
    fresh = mesh.add_face(ring[::-1])
    origin, normal = V(0, 0), V(0, 0, 1)
    probes = [(0.5, 0.5), (2.0, 2.0), (10.0, 10.0)]
    assert object_cap_coverage([old], {fresh}, None, origin, normal,
                               plane_basis(normal), probes, 1e-4) == [
        (True, 1, 2), (False, 0, 2), (False, 0, 0)]


def test_collinear_outline_turns_use_the_runtime_grid_key_order():
    from ingetrazo_core import union_plane_regions as _union_outline

    for x in (0.0, float(2**51)):
        a, b, c = (x, 0.), (x + 1, 0.), (x, 1.)
        d, e = (x + 2, 0.), (x + 1, 2.)
        def cell(p):
            return tuple(round(v / 1e-4) for v in p)
        # At a, b and d make exactly the same angle. The original stable
        # angular sort retains the order of their cells in the neighbour set.
        neighbours = set()
        for p in (b, c, d, e):
            neighbours.add(cell(p))
        tied = [k for k in neighbours if k in (cell(b), cell(d))]
        expected = [([a, b, c], []), ([a, d, e], [])] if tied[-1] == cell(b) else [
            ([a, b, c, a, d, e], [])]
        assert _union_outline([([a, b, c], []), ([a, d, e], [])]) == expected

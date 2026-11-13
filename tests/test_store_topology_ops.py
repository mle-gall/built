# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Port step 2 (docs/meshstore-design.md §15): the incidence primitives of
``core/mesh.py`` as store ops, against the Python ``Mesh`` as the oracle.

Random sequences of ``vertex``, ``add_edge``, ``_link_edge``,
``add_face`` (holes, rings of 1 and 2 corners, slits), ``remove_edge``,
``remove_face``, ``relink_edge``, ``relink_face``, ``add_hole``,
``remove_hole``, ``purge_debris``, ``drop_orphan_vertices``,
``relink_vertices``, moves onto other vertices and snapshot / restore run
on a Python mesh and on a store-backed one built from the same crowded
block (points a hair across weld-cell borders). After EVERY op both must
show the same thing: positions, list order of all three kinds, each
vertex's edges, each edge's ends, flags and faces with multiplicity, each
face's rings, attrs and flags, the weld registry and displaced queues,
``registers`` (removed vertices included), ``vertex_at``, the revision
counters, the same exception. Keys are compared through a bijection built
as elements appear: an element keeps its key across remove / relink and
restore, a new element gets a key never seen before.

An op the store does not represent falls back to the Python kernel
(``StoreFallback``): the mesh materialises, the states must still agree,
and the sequence ends there (the Python kernel then answers by set order
where duplicates exist, which two Python runs do not share either)."""
from __future__ import annotations

import os
import random

import numpy as np
import pytest
from PySide6.QtGui import QVector3D as V

import core._native as _native
from core import mesh as M
from core import store_mesh as S
from tests.test_meshstore import build_mesh, crowded_block, store_of_block

mod = _native.available()
pytestmark = pytest.mark.skipif(
    mod is None or not hasattr(mod, "MeshStore")
    or not hasattr(mod.MeshStore, "add_face"),
    reason="native module without the step-2 store ops")


def python_mesh(blk):
    """The block built as Python objects (the store switched off)."""
    was = os.environ.get("INGETRAZO_STORE")
    os.environ["INGETRAZO_STORE"] = "0"
    try:
        return build_mesh(*blk)
    finally:
        if was is None:
            del os.environ["INGETRAZO_STORE"]
        else:
            os.environ["INGETRAZO_STORE"] = was


class Keys:
    """The key bijection between the two engines (store key -> Python)."""

    def __init__(self):
        self.s2p: dict = {}
        self.p2s: dict = {}

    def pair(self, pk, sk):
        a, b = self.p2s.get(pk), self.s2p.get(sk)
        if a is None and b is None:
            self.p2s[pk] = sk
            self.s2p[sk] = pk
            return
        assert a == sk and b == pk, ("key rule broken", pk, sk, a, b)

    def name(self, sk):
        return self.s2p.get(sk, ("?", sk))


def _ref(k, index, mapper):
    i = index.get(k)
    return i if i is not None else ("gone", mapper(k))


def state(m, keys: Keys | None):
    """Everything observable, by list index; an unlisted element by its
    (Python-side) key."""
    mapper = (lambda k: k) if keys is None else keys.name
    Vs, Es, Fs = list(m.vertices), list(m.edges), list(m.faces)
    vid = {v.key: i for i, v in enumerate(Vs)}
    eid = {e.key: i for i, e in enumerate(Es)}
    fid = {f.key: i for i, f in enumerate(Fs)}
    rv = lambda x: _ref(x.key, vid, mapper)        # noqa: E731
    out = dict(
        v=[v.position.toTuple() for v in Vs],
        vedges=[sorted((_ref(e.key, eid, mapper) for e in v.edges), key=str)
                for v in Vs],
        e=[(rv(e.v0), rv(e.v1), bool(e.soft), bool(e.hidden), e.curve,
            e.layer, [_ref(f.key, fid, mapper) for f in e.faces])
           for e in Es],
        f=[([rv(v) for v in f.loop], [[rv(v) for v in h] for h in f.hole_loops],
            sorted(f.attrs.items(), key=str), bool(f.interior)) for f in Fs],
        rev=(m.revision, m._mut_serial),
    )
    if type(m) is S.StoreMesh:
        st = m._store
        cells, ids = st.registry()
        out["reg"] = {tuple(c): i for c, i in zip(cells.tolist(), ids.tolist())}
        out["disp"] = {c: [_ref(k, vid, mapper) for k in q]
                       for c, q in st.displaced_keys().items()}
        st.check()
    else:
        out["reg"] = {c: _ref(v.key, vid, mapper)
                      for c, v in m._registry.items()}
        out["disp"] = {c: [rv(w) for w in q] for c, q in m._displaced.items()}
    out["registers"] = [m.registers(v) for v in Vs]
    return out


def pair_keys(keys, pm, sm):
    for a, b in zip(pm.vertices, sm.vertices):
        keys.pair(a.key, b.key)
    for a, b in zip(pm.edges, sm.edges):
        keys.pair(a.key, b.key)
    for a, b in zip(pm.faces, sm.faces):
        keys.pair(a.key, b.key)


class Run:
    #: Corner counts of the rings add_face draws.
    ring_sizes = (1, 2, 3, 3, 4, 4, 5)

    @staticmethod
    def block(rng, n_points, n_edges, n_faces):
        return crowded_block(rng, n_points=n_points, n_edges=n_edges,
                             n_faces=n_faces)

    def __init__(self, seed, n_points=90, n_edges=60, n_faces=25):
        self.rng = rng = random.Random(seed)
        blk = self.block(rng, n_points, n_edges, n_faces)
        self.pm = python_mesh(blk)
        self.sm = M.Mesh()
        # The weld constants the app gives a store (_weld_args), so the
        # probe distance is Qt's here too (fused on arm64).
        pos, edges, flags, curves, layers, names, rings, sizes, counts = blk
        S.become_store(self.sm, mod.MeshStore.from_block(
            next(M._KEY_NAMESPACES), pos, edges, flags, curves, layers,
            names, rings, sizes, counts, None, **S._weld_args()), [])
        assert type(self.pm) is M.Mesh and type(self.sm) is S.StoreMesh
        self.keys = Keys()
        pair_keys(self.keys, self.pm, self.sm)
        self.base = [[rng.randrange(-4, 4) * 0.5 for _ in range(3)]
                     for _ in range(12)]
        # Removed elements held as history holds them: (python, store).
        self.gone = {"v": [], "e": [], "f": []}
        self.holes = []          # (face pair, loop pair) from add_hole
        self.snaps = []
        # (op count, key) of writes into elements while removed: a restore
        # rolls a store tombstone back with the state, while Python's held
        # object keeps the write (documented deviation, §15).
        self.dead_writes = []
        self.ops = 0
        self.on_store = 0
        self.fallbacks = []
        # Revision counters are compared as deltas from here (the builds
        # took different routes).
        self.rev0 = ((self.pm.revision, self.pm._mut_serial),
                     (self.sm.revision, self.sm._mut_serial))
        self.compare()

    # -- helpers
    def point(self):
        rng = self.rng
        if self.pm.vertices and rng.random() < 0.35:
            p = rng.choice(list(self.pm.vertices)).position
            d = rng.choice([0, 0, 4.9e-5, -5.1e-5, 1e-6])
            return V(p.x() + d, p.y(), p.z() - d)
        b = rng.choice(self.base)
        return V(*[c + rng.choice([0, 4.9e-5, 5.1e-5, -5e-5, 1e-6, 0.25, 1.0])
                   for c in b])

    def pick(self, kind):
        """A live element pair of kind 'v' / 'e' / 'f', by list index."""
        name = {"v": "vertices", "e": "edges", "f": "faces"}[kind]
        pl = getattr(self.pm, name)
        if not pl:
            return None
        i = self.rng.randrange(len(pl))
        return pl[i], getattr(self.sm, name)[i]

    def held(self, kind):
        g = self.gone[kind]
        return self.rng.choice(g) if g else None

    def compare(self):
        pair_keys(self.keys, self.pm, self.sm)
        a = state(self.pm, None)
        b = state(self.sm, self.keys)
        for x, base in ((a, self.rev0[0]), (b, self.rev0[1])):
            x["rev"] = (x["rev"][0] - base[0], x["rev"][1] - base[1])
        assert b == a
        # Removed vertices answer `registers` the same too.
        for pv, sv in self.gone["v"]:
            assert self.sm.registers(sv) == self.pm.registers(pv)
        for _ in range(3):
            p = self.point()
            hp, hs = self.pm.vertex_at(p), self.sm.vertex_at(p)
            assert (hs is None) == (hp is None)
            if hp is not None:
                assert self.keys.name(hs.key) == hp.key

    def both(self, name, pargs, sargs):
        """Run ``name`` on both meshes; the same result or exception."""
        errs = []
        out = []
        for m, args in ((self.pm, pargs), (self.sm, sargs)):
            try:
                out.append(getattr(m, name)(*args))
                errs.append(None)
            except Exception as exc:  # noqa: BLE001 - compared below
                out.append(None)
                errs.append(type(exc))
        assert errs[0] == errs[1], (name, errs)
        self.ops += 1
        if type(self.sm) is S.StoreMesh:
            self.on_store += 1
        return out

    # -- one random op
    def step(self):
        rng, pm, sm = self.rng, self.pm, self.sm
        r = rng.random()
        if r < 0.06:
            p = self.point()
            a, b = self.both("vertex", (p,), (p,))
            if a is not None:
                self.keys.pair(a.key, b.key)
        elif r < 0.14:
            p, q = self.point(), (self.point() if rng.random() < 0.9
                                  else None)
            q = q if q is not None else V(p)
            a, b = self.both("add_edge", (p, q), (p, q))
            if a is not None:
                self.keys.pair(a.key, b.key)
        elif r < 0.19:
            x, y = self.pick("v"), self.pick("v")
            if x and y:
                if rng.random() < 0.1:
                    y = x                          # a self-loop
                a, b = self.both("_link_edge", (x[0], y[0]), (x[1], y[1]))
                self.keys.pair(a.key, b.key)
        elif r < 0.33:
            n = rng.choice(self.ring_sizes)
            ring = [self.point() for _ in range(n)]
            if n >= 3 and rng.random() < 0.2:
                ring = ring[:2] + [ring[1]] + ring[2:]      # a slit corner
            holes = None
            if rng.random() < 0.3:
                holes = [[self.point() for _ in range(rng.choice([3, 4]))]
                         for _ in range(rng.choice([1, 2, 3]))]
            a, b = self.both("add_face", (ring, holes), (ring, holes))
            if a is not None:
                self.keys.pair(a.key, b.key)
                if rng.random() < 0.3:
                    a.attrs["color"] = [0.1, 0.2, 0.3]
                    b.attrs["color"] = [0.1, 0.2, 0.3]
        elif r < 0.42:
            x = self.pick("e") if rng.random() < 0.85 else self.held("e")
            if x:
                if rng.random() < 0.2:
                    if x[0] not in pm.edges:
                        self.dead_writes.append((self.ops, x[0].key))
                    x[0].soft = x[1].soft = True
                self.both("remove_edge", (x[0],), (x[1],))
                if x not in self.gone["e"]:
                    self.gone["e"].append(x)
        elif r < 0.51:
            x = self.pick("f") if rng.random() < 0.85 else self.held("f")
            if x:
                self.both("remove_face", (x[0],), (x[1],))
                if x not in self.gone["f"]:
                    self.gone["f"].append(x)
        elif r < 0.60:
            x = self.held("e") if rng.random() < 0.9 else self.pick("e")
            if x:
                self.both("relink_edge", (x[0],), (x[1],))
                if x in self.gone["e"] and x[0] in pm.edges:
                    self.gone["e"].remove(x)
        elif r < 0.69:
            x = self.held("f") if rng.random() < 0.9 else self.pick("f")
            if x:
                self.both("relink_face", (x[0],), (x[1],))
                if x in self.gone["f"] and x[0] in pm.faces:
                    self.gone["f"].remove(x)
        elif r < 0.74:
            x = self.pick("f")
            if x:
                ring = [self.point() for _ in range(rng.choice([1, 3, 4]))]
                a, b = self.both("add_hole", (x[0], ring), (x[1], ring))
                if a is not None:
                    self.holes.append((x, (a, b)))
        elif r < 0.79:
            if self.holes and rng.random() < 0.6:
                x, (lp, ls) = rng.choice(self.holes)
                # Python's add_hole hands back the list the face holds, so
                # a stitch since may have edited it in place: both engines
                # get the loop as it stands.
                ls = [S.VertexView(self.sm, self.keys.p2s[v.key])
                      if type(self.sm) is S.StoreMesh else v for v in lp]
                if any(k is None for k in ls):
                    return
            else:
                x = self.pick("f")
                if not x or not x[0].hole_loops:
                    return
                i = rng.randrange(len(x[0].hole_loops))
                lp, ls = x[0].hole_loops[i], x[1].hole_loops[i]
            if x[0] not in pm.faces:
                self.dead_writes.append((self.ops, x[0].key))
            self.both("remove_hole", (x[0], lp), (x[1], ls))
        elif r < 0.82:
            a, b = self.both("purge_debris", (), ())
            assert a == b
        elif r < 0.87:
            vs = [self.pick("v") for _ in range(rng.choice([1, 3, 6]))]
            vs = [x for x in vs if x] + ([self.held("v")] if self.gone["v"]
                                         and rng.random() < 0.3 else [])
            before = {v.key for v in pm.vertices}
            a, b = self.both("drop_orphan_vertices", ([x[0] for x in vs],),
                             ([x[1] for x in vs],))
            assert a == b
            for x in vs:
                if x[0].key in before and x[0] not in pm.vertices \
                        and x not in self.gone["v"]:
                    self.gone["v"].append(x)
        elif r < 0.90:
            if self.gone["v"]:
                vs = rng.sample(self.gone["v"],
                                min(len(self.gone["v"]), rng.choice([1, 2])))
                self.both("relink_vertices", ([x[0] for x in vs],),
                          ([x[1] for x in vs],))
                for x in vs:
                    if x[0] in pm.vertices:
                        self.gone["v"].remove(x)
        elif r < 0.95:
            x, y = self.pick("v"), self.pick("v")
            if x and y:
                d = y[0].position - x[0].position
                if rng.random() < 0.5:
                    d = V(rng.choice([0.5, -0.5]), 0, rng.choice([0, 4.9e-5]))
                self.both("move_vertex", (x[0], d), (x[1], d))
        elif r < 0.975:
            sp, ss = pm.capture_state(), sm.capture_state()
            self.snaps.append((sp, ss, {k: list(g) for k, g in
                                        self.gone.items()}, list(self.holes),
                               self.ops))
        elif self.snaps:
            sp, ss, gone, holes, at = rng.choice(self.snaps)
            assert (ss == sm.capture_state()) == (sp == pm.capture_state())
            self.both("restore_state", (sp,), (ss,))
            # Held elements that stayed removed since the snapshot can still
            # be relinked; the rest went with the discarded states.
            written = {k for n, k in self.dead_writes if n >= at}
            self.gone = {k: [x for x in g if x in self.gone[k]
                             and x[0].key not in written]
                         for k, g in gone.items()}
            self.holes = [h for h in holes if h in self.holes]
        else:
            return
        self.compare()


def run(seed, n_ops):
    r = Run(seed)
    for _ in range(n_ops):
        r.step()
        if type(r.sm) is not S.StoreMesh:
            r.fallbacks.append(r.ops)
            break
    return r


@pytest.mark.parametrize("seed", range(16))
def test_random_ops_match_the_python_mesh(seed):
    r = run(seed, 160)
    assert r.ops > 40
    if not r.fallbacks:
        assert r.on_store == r.ops


def test_most_sequences_never_leave_the_store():
    runs = [run(1000 + s, 120) for s in range(12)]
    assert sum(not r.fallbacks for r in runs) >= 8


@pytest.mark.slow
@pytest.mark.parametrize("seed", range(64))
def test_random_ops_match_the_python_mesh_fuzz(seed):
    r = Run(5000 + seed, n_points=150, n_edges=120, n_faces=60)
    for _ in range(600):
        r.step()
        if type(r.sm) is not S.StoreMesh:
            break


# ---- the documented fallbacks and the history paths ---------------------

def _pair_meshes():
    pm, sm = M.Mesh(), M.Mesh()
    S.become_store(sm, mod.MeshStore(next(M._KEY_NAMESPACES),
                                     **S._weld_args()), [])
    return pm, sm


def _square(m, x=0.0):
    return m.add_face([V(x, 0, 0), V(x + 1, 0, 0), V(x + 1, 1, 0),
                       V(x, 1, 0)])


def test_relinking_beside_a_redrawn_edge_falls_back_to_python():
    """Undo of an erase after the same segment was drawn again: Python
    would hold two edges between the same vertices; the store refuses and
    the Python kernel runs, with the same result."""
    out = []
    for m in _pair_meshes():
        e = m.add_edge(V(0, 0, 0), V(1, 0, 0))
        m.remove_edge(e)
        m.add_edge(V(0, 0, 0), V(1, 0, 0))
        m.relink_edge(e)
        out.append((type(m), len(m.edges), state(m, None)["rev"]))
    assert out[1][0] is S.MaterialisedMesh
    assert out[0][1:] == out[1][1:]


def test_dropping_a_corner_of_a_face_falls_back_to_python():
    for m in _pair_meshes():
        f = _square(m)
        corner = f.loop[0]
        for e in list(corner.edges):
            m.remove_edge(e)
        assert m.drop_orphan_vertices([corner]) == 1
        assert corner not in m.vertices and f in m.faces
    assert type(m) is S.MaterialisedMesh


def test_removed_elements_read_like_held_objects():
    """History reads a removed face's loop (AddFaceCommand.undo) and a
    removed edge's ends and faces: tombstones answer as the Python object
    would, a materialisation gives them objects, and relinking those
    brings the very keys back."""
    pm, sm = _pair_meshes()
    held = []
    for m in (pm, sm):
        f = _square(m)
        e = m.find_edge(f.loop[0], f.loop[1])
        m.remove_edge(e)
        m.remove_face(f)
        assert [v.position.toTuple() for v in f.loop][1] == (1.0, 0.0, 0.0)
        assert e.faces == [f] and e.v0 == f.loop[0]
        assert f not in m.faces and e not in m.edges
        held.append((f, e))
    assert type(sm) is S.StoreMesh
    sm._materialise()
    objs = sm.__dict__["_objmap"]
    face, edge = objs[held[1][0].key], objs[held[1][1].key]
    assert edge.faces == [face] and edge.v0 is face.loop[0]
    for m, (f, e) in zip((pm, sm), held):
        m.relink_face(f)            # the view resolves to the tombstone object
        m.relink_edge(e)
    assert state(sm, None) == state(pm, None) | {"rev": state(sm, None)["rev"]}
    assert [x.key for x in sm.faces] == [held[1][0].key]


@pytest.mark.parametrize("on", [False, True])
def test_draw_erase_undo_redo_stays_in_the_store(monkeypatch, tmp_path, on):
    """The history commands of drawing and erasing (AddFace / AddEdge /
    DeleteEdges / DeleteFace, with undo and redo) on a store-backed mesh
    give the Python results and never materialise it."""
    from core.history import (AddEdgeCommand, AddFaceCommand,
                              DeleteEdgesCommand, DeleteFaceCommand, History)
    from core.scene import Scene
    from formats import igz
    from tests.test_deferred_mesh import _random_scene
    from tests.test_store_mesh import fingerprint
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    monkeypatch.setattr(_native, "wanted", lambda: True)
    # `_native.native` is fixed at its first read in a session; an earlier
    # test may have fixed it to None.
    monkeypatch.setattr(_native, "native", mod, raising=False)
    path = tmp_path / "d.igz"
    igz.save_scene(_random_scene(random.Random(11)), path)
    results = []
    for store in (False, True):
        monkeypatch.setenv("INGETRAZO_STORE", "1" if store else "0")
        scene = Scene()
        igz.load_into(scene, path)
        m = scene.mesh
        len(m.faces)
        assert (type(m) is S.StoreMesh) == store
        h = History(scene)
        e0 = m.edges[3]
        h.execute(AddFaceCommand([V(90, 0, 0), V(91, 0, 0), V(91, 1, 0),
                                  V(90, 1, 0)], auto=on))
        h.execute(AddEdgeCommand(V(90, 0, 0), V(91, 1, 0)))
        h.execute(DeleteEdgesCommand([e0]))
        h.execute(DeleteFaceCommand(scene.mesh.faces[0]))
        steps = [fingerprint(scene.mesh)]
        for _ in range(4):
            h.undo()
            steps.append(fingerprint(scene.mesh))
        for _ in range(4):
            h.redo()
            steps.append(fingerprint(scene.mesh))
        assert h.last_error is None
        if store and not on:
            assert type(scene.mesh) is S.StoreMesh
        results.append(steps)
    assert results[0] == results[1]


# ---- the engine fuzz on store-backed meshes --------------------------------

def _engine_run(monkeypatch, scenario, seed, store):
    """``tests.test_fuzz_engine.run_sequence`` with every new scene's mesh
    store-backed (``store``) or plain; returns the scenes made and how many
    of the loose mesh's ops left it in the store."""
    import tests.test_fuzz_engine as FE
    made = []

    class _Scene(FE.Scene):
        def __init__(self, *a, **k):
            super().__init__(*a, **k)
            if store:
                S.become_store(self.mesh, mod.MeshStore(
                    next(M._KEY_NAMESPACES), **S._weld_args()), [])
            made.append(self)
    monkeypatch.setattr(FE, "Scene", _Scene)
    mats = []
    real = S._materialise

    def counted(m):
        mats.append(m)
        return real(m)
    monkeypatch.setattr(S, "_materialise", counted)
    FE.run_sequence(scenario, seed)
    _engine_run.materialised = len(mats)
    return made


@pytest.mark.parametrize("scenario", ["cube", "prism", "plan", "group"])
@pytest.mark.parametrize("seed", range(6))
def test_engine_fuzz_on_a_store_backed_mesh(monkeypatch, scenario, seed):
    """The engine certifier (draw on faces, push/pull, undo/redo, with its
    invariants after every commit) with the loose mesh starting in the
    store: drawing and pushing stay there (port step 3: the stitch, the
    plane rebuild's drop, turning a face over), undo restores store
    snapshots. Same final model as the Python run of the same seed."""
    import tests.test_fuzz_engine as FE
    py = _engine_run(monkeypatch, scenario, seed, False)[0]
    st = _engine_run(monkeypatch, scenario, seed, True)[0]
    assert _engine_run.materialised == 0
    assert FE._scene_fingerprint(st) == FE._scene_fingerprint(py)


@pytest.mark.slow
@pytest.mark.parametrize("scenario", ["cube", "prism", "plan", "group"])
@pytest.mark.parametrize("seed", range(6, 60))
def test_engine_fuzz_on_a_store_backed_mesh_fuzz(monkeypatch, scenario, seed):
    import tests.test_fuzz_engine as FE
    py = _engine_run(monkeypatch, scenario, seed, False)[0]
    st = _engine_run(monkeypatch, scenario, seed, True)[0]
    assert FE._scene_fingerprint(st) == FE._scene_fingerprint(py)


# ---- port step 3: the splits and the stitch (docs/meshstore-design.md §16) --

def _drop_ghosts(pm):
    """Design decision 4 on the Python side: a registry entry naming a
    vertex the mesh no longer lists goes (a cross-border weld gives the
    survivor the welded vertex's cell; Python leaves that entry pointing at
    the dead survivor when it dies later, the store drops it)."""
    live = {v.key for v in pm.vertices}
    for k in [k for k, v in pm._registry.items() if v.key not in live]:
        del pm._registry[k]


_STITCH_OPS = ("grid", "split_edge", "split_edge_at", "interior_vertex_on",
               "collapsible_vertex", "collapse_vertex", "collapse_collinear",
               "_weld_into", "weld_coincident", "dedupe_faces",
               "dissolve_edge", "dissolve_edge_recorded",
               "dissolve_coplanar_pair", "dissolve_coplanar_region",
               "turn_face_over")


class StitchRun(Run):
    """``Run`` with the step-3 ops mixed into the step-2 ones. Inputs that
    lead to a coplanar merge are drawn on purpose (grids of quads in axis
    planes, some split into triangles, some wound the other way, some with
    a hole), as are collinear valence-2 vertices (splits) and coincident
    ones (moves onto another vertex or a hair across a cell border)."""

    # One-corner rings (self-loops) are rarer: a weld of a vertex with a
    # self-loop is refused (Python would leave a live edge on the vertex it
    # drops), and ends the sequence.
    ring_sizes = (1, 2, 3, 3, 3, 4, 4, 4, 5, 2, 3, 4, 3, 4, 4)

    def __init__(self, seed, clean=False, **kw):
        self.stitched = dict.fromkeys(_STITCH_OPS, 0)
        self.clean = clean
        if clean:
            self.ring_sizes = (3, 3, 4, 4, 4, 5)
        super().__init__(seed, **kw)

    def block(self, rng, n_points, n_edges, n_faces):
        """The crowded block; ``clean``: without its rings of 1 and 2
        corners and edges onto their own point (the self-loops that make
        a weld refuse and end the sequence), so sequences run longer."""
        blk = crowded_block(rng, n_points=n_points, n_edges=n_edges,
                            n_faces=n_faces)
        if not self.clean:
            return blk
        pos, edges, flags, curves, layers, names, rings, sizes, counts = blk
        keep = edges[:, 0] != edges[:, 1]
        # Rebuild ring by ring, faces keeping only rings of 3+ corners.
        r, sz, ct = [], [], []
        i = j = 0
        for n_r in counts.tolist():
            mine = []
            for _ in range(n_r):
                s_ = int(sizes[j])
                ring = rings[i:i + s_].tolist()
                i += s_
                j += 1
                if s_ >= 3 and len(set(ring)) == s_:
                    mine.append(ring)
            if mine:
                ct.append(len(mine))
                for ring in mine:
                    sz.append(len(ring))
                    r.extend(ring)
        return (pos, edges[keep], flags[keep], curves[keep], layers[keep],
                names, np.asarray(r, np.int64), np.asarray(sz, np.int64),
                np.asarray(ct, np.int64))

    def compare(self):
        _drop_ghosts(self.pm)
        if type(self.sm) is not S.StoreMesh:      # fell back: Python now
            _drop_ghosts(self.sm)
        super().compare()

    def same(self, a, b):
        """Results of the two engines: elements through the key bijection."""
        if isinstance(a, (M.Vertex, M.Edge, M.Face)):
            return b is not None and self.keys.name(b.key) == a.key
        if isinstance(a, (list, tuple)):
            return len(a) == len(b) and all(map(self.same, a, b))
        if isinstance(a, dict):
            return set(a) == set(b) - {"_undo"} and all(
                self.same(a[k], b[k]) for k in a)
        return a == b

    def run_op(self, name, pargs, sargs, fn=None):
        before = {k: {x.key: x for x in getattr(self.pm, n)}
                  for k, n in (("v", "vertices"), ("e", "edges"),
                               ("f", "faces"))}
        if fn is not None:
            self._fn = fn
            a, b = self.both("_call", (self.pm,) + pargs, (self.sm,) + sargs)
        else:
            a, b = self.both(name, pargs, sargs)
        self.stitched[name] += 1
        pair_keys(self.keys, self.pm, self.sm)
        assert self.same(a, b), (name, a, b)
        # Vertices that went are held, as history would hold them, for the
        # `registers` check and relink_vertices. (Faces are not: relinking
        # one whose corner was welded away since is a refusal of step 2,
        # and would end most sequences.)
        now = {x.key for x in self.pm.vertices}
        for key, obj in before["v"].items():
            if key not in now and type(self.sm) is S.StoreMesh:
                pair = (obj, S.VertexView(self.sm, self.keys.p2s[key]))
                if pair[0] not in [x[0] for x in self.gone["v"]]:
                    self.gone["v"].append(pair)
        return a, b

    def both(self, name, pargs, sargs):
        if name == "_call":
            errs, out = [], []
            for args in (pargs, sargs):
                try:
                    out.append(self._fn(*args))
                    errs.append(None)
                except Exception as exc:  # noqa: BLE001 - compared below
                    out.append(None)
                    errs.append(type(exc))
            assert errs[0] == errs[1], (self._fn, errs)
            self.ops += 1
            if type(self.sm) is S.StoreMesh:
                self.on_store += 1
            return out
        return super().both(name, pargs, sargs)

    # -- inputs
    def plane_point(self, axis, base, u, v):
        c = list(base)
        a, b = [i for i in range(3) if i != axis]
        c[a] += u
        c[b] += v
        return V(*c)

    def coplanar_faces(self, f):
        """Faces reached from ``f`` across edges, coplanar with it (either
        winding), in the order found: a region to dissolve."""
        n0 = f.normal()
        comp, stack = [f], [f]
        while stack:
            g = stack.pop()
            for lp in (g.loop, *g.hole_loops):
                for a, b in zip(lp, lp[1:] + lp[:1]):
                    e = self.pm.find_edge(a, b)
                    if e is None:
                        continue
                    for h in e.faces:
                        if h not in comp and abs(V.dotProduct(
                                n0, h.normal())) > 0.999:
                            comp.append(h)
                            stack.append(h)
        return comp

    def store_face(self, f):
        return self.sm.faces[self.pm.faces.index(f)]

    def step(self):
        rng, pm, sm = self.rng, self.pm, self.sm
        if rng.random() < 0.45:
            return super().step()
        r = rng.random()
        if r < 0.12:
            axis = rng.randrange(3)
            base = rng.choice(self.base)
            s = rng.choice([0.25, 0.5])
            for i in range(rng.randint(1, 3)):
                for j in range(rng.randint(1, 2)):
                    P = lambda u, v: self.plane_point(  # noqa: E731
                        axis, base, (i + u) * s, (j + v) * s)
                    rings = [[P(0, 0), P(1, 0), P(1, 1), P(0, 1)]]
                    if rng.random() < 0.25:
                        rings = [[P(0, 0), P(1, 0), P(1, 1)],
                                 [P(0, 0), P(1, 1), P(0, 1)]]
                    for ring in rings:
                        if rng.random() < 0.15:
                            ring.reverse()
                        holes = None
                        if len(ring) == 4 and rng.random() < 0.15:
                            holes = [[P(.3, .3), P(.3, .6), P(.6, .6),
                                      P(.6, .3)]]
                        a, b = self.run_op("grid", (), (), fn=lambda m, r=ring,
                                           h=holes: m.add_face(r, h))
                        self.compare()
            return
        if r < 0.20:
            x = self.pick("e")
            if not x:
                return
            a, b = x[0].v0.position, x[0].v1.position
            p = rng.choice([(a + b) * 0.5, a + (b - a) * rng.random(),
                            V(a), self.point()])
            self.run_op("split_edge", (x[0], p), (x[1], p))
        elif r < 0.26:
            x = self.pick("e")
            if not x:
                return
            mid = pm.interior_vertex_on(x[0])
            if mid is None or rng.random() < 0.3:
                y = self.pick("v")
                if not y or y[0] in (x[0].v0, x[0].v1):
                    return
                mid = y[0]
            smid = sm.vertices[pm.vertices.index(mid)]
            self.run_op("split_edge_at", (x[0], mid), (x[1], smid))
        elif r < 0.29:
            x = self.pick("e")
            if x:
                self.run_op("interior_vertex_on", (x[0],), (x[1],))
        elif r < 0.32:
            x = self.pick("v")
            if x:
                self.run_op("collapsible_vertex", (x[0],), (x[1],))
        elif r < 0.40:
            cands = [i for i, v in enumerate(pm.vertices)
                     if len(v.edges) == 2
                     and len({e.other(v).position.toTuple()
                              for e in v.edges}) == 2
                     and all(e.v0 is not e.v1 for e in v.edges)]
            good = [i for i in cands if pm.collapsible_vertex(pm.vertices[i])]
            pool = good if good and rng.random() < 0.8 else cands
            if not pool:
                return
            i = rng.choice(pool)
            self.run_op("collapse_vertex", (pm.vertices[i],),
                        (sm.vertices[i],))
        elif r < 0.44:
            self.run_op("collapse_collinear", (), ())
        elif r < 0.49:
            x, y = self.pick("v"), self.pick("v")
            if not x or not y or x[0] is y[0] or any(
                    e.v0 is e.v1 for e in y[0].edges):
                return
            self.run_op("_weld_into", (x[0], y[0]), (x[1], y[1]))
        elif r < 0.58:
            for _ in range(rng.choice([0, 1, 1, 2, 3])):
                x, y = self.pick("v"), self.pick("v")
                if x and y and x[0] is not y[0]:
                    d = y[0].position - x[0].position
                    d += V(rng.choice([0, 0, 0, 3e-5, -4e-5, 6e-5]), 0,
                           rng.choice([0, 0, 2e-5]))
                    self.both("move_vertex", (x[0], d), (x[1], d))
                    self.compare()
            self.run_op("weld_coincident", (), ())
        elif r < 0.64:
            if rng.random() < 0.6:
                x = self.pick("f")
                if x:
                    holes = x[0].holes if rng.random() < 0.6 else None
                    self.run_op("grid", (), (), fn=lambda m, f=x[0], h=holes:
                                m.add_face(f.vertices, h))
                    self.compare()
            self.run_op("dedupe_faces", (), ())
        elif r < 0.78:
            two = [i for i, e in enumerate(pm.edges) if len(e.faces) >= 2]
            if not two:
                return
            i = rng.choice(two)
            pe, se = pm.edges[i], sm.edges[i]
            name = rng.choice(["dissolve_edge", "dissolve_edge",
                               "dissolve_edge_recorded",
                               "dissolve_coplanar_pair"])
            if name == "dissolve_coplanar_pair":
                fa, fb = pe.faces[0], pe.faces[1]
                self.run_op(name, (fa, fb), (self.store_face(fa),
                                             self.store_face(fb)))
            else:
                self.run_op(name, (pe,), (se,))
        elif r < 0.92:
            x = self.pick("f")
            if not x:
                return
            comp = self.coplanar_faces(x[0])
            if rng.random() < 0.3:
                rng.shuffle(comp)
            if rng.random() < 0.15 and len(comp) > 2:
                comp = comp[:2]
            self.run_op("dissolve_coplanar_region", (comp,),
                        ([self.store_face(f) for f in comp],))
        else:
            x = self.pick("f")
            if x:
                self.run_op("turn_face_over", (x[0],), (x[1],),
                            fn=lambda m, f: M.turn_face_over(f))
        self.compare()


def stitch_run(seed, n_ops, **kw):
    r = StitchRun(seed, clean=seed % 2 == 1, **kw)
    for _ in range(n_ops):
        r.step()
        if type(r.sm) is not S.StoreMesh:
            r.fallbacks.append(r.ops)
            break
    return r


@pytest.mark.parametrize("seed", range(16))
def test_stitch_ops_match_the_python_mesh(seed):
    """Every op compared (state, keys, results); a sequence ends early only
    on a documented refusal, after which the states are compared too."""
    r = stitch_run(7000 + seed, 220)
    assert r.ops > 60 or r.fallbacks
    if not r.fallbacks:
        assert r.on_store == r.ops


def test_the_stitch_fuzz_reaches_every_op():
    """The draws above are not vacuous: each step-3 op runs, merges happen
    (faces go and come), and most ops stay on the store."""
    runs = [stitch_run(7100 + s, 160) for s in range(8)]
    seen = {k: sum(r.stitched[k] for r in runs) for k in _STITCH_OPS}
    assert all(n >= 3 for n in seen.values()), seen
    assert sum(r.on_store for r in runs) >= 400


@pytest.mark.slow
@pytest.mark.parametrize("seed", range(48))
def test_stitch_ops_match_the_python_mesh_fuzz(seed):
    stitch_run(9000 + seed, 400, n_points=120, n_edges=90, n_faces=40)

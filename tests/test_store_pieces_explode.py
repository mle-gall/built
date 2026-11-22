# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Pieces, Explode and Purge on store-backed meshes against the Python
kernel (docs/meshstore-design.md, "Pieces, Explode and Purge").

Each scenario runs twice from the same construction: on Python meshes and
on store-backed ones. The store run must never materialise; both must
show the same model after do, undo and redo (``fingerprint``: positions,
lists in order, edge flags and faces, rings, attrs, normals); and the
store run's change-feed steps must be exactly what changed, by key."""
from __future__ import annotations

import pytest
from PySide6.QtGui import QMatrix4x4, QVector3D as V

from core import mesh as M
from core import store_mesh as S
from core.group import Group, world_mesh
from core.history import (DeleteEdgesCommand, ExplodeGroupCommand, History,
                          SplitIntoPiecesCommand)
from core.layers import Layer
from core.pieces import connected_parts, solid_parts, split_into_pieces
from core.protocol import commands as C
from core.protocol.keys import mesh_key
from core.purge import layer_counts, used_layers, used_materials
from core.scene import Scene
from tests.test_core_protocol import _expected, _got, _inverse, _read
from tests.test_store_mesh import fingerprint, pytestmark as store_required
from core._native import available, qt_hypot

mod = available()
needs_ops = pytest.mark.skipif(
    mod is None or not hasattr(mod.MeshStore, "paste_from")
    or qt_hypot() is None,
    reason="native module without the pieces / explode ops")

_QUADS = ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4),
          (2, 6, 7, 3), (0, 4, 6, 2), (1, 3, 7, 5))


pytestmark = [store_required, pytest.mark.usefixtures("explicit_store_meshes")]


def _mesh(store):
    m = M.Mesh()
    if store:
        S.become_store(m, mod.MeshStore(next(M._KEY_NAMESPACES),
                                        **S._weld_args()), [])
    return m


def _cube(m, x0=0.0, y0=0.0, z0=0.0, s=1.0, quads=_QUADS):
    c = [V(x0 + dx * s, y0 + dy * s, z0 + dz * s)
         for dz in (0, 1) for dy in (0, 1) for dx in (0, 1)]
    return [m.add_face([c[k] for k in q]) for q in quads]


@pytest.fixture
def no_materialise(monkeypatch):
    def fail(mesh):
        pytest.fail("a ported command materialised its mesh")
    monkeypatch.setattr(S.StoreMesh, "_materialise", fail)


def _translate(x, y, z, turn=0.0):
    t = QMatrix4x4()
    t.translate(x, y, z)
    if turn:
        t.rotate(turn, 0, 0, 1)
    return t


# ---------------------------------------------------------------- explode

def _explode_scene(store):
    """A box in the loose mesh; beside it a group (a placed box, turned a
    quarter and moved so one wall lands on the box's wall) with paint,
    textures with and without a map, a soft edge, a tagged curve wire
    and a component nested in it."""
    sc = Scene()
    sc.mesh = _mesh(store)
    loose = _cube(sc.mesh)
    loose[2].attrs.update({"mat": "Brick", "color": [0.6, 0.2, 0.1]})
    gm = _mesh(store)
    faces = _cube(gm)
    faces[0].attrs.update({"texture": {"path": "wood.png", "uvw": [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]}})
    faces[1].attrs.update({"texture": {"path": "tile.png", "scale": 1.0}})
    faces[3].attrs.update({"back": {"color": [0.0, 0.0, 1.0]}})
    e = gm.find_edge(gm.vertex_at(V(0, 0, 0)), gm.vertex_at(V(0, 0, 1)))
    e.soft = True
    w = gm.add_edge(V(0.2, 0.5, 1.0), V(0.2, 0.5, 2.0))
    w.layer, w.curve = "T1", 77
    w.hidden = True
    g = Group(gm, name="Cajón")
    g.xform = _translate(2, 0, 0, turn=90)
    g.material = {"color": [0.5, 0.5, 0.5]}
    cm = _mesh(store)
    _cube(cm, s=0.25)
    kid = Group(cm, name="Tirador")
    kid.xform = _translate(0.3, 0.3, 1.0)
    g.adopt([kid])
    sc.groups.append(g)
    return sc, g, kid


def _model(sc):
    out = {"mesh": fingerprint(sc.mesh),
           "groups": [(g.name, tuple(g.xform.data()) if g.xform else None,
                       g.material, fingerprint(g.mesh)) for g in sc.groups]}
    return out


def _run_explode(store):
    sc, g, kid = _explode_scene(store)
    h = History(sc)
    seen = [_model(sc)]
    h.execute(ExplodeGroupCommand(g))
    assert h.last_error is None
    seen.append(_model(sc))
    assert h.undo()
    seen.append(_model(sc))
    assert h.redo()
    seen.append(_model(sc))
    assert (type(sc.mesh) is S.StoreMesh) == store
    return seen, sc, h, g, kid


@needs_ops
def test_explode_with_a_transform_onto_coincident_geometry(monkeypatch):
    """The quarter-turned box welds onto the loose box: shared corners,
    one wall instead of two coincident ones, every flag and paint carried
    — the same model as the Python kernel after do, undo and redo."""
    py, *_ = _run_explode(False)
    monkeypatch.setattr(S.StoreMesh, "_materialise",
                        lambda m: pytest.fail("explode materialised"))
    st, sc, h, g, kid = _run_explode(True)
    assert st == py
    assert py[0] != py[1] and py[2] == py[0] and py[3] == py[1]
    # The shared wall is one face: 6 + 6 - 2 + 1 rebuilt.
    assert len(sc.mesh.faces) == 11
    assert [x.name for x in sc.groups] == ["Tirador"]


@needs_ops
def test_explode_feed_names_both_meshes_the_placement_and_the_keys(
        no_materialise):
    sc, g, kid = _explode_scene(True)
    h = History(sc)
    m, src = sc.mesh, g.mesh
    src_keys = {k: {x.key for x in getattr(src, n)}
                for k, n in (("vertex", "vertices"), ("edge", "edges"),
                             ("face", "faces"))}
    before = _read(m)
    r = C.run(h, "explode_group", placement=g.uid)
    assert r["ok"], r
    after = _read(m)
    step = r["step"]
    want = _expected(before, after)
    (change,) = step.meshes
    assert change.key == mesh_key(m) and change.backing == "store"
    assert _got(change)["created"] == want["created"]
    assert _got(change)["removed"] == want["removed"]
    # The loose box's wall went into the rebuilt one; its other faces keep
    # their keys; every vertex it had is still there (welded onto).
    assert set(before[0]) <= set(after[0])
    assert len(set(before[2]) & set(after[2])) == 5
    # Placements: the group went, its child came out.
    assert step.placements["removed"] == [g.uid]
    assert step.placements["created"] == [kid.uid] and step.complete
    # Copies: from the group's mesh into the open mesh, by key.
    (copy,) = step.copies
    assert (copy["source"], copy["target"]) == (mesh_key(src), mesh_key(m))
    for kind in ("vertex", "edge", "face"):
        pairs = copy["keys"][kind]
        assert {a for a, _b in pairs} <= src_keys[kind]
        assert all(b in after[("vertex", "edge", "face").index(kind)]
                   for _a, b in pairs)
    assert len(copy["keys"]["vertex"]) == 10        # 8 corners + the wire
    assert len(copy["keys"]["face"]) == 5           # its wall was rebuilt
    assert step.to_json()["copies"][0]["keys"]["face"][0][0] == str(
        copy["keys"]["face"][0][0])
    # Undo: the inverse; no copies.
    undo = C.run(h, "undo")["step"]
    assert _got(undo.meshes[0])["created"] == _inverse(want)["created"]
    assert _got(undo.meshes[0])["removed"] == _inverse(want)["removed"]
    assert undo.placements["created"] == [g.uid] and not undo.copies
    assert _read(m) == before
    # Redo: the same step again, the same keys (the store puts back the
    # state the first run made).
    redo = C.run(h, "redo")["step"]
    assert _read(m) == after
    assert _got(redo.meshes[0])["created"] == want["created"]
    assert redo.copies[0]["keys"]["face"] == copy["keys"]["face"]
    assert type(m) is S.StoreMesh and type(src) is S.StoreMesh


def _nested_scene(store):
    """A group holding a box and a component, the component holding a box
    and a component of its own: Explode twice, one level each time."""
    sc = Scene()
    sc.mesh = _mesh(store)
    _cube(sc.mesh, z0=-1.0)                     # the floor box, below
    inner = _mesh(store)
    _cube(inner, s=0.5)
    d = Group(inner, name="D")
    d.xform = _translate(0.25, 0.25, 0.5)
    cm = _mesh(store)
    _cube(cm)                                    # sits on the floor box
    c = Group(cm, name="C")
    c.xform = _translate(3, 0, 0, turn=45)
    c.adopt([d])
    gm = _mesh(store)
    _cube(gm, x0=-1.0)                           # shares a wall with C? no:
    g = Group(gm, name="G")
    g.xform = _translate(0, 0, 0)
    g.adopt([c])
    sc.groups.append(g)
    return sc, g, c, d


def _run_nested(store):
    sc, g, c, d = _nested_scene(store)
    h = History(sc)
    seen = [_model(sc)]
    h.execute(ExplodeGroupCommand(g))
    seen.append(_model(sc))
    h.execute(ExplodeGroupCommand(sc.groups[0]))   # C, now at the top
    assert h.last_error is None
    seen.append(_model(sc))
    for step in ("undo", "undo", "redo", "redo"):
        assert getattr(h, step)()
        seen.append(_model(sc))
    assert (type(sc.mesh) is S.StoreMesh) == store
    return seen, sc, h


@needs_ops
def test_explode_nested_one_level_at_a_time(monkeypatch):
    py, *_ = _run_nested(False)
    monkeypatch.setattr(S.StoreMesh, "_materialise",
                        lambda m: pytest.fail("explode materialised"))
    st, sc, h = _run_nested(True)
    assert st == py
    assert [x.name for x in sc.groups] == ["D"]
    assert st[3] == st[1] and st[4] == st[0] and st[6] == st[2]
    steps = h.feed.since(0)["steps"]
    assert [len(s.placements["removed"]) for s in steps[:2]] == [1, 1]
    assert all(s.copies for s in steps if s.phase != "undo")
    assert not any(s.copies for s in steps if s.phase == "undo")


# ---------------------------------------------------------------- pieces

def _three_pieces(m):
    """Two boxes apart, an open patch lying on the first (it joins it),
    a sheet far away (a piece of its own), a wire hanging off the second
    box (goes with it) and boards pressed together (edges with four
    faces) for the partner rule."""
    _cube(m)
    _cube(m, x0=3.0)
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])       # patch on box 1
    m.add_face([V(10, 0, 0), V(11, 0, 0), V(11, 1, 0), V(10, 1, 0)])
    m.add_edge(V(4, 1, 1), V(4, 2, 2))                    # wire on box 2
    m.add_edge(V(20, 0, 0), V(21, 0, 0))                  # wire alone
    _cube(m, x0=6.0)
    _cube(m, x0=6.0, z0=1.0)                              # stacked boards
    _cube(m, x0=7.0)                                      # beside: an edge


def _by_index(m, parts):
    fi = {f.key: i for i, f in enumerate(m.faces)}
    ei = {e.key: i for i, e in enumerate(m.edges)}
    return [([fi[f.key] for f in fs], [ei[e.key] for e in es])
            for fs, es in parts]


@needs_ops
def test_pieces_are_pythons_in_order(no_materialise):
    py, st = _mesh(False), _mesh(True)
    _three_pieces(py)
    _three_pieces(st)
    assert fingerprint(py) == fingerprint(st)
    st.__dict__["_attr_own"].clear()             # fingerprint read attrs
    for fn in (solid_parts, connected_parts):
        a, b = fn(py), fn(st)
        assert _by_index(py, a) == _by_index(st, b), fn.__name__
    assert [len(f) for f, _e in solid_parts(st)] == [7, 6, 6, 6, 6, 1, 0]
    assert type(st) is S.StoreMesh and not st.__dict__["_attr_own"]


def _split_run(store):
    sc = Scene()
    gm = _mesh(store)
    _cube(gm)
    _cube(gm, x0=3.0)
    gm.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    gm.add_face([V(10, 0, 0), V(11, 0, 0), V(11, 1, 0), V(10, 1, 0)])
    e = gm.add_edge(V(4, 1, 1), V(4, 2, 2))
    e.soft, e.layer = True, "T2"
    gm.faces[7].attrs.update({"mat": "Steel"})
    g = Group(gm, name="Mesa")
    g.xform = _translate(1, 2, 3)
    sc.groups.append(g)
    h = History(sc)
    pieces = split_into_pieces(g)
    seen = [[(p.name, fingerprint(p.mesh)) for p in pieces]]
    h.execute(SplitIntoPiecesCommand(g, pieces))
    assert h.last_error is None
    seen.append([(k.name, fingerprint(k.mesh)) for k in g.children])
    assert h.undo()
    seen.append(fingerprint(g.mesh))
    assert h.redo()
    seen.append([(k.name, fingerprint(k.mesh)) for k in g.children])
    return seen, sc, h, g, gm


@needs_ops
def test_split_a_mesh_with_three_pieces(monkeypatch):
    py, *_ = _split_run(False)
    monkeypatch.setattr(S.StoreMesh, "_materialise",
                        lambda m: pytest.fail("split materialised"))
    st, sc, h, g, gm = _split_run(True)
    assert st == py
    assert len(st[0]) == 3
    assert all(type(k.mesh) is S.StoreMesh for k in g.children)
    steps = h.feed.since(0)["steps"]
    do, undo, redo = steps
    assert do.placements["modified"] == [g.uid]
    assert do.placements["created"] == [k.uid for k in g.children]
    assert undo.placements["removed"] == do.placements["created"]
    assert do.complete and undo.complete
    # The pieces' faces by source key: every source face exactly once.
    pairs = [p for c in do.copies for p in c["keys"]["face"]]
    assert {c["source"] for c in do.copies} == {mesh_key(gm)}
    assert sorted(a for a, _b in pairs) == sorted(f.key for f in gm.faces)
    assert redo.copies == do.copies and not undo.copies


# ---------------------------------------------------------------- purge

def _purge_run(store):
    sc = Scene()
    sc.mesh = _mesh(store)
    m = sc.mesh
    m.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    m.add_edge(V(5, 0, 0), V(6, 0, 0))
    m.add_edge(V(6, 0, 0), V(6, 1, 0))
    h = History(sc)
    # Delete (the key, not the Eraser) takes the edges and leaves their
    # corners behind.
    h.execute(DeleteEdgesCommand([e for e in m.edges if not e.faces]))
    assert h.last_error is None
    seen = [fingerprint(m), len(m.vertices)]
    before = _read(m)
    r = C.run(h, "purge_debris")
    assert r["ok"], r
    after = _read(m)
    seen += [fingerprint(m), len(m.vertices)]
    assert h.undo()
    seen.append(fingerprint(m))
    assert h.redo()
    seen.append(fingerprint(m))
    return seen, r["step"], before, after, m


@needs_ops
def test_purge_debris_after_a_delete(no_materialise):
    py, *_ = _purge_run(False)
    st, step, before, after, m = _purge_run(True)
    assert st == py
    assert py[1] - py[3] == 3            # the wire's three corners
    (change,) = step.meshes
    assert _got(change) == _expected(before, after)
    assert len(change.removed["vertex"]) == 3
    assert type(m) is S.StoreMesh


@needs_ops
def test_purge_unused_reads_store_meshes_without_touching_faces(
        no_materialise):
    sc = Scene()
    sc.mesh = _mesh(True)
    f = sc.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0)])
    f.attrs.update({"mat": "Oak", "layer": "Muros"})
    gm = _mesh(True)
    _cube(gm)
    g = Group(gm)
    sc.groups.append(g)
    e = gm.edges[0]
    e.layer = "Aristas"
    for name in ("Muros", "Aristas", "Vacía"):
        sc.layers.append(Layer(name))
    py = Scene()
    py.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0)]).attrs.update(
        {"mat": "Oak", "layer": "Muros"})
    pg = M.Mesh()
    _cube(pg)
    pg.edges[0].layer = "Aristas"
    py.groups.append(Group(pg))
    for name in ("Muros", "Aristas", "Vacía"):
        py.layers.append(Layer(name))
    gm.__dict__["_attr_own"].clear()
    assert used_materials(sc) == used_materials(py) == {"Oak"}
    assert used_layers(sc) == used_layers(py)
    assert layer_counts(sc) == layer_counts(py)
    assert not gm.__dict__["_attr_own"]          # no face got a dict
    h = History(sc)
    r = C.run(h, "purge_unused")
    assert r["ok"] and r["step"].tags["removed"] == ["Vacía"]
    assert C.run(h, "purge_unused")["ok"] is False   # nothing left to go


@needs_ops
def test_explode_view_and_the_new_commands_refuse_unknown_placements():
    sc, g, kid = _explode_scene(True)
    h = History(sc)
    with pytest.raises(KeyError):
        C.run(h, "explode_group", placement="no-such-uid")
    r = C.run(h, "explode_view", placement=g.uid, factor=1.0, mode="z")
    assert r["ok"] and r["step"].placements["modified"] == [kid.uid]
    assert r["step"].complete
    with pytest.raises(ValueError):
        C.run(h, "explode_view", placement=g.uid, factor=1.0, mode="w")


def _random_scene(m, seed):
    """Boxes of random size and turn, some stacked or beside another
    (welded boards: edges with four or six faces), some wound wrong, some
    open; patches on them, a holed plate, loose wires."""
    import math
    import random
    rnd = random.Random(seed)
    for i in range(rnd.randint(3, 7)):
        x0 = rnd.choice([0.0, 1.0, 2.0, 5.0 + i * 3])
        z0 = rnd.choice([0.0, 0.0, 1.0])
        s = 1.0 if x0 < 5 else rnd.uniform(0.3, 2.0)
        turn = 0.0 if x0 < 5 else rnd.uniform(0, 2 * math.pi)
        c, sn = math.cos(turn), math.sin(turn)
        pts = [V(x0 + (dx * c - dy * sn) * s, (dx * sn + dy * c) * s,
                 z0 + dz * s) for dz in (0, 1) for dy in (0, 1)
               for dx in (0, 1)]
        quads = list(_QUADS)
        if rnd.random() < 0.3:
            quads.pop(rnd.randrange(6))                  # open
        for q in quads:
            ring = [pts[k] for k in q]
            if rnd.random() < 0.15:
                ring.reverse()                           # wound wrong
            m.add_face(ring)
        if rnd.random() < 0.4:
            m.add_face([pts[0], pts[1], pts[3]])         # a patch on it
    if rnd.random() < 0.7:
        hole = [V(31.2, 0.2, 0), V(31.2, 0.8, 0), V(31.8, 0.8, 0),
                V(31.8, 0.2, 0)]
        m.add_face([V(31, 0, 0), V(32, 0, 0), V(32, 1, 0), V(31, 1, 0)],
                   [hole])
    for _ in range(rnd.randint(0, 3)):
        a = V(rnd.uniform(-2, 30), rnd.uniform(-2, 2), rnd.uniform(0, 2))
        m.add_edge(a, a + V(0.5, 0.1, 0.2))


@needs_ops
@pytest.mark.parametrize("seed", range(24))
def test_random_pieces_match_python(seed, no_materialise):
    py, st = _mesh(False), _mesh(True)
    _random_scene(py, seed)
    _random_scene(st, seed)
    assert fingerprint(py) == fingerprint(st)
    for fn in (solid_parts, connected_parts):
        assert _by_index(py, fn(py)) == _by_index(st, fn(st)), fn.__name__
    from core.pieces import copy_part
    for (fa, ea), (fb, eb) in zip(solid_parts(py), solid_parts(st)):
        assert fingerprint(copy_part(fa, ea)) == fingerprint(copy_part(fb, eb))


def _overlap_scene(store, seed):
    """Plates on two levels in the loose mesh; a group of plates that,
    placed, lands partly on them (coplanar overlaps to rebuild), partly
    in free space, some tilted."""
    import random
    rnd = random.Random(seed)
    sc = Scene()
    sc.mesh = _mesh(store)

    def plates(m, n, spread):
        for _ in range(n):
            x, y = rnd.randint(0, spread), rnd.randint(0, spread)
            z = rnd.choice([0.0, 0.0, 1.0])
            w, d = rnd.randint(1, 3), rnd.randint(1, 3)
            tilt = rnd.random() < 0.2
            m.add_face([V(x, y, z), V(x + w, y, z),
                        V(x + w, y + d, z + (0.5 if tilt else 0.0)),
                        V(x, y + d, z + (0.5 if tilt else 0.0))])
    plates(sc.mesh, rnd.randint(2, 6), 8)
    gm = _mesh(store)
    plates(gm, rnd.randint(2, 6), 6)
    g = Group(gm, name="Placas")
    g.xform = _translate(rnd.choice([0.0, 0.5, 1.0]), rnd.choice([0.0, 1.0]),
                         0.0)
    sc.groups.append(g)
    return sc, g


def _overlap_run(store, seed):
    sc, g = _overlap_scene(store, seed)
    h = History(sc)
    seen = [fingerprint(sc.mesh)]
    h.execute(ExplodeGroupCommand(g))
    assert h.last_error is None, h.last_error
    seen.append(fingerprint(sc.mesh))
    h.undo()
    seen.append(fingerprint(sc.mesh))
    h.redo()
    seen.append(fingerprint(sc.mesh))
    return seen, type(sc.mesh)


@needs_ops
@pytest.mark.parametrize("seed", range(16))
def test_random_explodes_onto_overlapping_plates_match_python(seed):
    py, _ = _overlap_run(False, seed)
    st, kind = _overlap_run(True, seed)
    assert st == py
    assert py[2] == py[0] and py[3] == py[1]
    assert kind is S.StoreMesh

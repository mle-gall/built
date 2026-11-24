# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Three gaps the visual check of the ported commands found in the change
feed (docs/core-protocol.md §4) and in element keys, each run with the
store on and off:

1. Leaving an edited component instance (two copies): the Reshare step
   names the shared definition's mesh and every placement that shows it,
   on execute, undo and redo; redo gives the definition the keys the
   first run gave it.
2. Free edges drawn on a plane beside small plates (under the rebuild's
   400-face limit): the plane rebuild keeps the plates' faces, keys and
   attrs, because their rings come out the same; the step names only
   what was drawn.
3. Split into Pieces empties the group into a store mesh when the store
   is on (not a Python ``Mesh`` beside store pieces; ``Mesh()`` is
   store-backed since the thin-handle merge), the same one on redo."""
from __future__ import annotations

import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V

from core import mesh as M
from core import store_mesh as S
from core.group import Group
from core.history import History, SplitIntoPiecesCommand
from core.reference_edit import ReshareInstanceCommand
from core.pieces import split_into_pieces
from core.protocol import commands as C
from core.protocol.keys import mesh_key
from core.scene import Scene
from tests.test_core_protocol import _expected, _got, _inverse, _read
from tests.test_store_mesh import fingerprint, pytestmark  # noqa: F401
from core._native import available, qt_hypot

mod = available()
STORE = [False, pytest.param(True, marks=pytest.mark.skipif(
    mod is None or not hasattr(getattr(mod, "StoreSnapshot", None), "changes_since")
    or qt_hypot() is None, reason="native module without the store ops"))]


def _mesh(store):
    m = M.Mesh()
    if store:
        S.become_store(m, mod.MeshStore(next(M._KEY_NAMESPACES),
                                        **S._weld_args()), [])
    return m


def _scene(store):
    sc = Scene()
    if store:
        S.become_store(sc.mesh, mod.MeshStore(next(M._KEY_NAMESPACES),
                                              **S._weld_args()), [])
    return sc


@pytest.fixture(autouse=True)
def engine(request, monkeypatch):
    """Store off means Python meshes everywhere: ``Mesh()`` and ``Scene()``
    make store meshes when the engine is on."""
    params = getattr(getattr(request.node, "callspec", None), "params", {})
    if "store" in params:
        monkeypatch.setenv("INGETRAZO_STORE", "1" if params["store"] else "0")


@pytest.fixture
def guard(monkeypatch):
    """No ported path materialises a store mesh."""
    monkeypatch.setattr(S.StoreMesh, "_materialise",
                        lambda m: pytest.fail("materialised a store mesh"))


# ------------------------------------------------- 1. leaving an instance

def _instances(store):
    sc = _scene(store)
    proto = _mesh(store)
    proto.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    copies = []
    for dx in (0, 2):
        g = Group(proto, name=f"Inst{dx}")
        xf = QMatrix4x4()
        xf.translate(dx, 0, 0)
        g.xform, g.component = xf, True
        sc.groups.append(g)
        copies.append(g)
    return sc, proto, copies


def _edit_and_leave(sc, h, group):
    """What the viewport does: enter, push the face up, leave; the
    session folds into one Reshare step."""
    sc.begin_group_edit(group)
    mark = len(h.undo_stack)
    r = C.run(h, "push_pull", face=str(sc.mesh.faces[0].key), distance=0.8)
    assert r["ok"], r
    share = sc._edit_stack[-1].pop("share", None)
    if share is None:
        sc.end_group_edit()
        return
    g, proto, xform, _state0 = share
    edited = g.mesh
    sc.end_group_edit()
    inner = h.undo_stack[mark:]
    del h.undo_stack[mark:]
    h.execute(ReshareInstanceCommand(g, proto, xform, edited, inner))


@pytest.mark.parametrize("store", STORE)
def test_leaving_an_instance_names_the_definition_and_its_copies(store, guard):
    sc, proto, copies = _instances(store)
    h = History(sc)
    before = _read(proto)
    _edit_and_leave(sc, h, copies[0])
    after = _read(proto)
    assert len(proto.faces) == 6 and all(g.mesh is proto for g in copies)
    uids = sorted(g.uid for g in copies)
    steps = [h.feed.last]
    assert h.undo()
    assert _read(proto) == before
    steps.append(h.feed.last)
    assert h.redo()
    assert _read(proto) == after            # the first run's keys again
    steps.append(h.feed.last)
    want = _expected(before, after)
    for step, phase, rec in zip(steps, ("execute", "undo", "redo"),
                                (want, _inverse(want), want)):
        assert step.command == steps[0].command and step.phase == phase
        assert step.complete, phase
        assert step.placements == {"created": [], "removed": [],
                                   "modified": uids}, phase
        change = step.mesh_change(proto)
        assert change is not None, phase
        assert change.key == mesh_key(proto)
        if store:
            assert type(proto) is S.StoreMesh
            assert change.backing == "store" and not change.replaced, phase
            # The definition takes the edited copy's elements under
            # fresh keys: every old key removed, every new one created.
            assert _got(change) == rec, phase
        else:
            # The push made the Python copy store-backed first
            # (docs/rust-tools.md §24): the definition takes a store, and
            # undo gives it back its Python objects.
            backing = "python" if phase == "undo" else "store"
            assert (change.backing, change.replaced) == (backing, True), phase


# ------------------------------------------- 2. drawing beside small plates

def _plates(store):
    sc = _scene(store)
    m = sc.mesh
    m.add_face([V(4, 0, 0), V(8, 0, 0), V(8, 4, 0), V(4, 4, 0)])
    m.add_face([V(10, 0, 0), V(14, 0, 0), V(14, 4, 0), V(10, 4, 0)])
    m.faces[0].attrs["color"] = [10, 20, 30]
    return sc


def _faces(m):
    return {f.key: (tuple(v.key for v in f.loop), dict(f.attrs)) for f in m.faces}


def _draw_beside(store):
    sc = _plates(store)
    h = History(sc)
    plates = _faces(sc.mesh)
    seen, records = [fingerprint(sc.mesh)], []

    def run(name, **args):
        before = _read(sc.mesh)
        r = C.run(h, name, **args)
        assert r["ok"], r
        (change,) = r["step"].meshes
        records.append((change, _expected(before, _read(sc.mesh))))
        seen.append(fingerprint(sc.mesh))
        # The plates keep their faces: keys, corners and attrs.
        now = _faces(sc.mesh)
        assert {k: now.get(k) for k in plates} == plates, name

    run("add_edges", segments=[[(0, 0, 0), (2, 0, 0)], [(2, 0, 0), (2, 2, 0)],
                               [(2, 2, 0), (0, 2, 0)]])
    run("undo")
    run("redo")
    run("add_edge", a=(0, 2, 0), b=(0, 0, 0))     # closes a new square
    assert len(sc.mesh.faces) == 3
    return seen, records, sc


@pytest.mark.parametrize("store", STORE)
def test_drawing_beside_small_plates_keeps_their_faces(store, guard):
    seen, records, sc = _draw_beside(store)
    if store:
        assert type(sc.mesh) is S.StoreMesh
        for change, want in records:
            assert change.backing == "store" and not change.replaced
            assert _got(change) == want
        (draw, w), _undo, _redo, (close, c) = records
        # Three free edges: their vertices and edges, nothing else.
        assert [len(w["created"][k]) for k in ("vertex", "edge", "face")] == [4, 3, 0]
        assert not any(w["removed"].values()) and not any(w["modified"].values())
        # The closing line: its edge and the one new face; no plate.
        assert [len(c["created"][k]) for k in ("vertex", "edge", "face")] == [0, 1, 1]
        assert not any(c["removed"].values()) and not c["modified"]["face"]
    else:
        for change, _want in records:
            assert (change.backing, change.replaced) == ("python", True)


@pytest.mark.parametrize("store", STORE[1:])
def test_drawing_beside_plates_matches_python(store, guard, monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", "0")
    py, _r, sc = _draw_beside(False)
    assert type(sc.mesh) is M.Mesh
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    st, _r, _s = _draw_beside(True)
    assert st == py


@pytest.mark.parametrize("store", STORE)
def test_a_plate_whose_ring_changes_is_still_rebuilt(store):
    """A line from outside onto the middle of a plate's side adds a corner
    to its ring: that plate is rebuilt (new key), the other one kept."""
    sc = _plates(store)
    h = History(sc)
    a, b = sc.mesh.faces
    r = C.run(h, "add_edges", segments=[[(6, -2, 0), (6, 0, 0)]])
    assert r["ok"], r
    keys = {f.key for f in sc.mesh.faces}
    assert len(keys) == 2 and b.key in keys and a.key not in keys
    split = next(f for f in sc.mesh.faces if f.key != b.key)
    assert len(split.loop) == 5 and split.attrs.get("color") == [10, 20, 30]


# --------------------------------------------------- 3. split into pieces

@pytest.mark.parametrize("store", STORE)
def test_split_empties_the_group_into_a_store_mesh(store, guard):
    m = _mesh(store)
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    m.add_face([V(4, 0, 0), V(5, 0, 0), V(5, 1, 0), V(4, 1, 0)])
    sc = Scene()
    g = Group(m, name="Parts")
    sc.groups.append(g)
    h = History(sc)
    h.execute(SplitIntoPiecesCommand(g, split_into_pieces(g)))
    assert h.last_error is None and len(g.children) == 2
    kind = S.StoreMesh if store else M.Mesh
    emptied = g.mesh
    assert type(emptied) is kind and not emptied.faces and not emptied.vertices
    assert all(type(k.mesh) is kind for k in g.children)
    assert h.undo() and g.mesh is m and len(m.faces) == 2
    assert h.redo() and g.mesh is emptied           # the same mesh, same key
    assert type(g.mesh) is kind and not g.mesh.faces

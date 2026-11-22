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
   is on (not a Python ``Mesh`` beside store pieces), the same one on
   redo."""
from __future__ import annotations

import pytest
from PySide6.QtGui import QMatrix4x4, QVector3D as V

from core import mesh as M
from core import store_mesh as S
from core.group import Group
from core.history import History, ReshareInstanceCommand, SplitIntoPiecesCommand
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
    g, proto, xform, _state0 = sc.take_edit_share()
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
        assert (step.command, step.phase) == ("ReshareInstanceCommand", phase)
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
            assert (change.backing, change.replaced) == ("python", True), phase

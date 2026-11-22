# SPDX-License-Identifier: GPL-3.0-or-later
"""Redo across a snapshot step relinks what came after it.

Undo is LIFO, but redo crosses whole-state restores: a line, a draw the
snapshot planner records (``SnapshotCompound``), a line closing a square
and its face; undo everything, redo everything. Both snapshots predate the
last line and face, whose redo relinks them by key. A store restore used
to forget their tombstones (and the corner only the last line made), so
the redo materialised the mesh and left a ghost vertex behind.
"""
import pytest
from PySide6.QtGui import QVector3D as V

from core import _native
from core.history import AddEdgeCommand, AddFaceCommand, History, SnapshotCompound
from core.scene import Scene
from core.store_mesh import StoreMesh

SQUARE = [V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)]


@pytest.fixture(params=["store", "python"])
def engine(request, monkeypatch):
    if request.param == "store":
        module = pytest.importorskip("ingetrazo_core")
        monkeypatch.setattr(_native, "native", module)
        monkeypatch.setenv("INGETRAZO_STORE", "1")
        monkeypatch.setattr(StoreMesh, "_materialise",
                            lambda *_: pytest.fail("materialised"))
    else:
        monkeypatch.setenv("INGETRAZO_STORE", "0")
    return request.param


def _state(mesh):
    return (sorted((round(v.position.x(), 6), round(v.position.y(), 6))
                   for v in mesh.vertices),
            len(mesh.edges), len(mesh.faces))


def test_redo_relinks_what_a_later_snapshot_never_held(engine):
    scene = Scene()
    history = History(scene)
    assert (type(scene.mesh) is StoreMesh) == (engine == "store")
    history.execute(AddEdgeCommand(SQUARE[0], SQUARE[1]))
    history.execute(SnapshotCompound([AddEdgeCommand(SQUARE[1], SQUARE[2])]))
    history.execute(AddEdgeCommand(SQUARE[2], SQUARE[3]))
    history.execute(AddEdgeCommand(SQUARE[3], SQUARE[0]))
    history.execute(AddFaceCommand(SQUARE))
    done = _state(scene.mesh)
    keys = ({e.key for e in scene.mesh.edges}, {f.key for f in scene.mesh.faces})
    assert done == (sorted([(0, 0), (1, 0), (1, 1), (0, 1)]), 4, 1)
    for _ in range(5):
        history.undo()
    assert _state(scene.mesh)[1:] == (0, 0)
    for _ in range(5):
        history.redo()
    assert _state(scene.mesh) == done
    # The very elements came back, every corner listed once.
    assert ({e.key for e in scene.mesh.edges}, {f.key for f in scene.mesh.faces}) == keys
    assert len({v.key for v in scene.mesh.vertices}) == 4
    for e in scene.mesh.edges:
        assert scene.mesh.registers(e.v0) and scene.mesh.registers(e.v1)

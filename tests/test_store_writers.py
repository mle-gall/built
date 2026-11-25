# SPDX-License-Identifier: GPL-3.0-or-later
"""Command history on the store must never convert geometry to Python lists."""
from types import SimpleNamespace

import gzip
import json
from pathlib import Path

import pytest

from core.group import Group
from core.history import (AddFaceCommand, FlipFacesCommand, FlipGroupsCommand,
                          FlipVerticesCommand, History, StitchSolidCommand)
from core.scene import Scene
from core.store_mesh import StoreMesh
from plugins.solid_inspector import RepairSolidsCommand, repair_item
from tests.test_push_native import V, _mesh
from tests.test_store_mesh import fingerprint, pytestmark
from tools.paste import PasteTool


def square(x, y, size):
    return [V(x, y), V(x + size, y), V(x + size, y + size), V(x, y + size)]




def exercise(kind):
    scene = Scene()
    m = Scene().mesh
    group = Group(m)
    if kind == "mirror group":
        scene.groups.append(group)
    else:
        scene.mesh = m
    history = History(scene)
    rim = square(2, 2, 1)
    face = m.add_face(square(0, 0, 8), [rim] if kind != "cap" else None)
    face.attrs.update({"layer": "test", "back": {"color": [1, 0, 0]}})
    if kind == "stitch":
        m.add_face(face.vertices, face.holes)
    before = fingerprint(m)
    before_snapshot = m.capture_state()

    if kind == "paste":
        tool = PasteTool()
        tool._clip = {"faces": [(square(10, 10, 1), [])], "edges": [], "groups": []}
        assert tool.stamp(SimpleNamespace(scene=scene, history=history), V(0, 0))
    else:
        if kind == "reverse":
            cmd = FlipFacesCommand([face])
        elif kind == "mirror group":
            cmd = FlipGroupsCommand([group], V(0, 0), V(1, 0))
        elif kind == "mirror vertices":
            cmd = FlipVerticesCommand([v.position for v in m.vertices], V(0, 0),
                                      V(1, 0), [face])
        elif kind == "enlarge hole":
            cmd = AddFaceCommand(square(3, 2, 1))
        elif kind == "stitch":
            cmd = StitchSolidCommand(face.vertices)
        else:
            item = {"kind": "open", "ents": list(m.edges)}
            cmd = RepairSolidsCommand([m], lambda mesh: repair_item(mesh, item))
        history.execute(cmd)
        if kind == "enlarge hole":
            assert cmd._hole_swaps
        if kind == "cap":
            assert cmd.done["holes"] == 1
            assert m.faces[-1].normal() == -face.normal()
    assert history.last_error is None
    after = fingerprint(m)
    assert after != before
    after_keys = [[v.key for v in seq] for seq in (m.vertices, m.edges, m.faces)]
    snapshot = m.capture_state()
    if kind == "paste":
        scene.selection.add(face)
    assert history.undo()
    if kind == "paste":
        assert scene.selection == {face}
    undone = fingerprint(m)
    assert undone == before
    assert m.capture_state() == before_snapshot
    assert history.redo()
    assert [[v.key for v in seq] for seq in (m.vertices, m.edges, m.faces)] == after_keys
    assert fingerprint(m) == after
    assert m.capture_state() == snapshot
    m.restore_state(snapshot)
    assert fingerprint(m) == after
    assert isinstance(m, StoreMesh)
    return before, after, undone


@pytest.mark.parametrize("command", ["reverse", "mirror group", "mirror vertices",
                                     "enlarge hole", "paste", "cap", "stitch"])
def test_command_history_matches_recorded_states(command):
    with gzip.open(Path(__file__).parent / "fixtures" / "store_writer_history.json.gz", "rt") as fixture:
        states = json.load(fixture)
    assert json.loads(json.dumps(exercise(command))) == states[command]


def test_stitch_snapshot_is_required_and_restores_keys():
    m = _mesh(True)
    edge = m.add_edge(V(0, 0), V(2, 0))
    mid = m.vertex(V(1, 0))
    before = m.capture_state()
    rec = m.split_edge_at(edge, mid)
    after = m.capture_state()
    with pytest.raises(ValueError):
        m.revert_stitch({"kind": "split"})
    assert m.capture_state() == after
    m.revert_stitch(rec)
    assert m.capture_state() == before
    m.restore_state(after)
    assert m.capture_state() == after


def test_relink_hole_validates_all_keys_before_touching_incidence():
    m = _mesh(True)
    face = m.add_face(square(0, 0, 8), [square(2, 2, 1)])
    ring = list(face.hole_loops[0])
    edges = [m.find_edge(v, ring[(i + 1) % len(ring)]).key
             for i, v in enumerate(ring)]
    m.remove_hole(face, ring)
    before = m.capture_state()
    keys = [v.key for v in ring]
    foreign = _mesh(True).vertex(V(4, 4))
    with pytest.raises(KeyError):
        m._store.relink_hole(face.key, keys + [foreign.key])
    assert m.capture_state() == before
    change = m._store.relink_hole(face.key, keys)
    assert change.modified == {"vertex": [], "edge": edges, "face": [face.key]}
    assert all(not keys for keys in change.created.values())
    assert all(not keys for keys in change.removed.values())
    assert change.moved == []
    assert list(face.hole_loops[0]) == ring

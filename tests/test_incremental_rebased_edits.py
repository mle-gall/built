# SPDX-License-Identifier: GPL-3.0-or-later
"""Edits of a model far from the origin stay incremental.

A model in site coordinates is packed *rebased*: each prototype is baked in
a frame (anchor) of its own and the rows carry their matrix in the view
frame (``model_translations``). The resident diff refused any such pack, so
every geometry edit on a large real model — a line, an erase, a push — ran a
whole-scene pack and upload (seconds). It now writes the rows in the view
frame and rebakes the edited prototypes in the frames they already have.

Each check compares the resident pack after the edit with a fresh pack of
the same scene, drawn geometry in model coordinates per row."""
from __future__ import annotations

import numpy as np
import pytest
from built_core import Matrix4 as QMatrix4x4, Vec3 as V
from built_core import InstanceUpdates, pack_scene
from built_core.group import Group
from built_core.history import (AddEdgeCommand, History, InsertGroupCommand, MoveGroupCommand,
                                SnapshotMutation)
from built_core.mesh import Mesh
from built_core.scene import Scene

FAR = 5000.0


def moved(x, y, z=0.0):
    m = QMatrix4x4()
    m.translate(x, y, z)
    return m


def tile(mesh, x0, y0, n=3, z=0.0):
    for i in range(n):
        for j in range(n):
            x, y = x0 + i, y0 + j
            mesh.add_face([V(x, y, z), V(x + 1, y, z), V(x + 1, y + 1, z), V(x, y + 1, z)])


def far_scene():
    """Loose tiles at the site, a placed group with its own tiles, a shared
    part placed twice (one painted) and a nested container."""
    scene = Scene()
    tile(scene.mesh, FAR, 0.0)
    slab = Group(name="slab")
    tile(slab.mesh, 0.0, 0.0)
    slab.xform = moved(FAR + 10, 0.0)
    part = Mesh()
    tile(part, 0.0, 0.0, n=1)
    a, b = Group(part, name="a"), Group(part, name="b")
    a.xform, b.xform = moved(1.0, 0.0), moved(3.0, 0.0)
    b.material = {"color": [0.8, 0.2, 0.2]}
    block = Group(name="block")
    block.adopt([a, b])
    block.xform = moved(FAR - 20, 5.0)
    scene.groups.extend([slab, block])
    return scene, slab, block, a, b


def drawn(packed):
    """uid -> the visible triangles' corners of its row, in model
    coordinates, rounded and sorted."""
    origin = np.asarray(packed.origin, np.float64)
    out = {}
    for row, uid in enumerate(packed.uids):
        info = packed.instances["info"][row]
        if info[2] & 1:
            continue
        code = int(info[0])
        c = packed.commands[code]
        first, count, base = int(c["first"]), int(c["count"]), int(c["base"])
        local = packed.vertices["pos"][base + packed.indices[first:first + count].astype(np.int64)].astype(np.float64)
        world = packed.instances["world"][row].astype(np.float64)
        points = local @ world[:3, :3] + world[3, :3] + origin
        out[uid] = sorted(map(tuple, np.round(points, 3)))
    return out


def assert_draws_like_a_fresh_pack(updates, scene):
    fresh = pack_scene(scene)
    assert fresh.model_translations is not None
    live = {uid: tris for uid, tris in drawn(updates.packed).items()
            if updates.resident.is_active(uid)}
    assert live == drawn(fresh)
    # The rows' model-frame translations match the bytes written.
    packed = updates.packed
    rows = np.flatnonzero((packed.instances["info"][:, 2] & 1) == 0)
    shifted = packed.instances["world"][rows, 3, :3].astype(np.float64) + np.asarray(packed.origin)
    np.testing.assert_allclose(shifted, packed.model_translations[rows], atol=1e-2)


@pytest.fixture
def far():
    scene, slab, block, a, b = far_scene()
    updates = InstanceUpdates(scene, pack_scene(scene))
    history = History(scene)
    updates.observe(history)
    assert updates.packed.model_translations is not None, "the fixture packs rebased"
    return scene, history, updates, slab, block, a, b


def step(updates, scene):
    delta = updates.diff(scene)
    assert delta is not None, "a whole-scene pack"
    updates.apply(*delta)
    assert_draws_like_a_fresh_pack(updates, scene)
    return delta


def test_a_line_on_the_loose_mesh_rebakes_it_alone(far, monkeypatch):
    scene, history, updates, *_ = far
    calls = []
    original = pack_scene
    monkeypatch.setattr("built_core.pack_scene", lambda s, **kw: (calls.append(len(s.groups)), original(s, **kw))[1])
    history.execute(AddEdgeCommand(V(FAR + 0.5, 0.5, 0), V(FAR + 2.5, 0.5, 0)))
    step(updates, scene)
    assert calls == [1], "one prototype rebaked, not the scene"
    assert len(updates.changed) == 1
    history.undo()
    step(updates, scene)
    history.redo()
    step(updates, scene)


def test_an_edit_inside_a_placed_group(far):
    scene, history, updates, slab, *_ = far
    scene.begin_group_edit(slab)
    history.execute(SnapshotMutation(lambda s: s.mesh.move_vertex(s.mesh.vertices[0], V(0.0, 0.0, 0.5))))
    scene.end_group_edit()
    step(updates, scene)
    history.undo()
    step(updates, scene)


def test_moving_a_group_writes_its_rows_in_the_view_frame(far):
    scene, history, updates, slab, block, *_ = far
    before = len(updates.packed.commands)
    history.execute(MoveGroupCommand(block, V(2.0, 1.0, 0.0)))
    ids, _ = step(updates, scene)
    assert len(ids) == 2 and not updates.changed
    assert len(updates.packed.commands) == before
    history.undo()
    step(updates, scene)


def test_a_new_far_group_gets_a_prototype_in_its_own_frame(far):
    scene, history, updates, *_ = far
    added = Group(name="added")
    tile(added.mesh, FAR + 40.0, 30.0, n=2)
    history.execute(InsertGroupCommand(added))
    step(updates, scene)
    code = int(updates.packed.instances["info"][-1][0])
    assert any(updates.packed.prototype_origins[code]), "baked in a frame near it"
    assert updates.remote, "the tools learn its frame"
    history.undo()
    step(updates, scene)
    history.redo()
    step(updates, scene)


def test_a_mesh_moved_out_of_reach_of_its_frame_repacks(far):
    scene, history, updates, slab, *_ = far
    scene.begin_group_edit(slab)
    history.execute(SnapshotMutation(lambda s: [s.mesh.move_vertex(v, V(0.0, 3000.0, 0.0))
                                                for v in list(s.mesh.vertices)]))
    scene.end_group_edit()
    assert updates.diff(scene) is None


def _contour(seed):
    """A circle tagged as a curve with a few edges attached."""
    import math
    import random
    rng = random.Random(seed)
    mesh = Mesh()
    n = rng.randint(12, 36)
    points = [V(FAR + 2 * math.cos(math.tau * i / n), 2 * math.sin(math.tau * i / n), 3) for i in range(n)]
    mesh.add_face(points)
    mesh.tag_curve(points, closed=bool(seed % 3))
    for i in rng.sample(range(n), 5):
        v = points[i]
        mesh.add_edge(v, v + (V(0, 0, 1) if i % 2 else V(0.2, -0.12, 0)))
    return mesh


@pytest.mark.parametrize("matrix", [None, "turned"])
def test_curves_split_by_the_store_in_a_tool_frame_as_by_the_edges(monkeypatch, matrix):
    """In the tools' frame (the view frame of a far model, an open group's
    placement) the store splits the curves itself, as walking the edge views
    there did (~0.2-0.3 s per line on a 200k-face mesh)."""
    import math
    from built_core import mesh as mesh_api
    from built_core.coordinate_frame import mesh_frame
    if matrix == "turned":
        c, s = math.cos(0.5), math.sin(0.5)
        matrix = (c, s, 0, 0, -s, c, 0, 0, 0, 0, 1, 0, FAR + 3.0, -2.0, 1.0, 1)
    kw = {} if matrix is None else {"matrix": matrix}
    for seed in range(12):
        results = []
        for native in (True, False):
            monkeypatch.setattr(mesh_api, "_CURVE_COUNTER", 1000)
            mesh = _contour(seed)
            edges = list(mesh.edges)
            with mesh_frame(mesh, (FAR, 0.0, 0.0), **kw):
                assert not mesh._store._model_frame
                if native:
                    mesh.resplit_curves()
                else:
                    mesh_api._CURVE_COUNTER = mesh_api.object_drawing.resplit_curves(
                        list(mesh.edges), mesh_api._CURVE_COUNTER)
            results.append(([e.curve for e in edges], mesh_api._CURVE_COUNTER))
        assert results[0] == results[1]

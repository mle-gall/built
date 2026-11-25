# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The core protocol (docs/core-protocol.md): the change feed History
publishes, its keys, the keyed commands and queries, the renderer's use
of the feed and the agent bridge's tools.

The feed's per-mesh record on a store-backed mesh is checked against an
oracle read through the mesh itself before and after each step (keys,
positions, ends, faces, rings, edge sets): created / removed / moved /
modified must be exactly what changed, and an undo exactly the inverse."""
from __future__ import annotations
from ingetrazo_core import store_views

import json
import random
import threading

import numpy as np
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V

import core._native as _native
from core import mesh as M
from core import store_mesh as S
from core.history import (AssignLayerCommand, History, InsertGroupCommand,
                          SetFaceColorCommand)
from core.protocol import commands as C
from core.protocol import queries as Q
from core.protocol.keys import mesh_key, resolve_mesh
from core.scene import Scene

mod = _native.available()
needs_store = pytest.mark.skipif(
    mod is None or not hasattr(getattr(mod, "StoreSnapshot", None), "changes_since"),
    reason="native module without StoreSnapshot.changes_since")



def _scene():
    sc = Scene()
    sc.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    return sc


def _read(m):
    """What the mesh shows, by key (the oracle)."""
    vs = {v.key: (v.position_tuple(), frozenset(e.key for e in v.edges))
          for v in m.vertices}
    es = {e.key: ((e.v0.key, e.v1.key), bool(e.soft), bool(e.hidden),
                  e.curve, e.layer, tuple(f.key for f in e.faces))
          for e in m.edges}
    fs = {f.key: (tuple(v.key for v in f.loop),
                  tuple(tuple(v.key for v in h) for h in (f.hole_loops or ())),
                  bool(f.interior))
          for f in m.faces}
    return vs, es, fs


def _expected(before, after):
    out = {"created": {}, "removed": {}, "modified": {}, "moved": set()}
    for kind, a, b in zip(("vertex", "edge", "face"), before, after):
        out["created"][kind] = set(b) - set(a)
        out["removed"][kind] = set(a) - set(b)
        both = set(a) & set(b)
        if kind == "vertex":
            out["moved"] = {k for k in both if a[k][0] != b[k][0]}
            out["modified"][kind] = {k for k in both if a[k][1] != b[k][1]}
        else:
            out["modified"][kind] = {k for k in both if a[k] != b[k]}
    return out


def _got(change):
    return {"created": {k: set(v) for k, v in change.created.items()},
            "removed": {k: set(v) for k, v in change.removed.items()},
            "modified": {k: set(v) for k, v in change.modified.items()},
            "moved": set(change.moved)}


def _inverse(rec):
    return {**rec, "created": rec["removed"], "removed": rec["created"]}


@needs_store
def test_store_steps_report_exactly_what_changed_and_undo_the_inverse():
    """Move, push/pull and erase through the keyed commands on a
    store-backed mesh, each undone and redone: every step's record is the
    oracle's difference, undo its inverse, redo the step again; the mesh
    never leaves the store."""
    sc = _scene()
    h = History(sc)
    m = sc.mesh
    face = m.faces[0]
    # A push makes a box; then a move of its top and an erase of an edge.
    steps = [("push_pull", lambda: dict(face=str(face.key), distance=1.0))]
    steps.append(("move_vertices", lambda: dict(
        vertices=[str(v.key) for v in sc.mesh.vertices
                  if v.position.z() > 0.5], delta=[0.0, 0.0, 0.5])))
    steps.append(("erase", lambda: dict(edges=[str(next(
        e.key for e in sc.mesh.edges
        if e.v0.position.z() > 0.5 and e.v1.position.z() > 0.5))])))
    for name, args in steps:
        before = _read(sc.mesh)
        r = C.run(h, name, **args())
        assert r["ok"], (name, r)
        after = _read(sc.mesh)
        assert type(sc.mesh) is S.StoreMesh, name
        step = r["step"]
        assert (step.phase, step.revision) == ("execute", h.revision)
        assert [c.key for c in step.meshes] == [mesh_key(sc.mesh)]
        change = step.meshes[0]
        assert change.backing == "store" and not change.replaced
        want = _expected(before, after)
        assert _got(change) == want, name
        assert any(want["created"].values()) or want["moved"] or any(
            want["removed"].values())
        undo = C.run(h, "undo")["step"]
        assert undo.phase == "undo"
        assert _read(sc.mesh) == before
        assert _got(undo.meshes[0]) == _inverse(want), name
        redo = C.run(h, "redo")["step"]
        assert redo.phase == "redo"
        assert _read(sc.mesh) == after
        assert _got(redo.meshes[0]) == want, name
        assert type(sc.mesh) is S.StoreMesh, name
    assert [(s.command, s.phase) for s in h.feed.since(0)["steps"]] == [
        (c, p) for c in ("SnapshotMutation", "MoveVerticesCommand",
                         "EraseSelectionCommand")
        for p in ("execute", "undo", "redo")]


@needs_store
@pytest.mark.parametrize("seed", range(8))
def test_store_changes_since_matches_the_oracle_over_random_ops(seed):
    rng = random.Random(seed)
    sm = M.Mesh()
    snapshots = []
    removed = []
    def point():
        return V(rng.randrange(5), rng.randrange(5), rng.randrange(2))
    for step in range(140):
        snap = sm._store.snapshot()
        before = _read(sm)
        op = rng.randrange(8)
        if op == 0 and sm.faces:
            face = rng.choice(list(sm.faces))
            sm.remove_face(face)
            removed.append(face)
        elif op == 1 and removed:
            sm.relink_face(removed.pop())
        elif op == 2 and sm.vertices:
            sm.move_vertex(rng.choice(list(sm.vertices)), V(0, 0, 0.25))
        elif op == 3 and sm.edges:
            rng.choice(list(sm.edges)).soft = bool(step % 2)
        elif op == 4:
            snapshots.append(sm.capture_state())
        elif op == 5 and snapshots:
            sm.restore_state(rng.choice(snapshots))
            removed.clear()
        elif op == 6:
            sm.add_face([point(), point(), point()])
        else:
            a, b = point(), point()
            if a == b:
                with pytest.raises(ValueError, match="degenerate edge"):
                    sm.add_edge(a, b)
            else:
                sm.add_edge(a, b)
        after = _read(sm)
        ch = sm._store.snapshot().changes_since(snap, [])
        assert _got(ch) == _expected(before, after), (seed, step)


@needs_store
def test_push_and_undo_report_the_store():
    sc = _scene()
    h = History(sc)
    face = sc.mesh.faces[0]
    got = C.run(h, "push_pull", face=str(face.key), distance=1.0)
    assert got["ok"]
    (change,) = got["step"].meshes
    assert change.backing == "store"
    assert change.key == mesh_key(sc.mesh)
    got = C.run(h, "undo")
    (change,) = got["step"].meshes
    assert change.backing == "store" and not change.replaced


@needs_store
def test_paint_reports_the_faces_and_nothing_else():
    sc = _scene()
    h = History(sc)
    f = sc.mesh.faces[0]
    h.execute(SetFaceColorCommand([f], [1.0, 0.0, 0.0]))
    (change,) = h.feed.last.meshes
    assert change.modified["face"] == [f.key]
    assert not change.replaced and not change.moved
    h.undo()
    assert h.feed.last.meshes[0].modified["face"] == [f.key]


@needs_store
def test_store_attrs_edits_are_face_modifications(monkeypatch):
    """Face attrs live beside the store: an edit by any command shows as
    a modified face (StoreState.changes_since); past ATTRS_COPY_MAX the
    feed no longer copies them, and paint commands still name theirs."""
    from core.history import SnapshotMutation
    from core.protocol import feed as F
    sc = _scene()
    h = History(sc)
    f = sc.mesh.faces[0]

    def stamp(scene):
        scene.mesh.faces[0].attrs["mat"] = "Oak"
    h.execute(SnapshotMutation(stamp))
    (change,) = h.feed.last.meshes
    assert change.modified["face"] == [f.key] and not change.replaced
    monkeypatch.setattr(F, "ATTRS_COPY_MAX", -1)
    h.execute(SetFaceColorCommand([f], [0.0, 1.0, 0.0]))
    assert h.feed.last.meshes[0].modified["face"] == [f.key]
    assert type(sc.mesh) is S.StoreMesh


def test_document_changes_by_key():
    from core.layers import Layer
    sc = _scene()
    h = History(sc)
    g = M.Mesh()
    g.add_face([V(5, 0, 0), V(6, 0, 0), V(6, 1, 0)])
    from core.group import Group
    group = Group(g, name="Box")
    h.execute(InsertGroupCommand(group))
    step = h.feed.last
    assert step.placements["created"] == [group.uid] and step.complete
    assert not step.meshes          # the open mesh did not change
    h.undo()
    assert h.feed.last.placements["removed"] == [group.uid]
    h.redo()
    layer = Layer("Walls")
    sc.layers.append(layer)
    h.execute(AssignLayerCommand([group], layer.key))
    step = h.feed.last
    assert step.placements["modified"] == [group.uid]
    # The tag was added outside History: the next step sees no change of
    # it; renaming it inside one does.
    assert step.tags == {"created": [], "removed": [], "modified": []}
    from core.history import SnapshotMutation

    def rename(scene):
        layer.name = "Muros"
    h.execute(SnapshotMutation(rename))
    assert h.feed.last.tags["modified"] == [layer.key]


def test_since_and_the_ring():
    sc = _scene()
    h = History(sc)
    for i in range(3):
        C.run(h, "move_vertices", vertices=[str(sc.mesh.vertices[0].key)],
              delta=[0, 0, 0.1])
    got = h.feed.since(1)
    assert got["complete"] and [s.revision for s in got["steps"]] == [2, 3]
    for _ in range(h.feed.maxlen - 2):
        if h.undo_stack:
            h.undo()
        else:
            h.redo()
    assert not h.feed.since(0)["complete"] and h.feed.since(1)["complete"]
    with pytest.raises(ValueError):
        h.feed.since(h.revision + 1)
    # JSON-safe, keys as strings.
    text = json.dumps(Q.feed(h, 0), allow_nan=False)
    assert '"steps"' in text


def test_a_failed_or_rejected_step_publishes_nothing():
    from core.history import SnapshotMutation
    sc = _scene()
    h = History(sc)

    def boom(scene):
        scene.mesh.move_vertex(scene.mesh.vertices[0], V(0, 0, 1))
        raise RuntimeError("no")
    h.error_log = "/dev/null"
    h.execute(SnapshotMutation(boom))
    assert h.last_error and h.feed.last is None and h.revision == 0


def test_feed_subscribers_and_a_failing_one():
    sc = _scene()
    h = History(sc)
    seen = []

    def bad(step):
        raise RuntimeError("consumer bug")
    h.feed.subscribe(bad)
    off = h.feed.subscribe(seen.append)
    C.run(h, "move_vertices", vertices=[str(sc.mesh.vertices[0].key)], delta=[0, 0, 1])
    assert [s.revision for s in seen] == [1]
    off()
    C.run(h, "undo")
    assert len(seen) == 1


@needs_store
def test_queries_name_faces_by_key():
    sc = _scene()
    m = sc.mesh
    f = m.faces[0]
    info = Q.mesh_info(sc)
    assert info["mesh"] == mesh_key(m) and (info["faces"], info["edges"]) == (1, 4)
    assert info["backing"] == "store"
    got = Q.faces(sc, [str(f.key), "12345"])
    (face,) = got["faces"]
    assert face["rings"][0][1] == [2.0, 0.0, 0.0]
    assert face["corners"][0] == [str(v.key) for v in f.loop]
    assert got["missing"] == ["12345"]
    e = m.edges[0]
    (edge,) = Q.edges(sc, [str(e.key)])["edges"]
    assert edge["faces"] == [str(f.key)]
    sel = Q.select(sc, faces=[str(f.key)], edges=[str(e.key)])
    assert sel["face"] == [str(f.key)] and sel["edge"] == [str(e.key)]
    assert Q.selection(sc)["mesh"] == mesh_key(m)
    assert Q.select(sc, mode="clear")["face"] == []
    assert resolve_mesh(sc, mesh_key(m)) is m and resolve_mesh(sc, "active") is m
    assert type(m) is S.StoreMesh


def test_unknown_keys_refuse_before_running():
    sc = _scene()
    h = History(sc)
    with pytest.raises(KeyError):
        C.run(h, "erase", edges=["1"])
    with pytest.raises(ValueError):
        C.run(h, "explode")
    assert h.revision == 0


@needs_store
def test_renderer_reads_the_feed_for_a_store_mesh():
    """InstanceUpdates: a mesh edit inside an open group is found from
    the feed's step (not from the command class) and the element records
    travel with the rebake."""
    from core.group import Group
    from core.history import SnapshotMutation
    from ingetrazo_core import pack_scene
    from ingetrazo_core import InstanceUpdates
    mesh = M.Mesh()
    store_views.become_store(mesh, mod.MeshStore(next(M._KEY_NAMESPACES), **S._weld_args()), [])
    mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(0, 3, 0)])
    # A classic group: its own mesh is edited in place (a component
    # instance would be edited through a world copy).
    scene = Scene(groups=[Group(mesh), Group(M.Mesh())])
    updates = InstanceUpdates(scene, pack_scene(scene))
    scene.begin_group_edit(scene.groups[0])
    updates.apply(*updates.diff(scene))
    history = History(scene)
    updates.observe(history)
    edited = scene.mesh
    assert edited is mesh and type(edited) is S.StoreMesh
    v = edited.vertices[0]
    history.execute(SnapshotMutation(lambda s: s.mesh.move_vertex(
        s.mesh.vertices[0], V(0, 0, 1))))
    updates.apply(*updates.diff(scene))
    assert len(updates.changed) == 1 and updates.changed[0][0] is edited
    (records,) = updates.changed_records.values()
    assert records[-1].moved == [v.key]
    history.undo()
    updates.apply(*updates.diff(scene))
    assert len(updates.changed) == 1
    assert list(updates.changed_records.values())[0][-1].moved == [v.key]


@needs_store
def test_renderer_undo_after_leaving_the_group_rebakes_it():
    """Edit inside a group, leave it, undo: the undo ran on the group's
    mesh, which the feed names. Derived from the command class (the open
    mesh, now the loose one) the resident pack kept the edit drawn."""
    from core.group import Group
    from core.history import SnapshotMutation
    from tests.test_wgpu_updates import assert_matches_fresh_pack
    from ingetrazo_core import pack_scene
    from ingetrazo_core import InstanceUpdates
    mesh = M.Mesh()
    mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(0, 3, 0)])
    scene = Scene(groups=[Group(mesh), Group(M.Mesh())])
    updates = InstanceUpdates(scene, pack_scene(scene))
    scene.begin_group_edit(scene.groups[0])
    updates.apply(*updates.diff(scene))
    history = History(scene)
    updates.observe(history)
    history.execute(SnapshotMutation(lambda s: s.mesh.move_vertex(
        s.mesh.vertices[0], V(0, 0, 1))))
    updates.apply(*updates.diff(scene))
    scene.end_group_edit()
    updates.apply(*updates.diff(scene))
    history.undo()
    updates.apply(*updates.diff(scene))
    assert updates.last_local and [c[0] for c in updates.changed] == [mesh]
    assert_matches_fresh_pack(updates, scene)


@pytest.fixture
def bridge(monkeypatch, native_window):
    from plugins.ai_bridge import _Bridge
    monkeypatch.setenv("INGETRAZO_AI_PORT", "0")
    win = native_window
    scene = win.viewport.scene
    scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    b = _Bridge(win.viewport)
    yield win, b
    b.stop()


def _call(b, tool, **args):
    job = dict(req=dict(id=1, tool=tool, args=args), done=threading.Event())
    b._run_on_main(job)
    assert job["reply"]["ok"], job["reply"]
    return job["reply"]["result"]


def test_bridge_tools_name_faces_by_key_and_read_the_feed(bridge):
    win, b = bridge
    vp = win.viewport
    names = {t["name"] for t in _call(b, "list_tools")["tools"]}
    assert {"query_feed", "query_mesh", "query_faces", "query_edges",
            "query_vertices", "query_selected_elements",
            "select_elements"} <= names
    info = _call(b, "query_mesh")
    face_key = str(vp.scene.mesh.faces[0].key)
    got = _call(b, "query_faces", keys=[face_key], mesh=info["mesh"])
    assert got["faces"][0]["key"] == face_key
    sel = _call(b, "select_elements", faces=[face_key])
    assert sel["face"] == [face_key]
    assert _call(b, "query_selected_elements")["face"] == [face_key]
    rev = _call(b, "query_feed")["revision"]
    C.run(vp.history, "push_pull", face=face_key, distance=0.5)
    feed = _call(b, "query_feed", since_revision=rev)
    (step,) = feed["steps"]
    assert step["meshes"][0]["mesh"] == info["mesh"]
    json.dumps(feed, allow_nan=False)


def test_placement_commands_reach_nested_placements():
    """hide / unhide / lock / rename_placement / set_tag (added for the
    egui UI) take uids at any depth and answer with the feed's step."""
    from core.group import Group
    from core.layers import Layer
    sc = _scene()
    h = History(sc)
    inner_mesh = M.Mesh()
    inner_mesh.add_face([V(5, 0, 0), V(6, 0, 0), V(6, 1, 0)])
    inner = Group(inner_mesh, name="Bolt")
    outer = Group(M.Mesh(), name="Frame")
    outer.children = [inner]
    h.execute(InsertGroupCommand(outer))
    out = C.run(h, "hide", placements=[inner.uid])
    assert out["ok"] and inner.hidden and inner.uid in out["step"].placements["modified"]
    assert C.run(h, "unhide", placements=[inner.uid])["ok"] and not inner.hidden
    out = C.run(h, "lock", placements=[inner.uid])
    assert out["ok"] and inner.locked and out["step"].placements["modified"] == [inner.uid]
    assert C.run(h, "lock", placements=[inner.uid], locked=False)["ok"] and not inner.locked
    out = C.run(h, "rename_placement", placement=inner.uid, name="M8 bolt")
    assert out["ok"] and inner.name == "M8 bolt"
    assert out["step"].placements["modified"] == [inner.uid] and out["step"].complete
    layer = Layer("Fasteners")
    sc.layers.append(layer)
    assert C.run(h, "set_tag", placements=[inner.uid], tag=layer.key)["ok"]
    assert inner.layer == layer.key
    h.undo()
    assert inner.layer != layer.key
    with pytest.raises(KeyError):
        C.run(h, "hide", placements=["nope"])
    with pytest.raises(KeyError):
        C.run(h, "set_tag", placements=[inner.uid], tag="no-such-tag")


def test_resegment_curve_and_repair_solid():
    from core.input_codes import Input
    from ingetrazo_core import ScreenPoint
    from tools.base import ToolContext
    from tools.circle import CircleTool
    from tests.controller_surface import ControllerSurface as Viewport
    from core.protocol.keys import to_json_key
    vp = Viewport(None)
    vp.flash_status = lambda *a, **k: None
    tool = CircleTool()
    tool.on_activate(vp)
    for p in (V(0, 0, 0), V(2, 0, 0)):
        tool.on_click(ToolContext(viewport=vp, world=p, screen=ScreenPoint(0, 0),
                                  modifiers=Input.NoModifier, snap=None))
    edge = next(e for e in vp.scene.mesh.edges if e.curve is not None)
    before = sum(1 for e in vp.scene.mesh.edges if e.curve is not None)
    out = C.run(vp.history, "resegment_curve", edge=to_json_key(edge.key), segments=before * 2)
    assert out["ok"]
    assert sum(1 for e in vp.scene.mesh.edges if e.curve is not None) == before * 2
    # An open box (no top) as a group: repair closes the flat hole.
    from core.group import Group
    box = M.Mesh()
    a, b, c, d = V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)
    up = V(0, 0, 1)
    box.add_face([a, d, c, b])
    for p, q in ((a, b), (b, c), (c, d), (d, a)):
        box.add_face([p, q, q + up, p + up])
    g = Group(box, name="Open box")
    vp.history.execute(InsertGroupCommand(g))
    out = C.run(vp.history, "repair_solid", placement=g.uid)
    assert out["ok"] and out["repaired"]["holes"] == 1


def test_bridge_scales_named_vertices_in_one_undo_step(bridge):
    win, b = bridge
    vp = win.viewport
    face = vp.scene.mesh.faces[-1]
    before = sorted(v.position.toTuple() for v in face.loop)
    depth = len(vp.history.undo_stack)
    result = _call(b, "scale_selection", anchor=[0, 0, 0], factors=[3, 1, 1],
                   vertices=[str(v.key) for v in face.loop])
    assert result["ok"] and result["step"]
    assert len(vp.history.undo_stack) == depth + 1
    assert sorted(v.position.toTuple() for v in face.loop) == sorted((3*x, y, z) for x, y, z in before)
    assert vp.history.undo()
    assert sorted(v.position.toTuple() for v in vp.scene.mesh.faces[-1].loop) == before


def test_bridge_rotates_named_vertices_in_one_undo_step(bridge):
    win, b = bridge
    vp = win.viewport
    face = vp.scene.mesh.faces[-1]
    before = sorted(v.position.toTuple() for v in face.loop)
    depth = len(vp.history.undo_stack)
    result = _call(b, "rotate_selection", center=[0, 0, 0], axis=[0, 0, 1], angles=[90],
                   vertices=[str(v.key) for v in face.loop])
    assert result["ok"] and result["step"]
    assert len(vp.history.undo_stack) == depth + 1
    actual = sorted(tuple(round(x, 9) for x in v.position.toTuple()) for v in face.loop)
    assert actual == sorted((-y, x, z) for x, y, z in before)
    assert vp.history.undo()
    assert sorted(v.position.toTuple() for v in vp.scene.mesh.faces[-1].loop) == before


def test_offset_uses_the_face_key_and_model_frame_for_a_distant_path():
    from core.protocol.commands import run
    from core.scene import Scene
    from core.history import History
    from ingetrazo_core import Vec3
    scene = Scene()
    history = History(scene)
    face = scene.mesh.add_face([Vec3(0, 0, 0), Vec3(4, 0, 0), Vec3(4, 4, 0), Vec3(0, 4, 0)])
    assert run(history, "offset_outline", face=str(face.key), distance=.5)["ok"]
    assert [f.area() for f in scene.mesh.faces if not f.holes] == pytest.approx([9])
    assert sum(bool(f.holes) for f in scene.mesh.faces) == 1
    assert history.undo()
    start = 1_000_000.
    assert run(history, "offset_outline", points=[[start, 0, 0], [start + 2, 0, 0],
                                                  [start + 2, 1, 0]],
               normal=[0, 0, 1], distance=.25)["ok"]
    coordinates = {v.position.toTuple() for v in scene.mesh.vertices}
    assert (start, .25, 0) in coordinates and (start + 1.75, 1, 0) in coordinates
    assert history.undo() and len(scene.mesh.vertices) == 4


def test_paint_material_uses_named_recipe_on_the_requested_side_and_preserves_it_on_undo():
    from core.protocol.commands import run
    from core.materials import Material
    from core.scene import Scene
    from core.history import History
    from ingetrazo_core import Vec3
    scene = Scene()
    history = History(scene)
    face = scene.mesh.add_face([Vec3(), Vec3(2, 0, 0), Vec3(0, 2, 0)])
    scene.materials["Blue glass"] = Material("Blue glass", color=(.1, .2, .8), opacity=.4)
    assert run(history, "paint_material", faces=[str(face.key)], material="Blue glass", back=True)["ok"]
    assert "color" not in face.attrs and face.attrs["back"]["opacity"] == .4
    assert face.attrs["back"]["mat"] == "Blue glass"
    assert history.undo() and dict(face.attrs) == {}
    assert "Blue glass" in scene.materials


def test_bridge_bevels_keyed_edges_and_undoes(bridge):
    from tests.test_fillet_tool import _box_scene, edge_at
    win, b = bridge
    vp = win.viewport
    mesh = _box_scene(vp)
    edge = edge_at(mesh, (0, 0, 1), (2, 0, 1))
    result = _call(b, "bevel_selection", edges=[str(edge.key)], radius=.1, segments=4)
    assert result["ok"] and result["step"]
    assert len(mesh.faces) == 10
    assert vp.history.undo() and len(mesh.faces) == 6


def test_bridge_mirrors_and_stamps_the_model_clipboard(bridge):
    from ingetrazo_core import Vec3
    win, b = bridge
    vp = win.viewport
    face = vp.scene.mesh.faces[-1]
    original = sorted(v.position.toTuple() for v in face.loop)
    count = len(vp.scene.mesh.faces)
    result = _call(b, "flip_selection", center=[0, 0, 0], axis=[1, 0, 0], faces=[str(face.key)])
    assert result["ok"] and result["step"]
    assert sorted(v.position.toTuple() for v in face.loop) == sorted((-x, y, z) for x, y, z in original)
    assert vp.history.undo()
    vp.clipboard = {"ref": Vec3(), "faces": [([Vec3(), Vec3(1, 0, 0), Vec3(0, 1, 0)], [])], "edges": []}
    result = _call(b, "paste_clipboard", offset=[8, 2, 1])
    assert result["ok"] and result["step"] and len(vp.scene.mesh.faces) == count + 1
    assert vp.history.undo() and len(vp.scene.mesh.faces) == count


def test_group_axes_verb_rejects_collinearity_and_preserves_world_points():
    from core.group import frame_axes, group_frame
    from core.history import History
    from core.protocol.commands import run
    from core.scene import Scene
    from tests.test_group_axes import _slab, _world_pts
    scene, group = Scene(), _slab()
    scene.groups.append(group)
    history = History(scene)
    original = _world_pts(group)
    args = dict(placement=group.uid, origin=[2, 0, 0], red=[2, 2, 0])
    with pytest.raises(ValueError):
        run(history, "set_group_axes", **args, green=[2, 1, 0])
    assert not history.undo_stack
    assert run(history, "set_group_axes", **args, green=[0, 3, 0])["ok"]
    assert tuple(frame_axes(group_frame(group))[1].toTuple()) == (0., 1., 0.)
    assert _world_pts(group) == original
    assert history.undo() and group_frame(group) is None

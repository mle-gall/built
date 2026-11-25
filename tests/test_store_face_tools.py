# SPDX-License-Identifier: GPL-3.0-or-later
"""Geometry, flags and history from the tool paths against the Python kernel."""
from ingetrazo_core import store_views
import math
from types import SimpleNamespace

import numpy as np
import pytest
from ingetrazo_core import Vec3 as V

from core import store_mesh as S
from core.history import History
from core.intersect import WITH_CONTEXT, segments_for
from core.intersect import build_intersect_faces
from core.mesh import _KEY_NAMESPACES
from core.scene import Scene
from tests.test_store_mesh import fingerprint
from tests.test_offset_regions import _VP, _offset
from tests.test_intersect_faces import _box, RED
from tools.followme import FollowMeTool



def scenes(monkeypatch):
    native = pytest.importorskip("ingetrazo_core")
    for backed in (False, True):
        scene = Scene()
        if backed:
            store_views.become_store(scene.mesh, native.MeshStore(next(_KEY_NAMESPACES), **S._weld_args()), [])
        yield scene


def history_result(scene, history, before):
    m = scene.mesh
    assert not history.last_error
    after = fingerprint(m)
    keys = [f.key for f in m.faces]
    assert after != before
    if isinstance(m, S.StoreMesh):
        cmd = history.undo_stack[-1]
        change = cmd.change
        for kind in ("vertex", "edge", "face"):
            old = set(cmd.before.snap.keys(kind))
            new = set(m._store.keys(kind))
            assert set(change.created[kind]) == new - old
            assert set(change.removed[kind]) == old - new
            assert set(change.modified[kind]) <= old & new
    history.undo()
    assert fingerprint(m) == before
    history.redo()
    assert fingerprint(m) == after
    assert [f.key for f in m.faces] == keys
    if isinstance(m, S.StoreMesh):
        m._store.check()
    return after


@pytest.mark.parametrize("shape,distance", [
    ("square", .25), ("square", -.25), ("ell", .25), ("hole", .25), ("hole", -.25)])
def test_offset(monkeypatch, shape, distance):
    results = []
    for scene in scenes(monkeypatch):
        xy = ([(0, 0), (4, 0), (4, 2), (2, 2), (2, 4), (0, 4)] if shape == "ell"
              else [(0, 0), (4, 0), (4, 4), (0, 4)])
        holes = [[V(x, y, 0) for x, y in [(1, 1), (1, 2), (2, 2), (2, 1)]]]
        face = scene.mesh.add_face([V(x, y, 0) for x, y in xy], holes if shape == "hole" else [])
        face.attrs["finish"] = "stone"
        vp = _VP(scene)
        before = fingerprint(scene.mesh)
        _offset(vp, face, distance)
        results.append(history_result(scene, vp.history, before))
    assert results[1] == results[0]


@pytest.mark.parametrize("shape", ["open", "closed", "arc"])
def test_sweep(monkeypatch, shape):
    results = []
    for scene in scenes(monkeypatch):
        path = [V(0, 0, 0), V(0, 4, 0), V(4, 4, 0)]
        if shape != "open":
            path = [V(4 * math.cos(i * math.pi / 6), 4 * math.sin(i * math.pi / 6), 0)
                    for i in range(12 if shape == "closed" else 5)]
        face = scene.mesh.add_face([path[0] + V(x, 0, z) for x,z in [(0,0),(1,0),(1,1),(0,1)]])
        if shape == "arc":
            import core.mesh
            monkeypatch.setattr(core.mesh, "_CURVE_COUNTER", 300)
            for p, q in zip(path, path[1:]):
                scene.mesh.add_edge(p, q)
            scene.mesh.tag_curve(path, closed=False)
        vp = SimpleNamespace(scene=scene, history=History(scene), set_hover=lambda *_: None,
                             update=lambda: None, flash_status=lambda *_: None)
        before = fingerprint(scene.mesh)
        FollowMeTool()._commit(vp, face, path, shape == "closed")
        results.append(history_result(scene, vp.history, before))
    py, st = results
    assert py["vdeg"] == st["vdeg"] and py["e"] == st["e"]
    np.testing.assert_allclose(st["v"], py["v"], atol=2e-6, rtol=0)
    for a, b in zip(py["f"], st["f"], strict=True):
        assert a[:4] == b[:4]
        np.testing.assert_allclose(a[4], b[4], atol=2e-6, rtol=0)
        assert a[5] == pytest.approx(b[5], abs=2e-6)


@pytest.mark.parametrize("shape", ["boxes", "plane", "coplanar"])
def test_intersect(monkeypatch, shape):
    results = []
    for scene in scenes(monkeypatch):
        m = scene.mesh
        box = _box(0, 0, 0, 2, 2, 2, RED)
        if shape == "coplanar":
            m.add_face(box.mesh.faces[1].vertices)
            selected = [m.add_face([p + V(1, 1, 0) for p in box.mesh.faces[1].vertices])]
        else:
            for f in box.mesh.faces:
                m.add_face(f.vertices)
            if shape == "boxes":
                selected = [m.add_face([v + V(1, 1, 1) for v in f.vertices]) for f in box.mesh.faces]
            else:
                selected = [m.add_face([V(-1,-1,1), V(3,-1,1), V(3,3,1), V(-1,3,1)])]
        scene.selection = selected
        segs = segments_for(scene, WITH_CONTEXT)
        if shape == "coplanar":
            assert not segs
            continue
        assert len(segs) == (6 if shape == "boxes" else 4)
        hist = History(scene)
        before = fingerprint(m)
        # Preserve the source order used by the differential query.
        scene.selection = selected
        count, command = build_intersect_faces(
            scene, WITH_CONTEXT, [f.key for f in selected] if isinstance(m, S.StoreMesh) else None)
        assert count == len(segs)
        scene.selection = set(selected)
        hist.execute(command)
        results.append(history_result(scene, hist, before))
    if results:
        assert results[1] == results[0]


def test_tilted_plates_match_order_and_float_endpoints(monkeypatch):
    import random
    from tests.test_intersect_native import _plates
    from core.intersect import WITH_SELECTION

    plates = _plates(random.Random(104), 50)
    results = []
    for scene in scenes(monkeypatch):
        scene.selection = [scene.mesh.add_face(f.vertices) for f in plates.mesh.faces]
        results.append([(a.toTuple(), b.toTuple()) for a, b in segments_for(scene, WITH_SELECTION)])
    assert results[0]
    np.testing.assert_allclose(results[1], results[0], atol=5e-6, rtol=0)


def test_transformed_group_crossings_ignore_foreign_keys(monkeypatch):
    from ingetrazo_core import Matrix4 as QMatrix4x4
    from core.intersect import WITH_MODEL

    results = []
    for scene in scenes(monkeypatch):
        for f in _box(0, 0, 0, 2, 2, 2, RED).mesh.faces:
            scene.mesh.add_face(f.vertices)
        group = _box(0, 0, 0, 2, 2, 2, RED)
        matrix = QMatrix4x4()
        matrix.translate(1, 1, 1)
        group.xform = matrix
        scene.groups.append(group)
        scene.selection = {group}
        pairs = segments_for(scene, WITH_MODEL)
        results.append([(a.toTuple(), b.toTuple()) for a, b in pairs])
    assert len(results[0]) == 6 and results[1] == results[0]


def test_rejected_sweep_preserves_snapshot(monkeypatch):
    from core.sweep import build_sweep_face

    scene = list(scenes(monkeypatch))[-1]
    mesh = scene.mesh
    face = mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 0, 1)])
    path = [V(0, 0, 0), V(0, 1, 0), V(0, 0, 0)]
    before = mesh.capture_state()
    history = History(scene)
    history.execute(build_sweep_face(mesh, face.key, path, False))
    assert history.last_rejected and not history.undo_stack
    assert before == mesh.capture_state()


def test_direct_offset_record_and_hole_refusal(monkeypatch):
    from core.offset import build_offset_face, OffsetHoleCollision

    scene = list(scenes(monkeypatch))[-1]
    mesh = scene.mesh
    outer = [V(x, y, 0) for x, y in [(0, 0), (4, 0), (4, 4), (0, 4)]]
    hole = [V(x, y, 0) for x, y in [(1, 1), (2, 1), (2, 2), (1, 2)]]
    face = mesh.add_face(outer, [hole])
    before = mesh.capture_state()
    with pytest.raises(OffsetHoleCollision):
        build_offset_face(mesh, face.key, 1.0)
    assert before == mesh.capture_state()
    edges = set(mesh._store.keys("edge"))
    command = build_offset_face(mesh, face.key, .25)
    change = command.apply(scene)
    assert change.removed["face"] == [face.key]
    assert len(change.created["face"]) == 2
    assert edges <= set(mesh._store.keys("edge"))
    assert edges <= set(change.modified["edge"])
    command.undo(scene)
    assert before == mesh.capture_state()


def _check_feed(step, before):
    from tests.test_core_protocol import _read, _got, _expected

    assert step.complete
    assert {c.mesh for c in step.meshes} == set(before)
    for change in step.meshes:
        if isinstance(change.mesh, S.StoreMesh):
            assert not change.replaced
            assert _got(change) == _expected(before[change.mesh], _read(change.mesh))
        else:
            assert change.replaced


def test_reverse_profile_paints_outward_faces_only(monkeypatch):
    from tests.test_core_protocol import _read
    from core.protocol.commands import run
    for scene in scenes(monkeypatch):
        m, hist = scene.mesh, History(scene)
        profile = m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 0, 1), V(0, 0, 1)])
        profile.attrs["color"] = RED
        other = m.add_face([p + V(5, 0, 0) for p in profile.vertices])
        untouched = {}
        for f in _box(9, 0, 0, 10, 1, 1, RED).mesh.faces:
            inward = m.add_face(f.vertices[::-1])
            untouched[inward.key] = inward.normal().toTuple()
        before = {m: _read(m)}
        step = run(hist, "follow_me", face=profile.key, path=[[0, 0, 0], [0, 3, 0]])["step"]
        _check_feed(step, before)
        center = V(.5, 1.5, .5)
        shell = {f.key: f.normal().toTuple() for f in m.faces
                 if f.key != other.key and f.key not in untouched}
        assert len(shell) == 6
        for f in m.faces:
            if f.key in shell:
                assert V.dotProduct(f.normal(), f.centroid() - center) > 0
                assert f.attrs["color"] == RED
        before = {m: _read(m)}
        step = run(hist, "follow_me", face=other.key, path=[[5, 0, 0], [5, 2, 0]])["step"]
        _check_feed(step, before)
        assert {f.key: f.normal().toTuple() for f in m.faces if f.key in shell} == shell
        assert {f.key: f.normal().toTuple() for f in m.faces if f.key in untouched} == untouched
        assert not (set(step.meshes[0].modified["face"]) & (shell.keys() | untouched.keys()))


@pytest.mark.parametrize("placed", [False, True])
def test_piercing_slab_partitions_both_solids(monkeypatch, placed):
    from core.group import Group
    from core.mesh import Mesh
    from tests.test_core_protocol import _read
    from core.protocol.commands import run
    from ingetrazo_core import Matrix4 as QMatrix4x4

    for scene in scenes(monkeypatch):
        hist = History(scene)
        targets, groups, slab = [], [], []
        for i, box in enumerate((_box(0, 0, 0, 2, 2, 2, RED),
                                 _box(-.5, .5, .5, 2.5, 1.5, 1.5, [0, 0, 1]))):
            mesh = scene.mesh
            if placed:
                mesh = Mesh()
                if isinstance(scene.mesh, S.StoreMesh):
                    import ingetrazo_core as native
                    store_views.become_store(mesh, native.MeshStore(next(_KEY_NAMESPACES), **S._weld_args()), [])
                group = Group(mesh)
                group.xform = QMatrix4x4()
                group.xform.translate(i, 0, 0)
                groups.append(group)
                scene.groups.append(group)
            targets.append(mesh)
            for f in box.mesh.faces:
                new = mesh.add_face([p - V(i if placed else 0, 0, 0) for p in f.vertices])
                new.attrs = dict(f.attrs)
                if i:
                    slab.append(new.key)
        before = {m: _read(m) for m in targets}
        args = (dict(mode="placements", placements=[g.uid for g in groups]) if placed
                else dict(mode=WITH_CONTEXT, faces=slab))
        ans = run(hist, "intersect_faces", **args)
        assert ans["ok"], ans
        _check_feed(ans["step"], before)
        cube, bar = targets
        assert sum(f.attrs.get("color") == [0, 0, 1] for f in bar.faces) == 14
        for x in (0, 2):
            on = [f for f in cube.faces if all(v.x() == x for v in f.vertices)]
            assert sorted((len(f.holes), round(f.net_area(), 5)) for f in on) == [(0, 1), (1, 3)]
            assert all(f.attrs["color"] == RED for f in on)
        assert hist.undo()
        assert {m: _read(m) for m in targets} == before
        assert hist.redo()
        if not placed:
            blue = [f.key for f in cube.faces if f.attrs["color"] == [0, 0, 1]]
            assert run(hist, "erase", faces=blue)["ok"]
            for x in (0, 2):
                on = [f for f in cube.faces if all(v.x() == x for v in f.vertices)]
                assert sorted(f.net_area() for f in on) == [1., 3.]
                rim = next(f for f in on if f.holes).hole_loops[0]
                assert all(len(cube.find_edge(a, b).faces) == 2
                           for a, b in zip(rim, rim[1:] + rim[:1]))


def test_placement_cut_failure_restores_both_meshes(monkeypatch):
    from core import intersect as X
    from core.group import Group
    from tests.test_core_protocol import _read

    left, right = [s.mesh for s in scenes(monkeypatch)]
    for mesh, box in ((left, _box(0, 0, 0, 2, 2, 2, RED)),
                      (right, _box(-.5, .5, .5, 2.5, 1.5, 1.5, RED))):
        for f in box.mesh.faces:
            mesh.add_face(f.vertices)
    scene = Scene()
    scene.groups = [Group(left), Group(right)]
    scene.selection = set(scene.groups)
    history = History(scene)
    before = [_read(m) for m in (left, right)]
    partition = X._partition_faces

    def fail_on_second(mesh, cuts):
        partition(mesh, cuts)
        if mesh == right:
            raise ValueError("second operand refused")

    monkeypatch.setattr(X, "_partition_faces", fail_on_second)
    _, cmd = X.build_intersect_faces(scene, "placements", placements=scene.groups)
    history.execute(cmd)
    assert "second operand refused" in history.last_error
    assert not history.undo_stack
    assert [_read(m) for m in (left, right)] == before

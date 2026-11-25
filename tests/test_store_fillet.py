# SPDX-License-Identifier: GPL-3.0-or-later
"""Differential tests of the keyed fillet command and both affected-element modes."""
from built_core import store_views
from collections import defaultdict
from types import SimpleNamespace

import numpy as np
import pytest
from built_core import Vec3 as V

from core import _native
from built_core import mesh as M, store_mesh as S
from built_core import apply_fillet, plan_fillet
from built_core.history import SnapshotMutation
from built_core import plan_vertex_bevel
from tests.test_fillet import box, edge_at
from tests.test_vertex_bevel import pyramid

native = _native.available()
pytestmark = [pytest.mark.skipif(
    not hasattr(getattr(native, "MeshStore", None), "plan_fillet"),
    reason="requires native fillet operations")]


def store_copy(mesh):
    copy = M.Mesh()
    store_views.become_store(copy, native.MeshStore(next(M._KEY_NAMESPACES), **S._weld_args()), [])
    for face in mesh.faces:
        f = copy.add_face(face.vertices, face.holes)
        f.attrs = dict(face.attrs)
        f.interior = face.interior
    for edge in mesh.edges:
        e = copy.add_edge(edge.a, edge.b)
        M.stamp_edge_flags(e, M.edge_flags(edge))
    return copy


def point(p):
    return tuple(round(v, 5) for v in p.toTuple())


def ring(points):
    loop = tuple(map(point, points))
    return min(loop[i:] + loop[:i] for i in range(len(loop)))


def model(mesh):
    faces = sorted((ring(f.vertices), sorted(map(ring, f.holes)), f.interior,
                    repr(sorted(f.attrs.items()))) for f in mesh.faces)
    edges = []
    curves = defaultdict(list)
    for e in mesh.edges:
        ends = tuple(sorted((point(e.a), point(e.b))))
        edges.append((ends, e.soft, e.hidden, e.flat, e.layer, len(e.faces)))
        if e.curve is not None:
            curves[e.curve].append(ends)
    return (sorted(point(v.position) for v in mesh.vertices), sorted(edges),
            faces, sorted(sorted(c) for c in curves.values()))


def assert_profile_cells(pm, sm, counts):
    """Rounded outline cells can split across a diagonal at the 1e-7 plane test."""
    assert sm._store.counts() == counts
    reference = np.array([v.position_tuple() for v in pm.vertices])
    assert len(reference) == counts[0]
    # Stable vertex correspondence even when rounding changes a cell diagonal
    # and therefore the order in which its corners are created.
    mapping = {}
    for v in sm.vertices:
        near = np.where(np.max(abs(reference - v.position_tuple()), axis=1) < 3e-7)[0]
        i, = near
        mapping[v.key] = pm.vertices[i].key
    assert len(set(mapping.values())) == counts[0]
    def cycle(keys):
        return min(tuple(keys[i:] + keys[:i]) for i in range(len(keys)))
    faces = {cycle([v.key for v in f.loop]): f for f in pm.faces}
    for f in sm.faces:
        ring = [mapping[v.key] for v in f.loop]
        parts = [cycle(ring)]
        if parts[0] not in faces:
            a, b, c, d = ring
            parts = [cycle([a,b,c]), cycle([a,c,d])]
            if parts[0] not in faces:
                parts = [cycle([a,b,d]), cycle([b,c,d])]
            assert not f.holes
            # The store's unsplit cell must satisfy the original plane
            # criterion, measured in model space before rounding to Qt.
            pts = np.array([v.position_tuple() for v in f.loop])
            rel = pts - pts[0]
            normal = np.cross(rel, np.roll(rel, -1, axis=0)).sum(axis=0)
            assert np.max(abs(rel @ normal)) < 1e-7 * np.linalg.norm(normal)
        for k in parts:
            original = faces.pop(k)
            assert (f.attrs, f.interior, f.holes) == (original.attrs, original.interior, original.holes)
    assert not faces
    edges = {frozenset((e.v0.key,e.v1.key)): e for e in pm.edges}
    for e in sm.edges:
        original = edges[frozenset((mapping[e.v0.key], mapping[e.v1.key]))]
        assert (e.flat, e.hidden, e.layer, e.curve, len(e.faces)) == (
            original.flat, original.hidden, original.layer, original.curve, len(original.faces))
        if e.soft != original.soft:
            # The support profile meets the 30 degree smoothing cutoff exactly.
            a, b = e.faces
            assert V.dotProduct(a.normal(), b.normal()) == pytest.approx(3**.5/2, abs=2e-6)


def prism(outer, holes=()):
    m = M.Mesh()
    paths = [[V(x, y, 0) for x, y in path] for path in (outer, *holes)]
    top = lambda p: V(p.x(), p.y(), 1)
    m.add_face(paths[0][::-1], [h[::-1] for h in paths[1:]])
    m.add_face(list(map(top, paths[0])), [list(map(top, h)) for h in paths[1:]])
    for path in paths:
        for a, b in zip(path, path[1:] + path[:1]):
            m.add_face([a, b, top(b), top(a)])
    return m


def fixture(case):
    if case == "hole":
        return prism([(0, 0), (4, 0), (4, 4), (0, 4)],
                     [[(1, 1), (1, 2), (2, 2), (2, 1)]])
    if case == "concave":
        return prism([(0, 0), (2, 0), (2, 1), (1, 1), (1, 2), (0, 2)])
    if case == "apex":
        return pyramid()
    return box()


def preview(m, case):
    if case in ("vertex", "apex"):
        v = m.vertex_at(V(1, 1, 1) if case == "apex" else V(0, 0, 0))
        return plan_vertex_bevel(m, [v], .15, 1 if case == "apex" else 5)
    if case in ("concave", "hole"):
        edges = [edge_at(m, (1, 1, 0), (1, 1, 1))]
    elif case == "chain":
        edges = [edge_at(m, (0, 0, 1), (2, 0, 1)), edge_at(m, (2, 0, 1), (2, 1, 1))]
    elif case in ("round", "cut", "cornice", "steps", "crown", "supports"):
        edges = list(m.edges)
    else:
        edges = [edge_at(m, (0, 0, 1), (2, 0, 1))]
    return plan_fillet(m, edges, .15, 5, corners="cut" if case == "cut" else "round",
                       shape=case if case in ("steps", "cornice", "crown", "supports") else None)


@pytest.mark.parametrize("case", ["edge", "concave", "chain", "hole", "vertex", "apex",
                                  "round", "cut", "steps", "cornice", "crown", "supports"])
def test_command_parity_and_snapshot_history(case, monkeypatch):
    pm = fixture(case)
    for f in pm.faces:
        f.attrs.update(mat="oak", name="source")
    pm.faces[0].interior = True
    pm.edges[0].layer = "trim"
    pm.edges[0].hidden = True
    pm.edges[0].flat = True
    pm.edges[0].curve = 9801
    sm = store_copy(pm)
    reference = preview(pm, case)
    plan = preview(sm, case)
    assert not isinstance(reference, str), reference
    assert not isinstance(plan, str), plan
    assert isinstance(plan, native.BevelPreview)
    python_serial, store_serial = pm._mut_serial, sm._mut_serial
    apply_fillet(pm, reference)
    before = sm.capture_state()
    keys_before = {k: set(sm._store.keys(k)) for k in ("vertex", "edge", "face")}
    scene = SimpleNamespace(mesh=sm, version=0)
    changes = []
    command = SnapshotMutation(lambda _: changes.append(apply_fillet(sm, plan)), mesh=sm)
    command.do(scene)
    profile_counts = {"cornice": (1264, 2556, 1302), "crown": (896, 1812, 926),
                      "supports": (256, 512, 266)}
    if case in profile_counts:
        assert_profile_cells(pm, sm, profile_counts[case])
    else:
        assert sm._mut_serial - store_serial == pm._mut_serial - python_serial
        assert model(sm) == model(pm)
    assert set(changes[0].removed["face"]) == set(plan.removed_faces)
    assert set(changes[0].removed["edge"]) == set(plan.removed_edges)
    for kind, old in keys_before.items():
        removed = set(changes[0].removed[kind])
        created = set(changes[0].created[kind])
        live = set(sm._store.keys(kind))
        assert removed == old - live
        assert created == live - old
        assert set(changes[0].modified[kind]) <= old & live
    assert not changes[0].moved
    after = sm.capture_state()
    command.undo(scene)
    assert isinstance(sm, S.StoreMesh) and sm.capture_state() == before
    command.do(scene)
    assert isinstance(sm, S.StoreMesh) and sm.capture_state() == after
    assert len(changes) == 1


@pytest.mark.parametrize("case", ["large", "loose", "nonmanifold", "flat", "empty",
                                  "long vertex", "round apex", "wire vertex", "profile"])
def test_refusals(case, monkeypatch):
    pm = pyramid() if case == "round apex" else box()
    if case in ("wire vertex", "loose"):
        pm.add_edge(V(0, 0, 0), V(-1, 0, 0))
    if case in ("nonmanifold", "flat"):
        if case == "flat":
            pm = M.Mesh()
            pm.add_face([V(0, 0, 0), V(2, 0, 0), V(0, 1, 0)])
        pm.add_face([V(0, 0, 0), V(2, 0, 0), V(0, -2, 0)])
    sm = store_copy(pm)
    def refused(m):
        edge = edge_at(m, (0, 0, 0), (-1, 0, 0) if case == "loose" else (2, 0, 0))
        if case in ("round apex", "long vertex", "wire vertex"):
            v = m.vertex_at(V(1, 1, 1) if case == "round apex" else V())
            return plan_vertex_bevel(m, [v], 3 if case == "long vertex" else .1)
        return plan_fillet(m, [] if case == "empty" else [edge],
                           5 if case == "large" else .1, profile=1 if case == "profile" else .5)
    before = sm.capture_state()
    message = refused(sm)
    assert isinstance(message, str) and message == refused(pm)
    assert sm.capture_state() == before


def test_reject_stale_preview_and_foreign_keys():
    sm = store_copy(box())
    plan = preview(sm, "edge")
    foreign = store_copy(box())
    with pytest.raises(KeyError):
        plan_fillet(sm, [foreign.edges[0]], .1)
    sm.add_edge(V(10, 0, 0), V(11, 0, 0))
    before = sm.capture_state()
    with pytest.raises(ValueError, match="stale"):
        apply_fillet(sm, plan)
    assert sm.capture_state() == before


@pytest.mark.parametrize("vertices", [False, True])
def test_tool_uses_native_preview_and_one_snapshot_command(vertices, monkeypatch, tmp_path):
    from built_core.scene import Scene
    from built_core.history import History
    from tools.fillet import FilletTool
    sm = store_copy(box())
    scene = Scene(mesh=sm)
    scene.select([edge_at(sm, (0, 0, 1), (2, 0, 1))])
    vp = SimpleNamespace(scene=scene, history=History(scene),
                         flash_status=lambda *args: None, update=lambda: None)
    vp.history.error_log = str(tmp_path / "history.log")
    for key, value in dict(radius=.15, segments=3, profile=.5, measure="radius",
                           shape=None, corners="round", affect="vertices" if vertices else "edges").items():
        monkeypatch.setattr(FilletTool, key, value)
    tool = FilletTool()
    tool.on_activate(vp)
    assert tool.preview_faces() and isinstance(tool._plan, native.BevelPreview)
    # A flag write changes the store revision independently of the tool's
    # Python dirty counter. Commit must replan before applying the preview.
    edge = sm.edges[0]
    edge.hidden = not edge.hidden
    before = sm.capture_state()
    assert tool.on_value(vp, .15)
    assert vp.history.last_error is None
    assert len(vp.history.undo_stack) == 1
    assert vp.history.undo_stack[-1].change.created["face"]
    after = sm.capture_state()
    assert after != before
    assert vp.history.undo() and sm.capture_state() == before
    assert vp.history.redo() and sm.capture_state() == after
    assert isinstance(sm, S.StoreMesh)


@pytest.mark.parametrize("vertices", [False, True])
def test_keyed_command(vertices):
    sm = store_copy(box())
    before = sm._store.snapshot()
    if vertices:
        key = sm.vertices[0].key
        change = sm.bevel_vertices([key], .1)
        assert sm._store.is_live(key)
    else:
        key = sm.edges[0].key
        change = sm.fillet_edges([key], .1)
        assert not sm._store.is_live(key)
    assert change.created["face"] and change.removed["face"]
    net = sm._store.snapshot().changes_since(before, [])
    for kind in ("vertex", "edge", "face"):
        assert set(change.modified[kind]) == set(net.modified[kind])
        removed = set(change.removed[kind])
        created = set(change.created[kind])
        modified = set(change.modified[kind])
        assert not removed & (created | modified)
        assert not created & modified


def test_rebuilt_face_filters_nested_holes_at_commit():
    pm = box(8., 8., 1.)
    sm = store_copy(pm)
    for m in (pm, sm):
        top = m.faces[1]
        for a, b in [(1., 4.), (2., 3.), (5., 6.)]:
            m.add_hole(top, [V(a, a, 1), V(a, b, 1), V(b, b, 1), V(b, a, 1)])
        e = edge_at(m, (0, 0, 1), (8, 0, 1))
        plan = plan_fillet(m, [e], .2, 3)
        apply_fillet(m, plan)
    assert model(sm) == model(pm)
    assert max(len(f.holes) for f in sm.faces) == 2


@pytest.mark.parametrize("measure,profile,shape,corners", [
    ("offset", .65, None, "round"), ("depth", .25, None, "cut"),
    ("width", .5, ((1, 0, False), (.8, .4, True), (.3, .6, False), (0, 1, False)), "round"),
    ("offset", .5, "steps", "cut")])
def test_profile_parameters(measure, profile, shape, corners):
    pm = box()
    sm = store_copy(pm)
    for m in (pm, sm):
        edges = list(m.edges)
        plan = plan_fillet(m, edges, .1, 4, profile, measure, corners, shape)
        assert not isinstance(plan, str), plan
        apply_fillet(m, plan)
    if isinstance(shape, tuple):
        assert_profile_cells(pm, sm, (176, 348, 182))
    else:
        assert model(sm) == model(pm)


@pytest.mark.parametrize("segments,profile", [(1, .25), (3, .4), (4, .65)])
def test_multiple_chosen_vertices_and_duplicate_input(segments, profile):
    pm = box()
    sm = store_copy(pm)
    for m in (pm, sm):
        vertices = list(m.vertices) + [m.vertices[0]]
        plan = plan_vertex_bevel(m, vertices, .1, segments, profile)
        assert not isinstance(plan, str), plan
        apply_fillet(m, plan)
    assert model(sm) == model(pm)


@pytest.mark.parametrize("case", ["two ends overlap", "isolated vertex", "nonmanifold vertex",
                                  "edges too flat", "four edges", "empty vertices", "zero size",
                                  "bad outline", "bad vertex profile"])
def test_additional_refusals(case):
    pm = pyramid() if case == "four edges" else box()
    if case == "nonmanifold vertex":
        pm.add_face([V(0, 0, 0), V(2, 0, 0), V(0, -1, 0)])
    if case == "edges too flat":
        pm = M.Mesh()
        pm.add_face([V(-1, 0, 0), V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    sm = store_copy(pm)
    def refusal(m):
        if case == "four edges":
            return plan_fillet(m, list(m.vertex_at(V(1, 1, 1)).edges), .1)
        if case == "bad outline":
            return plan_fillet(m, [m.edges[0]], .1, shape=[(1., 0., False)])
        chosen = [m.vertex_at(V())]
        if case == "two ends overlap":
            chosen.append(m.vertex_at(V(0, 1, 0)))
            return plan_vertex_bevel(m, chosen, .5)
        if case == "isolated vertex":
            chosen = [m.vertex(V(3, 3, 3))]
        return plan_vertex_bevel(m, [] if case == "empty vertices" else chosen,
                                 0. if case == "zero size" else .1,
                                 profile=0. if case == "bad vertex profile" else .5)
    message = refusal(pm)
    assert isinstance(message, str)
    assert refusal(sm) == message


def test_native_apply_uses_attribute_indices_and_explicit_curve_ids():
    sm = store_copy(box())
    st = sm._store
    sm.__dict__["_attr_table"] = [{}, {"mat": "oak"}, {}, {"mat": "steel"}]
    edge = sm.edges[0]
    for f in edge.faces:
        st.set_face_attr(f.key, 1)
    plan = st.plan_fillet([edge.key], .1, 4, .5, "radius", "round", "", [], _native.qt_hypot())
    before = st.snapshot()
    with pytest.raises(ValueError):
        st.apply_fillet(plan, [])
    assert st.same_as(before)
    change = st.apply_fillet(plan, [41, 42], material_index=3)
    faces = change.created["face"]
    assert [st.face_info(k)[0] for k in faces[:len(plan.faces)]] == [
        st.face_info(k)[0] for k in plan.faces]
    assert all(st.face_info(k)[0] == 3 for k in faces[len(plan.faces):])
    assert {e.curve for e in sm.edges if e.curve is not None} == {41, 42}


def test_nonplanar_apex_refuses_a_flat_cap():
    pm = pyramid()
    sm = store_copy(pm)
    messages = []
    for m in (pm, sm):
        v = m.vertex_at(V(1, 1, 1))
        m.move_vertex(v, V(.4, .2, 0))
        before = m.capture_state()
        messages.append(plan_vertex_bevel(m, [v], .15, 1))
        assert m.capture_state() == before
    assert messages[0] == messages[1] and "not flat enough" in messages[0]


def test_welding_to_a_live_vertex_reports_its_new_incidence():
    sm = store_copy(box())
    cut = sm.vertex(V(.125, 0, 0))
    snapshot = sm._store.snapshot()
    change = sm.bevel_vertices([sm.vertex_at(V()).key], .125, 1)
    assert cut.key in change.modified["vertex"]
    net = sm._store.snapshot().changes_since(snapshot, [])
    assert set(change.modified["vertex"]) == set(net.modified["vertex"])

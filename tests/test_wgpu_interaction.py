"""Device-free checks keep native input tied to document and GL snap semantics."""
import math
from types import SimpleNamespace
import numpy as np
import pytest
from PySide6.QtCore import Qt, QPointF
from PySide6.QtGui import QVector3D, QMatrix4x4, QMouseEvent
from PySide6.QtWidgets import QApplication
from core.scene import Scene
from core.mesh import Mesh
from core.group import Group
from core.snap import compute_snap
from views.wgpu_scene import pack_scene
from views.wgpu_inference import prepare, snap
from views.wgpu_view import WgpuView

app = QApplication.instance() or QApplication([])


@pytest.fixture
def view():
    scene = Scene()
    mesh = Mesh()
    mesh.add_face([QVector3D(0, 0, 0), QVector3D(2, 0, 0),
                   QVector3D(2, 2, 0), QVector3D(0, 2, 0)])
    group = Group(mesh, name='placed')
    group.xform = QMatrix4x4()
    group.xform.translate(4, 3, 0)
    scene.groups.append(group)
    v = WgpuView()
    v.resize(800, 600)
    v.scene = scene
    v.packed = pack_scene(scene)
    v.uid_rows = {uid: [i] for i, uid in enumerate(v.packed.uids)}
    v.camera.target = QVector3D(5, 4, 0)
    v.camera.pitch = math.pi/2-.001
    v.camera.distance = 8
    v.camera.aspect = 800/600
    v.inference_index = prepare(v.packed)
    v.renderer = SimpleNamespace(set_highlight=lambda *a: None,
                                 set_rest=lambda *a: None,
                                 pick=lambda *a: 0, pick_rect=lambda *a: [0])
    yield v
    v.renderer = None
    v.close()


def test_resolution(view):
    assert view.resolve(0) == (view.scene.groups[0], 0)
    assert view.resolve(-1) is None
    assert view.resolve(100) is None


@pytest.mark.parametrize('point,kind', [((6, 5, 0), 'endpoint'),
                                       ((5, 3, 0), 'midpoint'),
                                       ((5, 4, 0), 'on_face')])
def test_snap_matches_exact_rules(view, point, kind):
    world = QVector3D(*point)
    px = view._world_to_pixel(world)
    result = snap(view.inference_index, 0, view, *px)
    mesh = view.scene.groups[0].mesh
    transform = view.scene.groups[0].xform
    edges = [SimpleNamespace(a=transform.map(e.a), b=transform.map(e.b),
                             in_group=True, context='group') for e in mesh.edges]
    expected = compute_snap(world, px, SimpleNamespace(edges=edges),
                            view._world_to_pixel, 12., face_under_cursor=True)
    assert result.kind == expected.kind == kind
    assert (result.point-expected.point).length() < 1e-4


def test_selection_modifiers_and_external_change(view):
    group = view.scene.groups[0]
    calls = []
    view.renderer.set_highlight = lambda *args: calls.append(args)
    view.select_rows([0])
    assert view.scene.selection == {group}
    view.select_rows([0], Qt.ShiftModifier)
    assert not view.scene.selection
    view.select_rows([0], Qt.ControlModifier)
    assert view.scene.selection == {group}
    view.select_rows([0], Qt.ControlModifier | Qt.ShiftModifier)
    assert not view.scene.selection
    view.scene.select([group])
    view.sync_selection()
    assert calls[-1][0] == [0]  # native edge-only group selection


def test_rectangle_maps_to_selection(view):
    calls = []
    view.renderer.pick_rect = lambda *args: calls.append(args) or [0]
    view.select_rows(view.rect_rows(30, 40, 10, 20))
    assert view.scene.selection == {view.scene.groups[0]}
    scale = view.devicePixelRatioF()
    assert calls == [tuple(int(v*scale) for v in (10, 20, 30, 40))]


def test_camera_projection_matches_gl(view):
    from views.viewport import Viewport
    point = QVector3D(5, 4, 0)
    assert view._world_to_pixel(point) == Viewport._world_to_pixel(view, point)
    assert view._pixel_to_ray(240, 180) == Viewport._pixel_to_ray(view, 240, 180)


@pytest.mark.parametrize('modifiers', [Qt.NoModifier, Qt.ShiftModifier])
def test_middle_gesture_matches_gl(view, modifiers):
    import copy
    from views.viewport import Viewport
    gl = Viewport()
    gl.resize(view.size())
    gl.scene = view.scene
    gl.camera = copy.deepcopy(view.camera)
    pivot = QVector3D(5, 4, 0)
    view._orbit_pivot_at = lambda *args: pivot
    gl._orbit_pivot_at = lambda *args: pivot
    view.source_viewport = gl
    gl._invert_orbit_y = False
    press = QMouseEvent(QMouseEvent.MouseButtonPress, QPointF(400, 300),
                        QPointF(400, 300), Qt.MiddleButton, Qt.MiddleButton, modifiers)
    move = QMouseEvent(QMouseEvent.MouseMove, QPointF(420, 311),
                       QPointF(420, 311), Qt.NoButton, Qt.MiddleButton, modifiers)
    view.mousePressEvent(press)
    gl.mousePressEvent(press)
    view.mouseMoveEvent(move)
    gl.mouseMoveEvent(move)
    assert view.camera.view_matrix().data() == gl.camera.view_matrix().data()
    view.source_viewport = None
    gl.close()


def test_edit_context_does_not_duplicate_group_geometry(view):
    group = view.scene.groups[0]
    view.scene.begin_group_edit(group)
    packed = pack_scene(view.scene)
    assert packed.groups == [group]
    view.scene.end_group_edit()


@pytest.mark.parametrize('point', [(4, 3, 0), (5, 3, 0), (5, 4, 0)])
def test_snap_matches_gl_viewport(view, point):
    from views.viewport import Viewport
    gl = Viewport()
    gl.resize(view.size())
    gl.scene = view.scene
    gl.camera = view.camera
    gl.active_tool = None
    pixel = view._world_to_pixel(QVector3D(*point))
    actual = snap(view.inference_index, 0, view, *pixel)
    expected = gl._snap_at(*pixel, Qt.NoModifier)
    assert actual.kind == expected.kind
    assert (actual.point-expected.point).length() < 1e-4
    gl.close()


def test_wheel_and_window_match_gl(view):
    import copy
    from PySide6.QtCore import QPoint
    from PySide6.QtGui import QWheelEvent
    from views.viewport import Viewport
    gl = Viewport()
    gl.resize(view.size())
    gl.scene = view.scene
    gl.camera = copy.deepcopy(view.camera)
    view.source_viewport = gl
    focus = QVector3D(5.2, 4.1, 0)
    view._depth_world_at = gl._depth_world_at = lambda *args: ('hit', focus)
    gl._model_diag = view._model_diag
    gl._invert_wheel = False
    event = QWheelEvent(QPointF(420, 315), QPointF(420, 315), QPoint(),
                        QPoint(0, 120), Qt.NoButton, Qt.NoModifier,
                        Qt.NoScrollPhase, False)
    view.wheelEvent(event)
    gl.wheelEvent(event)
    assert view.camera.view_matrix().data() == gl.camera.view_matrix().data()
    start, end = QPointF(240, 150), QPointF(600, 480)
    Viewport._zoom_to_box(view, start, end)
    gl._zoom_to_box(start, end)
    assert view.camera.view_matrix().data() == gl.camera.view_matrix().data()
    view.source_viewport = None
    gl.close()


def test_native_face_pick_resolves_prototype_and_edit_context(view):
    from types import SimpleNamespace
    view.source_viewport = SimpleNamespace(scene=view.scene)
    pixel = view._world_to_pixel(QVector3D(5, 4, 0))
    face, group = view.tools.pick_face_any(*pixel)
    assert face is view.scene.groups[0].mesh.faces[0]
    assert group is view.scene.groups[0]
    assert view.tools.pick_face(*pixel) is None
    view.scene.begin_group_edit(group)
    view.packed = pack_scene(view.scene)
    face, owner = view.tools.pick_face_any(*pixel)
    assert face is view.scene.mesh.faces[0]
    assert owner is None
    assert view.tools.pick_face(*pixel) is face
    view.source_viewport = None


def test_worker_warms_first_hover_for_the_captured_camera(view):
    view._prepare_inference()
    index = view._inference_future.result(timeout=5)
    cache = index.screen_cache
    assert cache is not None
    point = QVector3D(6, 5, 0)
    assert snap(index, 0, view, *view._world_to_pixel(point)).kind == 'endpoint'
    assert index.screen_cache is cache
    view.camera.distance *= 1.2
    assert snap(index, 0, view, *view._world_to_pixel(point)).kind == 'endpoint'
    assert index.screen_cache is not cache


def test_idle_document_edits_refresh_native_view(monkeypatch):
    from PySide6.QtTest import QTest
    from PySide6.QtWidgets import QStackedWidget
    from core import renderer_settings
    from core.history import HideCommand, DeleteGroupCommand
    from views.main_window import MainWindow
    from views.viewport import Viewport

    # Exercise the document timers and real packed rows without a GPU.
    stack = QStackedWidget()
    viewport = Viewport(stack)
    stack.addWidget(viewport)
    doc = SimpleNamespace(viewport=viewport)
    stack._doc_stack = stack
    stack._doc = doc
    stack._document_first_frame = lambda *_: None
    monkeypatch.setattr(renderer_settings, 'arm', lambda: None)
    monkeypatch.setattr(WgpuView, '_upload', lambda self: None)
    mesh = Mesh()
    mesh.add_edge(QVector3D(), QVector3D(1, 0, 0))
    group = Group(mesh)
    viewport.scene.groups.append(group)
    MainWindow._install_wgpu_display(stack, doc)
    display = doc.stand_in
    stack.show()
    assert not viewport.isVisible()

    def wait_for(predicate):
        for _ in range(100):
            if predicate():
                return
            QTest.qWait(10)
        assert predicate()

    def visible():
        return display.resolve(0) is not None

    try:
        wait_for(visible)
        for command in (HideCommand([group]), DeleteGroupCommand(group)):
            viewport.history.execute(command)
            viewport.update()
            wait_for(lambda: not visible())
            viewport.history.undo()
            viewport.update()
            wait_for(visible)
    finally:
        display.close()
        stack.close()


def test_orbit_pivot_is_under_cursor_without_snap_index(view):
    # The snap index is built on a worker and dropped on edits and at open;
    # meanwhile the pivot fell back to the middle of the model.
    view.inference_index = None
    target = QVector3D(5.5, 3.5, 0)
    px = view._world_to_pixel(target)
    pivot = view._orbit_pivot_at(*px)
    assert (pivot - target).length() < 1e-3


@pytest.mark.parametrize('point', [(6, 5, 0), (5, 3, 0), (5, 4, 0)])
def test_batch_native_matches_python_prototypes(view, point, monkeypatch):
    """Keep the former float64 expansion as an independent snap oracle."""
    from core import _native
    native = _native.available()
    def reference(inputs, ids, mats, lo, hi):
        protos = []
        for tri, pos, edges, keep, curves in inputs:
            tris = tri.astype(float).reshape(-1, 3, 3)
            segments = pos[edges[keep]].reshape(-1, 6)
            protos.append(native.InstanceProto(
                np.ascontiguousarray(tris[:, 0]),
                np.ascontiguousarray(tris[:, 1]-tris[:, 0]),
                np.ascontiguousarray(tris[:, 2]-tris[:, 0]), segments))
        return protos, native.InstanceSet(protos, ids, mats, lo, hi)
    actual_index = prepare(view.packed, view.scene)
    with monkeypatch.context() as m:
        m.setattr(native, 'build_snap_index', reference)
        expected_index = prepare(view.packed, view.scene)
    pixel = view._world_to_pixel(QVector3D(*point))
    actual = snap(actual_index, 0, view, *pixel)
    expected = snap(expected_index, 0, view, *pixel)
    assert actual.kind == expected.kind
    assert (actual.point-expected.point).length() < 1e-6


def test_native_curve_order_and_invalid_indices():
    from core import _native
    native = _native.available()
    pos = np.arange(24, dtype=float).reshape(8, 3)
    edges = np.arange(8, dtype=np.int64).reshape(4, 2)
    keep = np.array([True, False, True, True])
    ids = np.array([9, 3, 2, 9], np.int64)
    def build(edges):
        return native.build_snap_index([(np.empty((0, 3), np.float32), pos, edges, keep, ids)],
            np.empty(0, np.int64), np.empty((0, 16)), np.empty((0, 3)), np.empty((0, 3)))
    protos, _ = build(edges)
    curves = protos[0].curve_segments()
    np.testing.assert_array_equal(curves[0], pos[edges[[0, 3]]].reshape(-1, 6))
    np.testing.assert_array_equal(curves[1], pos[edges[[2]]].reshape(-1, 6))
    edges[0, 0] = 100
    with pytest.raises(ValueError):
        build(edges)


def test_full_pack_starts_inference_without_after_open(monkeypatch):
    view = WgpuView()
    monkeypatch.setattr(view, '_upload', lambda: None)
    view.set_scene(Scene(), opening=True)
    assert view._inference_future is not None
    assert view._inference_future.result(timeout=5) is not None
    assert view._refresh_inference not in view.after_open
    view.close()


def _source_stub(view):
    view.source_viewport = SimpleNamespace(plano_style=None, style_override=None,
                                           _preview_groups=())


def test_selected_group_is_boxed_in_the_overlay(view):
    # GL boxes a selected group (its selection cue); Metal only tinted it.
    _source_stub(view)
    group = view.scene.groups[0]
    assert view._selection_box_primitives() == []
    view.scene.selection = {group}
    lines = view._selection_box_primitives()
    assert len(lines) == 8      # a flat square: its 4 vertical edges have no length
    drawn = [p for line in lines for p in line.points]
    corners = [view._world_to_pixel(QVector3D(x, y, 0)) for x in (4, 6) for y in (3, 5)]
    for axis in (0, 1):
        assert min(p[axis] for p in drawn) == pytest.approx(min(c[axis] for c in corners), abs=1)
        assert max(p[axis] for p in drawn) == pytest.approx(max(c[axis] for c in corners), abs=1)


def test_selection_box_skips_billboards_and_follows_a_move(view):
    _source_stub(view)
    group = view.scene.groups[0]
    view.scene.selection = {group}
    before = view._selection_box_primitives()[0].points
    group.xform.translate(1, 0, 0)
    view.scene.version += 1
    after = view._selection_box_primitives()[0].points
    assert after != before
    group.billboard = 'mesh'
    assert view._selection_box_primitives() == []


def test_leaving_a_group_clears_the_rest_after_an_update_reset(view):
    # An incremental update clears _rest_signature; leaving the group then
    # skipped set_rest (no rows, no signature) and the model stayed hidden.
    calls = []
    view.renderer.set_rest = lambda rows, mode: calls.append((list(rows), mode))
    view.source_viewport = SimpleNamespace(edit_rest_mode='hide')
    view.scene.edit_group = view.scene.groups[0]
    view.sync_edit_context()
    view._rest_signature = None               # what an incremental update does
    view.scene.edit_group = None
    view.sync_edit_context()
    assert calls[-1] == ([], 'hide')

# SPDX-License-Identifier: GPL-3.0-or-later
"""Document camera and Metal rows for a detail millions of metres from the scale figure."""
from types import SimpleNamespace
from functools import partial
import numpy as np
from PySide6.QtGui import QVector3D as V

from core import scale_figure
from tests.test_.skp_inference_gaps import view, move, click, use, refresh
from tests.test_wgpu_soft_shadows import native
from views.main_window import MainWindow

BASE = np.array([650000., 6860000., 0.])
CORNERS = np.array([[.01,.02,0.],[.03,.02,0.],[.03,.04,0.],[.01,.04,0.]])


def _document(view):
    window = SimpleNamespace(viewport=view.source_viewport,
                             _doc=SimpleNamespace(view_grid=None))
    for name in ("_make_billboard_person", "_extents_bounds", "_figure_bounds", "_image_bounds"):
        setattr(window, name, partial(getattr(MainWindow, name), window))
    MainWindow._insert_scale_figure(window)
    assert scale_figure.is_marked(view.scene.groups[0])
    return window


def _frame(window, view):
    MainWindow._on_zoom_extents(window)
    view._sync_scene_camera()
    assert view.packed.origin == tuple(BASE)
    lo, hi = view.scene.bounds()
    expected = (np.array(lo.toTuple()) + hi.toTuple()) * .5
    np.testing.assert_allclose(window.viewport.camera.target.toTuple(), expected, atol=.003, rtol=0)


def test_draw_and_orbit_beside_distant_scale_figure(view):
    from PySide6.QtCore import Qt
    from PySide6.QtTest import QTest
    from tools.line import LineTool
    from tools.rectangle import RectangleTool
    from tools.pushpull import PushPullTool
    window = _document(view)
    mesh = view.scene.mesh
    mesh.add_edge(BASE + [0,.02,0], BASE + CORNERS[0])
    refresh(view)
    assert view.packed.origin == (0.,0.,0.)
    _frame(window, view)
    view.camera.target = CORNERS.mean(axis=0)
    view.camera.distance = .12
    use(view, RectangleTool())
    for i in (0, 2):
        move(view, V(*CORNERS[i]))
        click(view, V(*CORNERS[i]))
    face, = mesh.faces
    use(view, PushPullTool())
    click(view, V(*CORNERS.mean(axis=0)))
    assert view.tools.tool.dragging
    QTest.keyClicks(view, '0.012345m')
    QTest.keyClick(view, Qt.Key_Return)
    refresh(view)
    assert mesh._store.counts() == (9, 13, 6)
    _frame(window, view)
    use(view, LineTool())
    top = max(mesh.faces, key=lambda f: min(v.position_tuple()[2] for v in f.loop))
    local = np.array([v.position_tuple() for v in top.loop]) - BASE
    for p in local:
        snap = move(view, V(*p))
        assert snap.kind == 'endpoint'
        np.testing.assert_allclose(snap.point.toTuple(), p, atol=2e-8, rtol=0)
    pivot = V(*local.mean(axis=0))
    pixel = view._world_to_pixel(pivot)
    assert view.tools.pick_face_placement(*pixel)[0].key == top.key
    for _ in range(8):
        view.camera.orbit_about(pivot, 4., 2., view.height())
        view._sync_scene_camera()
    np.testing.assert_allclose(view._world_to_pixel(pivot), pixel, atol=.02)
    view.scene.selection.add(top)
    MainWindow._on_zoom_selection(window)
    np.testing.assert_allclose(window.viewport.camera.target.toTuple(), BASE + local.mean(axis=0),
                               atol=.004, rtol=0)


def test_mixed_scene_metal_frame_and_id_buffer(view, native, tmp_path):
    from core.protocol import commands
    window = _document(view)
    face = view.scene.mesh.add_face(BASE + CORNERS)
    assert commands.push_pull(view.source_viewport.history, face.key, .012345)["ok"]
    refresh(view)
    # Upload the near-camera rows first. Framing the prism must update
    # the resident GPU transforms, as well as the software pick/snap rows.
    view.packed.upload(native)
    view.renderer = native
    view._scene_dirty = False
    view.source_viewport.camera.aspect = 1.
    view.source_viewport.camera.pitch = .58
    MainWindow._on_zoom_extents(window)
    image = view.render_image(256, 256)
    np.testing.assert_array_equal(view.packed.origin, BASE)
    view.camera.aspect = 1.
    native.set_camera(list(view.camera.view_matrix().data()), list(view.camera.projection_matrix().data()))
    assert native.pick(128, 128) == view.packed.groups.index(None)
    assert all(native.pick(x, y) == -1 for x, y in ((0, 128), (255, 128), (128, 0), (128, 255)))
    assert image.save(str(tmp_path / 'lambert-prism.png'))


def test_zoom_selection_across_one_wide_mesh(view):
    mesh = view.scene.mesh
    near = mesh.add_face(CORNERS)
    far = mesh.add_face(BASE + CORNERS)
    refresh(view)
    window = SimpleNamespace(viewport=view.source_viewport, _doc=SimpleNamespace(view_grid=None))
    for face, origin in ((far, BASE), (near, np.zeros(3)), (far, BASE)):
        view.scene.selection.clear()
        view.scene.selection.add(face)
        MainWindow._on_zoom_selection(window)
        view._sync_scene_camera()
        np.testing.assert_array_equal(view.packed.origin, origin)
        positions = view.packed.vertices['pos']
        for corner in CORNERS:
            assert np.min(np.linalg.norm(positions - corner, axis=1)) < 2e-8


def test_nested_built_placement_framed_in_model_coordinates(view):
    from core.group import Group
    from formats.built import placement
    from core.mesh import Mesh
    group = Group(Mesh())
    group.mesh.add_face(CORNERS)
    parent = Group(Mesh())
    parent.children = [group]
    world = BASE + [.12345678, .23456789, 0.]
    for obj, delta in ((parent, world), (group, [.01,.02,0.])):
        matrix = np.eye(4)
        matrix[3, :3] = delta
        obj.xform = placement(matrix.ravel())
    view.scene.groups.append(parent)
    view.scene.selection.add(parent)
    for bounds in (view.scene.bounds(), view.scene.selection_bounds()):
        doc = np.array([v.toTuple() for v in bounds])
        expected = world + [.01,.02,0.] + np.stack((CORNERS.min(axis=0), CORNERS.max(axis=0)))
        np.testing.assert_allclose(doc, expected, atol=2e-9, rtol=0)
    refresh(view)
    packed = view.packed
    np.testing.assert_allclose(packed.instances['world'][0,3,:3],
                               world + [.01,.02,0.] - packed.origin, atol=2e-8, rtol=0)

# SPDX-License-Identifier: GPL-3.0-or-later
"""Coordinate precision through the store and a geographically placed scene."""
import numpy as np
import pytest

from core.history import History
from core.protocol import commands, queries
from core.scene import Scene
from core.store_mesh import StoreMesh

pytest.importorskip("ingetrazo_core")

from tests.test_.skp_inference_gaps import view  # noqa: F401
from tests.test_wgpu_soft_shadows import native  # noqa: F401


@pytest.mark.parametrize("origin", [(0., 0.), (650000., 6860000.)])
def test_store_coordinates_moves_weld_and_snapshots(origin, monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    monkeypatch.setenv("INGETRAZO_RUST", "1")
    scene = Scene()
    mesh = scene.mesh
    assert isinstance(mesh, StoreMesh)
    monkeypatch.setattr(StoreMesh, "_materialise", lambda *_: pytest.fail("materialised"))
    x, y = origin
    ring = [(x + .012, y + .02, 0.), (x + .042, y + .02, 0.),
            (x + .042, y + .06, 0.), (x + .012, y + .06, 0.)]
    mesh.add_faces_bulk(np.array(ring), [4], [1])
    assert mesh._store.counts() == (4, 4, 1)
    face = mesh.faces[0]
    v = face.loop[0]
    assert v.position_tuple() == ring[0]
    assert mesh.vertex((x + .01206, y + .02, 0.)).key == v.key
    assert mesh.edges[0].length() == pytest.approx(.03, abs=1e-9)
    h = History(scene)
    delta = (.001234567891, .002345678912, 0.)
    assert commands.move_vertices(h, [v.key], delta)["ok"]
    expected = [a+b for a,b in zip(ring[0], delta)]
    assert queries.vertices(scene, [v.key])["vertices"][0]["position"] == expected
    assert mesh._store.counts() == (4, 4, 1)
    h.undo()
    assert v.position_tuple() == ring[0]
    h.redo()
    assert list(v.position_tuple()) == expected


def test_georeferenced_draw_push_weld_pack_and_endpoint(view, monkeypatch):
    from georef.datum import SceneDatum, utm_inverse
    from tests.test_.skp_inference_gaps import move, refresh
    from PySide6.QtGui import QVector3D

    monkeypatch.setenv("INGETRAZO_STORE", "1")
    monkeypatch.setenv("INGETRAZO_RUST", "1")
    scene = view.scene
    mesh = scene.mesh
    assert isinstance(mesh, StoreMesh)
    datum = SceneDatum(*utm_inverse(650000., 6860000., 31, True))
    scene.georef = datum
    x, y, _ = datum.local_to_utm(QVector3D())
    def point(dx, dy):
        return datum.utm_to_local(x + dx, y + dy)

    ring = [point(dx, dy) for dx, dy in
            [(0.012, .020), (1.012, .020), (1.012, 1.020), (.012, 1.020)]]
    history = History(scene)
    assert commands.add_edges(history, [(p.toTuple(), q.toTuple())
        for p, q in zip(ring, ring[1:] + ring[:1])])["ok"]
    face, = mesh.faces
    assert mesh.vertex(point(.01206, .020)).key == mesh.vertex_at(ring[0]).key
    assert commands.push_pull(history, face.key, .12345)["ok"]
    assert mesh._store.counts() == (8, 12, 6)
    refresh(view)
    cap = max(mesh.vertices, key=lambda v: v.position.z()).position
    snap = move(view, cap)
    assert snap.kind == "endpoint"
    assert (snap.point - cap).length() < 1e-5
    assert np.any(np.linalg.norm(view.packed.vertices["pos"] - cap.toTuple(), axis=1) < 1e-6)



def test_format_three_rounds_only_the_saved_copy(tmp_path, monkeypatch):
    from formats import igz

    monkeypatch.setenv("INGETRAZO_RUST", "1")
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    scene = Scene()
    a = (650000.031, 6860000.249, 0.)
    b = (650001.031, 6860001.249, 0.)
    edge = scene.mesh.add_edge(a, b)
    path = tmp_path / "survey.igz"
    igz.save_scene(scene, path)
    assert edge.v0.position_tuple() == a
    restored = Scene()
    igz.load_into(restored, path)
    e, = restored.mesh.edges
    assert e.v0.position_tuple() == tuple(np.float32(a).astype(float))
    assert e.v1.position_tuple() == tuple(np.float32(b).astype(float))
    assert e.v0.position_tuple() != a

def test_raw_lambert_draw_pick_snap_push_and_weld(view, monkeypatch):
    from PySide6.QtGui import QVector3D as V
    from tests.test_.skp_inference_gaps import refresh, move

    mesh = view.scene.mesh
    assert isinstance(mesh, StoreMesh)
    monkeypatch.setattr(StoreMesh, "_materialise", lambda *_: pytest.fail("materialised"))
    history = view.source_viewport.history
    ring = [(650000.+dx, 6860000.+dy, 0.) for dx, dy in
            [(.01, .02), (.03, .02), (.03, .04), (.01, .04)]]
    assert commands.add_edge(history, *ring[:2])["ok"]
    assert commands.add_edges(history, list(zip(ring[1:], ring[2:]+ring[:1])))["ok"]
    assert mesh._store.counts() == (4, 4, 1)
    np.testing.assert_array_equal([v.position_tuple() for v in mesh.vertices], ring)
    face, = mesh.faces
    assert mesh.vertex((650000.01006, 6860000.02, 0.)).key == mesh.vertex_at(ring[0]).key
    refresh(view)
    p = view.packed
    assert p.origin == (650000., 6860000., 0.)
    assert len(np.unique(p.vertices['pos'], axis=0)) == 4
    view.camera.target = V(.02, .03, 0.)
    view.camera.distance = .12
    xy = view._world_to_pixel(view.camera.target)
    assert view.pick_row(*xy) == 0
    snap = move(view, V(.01,.02,0.))
    assert snap.kind == "endpoint"
    np.testing.assert_allclose(snap.point.toTuple(), np.subtract(ring[0], p.origin), atol=1e-8)
    assert commands.push_pull(history, face.key, .012345)["ok"]
    assert mesh._store.counts() == (8, 12, 6)
    positions = {v.key: v.position_tuple() for v in mesh.vertices}
    assert max(v[2] for v in positions.values()) == .012345
    assert history.undo()
    assert mesh._store.counts() == (4, 4, 1)
    assert history.redo()
    assert {v.key: v.position_tuple() for v in mesh.vertices} == positions


def test_metal_mouse_tools_at_raw_lambert(view):
    from PySide6.QtCore import Qt
    from PySide6.QtGui import QVector3D as V
    from PySide6.QtTest import QTest
    from tests.test_.skp_inference_gaps import click, move, refresh, use
    from tools.line import LineTool
    from tools.pushpull import PushPullTool
    from tools.rectangle import RectangleTool

    mesh = view.scene.mesh
    model = np.array([650000., 6860000., 0.])
    # The survey edge supplies the drawing origin and the first snapped corner.
    mesh.add_edge(model + [0., .02, 0.], model + [.01, .02, 0.])
    refresh(view)
    view.camera.target = V(.02, .03, 0.)
    view.camera.distance = .12
    use(view, RectangleTool())
    for point in (V(.01, .02, 0.), V(.03, .04, 0.)):
        move(view, point)
        click(view, point)
    face, = mesh.faces
    assert mesh._store.counts() == (5, 5, 1)
    corners = np.array([v.position_tuple() for v in face.loop]) - model
    np.testing.assert_allclose(corners.max(axis=0) - corners.min(axis=0), [.02, .02, 0.],
                               atol=.00015)
    use(view, LineTool())
    for point in (V(.03,.04,0.), V(.045,.05,0.)):
        move(view, point)
        click(view, point)
    assert mesh._store.counts() == (6, 6, 1)
    QTest.keyClick(view, Qt.Key_Escape)
    use(view, PushPullTool())
    click(view, V(.02, .03, 0.))
    assert view.tools.tool.dragging
    QTest.keyClicks(view, '0.012345m')
    QTest.keyClick(view, Qt.Key_Return)
    assert not view.tools.tool.dragging
    assert mesh._store.counts() == (10, 14, 6)
    assert max(v.position_tuple()[2] for v in mesh.vertices) == .012345


def test_scene_frame_stays_put_and_composes_child_offsets_in_double():
    from PySide6.QtGui import QMatrix4x4
    from core.group import Group
    from core.mesh import Mesh
    from views.wgpu_scene import pack_scene

    leaf = Group(Mesh())
    leaf.mesh.add_face([(0.,0.,0.), (.02,0.,0.), (.02,.02,0.), (0.,.02,0.)])
    leaf.xform = QMatrix4x4()
    leaf.xform.translate(.01, .02, 0.)
    root = Group(Mesh())
    root.children = [leaf]
    root.xform = QMatrix4x4()
    root.xform.translate(650000., 6860000., 0.)
    scene = Scene(groups=[root])
    packed = pack_scene(scene)
    assert packed.origin == (650000., 6860000., 0.)
    np.testing.assert_allclose(packed.world_bounds, [[.01,.02,0.],[.03,.04,0.]], atol=1e-8)
    root.xform.translate(2., 4., 0.)
    adjusted = pack_scene(scene, previous_origin=packed.origin)
    assert adjusted.origin == packed.origin
    root.xform.translate(1024., 0., 0.)
    moved = pack_scene(scene, previous_origin=adjusted.origin)
    assert moved.origin == (651026., 6860004., 0.)
    np.testing.assert_allclose(moved.world_bounds, packed.world_bounds, atol=1e-8)


def test_frame_preserves_numeric_input_and_feed_coordinates(view):
    from core.coordinate_frame import mesh_frame
    from PySide6.QtGui import QVector3D as V
    from tests.test_.skp_inference_gaps import click, move, refresh, use
    from tools.line import LineTool
    from tools.rectangle import RectangleTool

    scene, history = view.scene, view.source_viewport.history
    mesh = scene.mesh
    start = np.array([650000.012345678, 6860000.023456789, 0.])
    mesh.add_edge(start - [.02,0.,0.], start)
    refresh(view)
    p = V(*(start - view.packed.origin))
    view.camera.target = p
    view.camera.distance = .15
    observed = []
    history.add_listener(after=lambda *args: observed.append(mesh._store.frame_origin))
    use(view, LineTool())
    move(view, p)
    click(view, p)
    delta = (.012345678912, .017891234567, .001234567891)
    with mesh_frame(mesh, view.packed.origin):
        assert view.tools.tool.on_value(view.tools, delta)
    result = start + delta
    np.testing.assert_array_equal(mesh.vertex_at(result).position_tuple(), result)
    assert observed[-1] == [0., 0., 0.]
    assert history.undo()
    use(view, RectangleTool())
    move(view, p)
    click(view, p)
    move(view, p + V(.02,.03,0.))
    w, h = .022345678912, .038912345678
    with mesh_frame(mesh, view.packed.origin):
        assert view.tools.tool.on_value(view.tools, (w,h))
    expected = start + [w,h,0.]
    np.testing.assert_array_equal(mesh.vertex_at(expected).position_tuple(), expected)


def test_raw_lambert_metal_raster_and_id_buffer(native):
    from PySide6.QtGui import QMatrix4x4, QVector3D as V
    from views.wgpu_scene import pack_scene

    camera, projection = QMatrix4x4(), QMatrix4x4()
    camera.lookAt(V(.02,.03,.1), V(.02,.03,0.), V(0.,1.,0.))
    projection.ortho(-.02, .02, -.02, .02, .001, 1.)
    images = []
    for x, y in ((0.,0.), (650000.,6860000.)):
        scene = Scene()
        scene.mesh.add_face([(x+dx, y+dy, 0.) for dx, dy in
                             [(.01,.02),(.03,.02),(.03,.04),(.01,.04)]])
        pack_scene(scene).upload(native)
        native.set_camera(list(camera.data()), list(projection.data()))
        images.append(bytes(native.render_image(256, 256, list(camera.data()),
                                                 list(projection.data()), dict(sky=False, edges=False))))
        assert [native.pick(i,j) for i,j in [(80,80),(176,80),(176,176),(80,176),(40,40)]] == [0,0,0,0,-1]
    assert images[0] == images[1]


@pytest.mark.parametrize("dtype,counts", [("f4",(1,0,0)), ("f8",(2,1,0))])
def test_bulk_weld_uses_source_precision_at_the_radius(dtype, counts):
    from core.store_mesh import store_from_source
    points = np.array([[0.,0.,0.], [3.0003e-8,1e-4,0.]], dtype='f4').astype(dtype)
    empty = np.empty(0, dtype=np.int64)
    source = dict(pos=points, edges=[[0,1]], counts=empty, sizes=empty, rings=empty)
    store, _ = store_from_source(source)
    assert store.counts() == counts
    mesh = Scene().mesh
    vertices, ids = mesh.bulk_weld(points)
    mesh.add_edges_welded(vertices, ids[:1], ids[1:])
    assert mesh._store.counts() == counts


def test_metal_camera_tracks_document_controls_without_quantising_local_pan(view):
    from PySide6.QtGui import QVector3D as V
    from tests.test_.skp_inference_gaps import refresh
    mesh = view.scene.mesh
    mesh.add_face([(650000.+dx, 6860000.+dy, 0.) for dx, dy in
                   [(.01,.02),(.03,.02),(.03,.04),(.01,.04)]])
    refresh(view)
    np.testing.assert_allclose(view.camera.target.toTuple(), [.02,.03,0.], atol=1e-8)
    assert view.camera.distance < .2
    source = view.source_viewport.camera
    view.camera.target += V(.001234, .002345, .003456)
    precise = view.camera.target.toTuple()
    view._sync_scene_camera()
    source.yaw += .3
    source.fov_deg = 50.
    view._sync_scene_camera()
    assert view.camera.target.toTuple() == precise
    assert view.camera.yaw == source.yaw and view.camera.fov_deg == 50.

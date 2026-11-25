# SPDX-License-Identifier: GPL-3.0-or-later
"""Coordinate precision through the store and a geographically placed scene."""
import numpy as np
import pytest

from core.history import History
from core.protocol import commands, queries
from core.scene import Scene
from built_core.store_mesh import StoreMesh

pytest.importorskip("built_core")

from tests.test_.skp_inference_gaps import view  # noqa: F401
from tests.test_wgpu_soft_shadows import native  # noqa: F401


@pytest.mark.parametrize("origin", [(0., 0.), (650000., 6860000.)])
def test_store_coordinates_moves_weld_and_snapshots(origin, monkeypatch):
    monkeypatch.setenv("BUILT_RUST", "1")
    scene = Scene()
    mesh = scene.mesh
    assert isinstance(mesh, StoreMesh)
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
    from built_core import Vec3 as QVector3D

    monkeypatch.setenv("BUILT_RUST", "1")
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

    monkeypatch.setenv("BUILT_RUST", "1")
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
    from built_core import Vec3 as V
    from tests.test_.skp_inference_gaps import refresh, move

    mesh = view.scene.mesh
    assert isinstance(mesh, StoreMesh)
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
    from built_core import Input
    from built_core import Vec3 as V
    from built_render import KeyEvent
    from tests.test_.skp_inference_gaps import click, move, refresh, use
    from tools.rust_tools import LineTool
    from tools.pushpull import PushPullTool
    from tools.rust_tools import RectangleTool

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

    def press(code, text=""):
        view.keyPressEvent(KeyEvent(True, code, text=text))

    press(Input.Key_Escape)
    use(view, PushPullTool())
    click(view, V(.02, .03, 0.))
    assert view.tools.tool.dragging
    for char in '0.012345m':
        press(ord(char.upper()), char)
    press(Input.Key_Return)
    assert not view.tools.tool.dragging
    assert mesh._store.counts() == (10, 14, 6)
    assert max(v.position_tuple()[2] for v in mesh.vertices) == .012345


def test_scene_frame_stays_put_and_composes_child_offsets_in_double():
    from built_core import Matrix4 as QMatrix4x4
    from core.group import Group
    from core.mesh import Mesh
    from built_core import pack_scene

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
    from built_core.coordinate_frame import mesh_frame
    from built_core import Vec3 as V
    from tests.test_.skp_inference_gaps import click, move, refresh, use
    from tools.rust_tools import LineTool
    from tools.rust_tools import RectangleTool

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
    from built_core import Matrix4 as QMatrix4x4, Vec3 as V
    from built_core import pack_scene

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
    from built_core import store_views
    points = np.array([[0.,0.,0.], [3.0003e-8,1e-4,0.]], dtype='f4').astype(dtype)
    empty = np.empty(0, dtype=np.int64)
    source = dict(pos=points, edges=[[0,1]], counts=empty, sizes=empty, rings=empty)
    store, _ = store_views.store_from_source(source)
    assert store.counts() == counts
    mesh = Scene().mesh
    vertices, ids = mesh.bulk_weld(points)
    mesh.add_edges_welded(vertices, ids[:1], ids[1:])
    assert mesh._store.counts() == counts


def test_metal_camera_tracks_document_controls_without_quantising_local_pan(view):
    from built_core import Vec3 as V
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


def test_group_face_pick_at_raw_coordinates(view):
    from core.group import Group
    from core.mesh import Mesh
    from tests.test_.skp_inference_gaps import refresh
    group = Group(Mesh())
    face = group.mesh.add_face([(650000.+dx, 6860000.+dy, 0.) for dx, dy in
                                [(.01,.02),(.03,.02),(.03,.04),(.01,.04)]])
    view.scene.groups.append(group)
    refresh(view)
    x, y = view._world_to_pixel(view.camera.target)
    picked, context = view.tools.pick_face_placement(x, y)
    assert picked.key == face.key and context is not None
    assert face.loop[0].position_tuple() == (650000.01, 6860000.02, 0.)


@pytest.mark.parametrize("reader", ["deferred", "eager-bulk", "eager-sequential"])
def test_format_three_reopens_edge_with_original_float32_weld_decision(
        tmp_path, monkeypatch, reader):
    from formats import igz
    scene = Scene()
    points = np.array([[0.,0.,0.], [3.0003e-8,1e-4,0.]], dtype="f4").astype(float)
    scene.mesh.add_edge(*points)
    assert scene.mesh._store.counts() == (2,1,0)
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    if reader == "eager-bulk":
        monkeypatch.setattr(igz, "_BULK_MIN", 1)
    if reader != "deferred":
        from core.mesh import DeferredMesh
        monkeypatch.setattr(DeferredMesh, "defer", staticmethod(lambda *args: False))
    path = tmp_path / "weld-boundary.igz"
    igz.save_scene(scene, path)
    restored = Scene()
    igz.load_into(restored, path)
    assert len(restored.mesh.edges) == 0
    assert len(restored.mesh.vertices) == 1


def test_camera_relative_edit_through_a_far_mirrored_occurrence(monkeypatch):
    from built_core.coordinate_frame import mesh_frame
    from core.group import Group
    from core.history import SnapshotMutation
    from built_core import Matrix4, Vec3

    scene = Scene()
    group = Group()
    origin = (float(2**48), -float(2**48), 42.)
    transform = Matrix4()
    transform.translate(*origin)
    transform.rotate(33, 0, 0, 1)
    transform.scale(-2, 3, .5)
    group.xform = transform
    scene.groups.append(group)
    points = [Vec3(.00037, .02634, .04321), Vec3(.01867, .04678, .05432)]
    group.mesh.add_edge(*points)
    definition = group.mesh
    key = definition.vertices[0].key
    history = History(scene)
    scene.begin_group_edit(group)
    with mesh_frame(scene.mesh, origin):
        vertex = scene.mesh.vertices[0]
        np.testing.assert_allclose(vertex.position_tuple(), transform.mapVector(points[0]).toTuple(),
                                   atol=1e-14, rtol=0)
        moved = points[0] + Vec3(.00321, -.00432, .00876)
        history.execute(SnapshotMutation(lambda s: s.mesh.place_vertex(vertex, transform.mapVector(moved))))
        assert not history.last_error and not history.last_rejected
        assert vertex.key == key
    scene.end_group_edit()
    np.testing.assert_allclose(definition.vertices[0].position_tuple(), moved.toTuple(), atol=1e-14, rtol=0)
    assert definition.vertices[1].position_tuple() == points[1].toTuple()
    assert history.undo() and definition.vertices[0].position_tuple() == points[0].toTuple()
    assert history.redo() and definition.vertices[0].key == key
    np.testing.assert_allclose(definition.vertices[0].position_tuple(), moved.toTuple(), atol=1e-14, rtol=0)

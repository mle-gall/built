# SPDX-License-Identifier: GPL-3.0-or-later
"""Native tool commits on a model far from the origin update the resident
scene in place.

The shell's view repacked the whole scene on every Line, Eraser, Push/Pull
or Move commit of a model in site coordinates (seconds on a large model),
and on the first tool event after any document change. Each commit here
must leave the whole-scene pack count where it was and draw what a fresh
pack of the document draws, through undo and redo too."""
from __future__ import annotations

from tests.shell_isolation import open_test_host, attach_test_renderer, restore_shell_patches  # noqa: F401

import pytest
from built_core import Input as Qt, Matrix4 as QMatrix4x4, Vec3 as V
from built_core import pack_scene

from tests.test_incremental_rebased_edits import FAR, drawn, tile


def _model(path):
    from built_core.group import Group
    from built_core.scene import Scene
    from formats import igz
    scene = Scene()
    tile(scene.mesh, FAR, 0.0, n=12)
    slab = Group(name="Slab")
    tile(slab.mesh, 0.0, 0.0, n=6)
    m = QMatrix4x4()
    m.translate(FAR - 30.0, 0.0, 0.0)
    slab.xform = m
    scene.groups.append(slab)
    igz.save_scene(scene, path)


@pytest.fixture
def far_window(tmp_path, monkeypatch):
    path = tmp_path / "far.igz"
    _model(path)
    import views.wgpu_view as resident_view
    real = resident_view.pack_scene
    whole = []

    def counted(scene, *args, **kwargs):
        whole.append(scene)
        return real(scene, *args, **kwargs)
    # The view's whole-scene path packs through this name.
    monkeypatch.setattr(resident_view, "pack_scene", counted)
    with open_test_host(tmp_path / "profile", width=900, height=700) as host:
        win = host.window
        assert win.open_path(path) is not False
        for _ in range(3):
            host.idle()
            host.frame()
        attach_test_renderer(host)
        vp, view = win.viewport, host.view
        vp.camera.set_view("top")
        vp.camera.fit_to(V(FAR, 0, 0), V(FAR + 12, 12, 0))
        for _ in range(3):
            host.idle()
            host.frame()
        assert view.packed.model_translations is not None, "the model packs rebased"
        view.whole_packs = whole
        yield host, vp, view


def _px(view, p):
    view._sync_scene_camera()
    o = view.packed.origin
    got = view._world_to_pixel(V(p.x() - o[0], p.y() - o[1], p.z() - o[2]))
    return (got.x(), got.y()) if hasattr(got, "x") else tuple(got[:2])


def _settle(host):
    for _ in range(2):
        host.idle()
        host.frame()


def _move(host, view, p):
    host.pointer("move", *_px(view, p), 0, 0, 0)
    host.idle()


def _click(host, view, p):
    x, y = _px(view, p)
    host.pointer("move", x, y, 0, 0, 0)
    host.pointer("press", x, y, 1, 1, 0)
    host.pointer("release", x, y, 1, 0, 0)
    _settle(host)


def _right_click(host, view, point):
    pixel = _px(view, point)
    host.pointer("move", *pixel, 0, 0, 0)
    host.pointer("press", *pixel, 2, 2, 0)
    host.pointer("release", *pixel, 2, 0, 0)
    _settle(host)
    menu = view._ctx_menu
    assert menu is not None
    selected = list(menu.sel)
    _escape(host)
    return selected


def _escape(host):
    host.key(True, Qt.Key_Escape.value, 0)
    host.key(False, Qt.Key_Escape.value, 0)
    _settle(host)


def assert_resident_like_fresh(vp, view):
    tracker = view._instance_updates
    live = {uid: tris for uid, tris in drawn(view.packed).items() if tracker.resident.is_active(uid)}
    assert live == drawn(pack_scene(vp.scene))


def packs(view):
    """Whole-scene packs so far: the view's own count (its test hook) and
    the calls of the pack it makes them with."""
    return getattr(view, "full_packs", None), len(view.whole_packs)


def committed(host, vp, view, before, start, check_pick):
    assert len(vp.history.undo_stack) == before + 1, "the gesture committed"
    assert packs(view) == start, "a whole-scene pack"
    assert_resident_like_fresh(vp, view)
    host.menu("tool:select")
    check_pick()
    host.menu("undo")
    _settle(host)
    assert packs(view) == start, "undo repacked the scene"
    assert_resident_like_fresh(vp, view)
    host.menu("redo")
    _settle(host)
    assert packs(view) == start, "redo repacked the scene"
    assert_resident_like_fresh(vp, view)
    _move(host, view, W(6, 6))
    vp.scene.clear_selection()
    check_pick()


def W(x, y, z=0.0):
    return V(FAR + x, y, z)


def test_line_commit(far_window):
    host, vp, view = far_window
    start, before = packs(view), len(vp.history.undo_stack)
    host.menu("tool:line")
    _settle(host)
    _click(host, view, W(3.5, 3.5))
    _click(host, view, W(6.5, 3.5))
    _escape(host)

    def check_pick():
        mesh = vp.scene.mesh
        edge = mesh.find_edge(mesh.vertex_at(W(4, 3.5)), mesh.vertex_at(W(5, 3.5)))
        _move(host, view, W(4.5, 3.5))
        assert view.tools._mesh_hover == edge
        _click(host, view, W(4.5, 3.5))
        assert edge is not None and list(vp.scene.selection) == [edge]

    committed(host, vp, view, before, start, check_pick)


def test_erase_commit(far_window):
    host, vp, view = far_window
    start, before = packs(view), len(vp.history.undo_stack)
    host.menu("tool:eraser")
    _settle(host)
    x, y = _px(view, W(5.0, 4.0))
    _move(host, view, W(5.0, 4.0))
    host.pointer("press", x, y, 1, 1, 0)
    _settle(host)
    host.pointer("release", x, y, 1, 0, 0)
    _settle(host)

    def check_pick():
        merged = next(face for face in vp.scene.mesh.faces if len(face.loop) == 6)
        _click(host, view, W(5., 3.5))
        assert list(vp.scene.selection) == [merged]

    committed(host, vp, view, before, start, check_pick)


def test_push_pull_commit(far_window):
    host, vp, view = far_window
    start, before = packs(view), len(vp.history.undo_stack)
    host.menu("tool:pushpull")
    _settle(host)
    _click(host, view, W(8.5, 5.5))
    _click(host, view, W(8.5, 6.5))

    def check_pick():
        cap = next(face for face in vp.scene.mesh.faces
                   if all(v.position.z() < 0 for v in face.loop))
        point = W(8.5, 5.5, cap.loop[0].position.z())
        vp.camera.fit_to(point - V(1, 1, 0), point + V(1, 1, 0))
        _settle(host)
        _click(host, view, point)
        assert list(vp.scene.selection) == [cap]
        pixel = _px(view, point)
        host.pointer("double", *pixel, 1, 1, 0)
        host.pointer("release", *pixel, 1, 0, 0)
        loop = list(cap.loop)
        boundary = {vp.scene.mesh.find_edge(a, b) for a, b in zip(loop, loop[1:] + loop[:1])}
        assert set(vp.scene.selection) == {cap, *boundary}
        host.pointer("press", *pixel, 1, 1, 0)
        host.pointer("release", *pixel, 1, 0, 0)
        assert set(vp.scene.selection) == {*vp.scene.mesh.faces, *vp.scene.mesh.edges}

    committed(host, vp, view, before, start, check_pick)


def test_move_commit(far_window):
    host, vp, view = far_window
    start, before = packs(view), len(vp.history.undo_stack)
    host.menu("tool:move")
    _settle(host)
    _click(host, view, W(2.0, 6.0))
    _click(host, view, W(2.3, 6.2))

    def check_pick():
        mesh = vp.scene.mesh
        edge = mesh.find_edge(mesh.vertex_at(W(2., 5.)), mesh.vertex_at(W(2.3, 6.2)))
        _click(host, view, W(2.15, 5.6))
        assert edge is not None and list(vp.scene.selection) == [edge]

    committed(host, vp, view, before, start, check_pick)


def test_first_tool_after_a_document_change_does_not_repack(far_window):
    """The first tool event after any change of the document's version used
    to repack a far model whole (the first L press of a session)."""
    host, vp, view = far_window
    start = packs(view)
    vp.scene.version += 1
    host.key(True, Qt.Key_L.value, 0, "l")
    host.key(False, Qt.Key_L.value, 0, "l")
    _move(host, view, W(3.5, 3.5))
    _settle(host)
    assert type(vp.active_tool).__name__ == "LineTool"
    assert packs(view) == start


@pytest.mark.parametrize("cover", ["hidden", "off_tag", "section"])
def test_pick_visible_face_in_a_partly_hidden_mesh(tmp_path, cover):
    """The ID pass identifies a placement. Its closest mesh face can be hidden."""
    from built_core.layers import Layer, assign_layer
    from built_core.scene import Scene
    from built_core.section import SectionPlane
    from formats import igz

    scene = Scene()
    tile(scene.mesh, 0, 0, n=2, z=20)
    roof = scene.mesh.add_face([V(0, 0, 21), V(2, 0, 21), V(2, 2, 21), V(0, 2, 21)])
    if cover == "hidden":
        roof.attrs["hidden"] = True
    elif cover == "off_tag":
        scene.layers.append(Layer("Roof", visible=False))
        assign_layer(roof, "Roof")
    else:
        cut = SectionPlane(V(0, 0, 20.5), V(0, 0, 1))
        scene.section_planes.append(cut)
        scene.set_active_section(cut)
        scene.show_section_planes = False
    path = tmp_path / "levels.igz"
    igz.save_scene(scene, path)
    with open_test_host(tmp_path / "profile", width=900, height=700) as host:
        host.window.open_path(path)
        _settle(host)
        attach_test_renderer(host)
        vp, view = host.window.viewport, host.view
        vp.camera.set_view("top")
        vp.camera.fit_to(V(0, 0, 20), V(2, 2, 21))
        _settle(host)
        point = V(.5, .5, 20)
        pixel = _px(view, point)
        assert view.pick_row(*pixel) is not None
        expected = vp.scene.mesh.faces[0]
        assert view.tools.pick_face(*pixel) == expected
        host.menu("tool:pushpull")
        _move(host, view, point)
        assert view.tools._mesh_hover == expected
        host.menu("tool:select")
        _click(host, view, point)
        assert list(vp.scene.selection) == [expected]
        assert _right_click(host, view, point) == [expected]


def test_pick_appended_placement_after_incremental_edit(far_window):
    from built_core.group import Group
    from built_core.history import InsertGroupCommand

    host, vp, view = far_window
    old, empty = W(3.5, 3.5), W(3.5, -1.)
    face = view.tools.pick_face(*_px(view, old))
    assert face is not None
    assert _right_click(host, view, old) == [face]
    assert _right_click(host, view, empty) == []
    group = Group(name="Raised tile")
    tile(group.mesh, 0, 0, n=1)
    matrix = QMatrix4x4()
    matrix.translate(FAR + 4, 4., 1.)
    group.xform = matrix
    start = packs(view)
    vp.history.execute(InsertGroupCommand(group))
    _settle(host)
    assert packs(view) == start
    point = W(4.5, 4.5, 1)
    assert view.tools.pick_group(*_px(view, point)) is group
    _click(host, view, point)
    assert list(vp.scene.selection) == [group]
    assert _right_click(host, view, point) == [group]
    assert _right_click(host, view, old) == [face]
    assert _right_click(host, view, empty) == []


def test_pick_inside_rebased_group_after_an_unrelated_edit(far_window):
    from built_core.history import AddEdgeCommand

    host, vp, view = far_window
    slab = vp.scene.groups[0]
    vp.begin_group_edit(slab)
    vp.camera.set_view("top")
    vp.camera.fit_to(W(-30, 0), W(-24, 6))
    _settle(host)
    point = W(-28.5, 1.5)
    _click(host, view, point)
    expected, = vp.scene.selection
    assert expected in vp.scene.mesh.faces
    vp.history.execute(AddEdgeCommand(W(-24, 2), W(-23, 2)))
    _settle(host)
    vp.scene.clear_selection()
    _click(host, view, point)
    assert list(vp.scene.selection) == [expected]

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
def far_window(tmp_path):
    path = tmp_path / "far.igz"
    _model(path)
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


def _escape(host):
    host.key(True, Qt.Key_Escape.value, 0)
    host.key(False, Qt.Key_Escape.value, 0)
    _settle(host)


def assert_resident_like_fresh(vp, view):
    tracker = view._instance_updates
    live = {uid: tris for uid, tris in drawn(view.packed).items() if tracker.resident.is_active(uid)}
    assert live == drawn(pack_scene(vp.scene))


def committed(host, vp, view, before, packs):
    assert len(vp.history.undo_stack) == before + 1, "the gesture committed"
    assert view.full_packs == packs, "a whole-scene pack"
    assert_resident_like_fresh(vp, view)
    host.menu("undo")
    _settle(host)
    assert view.full_packs == packs, "undo repacked the scene"
    assert_resident_like_fresh(vp, view)
    host.menu("redo")
    _settle(host)
    assert view.full_packs == packs, "redo repacked the scene"
    assert_resident_like_fresh(vp, view)


def W(x, y, z=0.0):
    return V(FAR + x, y, z)


def test_line_commit(far_window):
    host, vp, view = far_window
    packs, before = view.full_packs, len(vp.history.undo_stack)
    host.menu("tool:line")
    _settle(host)
    _click(host, view, W(3.5, 3.5))
    _click(host, view, W(6.5, 3.5))
    _escape(host)
    committed(host, vp, view, before, packs)


def test_erase_commit(far_window):
    host, vp, view = far_window
    packs, before = view.full_packs, len(vp.history.undo_stack)
    host.menu("tool:eraser")
    _settle(host)
    x, y = _px(view, W(5.0, 4.0))
    _move(host, view, W(5.0, 4.0))
    host.pointer("press", x, y, 1, 1, 0)
    _settle(host)
    host.pointer("release", x, y, 1, 0, 0)
    _settle(host)
    committed(host, vp, view, before, packs)


def test_push_pull_commit(far_window):
    host, vp, view = far_window
    packs, before = view.full_packs, len(vp.history.undo_stack)
    host.menu("tool:pushpull")
    _settle(host)
    _click(host, view, W(8.5, 5.5))
    _click(host, view, W(8.5, 6.5))
    committed(host, vp, view, before, packs)


def test_move_commit(far_window):
    host, vp, view = far_window
    packs, before = view.full_packs, len(vp.history.undo_stack)
    host.menu("tool:move")
    _settle(host)
    _click(host, view, W(2.0, 6.0))
    _click(host, view, W(2.3, 6.2))
    committed(host, vp, view, before, packs)


def test_first_tool_after_a_document_change_does_not_repack(far_window):
    """The first tool event after any change of the document's version used
    to repack a far model whole (the first L press of a session)."""
    host, vp, view = far_window
    packs = view.full_packs
    vp.scene.version += 1
    host.key(True, Qt.Key_L.value, 0, "l")
    host.key(False, Qt.Key_L.value, 0, "l")
    _move(host, view, W(3.5, 3.5))
    _settle(host)
    assert type(vp.active_tool).__name__ == "LineTool"
    assert view.full_packs == packs

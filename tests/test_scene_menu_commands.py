# SPDX-License-Identifier: GPL-3.0-or-later
"""Scene menu actions on the native host and the document protocol."""
from types import SimpleNamespace

import pytest

from app_shell.window import ShellScenes, _SceneRow
from core import scene_folders
from core.camera import OrbitCamera
from core.composition import Composicion
from core.history import History
from core.protocol.commands import edit_saved_scene
from core.saved_views import SavedView
from core.scene import Scene


@pytest.fixture
def menu():
    scene = Scene()
    vp = SimpleNamespace(scene=scene, history=History(scene), camera=OrbitCamera(),
                         update=lambda: None, notify_scene_changed=lambda: None)
    messages = []
    host = SimpleNamespace(viewport=vp, _scene_tabs=_SceneRow(), active_style=lambda: None,
                           statusBar=lambda: SimpleNamespace(showMessage=lambda s, ms: messages.append(s)),
                           _doc=SimpleNamespace(stand_in=None))
    return ShellScenes(host), host, messages


def test_add_is_undoable(menu):
    actions, win, _ = menu
    view = actions.add_scene()
    assert view.name == "Scene 1" and win._scene_tabs.marked_view() is view
    vp = win.viewport
    vp.history.undo()
    assert list(vp.scene.saved_views) == []
    vp.history.redo()
    assert [v.name for v in vp.scene.saved_views] == ["Scene 1"]


def test_update_is_undoable(menu):
    actions, win, _ = menu
    view = actions.add_scene()
    vp = win.viewport
    before = view.distance
    vp.camera.distance += 37
    actions.update_view(view)
    assert view.distance == pytest.approx(before + 37)
    vp.history.undo()
    assert view.distance == pytest.approx(before)
    assert view.name == "Scene 1"
    vp.history.redo()
    assert view.distance == pytest.approx(before + 37)


def test_rename_in_the_row_is_undoable_and_carries_the_sheets(menu):
    actions, win, _ = menu
    a, b = (actions.add_scene() for _ in range(2))
    scene = win.viewport.scene
    page = Composicion(name="Sheet")
    frame = page.default_frame()
    frame.view_key = "scene:Scene 1"
    page.frames.append(frame)
    scene.compositions.append(page)
    assert actions.rename_view(a, "Ground floor")
    assert [v.name for v in scene.saved_views] == ["Ground floor", b.name]
    assert frame.view_key == "scene:Ground floor"
    win.viewport.history.undo()
    assert a.name == "Scene 1" and frame.view_key == "scene:Scene 1"


def test_a_name_another_scene_has_is_refused_and_nothing_to_undo(menu):
    actions, win, messages = menu
    a, b = (actions.add_scene() for _ in range(2))
    before = win.viewport.history.revision
    assert not actions.rename_view(b, a.name)
    assert [v.name for v in win.viewport.scene.saved_views] == ["Scene 1", "Scene 2"]
    assert win.viewport.history.revision == before
    assert "already called" in messages[-1]


def test_delete_is_still_undoable(menu):
    actions, win, _ = menu
    a, b = (actions.add_scene() for _ in range(2))
    actions.delete_views([a])
    assert list(win.viewport.scene.saved_views) == [b]
    win.viewport.history.undo()
    assert list(win.viewport.scene.saved_views) == [a, b]


def test_updating_a_scene_keeps_its_folder(menu):
    actions, win, _ = menu
    scene = win.viewport.scene
    scene_folders.add_folder(scene, "Plans", None)
    view = actions.add_scene()
    scene_folders.move_rows(scene, [view], [], "Plans")
    actions.update_view(view)
    assert view.folder == "Plans"


def test_a_tab_dragged_among_folders_lands_with_the_scene_after_it(menu):
    actions, win, _ = menu
    scene = win.viewport.scene
    a, b, c, d = (actions.add_scene() for _ in range(4))
    scene_folders.add_folder(scene, "F", None)
    scene_folders.move_rows(scene, [c, d], [], "F")
    assert list(scene.saved_views) == [c, d, a, b]
    steps = len(win.viewport.history.undo_stack)
    actions.move_view(a, 1)
    assert list(scene.saved_views) == [c, a, d, b] and a.folder == "F"
    assert len(win.viewport.history.undo_stack) == steps + 1
    actions.move_view(b, 3)
    assert list(scene.saved_views) == [c, a, d, b] and b.folder == "F"
    win.viewport.history.undo()
    win.viewport.history.undo()
    assert list(scene.saved_views) == [c, d, a, b] and a.folder is None


def test_plus_of_the_tabs_files_the_scene_with_the_one_shown(menu):
    actions, win, _ = menu
    scene = win.viewport.scene
    a, b = (actions.add_scene() for _ in range(2))
    scene_folders.add_folder(scene, "F", None)
    scene_folders.move_rows(scene, [a], [], "F")
    win._scene_tabs.mark_view(a)
    new = actions.add_scene()
    assert list(scene.saved_views) == [a, new, b]
    assert new.folder == "F" and win._scene_tabs.marked_view() is new


def test_stale_scene_in_a_batch_leaves_every_target_alone(menu):
    actions, win, _ = menu
    local = actions.add_scene()
    result = edit_saved_scene(win.viewport.history, "delete", views=[local, SavedView(local.name)])
    assert not result["ok"]
    assert list(win.viewport.scene.saved_views) == [local]

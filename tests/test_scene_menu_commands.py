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


def test_duplicate_from_the_menu_goes_right_after_and_undoes(menu):
    actions, win, _ = menu
    a, b = (actions.add_scene() for _ in range(2))
    actions.rename_view(a, "A")
    actions.rename_view(b, "B")
    first = actions.duplicate_view(a)
    scene = win.viewport.scene
    assert list(scene.saved_views) == [a, first, b] and first.name == "A (copy)"
    assert win._scene_tabs.marked_view() is first
    assert first.distance == a.distance and first.folder == a.folder
    second = actions.duplicate_view(a)
    assert second.name == "A (copy) 2"
    win.viewport.history.undo()
    win.viewport.history.undo()
    assert list(scene.saved_views) == [a, b]


def test_scene_folder_menu_keeps_reference_order_history_and_document_identity(menu):
    import json
    from ingetrazo_core import scene_tree

    actions, win, _ = menu
    a, b, c = (actions.add_scene() for _ in range(3))
    vp = win.viewport
    scene = vp.scene
    owner = str(id(scene))

    def edit(op, folder="", dest="", name=""):
        return scene_tree.edit_menu_folder(vp.history, json.dumps([op, owner, folder, dest, name]))

    parent = edit("add", name="Plans / drawing")
    child = edit("add", dest=parent, name="Floor 1")
    edit("file", dest=child, name=b.name)
    edit("file", dest=parent, name=c.name)
    scenes, folders = scene_tree.menu_rows(scene, b)
    assert [(r[1], r[3], r[4]) for r in scenes] == [(b.name, child, 1), (c.name, parent, 0), (a.name, "", 0)]
    assert (folders[0][0], folders[0][2], folders[0][3]) == ("", str(scene.content_version), owner)
    assert [(r[0], r[2]) for r in folders[1:]] == [(parent, ""), (child, parent)]
    with pytest.raises(ValueError):
        edit("move", parent, child)
    with pytest.raises(ValueError):
        edit("rename", parent, name=child)
    edit("remove", parent)
    assert not c.folder and scene_folders.folder_parent(scene, child) is None
    vp.history.undo()
    assert scene_folders.folder_parent(scene, child) == parent and c.folder == parent
    edit("rename", parent, name="Plans")
    assert scene_folders.folder_parent(scene, child) == "Plans"
    with pytest.raises(ValueError, match="another document"):
        scene_tree.edit_menu_folder(History(Scene()), json.dumps(["remove", owner, "Plans", "", ""]))

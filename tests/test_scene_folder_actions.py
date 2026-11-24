# SPDX-License-Identifier: GPL-3.0-or-later
"""File and recall saved views through the native Scene chooser."""
import json

import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.scene_controls import choose, prompt, rows
from tests.test_scene_tabs import add, checked, labels
from views.wgpu_ui import _run


@pytest.fixture
def win(native_host):
    return native_host.window


def edit(win, verb, folder="", dest="", label="", *, document=None):
    scene = win.viewport.scene if document is None else document
    request = [verb, str(id(scene)), folder, dest, label]
    _run(win, ["tab:scenefolders:" + json.dumps(request)])


def folders(win):
    rows(win)
    return [(item[0], item[2]) for item in win.view._ui_tabs[3][1:]]


def test_scenes_keep_their_folder_order_through_save(win, tmp_path):
    from core.scene import Scene
    from formats import igz

    ground, first, north = [add(win, name) for name in ("Ground", "First", "North")]
    edit(win, "add", label="Plans")
    edit(win, "add", dest="Plans", label="Floors")
    edit(win, "file", dest="Floors", label=first.name)
    edit(win, "file", dest="Plans", label=ground.name)
    assert folders(win) == [("Plans", ""), ("Floors", "Plans")]
    assert [(r[1], r[3]) for r in rows(win)] == [
        ("First", "Floors"), ("Ground", "Plans"), ("North", "")]
    path = tmp_path / "scene-folders.built"
    igz.save_scene(win.viewport.scene, path)
    reopened = Scene()
    igz.load_into(reopened, path)
    assert reopened.scene_folder_parents["Floors"] == "Plans"
    assert [(v.name, v.folder) for v in reopened.saved_views] == [
        (first.name, "Floors"), (ground.name, "Plans"), (north.name, None)]


def test_cyclic_move_and_duplicate_rename_leave_rows_and_history_intact(win, monkeypatch):
    edit(win, "add", label="Plans")
    edit(win, "add", dest="Plans", label="Floor")
    history = win.viewport.history
    before = history.revision
    errors = []
    monkeypatch.setattr(win.viewport, "flash_status", errors.append)
    edit(win, "move", "Plans", "Floor")
    edit(win, "rename", "Floor", label="Plans")
    assert len(errors) == 2 and "already called" in errors[1]
    assert history.revision == before
    assert folders(win) == [("Plans", ""), ("Floor", "Plans")]


def test_capture_files_the_new_scene_in_the_open_folder_and_rejects_stale_menus(win):
    outside = add(win, "Outside")
    edit(win, "add", label="Plans")
    before = len(win.viewport.history.undo_stack)
    assert checked(win) == [outside.name]
    edit(win, "capture", "Plans")
    inside = win._scene_tabs.marked_view()
    assert inside is not outside and inside.folder == "Plans"
    assert len(win.viewport.history.undo_stack) == before + 1
    win._on_undo()
    assert labels(win) == ["Outside"]
    old = win.viewport.scene
    win.new_document()
    edit(win, "capture", "Plans", document=old)
    assert labels(win) == []


def test_each_folder_operation_undoes_as_one_step(win):
    ground = add(win, "Ground")
    history = win.viewport.history
    before = len(history.undo_stack)
    edit(win, "add", label="Folder")
    edit(win, "rename", "Folder", label="Plans")
    edit(win, "add", dest="Plans", label="Floor")
    edit(win, "file", dest="Floor", label=ground.name)
    edit(win, "remove", "Floor")
    assert len(history.undo_stack) == before + 5
    assert folders(win) == [("Plans", "")] and ground.folder == "Plans"
    for expected, parent in [
        (["Plans", "Floor"], "Floor"), (["Plans", "Floor"], None),
        (["Plans"], None), (["Folder"], None), ([], None),
    ]:
        win._on_undo()
        assert [f for f, _ in folders(win)] == expected and ground.folder == parent
    for _ in range(5):
        win._on_redo()
    assert folders(win) == [("Plans", "")] and ground.folder == "Plans"


def test_move_before_a_filed_scene_and_back_to_root(win):
    a, b, c = [add(win, n) for n in "ABC"]
    edit(win, "add", label="F")
    for saved in [a, b]:
        edit(win, "file", dest="F", label=saved.name)
    before = len(win.viewport.history.undo_stack)
    win.tray.scenes.move_view(c, 0)
    assert labels(win) == ["C", "A", "B"] and c.folder == "F"
    assert len(win.viewport.history.undo_stack) == before + 1
    win._on_undo()
    assert labels(win) == ["A", "B", "C"] and c.folder is None
    win._on_redo()
    edit(win, "file", label=b.name)
    assert [(r[1], r[3]) for r in rows(win)] == [("C", "F"), ("A", "F"), ("B", "")]


def test_the_mark_follows_recall_and_clears_on_deletion(win):
    add(win, "A")
    add(win, "B")
    for i, name in [(0, "A"), (1, "B"), (0, "A")]:
        choose(win, i)
        assert checked(win) == [name]
    prompt(win, "scene_delete", "yes")
    assert checked(win) == [] and labels(win) == ["B"]


def test_scene_picture_mode_is_saved_by_the_chooser(win):
    from core.settings import settings
    for action, stored in [("0", False), ("1", True)]:
        _run(win, ["scenephotomode:" + action])
        assert settings().value("scenes/thumbnails", type=bool) is stored

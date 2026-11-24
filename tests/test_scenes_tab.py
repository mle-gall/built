# SPDX-License-Identifier: GPL-3.0-or-later
"""Saved scene commands, history and navigation in the native chooser."""
import pytest

from core import scene_folders as SF
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_one_command_of_each_kind_for_scenes():
    """Main's AddSceneCommand (with a place and the folder now) and its
    RenameSceneCommand: no second class of the same name beside them."""
    import core.history as H
    import core.saved_views as S
    import core.history_commands as commands
    assert H.AddSceneCommand is commands.AddSceneCommand
    src = open(commands.__file__, encoding="utf8").read()
    assert src.count("class AddSceneCommand(") == 1
    assert "class RenameSceneCommand(" not in src
    assert hasattr(S, "RenameSceneCommand") and hasattr(S, "MoveSceneCommand")


def test_move_earlier_and_later_stay_within_the_marked_scene_folder(native_host):
    from tests.scene_controls import choose, rows
    from views.wgpu_ui import _run
    win = native_host.window
    scene = win.viewport.scene
    a, b, c = [win.tray.scenes.add_scene() for _ in range(3)]
    SF.add_folder(scene, "Plans", None)
    SF.move_rows(scene, [b, c], [], "Plans")
    scene.version += 1
    assert list(scene.saved_views) == [b, c, a]
    choose(win, 1)
    _run(win, ["tab:scene:up"])
    assert list(scene.saved_views) == [c, b, a]
    steps = len(win.viewport.history.undo_stack)
    _run(win, ["tab:scene:up"])
    assert len(win.viewport.history.undo_stack) == steps
    _run(win, ["tab:scene:down", "tab:scene:down"])
    assert list(scene.saved_views) == [b, c, a]
    assert len(win.viewport.history.undo_stack) == steps + 1
    assert [r[1] for r in rows(win) if r[4] == 1] == [c.name]
    assert SF.folder_of(scene, c) == "Plans"


def test_update_from_the_scene_chooser_is_one_undo_step(native_host):
    win = native_host.window
    win.run_command("scene_add")
    view, = win.viewport.scene.saved_views
    before = view.distance
    win.viewport.camera.distance = before + 25.0
    win.run_command("scene_update")
    assert view.distance == pytest.approx(before + 25.0)
    win.viewport.history.undo()
    assert view.distance == pytest.approx(before)


def test_a_scene_shown_from_the_menu_is_marked_in_the_chooser(native_host):
    from tests.scene_controls import rows
    win = native_host.window
    win.run_command("scene_add")
    win.run_command("scene_add")
    win.run_command("scene_previous")
    win.run_command("scene_show")
    assert [r[4] for r in rows(win)] == [1, 0]


def test_insert_scene_clamps_large_integer_positions():
    from core.history import AddSceneCommand
    from core.saved_views import SavedView
    from core.scene import Scene

    scene = Scene()
    centre, first, last = [SavedView(name) for name in ("Centre", "First", "Last")]
    scene.saved_views.append(centre)
    AddSceneCommand(first, -(2**100)).do(scene)
    AddSceneCommand(last, 2**100).do(scene)
    assert list(scene.saved_views) == [first, centre, last]

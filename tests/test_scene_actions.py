"""Scene commands share the native Camera menu, Keyboard table and document history."""
import pytest

from core.input_types import Qt
from tests.scene_controls import choose, prompt
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_render.settings_sheet import SettingsSheet


KEYS = ("scene_show", "scene_add", "scene_update", "scene_rename", "scene_duplicate", "scene_delete")


@pytest.fixture
def win(native_host):
    return native_host.window


def keyboard(win):
    sheet = SettingsSheet("keyboard")
    sheet._win = win
    return sheet


def test_each_takes_a_key_in_preferences_and_has_none_yet(win):
    sheet = keyboard(win)
    for key in KEYS:
        action = sheet._action("sc:menu:" + key)
        assert action in sheet.actions() and action.portable_keys() == []


def test_they_live_in_camera_scenes(win):
    for key in KEYS:
        assert win._command_paths["menu:" + key] == "Camera ▸ Scenes"


def test_add_update_and_delete_reach_history(win):
    assert win.run_command("scene_add")
    added, = win.viewport.scene.saved_views
    assert "added" in win.statusBar().currentMessage()
    win.viewport.camera.distance *= 2
    win.run_command("scene_update")
    assert added.distance == pytest.approx(win.viewport.camera.distance)
    prompt(win, "scene_delete", "yes")
    assert not win.viewport.scene.saved_views
    assert win.viewport.history.undo()
    assert win.viewport.scene.saved_views == [added]


def test_without_a_selected_scene_the_status_explains(win):
    for key in KEYS:
        if key != "scene_add":
            win.statusBar().showMessage("", 0)
            assert win.run_command(key)
            assert "Select a scene" in win.statusBar().currentMessage()
    assert not win.viewport.scene.saved_views


def test_a_key_given_to_add_scene_adds_one(win):
    sheet = keyboard(win)
    sheet._assign(sheet._action("sc:menu:scene_add"), ["Ctrl+Alt+Shift+F11"])
    modifiers = (Qt.ControlModifier | Qt.AltModifier | Qt.ShiftModifier).value
    win._host.key(True, Qt.Key_F11, modifiers)
    win._host.key(False, Qt.Key_F11, modifiers)
    assert len(win.viewport.scene.saved_views) == 1


def test_show_scene_brings_the_selected_camera_back(win):
    win.run_command("scene_add")
    view, = win.viewport.scene.saved_views
    win.viewport.camera.distance *= 3
    win.run_command("scene_show")
    assert win.viewport.camera.distance == pytest.approx(view.distance)
    assert view.name in win.statusBar().currentMessage()


def test_rename_from_the_camera_menu_validates_and_can_be_cancelled(win):
    for _ in range(2):
        win.run_command("scene_add")
    other, view = win.viewport.scene.saved_views
    before = view.name
    prompt(win, "scene_rename", "yes", other.name)
    assert view.name == before and other.name in win.statusBar().currentMessage()
    prompt(win, "scene_rename", "yes", "  Fachada  ")
    assert view.name == "Fachada" and win._scene_tabs.marked_view() is view
    prompt(win, "scene_rename", "no", "Outra")
    assert view.name == "Fachada"


def test_duplicate_inserts_after_the_original_and_never_reuses_a_name(win):
    for _ in range(2):
        win.run_command("scene_add")
    scene = win.viewport.scene
    original = scene.saved_views[0]
    for _ in range(2):
        choose(win, 0)
        win.run_command("scene_duplicate")
        copy = scene.saved_views[1]
        assert copy is win._scene_tabs.marked_view() and copy is not original
        assert copy.to_dict() | {"name": original.name} == original.to_dict()
    assert len({v.name for v in scene.saved_views}) == 4

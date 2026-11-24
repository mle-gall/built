"""The winit display presents each camera pose and owns playback interruption."""
from unittest.mock import Mock
import pytest

from core.scene_animation import SceneAnimation
from tests.scene_controls import choose, interrupt
from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.fixture
def win(native_host, monkeypatch):
    w = native_host.window
    vp = w.viewport
    for pose in ("top", "front"):
        vp.camera.set_view(pose)
        w.run_command("scene_add")
    vp.camera.set_view("iso")
    # A surface is attached. Use a manual clock and count its frame requests.
    monkeypatch.setattr(w.view, "renderer", Mock())
    monkeypatch.setattr(w.view, "update", Mock())
    return w


def test_a_scene_glides_on_the_metal_view(win, monkeypatch):
    player = win.scene_player
    time = 100.
    monkeypatch.setattr(player, "_clock", lambda: time)
    choose(win, 0)
    start = win.viewport.camera.pitch
    target = win.viewport.scene.saved_views[0].pitch
    assert player.is_active()
    time += .5
    player.tick()
    assert start < win.viewport.camera.pitch < target
    assert win.view.update.called and player._display is win.view
    time += 5.
    player.tick()
    assert not player.is_active() and player._display is None
    assert win.viewport.camera.pitch == pytest.approx(target)


@pytest.mark.parametrize("hidden", [True, False])
def test_a_hidden_display_or_disabled_transition_jumps(win, hidden):
    if hidden:
        win.view.renderer = None
    else:
        win.viewport.scene.animation = SceneAnimation(enabled=False)
    choose(win, 0)
    assert not win.scene_player.is_active()
    assert win.viewport.camera.pitch == pytest.approx(win.viewport.scene.saved_views[0].pitch)


@pytest.mark.parametrize("kind", ["press", "double", "wheel", "esc"])
def test_input_stops_the_glide_and_releases_the_display(win, kind):
    choose(win, 0)
    assert win.scene_player._display is win.view
    interrupt(win, kind)
    assert not win.scene_player.is_active() and win.scene_player._display is None


def test_escape_stops_both_glide_and_play(win):
    choose(win, 1)
    assert win.scene_player.is_active()
    interrupt(win, "esc")
    assert not win.scene_player.is_active()
    win.run_command("scene_play")
    assert win.scene_player.is_playing()
    interrupt(win, "esc")
    assert not win.scene_player.is_active()

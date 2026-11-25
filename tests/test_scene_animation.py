# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Scene transitions (core/scene_animation.py, views/scene_playback.py):
showing a scene glides the camera there instead of jumping, the rest of
the scene comes in on arrival, Play runs through every scene in turn,
anything the user does takes the view back, and the document keeps its
transition and pause times."""
from __future__ import annotations

import math
from unittest.mock import Mock

import pytest

from tests.scene_controls import choose, interrupt
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from ingetrazo_core import (Pose, SceneRun, SceneAnimation,
                                  between, ease, glide, turn)


def _pose(x=0.0, distance=10.0, yaw=0.0, pitch=0.3, fov=45.0,
          perspective=True, two_point=False):
    return Pose((x, 0.0, 0.0), distance, yaw, pitch, fov, perspective,
                two_point)


# ---- The maths ------------------------------------------------------------------
def test_ease_starts_and_stops_gently():
    assert ease(0.0) == 0.0 and ease(1.0) == pytest.approx(1.0)
    assert ease(0.5) == pytest.approx(0.5)
    assert ease(-1.0) == 0.0 and ease(2.0) == pytest.approx(1.0)
    samples = [ease(i / 20) for i in range(21)]
    assert samples == sorted(samples)
    # Flat at the ends: the first and last steps are the smallest.
    steps = [b - a for a, b in zip(samples, samples[1:])]
    assert steps[0] < steps[10] and steps[-1] < steps[10]


def test_yaw_turns_the_short_way_round():
    a, b = math.radians(170), math.radians(-170)
    assert math.degrees(turn(a, b)) == pytest.approx(20.0)
    assert math.degrees(turn(b, a)) == pytest.approx(-20.0)
    mid = between(_pose(yaw=a), _pose(yaw=b), 0.5)
    assert math.degrees(mid.yaw) % 360 == pytest.approx(180.0)


def test_the_glide_starts_where_the_camera_is_and_ends_on_the_scene():
    a = _pose(x=0, distance=1, yaw=0.0, pitch=0.2, fov=40)
    b = _pose(x=10, distance=100, yaw=1.0, pitch=0.8, fov=60)
    assert between(a, b, 0.0) == a
    assert between(a, b, 1.0) == b
    mid = between(a, b, 0.5)
    assert mid.target[0] == pytest.approx(5.0)
    # Distance in proportion: halfway from 1 m to 100 m is 10 m, not 50.
    assert mid.distance == pytest.approx(10.0)
    assert (mid.yaw, mid.pitch, mid.fov_deg) == pytest.approx((0.5, 0.5, 50))


def test_a_glide_of_no_time_or_past_its_end_is_the_scene():
    a, b = _pose(x=0), _pose(x=4)
    assert glide(a, b, 0.0, 0.0) == b
    assert glide(a, b, 2.0, 2.5) == b
    assert glide(a, b, 2.0, 1.0).target[0] == pytest.approx(2.0)


@pytest.mark.parametrize("a_persp,b_persp,during", [
    (True, False, True),        # to a plan: perspective until it arrives
    (False, True, True),        # from a plan: perspective at once
    (False, False, False),      # plan to elevation: parallel all the way
    (True, True, True)])
def test_the_glide_is_parallel_only_between_two_parallel_scenes(
        a_persp, b_persp, during):
    a, b = _pose(perspective=a_persp), _pose(x=5, perspective=b_persp)
    assert between(a, b, 0.0).perspective is a_persp
    assert between(a, b, 0.5).perspective is during
    assert between(a, b, 1.0).perspective is b_persp


def test_two_point_holds_only_between_two_two_point_scenes():
    a, b = _pose(two_point=True), _pose(x=3, two_point=False)
    assert between(a, b, 0.5).two_point is False
    assert between(a, b, 1.0).two_point is False
    c = _pose(x=6, two_point=True)
    assert between(a, c, 0.5).two_point is True


def test_close_to_ignores_a_full_turn_and_rounding():
    a = _pose(yaw=0.1)
    assert a.close_to(_pose(yaw=0.1 + 2 * math.pi))
    assert a.close_to(_pose(yaw=0.1 + 1e-9))
    assert not a.close_to(_pose(yaw=0.2))
    assert not a.close_to(_pose(yaw=0.1, perspective=False))


def test_a_run_glides_pauses_and_goes_round():
    start = _pose(x=-1)
    poses = [_pose(x=0), _pose(x=10), _pose(x=20)]
    run = SceneRun(start, poses, transition=2.0, pause=1.0)
    assert run.duration() == pytest.approx(9.0)
    pose, reached = run.at(0.0)
    assert pose == start and reached == -1
    pose, reached = run.at(1.0)                    # gliding to the first
    assert -1 < pose.target[0] < 0 and reached == -1
    assert run.at(2.0) == (poses[0], 0)            # arrived, pausing
    assert run.at(2.9) == (poses[0], 0)
    pose, reached = run.at(4.0)                    # on the way to the second
    assert pose.target[0] == pytest.approx(5.0) and reached == 0
    assert run.at(5.5) == (poses[1], 1)
    assert run.at(8.5) == (poses[2], 2)
    pose, reached = run.at(10.0)                   # round: last → first
    assert pose.target[0] == pytest.approx(10.0) and reached == 2
    assert run.at(11.0) == (poses[0], 0)


def test_a_run_with_no_time_shows_the_first_scene():
    run = SceneRun(_pose(x=-1), [_pose(x=0), _pose(x=1)], 0.0, 0.0)
    assert run.at(5.0) == (run.poses[0], 0)
    assert SceneRun(_pose(), [], 1.0, 1.0).at(3.0)[1] == -1


# ---- The settings -------------------------------------------------------------
def test_settings_default_and_write_nothing_when_untouched():
    anim = SceneAnimation()
    assert (anim.enabled, anim.transition, anim.pause) == \
        (True, 2.0, 1.0)
    assert anim.to_dict() == {}
    assert anim.glide_seconds() == 2.0
    assert SceneAnimation(enabled=False).glide_seconds() == 0.0


def test_settings_round_trip_and_refuse_nonsense():
    anim = SceneAnimation(enabled=False, transition=3.5, pause=0.0)
    assert SceneAnimation.from_dict(anim.to_dict()) == anim
    odd = SceneAnimation.from_dict({"transition": "x", "pause": -4})
    assert odd.transition == 2.0 and odd.pause == 0.0
    assert SceneAnimation(transition=2000).transition == 60.0
    assert SceneAnimation(transition=float("nan")).transition == \
        2.0
    assert SceneAnimation.from_dict(None) == SceneAnimation()


def test_the_document_keeps_its_transition_times(tmp_path):
    from core.scene import Scene
    from formats import igz
    scene = Scene()
    assert scene.animation == SceneAnimation()
    scene.animation = SceneAnimation(transition=4.0, pause=2.5)
    path = tmp_path / "animada.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.animation == SceneAnimation(transition=4.0, pause=2.5)


def test_an_old_document_opens_with_the_default_times(tmp_path):
    from core.scene import Scene
    from formats import igz
    path = tmp_path / "sin-ajustes.igz"
    igz.save_scene(Scene(), path)
    back = Scene()
    back.animation = SceneAnimation(enabled=False)
    igz.load_into(back, path)
    assert back.animation == SceneAnimation()


def test_a_new_drawing_forgets_the_times():
    from core.scene import Scene
    scene = Scene()
    scene.animation = SceneAnimation(pause=5.0)
    scene.clear()
    assert scene.animation == SceneAnimation()


# ---- The player ----------------------------------------------------------------
class _Clock:
    def __init__(self) -> None:
        self.now = 100.0

    def __call__(self) -> float:
        return self.now


@pytest.fixture
def rig(native_host, monkeypatch):
    win = native_host.window
    clock = _Clock()
    monkeypatch.setattr(win.scene_player, "_clock", clock)
    monkeypatch.setattr(win.viewport, "update", Mock())
    win.viewport.camera.target = (0, 0, 0)
    return win, win.scene_player, clock


def _view(name, view="top", x=0.0):
    from ingetrazo_core import Vec3 as QVector3D

    from core.camera import OrbitCamera
    from core.saved_views import SavedView
    from core.scene import Scene
    cam = OrbitCamera()
    cam.set_view(view)
    cam.target = QVector3D(x, 0, 0)
    return SavedView.capture(name, Scene(), cam)


def test_a_glide_moves_the_camera_bit_by_bit_and_arrives(rig):
    window, player, clock = rig
    cam = window.viewport.camera
    view = _view("Planta", "top", x=8.0)
    arrived = []
    assert player.glide_to(view, 2.0, arrived.append)
    assert player.is_active() and not player.is_playing()
    clock.now += 1.0
    player.tick()
    assert 0.0 < cam.target.x() < 8.0                # halfway, not there yet
    assert arrived == []
    clock.now += 1.5
    player.tick()
    assert cam.target.x() == pytest.approx(8.0)
    assert cam.pitch == pytest.approx(view.pitch)
    assert arrived == [view] and not player.is_active()
    assert window.viewport.update.call_count == 2


def test_no_glide_without_time_or_when_already_there(rig):
    window, player, _clock = rig
    view = _view("Planta")
    assert not player.glide_to(view, 0.0, lambda v: None)
    view.apply_camera(window.viewport.camera)
    assert not player.glide_to(view, 2.0, lambda v: None)
    assert not player.is_active()


def test_moving_the_camera_mid_glide_stops_it_where_it_is(rig):
    window, player, clock = rig
    cam = window.viewport.camera
    view = _view("Planta", x=8.0)
    arrived = []
    player.glide_to(view, 2.0, arrived.append)
    clock.now += 0.5
    player.tick()
    cam.yaw += 0.4                               # the user orbits
    taken = Pose.of_camera(cam)
    clock.now += 0.5
    player.tick()
    assert not player.is_active()
    assert Pose.of_camera(cam) == taken          # left where the user put it
    assert arrived == [view]                     # its layers and style come


@pytest.mark.parametrize("kind", ["press", "wheel", "esc"])
def test_a_click_the_wheel_or_esc_in_the_view_stops_it(rig, kind):
    window, player, _ = rig
    player.glide_to(_view("Planta", x=8), 2., lambda v: None)
    interrupt(window, kind)
    assert not player.is_active()


def test_the_player_releases_the_display_after_stop_or_cancel(rig):
    window, player, _ = rig
    arrived = []
    player.glide_to(_view("A", x=8), 2., arrived.append)
    assert player._display is window.view
    interrupt(window, "press")
    assert len(arrived) == 1 and player._display is None
    player.glide_to(_view("B", x=-8), 2., arrived.append)
    player.cancel()
    interrupt(window, "press")
    assert len(arrived) == 1 and player._display is None


def test_another_project_in_front_stops_it(rig):
    window, player, clock = rig
    player.glide_to(_view("Planta", x=8.0), 2.0, lambda v: None)
    window.new_document()
    clock.now += 0.5
    player.tick()
    assert not player.is_active()


def test_a_new_scene_mid_glide_takes_over_without_the_old_ones_state(rig):
    window, player, clock = rig
    first, second = _view("A", x=8.0), _view("B", "front", x=-8.0)
    arrived = []
    player.glide_to(first, 2.0, arrived.append)
    clock.now += 0.5
    player.tick()
    assert player.glide_to(second, 2.0, arrived.append)
    clock.now += 3.0
    player.tick()
    assert arrived == [second]


def test_play_runs_through_every_scene_in_turn(rig):
    window, player, clock = rig
    views = [_view("A", x=0.0), _view("B", "front", x=10.0),
             _view("C", "right", x=20.0)]
    window.viewport.scene.saved_views = list(views)
    reached = []
    assert player.play(views, 1, 1.0, 1.0, reached.append)
    assert player.is_playing()
    for _ in range(14):                       # 7 s in half-second frames
        clock.now += 0.5
        player.tick()
    # From the scene after the one shown (index 1 → B), round to A again.
    assert [v.name for v in reached] == ["B", "C", "A", "B"]
    assert player.is_playing()
    player.stop()
    assert not player.is_active()


def test_play_stops_when_its_next_scene_was_deleted(rig):
    window, player, clock = rig
    views = [_view("A", x=0.0), _view("B", x=10.0)]
    window.viewport.scene.saved_views = [views[0]]     # B deleted meanwhile
    reached = []
    player.play(views, 0, 1.0, 0.0, reached.append)
    for _ in range(6):
        clock.now += 0.5
        player.tick()
    assert [v.name for v in reached] == ["A"]
    assert not player.is_active()


def test_play_with_a_parallel_scene_does_not_stop_itself(rig):
    """Arriving at a parallel scene switches the projection: that is the
    player's own doing, not the user taking the view back."""
    window, player, clock = rig
    plan = _view("Planta", "top", x=5.0)
    plan.perspective = False
    views = [plan, _view("Vista", "iso", x=0.0)]
    window.viewport.scene.saved_views = list(views)

    def arrive(view):
        view.apply_state(window.viewport.scene)
    player.play(views, 0, 1.0, 1.0, arrive)
    for _ in range(5):
        clock.now += 0.5
        player.tick()
    assert player.is_playing()


# ---- In the window ------------------------------------------------------------
@pytest.fixture
def win(native_host):
    return native_host.window


def _scenes(win, *specs):
    from core.saved_views import SavedView
    cam = win.viewport.camera
    for name, view, hidden in specs:
        cam.set_view(view)
        scene = win.viewport.scene
        for ly in scene.layers:
            ly.visible = ly.name not in hidden
        scene.saved_views.append(SavedView.capture(name, scene, cam))
    win.viewport.scene.version += 1


def _visible(win):
    """Make the window's checks believe the viewport is on screen (the
    tests run with the window unshown, where a scene still jumps)."""
    win.view.renderer = Mock()


def _with_layer(win, name):
    from core.layers import Layer
    win.viewport.scene.layers.append(Layer(name))


def test_a_scene_tab_glides_and_its_layers_come_on_arrival(win):
    _with_layer(win, "Muebles")
    _scenes(win, ("Planta", "top", {"Muebles"}), ("Vista", "iso", set()))
    _visible(win)
    clock = _Clock()
    win.scene_player._clock = clock
    win.viewport.camera.set_view("iso")
    choose(win, 0)
    player = win.scene_player
    assert player.is_active()
    assert win._scene_tabs.marked_view() is win.viewport.scene.saved_views[0]      # marked at once
    muebles = [ly for ly in win.viewport.scene.layers
               if ly.name == "Muebles"][0]
    assert muebles.visible                     # not yet: still on the way
    clock.now += 1.0
    player.tick()
    assert muebles.visible
    clock.now += 2.0
    player.tick()
    assert not player.is_active()
    assert not muebles.visible                 # arrived: the plan's layers
    top = type(win.viewport.camera)()
    top.set_view("top")
    assert win.viewport.camera.pitch == pytest.approx(top.pitch)


def test_with_transitions_off_a_scene_still_jumps(win):
    _scenes(win, ("Planta", "top", set()))
    _visible(win)
    win.viewport.scene.animation = SceneAnimation(enabled=False)
    win.viewport.camera.set_view("iso")
    choose(win, 0)
    assert not win.scene_player.is_active()
    top = type(win.viewport.camera)()
    top.set_view("top")
    assert win.viewport.camera.pitch == pytest.approx(top.pitch)


def test_next_and_previous_scene_go_round_the_ends(win):
    _scenes(win, ("A", "top", set()), ("B", "front", set()),
            ("C", "right", set()))
    choose(win, 0)
    shown = []
    win.tray.scenes.show_view = lambda v, animate=True: (
        shown.append(v.name), win._scene_tabs.mark_view(v))
    win.step_scene(1)
    win.step_scene(1)
    win.step_scene(1)
    win.step_scene(1)
    win.step_scene(-1)
    assert shown == ["B", "C", "A", "B", "A"]


def test_play_from_the_menu_and_stop_follows_the_menu(win):
    _scenes(win, ("A", "top", set()), ("B", "front", set()))
    win.run_command("scene_play")
    assert win.scene_player.is_playing()
    win.run_command("scene_play")
    assert not win.scene_player.is_active()


def test_the_window_s_esc_stops_a_glide_and_keeps_the_selection(win):
    """Esc reaches the window's «Cancel current tool» shortcut, never the
    viewport the player filters: the camera glided on to the scene
    (Windows round 0.6.2). It stops the glide and does nothing else."""
    _scenes(win, ("Planta", "top", set()), ("Alzado", "front", set()))
    _visible(win)
    win.viewport.camera.set_view("iso")
    choose(win, 1)
    assert win.scene_player.is_active()
    scene = win.viewport.scene
    from core.group import Group
    from core.mesh import Mesh
    group = Group(Mesh(), "Esc selection")
    scene.groups.append(group)
    win.view.set_scene(scene)
    scene.select([group])
    interrupt(win, "esc")
    assert not win.scene_player.is_active()
    assert set(scene.selection) == {group}
    interrupt(win, "esc")               # nothing moving: the cascade
    assert not scene.selection


def test_the_window_s_esc_stops_play(win):
    _scenes(win, ("A", "top", set()), ("B", "front", set()))
    win.play_scenes(True)
    assert win.scene_player.is_playing()
    interrupt(win, "esc")
    assert not win.scene_player.is_active()


def test_play_without_scenes_says_why(win):
    win.play_scenes(True)
    assert not win.scene_player.is_active()
    from core.i18n import tr
    assert win.statusBar().currentMessage().startswith(
        tr("This project has no scenes to play: keep a view as a scene "
           "first (Scenes panel, «+ Scene»).")[:20])


def test_page_up_and_down_are_no_other_commands_shortcut(win):
    from core.shortcuts import action_keys
    from ingetrazo_render.settings_sheet import SettingsSheet
    sheet = SettingsSheet("keyboard")
    sheet._win = win
    for key, command in (("PgDown", "scene_next"), ("PgUp", "scene_previous")):
        action = sheet._action("sc:menu:" + command)
        assert [a for a in sheet.actions() if key in action_keys(a)] == [action]


def test_the_settings_change_the_document(win, monkeypatch):
    forms = []
    monkeypatch.setattr("ingetrazo_render.chrome_forms.show", lambda _, form: forms.append(form))
    before = win.viewport.scene.version
    win.ask_animation()
    form = forms.pop()
    form.receive("transition", "5")
    form.press("ok")
    assert win.viewport.scene.animation.transition == 5.0
    assert win.viewport.scene.version > before
    after = win.viewport.scene.version
    win.ask_animation()
    forms.pop().press("cancel")
    assert win.viewport.scene.version == after


def test_the_settings_dialog_reads_and_returns_the_times(win, monkeypatch):
    forms = []
    monkeypatch.setattr("ingetrazo_render.chrome_forms.show", lambda _, form: forms.append(form))
    win.ask_animation()
    form = forms.pop()
    form.receive("transition", "3")
    form.receive("pause", "0.5")
    form.press("ok")
    assert win.viewport.scene.animation == SceneAnimation(transition=3.0, pause=0.5)


def test_the_menu_turns_the_transitions_off_and_on(win):
    """Camera ▸ Animation ▸ Scene Transitions: one click, no dialog."""
    _scenes(win, ("Planta", "top", set()))
    _visible(win)
    assert win.viewport.scene.animation.enabled
    before = win.viewport.scene.version
    win.run_command("scene_transitions")                                   # off
    assert not win.viewport.scene.animation.enabled
    assert win.viewport.scene.version > before      # the project changed
    win.viewport.camera.set_view("iso")
    choose(win, 0)
    assert not win.scene_player.is_active()         # it jumped
    win.run_command("scene_transitions")                                   # on again
    assert win.viewport.scene.animation.enabled
    win.viewport.camera.set_view("iso")
    choose(win, 0)
    assert win.scene_player.is_active()             # it glides


def test_turning_them_off_keeps_the_times_and_stops_a_glide(win):
    _scenes(win, ("Planta", "top", set()))
    _visible(win)
    win.viewport.scene.animation = SceneAnimation(transition=4.0, pause=0.5)
    win.viewport.camera.set_view("iso")
    choose(win, 0)
    assert win.scene_player.is_active()
    win.set_scene_transitions(False)
    assert not win.scene_player.is_active()
    assert win.viewport.scene.animation == SceneAnimation(
        enabled=False, transition=4.0, pause=0.5)


def test_the_toggle_shows_the_choice_of_the_project_in_front(win):
    first = win._doc
    win.run_command("scene_transitions")
    assert not win.viewport.scene.animation.enabled
    win.new_document()
    assert win.viewport.scene.animation.enabled
    win.activate_document(first)
    assert not win.viewport.scene.animation.enabled


# ---- The video ---------------------------------------------------------------
def test_a_video_shows_the_first_scene_glides_and_ends_on_the_last():
    from ingetrazo_core import video_at, video_frame_count, video_seconds
    poses = [_pose(x=0), _pose(x=10), _pose(x=20)]
    assert video_seconds(3, 2.0, 1.0) == pytest.approx(7.0)   # 1 + 2·(2+1)
    assert video_frame_count(3, 2.0, 1.0, 10) == 71
    assert video_at(poses, 2.0, 1.0, 0.0) == (poses[0], 0)
    assert video_at(poses, 2.0, 1.0, 0.9) == (poses[0], 0)
    pose, shown = video_at(poses, 2.0, 1.0, 2.0)      # halfway to the second
    assert pose.target[0] == pytest.approx(5.0) and shown == 0
    assert video_at(poses, 2.0, 1.0, 3.5) == (poses[1], 1)
    assert video_at(poses, 2.0, 1.0, 7.0) == (poses[2], 2)
    assert video_at(poses, 2.0, 1.0, 99.0) == (poses[2], 2)   # no round
    assert video_frame_count(0, 2.0, 1.0, 30) == 0
    assert video_frame_count(1, 2.0, 0.0, 30) == 1
    with pytest.raises(ValueError):
        video_at([], 1.0, 1.0, 0.0)


def test_the_mkv_writer_writes_a_matroska_file(tmp_path):
    from ingetrazo_core import MkvWriter
    path = tmp_path / "prueba.mkv"
    writer = MkvWriter(path, 65, 48, 10)
    assert (writer.width, writer.height) == (64, 48)
    for i in range(12):
        writer.write(bytes((i * 20, 128, 128, 255)) * (64 * 48))
    writer.close()
    assert writer.frames == 12
    data = path.read_bytes()
    assert data[:4] == b"\x1a\x45\xdf\xa3"            # Matroska (EBML)
    assert len(data) > 1000


def test_a_discarded_video_leaves_no_file(tmp_path):
    from ingetrazo_core import MkvWriter
    path = tmp_path / "a-medias.mkv"
    writer = MkvWriter(path, 64, 48, 10)
    writer.write(bytes(64 * 48 * 4))
    writer.discard()
    assert not path.exists()


def test_cancel_after_first_frame_keeps_existing_destination(tmp_path):
    from ingetrazo_core import MkvWriter
    path = tmp_path / "existing.mkv"
    path.write_bytes(b"previous export")
    writer = MkvWriter(path, 64, 48, 10)
    writer.write(bytes(64 * 48 * 4))
    assert path.read_bytes() == b"previous export"
    writer.discard()
    assert path.read_bytes() == b"previous export"
    assert list(tmp_path.iterdir()) == [path]


def _no_qt(monkeypatch):
    """Any import of Qt fails: the writer must not reach for its event loop
    or its version probe."""
    import sys
    for name in ("PySide6", "PySide6.QtCore", "PySide6.QtGui", "PySide6.QtWidgets"):
        monkeypatch.setitem(sys.modules, name, None)


def test_finishing_a_video_does_not_pump_the_ui_thread(tmp_path, monkeypatch):
    from ingetrazo_core import MkvWriter
    _no_qt(monkeypatch)
    writer = MkvWriter(tmp_path / "complete.mkv", 2, 2, 24)
    writer.write(bytes(16))
    writer.close()


def test_writer_finalization_needs_no_qt_version_probe(tmp_path, monkeypatch):
    from ingetrazo_core import MkvWriter
    _no_qt(monkeypatch)
    writer = MkvWriter(tmp_path / "one.mkv", 2, 2, 30)
    writer.write(bytes(16))
    writer.close()
    assert writer.frames == 1


def _fake_render(win, log):
    """Supply a synthetic RGBA frame through the display's readback API."""
    def render(width, height=None, overlays=True, annotations_only=False):
        cam = win.viewport.camera
        log.append((round(cam.pitch, 3), cam.perspective, cam.aspect,
                    tuple(ly.visible for ly in win.viewport.scene.layers)))
        return width, height, bytes((128, 128, 128, 255)) * (width * height)
    win.view.render_pixels = render


def test_export_video_renders_every_frame_and_puts_the_view_back(
        win, tmp_path):
    from ingetrazo_render import export_video
    _with_layer(win, "Muebles")
    _scenes(win, ("Vista", "iso", set()), ("Planta", "top", {"Muebles"}))
    win.viewport.scene.animation = SceneAnimation(transition=1.0, pause=0.5)
    cam = win.viewport.camera
    cam.set_view("front")
    before = Pose.of_camera(cam), cam.aspect
    layers = [ly.visible for ly in win.viewport.scene.layers]
    log = []
    _fake_render(win, log)
    seen = []
    path = tmp_path / "casa.mkv"
    frames = export_video(win, path, 320, 180, 10,
                          lambda d, t: seen.append((d, t)))
    # 0.5 + (1 + 0.5) = 2 s at 10 fps, and the last frame: 21.
    assert frames == 21 and seen[-1] == (21, 21) and len(log) == 21
    assert path.read_bytes()[:4] == b"\x1a\x45\xdf\xa3"
    assert all(abs(aspect - 320 / 180) < 1e-9 for _p, _q, aspect, _l in log)
    muebles = [ly.name for ly in win.viewport.scene.layers].index("Muebles")
    assert log[0][3][muebles] is True              # the first scene's layers
    assert log[-1][3][muebles] is False            # the plan's, on arrival
    assert log[10][3][muebles] is True             # not while gliding
    # The view is as it was: camera, aspect and layers.
    assert Pose.of_camera(cam) == before[0] and cam.aspect == before[1]
    assert [ly.visible for ly in win.viewport.scene.layers] == layers


def test_cancelling_the_video_removes_the_file(win, tmp_path):
    from ingetrazo_render import export_video
    _scenes(win, ("A", "iso", set()), ("B", "top", set()))
    _fake_render(win, [])
    path = tmp_path / "cancelado.mkv"
    assert export_video(win, path, 160, 90, 10, lambda d, t: d < 3) == 0
    assert not path.exists()


def test_no_scenes_no_video(win, tmp_path):
    from ingetrazo_render import VideoError, export_video
    with pytest.raises(VideoError):
        export_video(win, tmp_path / "nada.mkv", 160, 90, 10)
    assert not (tmp_path / "nada.mkv").exists()


def test_the_video_options_offer_sizes_fps_and_the_length(win):
    from ingetrazo_render import OperationForm
    from ingetrazo_core import video_seconds
    model = OperationForm.video(1001, 601, 3, video_seconds(3, 2., 1.))
    assert model.values() == dict(width=1920, height=1080, fps=30)
    model.receive("size", "4")
    assert (model.values()["width"], model.values()["height"]) == (1000, 600)
    assert "7.0" in model.rows()[2][3] and "211 frames" in model.rows()[2][3]
    model.receive("fps", "0")
    assert "169 frames" in model.rows()[2][3]


def test_invalid_transition_can_be_disabled_without_losing_saved_time():
    from ingetrazo_render import OperationForm
    model = OperationForm.animation(True, 2.345, 0.075)
    model.receive("transition", "-")
    assert model.invalid_range() == (0, 60)
    model.receive("enabled", "0")
    assert model.invalid_range() is None
    assert model.values() == dict(enabled=False, transition=2.345, pause=0.075)


def test_the_video_is_in_file_export_and_in_the_animation_menu():
    from ingetrazo_render import command_registry
    paths = {menu_id: path for _, path, _, _, _, menu_id in command_registry([], [])}
    assert paths["export:video"] == "File ▸ Export"
    assert paths["animation_video"] == "Camera ▸ Animation"


def _without_multimedia(monkeypatch):
    """What the CI's Linux machine does: the import of Qt Multimedia
    fails (there, on libpulse.so.0)."""
    import sys
    monkeypatch.setitem(sys.modules, "PySide6.QtMultimedia", None)


def test_without_qt_multimedia_the_writer_still_encodes(monkeypatch, tmp_path):
    from ingetrazo_core import MkvWriter
    _without_multimedia(monkeypatch)
    path = tmp_path / "video.mkv"
    writer = MkvWriter(path, 64, 48, 10)
    writer.write(bytes(64 * 48 * 4))
    writer.close()
    assert path.read_bytes().startswith(b"\x1a\x45\xdf\xa3")


def test_without_qt_multimedia_export_still_offers_a_destination(win, monkeypatch):
    from ingetrazo_render.shell_files import ShellFileDialog
    _scenes(win, ("A", "iso", set()))
    _without_multimedia(monkeypatch)
    asked = []
    monkeypatch.setattr(ShellFileDialog, "getSaveFileName", lambda *a, **k: asked.append(a) or ("", ""))
    win.export_video()
    assert len(asked) == 1


@pytest.mark.parametrize("suffix,codec", [(".mkv", "mjpeg"), (".mp4", "h264")])
def test_independent_decoder_reads_every_frame_at_requested_rate(tmp_path, suffix, codec):
    import json
    import shutil
    import subprocess
    from ingetrazo_core import MkvWriter, Mp4Writer, video_format
    if suffix == ".mp4" and video_format()[0] != suffix:
        pytest.skip("system H.264 encoder unavailable")
    probe = shutil.which("ffprobe")
    if probe is None:
        pytest.skip("optional independent video decoder")
    path = tmp_path / ("short" + suffix)
    writer = (Mp4Writer if suffix == ".mp4" else MkvWriter)(path, 64, 32, 24)
    for i in range(26):
        writer.write(bytes((i * 8, 16, 80, 255)) * (64 * 16)
                     + bytes((240, 24, 32, 255)) * (64 * 16))
    writer.close()
    data = subprocess.check_output([probe, "-v", "error", "-count_frames", "-show_streams",
                                    "-show_format", "-of", "json", str(path)])
    info = json.loads(data)
    video, = info["streams"]
    assert (video["codec_name"], video["width"], video["height"]) == (codec, 64, 32)
    assert video["nb_read_frames"] == "26" and video["avg_frame_rate"] == "24/1"
    assert float(info["format"]["duration"]) == pytest.approx(26 / 24, abs=.001)
    decoder = shutil.which("ffmpeg")
    if decoder:
        frame = subprocess.check_output([decoder, "-v", "error", "-i", str(path),
                                         "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        top, bottom = frame[:3], frame[-3:]
        assert top[2] > top[0] + 45 and bottom[0] > bottom[2] + 140


def test_mp4_discard_preserves_existing_destination(tmp_path):
    from ingetrazo_core import Mp4Writer, video_format
    if video_format()[0] != ".mp4":
        pytest.skip("system H.264 encoder unavailable")
    path = tmp_path / "existing.mp4"
    path.write_bytes(b"previous movie")
    writer = Mp4Writer(path, 65, 33, 24)
    assert (writer.width, writer.height) == (64, 32)
    writer.write(bytes(64 * 32 * 4))
    with pytest.raises(ValueError):
        writer.write(b"short")
    writer.discard()
    assert path.read_bytes() == b"previous movie" and list(tmp_path.iterdir()) == [path]


@pytest.mark.parametrize("suffix,label", [(".mp4", "H.264 / MP4 (*.mp4)"),
                                          (".mkv", "Motion JPEG / Matroska (*.mkv)")])
def test_video_picker_and_sheet_agree_on_encoder_format(win, monkeypatch, tmp_path, suffix, label):
    from ingetrazo_render import chrome_forms
    from ingetrazo_render.shell_files import ShellFileDialog
    _scenes(win, ("A", "iso", set()))
    monkeypatch.setattr("ingetrazo_core.video_format", lambda: (suffix, label))
    picker = []
    monkeypatch.setattr(ShellFileDialog, "getSaveFileName", lambda *args: picker.append(args)
                        or (str(tmp_path / ("tour" + suffix)), ""))
    forms = []
    monkeypatch.setattr(chrome_forms, "show", lambda w, form: forms.append(form))
    win.export_video()
    assert picker[0][2].endswith(suffix) and picker[0][3] == label
    assert label in forms[0].wire()[2]


def test_normalizing_the_movie_suffix_preserves_an_unconfirmed_file(win, monkeypatch, tmp_path):
    from ingetrazo_render import chrome_dialogs
    from ingetrazo_render import chrome_forms
    from ingetrazo_render.shell_files import ShellFileDialog
    _scenes(win, ("A", "iso", set()))
    existing = tmp_path / "site.mp4"
    existing.write_bytes(b"original movie")
    monkeypatch.setattr("ingetrazo_core.video_format", lambda: (".mp4", "H.264 / MP4 (*.mp4)"))
    monkeypatch.setattr(ShellFileDialog, "getSaveFileName", lambda *args: (str(tmp_path / "site"), ""))
    prompts = []
    monkeypatch.setattr(chrome_dialogs, "ask", lambda *args, **kwargs: prompts.append(args) or "cancel")
    monkeypatch.setattr(chrome_forms, "show", lambda *args: pytest.fail("Export advanced after cancellation"))
    win.export_video()
    assert prompts and existing.name in prompts[0][2]
    assert existing.read_bytes() == b"original movie"


def test_playback_and_transitions_follow_the_front_document_in_the_native_menu(tmp_path):
    import json
    from pathlib import Path
    import sys

    from scripts.isolated_launch import run

    root = Path(__file__).resolve().parents[1]
    binary = root / "rust/built_app/target/release/built"
    if sys.platform != "darwin" or not binary.is_file():
        pytest.skip("requires the macOS release menu")
    profile = tmp_path / "profile"
    probe = profile / "data/ingetrazo/plugins/playback_probe.py"
    probe.parent.mkdir(parents=True)
    probe.write_text("""import json
from pathlib import Path

def setup(app):
    def record():
        items = app.window._menu_items
        checks = [items[key].isChecked() for key in ("scene_play", "scene_transitions")]
        with Path(__file__).with_suffix(".jsonl").open("a") as output:
            print(json.dumps(checks), file=output)
    app.add_menu_action("Read animation menu", record)
""")
    steps = tmp_path / "steps.txt"
    commands = ["menu scene_add", "menu scene_add", "menu extension:playback_probe:0",
                "menu scene_play", "menu extension:playback_probe:0", "key escape",
                "menu extension:playback_probe:0", "menu scene_transitions",
                "menu extension:playback_probe:0", "state", "quit"]
    steps.write_text("wait 500\n" + "\nwait 120\n".join(commands) + "\n")
    result = run([binary, "--root", root, "--venv", root / ".venv", "--new-instance",
                  "--script", steps, root / "tests/fixtures/shell/slab-and-cube.igz"],
                 settings=profile, capture_output=True, text=True, timeout=60, check=True)
    assert [json.loads(line) for line in probe.with_suffix(".jsonl").read_text().splitlines()] == [
        [False, True], [True, True], [False, True], [False, False]]
    state = next(json.loads(s.removeprefix("[built] state ")) for s in result.stdout.splitlines()
                 if s.startswith("[built] state "))
    assert state["errors"] == 0

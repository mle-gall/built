# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""First Person (tools/walkthrough.py) — a walkthrough mode that plays like
a game, next to the classic Walk and not instead of it: W/A/S/D move, Q/E go
down/up, Shift runs, Alt goes through walls, and a mouse drag (right button,
or left) turns the head.
"""
from __future__ import annotations

import pytest
from core.input_types import QPoint, Qt
from ingetrazo_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from tools import walkthrough as wt
from tools.walkthrough import FirstPersonTool, WalkTool


@pytest.fixture(autouse=True)
def _default_eye_height(monkeypatch):
    monkeypatch.setattr(wt, "eye_height", lambda: 1.68)
    monkeypatch.setattr(wt, "set_eye_height", lambda v: None)


def _room(vp):
    """A 6 × 6 m room, walls 3 m tall, on the ground."""
    mesh = vp.scene.mesh
    floor = [QVector3D(0, 0, 0), QVector3D(6, 0, 0), QVector3D(6, 6, 0),
             QVector3D(0, 6, 0)]
    mesh.add_face(floor)
    for a, b in zip(floor, floor[1:] + floor[:1]):
        mesh.add_face([a, b, QVector3D(b.x(), b.y(), 3.0),
                       QVector3D(a.x(), a.y(), 3.0)])
    vp.scene.version += 1
    vp.update()


@pytest.fixture
def win(native_host):
    w = native_host.window
    _room(w.viewport)
    # Standing in the middle of the room, looking north (+Y).
    w.viewport.camera.perspective = True
    w.viewport.camera.look_from(QVector3D(3, 3, 1.68), QVector3D(0, 1, 0))
    return w


def _tool(win) -> FirstPersonTool:
    win._activate_tool("first_person")
    tool = win.viewport.active_tool
    assert isinstance(tool, FirstPersonTool)
    return tool


def _walk(tool, vp, keys, seconds=0.5, mods=Qt.NoModifier):
    """Move for ``seconds`` with ``keys`` held; the eye's displacement."""
    before = vp.camera.eye()
    dt = FirstPersonTool.TICK_MS / 1000.0
    for _ in range(int(round(seconds / dt))):
        tool.step(vp, set(keys), dt, mods)
    return vp.camera.eye() - before


# ---- Moving ---------------------------------------------------------------------

def test_w_walks_forward_and_s_back(win):
    vp = win.viewport
    tool = _tool(win)
    d = _walk(tool, vp, {Qt.Key_W})
    assert d.y() > 0.5 and abs(d.x()) < 1e-4 and abs(d.z()) < 1e-4
    d = _walk(tool, vp, {Qt.Key_S})
    assert d.y() < -0.5 and abs(d.x()) < 1e-4


def test_a_and_d_step_sideways_without_turning(win):
    vp = win.viewport
    tool = _tool(win)
    heading = vp.camera.forward()
    d = _walk(tool, vp, {Qt.Key_D})
    assert d.x() > 0.5 and abs(d.y()) < 1e-4          # facing +Y, right is +X
    d = _walk(tool, vp, {Qt.Key_A})
    assert d.x() < -0.5 and abs(d.y()) < 1e-4
    assert (vp.camera.forward() - heading).length() < 1e-6


def test_diagonal_is_no_faster_than_straight(win):
    vp = win.viewport
    tool = _tool(win)
    straight = _walk(tool, vp, {Qt.Key_W}, 0.3).length()
    vp.camera.look_from(QVector3D(3, 3, 1.68), QVector3D(0, 1, 0))
    diagonal = _walk(tool, vp, {Qt.Key_W, Qt.Key_D}, 0.3)
    assert abs(diagonal.length() - straight) < 1e-4
    assert diagonal.x() > 0 and diagonal.y() > 0


def test_shift_runs(win):
    vp = win.viewport
    tool = _tool(win)
    walk = _walk(tool, vp, {Qt.Key_W}, 0.3).length()
    vp.camera.look_from(QVector3D(3, 3, 1.68), QVector3D(0, 1, 0))
    run = _walk(tool, vp, {Qt.Key_W}, 0.3, Qt.ShiftModifier).length()
    assert run == pytest.approx(walk * FirstPersonTool.RUN_FACTOR, rel=1e-3)


def test_e_goes_up_and_q_down(win):
    vp = win.viewport
    tool = _tool(win)
    d = _walk(tool, vp, {Qt.Key_E})
    assert d.z() > 0.3 and abs(d.x()) < 1e-4 and abs(d.y()) < 1e-4
    d = _walk(tool, vp, {Qt.Key_Q}, 0.2)
    assert d.z() < -0.1


def test_walls_stop_you_and_alt_goes_through(win):
    vp = win.viewport
    tool = _tool(win)
    _walk(tool, vp, {Qt.Key_W}, 10.0)                   # 15 m at 1.5 m/s
    y = vp.camera.eye().y()
    assert 5.0 < y < 6.0                                # stopped at the north wall
    _walk(tool, vp, {Qt.Key_W}, 2.0, Qt.AltModifier)
    assert vp.camera.eye().y() > 6.0


def test_walking_keeps_the_eye_height_over_the_floor(win):
    vp = win.viewport
    tool = _tool(win)
    _walk(tool, vp, {Qt.Key_E}, 0.5)                    # fly up…
    assert vp.camera.eye().z() > 2.0
    _walk(tool, vp, {Qt.Key_W}, 0.1)                    # …and walking lands you
    assert vp.camera.eye().z() == pytest.approx(1.68, abs=1e-4)


# ---- Keys through the viewport ----------------------------------------------------

def _press(win, key, pressed=True, modifiers=0):
    win._host.key(pressed, key, modifiers, chr(key).lower())


def test_wasd_move_you_instead_of_switching_tools(win):
    tool = _tool(win)
    for key in (Qt.Key_W, Qt.Key_A, Qt.Key_S, Qt.Key_D, Qt.Key_Q, Qt.Key_E):
        _press(win, key)
        assert win.viewport.active_tool is tool
        assert key in tool.held
        _press(win, key, False)
        assert key not in tool.held
    assert win.viewport.active_tool is tool
    assert not tool.moving


def test_holding_a_key_runs_the_timer(win):
    tool = _tool(win)
    _press(win, Qt.Key_W)
    assert tool.moving and tool._timer.isActive()
    _press(win, Qt.Key_W, False)
    assert not tool._timer.isActive()


def test_the_letters_still_switch_tools_from_walk(win):
    win._activate_tool("walk")
    assert isinstance(win.viewport.active_tool, WalkTool)
    _press(win, Qt.Key_W)
    assert win.viewport.active_tool is win._tools["followme"]


def test_ctrl_shortcuts_still_work_in_first_person(win):
    """Ctrl+letter is the window's (Ctrl+S saves, Ctrl+Q quits): only the
    bare letters are claimed. Ctrl+A — Select All — stands in for them here
    because it opens no dialog."""
    tool = _tool(win)
    _press(win, Qt.Key_A, modifiers=Qt.ControlModifier.value)
    assert Qt.Key_A not in tool.held
    assert set(win.viewport.scene.mesh.faces).issubset(win.viewport.scene.selection)


def test_losing_focus_stops_you(win):
    tool = _tool(win)
    _press(win, Qt.Key_W)
    assert tool.moving
    win._host.focus(False)
    assert not tool.moving and not tool._timer.isActive()


def test_leaving_the_tool_stops_you(win):
    tool = _tool(win)
    _press(win, Qt.Key_W)
    win._activate_tool("select")
    assert not tool.moving and not tool._timer.isActive()


# ---- Looking ----------------------------------------------------------------------

@pytest.mark.parametrize("button", [Qt.RightButton, Qt.LeftButton])
def test_a_drag_turns_the_head_and_keeps_the_eye(win, button):
    vp = win.viewport
    _tool(win)
    eye = vp.camera.eye()
    btn = button.value
    host = win._host
    host.pointer("press", 200, 300, btn, btn)
    host.pointer("move", 500, 300, held=btn)
    f = vp.camera.forward()
    assert f.x() > 0.1
    host.pointer("move", 500, 100, held=btn)
    assert vp.camera.forward().z() > f.z() + 0.1
    host.pointer("release", 500, 100, btn)
    assert (vp.camera.eye() - eye).length() < 1e-4
    assert vp._look_drag is None


def _turned_deg(vp) -> float:
    import math
    f = vp.camera.forward()
    return math.degrees(math.atan2(f.x(), f.y()))


def test_look_is_a_fixed_rate_per_pixel(win):
    vp = win.viewport
    tool = _tool(win)
    tool.on_look(vp, 100, 0)
    assert _turned_deg(vp) == pytest.approx(100 * wt.look_deg_per_px(),
                                            abs=1e-3)


def test_the_default_sensitivity_is_gentle():
    """A third of the first cut's 0.15 °/px, which was far too quick:
    a 1200 px drag across the view turns 60°, not 180°."""
    assert wt.look_deg_per_px(wt.DEFAULT_LOOK_SENSITIVITY) == pytest.approx(0.05)


@pytest.mark.parametrize("sensitivity", [1, 25, 100])
def test_the_sensitivity_setting_scales_the_turn(win, monkeypatch,
                                                 sensitivity):
    monkeypatch.setattr(wt, "look_sensitivity", lambda: sensitivity)
    vp = win.viewport
    tool = _tool(win)
    tool.on_look(vp, 100, 0)
    assert _turned_deg(vp) == pytest.approx(100 * sensitivity / 500.0,
                                            abs=1e-3)


def test_a_stored_sensitivity_out_of_range_falls_back(monkeypatch):
    from core.settings import settings
    settings().setValue("walk/look_sensitivity", "9999")
    assert wt.look_sensitivity() == wt.DEFAULT_LOOK_SENSITIVITY


def test_right_click_opens_no_menu(win):
    tool = _tool(win)
    assert tool.context_menu(win.viewport, QPoint(0, 0)) is True


# ---- Where it lives ------------------------------------------------------------------

def test_first_person_is_available_with_the_other_walkthrough_commands(win):
    for name in ("position_camera", "walk", "look_around", "first_person"):
        win._host.menu("tool:" + name)
        assert win.viewport.active_tool is win._tools[name]
    assert win._tool_actions["first_person"].portable_keys() == []

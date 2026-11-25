# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Alt cycles the linear inferences DURING an operation, the classic way.

@pacaeiro, on 0.4.2 (issue #26): «I finally discovered that ALT key switch
off some inferences (witch is very good), but the problem is the ALT key
itself. In my daily job I use a lot the shortcuts ALT and ALT+TAB to switch
between programs, and every time I do that with Built I lost the
inferences.»

The key is right — Marco's reference screenshot reads «Alt =
Activar/desactivar "Inferencias lineales" (Ninguno activo)», the same three
states we cycle. What was wrong is WHEN: the convention offers it only after the
first click of a line, and we had it global, at any idle moment. So an
Alt+Tab with nothing in progress moved a mode he was not even using.

Gated on an operation being under way, his daily window switching cannot
touch it: idle, there is nothing to steal.

The first attempt cycled on the RELEASE of a clean tap instead, and he
tested it from main: «sometimes it triggers the command, some other do
not». Measured on Marco's desktop with an application-level probe — the
Alt release is what makes Qt's menu bar take focus, so it lands on the
QMenuBar and the focus ping-pongs. One tap in six reached the viewport.
Hence: act on the PRESS, and swallow the release so the menu bar never
takes it.
"""
from __future__ import annotations


from built_render import KeyEvent, MouseEvent
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core import Input as Qt
from built_core import Vec3 as QVector3D


def _viewport(native_host):
    win = native_host.window
    return win, win.viewport


def _press(vp, key, text=""):
    vp.keyPressEvent(KeyEvent(True, key, Qt.NoModifier, text))


def _release(vp, key, text=""):
    ev = KeyEvent(False, key, Qt.NoModifier, text)
    vp.keyReleaseEvent(ev)
    return ev


def _busy(vp):
    """Put the active tool mid-operation, the way the first click does."""
    vp.active_tool.start_point = QVector3D(0.0, 0.0, 0.0)
    assert vp._tool_busy(vp.active_tool)


def test_alt_does_nothing_when_no_operation_is_under_way(native_host):
    """The whole of issue #26: his Alt+Tab happens between operations."""
    win, vp = _viewport(native_host)
    assert vp.linear_inference_mode == "all"
    _press(vp, Qt.Key_Alt)
    _release(vp, Qt.Key_Alt)
    assert vp.linear_inference_mode == "all"


def test_alt_cycles_while_a_line_is_being_drawn(native_host):
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    _busy(vp)
    for expected in ("off", "parallel_perp", "all"):
        _press(vp, Qt.Key_Alt)
        _release(vp, Qt.Key_Alt)
        assert vp.linear_inference_mode == expected


def test_it_acts_on_the_press_so_the_menu_bar_cannot_steal_it(native_host):
    """Qt's menu bar takes focus on the Alt RELEASE. Acting on the press is
    what makes the toggle reach us at all."""
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    _busy(vp)
    _press(vp, Qt.Key_Alt)
    assert vp.linear_inference_mode == "off"      # already done


def test_the_release_is_swallowed_so_the_menu_bar_never_sees_it(native_host):
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    _busy(vp)
    _press(vp, Qt.Key_Alt)
    ev = _release(vp, Qt.Key_Alt)
    assert ev.isAccepted()


def test_an_idle_alt_release_is_left_alone_for_the_menus(native_host):
    """With nothing in progress the key is not ours: Alt must keep opening
    the menu bar like it does in every other program."""
    win, vp = _viewport(native_host)
    _press(vp, Qt.Key_Alt)
    assert vp._alt_tap is False


def test_losing_focus_drops_the_claim_on_the_release(native_host):
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    _busy(vp)
    _press(vp, Qt.Key_Alt)
    vp.focusOutEvent(None)
    assert vp._alt_tap is False


def test_a_click_mid_operation_does_not_give_the_inferences_back(native_host):
    """The second click of a line is not a new gesture."""

    win, vp = _viewport(native_host)
    win._activate_tool("line")
    _busy(vp)
    _press(vp, Qt.Key_Alt)
    vp.mousePressEvent(MouseEvent('press', 5, 5, Qt.LeftButton, Qt.LeftButton, Qt.NoModifier))
    assert vp.linear_inference_mode == "off"


def test_changing_tool_gives_the_inferences_back(native_host):
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    _busy(vp)
    _press(vp, Qt.Key_Alt)
    assert vp.linear_inference_mode == "off"
    win._activate_tool("select")
    assert vp.linear_inference_mode == "all"


def test_escape_gives_the_inferences_back(native_host):
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    _busy(vp)
    _press(vp, Qt.Key_Alt)
    vp.escape()
    assert vp.linear_inference_mode == "all"


def test_the_status_bar_offers_alt_and_names_the_state(native_host):
    """The usual status bar puts the offer AND the current state on one line
    while drawing: «Alt = Activar/desactivar "Inferencias lineales"
    (Ninguno activo)»."""
    from built_render.status_hints import alt_inference_hint, hint_for

    # Shorter than the usual wording on purpose: it has the whole bar
    # for this line and we have half of it, so the full name «linear
    # inferences (none active)» spent 52 of a ~110-character budget and got
    # the end of the hint elided on a 1366 screen. The long name still runs
    # on the canvas, where it competes with nothing.
    assert "all" in alt_inference_hint("all")
    assert "none" in alt_inference_hint("off")
    assert "parallel" in alt_inference_hint("parallel_perp")

    win, vp = _viewport(native_host)
    win._activate_tool("line")
    key = win._tool_key(vp.active_tool)
    idle = hint_for(key, vp.active_tool, None, "all")
    _busy(vp)
    drawing = hint_for(key, vp.active_tool, None, "off")
    assert "Alt" not in idle, "nothing to offer before the first click"
    assert "Alt" in drawing and "none" in drawing

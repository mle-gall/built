# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""«x5» right after a copy makes the array, like «5x» (#542).

The X only entered the value box after a digit, because bare X is the Text
shortcut (#20) — and the window's shortcut took it before the viewport saw
the key. Right after a Move or Rotate copy, while «3x» / «/3» would lay an
array, a bare X now opens the value box; anywhere else it is still Text.
Keys go through the native host and its shortcut map.
"""
from __future__ import annotations


from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import ScreenPoint as QPointF, Input as Qt
from ingetrazo_core import Vec3 as V

from tools.base import ToolContext  # noqa: E402


def _window(native_host):
    win = native_host.window
    vp = win.viewport
    face = vp.scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0),
                                   V(0, 1, 0)])
    vp.scene.version += 1
    vp.setFocus()
    return win, vp, face


def _ctx(vp, x, y):
    return ToolContext(viewport=vp, world=V(x, y, 0), screen=QPointF(0, 0),
                       modifiers=Qt.NoModifier, snap=None)


def _key(vp, key, text=None):
    if text is None:
        text = chr(key) if key < 128 else ""
    vp._host.key(True, int(key), 0, text)
    vp._host.key(False, int(key), 0, text)


def _xs(vp):
    return sorted(round(min(p.x() for p in f.vertices), 6)
                  for f in vp.scene.mesh.faces)


def _move_copy(win, vp, face):
    vp.scene.selection = [face]
    win._activate_tool("move")
    tool = vp.active_tool
    tool.on_click(_ctx(vp, 0, 0))
    tool.on_key(vp, Qt.Key_Control, Qt.NoModifier)
    tool.on_click(_ctx(vp, 2, 0))                       # one copy at +2
    assert _xs(vp) == [0.0, 2.0]
    return tool


def _type(vp, text):
    for char in text:
        _key(vp, ord(char.upper()), char)
    _key(vp, Qt.Key_Return)


def test_x5_after_a_move_copy_lays_five_copies(native_host):
    win, vp, face = _window(native_host)
    tool = _move_copy(win, vp, face)
    _type(vp, "x5")
    assert vp.active_tool is tool                   # not the Text tool
    assert _xs(vp) == [0.0, 2.0, 4.0, 6.0, 8.0, 10.0]


def test_5x_still_works(native_host):
    win, vp, face = _window(native_host)
    tool = _move_copy(win, vp, face)
    _type(vp, "5x")
    assert vp.active_tool is tool
    assert _xs(vp) == [0.0, 2.0, 4.0, 6.0, 8.0, 10.0]


def test_bare_x_without_a_copy_is_still_the_text_tool(native_host):
    win, vp, face = _window(native_host)
    win._activate_tool("move")
    move = vp.active_tool
    _key(vp, Qt.Key_X)
    assert vp.active_tool is not move
    assert type(vp.active_tool).__name__ == "TextTool"


def test_once_the_array_window_closes_x_is_text_again(native_host):
    win, vp, face = _window(native_host)
    tool = _move_copy(win, vp, face)
    tool.on_click(_ctx(vp, 5, 5))                   # a click ends the window
    assert not tool.array_pending(vp)
    _key(vp, Qt.Key_X)
    assert type(vp.active_tool).__name__ == "TextTool"


def test_esc_right_after_a_copy_closes_the_array_window(native_host):
    """The Esc cascade gains one step, only right after a copy: the first
    Esc with an empty value box closes the array window — X is Text again —
    and the selection is left alone for the next Esc."""
    win, vp, face = _window(native_host)
    tool = _move_copy(win, vp, face)
    assert tool.array_pending(vp)
    _key(vp, Qt.Key_Escape)
    assert not tool.array_pending(vp)
    assert vp.scene.selection                       # still selected
    _key(vp, Qt.Key_X)
    assert type(vp.active_tool).__name__ == "TextTool"


def test_x_then_esc_esc_then_x_is_text(native_host):
    """A stray X: the first Esc empties the value box, the second closes
    the array window, and the next X is the Text tool."""
    win, vp, face = _window(native_host)
    _move_copy(win, vp, face)
    for key in (Qt.Key_X, Qt.Key_Escape, Qt.Key_Escape, Qt.Key_X):
        _key(vp, key)
    assert type(vp.active_tool).__name__ == "TextTool"
    assert _xs(vp) == [0.0, 2.0]                    # the copy stays

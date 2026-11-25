# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #191 (fafecm, Windows): «after the push/pull function cursor is
used once, the cursor symbol disappears upon its second and subsequent
use» — the Line too. Windows brought the arrow back over the native OpenGL
viewport when an operation ended, while Qt still believed the tool's
cursor was set. After every operation the viewport now unsets and sets the
tool's cursor again, and does the same when the pointer comes back in."""
from __future__ import annotations

from built_render import MouseEvent
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401


import pytest  # noqa: E402
from built_core import Input as Qt


def _spy(vp, monkeypatch):
    calls = []
    real_unset = vp.unsetCursor
    monkeypatch.setattr(vp, "unsetCursor",
                        lambda: (calls.append("unset"), real_unset()))
    return calls


def _click(vp):
    return MouseEvent('press', 50, 50, Qt.LeftButton, Qt.LeftButton, Qt.NoModifier)


def test_an_operation_puts_the_tool_cursor_back(native_window, monkeypatch):
    vp = native_window.viewport
    native_window._activate_tool("pushpull")
    tool = vp.active_tool
    from built_core.history import AddEdgeCommand
    from built_core import Vec3 as V
    monkeypatch.setattr(tool, "on_click", lambda ctx: vp.history.execute(
        AddEdgeCommand(V(0, 0, 0), V(1, 0, 0))))
    calls = _spy(vp, monkeypatch)
    vp._dispatch_tool_click(_click(vp))
    assert calls == ["unset"]                         # re-asserted…
    assert vp.cursor() == "tool:pushpull"     # …as the tool's icon


def test_a_click_that_makes_nothing_leaves_the_cursor_alone(native_window,
                                                            monkeypatch):
    vp = native_window.viewport
    native_window._activate_tool("line")
    monkeypatch.setattr(vp.active_tool, "on_click", lambda ctx: None)
    calls = _spy(vp, monkeypatch)
    vp._dispatch_tool_click(_click(vp))
    assert calls == []


def test_nothing_is_forced_while_a_camera_mode_owns_the_pointer(native_window,
                                                                monkeypatch):
    vp = native_window.viewport
    vp.set_nav_mode("orbit")
    calls = _spy(vp, monkeypatch)
    vp._reassert_tool_cursor()
    assert calls == []

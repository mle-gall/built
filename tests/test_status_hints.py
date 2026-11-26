# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The status bar shows ONE hint for the tool and its step (Marco,
2026-09-15: a strip of every shortcut at once «no es bueno»)."""
from __future__ import annotations

import pytest

from built_core import Vec3 as QVector3D

from tools.arc import ArcTool
from tools.fillet import FilletTool
from tools.rust_tools import LineTool
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
from built_render.status_hints import HINTS, NAV_HINTS, hint_for, phase_of


def test_the_hint_follows_the_tools_step():
    line = LineTool()
    idle = hint_for("line", line)
    assert idle.startswith("Click the start point")
    line.start_point = QVector3D(0, 0, 0)
    assert hint_for("line", line).startswith("Click the end point")
    arc = ArcTool()
    assert phase_of("arc", arc) == "idle"
    arc.start_point = QVector3D(0, 0, 0)
    assert phase_of("arc", arc) == "end"
    arc.end_point = QVector3D(1, 0, 0)
    assert phase_of("arc", arc) == "bulge"
    assert "Ns" in hint_for("arc", arc)
    fil = FilletTool()
    assert "size" in hint_for("fillet", fil)
    assert "Ctrl = measure (Radius)" in hint_for("fillet", fil)
    fil.sizing = True
    assert phase_of("fillet", fil) == "sizing"
    assert "'0.3p' = profile" in hint_for("fillet", fil)


def test_navigation_and_unknown_tools_have_a_line_too():
    class Odd:
        name = "Odd"
    assert hint_for(None, Odd()).startswith("Odd")
    assert hint_for("line", LineTool(), nav_mode="orbit") == NAV_HINTS["orbit"]


def test_every_registered_tool_has_a_hint(native_window):
    win = native_window
    missing = [k for k in win._tools if k not in HINTS and not k.startswith("plugin_")]
    assert not missing, missing
    win._activate_tool("line")
    assert win.statusBar().currentMessage().startswith("Line — Click the start point")
    win._activate_nav("orbit")
    assert win.statusBar().currentMessage().startswith("Orbit — Drag to orbit")


@pytest.mark.parametrize("key, attr, expected", [
    ("rectangle", "start_point", "Click the opposite corner"),
    ("line", "start_point", "Click the end point"),
    ("offset", "dragging", "Move to set the offset"),
    ("pushpull", "dragging", "Move, or type the distance"),
    ("rotate", "start_point", "Click the start of the angle"),
])
def test_gesture_reaches_the_displayed_bar(native_window, monkeypatch, key, attr, expected):
    from unittest.mock import MagicMock
    from views import wgpu_ui
    win = native_window
    win._activate_tool(key)
    setattr(win.viewport.active_tool, attr, True if attr == "dragging" else QVector3D())
    renderer = MagicMock()
    monkeypatch.setattr(win.view, "renderer", renderer)
    wgpu_ui.attach(win.view)
    wgpu_ui.before_frame(win.view)
    assert expected in renderer.ui_set_status.call_args.args[1]
    win.view.tools._set_value_buffer("30")
    wgpu_ui.before_frame(win.view)
    assert "Enter to apply" in renderer.ui_set_status.call_args.args[1]


@pytest.mark.parametrize("source,target", [
    ("followme", "offset"), ("fillet", "outer_shell"),
    ("fillet", "tape"), ("fillet", "protractor"),
])
def test_tool_messages_end_on_switch(native_window, source, target):
    win = native_window
    win._activate_tool(source)
    win.viewport.flash_status("Previous tool message")
    win._activate_tool(target)
    assert HINTS[target]["idle"] in win.statusBar().currentMessage()


def test_bar_uses_menu_modifier_names_and_omits_chipped_options(native_window):
    from built_render import shortcut_text
    win = native_window
    win._activate_tool("select")
    assert shortcut_text("Ctrl+A")[1].removesuffix("A").rstrip("+") in win.statusBar().currentMessage()
    for key in ("rectangle", "pushpull"):
        win._activate_tool(key)
        assert "base" not in win.statusBar().currentMessage()
        assert "centre" not in win.statusBar().currentMessage()


def test_hosted_placement_hint_on_transient_tool():
    from types import SimpleNamespace
    tool = SimpleNamespace(_wall_family=True)
    assert hint_for(None, tool) == "Hover a wall · Tab flips side · enter sill;offset · click to place"


def test_sampling_material_uses_the_hint_and_keeps_notifications_quiet(native_window, monkeypatch):
    win = native_window
    notices = []
    monkeypatch.setattr(win, "notify", lambda *args: notices.append(args))
    try:
        win._on_toggle_eyedropper(True)
        assert "Click a face to sample its material" in win.statusBar().currentMessage()
        assert notices == []
    finally:
        win._on_toggle_eyedropper(False)

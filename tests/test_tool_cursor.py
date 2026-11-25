# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Tool cursors: activating a tool turns the mouse pointer
into the tool's icon; Offset has an arrow with a badge, Select keeps the arrow."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_render import tool_cursor_art, tool_cursor_name


def test_tool_cursor_builds_bitmap_with_hotspot():
    for name, hotspot in {"line": (4, 28), "eraser": (10, 21), "move": (16, 16),
                          "orbit": (16, 16), "zoom": (14, 14)}.items():
        rgba, hot = tool_cursor_art(name, 32)
        assert len(rgba) == 32 * 32 * 4 and any(rgba[3::4])
        assert tuple(round(v * 32 / 48) for v in hot) == hotspot
        assert tool_cursor_name(name, False, "tool") == f"tool:{name}"
    for name in ("select", "no_such_tool", None):
        assert tool_cursor_name(name, False, "tool") is None
    for name in ("pan", "zoom_window"):
        assert tool_cursor_art(name, 32)


def test_offset_keeps_an_arrow_tip_with_its_icon_beside_it():
    rgba, hot = tool_cursor_art("offset", 32)
    assert tuple(round(v * 32 / 48) for v in hot) == (2, 2)
    for x, y in ((3, 7), (18, 18)):
        assert rgba[4 * (32 * y + x) + 3] > 0


def test_activating_tools_swaps_the_viewport_cursor(native_host):
    win = native_host.window
    vp = win.viewport
    assert not str(vp.cursor()).startswith("tool:")   # select at startup
    win._activate_tool("line")
    assert vp.cursor() == "tool:line"
    win._activate_tool("pushpull")
    assert vp.cursor() == "tool:pushpull"
    win._activate_tool("select")
    assert not str(vp.cursor()).startswith("tool:")   # back to the arrow


def test_tool_cursor_respects_cursor_style_preference(native_host):
    from core import general_settings
    for style, expected in (("tool", "tool:line"), ("crosshair", "crosshair"), ("arrow", None)):
        general_settings.set_value("cursor_style", style)
        assert native_host.tool_cursor("line") == expected

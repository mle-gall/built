# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""North is reachable through the native menu, rail, keyboard and model settings."""
import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_settings_sheet import act, fields, frame, win  # noqa: F401


def test_the_tool_is_in_the_tools_menu_and_the_annotate_bar(native_host):
    from ingetrazo_render import command_registry, rail_layout
    from tools.north import NorthTool
    assert any(("north", "") in group for group in rail_layout())
    assert any(row[1] == "Tools" and row[5] == "tool:north"
               for row in command_registry([], []))
    native_host.menu("tool:north")
    assert isinstance(native_host.viewport.active_tool, NorthTool)


def test_shift_n_picks_up_the_tool(native_host):
    from core.input_types import Qt
    from tools.north import NorthTool
    native_host.key(True, ord("N"), Qt.ShiftModifier.value, "N")
    assert isinstance(native_host.viewport.active_tool, NorthTool)


def test_location_north_field_sets_and_follows(win):
    win.open_preferences(page="location")
    assert "north" in fields(frame(win))
    act(win, "v", "north", "-60")
    vp = win.viewport
    assert vp.scene.north_deg == pytest.approx(-60)
    assert vp.history.undo() and vp.scene.north_deg == 0
    win._tools["north"].commit(vp, 15.)
    assert fields(frame(win))["north"][4] == pytest.approx(15.)


def test_the_on_screen_button_picks_up_the_tool(win):
    win.open_preferences(page="location")
    assert "north_tool" in fields(frame(win))
    assert not act(win, "b", "north_tool")
    assert win.viewport.active_tool is win._tools["north"]

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""North is reachable through the native menu, rail, keyboard and model settings."""
import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_settings_sheet import act, fields, frame, win  # noqa: F401


def test_the_tool_is_in_the_tools_menu_and_the_annotate_bar(native_host):
    from built_render import command_registry, rail_layout
    from built_render.chrome_sheet import current
    assert any(("north", "") in group for group in rail_layout())
    assert any(row[1] == "Tools" and row[5] == "tool:north"
               for row in command_registry([], []))
    native_host.menu("tool:north")
    assert current(native_host.window).site.placing


def test_shift_n_picks_up_the_tool(native_host):
    from built_core import Input as Qt
    from built_render.chrome_sheet import current
    native_host.key(True, ord("N"), Qt.ShiftModifier.value, "N")
    assert current(native_host.window).site.placing


def test_location_north_field_sets_and_follows(win):
    win.open_preferences(page="location")
    assert "north" in fields(frame(win))
    act(win, "v", "north", "-60")
    vp = win.viewport
    assert vp.scene.north_deg == pytest.approx(-60)
    assert vp.history.undo() and vp.scene.north_deg == 0
    win._tools["north"].commit(vp, 15.)
    assert fields(frame(win))["north"][4] == pytest.approx(15.)


def test_location_link_opens_site_placement(win):
    win.open_preferences(page="location")
    assert "add_location" in fields(frame(win))
    assert act(win, "b", "add_location")["id"] == "site-placement"

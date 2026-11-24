# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Window ▸ Preferences… on Ctrl+, (Control + comma), the usual key for
Preferences, free everywhere else."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import chrome_sheet
from views.shortcuts import action_keys, collect_actions


def test_preferences_has_ctrl_comma(native_host):
    action = native_host.window._registry_actions["menu:prefs"]
    assert action.text()
    assert action_keys(action) == ["Ctrl+,"]


def test_nobody_else_has_ctrl_comma(native_host):
    win = native_host.window
    assert [a for a in collect_actions(win) if "Ctrl+," in action_keys(a)] == [
        win._registry_actions["menu:prefs"]]


def test_ctrl_comma_opens_preferences_on_its_first_page(native_host):
    win = native_host.window
    win._registry_actions["menu:prefs"].trigger()
    assert chrome_sheet.current(win).section == "general"

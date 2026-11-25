# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Alt+X switches X-ray on and back off.

A look through the faces is a glance: the key goes into X-ray and, pressed
again, returns to the style you were in, edge and profile settings included.
"""
from __future__ import annotations

from built_render import command_chrome, command_registry
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
from views import view_commands


def _state(win, key):
    cluster, menu = command_chrome(view_commands._snapshot(win), {}, {})
    items = [r for group in cluster for r in group] + [r for _, group in menu for r in group]
    return next(r[4] for r in items if r[0] == key)


def test_alt_x_is_the_toggle_and_no_other_action_holds_it():
    assert [r[0] for r in command_registry([], []) if "Alt+X" in r[3]] == ["view:xray"]


def test_the_toggle_goes_back_to_the_style_you_left(native_window):
    w = native_window
    assert view_commands.run(w, "style_shaded")
    w._set_style_field("profiles", True)
    assert view_commands.run(w, "xray") and _state(w, "xray") == 1
    assert w.active_style().face_mode == "xray"
    assert view_commands.run(w, "xray") and _state(w, "style_shaded") == 1
    style = w.active_style()
    assert (style.name, style.face_mode, style.profiles) == ("Shaded", "shaded", True)


def test_xray_picked_from_the_menu_toggles_out_to_default(native_window):
    assert view_commands.run(native_window, "style_xray")
    assert view_commands.run(native_window, "xray") and native_window.active_style().name == "Default"

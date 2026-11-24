# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #142 (@pacaeiro): take the keyboard shortcuts to another
computer — Preferences ▸ Keyboard shortcuts ▸ Export… / Import…."""
from __future__ import annotations

import pytest
from core.settings import settings
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import shortcuts as sc


@pytest.fixture
def ventana(native_host):
    return native_host.window


def _action(win, key):
    return next(a for a in sc.collect_actions(win) if sc.action_key(a) == key)


def _keys(act):
    return act.portable_keys()


def test_export_then_import_brings_the_keys_back(ventana):
    line = ventana._tool_actions["line"]
    key = sc.action_key(line)
    line.setShortcuts(["Ctrl+Alt+L"])
    sc.save_shortcut(line, line.portable_keys())
    data = sc.export_shortcuts(ventana)
    assert data["format"] == sc.EXPORT_FORMAT
    assert data["shortcuts"][key] == "Ctrl+Alt+L"
    # Another machine: factory keys.
    line.setShortcuts(sc.default_shortcuts(line))
    settings().remove("shortcuts")
    applied, skipped, _clashes = sc.import_shortcuts(ventana, data)
    assert applied > 0 and skipped == 0
    assert _keys(_action(ventana, key)) == ["Ctrl+Alt+L"]
    assert settings().value(f"shortcuts/{key}") == "Ctrl+Alt+L"  # remembered


def test_a_key_the_file_gives_away_is_taken_from_the_other_action(ventana):
    line = ventana._tool_actions["line"]
    rect = ventana._tool_actions["rectangle"]
    rect_keys = _keys(rect)
    assert rect_keys                                   # R, the factory key
    data = {"format": sc.EXPORT_FORMAT, "version": 1,
            "shortcuts": {sc.action_key(line): rect_keys[0]}}
    sc.import_shortcuts(ventana, data)
    assert _keys(line) == [rect_keys[0]]
    assert rect_keys[0] not in _keys(rect)             # never both: Qt kills both


def test_unknown_actions_and_reserved_keys_are_left_out(ventana):
    line = ventana._tool_actions["line"]
    data = {"format": sc.EXPORT_FORMAT, "version": 1,
            "shortcuts": {"a_plugin_not_installed": "Ctrl+Alt+9",
                          sc.action_key(line): "Esc"}}
    applied, skipped, _clashes = sc.import_shortcuts(ventana, data)
    assert (applied, skipped) == (1, 1)
    assert "Esc" not in _keys(line)                    # the tools own Esc


def test_a_foreign_file_is_refused(ventana):
    with pytest.raises(ValueError):
        sc.import_shortcuts(ventana, {"keys": {"L": "Line"}})
    with pytest.raises(ValueError):
        sc.import_shortcuts(ventana, ["not", "a", "file"])

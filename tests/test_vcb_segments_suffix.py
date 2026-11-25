# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Discussion #267 (tomlai-git, found by Arthur031221): «16s» sets the
sides of a circle / the segments of an arc, and the parser understood it,
but the keyboard never let the «s» into the value box."""
from __future__ import annotations

import pytest
from core.input_types import Qt
from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.mark.parametrize("tool,attr", [("circle", "sides"),
                                       ("arc", "segments")])
def test_typing_16s_sets_the_count(native_host, tool, attr):
    win = native_host.window
    win._activate_tool(tool)
    vp = win.viewport
    for ch in "16s":
        native_host.key(True, ord(ch.upper()), text=ch)
    native_host.key(True, Qt.Key_Return, text="\r")
    assert getattr(vp.active_tool, attr) == 16
    assert vp._value_buffer == ""


def test_a_bare_s_is_still_a_shortcut(native_host):
    win = native_host.window
    win._activate_tool("circle")
    vp = win.viewport
    native_host.key(True, Qt.Key_S, text="s")
    assert vp.active_tool is win._tools["scale"]
    assert vp._value_buffer == ""

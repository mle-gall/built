# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""@pacaeiro, #47 point 1: «Activate Paint tool. Press Alt and release it.
Icon changed but we're still on Paint mode.» The first Alt release hands
the keyboard to the menu bar, so the viewport never heard the next press:
every other tap the pointer stayed a bucket. Alt is now heard at
application level, wherever the key event lands."""
from __future__ import annotations

import pytest

from built_core import Input
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tools.paint import PaintTool


@pytest.fixture
def paint_host(native_host, monkeypatch):
    monkeypatch.setattr(PaintTool, "sample_armed", False)
    native_host.window._activate_tool("paint")
    return native_host


def alt(host, pressed):
    host.key(pressed, Input.Key_Alt, Input.AltModifier if pressed else Input.NoModifier)


def test_alt_taps_on_the_menu_bar_still_reach_the_viewport(paint_host):
    paint_host.pointer("move", 10, 5)
    vp = paint_host.window.viewport
    alt(paint_host, True)
    assert vp._alt_down
    alt(paint_host, False)
    assert not vp._alt_down


def test_a_tap_toggles_the_eyedropper_and_it_stays(paint_host):
    for armed in (True, False):
        alt(paint_host, True)
        alt(paint_host, False)
        assert PaintTool.sample_armed is armed


def test_alt_tab_is_not_a_tap(paint_host):
    alt(paint_host, True)
    paint_host.key(True, Input.Key_Tab, Input.AltModifier)
    alt(paint_host, False)
    assert not PaintTool.sample_armed

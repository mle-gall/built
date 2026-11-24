# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Coordinate and measurement changes appear at the next chrome frame."""
from types import SimpleNamespace
from unittest.mock import Mock

from ingetrazo_core import Vec3
from ingetrazo_render import sync_status_readout
from core.input_types import QPointF
from georef.datum import SceneDatum
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views.wgpu_ui import _cursor_value


def test_coordinate_and_measurement_texts_are_coalesced(native_host, monkeypatch):
    win = native_host.window
    view, vp = win.view, win.viewport
    renderer = Mock()
    monkeypatch.setattr(view, "renderer", renderer)
    vp.scene.georef = SceneDatum(47, 7)
    monkeypatch.setattr(vp, "_world_from_pixel", lambda x, y: Vec3(x, y, 0))
    win._activate_tool("line")
    view.tools.sync()
    for x in range(50):
        vp._emit_coordinate(SimpleNamespace(position=lambda: QPointF(x, 0)))
        view.tools._set_value_buffer(str(x))
    renderer.ui_set_status.assert_not_called()
    renderer.ui_set_cursor_value.assert_not_called()
    sync_status_readout(view, "Line", "")
    _cursor_value(view, win)
    east, north = vp.scene.georef.local_to_utm(Vec3(49, 0, 0))[:2]
    expected = f"E {east:.1f}  N {north:.1f}"
    renderer.ui_set_status.assert_called_once_with("Line", "", expected, vp._last_coordinate_full)
    assert vp.scene.georef.hemisphere in vp._last_coordinate_full
    assert renderer.ui_set_cursor_value.call_args.args[1:3] == ("49 m", True)
    sync_status_readout(view, "Line", "")
    assert renderer.ui_set_status.call_count == 1
    vp.scene.georef = None
    vp._emit_coordinate(SimpleNamespace())
    sync_status_readout(view, "Line", "")
    assert renderer.ui_set_status.call_args.args == ("Line", "", "", "")


def test_a_temporary_message_gives_the_tool_hint_back(native_host):
    win = native_host.window
    win._activate_tool("line")
    bar = win.statusBar()
    standing = bar.currentMessage()
    assert standing.startswith("Line")
    win.viewport.flash_status("Temporary message")
    assert bar.currentMessage() == "Temporary message"
    bar.clearMessage()
    assert bar.currentMessage() == standing

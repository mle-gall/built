# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #205: a render light is selected by clicking its bulb with the
Select tool and deleted with Supr, one undo step — through the extension
API's ``add_pickable``, without the model's selection ever holding it."""
from __future__ import annotations

import pytest
from core.input_types import QPointF, Qt
from ingetrazo_core import Vec3 as V
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tools.base import ToolContext
from views.extension_api import ExtensionApp


@pytest.fixture
def window(native_host):
    return native_host.window


def _click(win, point):
    """A Select-tool click on the pixel where ``point`` is drawn."""
    vp = win.viewport
    win._activate_tool("select")
    px = vp._world_to_pixel(point)
    assert px is not None
    vp.active_tool.on_click(ToolContext(
        viewport=vp, world=point, screen=QPointF(px[0], px[1]),
        modifiers=Qt.NoModifier, snap=None))


def _two_lights(win):
    panel = win._panel_factories["render_blender"](ExtensionApp(win, "render_blender"), win.viewport.update)
    panel.add_light("point", V(0, 0, 3))
    panel.add_light("spot", V(4, 0, 3))
    vp = win.viewport
    vp.camera.fit_to(V(-2, -2, 0), V(6, 2, 4))
    return panel, vp


def test_a_click_on_the_bulb_selects_that_light(window):
    panel, vp = _two_lights(window)
    _click(window, V(4, 0, 3))
    assert vp.extension_pick is not None
    assert panel.view_pick[0] == 1 and panel.selected == "1"
    assert not vp.scene.selection            # the model's selection: empty


def test_supr_deletes_it_in_one_undo_step(window):
    from tools.select import delete_selection_or_hover
    panel, vp = _two_lights(window)
    _click(window, V(0, 0, 3))
    assert delete_selection_or_hover(vp) is True
    lights = panel.app.document_data({})["lights"]
    assert [lt["kind"] for lt in lights] == ["spot"]
    assert vp.extension_pick is None and panel.view_pick is None
    assert vp.history.undo()
    assert len(panel.app.document_data({})["lights"]) == 2


def test_a_click_elsewhere_or_esc_lets_it_go(window):
    panel, vp = _two_lights(window)
    _click(window, V(4, 0, 3))
    _click(window, V(2, 0, 0))              # empty ground
    assert vp.extension_pick is None and panel.view_pick is None
    _click(window, V(4, 0, 3))
    vp.escape()
    assert vp.extension_pick is None and panel.view_pick is None


def test_after_an_undo_supr_does_not_delete_by_a_stale_index(window):
    from tools.select import delete_selection_or_hover
    panel, vp = _two_lights(window)
    _click(window, V(4, 0, 3))              # light #1 picked
    vp.history.undo()                        # the spot is gone again
    # Whatever still remembers index 1, the document changed since the
    # pick: Supr lets it go and deletes nothing.
    delete_selection_or_hover(vp)
    assert len(panel.app.document_data({})["lights"]) == 1
    assert vp.extension_pick is None

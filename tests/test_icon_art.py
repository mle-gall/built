# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The renderer's icon and pointer artwork (render_glue: icon_art.rs,
cursors.rs), read as pixels without a toolkit: every tool key draws an
icon of its own, they follow the theme's ink and accent at the thinned
stroke, panel initials keep the icons' weight, and pointers keep their
hotspots at the tool's working point."""
from __future__ import annotations

import numpy as np
import pytest
from ingetrazo_render import icon_art, icon_keys, letter_art, tool_cursor_name, tool_cursor_raster

INK = (0xd9, 0xdc, 0xe1)          # views/theme.py icon_ink
ACCENT = (0x6a, 0xa5, 0xff)       # views/theme.py accent_color
BASE = (27, 32, 40)


def art(key, ink=INK, accent=ACCENT, base=BASE):
    got = icon_art(key, ink, accent, base)
    assert got is not None, key
    w, h, rgba = got
    return np.frombuffer(rgba, np.uint8).reshape(h, w, 4)


def test_every_tool_key_draws_an_icon_of_its_own():
    keys = icon_keys()
    assert len(keys) >= 144, "icons missing from the atlas"
    drawn = {}
    for key in keys:
        px = art(key)
        assert px[..., 3].any(), f"{key} draws nothing"
        drawn[key] = px.tobytes()
    assert icon_art("no-such-tool", INK, ACCENT, BASE) is None
    views = ["view_iso", "view_top", "view_bottom", "view_front", "view_back", "view_right", "view_left"]
    assert len({drawn[k] for k in views}) == len(views), "two views draw the same"
    solids = ["outer_shell", "solid_union", "solid_subtract", "solid_trim", "solid_intersect", "solid_split"]
    assert len({drawn[k] for k in solids}) == len(solids)
    panels = ["panel_properties", "panel_bim", "panel_terrain", "panel_browser", "panel_ai",
              "panel_render", "panel_solid_inspector", "panel_levels", "layers_panel"]
    assert len({drawn[k] for k in panels}) == len(panels)


def test_the_icons_follow_the_ink_and_the_accent():
    line = art("line")
    # The accent's dots: premultiplied, so a solid accent pixel is the accent.
    solid = line[line[..., 3] > 240][:, :3].astype(int)
    assert any(np.abs(p - ACCENT).max() < 12 for p in solid)
    assert any(np.abs(p - INK).max() < 12 for p in solid)
    magenta = art("line", ink=(250, 0, 250))
    assert magenta.tobytes() != line.tobytes()
    assert (magenta[..., 3] == line[..., 3]).all()          # same drawing, new ink


def test_icons_are_drawn_with_fine_lines_and_the_chevron_keeps_its_weight():
    """Round 14: lines at 65 % of their written width (Line inked 152 or
    more pixels at full weight); the «more tools» chevron at full weight."""
    assert int((art("line")[..., 3] > 128).sum()) <= 130
    assert int((art("move")[..., 3] > 128).sum()) <= 215
    chevron = art("overflow_h")[..., 3]
    assert max(int((chevron[:, x] > 200).sum()) for x in range(48)) >= 3


def test_a_panel_initial_has_the_weight_of_the_icons():
    w, h, rgba = letter_art("Zeta", INK, ACCENT, BASE)
    px = np.frombuffer(rgba, np.uint8).reshape(h, w, 4)
    assert [x for x in range(4, 14) if px[24, x, 3] > 60] == [8, 9]
    other = letter_art("Ωmega", INK, ACCENT, BASE)        # drawn with the system fonts
    assert other is not None and np.frombuffer(other[2], np.uint8).reshape(48, 48, 4)[24, 9, 3] > 60


@pytest.mark.parametrize("key, hotspot", [("line", (4, 28)), ("eraser", (10, 21)), ("move", (16, 16)),
                                          ("orbit", (16, 16)), ("zoom", (14, 14)), ("offset", (2, 2))])
def test_pointers_keep_their_working_point(key, hotspot):
    name = tool_cursor_name(key, False, "tool")
    assert name == f"tool:{key}"
    w, h, x, y, rgba = tool_cursor_raster(name)
    assert (w, h) == (32, 32) and len(rgba) == 32 * 32 * 4
    assert (x, y) == hotspot
    px = np.frombuffer(rgba, np.uint8).reshape(32, 32, 4)
    assert px[..., 3].any()


def test_pointer_names_follow_the_preference_and_the_guide_mode():
    assert tool_cursor_name("select", False, "tool") is None
    assert tool_cursor_name("no_such_tool", False, "tool") is None
    assert tool_cursor_name("line", False, "crosshair") == "crosshair"
    assert tool_cursor_name("line", False, "arrow") is None
    assert tool_cursor_name("tape", True, "tool") == "tool:tape+"
    assert tool_cursor_raster("crosshair") is None and tool_cursor_raster(None) is None
    plain, plus = tool_cursor_raster("tool:tape"), tool_cursor_raster("tool:tape+")
    assert plain[4] != plus[4]                               # the «+» of guide mode
    for state in ("no", "1", "2"):
        assert tool_cursor_raster(f"tool:solid-{state}") is not None
    # At twice the pixels, the hotspot scales with them.
    assert tool_cursor_raster("tool:line", 64)[2:4] == (8, 56)


def test_icons_module_names_pointers_for_hosts_without_a_factory():
    from views import icons
    assert icons.tool_cursor("line") == "tool:line"
    assert icons.tool_cursor("select") is None
    assert icons.solid_cursor("1") == "tool:solid-1"

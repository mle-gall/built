# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Soft sun shadows (#1281): Preferences ▸ General turns them on, at a
level, off by default since they cost the graphics card more while the sun
casts shadows. The penumbra widens with the distance from what casts it
(percentage-closer soft shadows), its width worked out in the world so the
whole-model map, the detail map of a settled view (#958) and an export's
bigger map soften alike."""
from __future__ import annotations


import pytest
from ingetrazo_core import NativeSettings
from ingetrazo_core import Vec3 as V


from tests.preferences_chrome import Controls, settings_file
from views.document_interaction import SOFT_SHADOW_SPREAD, _load_soft_shadows
from tests.controller_surface import ControllerSurface as Viewport


class _VP:
    edit_rest_mode = "fade"
    _msaa = 4                               # as the dialog leaves it
    _soft_shadows = 0

    def __init__(self):
        self.updates = 0

    def set_edit_rest_mode(self, mode):
        self.edit_rest_mode = mode

    def update(self):
        self.updates += 1


class _Win:
    def __init__(self):
        super().__init__()
        self.viewport = _VP()
        self.back = _VP()                   # a project in another tab
        self._rest_actions = {}

    def all_viewports(self):
        return [self.viewport, self.back]


def _fresh(path):
    return NativeSettings(path.parent.parent)


def test_off_by_default_with_its_level_greyed(settings_file):
    ui = Controls(_Win(), "display")
    assert not ui.value("soft_shadows")
    assert not ui.enabled("soft_level")
    assert ui.value("soft_level") == 2
    ui.set("soft_shadows", True)
    assert ui.enabled("soft_level")


def test_ok_saves_the_level_and_every_view_redraws_at_once(settings_file):
    win = _Win()
    ui = Controls(win, "display")
    ui.set("soft_shadows", True)
    ui.set("soft_level", 3)
    st = _fresh(settings_file)
    assert int(st.value("display/soft_shadows")) == 3
    for vp in (win.viewport, win.back):
        assert vp._soft_shadows == 3
        assert vp.updates == 2


def test_turned_off_the_level_is_kept_for_next_time(settings_file):
    st = _fresh(settings_file)
    st.setValue("display/soft_shadows", 1)
    st.setValue("display/soft_shadows_level", 1)
    st.sync()
    win = _Win()
    ui = Controls(win, "display")
    assert ui.value("soft_shadows")
    assert ui.value("soft_level") == 1
    ui.set("soft_shadows", False)
    st = _fresh(settings_file)
    assert int(st.value("display/soft_shadows")) == 0
    assert int(st.value("display/soft_shadows_level")) == 1
    assert win.viewport._soft_shadows == 0
    ui = Controls(_Win(), "display")
    assert not ui.value("soft_shadows")
    assert ui.value("soft_level") == 1


def test_restore_defaults_turns_them_off(settings_file):
    st = _fresh(settings_file)
    st.setValue("display/soft_shadows", 3)
    st.sync()
    ui = Controls(_Win(), "display")
    assert ui.value("soft_shadows")
    ui.restore()
    assert not ui.value("soft_shadows")
    assert ui.value("soft_level") == 2


@pytest.mark.parametrize("stored, level", [
    (None, 0), ("0", 0), ("2", 2), ("3", 3), ("7", 0), ("lots", 0)])
def test_the_viewport_reads_the_level_and_ignores_nonsense(
        tmp_path, monkeypatch, stored, level):
    from core import settings
    st = NativeSettings(tmp_path)
    monkeypatch.setattr(settings, "_store", st)
    if stored is not None:
        st.setValue("display/soft_shadows", stored)
    st.sync()
    assert _load_soft_shadows() == level


def test_the_footprint_of_the_map_comes_off_the_light_matrix():
    """The shader turns world widths into map coordinates with it: metres
    across a side and along the depth range, for the whole-model map and
    for a detail map of a settled view (#958) alike."""
    lo, hi = V(-3, -2, 0), V(5, 7, 4)
    d = (0.3, 0.5, 0.8)
    n = sum(c * c for c in d) ** 0.5
    d = tuple(c / n for c in d)
    _view, r = Viewport._light_view(d, lo, hi)
    across, along = Viewport._shadow_footprint(Viewport._light_vp(d, lo, hi))
    assert across == pytest.approx(2.0 * r, rel=1e-5)
    assert along == pytest.approx(3.0 * r, rel=1e-5)
    across, along = Viewport._shadow_footprint(
        Viewport._light_vp(d, lo, hi, (0.5, -0.2, 1.25)))
    assert across == pytest.approx(2.5, rel=1e-5)
    assert along == pytest.approx(3.0 * r, rel=1e-5)


def test_the_levels_grow():
    spreads = [SOFT_SHADOW_SPREAD[k] for k in sorted(SOFT_SHADOW_SPREAD)]
    assert spreads == sorted(spreads) and len(set(spreads)) == 3

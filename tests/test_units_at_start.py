# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The units question at startup (metres, centimetres, millimetres or
imperial, with
«Don't show this at startup»), its switch in Preferences ▸ Units, and
Window ▸ Units… on Ctrl+U."""
from __future__ import annotations

import pytest

from core import units
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import chrome_forms, units_question


@pytest.fixture
def win(native_host):
    units.set_ask_at_start(True)
    units.remember_new_document_units(units.DEFAULT_MODEL_UNITS)
    return native_host.window


def rows(form):
    return {row[0]: row for row in form.wire()[4]}


def choose(win, profile, *, dont_ask=False, accept=True):
    units_question.offer_at_start(win)
    form = chrome_forms.current(win)
    before = rows(form)["profile"]
    form.receive("profile", str(profile))
    form.receive("dont_ask", "1" if dont_ask else "0")
    preview = rows(form)["preview"][3]
    form.press("ok" if accept else "cancel")
    return before, preview


# ---- the four configurations -------------------------------------------------

@pytest.mark.parametrize("profile, expected, shown", [
    ("metric", {"length": "m", "precision": 2}, "1.23 m"),
    ("centimetric", {"length": "cm", "precision": 1}, "123.5 cm"),
    ("millimetric", {"length": "mm", "precision": 0}, "1235 mm"),
    ("imperial", {"length": "ft-in-frac", "precision": 2}, "4'5/8\""),
])
def test_each_choice_sets_the_new_document(win, profile,
                                           expected, shown):
    row, preview = choose(win, list(units.PROFILES).index(profile))
    assert len(row[7]) == 5 and row[4] == 0
    assert shown in preview
    scene = win.viewport.scene
    assert scene.units == expected
    assert scene.version == win._saved_version         # nothing to save
    assert units.new_document_units() == expected      # and every new one
    win._on_new()
    assert win.viewport.scene.units == expected


def test_metres_mean_square_and_cubic_metres_and_degrees(win):
    choose(win, 0)
    assert units.fmt_area(2.0) == "2.00 m²"
    assert units.fmt_volume(2.0) == "2.000 m³"
    assert units.fmt_angle_deg(30) == "30.0°"


def test_the_last_choice_is_preselected_next_time(win):
    for length, precision, index in [("mm", 0, 2), ("cm", 1, 1), ("cm", 3, 4)]:
        settings = {"length": length, "precision": precision}
        units.remember_new_document_units(settings)
        units_question.offer_at_start(win)
        form = chrome_forms.current(win)
        assert rows(form)["profile"][4] == index
        assert rows(form)["preview"][3] == units.preview_text(settings)
        form.press("cancel")


def test_centimetres_are_shown_in_order_and_follow_in_cm2_cm3(win):
    row, _ = choose(win, 1)
    choices = row[7]
    assert [s.partition(" — ")[0] for s in choices] == [
        "Metres", "Centimetres", "Millimetres", "Imperial (feet and inches)", "Custom…"]
    assert "cm · cm² · cm³" in choices[1]
    assert units.fmt_len(1.2345) == "123.5 cm"
    assert units.fmt_area(2.0) == "20000.0 cm²"
    assert units.fmt_volume(2e-6) == "2.000 cm³"


@pytest.mark.parametrize("dont_ask", [False, True])
def test_closing_the_question_changes_nothing(win, dont_ask):
    choose(win, 3, dont_ask=dont_ask, accept=False)
    assert units.ask_at_start() == (not dont_ask)
    assert win.viewport.scene.units == {"length": "m", "precision": 2}
    assert units.new_document_units() == {"length": "m", "precision": 2}


# ---- «Don't show this at startup» and back on ---------------------------------

def test_dont_show_again(win):
    choose(win, 2, dont_ask=True)
    assert win.viewport.scene.units["length"] == "mm"
    assert not units.ask_at_start()
    units_question.offer_at_start(win)
    assert chrome_forms.current(win) is None


def test_preferences_turn_it_back_on(win):
    from tests.units_chrome import edit_unit, units_row
    from ingetrazo_render.settings_sheet import Env
    units.set_ask_at_start(False)
    assert not units_row(win, "ask_start").value(Env(win))
    edit_unit(win, "ask_start", 1)
    edit_unit(win, "for_new", 0)
    assert units.ask_at_start() is True
    row, _ = choose(win, 0)
    assert row[4] == 0


def test_a_document_already_open_is_not_asked_about(win, tmp_path):
    from app_shell import files
    from core.scene import Scene
    from formats import igz
    from ingetrazo_core import Vec3
    from views import wgpu_start

    path = tmp_path / "plano.igz"
    igz.save_scene(Scene(), path)
    assert files.open_path(win, path)
    assert chrome_forms.current(win) is None and not wgpu_start.shown(win)
    win.new_document()
    win.viewport.scene.add_edge(Vec3(0, 0, 0), Vec3(1, 0, 0))
    files.cold_start(win)
    assert chrome_forms.current(win) is None and not wgpu_start.shown(win)


# ---- Ctrl+U -------------------------------------------------------------------------

def test_ctrl_u_opens_the_units_page(win):
    from ingetrazo_render import chrome_sheet
    from views.shortcuts import collect_actions, action_keys
    act = win._registry_actions["menu:units"]
    assert [a for a in collect_actions(win) if "Ctrl+U" in action_keys(a)] == [act]
    act.trigger()
    assert chrome_sheet.current(win).section == "units"


def test_custom_leads_to_the_units_panel(win):
    choose(win, 4)
    form = chrome_forms.current(win)
    assert rows(form)["for_new"][4] == 1
    form.receive("length", str(units.UNIT_CHOICES.index("cm")))
    form.receive("precision", "1")
    form.receive("area", str(1 + units.AREA_CHOICES.index("m2")))
    form.receive("area_precision", "2")
    form.receive("decimal", "1")
    form.press("ok")
    want = {"length": "cm", "precision": 1, "area": "m2", "area_precision": 2,
            "decimal": ","}
    assert win.viewport.scene.units == want
    assert win.viewport.scene.version == win._saved_version   # nothing to save
    assert units.new_document_units() == want
    assert units.fmt_len(1.2345) == "123,5 cm"
    assert units.fmt_area(12.345) == "12,35 m²"


def test_custom_cancelled_changes_nothing(win):
    choose(win, 4)
    form = chrome_forms.current(win)
    form.receive("length", str(units.UNIT_CHOICES.index("cm")))
    form.receive("for_new", "0")
    form.press("cancel")
    assert win.viewport.scene.units == units.DEFAULT_MODEL_UNITS
    assert units.new_document_units() == units.DEFAULT_MODEL_UNITS
    assert win.viewport.scene.version == win._saved_version

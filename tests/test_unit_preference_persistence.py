# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Built contributors.
"""Unit choices and the new-document opt-in survive Preferences and reopening."""
from __future__ import annotations

from types import SimpleNamespace

import pytest

from core import units
from core.scene import Scene
from formats import igz
from built_render.settings_sheet import Env, SettingsSheet
from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401
from tests.units_chrome import edit_unit, units_row


CM = {"length": "cm", "precision": 2}
METRES = {"length": "m", "precision": 2, "area": "m2", "area_precision": 2}


@pytest.fixture(autouse=True)
def isolated_preferences(private_settings):
    units.remember_new_document_units(CM)


class _Viewport:
    edit_rest_mode = "fade"
    history = None

    def __init__(self, settings):
        self.scene = Scene()
        units.apply_units(self.scene, settings)
        self.notifications = 0

    def update(self):
        pass

    def set_edit_rest_mode(self, mode):
        self.edit_rest_mode = mode

    def notify_scene_changed(self):
        self.notifications += 1


def _window(settings):
    window = SimpleNamespace()
    window.viewport = _Viewport(settings)
    return window


def _choose(window, settings):
    edit_unit(window, "length", units.UNIT_CHOICES.index(settings["length"]))
    edit_unit(window, "precision", settings["precision"])
    if "area" in settings:
        edit_unit(window, "area", units.AREA_CHOICES.index(settings["area"]) + 1)
        edit_unit(window, "area_precision", settings["area_precision"])


def _edit_autosave(window):
    env = Env(window)
    row = next(r for _, _, rows in SettingsSheet().groups(env, "general") for r in rows
               if r.id == "autosave_min")
    row.put(env, row.value(env) + 1)


def _for_new(window):
    return bool(units_row(window, "for_new").value(Env(window)))


@pytest.mark.parametrize("chosen", [METRES, {**METRES, "length": "cm"},
                                    {"length": "mm", "precision": 1}])
def test_changed_units_remain_the_default_when_the_open_model_differs(chosen):
    window = _window({"length": "ft", "precision": 2})
    _choose(window, chosen)

    # Applying the document's units must also remember the still-enabled
    # new-document choice; it cannot be inferred from the old units matching.
    assert window.viewport.scene.units == chosen
    assert units.new_document_units() == chosen
    from built_render import ShellWindow
    fresh_scene = Scene()
    fresh_window = SimpleNamespace(viewport=SimpleNamespace(scene=fresh_scene))
    ShellWindow._apply_new_document_units(fresh_window)
    assert fresh_scene.units == chosen


def test_accepting_unrelated_preferences_does_not_restore_a_stale_default():
    window = _window(CM)
    # Another window can change the global default while this dialog is open.
    units.remember_new_document_units(METRES)
    _edit_autosave(window)

    assert units.new_document_units() == METRES
    assert window.viewport.scene.units == CM
    assert window.viewport.notifications == 0


def test_document_only_choice_is_remembered_after_reopening_preferences():
    window = _window(CM)
    edit_unit(window, "for_new", 0)
    _choose(window, METRES)
    assert window.viewport.scene.units == METRES
    assert units.new_document_units() == CM

    # Match the saved default here: equality used to re-enable the checkbox.
    reopened_window = _window(CM)
    assert not _for_new(reopened_window)
    _choose(reopened_window, {"length": "mm", "precision": 0})
    assert reopened_window.viewport.scene.units["length"] == "mm"
    assert units.new_document_units() == CM


def test_unrelated_preferences_do_not_reenable_a_newer_document_only_choice():
    stale_window = _window(CM)
    other_window = _window(CM)
    edit_unit(other_window, "for_new", 0)
    _edit_autosave(stale_window)

    final_window = _window(CM)
    assert not _for_new(final_window)
    assert units.new_document_units() == CM


def test_explicitly_enabling_defaults_remembers_unchanged_document_units():
    window = _window(CM)
    edit_unit(window, "for_new", 0)

    other_window = _window(METRES)
    assert not _for_new(other_window)
    edit_unit(other_window, "for_new", 1)
    assert other_window.viewport.scene.units == METRES
    assert units.new_document_units() == METRES


def test_cancel_does_not_change_units_or_the_document_only_choice(native_question_host):
    units.remember_new_document_units(CM)
    window = _window(CM)
    edit_unit(window, "for_new", 0)

    from built_render import chrome_forms, units_question
    other_window = native_question_host.window
    units.apply_units(other_window.viewport.scene, CM)
    units_question.offer_at_start(other_window)
    form = chrome_forms.current(other_window)
    form.receive("profile", "4")
    form.press("ok")
    form = chrome_forms.current(other_window)
    form.receive("for_new", "1")
    form.receive("length", "0")
    form.press("cancel")
    assert units.new_document_units() == CM
    assert other_window.viewport.scene.units == CM
    final_window = _window(CM)
    assert not _for_new(final_window)


def test_independent_length_and_area_units_roundtrip_in_the_model(tmp_path):
    chosen = {**METRES, "length": "cm"}
    window = _window({"length": "mm", "precision": 0})
    _choose(window, chosen)
    path = tmp_path / "independent-units.igz"
    igz.save_scene(window.viewport.scene, path)

    loaded = Scene()
    igz.load_into(loaded, path)
    assert loaded.units == chosen
    assert loaded.dimension_style["units"] == "cm"
    assert units.new_document_units() == chosen

    # A document retains its own units even when the global default changes.
    units.remember_new_document_units(METRES)
    igz.load_into(loaded, path)
    assert loaded.units == chosen
    assert units.new_document_units() == METRES


def test_document_unit_changes_notify_the_window():
    window = _window(CM)
    original_version = window.viewport.scene.version
    edit_unit(window, "profile", 2)
    assert window.viewport.scene.version == original_version + 1
    assert window.viewport.notifications == 1

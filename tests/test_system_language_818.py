# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""#818: "Same as the system" as a UI language choice.

The OS display language picks the closest catalog; the choice is offered
in Preferences (and Window ▸ Language) and is what an unset choice means.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest
from built_render import shell_services as native
from built_core import i18n, settings

from tests.preferences_chrome import Controls, settings_file
from built_render.settings_sheet import _PENDING


@pytest.fixture(scope="module")
def complete_catalogues(tmp_path_factory):
    root = tmp_path_factory.mktemp("catalogues")
    directory = root / "i18n"
    directory.mkdir()
    inventory = Path(__file__).resolve().parents[1] / "rust/built_core/src/core_bulk/translation_sources.json"
    data = json.dumps(dict.fromkeys(json.loads(inventory.read_text("utf8")), "Translated"))
    for code in i18n.LANGUAGE_NAMES:
        (directory / f"{code}.json").write_text(data, encoding="utf8")
    return root


@pytest.mark.parametrize("locales, expected", [
    (["es-ES"], "es"),
    (["pt-BR"], "pt-BR"),
    (["pt-PT"], "pt-BR"),            # same base language
    (["de_AT"], "de"),               # Qt's underscore spelling too
    (["zh-Hans-CN"], "zh-CN"),
    (["zh-CN"], "zh-CN"),
    (["zh"], "zh-CN"),
    (["zh-SG"], "zh-CN"),
    (["zh-TW"], "zh-TW"),
    (["zh_TW"], "zh-TW"),
    (["zh-Hant-TW", "zh-TW", "zh-Hant", "zh"], "zh-TW"),  # Qt's list in Taiwan
    (["zh-Hant"], "zh-TW"),
    (["zh-HK"], "zh-TW"),
    (["zh-Hant-MO"], "zh-TW"),
    (["it-IT", "en-US"], "it"),
    (["en-US", "es-ES"], "en"),      # the first preferred one decides
    (["nl-NL", "fr-FR"], "fr"),      # no Dutch catalog: next preference
    (["nl-NL"], "en"),
    ([], "en"),
])
def test_system_locales_pick_the_closest_catalog(locales, expected, complete_catalogues, monkeypatch):
    monkeypatch.setattr(sys, "_MEIPASS", str(complete_catalogues), raising=False)
    assert i18n.language_for_locales(locales) == expected


@pytest.fixture
def system_is_french(monkeypatch):
    monkeypatch.setattr(native, "available", lambda: True)
    monkeypatch.setattr("built_render.preferred_languages", lambda: ["fr-FR", "en-US"])


def test_incomplete_system_and_saved_catalogues_use_english(system_is_french):
    assert i18n.resolve_language(None) == "en"
    assert i18n.resolve_language("") == "en"
    assert i18n.resolve_language(i18n.SYSTEM) == "en"
    assert i18n.resolve_language("fr") == "en"


@pytest.fixture
def prefs(settings_file, system_is_french):
    before = i18n.current_language()
    _PENDING.clear()
    i18n.set_language("en")
    yield settings_file
    i18n.set_language(before)
    _PENDING.clear()


class _Win:
    def __init__(self):
        super().__init__()

        class _VP:
            edit_rest_mode = "fade"

            def set_edit_rest_mode(self, mode):
                self.edit_rest_mode = mode

        self.viewport = _VP()
        self._rest_actions = {}


def test_preferences_offers_the_system_language_first_and_by_default(prefs):
    ui = Controls(_Win(), "general")
    assert next(iter(ui.options("language"))) == i18n.SYSTEM
    assert ui.value("language") == i18n.SYSTEM     # nothing saved yet


def test_going_back_to_the_system_language_is_saved(prefs):
    st = settings.settings()
    st.setValue("language", "fr")
    st.sync()
    ui = Controls(_Win(), "general")
    assert ui.value("language") == "en"
    assert any(b.get("text") == "Using English because your saved language (Français) is unavailable."
               for b in ui.wire()["content"]["blocks"])
    ui.set("language", i18n.SYSTEM)
    assert settings.settings().value("language") == "system"
    # The system's language is the one running: nothing to restart for.
    assert not _PENDING.labels()
    assert i18n.current_language() == "en"


def test_incomplete_choice_is_disabled(prefs):
    ui = Controls(_Win(), "general")
    labels = ui.options("language")
    count, total = i18n.language_coverage()["fr"]
    assert labels["fr"] == f"Français · {100 * count / total:.1f} % — incomplete"
    assert labels["ja"].startswith("Japanese · ")
    ui.set("language", "fr")
    assert settings.settings().value("language") is None
    assert not _PENDING.labels()
    assert i18n.current_language() == "en"


@pytest.mark.parametrize("saved", [None, i18n.SYSTEM, "fr"])
def test_startup_resolves_the_saved_choice(saved, tmp_path, monkeypatch,
                                           system_is_french):
    from built_render import start_profile
    from built_core import settings

    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setattr(settings, "_store", None)
    settings.use_native(str(tmp_path))
    if saved is not None:
        prefs = settings.settings()
        prefs.setValue("language", saved)
        prefs.sync()
    before = i18n.current_language()
    try:
        start_profile(str(tmp_path))
        assert i18n.current_language() == "en"
    finally:
        i18n.set_language(before)


def test_complete_saved_language_is_used(complete_catalogues, monkeypatch):
    monkeypatch.setattr(sys, "_MEIPASS", str(complete_catalogues), raising=False)
    assert i18n.resolve_language("fr") == "fr"

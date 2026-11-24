# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""#818: "Same as the system" as a UI language choice.

The OS display language picks the closest catalog; the choice is offered
in Preferences (and Window ▸ Language) and is what an unset choice means.
"""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
import PySide6.QtCore as QtCore
from PySide6.QtCore import QSettings
from PySide6.QtWidgets import QApplication, QWidget

from core import i18n

_inst = QApplication.instance()
if _inst is None:
    _app = QApplication([])
elif not isinstance(_inst, QApplication):
    pytest.skip("a non-widget QGuiApplication is already active",
                allow_module_level=True)

from tests.preferences_chrome import Controls, settings_file
from ingetrazo_render.settings_sheet import _PENDING


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
def test_system_locales_pick_the_closest_catalog(locales, expected):
    assert i18n.language_for_locales(locales) == expected


class _FakeLocale:
    languages = ["fr-FR", "en-US"]

    @classmethod
    def system(cls):
        return cls

    @classmethod
    def uiLanguages(cls):
        return cls.languages


@pytest.fixture
def system_is_french(monkeypatch):
    monkeypatch.setattr(QtCore, "QLocale", _FakeLocale)


def test_unset_or_system_follows_the_os_and_a_pinned_one_stays(system_is_french):
    assert i18n.resolve_language(None) == "fr"
    assert i18n.resolve_language("") == "fr"
    assert i18n.resolve_language(i18n.SYSTEM) == "fr"
    assert i18n.resolve_language("de") == "de"


@pytest.fixture
def prefs(settings_file, system_is_french):
    before = i18n.current_language()
    _PENDING.clear()
    i18n.set_language("fr")
    yield settings_file
    i18n.set_language(before)
    _PENDING.clear()


class _Win(QWidget):
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
    path = prefs
    st = QSettings(str(path), QSettings.IniFormat)
    st.setValue("language", "fr")
    st.sync()
    ui = Controls(_Win(), "general")
    assert ui.value("language") == "fr"
    ui.set("language", i18n.SYSTEM)
    assert QSettings(str(path), QSettings.IniFormat).value("language") == "system"
    # The system's language is the one running: nothing to restart for.
    assert not _PENDING.labels()
    assert i18n.current_language() == "fr"


def test_pinning_another_language_over_the_system_one_asks_a_restart(prefs):
    path = prefs
    ui = Controls(_Win(), "general")
    ui.set("language", "de")
    assert QSettings(str(path), QSettings.IniFormat).value("language") == "de"
    assert len(_PENDING.labels()) == 1
    assert i18n.current_language() == "de"


@pytest.mark.parametrize("saved", [None, i18n.SYSTEM, "de"])
def test_startup_resolves_the_saved_choice(saved, tmp_path, monkeypatch,
                                           system_is_french):
    """main._init_language runs the same resolution as the menus: an
    unset choice or "system" starts in the OS language, never crashes
    nor activates a catalog literally named "system"."""
    import main
    path = tmp_path / "start.ini"
    st = QSettings(str(path), QSettings.IniFormat)
    if saved is not None:
        st.setValue("language", saved)
    st.sync()
    monkeypatch.setattr(main, "QSettings",
                        lambda: QSettings(str(path), QSettings.IniFormat))
    installed = []
    monkeypatch.setattr(main, "_install_qt_translator", installed.append)
    before = i18n.current_language()
    try:
        main._init_language()
        expected = "de" if saved == "de" else "fr"
        assert i18n.current_language() == expected
        assert installed == [expected]
    finally:
        i18n.set_language(before)

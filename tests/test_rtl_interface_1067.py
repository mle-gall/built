# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Language direction, Arabic translations and the legacy launcher language hook."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PySide6.QtCore import QSettings, Qt
from PySide6.QtWidgets import QApplication

from core import i18n

_app = QApplication.instance() or QApplication([])


def test_rtl_languages():
    assert i18n.is_rtl("ar")
    assert i18n.is_rtl("fa-IR") and i18n.is_rtl("he") and i18n.is_rtl("ur")
    for lang in ("es", "en", "pt-BR", "zh-CN", "uk", "vi", "", None):
        assert not i18n.is_rtl(lang)


def test_arabic_is_offered_and_complete():
    assert i18n.LANGUAGE_NAMES["ar"] == "العربية"
    assert "ar" in i18n.available_languages()
    before = i18n.current_language()
    try:
        i18n.set_language("ar")
        # The «AI» tab: artificial intelligence, not Amnesty International.
        assert i18n.tr("AI") == "الذكاء الاصطناعي"
    finally:
        i18n.set_language(before)


@pytest.mark.parametrize("lang, direction", [
    ("ar", Qt.RightToLeft), ("es", Qt.LeftToRight), ("en", Qt.LeftToRight),
    ("de", Qt.LeftToRight)])
def test_startup_direction_follows_the_language(lang, direction, tmp_path,
                                                monkeypatch):
    import main
    path = tmp_path / "start.ini"
    st = QSettings(str(path), QSettings.IniFormat)
    st.setValue("language", lang)
    st.sync()
    monkeypatch.setattr(main, "QSettings",
                        lambda: QSettings(str(path), QSettings.IniFormat))
    monkeypatch.setattr(main, "_install_qt_translator", lambda _l: None)
    before = i18n.current_language()
    try:
        main._init_language()
        assert _app.layoutDirection() == direction
        # Left to right nothing is installed: the app is as it always was.
        assert (getattr(_app, "_rtl_numbers", None) is not None) == (
            direction == Qt.RightToLeft)
    finally:
        i18n.set_language(before)
        _app.setLayoutDirection(Qt.LeftToRight)
        f = getattr(_app, "_rtl_numbers", None)
        if f is not None:
            _app.removeEventFilter(f)
            _app._rtl_numbers = None

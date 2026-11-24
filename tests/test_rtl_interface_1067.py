# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Language direction, Arabic translations and the legacy launcher language hook."""
from __future__ import annotations

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from core import i18n


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


@pytest.mark.parametrize("lang, rtl", [("ar", True), ("es", False), ("en", False), ("de", False)])
def test_plugin_language_uses_the_document_host_locale(native_host, lang, rtl):
    from plugins._widget_language import apply_widget_language
    before = i18n.current_language()
    try:
        apply_widget_language(native_host, lang)
        assert i18n.current_language() == lang
        assert i18n.is_rtl(i18n.current_language()) == rtl
    finally:
        i18n.set_language(before)

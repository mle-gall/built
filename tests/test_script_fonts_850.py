# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Lao on Windows (#850): the interface font starts with one that holds
the script, so the vowel and tone marks compose with their consonant.
Every other language, and Lao elsewhere, start exactly as before."""
from __future__ import annotations

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from plugins._widget_language import chrome_font, script_font_for

WINDOWS_FONTS = ["Noto Sans Lao", "Segoe UI", "Leelawadee UI", "Arial"]


def test_lao_on_windows_takes_leelawadee_ui():
    assert script_font_for("lo", WINDOWS_FONTS, "win32") == "Noto Sans Lao"


def test_lao_on_windows_without_noto_takes_the_next():
    assert script_font_for("lo", [ "Noto Sans Lao", "Segoe UI",],
                           "win32") == "Noto Sans Lao"
    assert script_font_for("lo", ["Segoe UI", "Lao UI"], "win32") == "Lao UI"


def test_lao_on_windows_with_no_lao_font_changes_nothing():
    assert script_font_for("lo", ["Segoe UI", "Arial"], "win32") is None


@pytest.mark.parametrize("platform", ["linux", "darwin"])
def test_lao_elsewhere_changes_nothing(platform):
    assert script_font_for("lo", WINDOWS_FONTS, platform) is None


@pytest.mark.parametrize("lang", ["es", "en", "th", "ar", "zh-CN", "", None])
def test_other_languages_change_nothing(lang):
    assert script_font_for(lang, WINDOWS_FONTS, "win32") is None


def test_region_tags_read_as_their_language():
    assert script_font_for("lo-LA", WINDOWS_FONTS, "win32") == "Noto Sans Lao"
    assert script_font_for("lo_LA", WINDOWS_FONTS, "win32") == "Noto Sans Lao"



def test_native_chrome_selects_the_available_lao_face(tmp_path):
    default = chrome_font("es", "win32", [tmp_path])
    assert chrome_font("lo", "win32", [tmp_path]) == default
    second = tmp_path / "LeelawUI.ttf"
    second.touch()
    assert chrome_font("lo-LA", "win32", [tmp_path]) == str(second)
    preferred = tmp_path / "NotoSansLao-Regular.ttf"
    preferred.touch()
    assert chrome_font("lo", "win32", [tmp_path]) == str(preferred)
    assert chrome_font("es", "win32", [tmp_path]) == default
    assert chrome_font("lo", "linux", [tmp_path]) == default


def test_startup_uses_the_selected_catalogue(native_host):
    from built_core import i18n
    from plugins._widget_language import apply_widget_language
    previous = i18n.current_language()
    try:
        apply_widget_language(native_host, "lo")
        assert i18n.current_language() == "lo"
    finally:
        i18n.set_language(previous)

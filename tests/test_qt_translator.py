# SPDX-License-Identifier: GPL-3.0-or-later
"""Native panel forms and command shortcuts share the application catalogue."""
import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from built_core import i18n
from built_render.chrome_forms import Field, Form


@pytest.fixture
def language():
    before = i18n.current_language()
    yield i18n.set_language
    i18n.set_language(before)


@pytest.mark.parametrize("lang, cancel", [
    ("es", "Cancelar"), ("pt-BR", "Cancelar"), ("vi", "Hủy bỏ"), ("ja", "キャンセル"),
])
def test_standard_buttons_are_translated(language, lang, cancel):
    language(lang)
    assert dict(Form("Settings", []).buttons)["cancel"] == cancel


def test_english_uses_the_source_label(language):
    language("en")
    assert dict(Form("Settings", []).buttons)["cancel"] == "Cancel"


def test_colour_field_label_is_translated(language):
    language("vi")
    field = Field("colour", "color", i18n.tr("Colour"), "#ffffff")
    assert Form("Settings", [field]).wire()[4][0][2] == "Màu sắc"


@pytest.mark.parametrize("lang", ["es", "ja"])
def test_shortcuts_keep_their_english_key_names(language, native_host, lang):
    from views.extension_api import ExtensionApp
    language(lang)
    app = ExtensionApp(native_host.window, "translated")
    app.add_menu_action(i18n.tr("Save"), lambda: None, "Ctrl+Shift+PgUp")
    rows = native_host.window._extension_menus.rows()
    assert any(row[2] == i18n.tr("Save") and "PgUp" in str(row) for row in rows)

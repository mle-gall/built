# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Keyboard preferences use stable registry IDs across restarts and languages."""
from contextlib import contextmanager
import json
from pathlib import Path

import pytest

from ingetrazo_core.i18n import current_language, set_language
from ingetrazo_core.settings import settings
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_render import settings_sheet


def keyboard(win):
    sheet = settings_sheet.SettingsSheet("keyboard")
    sheet._win = win
    return sheet


@pytest.fixture
def sheet(native_host):
    return keyboard(native_host.window)


@pytest.fixture
def reopen(tmp_path, monkeypatch, restore_shell_patches):
    from ingetrazo_render import Host
    from ingetrazo_core import settings as settings_module
    from ingetrazo_core import NativeSettings
    profile = str(tmp_path / "profile")

    @contextmanager
    def open_window():
        monkeypatch.setattr(settings_module, "_store", NativeSettings(profile))
        host = Host(0, 900, 600, 1., settings=profile)
        try:
            yield keyboard(host.window)
        finally:
            host.close()

    return open_window


def rows(sheet):
    content = sheet._keyboard_snapshot()["content"]
    items = next(block["items"] for block in content if block.get("id") == "keyboard")
    return {r["id"]: r for r in items}


@pytest.mark.parametrize("lang", ["es", "ja"])
def test_a_shortcut_of_ones_own_survives_a_restart_and_the_language(reopen, lang):
    key = "sc:menu:explode_group"
    with reopen() as sheet:
        sheet._assign(sheet._action(key), ["Ctrl+Alt+E"])
    old = current_language()
    try:
        set_language(lang)
        with reopen() as fresh:
            assert fresh._action(key).portable_keys() == ["Ctrl+Alt+E"]
    finally:
        set_language(old)


def test_japanese_keeps_every_action_key(reopen):
    old = current_language()
    try:
        set_language("en")
        with reopen() as sheet:
            english = rows(sheet).keys()
        set_language("ja")
        with reopen() as sheet:
            assert rows(sheet).keys() == english
    finally:
        set_language(old)


def test_a_clash_takes_the_keys_from_the_other_action(sheet, monkeypatch):
    forms = []
    monkeypatch.setattr(settings_sheet.cf, "show", lambda _, form: forms.append(form))
    group = sheet._action("sc:menu:make_group")
    explode = sheet._action("sc:menu:explode_group")
    taken, = group.portable_keys()
    sheet._assign(explode, [taken])
    forms.pop().press("ok")
    assert not group.portable_keys() and taken in explode.portable_keys()
    assert sheet._action("sc:sheet:group").portable_keys() == [taken]
    from ingetrazo_render import shortcut_domains
    for domain in (1, 2):
        keys = [key for action in sheet.actions() if shortcut_domains(action.objectName()) & domain
                for key in action.portable_keys()]
        assert len(keys) == len(set(keys))


def test_default_puts_the_factory_keys_back(sheet):
    sheet.state.kb_focus = "sc:menu:make_group"
    action = sheet._action(sheet.kb_focus)
    factory = action.portable_keys()
    sheet._kb_verb("kb_clear")
    assert not action.portable_keys()
    sheet._kb_verb("kb_default")
    assert action.portable_keys() == factory
    assert not settings().contains("shortcuts/menu:make_group")


def test_keys_the_viewport_reads_itself_are_refused():
    for bad in ("5", "0", ".", ",", ";", "Esc", "Return", "Backspace",
                "Left", "Shift+Up", "Tab"):
        assert isinstance(settings_sheet._parse_keys(bad), str), bad
    for good in ("Ctrl+5", "Alt+Left", "Ctrl+Shift+G", "F4", "K", "Shift+K"):
        assert settings_sheet._parse_keys(good) == [good]


def test_the_key_box_accepts_one_combination(sheet, monkeypatch):
    forms = []
    monkeypatch.setattr(settings_sheet.cf, "show", lambda _, form: forms.append(form))
    sheet.state.kb_focus = "sc:menu:make_group"
    action = sheet._action(sheet.kb_focus)
    sheet._kb_verb("kb_change")
    form = forms.pop()
    form.receive("keys", "Control")
    form.press("ok")
    assert form.error and action.portable_keys() == ["Ctrl+G"]
    form.receive("keys", "Ctrl+Alt+K")
    form.press("ok")
    assert action.portable_keys() == ["Ctrl+Alt+K"]


def test_a_shortcut_on_an_action_with_a_slash_survives_a_restart(reopen):
    keys = {"sc:tool:pushpull": "Ctrl+Alt+Shift+J",
            "sc:menu:toggle_projection": "Ctrl+Alt+Shift+H"}
    with reopen() as sheet:
        for command, key in keys.items():
            sheet._assign(sheet._action(command), [key])
    with reopen() as fresh:
        for command, key in keys.items():
            assert fresh._action(command).portable_keys() == [key]


def test_a_shortcut_saved_nested_by_an_earlier_version_is_read_back(reopen):
    with reopen():
        settings().setValue("shortcuts/text:Push / Pull", "Ctrl+Alt+Shift+J")
        settings().sync()
    with reopen() as sheet:
        assert sheet._action("sc:tool:pushpull").portable_keys() == ["Ctrl+Alt+Shift+J"]


@pytest.mark.parametrize("name", ["Selected", "Last", "All"])
def test_a_command_says_which_menu_it_lives_in(sheet, name):
    row = rows(sheet)["sc:menu:unhide_" + name.lower()]
    assert (row["label"], row["sub"]) == (name, "Edit ▸ Unhide")


def test_the_filter_finds_every_command_of_a_menu(sheet):
    sheet.state.query = "unhide"
    shown = {r["label"] for r in rows(sheet).values()}
    assert {"Selected", "Last", "All"} <= shown
    assert "Make Group" not in shown


def test_the_filter_still_finds_a_name_and_a_key(sheet):
    for query in ("Make Group", "Ctrl+G", settings_sheet._keys_text(["Ctrl+G"])):
        sheet.state.query = query
        assert "sc:menu:make_group" in rows(sheet)
    sheet.state.query = "no such command anywhere"
    assert not rows(sheet)


def test_the_new_keys_show_in_their_column_and_the_menu_stays(sheet):
    command = "sc:menu:unhide_all"
    keys = ["Ctrl+Alt+Shift+U"]
    sheet._assign(sheet._action(command), keys)
    row = rows(sheet)[command]
    assert row["sub"] == "Edit ▸ Unhide"
    assert row["right"] == settings_sheet._keys_text(keys)
    assert settings().value("shortcuts/menu:unhide_all") == keys[0]


@pytest.mark.parametrize("lang,word", [("es", "Menú"), ("ja", "メニュー"), ("ar", "القائمة")])
def test_the_menu_detail_heading_is_translated(sheet, lang, word):
    old = current_language()
    try:
        set_language(lang)
        sheet.state.kb_focus = "sc:menu:unhide_all"
        detail = sheet._keyboard_snapshot()["detail"]
        assert next(block["rows"] for block in detail if block["kind"] == "kv")[0][0] == word
    finally:
        set_language(old)


def test_every_language_has_the_column_heading():
    root = Path(__file__).resolve().parent.parent / "i18n"
    for path in root.glob("*.json"):
        if path.stem != "en":
            data = json.loads(path.read_text(encoding="utf-8"))
            assert data.get("Menu"), path.name

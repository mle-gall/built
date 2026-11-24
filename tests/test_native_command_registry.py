"""The native Keyboard table edits the same bindings the window dispatches."""
from PySide6.QtCore import Qt
from core.settings import settings
from PySide6.QtGui import QKeySequence
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_tools_views_and_menus_reassign_and_reset_across_windows(tmp_path, monkeypatch):
    from app_shell.host import Host
    from ingetrazo_render import settings_sheet
    from views.shortcuts import action_key
    from ingetrazo_render import view_commands

    profile = str(tmp_path / "profile")
    hosts = [Host(0, 900, 700, 1.0, settings=profile)]
    try:
        win = hosts[0].window
        hosts.append(Host(0, 900, 700, 1.0, settings=profile))
        sheet = settings_sheet.SettingsSheet("keyboard")
        sheet._win = win
        keys = {action_key(a): a for a in sheet.actions()}
        assert set(win._registry_actions) <= keys.keys()
        assert {"view:" + row[1] for row in view_commands()} <= keys.keys()
        tool, view, menu = (keys[k] for k in ("tool:line", "view:orbit", "menu:cutlist"))
        forms = []
        monkeypatch.setattr(settings_sheet.cf, "show", lambda _, form: forms.append(form))
        sheet.state.kb_focus = "sc:tool:line"
        sheet._kb_verb("kb_change")
        form = forms.pop()
        form.receive("keys", "F9")
        form.press("ok")
        assert tool.portable_keys() == ["F9"]
        for action, previous in [(view, tool), (menu, view)]:
            sheet._assign(action, [QKeySequence("F9")])
            forms.pop().press("cancel")
            assert previous.shortcut() == QKeySequence("F9")
            sheet._assign(action, [QKeySequence("F9")])
            forms.pop().press("ok")
            assert not previous.shortcuts()
            assert hosts[1].window._registry_actions[action_key(action)].shortcut() == QKeySequence("F9")
        # A new host reads the saved override before receiving the first key.
        hosts.append(Host(0, 900, 700, 1.0, settings=profile))
        fresh = hosts[-1]
        seen = []
        monkeypatch.setattr(fresh.window, "run_command", lambda command: seen.append(command))
        assert fresh._shortcut(Qt.Key_F9, 0)
        assert seen == ["cutlist"]
        sheet.state.kb_focus = "sc:menu:cutlist"
        sheet._kb_verb("kb_default")
        assert not settings().contains("shortcuts/menu:cutlist")
        assert not fresh._shortcut(Qt.Key_F9, 0)
        sheet.state.kb_focus = "sc:view:orbit"
        sheet._kb_verb("kb_default")
        assert fresh.window._registry_actions["view:orbit"].portable_keys() == ["O"]
        sheet._assign(tool, [QKeySequence("F9")])
        sheet._kb_verb("kb_reset_all")
        forms.pop().press("ok")
        assert all(h.window._registry_actions["tool:line"].shortcut() == QKeySequence("L") for h in hosts)
        assert not fresh._shortcut(Qt.Key_F9, 0)
    finally:
        for h in reversed(hosts):
            h.close()


def test_keyboard_prompt_roundtrips_platform_text_and_reserved_keys():
    from ingetrazo_render import shortcut_alternatives, shortcut_text
    from ingetrazo_render.settings_sheet import _keys_text, _parse_keys

    for source in ("⌥⇧⌘É", "Ctrl+Alt+Shift+É"):
        assert _parse_keys(source) == ["Ctrl+Alt+Shift+É"]
        assert _parse_keys(_keys_text(_parse_keys(source))) == ["Ctrl+Alt+Shift+É"]
    assert _parse_keys("⌘,") == ["Ctrl+,"]
    assert _parse_keys("") == []
    assert isinstance(_parse_keys("Ctrl+K, Ctrl+C"), str)
    assert isinstance(_parse_keys("⇥"), str)
    assert shortcut_text("Ctrl+⇞")[2] == Qt.Key_PageUp | Qt.ControlModifier.value
    assert shortcut_alternatives("Alt+;; Shift+F9; Shift+F9; Down", True) == ["Alt+;", "Shift+F9"]

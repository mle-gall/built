# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Command search (F3): a box that filters the window's own actions and
runs the one picked — Blender's F3, Rhino's prompt."""
from __future__ import annotations

from unittest.mock import Mock

import pytest

from app_shell.command_action import ShellAction
from core import string_search as ss
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import command_metadata as cs, wgpu_ui


@pytest.fixture
def win(native_host):
    return native_host.window


# ---- the matching (Blender's), without a window ------------------------------
def _rank(query, *labels):
    entries = [(label, [ss.Item(label.split(" ▸ "))]) for label in labels]
    return ss.rank(query, entries)


def test_command_index_preserves_group_boundaries_and_scores():
    item = ss.Item(["Módelo", "", "Reverse-Faces"])
    assert ss.words_of("Módelo ▸ Reverse-Faces") == list(item.words)
    assert item.words == ("modelo", "reverse", "faces")
    assert item.main == (False, True, True)
    assert (item.main_length, item.total_length) == (12, 19)
    assert ss.score(["rev"], item) == 1010
    assert ss.score(["mod"], item) == 1009
    assert ss.score(["missing"], item) is None
    assert ss.fuzzy_errors("revs", "revs") == 0
    assert ss.fuzzy_errors("os", "") == -1


def test_every_word_typed_must_be_found():
    assert _rank("orient", "Orient Faces") == ["Orient Faces"]
    assert _rank("faces orient", "Orient Faces") == ["Orient Faces"]
    assert _rank("orient walls", "Orient Faces") == []


def test_accents_and_case_do_not_matter():
    assert _rank("rotacion", "Rotación") == ["Rotación"]
    assert _rank("ROTACIÓN", "rotacion") == ["rotacion"]


def test_initials_find_a_command():
    """Blender's «seboulo» → Select Boundary Loop."""
    assert _rank("rf", "Orient Faces", "Reverse Faces")[0] == "Reverse Faces"
    assert _rank("seboulo", "Select Boundary Loop") == [
        "Select Boundary Loop"]


def test_a_typing_error_is_forgiven():
    assert _rank("orinet", "Orient Faces", "Reverse Faces") == [
        "Orient Faces"]
    assert _rank("fcaes", "Orient Faces") == ["Orient Faces"]


def test_a_word_beginning_beats_initials_and_typos():
    assert _rank("rev", "Remove Everything", "Reverse Faces")[0] ==         "Reverse Faces"


def test_the_name_counts_more_than_the_path():
    assert _rank("faces", "Faces ▸ Something", "Edit ▸ Orient Faces")[0]         == "Edit ▸ Orient Faces"


def test_the_shortest_name_comes_first_among_equals():
    assert _rank("line", "Line Style", "Line")[0] == "Line"


def test_words_out_of_order_rank_lower():
    assert _rank("faces orient", "Faces Orient", "Orient Faces")[0] ==         "Faces Orient"


def test_recent_ones_lead_only_with_one_letter_or_none():
    entries = [(n, [ss.Item([n])]) for n in ("Copy", "Cut", "Cutter")]
    recent = {"Cutter": 2}.get
    assert ss.rank("", entries, lambda n: recent(n, -1))[0] == "Cutter"
    assert ss.rank("c", entries, lambda n: recent(n, -1))[0] == "Cutter"
    assert ss.rank("cut", entries, lambda n: recent(n, -1))[0] == "Cut"


def test_damerau_levenshtein():
    assert ss.damerau_levenshtein("abc", "abc") == 0
    assert ss.damerau_levenshtein("abc", "acb") == 1      # a swap
    assert ss.damerau_levenshtein("kitten", "sitting") == 3


def query(win, text="", scope=""):
    view = win.view
    renderer = Mock()
    from unittest.mock import patch
    with patch.object(view, "renderer", renderer):
        wgpu_ui.command_action(view, win, "cmdscope", scope)
        wgpu_ui.command_action(view, win, "cmdq", text)
    return renderer.ui_set_command_results.call_args.args[0], renderer.ui_set_command_details.call_args.args[0]


def test_model_search_lists_menu_commands_and_tools_and_prioritizes_recent_actions(win):
    rows, _ = query(win)
    names = [row[0] for row in rows]
    assert {"Orient Faces", "Line", "Push / Pull", "Settings…"} <= set(names)
    assert "Search commands…" not in names and "Chain dimension" not in names
    rows, _ = query(win, "unhide all")
    assert rows[0][:2] == ("All", "Edit ▸ Unhide")
    scoped, _ = query(win, "a", "Edit ▸ Unhide")
    assert scoped and all(r[1] == "Edit ▸ Unhide" for r in scoped)
    assert query(win, "xqzwv")[0] == []
    action = ShellAction(win, "Frobnicate the walls", run=Mock())
    rows, _ = query(win, "frobnic")
    assert rows == [("Frobnicate the walls", "", "", True)]
    wgpu_ui.command_action(win.view, win, "cmdrun", "0")
    action.run.assert_called_once()
    assert query(win)[0][0][0] == "Frobnicate the walls"
    action.remove()
    assert query(win, "frobnic")[0] == []


def test_a_plugin_can_hide_its_action_after_the_native_search_opens(win):
    from PySide6.QtGui import QAction
    action = QAction("Frobnicate the active sheet", win)
    ran = Mock()
    action.triggered.connect(ran)
    try:
        assert query(win, "frobnic")[0][0][0] == action.text()
        action.setVisible(False)
        wgpu_ui.command_action(win.view, win, "cmdrun", "0")
        ran.assert_not_called()
        assert not query(win, "frobnic")[0]
        action.setVisible(True)
        assert query(win, "frobnic")[0]
    finally:
        action.setParent(None)


def test_search_finds_translated_names_and_paths_by_either_language(tmp_path):
    from app_shell.host import Host
    from core import i18n
    from ingetrazo_core import NativeSettings
    profile = str(tmp_path / "profile")
    NativeSettings(profile).setValue("language", "es")
    original = i18n.current_language()
    host = Host(0, 900, 600, 1., settings=profile)
    try:
        win = host.window
        for term in ("orient faces", "orientar"):
            assert query(win, term)[0][0][0] == "Orientar caras"
        for term in ("edit all", "edicion mostrar todo"):
            assert "Todo" in [r[0] for r in query(win, term)[0]]
    finally:
        host.close()
        i18n.set_language(original)


def test_action_cache_tracks_live_name_shortcut_and_removal(win):
    action = ShellAction(win, "Frobnicate the walls")
    original = {c.action: c for c in cs.commands(win)}
    assert all(original[c.action] is c for c in cs.commands(win))
    action.setText("Frobnicate the floors")
    updated, = cs.search(win, "frobnicate")
    assert updated is not original[action] and updated.name == "Frobnicate the floors"
    action.setShortcuts(["Ctrl+Alt+F"])
    keyed, = cs.search(win, "frobnicate")
    assert keyed is not updated and keyed.keys
    action.remove()
    assert not cs.search(win, "frobnicate") and action not in win._command_cache


def test_unavailable_action_cannot_run_from_a_previous_search(win):
    run = Mock()
    action = ShellAction(win, "Frobnicate", run=run)
    assert query(win, "frobnicate")[0][0][3]
    action.setEnabled(False)
    wgpu_ui.command_action(win.view, win, "cmdrun", "0")
    assert not query(win, "frobnicate")[0][0][3]
    action.remove()
    wgpu_ui.command_action(win.view, win, "cmdrun", "0")
    run.assert_not_called()


def test_shortcut_editor_opens_on_the_searched_command(win):
    from tests.scene_controls import chrome
    from views import chrome_sheet
    with chrome(win):
        rows, detail = query(win, "orient faces")
        assert rows[0][:2] == ("Orient Faces", "Edit") and detail[0][2]
        action = win.view._ui_cmd_hits[0]
        wgpu_ui.command_action(win.view, win, "cmdshortcut", "0")
        sheet = chrome_sheet.current(win)
        assert sheet.section == "keyboard" and sheet._action(sheet.kb_focus) is action
        chrome_sheet.close(win)


def test_sheet_search_lists_and_runs_only_the_current_workspaces_commands(win, monkeypatch):
    from views import wgpu_sheets
    from tests.scene_controls import chrome
    host = wgpu_sheets.composer_for(win)
    ws = host._ws
    ws.shown, ws.view = True, win.view
    monkeypatch.setattr(win.view, "_sheets_ws", ws, raising=False)
    with chrome(win) as renderer:
        rows, tips = query(win, "chain dimension")
        assert rows[0][:2] == ("Chain dimension", "Dimensions")
        assert tips[0][0] and tips[0][2]
        wgpu_ui.command_action(win.view, win, "cmdrun", "0")
        assert host.tool_mode == "cota_cadena"
        rows, _ = query(win, "ungroup")
        from ingetrazo_render import shortcut_text
        assert rows[0][:3] == ("Ungroup", "Sheet", shortcut_text("Ctrl+Shift+G")[1])
        wgpu_ui.command_action(win.view, win, "cmdrun", "0")
        renderer.page_command.assert_called_once_with("ungroup")
        assert not query(win, "orient faces")[0]
    ws.shown = False
    assert not query(win, "chain dimension")[0] and query(win, "orient faces")[0]


def test_search_and_the_tool_share_descriptions_and_model_commands_can_repeat(win):
    rows, detail = query(win, "line")
    assert rows[0][:2] == ("Line", "Tools") and rows[0][2] == "L"
    assert detail[0][0] == win._tools["line"].description
    wgpu_ui.command_action(win.view, win, "cmdrun", "0")
    assert win.viewport.active_tool is win._tools["line"]
    win._activate_tool("select")
    assert win.repeat_last_command() and win.viewport.active_tool is win._tools["line"]
    assert not [c.key for c in cs.commands(win) if c.action.isEnabled() and not c.tip]


def test_description_drops_repeated_shortcuts_and_respects_non_key_brackets(win):
    action = ShellAction(win, "Orbit")
    action.setProperty("statusTip", "Orbit  (O) — left-drag to rotate the view")
    assert cs.Command(action, "").tip == "Left-drag to rotate the view"
    action.setText("Line")
    action.setProperty("statusTip", "Line  (L)")
    assert cs.Command(action, "").tip == ""
    action.setText("Pan")
    action.setProperty("statusTip", "Pan the sheet (or drag with the middle button)")
    assert cs.Command(action, "").tip.startswith("Pan the sheet")
    for label in ("AI Bridge (MCP)", "Português (Brasil)", "IFC (BIM)…", "Rebuild Faces (Planar)"):
        assert cs._keys_in(label) is None


def test_f3_runs_a_sheet_tool_and_returns_to_modelling_in_a_release_window(tmp_path):
    import json
    from pathlib import Path

    from scripts.isolated_launch import run

    root = Path(__file__).resolve().parents[1]
    binary = root / "rust/built_app/target/release/built"
    if not binary.is_file():
        pytest.skip("requires the release shell")
    profile = tmp_path / "profile"
    probe = profile / "data/ingetrazo/plugins/sheet_search_probe.py"
    probe.parent.mkdir(parents=True)
    probe.write_text("""import json
from pathlib import Path

def setup(app):
    from views import wgpu_sheets
    def record():
        win = app.window
        ws = wgpu_sheets.workspace_of(win.view)
        with Path(__file__).with_suffix(".jsonl").open("a") as out:
            print(json.dumps([ws.composer.tool_mode if ws else "model",
                              win.view.renderer.ui_claims_key("F5", 0)]), file=out)
    app.add_menu_action("Open Sheets", lambda: wgpu_sheets.to_sheets(app.window))
    app.add_menu_action("Read current workspace", record)
""")
    actions = ["menu extension:sheet_search_probe:0", "key f3", "type chain dimension",
               "menu extension:sheet_search_probe:1", "key enter", "menu extension:sheet_search_probe:1",
               "key f3", "type back to the model", "key enter", "menu extension:sheet_search_probe:1",
               "state", "quit"]
    steps = tmp_path / "steps"
    steps.write_text("wait 550\n" + "\nwait 220\n".join(actions) + "\n")
    result = run([binary, "--root", root, "--venv", root / ".venv", "--new-instance",
                  "--script", steps, root / "tests/fixtures/shell/slab-and-cube.igz"],
                 settings=profile, capture_output=True, text=True, timeout=60, check=True)
    states = [json.loads(line) for line in probe.with_suffix(".jsonl").read_text().splitlines()]
    assert len(states) == 3
    assert states == [["select", True], ["cota_cadena", False], ["model", False]]
    state = next(json.loads(s.removeprefix("[built] state ")) for s in result.stdout.splitlines()
                 if s.startswith("[built] state "))
    assert state["errors"] == 0 and state["qt_modules"] == []


def test_sheet_binding_shares_the_keyboard_table_and_follows_the_active_document(win, monkeypatch):
    from core.input_types import Qt
    from core.shortcuts import export_shortcuts, import_shortcuts
    from app_shell.events import KeyEvent
    from tests.scene_controls import chrome
    from views import wgpu_sheets
    from views.settings_sheet import SettingsSheet

    sheet = SettingsSheet("keyboard")
    sheet._win = win
    edit = sheet._action("sc:sheet:cota_cadena")
    assert edit is not None and win._composer is None
    model = sheet._action("sc:tool:line")
    assert model.portable_keys() == ["L"]
    sheet._assign(edit, ["L"])
    assert model.portable_keys() == ["L"]
    data = export_shortcuts(win)
    assert data["shortcuts"]["tool:line"] == data["shortcuts"]["sheet:cota_cadena"] == "L"
    assert import_shortcuts(win, {"format": data["format"], "shortcuts": {
        "sheet:cota_cadena": "F8", "tool:line": "L"}})[:2] == (2, 0)
    for _ in range(2):
        ws = wgpu_sheets.composer_for(win)._ws
        ws.shown, ws.view = True, win.view
        monkeypatch.setattr(win.view, "_sheets_ws", ws, raising=False)
        with chrome(win):
            ws.key(KeyEvent(True, Qt.Key_F8, 0, "", False), True)
            assert ws.composer.tool_mode == "cota_cadena"
            query(win, "chain dimension")
            wgpu_ui.command_action(win.view, win, "cmdshortcut", "0")
            from views import chrome_sheet
            assert chrome_sheet.current(win).kb_focus == "sc:sheet:cota_cadena"
            chrome_sheet.close(win)
        ws.shown = False
        win.new_document()
    sheet.state.kb_focus = "sc:sheet:cota_cadena"
    sheet._kb_verb("kb_default")
    assert not edit.portable_keys()

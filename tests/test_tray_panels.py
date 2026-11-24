# SPDX-License-Identifier: GPL-3.0-or-later
"""On-demand AI plugins publish their panel contributions and commands."""
from types import SimpleNamespace
from unittest.mock import Mock

import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.fixture
def contributions(native_host):
    from plugins import ai_assistant, ai_bridge
    app = SimpleNamespace(viewport=native_host.viewport, window=native_host.window,
                          add_panel=Mock(), add_menu_action=Mock(), show_panel=Mock())
    ai_assistant.setup(app)
    ai_bridge.setup(app)
    yield app
    app.window._ai_bridge_section.stop()
    for call in app.add_panel.call_args_list:
        call.args[1].close()
        call.args[1].deleteLater()


def test_the_assistant_and_bridge_register_for_the_same_panel(contributions):
    calls = contributions.add_panel.call_args_list
    assert [c.kwargs["panel"] for c in calls] == ["ai", "ai"]
    assert [c.args[1] for c in calls] == [contributions.window._ai_assistant,
                                          contributions.window._ai_bridge_section]


def test_assistant_shortcut_reopens_its_panel(contributions, monkeypatch):
    title, summon, key, _ = contributions.add_menu_action.call_args_list[0].args
    assert title == "AI Assistant" and key == "Ctrl+Shift+A"
    focus = Mock()
    monkeypatch.setattr(contributions.window._ai_assistant, "focus_input", focus)
    summon()
    contributions.show_panel.assert_called_once_with(contributions.add_panel.return_value)
    focus.assert_called_once()


def test_the_bridge_command_starts_and_stops(contributions, monkeypatch):
    monkeypatch.setenv("INGETRAZO_AI_PORT", "0")
    section = contributions.window._ai_bridge_section
    assert not section.running and section._text.isHidden()
    title, summon = contributions.add_menu_action.call_args_list[1].args
    assert title == "AI Bridge (MCP)"
    summon()
    try:
        assert section.running and section.section.header.isChecked()
        port = contributions.window._ai_bridge.port
        assert f"127.0.0.1:{port}" in section._text.toPlainText()
        assert not section._text.isHidden()
    finally:
        section.stop()
    assert not section.running and section._text.isHidden()


def test_the_prompt_takes_several_lines_and_enter_sends():
    from PySide6.QtCore import Qt
    from PySide6.QtGui import QKeyEvent
    from PySide6.QtCore import QEvent
    from plugins.ai_assistant import PromptEdit
    box = PromptEdit()
    sent = []
    box.submitted.connect(lambda: sent.append(box.text()))
    box.setText("una casa")
    box.moveCursor(box.textCursor().MoveOperation.End)
    shift = QKeyEvent(QEvent.KeyPress, Qt.Key_Return, Qt.ShiftModifier)
    box.keyPressEvent(shift)
    box.insertPlainText("con techo a dos aguas")
    assert box.text() == "una casa\ncon techo a dos aguas" and not sent
    box.keyPressEvent(QKeyEvent(QEvent.KeyPress, Qt.Key_Return,
                                Qt.NoModifier))
    assert sent == ["una casa\ncon techo a dos aguas"]
    assert box.minimumHeight() >= 2 * box.fontMetrics().lineSpacing()

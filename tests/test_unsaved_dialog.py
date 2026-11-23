# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Unsaved-document actions and translations in the chrome question."""
from __future__ import annotations

from PySide6.QtWidgets import QApplication

_app = QApplication.instance() or QApplication([])


def test_the_unsaved_changes_sheet_offers_discard_cancel_and_save(monkeypatch):
    from views.main_window import MainWindow
    seen = []

    def answer(win, title, text, buttons, **roles):
        seen.append(([key for key, _ in buttons], roles))
        return "discard"

    monkeypatch.setattr("views.chrome_dialogs.ask", answer)
    win = MainWindow()
    win.viewport.scene.version += 1               # something unsaved
    assert win._confirm_discard("Quit Built?") is True
    assert seen == [(["discard", "cancel", "save"], dict(default="save", cancel="cancel", discard="discard"))]
    win._saved_version = win.viewport.scene.version


def test_the_buttons_speak_the_app_language(monkeypatch):
    from core import i18n
    from views.main_window import MainWindow
    seen = []

    def answer(win, title, text, buttons, **roles):
        seen.extend(label for key, label in buttons)
        return "cancel"

    monkeypatch.setattr("views.chrome_dialogs.ask", answer)
    before = i18n.current_language()
    i18n.set_language("es")
    try:
        win = MainWindow()
        win.viewport.scene.version += 1
        assert win._confirm_discard("¿Salir de Built?") is False
    finally:
        i18n.set_language(before)
    assert seen == ["No guardar", "Cancelar", "Guardar"]
    win._saved_version = win.viewport.scene.version

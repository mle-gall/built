# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Unsaved-document actions and translations in the chrome question."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_the_unsaved_changes_sheet_offers_discard_cancel_and_save(native_host, monkeypatch):
    seen = []

    def answer(win, title, text, buttons, **roles):
        seen.append(([key for key, _ in buttons], roles))
        return "discard"

    monkeypatch.setattr("built_render.chrome_dialogs.ask", answer)
    win = native_host.window
    win.viewport.scene.version += 1               # something unsaved
    assert win._confirm_discard("Quit Built?") is True
    assert seen == [(["discard", "cancel", "save"], dict(default="save", cancel="cancel", discard="discard"))]


def test_the_buttons_speak_the_app_language(native_host, monkeypatch):
    from built_core import i18n
    seen = []

    def answer(win, title, text, buttons, **roles):
        seen.extend(label for key, label in buttons)
        return "cancel"

    monkeypatch.setattr("built_render.chrome_dialogs.ask", answer)
    before = i18n.current_language()
    i18n.set_language("es")
    try:
        win = native_host.window
        win.viewport.scene.version += 1
        assert win._confirm_discard("¿Salir de Built?") is False
    finally:
        i18n.set_language(before)
    assert seen == ["No guardar", "Cancelar", "Guardar"]

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The session's QApplication wears the app's style from the start, as the
app does: a later setStyle walks every widget alive and, mid-batch, met one
whose C++ half was gone (SIGSEGV in QApplication::setStyle, 12-11)."""
from PySide6.QtWidgets import QApplication

from views import theme


def test_the_style_is_already_the_apps():
    assert isinstance(QApplication.instance().style(), theme.ChromeStyle)


def test_startup_is_idempotent(monkeypatch):
    app = QApplication.instance()
    calls = []
    for setter in ("setStyle", "setPalette", "setStyleSheet"):
        monkeypatch.setattr(app, setter, calls.append)
    theme.install_dark_palette(app)
    assert calls == []

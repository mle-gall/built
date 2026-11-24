# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Qt's own standard buttons (OK, Cancel, Yes, No…) follow the app language.

Our catalog only covers ``tr()``; those buttons come from Qt's
``qtbase_<lang>.qm``, and without it they stayed in English. The keys in
the shortcuts stay in English on purpose: only the buttons are taken."""
from __future__ import annotations

from plugins import _widget_language as widget_language

import pytest
from PySide6.QtGui import QColor, QKeySequence
from PySide6.QtWidgets import QApplication, QColorDialog, QLabel, QMessageBox

_app = QApplication.instance() or QApplication([])


@pytest.mark.parametrize("lang, yes, cancel", [
    ("es", "&Sí", "Cancelar"),
    ("pt-BR", "&Sim", "Cancelar"),
    ("vi", "&Có", "Hủy"),
    ("ja", "はい(&Y)", "キャンセル"),
])
def test_standard_buttons_are_translated(monkeypatch, lang, yes, cancel):
    monkeypatch.setattr(widget_language, "_qt_translator", None)
    widget_language._install_qt_translator(lang)
    try:
        assert widget_language._qt_translator is not None
        box = QMessageBox(QMessageBox.Question, "", "",
                          QMessageBox.Yes | QMessageBox.Cancel)
        assert box.button(QMessageBox.Yes).text() == yes
        assert box.button(QMessageBox.Cancel).text() == cancel
    finally:
        QApplication.removeTranslator(widget_language._qt_translator)


def test_english_installs_nothing(monkeypatch):
    monkeypatch.setattr(widget_language, "_qt_translator", None)
    widget_language._install_qt_translator("en")
    assert widget_language._qt_translator is None


def test_colour_picker_labels_are_translated(monkeypatch):
    # #1178: the interface in another language still showed Qt's own
    # colour picker in English («Basic colors»…) — its strings live in
    # the QColorDialog context, which the buttons-only filter shut out.
    monkeypatch.setattr(widget_language, "_qt_translator", None)
    widget_language._install_qt_translator("es")
    try:
        dlg = QColorDialog(QColor(10, 20, 30))
        dlg.setOption(QColorDialog.DontUseNativeDialog, True)
        labels = {w.text() for w in dlg.findChildren(QLabel)}
        assert "Colores &básicos" in labels        # «Basic colors»
        assert "&Colores personalizados" in labels  # «Custom colors»
        assert "&Basic colors" not in labels
    finally:
        QApplication.removeTranslator(widget_language._qt_translator)


@pytest.mark.parametrize("lang", ["es", "ja"])
def test_shortcuts_keep_their_english_key_names(monkeypatch, lang):
    monkeypatch.setattr(widget_language, "_qt_translator", None)
    sequences = [QKeySequence("Ctrl+Shift+PgUp"), QKeySequence("Esc")]
    native = [seq.toString(QKeySequence.NativeText) for seq in sequences]
    widget_language._install_qt_translator(lang)
    try:
        assert [seq.toString(QKeySequence.NativeText) for seq in sequences] == native
        assert [seq.toString(QKeySequence.PortableText) for seq in sequences] == [
            "Ctrl+Shift+PgUp", "Esc"]
    finally:
        QApplication.removeTranslator(widget_language._qt_translator)

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The classic window's status bar: its messages in a label of its own.

A plain QStatusBar hides its normal widgets whenever a temporary message
shows and leaves the bar empty after a timed one. ``HintStatusBar`` keeps a
standing text — ``showMessage(text)`` with no timeout — that a timed
message replaces for a while and then gives back, and shows a menu entry's
description while it is hovered. It was the status bar half of
views/sheet_tabs.py, whose Model | sheets strip went with the sheet
composer (docs/rust-sheets.md); the Qt host's retirement takes it."""
from __future__ import annotations

from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QFontMetrics
from PySide6.QtWidgets import QLabel, QSizePolicy, QStatusBar, QWidget


class _ElidedLabel(QLabel):
    """A label that never asks for room: a plain QLabel's minimum width is
    its whole text, and the standing hint is a long line — as a permanent
    widget it made the window impossible to shrink or maximise onto a
    smaller screen. This one reports no minimum and elides."""

    def __init__(self, parent=None) -> None:
        super().__init__("", parent)
        self._full = ""
        self.setSizePolicy(QSizePolicy.Ignored, QSizePolicy.Preferred)
        self.setMinimumWidth(0)
        self.setTextInteractionFlags(Qt.NoTextInteraction)

    def setText(self, text: str) -> None:  # noqa: N802
        self._full = str(text or "")
        self._relayout()

    def text(self) -> str:
        return self._full

    def resizeEvent(self, event) -> None:  # noqa: N802
        super().resizeEvent(event)
        self._relayout()

    def _relayout(self) -> None:
        fm = QFontMetrics(self.font())
        shown = fm.elidedText(self._full, Qt.ElideRight, max(self.width() - 4, 0))
        super().setText(shown)
        # Cut short, the whole line is one hover away.
        self.setToolTip(self._full if shown != self._full else "")


class HintStatusBar(QStatusBar):
    """See the module docstring."""

    #: The message never takes more than this share of the bar, so the
    #: right end stays clear for the coordinates and the VCB.
    MESSAGE_SHARE = 0.6

    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self._msg = _ElidedLabel(self)
        self._base = ""
        self._timer = QTimer(self)
        self._timer.setSingleShot(True)
        self._timer.timeout.connect(self._restore)
        self.addPermanentWidget(self._msg, 10)
        # An empty filler takes what the capped message leaves, so the
        # message stays at the left whatever the windows add on the right.
        self._filler = QWidget(self)
        self.addPermanentWidget(self._filler, 1)
        # A menu entry's description reaches the bar through Qt's C++
        # showMessage, which never calls the override below: Qt announces
        # it, and the announcement goes to our label.
        self.messageChanged.connect(self._on_qt_message)

    def resizeEvent(self, event) -> None:  # noqa: N802
        super().resizeEvent(event)
        self._msg.setMaximumWidth(max(120, int(self.width() * self.MESSAGE_SHARE)))

    def showMessage(self, text: str, timeout: int = 0) -> None:  # noqa: N802
        text = str(text or "")
        if timeout and timeout > 0:
            self._msg.setText(text)
            self._timer.start(int(timeout))
        else:
            self._timer.stop()
            self._base = text
            self._msg.setText(text)

    def clearMessage(self) -> None:  # noqa: N802
        self._timer.stop()
        self._msg.setText(self._base)

    def currentMessage(self) -> str:  # noqa: N802
        return self._msg.text()

    def _restore(self) -> None:
        self._msg.setText(self._base)

    def _on_qt_message(self, text: str) -> None:
        """A status tip (a menu entry being hovered) while it lasts; the
        standing text again when it clears."""
        if text:
            self._timer.stop()
            self._msg.setText(text)
        else:
            self._msg.setText(self._base)

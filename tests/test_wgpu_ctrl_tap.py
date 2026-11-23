# SPDX-License-Identifier: GPL-3.0-or-later
"""The Metal view records whether a Ctrl came and went alone (#183): tools
that toggle on a Ctrl tap (Move's copy, Rectangle's centre) ask
``ctrl_tapped`` on the release. Only the controller's own key handler
recorded it, so on this view every tap read as a chord and was taken back."""
from PySide6.QtCore import QEvent, Qt
from PySide6.QtGui import QKeyEvent

from tests.test_.skp_inference_gaps import view  # noqa: F401  (fixture)


def _key(kind, key, mods=Qt.NoModifier):
    return QKeyEvent(kind, key, mods)


def test_a_lone_ctrl_is_a_tap_and_a_chord_is_not(view):  # noqa: F811
    source = view.source_viewport
    view.keyPressEvent(_key(QEvent.KeyPress, Qt.Key_Control))
    assert source.ctrl_tapped()
    view.keyPressEvent(_key(QEvent.KeyPress, Qt.Key_Z, Qt.ControlModifier))
    assert not source.ctrl_tapped()
    view.keyPressEvent(_key(QEvent.KeyPress, Qt.Key_Control))
    view.event(_key(QEvent.ShortcutOverride, Qt.Key_C, Qt.ControlModifier))
    assert not source.ctrl_tapped()

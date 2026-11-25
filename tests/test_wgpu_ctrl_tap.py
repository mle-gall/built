# SPDX-License-Identifier: GPL-3.0-or-later
"""The Metal view records whether a Ctrl came and went alone (#183): tools
that toggle on a Ctrl tap (Move's copy, Rectangle's centre) ask
``ctrl_tapped`` on the release. Only the controller's own key handler
recorded it, so on this view every tap read as a chord and was taken back."""
from built_render import KeyEvent, ShortcutOverride

from built_core import Key, KeyboardModifier
from tests.headless_gpu import _code
from tests.test_inference_and_face_cutting import view  # noqa: F401  (fixture)

CTRL = KeyboardModifier.ControlModifier


def _press(key, mods=0):
    return KeyEvent(True, _code(key), _code(mods))


def test_a_lone_ctrl_is_a_tap_and_a_chord_is_not(view):  # noqa: F811
    source = view.source_viewport
    view.keyPressEvent(_press(Key.Key_Control))
    assert source.ctrl_tapped()
    view.keyPressEvent(_press(Key.Key_Z, CTRL))
    assert not source.ctrl_tapped()
    view.keyPressEvent(_press(Key.Key_Control))
    view.event(ShortcutOverride(_code(Key.Key_C), _code(CTRL)))
    assert not source.ctrl_tapped()

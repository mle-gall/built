# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Keyboard shortcuts of one's own (issue #138, @pacaeiro: «right now
there's no way to configure our own shortcuts»).

Every action of the main window — menus and tools — can take the keys the
user wants. They are remembered per action in QSettings (``shortcuts/<key>``)
and put back at start-up over the factory ones. The key of an action is its
object name or, failing that, its ENGLISH text: the menus are translated, and
a shortcut set in Spanish must survive a switch to Portuguese.

Two actions must never hold the same keys: Qt then fires neither («Ambiguous
shortcut overload», tests/test_shortcuts.py). The dialog takes the keys away
from the action that had them, after asking.
"""
from __future__ import annotations

from core.shortcuts import (
    _DEFAULTS, _GROUP, _plain, _read_key, _store_key, _to_text, action_key, action_keys,
    apply_user_shortcuts, collect_actions, default_keys, export_shortcuts, import_shortcuts,
    portable_keys, remember_defaults, save_shortcut, set_action_keys, EXPORT_FORMAT,
)

from PySide6.QtGui import QAction, QKeySequence
from core.i18n import tr
from ingetrazo_render import shortcut_text

_TIP = "ingetrazo_tooltip_base"


def set_tooltip(action: QAction, base: str) -> None:
    """A toolbar tooltip that names the action's CURRENT keys: «Line  (L)»,
    and under them what the action does — its status tip, when it has one.

    The keys used to be written into the text once, at start-up, so a
    shortcut changed in Preferences showed in the menus (Qt reads the
    action) and not on the toolbar buttons (issue #171). The tooltip is
    rebuilt whenever the action changes -- whoever changed the keys."""
    fresh = action.property(_TIP) is None
    action.setProperty(_TIP, base)
    if fresh:
        action.changed.connect(lambda a=action: _refresh_tooltip(a))
    _refresh_tooltip(action)


def _refresh_tooltip(action: QAction) -> None:
    base = action.property(_TIP)
    if base is None:
        return
    keys = action.shortcut().toString(QKeySequence.NativeText)
    tip = f"{base}  ({keys})" if keys else base
    if action.statusTip():
        tip += "\n" + action.statusTip()
    # setToolTip with the same text returns early, so the changed signal
    # this emits does not loop.
    action.setToolTip(tip)


def reserved_reason(seq: QKeySequence):
    parsed = shortcut_text(seq.toString(QKeySequence.PortableText))
    return tr(parsed[3]) if parsed and parsed[3] else None


def default_shortcuts(action: QAction) -> list:
    return [QKeySequence(s) for s in default_keys(action)]

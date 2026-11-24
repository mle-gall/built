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

from PySide6.QtCore import QEvent, QObject, Qt
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




# ---- Two owners of one key -------------------------------------------------
# An extension's panel that takes a key of the window's own — the Node
# Editor's Ctrl+S and Ctrl+Shift+S for its graph — made that key ambiguous
# to Qt, which then runs neither: Ctrl+S stopped saving the model for those
# who had the panel (#1334). The owner nearest the focus wins: inside the
# panel, the panel's; anywhere else, the window's.

def _owner_widgets(obj, window):
    """The widgets a QShortcut or QAction of ``window`` answers in. A
    window-wide action of the window answers in the window, whether its
    menu is open or not."""
    from PySide6.QtGui import QShortcut
    from PySide6.QtWidgets import QWidget
    if isinstance(obj, QShortcut):
        parent = obj.parent()
        return [parent] if isinstance(parent, QWidget) else []
    parent = obj.parent()
    if (obj.shortcutContext() in (Qt.WindowShortcut, Qt.ApplicationShortcut)
            and (parent is window or (isinstance(parent, QWidget)
                                      and window.isAncestorOf(parent)
                                      and parent.isVisible()))):
        return [window if parent is window else parent]
    return [o for o in obj.associatedObjects() if isinstance(o, QWidget)]


def _holds(owner, focus) -> bool:
    return owner is focus or (focus is not None and owner.isAncestorOf(focus))


def _depth(widget) -> int:
    n = 0
    while widget is not None:
        widget = widget.parentWidget()
        n += 1
    return n


def _clash_winner(window, seq, focus):
    """The QShortcut or QAction of ``window`` that key ``seq`` should run,
    or None when it has no enabled owner reachable from ``focus``."""
    from PySide6.QtGui import QAction, QShortcut
    best, best_depth = None, -1
    owners = list(window.findChildren(QShortcut)) + list(
        window.findChildren(QAction))
    for obj in owners:
        if not obj.isEnabled():
            continue
        keys = obj.keys() if isinstance(obj, QShortcut) else obj.shortcuts()
        if not any(k.matches(seq) == k.SequenceMatch.ExactMatch
                   for k in keys):
            continue
        for owner in _owner_widgets(obj, window):
            if not owner.isVisible() or owner.window() is not window:
                continue
            # The window's own answer anywhere in it; a panel's only with
            # the focus inside the panel, and there the deeper one wins.
            if owner is window:
                depth = 1
            elif _holds(owner, focus):
                depth = _depth(owner)
            else:
                continue
            if depth > best_depth:
                best, best_depth = obj, depth
    return best


class _ClashResolver(QObject):
    def eventFilter(self, obj, event):  # noqa: N802 — Qt override
        if event.type() != QEvent.Shortcut or not event.isAmbiguous():
            return False
        from PySide6.QtWidgets import QApplication
        focus = QApplication.focusWidget()
        window = focus.window() if focus is not None \
            else QApplication.activeWindow()
        if window is None:
            return False
        winner = _clash_winner(window, event.key(), focus)
        if winner is None:
            return False
        from PySide6.QtGui import QAction
        if isinstance(winner, QAction):
            winner.trigger()
        else:
            winner.activated.emit()
        return True


_resolver = None


def resolve_shortcut_clashes() -> None:
    """Install, once, the rule above for every window of the application."""
    global _resolver
    from PySide6.QtWidgets import QApplication
    app = QApplication.instance()
    if app is None or _resolver is not None:
        return
    _resolver = _ClashResolver(app)
    app.installEventFilter(_resolver)

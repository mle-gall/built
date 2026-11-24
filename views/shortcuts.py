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

from functools import lru_cache

from core.settings import settings as QSettings
from PySide6.QtCore import QEvent, QObject, Qt
from PySide6.QtGui import QAction, QKeySequence
from core.i18n import current_language, source_of, tr
from ingetrazo_render import shortcut_alternatives, shortcut_text

_GROUP = "shortcuts"
_DEFAULTS = "ingetrazo_default_shortcuts"
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


def _plain(text: str) -> str:
    return text.replace("&&", "\0").replace("&", "").replace("\0", "&") \
        .rstrip("…").rstrip(".").strip()


def action_key(action: QAction) -> str:
    name = action.objectName()
    if name:
        return name
    return _text_key(action.text(), current_language())


@lru_cache(maxsize=4096)
def _text_key(text: str, _language: str) -> str:
    # Remembered per language: F3 asks for every action's key several
    # times each time it opens (views/command_metadata.py).
    return "text:" + _plain(source_of(text))


def collect_actions(window) -> list:
    """The window's actions a user can press: with text, not a separator,
    not a submenu, one per key (the first wins)."""
    seen: dict = {}
    for act in (*vars(window).get("_command_actions", {}).values(), *window.findChildren(QAction)):
        if act.isSeparator() or act.menu() is not None:
            continue
        if not _plain(act.text()):
            continue
        seen.setdefault(action_key(act), act)
    return sorted(seen.values(), key=lambda a: _plain(a.text()).lower())


def portable_keys(seqs):
    """Adapt keys supplied by classic actions or older plugin callers."""
    return [s if isinstance(s, str) else s.toString(QKeySequence.PortableText) for s in seqs]


def action_keys(action):
    if isinstance(action, QAction):
        return portable_keys(action.shortcuts())
    return action.portable_keys()


def set_action_keys(action, keys):
    if isinstance(action, QAction):
        keys = [QKeySequence(s) for s in keys]
    action.setShortcuts(keys)


def _to_text(seqs):
    return "; ".join(filter(None, portable_keys(seqs)))


def reserved_reason(seq: QKeySequence):
    parsed = shortcut_text(seq.toString(QKeySequence.PortableText))
    return tr(parsed[3]) if parsed and parsed[3] else None


def remember_defaults(window) -> None:
    """Note each action's factory keys before applying stored overrides."""
    for act in collect_actions(window):
        if act.property(_DEFAULTS) is None:
            act.setProperty(_DEFAULTS, "; ".join(action_keys(act)))


def default_keys(action):
    return shortcut_alternatives(action.property(_DEFAULTS) or "", False)


def default_shortcuts(action: QAction) -> list:
    return [QKeySequence(s) for s in default_keys(action)]


def _store_key(key: str) -> str:
    """``key`` as a QSettings key: QSettings reads «/» as a group, so
    «text:Push / Pull» was stored as a group «text:Push » holding « Pull»
    and never read back — the key worked until the next start (issue
    #236, @zhang-922). «/», «\\» and «%» travel escaped."""
    return (key.replace("%", "%25").replace("/", "%2F")
            .replace("\\", "%5C"))


def _read_key(stored: str) -> str:
    """The action key a stored QSettings key stands for. Keys written
    before the escape come back through ``allKeys`` with their «/»."""
    import re
    return re.sub(r"%(25|2F|5C)",
                  lambda m: {"25": "%", "2F": "/", "5C": "\\"}[m.group(1)],
                  stored)


def _saved_shortcuts() -> dict:
    st = QSettings()
    st.beginGroup(_GROUP)
    saved = {_read_key(k): str(st.value(k) or "") for k in st.allKeys()}
    st.endGroup()
    return saved


def apply_user_shortcuts(window) -> int:
    """Put the remembered keys on the window's actions. Returns how many."""
    saved = _saved_shortcuts()
    n = 0
    for act in collect_actions(window):
        key = action_key(act)
        if key not in saved:
            key = act.property("legacyShortcutKey")
        if key in saved:
            set_action_keys(act, shortcut_alternatives(saved[key], True))
            n += 1
    return n


def save_shortcut(action: QAction, seqs: list) -> None:
    st = QSettings()
    key = action_key(action)
    legacy = action.property("legacyShortcutKey")
    if legacy and legacy != key:
        st.remove(f"{_GROUP}/{_store_key(legacy)}")
    st.remove(f"{_GROUP}/{key}")               # a pre-escape entry, if any
    if _to_text(seqs) == (action.property(_DEFAULTS) or ""):
        st.remove(f"{_GROUP}/{_store_key(key)}")   # back to the factory keys
    else:
        st.setValue(f"{_GROUP}/{_store_key(key)}", _to_text(seqs))
    st.sync()
    from app_shell.windows import hosts
    for host in hosts():
        peer = vars(host.window).get("_registry_actions", {}).get(key)
        if peer is not None and peer is not action:
            peer.setShortcuts(portable_keys(seqs))


#: What an exported shortcuts file says it is (issue #142).
EXPORT_FORMAT = "ingetrazo-shortcuts"


def export_shortcuts(window) -> dict:
    """Every action's keys, by its language-free key: a file made in one
    language imports in any other, on any machine (issue #142, @pacaeiro:
    «transfer shortcut configurations between computers, O.S., friends»).
    Actions with no keys are written too, so a cleared key travels."""
    return {"format": EXPORT_FORMAT, "version": 1,
            "shortcuts": {action_key(a): "; ".join(action_keys(a))
                          for a in collect_actions(window)}}


def import_shortcuts(window, data) -> tuple:
    """Put the keys of an exported file on the window's actions, and
    remember them. Actions this machine does not have (a plugin not
    installed) and reserved keys are skipped; a key the file gives to one
    action is taken from any other that held it, as assigning it by hand
    does. A key the file gives to SEVERAL actions stays with the first and
    is taken from the rest: two actions sharing a key both go dead, as Z
    did for Zoom and Zoom Selection in a file written from another
    program's keys (#323, @tomlai-git). Returns ``(applied, skipped,
    clashes)`` — ``clashes`` lists ``(key, kept_by, dropped_from)``; raises
    ValueError for a file that is not an IngeTrazo shortcuts file."""
    if (not isinstance(data, dict) or data.get("format") != EXPORT_FORMAT
            or not isinstance(data.get("shortcuts"), dict)):
        raise ValueError(tr("This is not an IngeTrazo shortcuts file."))
    wanted = {str(k): str(v or "") for k, v in data["shortcuts"].items()}
    actions = collect_actions(window)
    by_key = {action_key(a): a for a in actions}
    applied = 0
    taken: dict = {}                      # key text -> the action that has it
    clashes: list = []
    for key, text in wanted.items():
        act = by_key.get(key)
        if act is None:
            continue
        seqs = []
        for t in shortcut_alternatives(text, True):
            if t in taken:
                clashes.append((t, taken[t], act))
                continue
            taken[t] = act
            seqs.append(t)
        set_action_keys(act, seqs)
        save_shortcut(act, seqs)
        applied += 1
    for act in actions:
        if action_key(act) in wanted:
            continue
        keys = action_keys(act)
        kept = [s for s in keys if s not in taken]
        if len(kept) != len(keys):
            set_action_keys(act, kept)
            save_shortcut(act, kept)
    return applied, len(wanted) - applied, clashes


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

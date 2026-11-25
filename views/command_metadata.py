# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Shared host action metadata for search, tooltips and Keyboard settings."""
from __future__ import annotations
import re

from core.settings import settings as QSettings
from ingetrazo_render import shortcut_text

from core import string_search
from core.i18n import source_of, tr
from core.shortcuts import _plain, action_key, action_keys, collect_actions


#: The command that opens this box — kept out of its own list.
OBJECT_NAME = "command_search"
_RECENT_KEY = "command_search/recent"
_RECENT_MAX = 100
_SEP = " ▸ "


def _label(action: ShellAction) -> str:
    """The menu text without its mnemonic «&» — the «…» stays: it says
    the command asks something before doing it."""
    return action.text().replace("&&", "\0").replace("&", "") \
        .replace("\0", "&").strip()


_TRAILING = re.compile(r"\s*\(([^()]+)\)\s*$")


def _keys_in(text: str):
    """The shortcut a label carries at its end — «Ungroup (Ctrl+Shift+G)»
    — in portable spelling, or ``None`` when the brackets say something else
    («Português (Brasil)», «AI Bridge (MCP)»)."""
    m = _TRAILING.search(text)
    if m is None:
        return None
    inner = m.group(1).strip()
    seq = shortcut_text(inner)
    if seq is None:
        return None
    same = seq[0].replace(" ", "").lower()
    return seq[0] if same == inner.replace(" ", "").lower() else None


def _without_keys(text: str) -> str:
    return _TRAILING.sub("", text).strip()


def menu_paths(window) -> dict:
    """``{action_key: "Edit ▸ Unhide"}``: where each command sits in the
    window's menus, as the window recorded them (``_command_paths``)."""
    return dict(vars(window).get("_command_paths", {}))


def _belongs(action, window) -> bool:
    """Is ``action`` the window's own (not another window's)?"""
    return action.parent() is window


def recent_keys() -> list:
    value = QSettings().value(_RECENT_KEY) or []
    return [str(v) for v in ([value] if isinstance(value, str) else value)]


def remember(act: ShellAction) -> None:
    key = action_key(act)
    keys = [key] + [k for k in recent_keys() if k != key]
    QSettings().setValue(_RECENT_KEY, keys[:_RECENT_MAX])


class Command:
    """One row: the action, its words in both languages, its menu path,
    its explanation. Worked out once, when the box opens — not per key."""

    __slots__ = ("action", "key", "name", "path", "keys", "tip", "items")

    def __init__(self, action: ShellAction, path: str) -> None:
        self.action = action
        self.key = action_key(action)
        english = _plain(source_of(action.text()))
        name = _label(action)
        keys = next(iter(action_keys(action)), "")
        in_text = _keys_in(english)
        if in_text is not None:
            # «Ungroup (Ctrl+Shift+G)»: the keys go to their column.
            name, english = _without_keys(name), _without_keys(english)
            keys = keys or in_text
        self.name = name
        self.path = path
        parsed = shortcut_text(keys)
        self.keys = parsed[1] if parsed else ""
        trail = path.split(_SEP) if path else []
        self.items = (
            string_search.Item(trail + [name]),
            string_search.Item([source_of(p) for p in trail] + [english]))
        self.tip = _description(action, name)
        if not self.tip:
            from ingetrazo_render import command_description
            if description := command_description(english, self.key):
                self.tip = tr(description)


def visible(action):
    return action.isVisible()


def run(action):
    if visible(action) and action.isEnabled():
        remember(action)
        action.trigger()


def _description(action: ShellAction, name: str) -> str:
    """What the command does, from its tooltip or status tip — without
    the name and the keys a tooltip often repeats («Line  (L)», «3D Text
    — build extruded text as a solid»). Empty when that is all it says."""
    tip = (action.statusTip() or action.toolTip() or "").strip()
    if _keys_in(source_of(tip)) is not None or _keys_in(tip) is not None:
        tip = _without_keys(tip)
    fold = string_search.fold
    bare = _plain(name)
    if fold(tip).startswith(fold(bare)):
        # Only a name that stands apart — «Orbit (O) — left-drag…» — is
        # taken off; «Pan the sheet…» is a sentence and stays whole.
        m = _LEAD.match(tip[len(bare):])
        if m is not None:
            rest = tip[len(bare) + m.end():]
            tip = rest[:1].upper() + rest[1:]
    return "" if fold(_plain(tip)) == fold(bare) else tip


#: What may follow a name a tooltip repeats: its keys, then a dash or a
#: colon — or nothing at all.
_LEAD = re.compile(r"\s*(\([^()]*\))?\s*([—–:\-]\s*|$)")


def _signature(action: ShellAction, path: str) -> tuple:
    """What a Command is worked out from: when none of it changed, the
    one made at the last opening still holds."""
    return (action.objectName(), action.text(), action.toolTip(),
            action.statusTip(),
            next(iter(action_keys(action)), ""), path)


def commands(window, scope: str = "") -> list:
    """Every command the box can offer, in the window as it is now; only
    those under the menu ``scope`` («Edit ▸ Unhide») when given.

    The list is read afresh each time — a plugin loaded a minute ago is
    in it — but a command whose text, tips, keys and path are unchanged
    reuses what the last opening worked out: splitting ~200 labels in
    two languages took most of the ~100 ms F3 needed to appear."""
    if sync := getattr(type(window), "refresh_commands", None):
        sync(window)
    paths = menu_paths(window)
    cache = getattr(window, "_command_cache", None) or {}
    fresh: dict = {}
    pool = []
    for a in collect_actions(window):
        if a.objectName() == OBJECT_NAME or not a.isVisible() \
                or not _belongs(a, window):
            continue
        path = paths.get(action_key(a), "")
        sig = _signature(a, path)
        hit = cache.get(a)
        cmd = hit[1] if hit is not None and hit[0] == sig else Command(a, path)
        fresh[a] = (sig, cmd)
        pool.append(cmd)
    window._command_cache = fresh            # only the actions alive now
    if scope:
        pool = [c for c in pool
                if c.path == scope or c.path.startswith(scope + _SEP)]
    return pool


def search(window, query: str, pool=None, recent=None) -> list:
    """The commands matching ``query``, best first. With one letter typed
    or none, the recent ones lead; an empty query lists them all."""
    pool = commands(window) if pool is None else pool
    if recent is None:
        keys = recent_keys()
        recent = {k: len(keys) - i for i, k in enumerate(keys)}
    return string_search.rank(query, [(c, c.items) for c in pool],
                              recent=lambda c: recent.get(c.key, -1))

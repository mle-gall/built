"""Persistent command bindings shared by the native shell and extension actions."""
import sys
from functools import lru_cache

from core.i18n import current_language, source_of, tr
from core.settings import settings as native_settings
from ingetrazo_render import shortcut_alternatives, shortcut_domains

_GROUP = "shortcuts"
_DEFAULTS = "ingetrazo_default_shortcuts"


def _plain(text: str) -> str:
    return text.replace("&&", "\0").replace("&", "").replace("\0", "&") \
        .rstrip("…").rstrip(".").strip()


def action_key(action) -> str:
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
    for act in vars(window).get("_command_actions", {}).values():
        if act.isSeparator() or act.menu() is not None:
            continue
        if not _plain(act.text()):
            continue
        seen.setdefault(action_key(act), act)
    return sorted(seen.values(), key=lambda a: _plain(a.text()).lower())


def keyboard_scopes(window):
    from ingetrazo_render import sheet_command_scope
    if "_documents" in vars(window) and "_command_store" in vars(window):
        return (window, sheet_command_scope(window))
    return (window,)


def portable_keys(seqs):
    """Adapt keys supplied by classic actions or older plugin callers."""
    return [s if isinstance(s, str) else s.toString() for s in seqs]


def action_keys(action):
    if hasattr(action, "portable_keys"):
        return action.portable_keys()
    return portable_keys(action.shortcuts())


def set_action_keys(action, keys):
    action.setShortcuts(keys)


def _to_text(seqs):
    return "; ".join(filter(None, portable_keys(seqs)))


def remember_defaults(window) -> None:
    """Note each action's factory keys before applying stored overrides."""
    for act in collect_actions(window):
        if act.property(_DEFAULTS) is None:
            act.setProperty(_DEFAULTS, "; ".join(action_keys(act)))


def default_keys(action):
    return shortcut_alternatives(action.property(_DEFAULTS) or "", False)


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
    st = native_settings()
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


def save_shortcut(action, seqs: list) -> None:
    st = native_settings()
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
        owner = vars(host.window).get("_sheet_command_scope") if key.startswith("sheet:") else host.window
        peer = vars(owner).get("_registry_actions", {}).get(key) if owner is not None else None
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
                          for scope in keyboard_scopes(window) for a in collect_actions(scope)}}


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
    actions = [a for scope in keyboard_scopes(window) for a in collect_actions(scope)]
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
            holders = taken.setdefault(t, [])
            owner = next((a for mask, a in holders if mask & shortcut_domains(key)), None)
            if owner is not None:
                clashes.append((t, owner, act))
                continue
            holders.append((shortcut_domains(key), act))
            seqs.append(t)
        set_action_keys(act, seqs)
        save_shortcut(act, seqs)
        applied += 1
    for act in actions:
        if action_key(act) in wanted:
            continue
        keys = action_keys(act)
        domains = shortcut_domains(action_key(act))
        kept = [s for s in keys if not any(mask & domains for mask, _ in taken.get(s, ()))]
        if len(kept) != len(keys):
            set_action_keys(act, kept)
            save_shortcut(act, kept)
    return applied, len(wanted) - applied, clashes

"""The platform services of the ``built`` binary (``built_shell``, see
rust/built_app/src/native.rs), with quiet fallbacks where it is missing:
pytest and any other interpreter that is not the shell's.

Tests replace the functions here (``native.open_files = …``) to answer
for the user.
"""
from __future__ import annotations

try:  # the built-in module of the `built` binary
    import built_shell as _shell
except ImportError:  # pytest, the Qt app
    _shell = None


def available() -> bool:
    return _shell is not None


def _filters(filters):
    """``[(name, ["built", "igz"])]`` from Qt's filter string or a list."""
    if not filters:
        return []
    if isinstance(filters, str):
        out = []
        for part in filters.split(";;"):
            name, _, rest = part.partition("(")
            exts = [e.strip().lstrip("*.") for e in rest.rstrip(")").split()]
            exts = [e for e in exts if e and e != "*"]
            if exts:
                out.append((name.strip() or "Files", exts))
        return out
    return [(name, [e.lstrip("*.") for e in exts]) for name, exts in filters]


def open_files(title, filters=None, directory=None, multiple=False) -> list:
    if _shell is None:
        return []
    return list(_shell.open_files(title, _filters(filters), directory or None, bool(multiple)))


def save_file(title, name="", filters=None, directory=None):
    if _shell is None:
        return None
    return _shell.save_file(title, name or "", _filters(filters), directory or None)


def pick_folder(title, directory=None):
    if _shell is None:
        return None
    return _shell.pick_folder(title, directory or None)


def alert(title, text, buttons, tone="info") -> int:
    """A native alert (the sheet's last resort): the index of the button,
    or the last one (the cancel) when there is no shell."""
    if _shell is None:
        return max(0, len(buttons) - 1)
    return int(_shell.alert(title, text, list(buttons), tone))


def pump(view: int, timeout_ms: float = 16.0) -> list:
    if _shell is None or not view:
        return []
    return list(_shell.pump(int(view), float(timeout_ms)))


def post_key(view: int, key_code: int, chars: str, command: bool = False) -> bool:
    """A key for the window, at the front of this app's own event queue
    (scripted answers to a sheet travel the modal pump like a real one)."""
    if _shell is None or not view or not hasattr(_shell, "post_key"):
        return False
    return bool(_shell.post_key(int(view), int(key_code), chars, bool(command)))


def pasteboard_write(items) -> bool:
    if _shell is None:
        return False
    return bool(_shell.pasteboard_write([(t, bytes(d)) for t, d in items]))


def pasteboard_read(kind):
    if _shell is None:
        return None
    data = _shell.pasteboard_read(kind)
    return None if data is None else bytes(data)


def pasteboard_types():
    if _shell is None:
        return [], 0
    types, count = _shell.pasteboard_types()
    return list(types), int(count)


def spawn_self(args=()) -> bool:
    if _shell is None:
        return False
    return bool(_shell.spawn_self([str(a) for a in args]))

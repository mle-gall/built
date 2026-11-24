"""The native app's windows, all in one process (shell design: windows
rule).

Each winit window has its own :class:`app_shell.host.Host`, its own
ShellWindow (its projects) and its own renderer. They share one Python, one
Qt application, the settings, the recent list and the link sources already
read. This replaces "⇧⌘N starts another `built` process" (an Archicad-style
instance per window, which costs memory and start-up time and races on the
settings).

- **New window** (⇧⌘N): a window on the start page.
- **Move to new window** (the tab menu): the project leaves this window and
  is the only one in a new window. Its controller, view, packed scene and
  undo history move with it, and nothing is read again. Dragging a tab out
  of the bar will do the same later.
- **A file opens only once.** Opening a file already open in another window
  brings that window and its tab to the front.

Python cannot make a window. It asks: :func:`request` queues the request,
:meth:`Host.extras` hands it to Rust (``new_window``), and Rust makes the
window and its Host and calls :meth:`Host.adopt` with the token.
"""
from __future__ import annotations

import itertools
import weakref

#: The hosts of the open windows, oldest first.
_HOSTS: list = []
#: Requests waiting for a window: token → ("new", None) | ("move", (window, doc)).
_PENDING: dict = {}
_TOKENS = itertools.count(1)


def register(host) -> None:
    _HOSTS.append(weakref.ref(host))


def unregister(host) -> None:
    _HOSTS[:] = [r for r in _HOSTS if r() is not None and r() is not host]


def hosts() -> list:
    out = [r() for r in _HOSTS]
    return [h for h in out if h is not None and not getattr(h.window, "_closed", False)]


def host_of(win):
    return next((h for h in hosts() if h.window is win), None)


def find_document(path, exclude=None):
    """``(host, doc)`` of the project holding ``path`` in any window but
    ``exclude``'s, or ``(None, None)``."""
    for h in hosts():
        if h.window is exclude:
            continue
        for doc in list(h.window._documents):
            if doc.same_file(path):
                return h, doc
    return None, None


def request(host, kind="new", payload=None) -> str:
    """Ask Rust for a new window; the token comes back in :meth:`Host.adopt`."""
    token = f"w{next(_TOKENS)}"
    _PENDING[token] = (kind, payload)
    host.window_requests.append(token)
    host.request_frame()
    return token


def take(token):
    return _PENDING.pop(token, None)


def can_move(win) -> bool:
    """Whether the tab menu offers Move to new window: the native app, with
    a window able to ask for another."""
    return host_of(win) is not None and len(win._documents) > 1


def move_to_new_window(win, doc) -> str | None:
    """Ask for a window that takes ``doc`` from ``win`` (the tab menu)."""
    host = host_of(win)
    if host is None or doc not in win._documents or len(win._documents) < 2:
        return None
    from views import chrome_dialogs as cd
    if cd.modal(win):
        return None
    return request(host, "move", (win, doc))


def transfer(src_win, doc, dst_host) -> bool:
    """Move ``doc`` from ``src_win`` into ``dst_host``'s window, in front.
    The window's blank first project, if untouched, goes."""
    if doc not in src_win._documents:
        return False
    dst = dst_host.window
    blank = [d for d in dst._documents if dst._doc_is_blank(d)]
    # Leave the source window: another project comes to the front there.
    if src_win._doc is doc:
        docs = src_win._documents
        i = docs.index(doc)
        nxt = docs[i + 1] if i + 1 < len(docs) else docs[i - 1]
        src_win.activate_document(nxt, closing=True)
    src_win._documents.remove(doc)
    # The project's controller and view now answer to the new window.
    for obj in (doc.viewport, doc.stand_in):
        if obj is not None and hasattr(obj, "_host"):
            obj._host = dst_host
    view = doc.stand_in
    if view is not None:
        view.move_timers(dst_host)
        view.renderer = None
    dst._documents.append(doc)
    dst.activate_document(doc)
    for d in blank:
        if d is not doc and d in dst._documents:
            dst._retire_document(d)
    for w in (src_win, dst):
        try:
            w._link_watcher().refresh()
        except Exception:  # noqa: BLE001
            pass
        w._update_title()
    from views import wgpu_start
    wgpu_start.hide(dst)
    src_win._host.request_frame()
    dst_host.request_frame()
    return True


def elsewhere(win) -> list:
    """The projects open in the other windows, for the ›› n menu:
    (id, name, window label)."""
    out = []
    for n, h in enumerate(hosts()):
        if h.window is win:
            continue
        front = h.window._doc
        label = f"Window {n + 1}" + (f" · {front.display_name()}" if front is not None else "")
        for i, doc in enumerate(h.window._documents):
            out.append((f"{id(h)}:{i}", doc.display_name(), label))
    return out


def goto(win, ident) -> bool:
    """A project in another window, from the ›› n menu: that window comes
    to the front with it."""
    hid, _, index = ident.partition(":")
    for h in hosts():
        if str(id(h)) == hid:
            docs = h.window._documents
            try:
                doc = docs[int(index)]
            except (ValueError, IndexError):
                return False
            h.window.activate_document(doc)
            h.focus_requested = True
            h.request_frame()
            return True
    return False

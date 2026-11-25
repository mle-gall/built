# SPDX-License-Identifier: GPL-3.0-or-later
"""Project tabs in the chrome's top bar (shell design option B).

The tab strip sits beside the window buttons, as in Figma. Python sends the
tabs, and ``rust/ingetrazo_render/src/ui/tabs.rs`` draws them and reports
clicks. Document switching, closing and creation use the window commands.

- A tab is one open project. ``•`` means unsaved. The tab in front has the
  accent line.
- A link source is a ⛓ tab: an open project whose file another open
  project links. It opens right of its host ("Open source" in the tree, the
  inspector or the right-click menu), and its tip names the host and
  counts the placements.
- A background tab can show its last model frame. The renderer owns the
  tab textures and reads the hover preference with this strip's payload.
- Scenes are never tabs: they keep the ‹ Scene ▾ › control.
"""
from __future__ import annotations

import time
from pathlib import Path

def _docs(win):
    documents = getattr(win, "documents", None)
    return documents() if documents is not None else []


def _dirty(win, doc) -> bool:
    fn = getattr(win, "_doc_is_dirty", None)
    try:
        return bool(fn(doc)) if fn is not None else False
    except Exception:  # noqa: BLE001
        return False


def link_host(win, doc, docs=None):
    """``(host document, placements)`` when ``doc``'s file is linked into
    another project open in this window, else ``(None, 0)``. The host the
    source was opened from wins ("Open source")."""
    if doc.path is None:
        return None, 0
    from core import links
    docs = docs if docs is not None else _docs(win)
    preferred = getattr(doc, "link_host", None)
    order = ([preferred] if preferred in docs else []) + [d for d in docs if d is not preferred]
    for host in order:
        if host is doc or host.viewport is None:
            continue
        try:
            n = len(links.linked_groups(host.viewport.scene, doc.path))
        except Exception:  # noqa: BLE001
            n = 0
        if n:
            return host, n
    return None, 0


def _saved_text(path) -> str:
    try:
        t = Path(path).stat().st_mtime
    except OSError:
        return ""
    if time.time() - t < 20 * 3600:
        return time.strftime("saved %H:%M", time.localtime(t))
    return time.strftime("saved %d %b", time.localtime(t))


def _tip(win, doc, host, n) -> str:
    lines = []
    if doc.path is not None:
        home = str(Path.home())
        where = str(doc.path)
        where = "~" + where[len(home):] if where.startswith(home) else where
        saved = _saved_text(doc.path)
        lines.append(where + (f" · {saved}" if saved else ""))
    else:
        lines.append("Not saved yet")
    if host is not None:
        lines.append(f"linked in {host.display_name()} · {n} placement" + ("" if n == 1 else "s"))
    if _dirty(win, doc):
        lines.append("Unsaved changes")
    return "\n".join(lines)


def pictures_on() -> bool:
    from core.settings import settings as native_settings
    return str(native_settings().value("tabs/hover_picture", "1")) != "0"


def tabs(win, docs=None) -> list:
    """The strip's tabs: (id, name, front, unsaved, link source, tip,
    optional external picture key), in the window's order. ``docs``: the
    window's documents, when the caller already read them."""
    docs = _docs(win) if docs is None else list(docs)
    front = getattr(win, "_doc", None)
    out = []
    for doc in docs:
        host, n = link_host(win, doc, docs)
        out.append((doc.tab_id, doc.display_name(), doc is front, _dirty(win, doc), host is not None,
                    _tip(win, doc, host, n), ""))
    return out


def _elsewhere(win) -> list:
    """The projects in the app's other windows (the native app): (id, name, window)."""
    try:
        from ingetrazo_render import shell_windows as windows
    except Exception:  # noqa: BLE001
        return []
    return windows.elsewhere(win)


def _can_move(win) -> bool:
    try:
        from ingetrazo_render import shell_windows as windows
    except Exception:  # noqa: BLE001
        return False
    return windows.can_move(win)


def document_tabs(view):
    """The renderer's record of what its document ▾ and tab strip show
    (``ingetrazo_render.DocumentTabs``): ``.sync(renderer, win, left)``
    sends what changed, once a frame. A stand-in renderer without one
    (tests) leaves the record on the view."""
    from ingetrazo_render import DocumentTabs
    own = getattr(getattr(view, "renderer", None), "document_tabs", None)
    state = own() if callable(own) else None
    if not isinstance(state, DocumentTabs):
        state = view.__dict__.get("_ui_document_tabs")
        if state is None:
            state = view._ui_document_tabs = DocumentTabs()
    return state


# ---- actions ------------------------------------------------------------------------
def _close(win, doc) -> bool:
    fn = getattr(win, "close_document", None)
    if fn is not None:
        return bool(fn(doc))
    return bool(win.run_command("close_tab", doc))


def _activate(win, doc) -> None:
    if doc is not getattr(win, "_doc", None):
        win.activate_document(doc)


def action(win, view, arg) -> None:
    """``ptab:<arg>`` from the strip."""
    docs = _docs(win)
    verb, _, rest = arg.partition(":")

    def doc_at(text):
        try:
            i = int(text)
        except ValueError:
            return None
        return docs[i] if 0 <= i < len(docs) else None

    if verb == "activate":
        doc = doc_at(rest)
        if doc is not None:
            _activate(win, doc)
    elif verb == "close":
        doc = doc_at(rest)
        if doc is not None:
            _close(win, doc)
    elif verb == "new":
        win._on_new_tab()
    elif verb == "goto":
        try:
            from ingetrazo_render import shell_windows as windows
        except Exception:  # noqa: BLE001
            return
        windows.goto(win, rest)
    elif verb == "menu":
        what, _, index = rest.partition(":")
        if doc_at(index) is not None:
            from ingetrazo_render import project_tab_menu
            project_tab_menu(win, what, int(index))
    if view is not None:
        document_tabs(view).stale()
        view.update()


def source_context(win):
    doc = getattr(win, "_doc", None)
    host = getattr(doc, "link_host", None)
    if host is None or host not in _docs(win):
        return None, None
    from core import links
    scene = host.viewport.scene
    mount = scene.resolve_occurrence(doc.link_mount) if getattr(doc, "link_mount", None) else None
    if mount is None or Path((links.link_of(mount) or {}).get("path", "")) != doc.path:
        return None, None
    return host, mount


def sync_host_context(view, win):
    doc = getattr(win, "_doc", None)
    host = getattr(doc, "link_host", None)
    scene = host.viewport.scene if host is not None and host in _docs(win) else None
    origin = tuple(getattr(getattr(view, "packed", None), "origin", (0., 0., 0.)))
    sig = (id(scene), scene.content_version, host.viewport.history.revision, origin) if scene is not None else ()
    if view.__dict__.get("_ui_host_sig") == sig:
        return
    host, mount = source_context(win)
    view.renderer.ui_set_host_context(host.display_name() if host else "", origin, scene if mount else None, mount)
    view._ui_host_sig = sig


def open_link_source(win, path, mount=None) -> bool:
    """"Open source": the linked file in a tab of its own, right of the
    project it is linked into (a ⛓ tab), or its tab brought to the front."""
    host = getattr(win, "_doc", None)
    ok = win.open_path(Path(path))

    def bind(doc):
        if doc is host or host not in _docs(win):
            return
        from core import links
        scene = host.viewport.scene
        source = mount or next(iter(links.linked_groups(scene, path)), None)
        doc.link_host = host
        doc.link_mount = scene.occurrence_path(source) if source is not None else None
        if getattr(win, "view", None) is not None:
            win.view._ui_host_sig = None
    if ok:
        from ingetrazo_render.document_open import after_open
        after_open(win, Path(path), bind)
    return bool(ok)


def step(win, delta) -> None:
    """⌃Tab / ⌃⇧Tab: the next or previous tab in strip order, wrapping."""
    docs = _docs(win)
    front = getattr(win, "_doc", None)
    if len(docs) < 2 or front not in docs:
        return
    _activate(win, docs[(docs.index(front) + delta) % len(docs)])

# SPDX-License-Identifier: GPL-3.0-or-later
"""Project tabs in the chrome's top bar (shell design option B).

The tab strip sits beside the window buttons, as in Figma. Python sends the
tabs, and ``rust/ingetrazo_render/src/ui/tabs.rs`` draws them and reports
clicks. The same code runs in the native `built` shell and in the Qt app
with the chrome. Both expose ``window._project_tabs`` and
``window._documents``, which the document ▾ already read (S1.1), so
switching, closing and New reuse that machinery.

- A tab is one open project. ``•`` means unsaved. The tab in front has the
  accent line.
- A link source is a ⛓ tab: an open project whose file another open
  project links. It opens right of its host ("Open source" in the tree, the
  inspector or the right-click menu), and its tip names the host and
  counts the placements.
- Hovering a tab in the back shows the picture of that project as it was
  left. The picture is drawn when the tab is first hovered after you left it
  (``ptab:hover``), or, in the native app, as you leave it, since there the
  one renderer goes to the project in front. **Picture on hover** in the
  tab menu turns it off (the ``tabs/hover_picture`` setting, shared with the
  classic app).
- Scenes are never tabs: they keep the ‹ Scene ▾ › control.
"""
from __future__ import annotations

import time
from pathlib import Path

#: The picture's width (pixels; the card shows it at 264 pt).
PICTURE_PX = 528


def _docs(win):
    bar = getattr(win, "_project_tabs", None)
    if bar is None:
        return []
    try:
        return list(bar.documents())
    except Exception:  # noqa: BLE001
        return list(getattr(win, "_documents", []) or [])


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
    from core.settings import settings as QSettings
    return str(QSettings().value("tabs/hover_picture", "1")) != "0"


def remember_picture(doc) -> bool:
    """The picture of ``doc`` as its view stands, for its tab's hover card
    (the native app calls this as the project is left; the Qt app on the
    first hover after). Only a view that holds a renderer can draw it."""
    view = getattr(doc, "stand_in", None)
    if view is None or getattr(view, "renderer", None) is None or getattr(view, "packed", None) is None:
        return False
    if not pictures_on():
        return False
    try:
        if view.width() <= 0 or view.height() <= 0:
            return False
        w, h, data = view.render_pixels(PICTURE_PX)
    except Exception:  # noqa: BLE001 — a missing picture only
        return False
    if not data or len(data) != w * h * 4:
        return False
    doc._tab_picture = (w, h, data)
    doc._tab_picture_n = getattr(doc, "_tab_picture_n", 0) + 1
    doc.picture_due = False
    return True


def _picture_key(view, doc) -> str:
    pic = getattr(doc, "_tab_picture", None)
    if pic is None:
        return ""
    key = f"tabpic:{id(doc)}:{getattr(doc, '_tab_picture_n', 0)}"
    sent = view.__dict__.setdefault("_ui_tab_pictures", set())
    if key not in sent:
        w, h, data = pic
        view.renderer.ui_set_icons([(key, w, h, data)])
        sent.add(key)
    return key


def tabs(win, view=None) -> list:
    """The strip's tabs: (id, name, front, unsaved, link source, tip,
    picture key), in the window's order."""
    docs = _docs(win)
    front = getattr(win, "_doc", None)
    out = []
    for i, doc in enumerate(docs):
        if doc is front:
            # Its picture is of how it was left; it changes from now on.
            doc._tab_picture = None
        host, n = link_host(win, doc, docs)
        key = _picture_key(view, doc) if view is not None and doc is not front else ""
        out.append((str(i), doc.display_name(), doc is front, _dirty(win, doc), host is not None,
                    _tip(win, doc, host, n), key))
    return out


def _elsewhere(win) -> list:
    """The projects in the app's other windows (the native app): (id, name, window)."""
    try:
        from app_shell import windows
    except Exception:  # noqa: BLE001
        return []
    return windows.elsewhere(win)


def _can_move(win) -> bool:
    try:
        from app_shell import windows
    except Exception:  # noqa: BLE001
        return False
    return windows.can_move(win)


def before_frame(view, win) -> None:
    """Send the tabs when they changed (cheap: names, flags, a stat per
    saved file once a second at most)."""
    renderer = getattr(view, "renderer", None)
    if renderer is None or not hasattr(renderer, "ui_set_project_tabs"):
        return
    now = time.monotonic()
    docs = _docs(win)
    sig = (tuple(id(d) for d in docs), id(getattr(win, "_doc", None)),
           tuple(_dirty(win, d) for d in docs), tuple(d.display_name() for d in docs),
           tuple(getattr(d, "_tab_picture_n", 0) for d in docs), pictures_on(),
           tuple(_elsewhere(win)))
    last = view.__dict__.get("_ui_ptabs_sig")
    if last is not None and last[0] == sig and now - last[1] < 1.0:
        return
    data = tabs(win, view)
    payload = (data, _elsewhere(win), _can_move(win), pictures_on())
    if payload != view.__dict__.get("_ui_ptabs"):
        renderer.ui_set_project_tabs(*payload)
        view._ui_ptabs = payload
    view._ui_ptabs_sig = (sig, now)


# ---- actions ------------------------------------------------------------------------
def _close(win, doc) -> bool:
    fn = getattr(win, "close_document", None)
    if fn is not None:
        return bool(fn(doc))
    return bool(win.run_command("close_tab", doc))


def _activate(win, doc) -> None:
    bar = getattr(win, "_project_tabs", None)
    if doc is getattr(win, "_doc", None):
        return
    if bar is not None and hasattr(bar, "activated"):
        bar.activated.emit(doc)
    else:
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
        bar = getattr(win, "_project_tabs", None)
        if bar is not None and hasattr(bar, "newRequested"):
            bar.newRequested.emit()
    elif verb == "hover":
        doc = doc_at(rest)
        if doc is not None and doc is not getattr(win, "_doc", None) \
                and getattr(doc, "_tab_picture", None) is None and remember_picture(doc):
            view._ui_ptabs_sig = None
    elif verb == "goto":
        try:
            from app_shell import windows
        except Exception:  # noqa: BLE001
            return
        windows.goto(win, rest)
    elif verb == "menu":
        what, _, index = rest.partition(":")
        menu(win, what, doc_at(index), docs)
    if view is not None:
        view._ui_ptabs_sig = None
        view.update()


def menu(win, what, doc, docs) -> None:
    """The tab menu's rows (ui/tabs.rs): ``doc`` is the tab right-clicked,
    already in front."""
    if what == "new":
        action(win, None, "new")
    elif what == "open":
        win._on_open()
    elif what == "save":
        win._on_save()
    elif what == "save_as":
        win._on_save_as()
    elif what == "close" and doc is not None:
        _close(win, doc)
    elif what in ("close_right", "close_left", "close_all") and doc is not None:
        i = docs.index(doc) if doc in docs else -1
        targets = {"close_right": docs[i + 1:], "close_left": docs[:i], "close_all": docs}[what]
        # Each asks about its own changes; a Cancel stops there.
        for d in list(reversed(targets)) if what == "close_right" else list(targets):
            if d in _docs(win) and not _close(win, d):
                break
    elif what == "move" and doc is not None:
        try:
            from app_shell import windows
        except Exception:  # noqa: BLE001
            return
        windows.move_to_new_window(win, doc)
    elif what == "pictures":
        from core.settings import settings as QSettings
        QSettings().setValue("tabs/hover_picture", "0" if pictures_on() else "1")


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
        from views.document_open import after_open
        after_open(win, Path(path), bind)
    return bool(ok)


def step(win, delta) -> None:
    """⌃Tab / ⌃⇧Tab: the next or previous tab in strip order, wrapping."""
    docs = _docs(win)
    front = getattr(win, "_doc", None)
    if len(docs) < 2 or front not in docs:
        return
    _activate(win, docs[(docs.index(front) + delta) % len(docs)])

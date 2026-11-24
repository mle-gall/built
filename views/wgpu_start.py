# SPDX-License-Identifier: GPL-3.0-or-later
"""The start page: a large sheet over the model (``views/chrome_sheet.py``,
option C of the 2026-11-23 review), in the Qt app with the Built chrome and
in the native ``built`` shell.

The Start sheet opens on a cold start without a file and from File ▸ Home.

What it shows (``rust/ingetrazo_render/src/start_view.rs`` decides the
rows, their words, the preview and the templates from what Python sends:
the recents, what each document noted, what the worker found; the sheet
owns the layout and the focus):

* the header: New model, From template ▾, Open…, New project…, a filter;
* a dense list on the left: Continue (the document open behind the page,
  or the last one), Pinned, Recent (missing files dimmed, "Clear missing"),
  Projects, Templates; a right click pins, reveals or removes;
* a preview on the right of the focused row: its picture, project, where
  it was left, problems, unfiled, and its linked files with their state
  (found, changed on disk, missing) read from the archive without opening
  it (``core/link_scan.py``); Open, Show in Finder, Locate…;
* the footer: Library · Settings · Extensions · Help, the keys, the engine.

Nothing here blocks the first frame. Recents come from QSettings
(``core.recent``) at once; whether each file still exists, its picture
(the ``thumbnail.png`` a save puts in the archive), its links and each
project's files are read by a worker thread and cached by path. The page
shows what it has and fills in as the worker reports.

Opening goes through ``MainWindow.open_path``, so the egui progress
capsule (``views/chrome_dialogs``) shows over the page while it loads;
the page goes when the document is in.
"""
from __future__ import annotations

import hashlib
import os
import sys
import threading
import time
from pathlib import Path

from core.settings import settings as QSettings
from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QGuiApplication, QImage

from core import recent

SEP = "\x1f"
#: Asked for and not drawn this long after: report the failed frame.
WATCHDOG_MS = 2000
#: Thumbnails wider than this are scaled down before upload (points × 2).
THUMB_PX = 512

# ---- Templates ---------------------------------------------------------------
def _templates():
    import json
    from ingetrazo_render import start_templates
    return [(tid, name, desc, json.loads(nodes) if nodes else None, active or None,
             json.loads(units) if units else None)
            for tid, name, desc, nodes, active, units in start_templates()]


#: (id, name, description, structure nodes or None, active node, units or
#: None), from rust/ingetrazo_render/src/start_view.rs.
TEMPLATES = _templates()


def _view(win):
    doc = getattr(win, "_doc", None)
    return getattr(doc, "stand_in", None) if doc is not None else None


def page(win) -> "Page":
    p = win.__dict__.get("_start_page")
    if p is None:
        p = win._start_page = Page(win)
    return p


def shown(win) -> bool:
    p = win.__dict__.get("_start_page") if win is not None else None
    return bool(p is not None and p.wanted)


# ---- Showing and hiding ------------------------------------------------------
def show(win) -> bool:
    """Open the Start sheet in this window."""
    return page(win).open()


def hide(win) -> None:
    p = win.__dict__.get("_start_page")
    if p is not None and p.wanted:
        p.close()


def can_return(win) -> bool:
    """A document is open behind the page (not just the blank untitled one)."""
    docs = list(getattr(win, "_documents", []) or [])
    if len(docs) > 1:
        return True
    try:
        return bool(docs) and not win._doc_is_blank(docs[0])
    except Exception:  # noqa: BLE001
        return False


def _docs_sig(win):
    return tuple((id(d), str(getattr(d, "path", None)), getattr(d, "import_name", None))
                 for d in getattr(win, "_documents", []) or []) + (id(getattr(win, "_doc", None)),)


class Page:
    """One window's start page: whether it is wanted, the loader, the
    sheet's provider (:class:`StartSheet`)."""

    def __init__(self, win):
        self.win = win
        self.wanted = False
        self.drawn = False
        self.docs_sig = None
        self.loader = None
        self.poll = None
        self.dog = None
        self.focused = False
        self.asked = 0.0
        self.sheet = StartSheet(self)

    # -- state ------------------------------------------------------------
    def open(self) -> bool:
        win = self.win
        for doc in list(getattr(win, "_documents", []) or []):
            if getattr(doc, "path", None) is not None:
                note(win, doc.path, doc)
        from views import chrome_sheet
        if self.wanted and chrome_sheet.current(win) is self.sheet:
            # Already up (asked again at the end of a cold start): fresh
            # rows, nothing typed lost.
            self.load()
            chrome_sheet.refresh(win)
            return True
        chrome_sheet.open(win, self.sheet)
        self.wanted = True
        self.drawn = False
        self.focused = False
        self.sheet.focus = None
        self.asked = time.monotonic()
        self.docs_sig = _docs_sig(win)
        self.load()
        view = _view(win)
        if view is not None:
            try:
                view.setFocus(Qt.OtherFocusReason)
            except RuntimeError:
                pass
        if QGuiApplication.platformName() in ("offscreen", "minimal") and not chrome_sheet.in_shell():
            return True
        if self.dog is None:
            self.dog = QTimer(win, singleShot=True)
            self.dog.timeout.connect(self._watchdog)
        self.dog.start(WATCHDOG_MS)
        return True

    def close(self) -> None:
        self.wanted = False
        if self.dog is not None:
            self.dog.stop()
        from views import chrome_sheet
        if chrome_sheet.current(self.win) is self.sheet:
            chrome_sheet.close(self.win)
        self.repaint()

    def _watchdog(self) -> None:
        if not self.wanted or self.drawn:
            return
        win = self.win
        view = _view(win)
        waited = time.monotonic() - self.asked
        coming = view is not None and (getattr(view, "renderer", None) is None
                                       or not getattr(view, "_ui_attached", False)
                                       or not win.isVisible())
        if coming and waited < 10.0:
            self.dog.start(500)     # a cold start: the Metal view is still on its way
            return
        self.wanted = False
        from views import chrome_sheet
        if chrome_sheet.current(win) is self.sheet:
            win.__dict__.pop("_chrome_sheet", None)
        win.statusBar().showMessage("Built could not draw the Start sheet.")

    def repaint(self) -> None:
        view = _view(self.win)
        if view is not None:
            try:
                view.update()
            except RuntimeError:
                pass

    def changed(self) -> None:
        from views import chrome_sheet
        chrome_sheet.refresh(self.win)
        self.repaint()

    # -- the worker -------------------------------------------------------
    def load(self) -> None:
        paths = [r["path"] for r in recent.entries("files")]
        paths += [r["path"] for r in recent.entries("projects")]
        last = last_file()
        if last and last not in paths:
            paths.append(last)
        todo = [p for p in paths if _cached(p) is None]
        if not todo or (self.loader is not None and self.loader.is_alive()):
            return
        self.loader = threading.Thread(target=_load_all, args=(todo,), daemon=True,
                                       name="start-page-thumbnails")
        self.loader.start()
        if self.poll is None:
            self.poll = QTimer(self.win)
            self.poll.setInterval(120)
            self.poll.timeout.connect(self._poll)
        self.poll.start()

    def _poll(self) -> None:
        busy = self.loader is not None and self.loader.is_alive()
        if _take_news():
            self.changed()
        if not busy:
            self.poll.stop()
            if _take_news():
                self.changed()

    # -- a frame ----------------------------------------------------------
    def before_frame(self, view) -> None:
        win = self.win
        if self.wanted and _docs_sig(win) != self.docs_sig:
            # A document came in (⌘O, a drop, the OS): the page has done its job.
            self.close()
        if not self.wanted:
            return
        from views import chrome_sheet
        if chrome_sheet.current(win) is not self.sheet:
            # Another sheet took the window (Settings from the footer): the
            # page waits under it and comes back when that one closes.
            if chrome_sheet.current(win) is None:
                chrome_sheet.open(win, self.sheet)
            return
        if not self.focused:
            # Keys come to the view; never focus from inside a frame (a
            # native view can expose, and paint, right there).
            self.focused = True

            def focus():
                try:
                    view.setFocus(Qt.OtherFocusReason)
                except RuntimeError:
                    pass    # the view went first
            QTimer.singleShot(0, focus)


# ---- The sheet ---------------------------------------------------------------
def _sheet_class():
    from views import chrome_sheet
    return chrome_sheet.Provider


class StartSheet(_sheet_class()):
    """The start page on the large sheet (option C of the 2026-11-23
    review): verbs and a filter in the header, a dense list on the left
    (continue, pinned, recent, projects, templates), a preview on the right
    that shows the focused file's picture, links and their state, unfiled
    and problems before it is opened."""
    id = "start"
    watchdog = False                    # the page has its own (cold starts are slow)

    def __init__(self, page):
        self.page = page
        self.focus = None               # the focused item id
        self.filter = ""
        self.cont_path = ""             # the Continue row's file, as last built

    # -- what is on it ------------------------------------------------------
    def signature(self, ctx):
        # A document opened behind the page changes Continue and Esc.
        return (can_return(self.page.win), self.page.win.__dict__.get("_pending_open"))

    def items(self, back: bool) -> dict:
        """The list's sections after the filter: {"cont", "pinned",
        "recent", "projects", "templates"} of (id, kind, path or template)."""
        from ingetrazo_render import start_sections
        return {key: rows for key, rows in start_sections(_state(self, back))}

    def build(self, ctx):
        import json
        from ingetrazo_render import start_sheet
        from views import chrome_sheet as cs
        page = self.page
        page.drawn = True
        spec = json.loads(start_sheet(_state(self, can_return(page.win))))
        self.focus = spec["focus"]
        self.cont_path = spec.pop("cont_path")
        s = cs.Spec(spec)
        pictures = {}
        with _LOCK:
            for c in _CACHE.values():
                if c.get("thumb"):
                    pictures[c["thumb"][0]] = c["thumb"]
        for key in spec.pop("icons"):
            if key in pictures:
                s.icon(*pictures[key])
        return s

    # -- what comes back --------------------------------------------------------
    def act(self, ctx, verb, args):
        win = self.page.win
        arg = args[0] if args else ""
        if verb == "search":
            self.filter = arg
        elif verb == "focus":
            self.focus = arg
        elif verb == "open":
            self.activate(win, arg)
        elif verb == "menu" and len(args) > 1:
            self.item_verb(win, args[1], arg)
        elif verb == "headmenu" and arg == "template" and len(args) > 1:
            from_template(win, args[1])
        elif verb == "verb":
            if arg in ("open", "finder", "locate", "forget", "pin"):
                self.item_verb(win, arg, self.focus or "")
            else:
                action(win, {"libraries": "nav" + SEP + "libraries", "prefs": "cmd" + SEP + "prefs",
                             "extensions": "cmd" + SEP + "extensions", "help": "cmd" + SEP + "help"}.get(arg, arg))

    def activate(self, win, rid) -> None:
        kind, _, rest = rid.partition(":")
        if rid == "cont":
            action(win, "continue")
        elif kind == "tpl":
            from_template(win, rest)
        elif kind in ("file", "proj"):
            open_recent(win, rest)

    def item_verb(self, win, verb, rid) -> None:
        kind, _, path = rid.partition(":")
        if rid == "cont":
            kind, path = "file", self.cont_path
        if verb == "open":
            self.activate(win, rid)
        elif not path or kind not in ("file", "proj"):
            return
        elif verb == "pin":
            action(win, "pin" + SEP + path)
        elif verb == "forget":
            action(win, "forget" + SEP + path)
        elif verb == "locate":
            open_recent(win, path)
        elif verb == "finder":
            reveal(path)

    def closed(self, ctx):
        if ctx.replaced:
            return                      # Settings over it: the page comes back after
        if self.page.wanted:
            self.page.wanted = False
            if self.page.dog is not None:
                self.page.dog.stop()


def _state(sheet, back) -> str:
    """What the Rust view model reads (start_view.rs ``State``): the
    recents, what each document noted, what the worker found."""
    import json
    win = sheet.page.win
    files, projects = recent.entries("files"), recent.entries("projects")
    st = QSettings()
    try:
        meta = json.loads(st.value("home/meta", "{}"))
    except (TypeError, ValueError):
        meta = {}
    open_doc = {}
    if back:
        doc = getattr(win, "_doc", None)
        path = getattr(doc, "path", None)
        # As the recents hold it (resolved): one row, one picture.
        open_doc = {"path": str(Path(path).resolve()) if path else "",
                    "name": doc.display_name() if doc is not None and hasattr(doc, "display_name") else "Untitled"}
    with _LOCK:
        cache = {p: {"exists": c.get("exists"), "mtime": c.get("mtime"),
                     "thumb": c["thumb"][0] if c.get("thumb") else "", "files": c.get("files"),
                     "links": c.get("links"), "autosave_newer": bool(c.get("autosave_newer"))}
                 for p, c in _CACHE.items()}
    pending = win.__dict__.get("_pending_open")
    return json.dumps({"filter": sheet.filter, "focus": sheet.focus, "back": bool(back), "open_doc": open_doc,
                       "last": last_file(), "files": files, "projects": projects, "cache": cache,
                       "meta": meta if isinstance(meta, dict) else {},
                       "pending": pending.job.label if pending else "", "engine": footer(),
                       "home": str(Path.home()), "now": time.time(),
                       "utc_offset": time.localtime().tm_gmtoff})


def reveal(path) -> None:
    """Show the file in the Finder (both apps)."""
    import subprocess
    try:
        if sys.platform == "darwin":
            subprocess.Popen(["open", "-R", str(path)])
        else:
            from PySide6.QtCore import QUrl
            from PySide6.QtGui import QDesktopServices
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(Path(path).parent)))
    except OSError:
        pass

# ---- Rows --------------------------------------------------------------------
def last_file() -> str:
    """The document opened or saved last (pins reorder the recent list, so
    its first row is not it)."""
    value = QSettings().value("home/last_file", "") or ""
    if isinstance(value, str) and value:
        return value
    rows = recent.entries("files")
    plain = [r for r in rows if not r.get("pinned")]
    return (plain or rows)[0]["path"] if rows else ""


def footer() -> str:
    try:
        from core._native import engine
        from core import renderer_settings
        return f"Engine: {engine()} · Renderer: {renderer_settings.status_text()}"
    except Exception:  # noqa: BLE001
        return ""


def _libraries():
    """The library browser, when one is installed (views/wgpu_library.py)."""
    try:
        import importlib.util
        if importlib.util.find_spec("views.wgpu_library") is None:
            return None
        from views import wgpu_library
    except Exception:  # noqa: BLE001
        return None
    for name in ("show", "open", "open_library", "toggle"):
        fn = getattr(wgpu_library, name, None)
        if callable(fn):
            return fn
    return None


# ---- What a document says about itself (noted on open, save, start) ----------
def note(win, path, doc=None) -> None:
    """Remember what the start page shows of a document while it is open:
    its project, links, origin, unfiled items and problems, where it was
    left. Cheap: what the window already knows."""
    try:
        doc = doc or getattr(win, "_doc", None)
        vp = getattr(doc, "viewport", None)
        scene = getattr(vp, "scene", None)
        if scene is None or path is None:
            return
        data = {}
        proj = (scene.plugin_data.get("project") or {}).get("path") if isinstance(scene.plugin_data, dict) else None
        if proj:
            data["project"] = Path(proj).stem
        from core import links
        data["links"] = len(links.sources(scene))
        name = getattr(doc, "import_name", None)
        if name:
            data["imported"] = Path(str(name)).suffix.lower().lstrip(".")
        view = getattr(doc, "stand_in", None)
        tree = getattr(view, "_ui_tree", None) if view is not None else None
        if isinstance(tree, dict):
            data["unfiled"] = len(tree.get("unfiled") or ())
            data["problems"] = len(tree.get("to_fix") or ())
        where = "Model"
        row = getattr(getattr(win, "_scene_tabs", None), "tabs", None)
        marked = row.marked_view() if row is not None and hasattr(row, "marked_view") else None
        if marked is not None and getattr(marked, "name", ""):
            where += " › " + marked.name
        data["workspace"] = where
        recent.note(path, data)
    except Exception:  # noqa: BLE001 — the page only loses a line
        pass


# ---- The worker: existence, pictures, project files ---------------------------
_CACHE: dict = {}       # path -> {"key": (mtime_ns, size), exists, mtime, thumb, files, autosave_newer}
_LOCK = threading.Lock()
_NEWS = [False]


def _cached(path):
    with _LOCK:
        return _CACHE.get(path)


def _take_news() -> bool:
    with _LOCK:
        news, _NEWS[0] = _NEWS[0], False
        return news


def _load_all(paths) -> None:
    for path in paths:
        try:
            entry = _load_one(path)
        except Exception:  # noqa: BLE001 — one bad file never stops the rest
            entry = {"exists": None}
        with _LOCK:
            _CACHE[path] = entry
            _NEWS[0] = True


def _load_one(path: str) -> dict:
    p = Path(path)
    try:
        st = p.stat()
    except OSError:
        return {"exists": False}
    entry = {"exists": True, "mtime": st.st_mtime, "key": (st.st_mtime_ns, st.st_size)}
    suffix = p.suffix.lower()
    if suffix == ".igzproj":
        try:
            from formats import project as storage
            value = storage.load(p)
            entry["files"] = [(str((p.parent / r["path"]).resolve()), r.get("role", "host"))
                              for r in value.get("files", []) if isinstance(r, dict) and r.get("path")]
        except Exception:  # noqa: BLE001 — unreadable: say so on its card
            entry["files"] = None
        return entry
    entry["thumb"] = _thumbnail(p, st)
    try:
        from core import link_scan
        entry["links"] = link_scan.scan(p)
    except Exception:  # noqa: BLE001 — the preview only loses its links
        entry["links"] = []
    try:
        from core import autosave
        slot = autosave.pending(p)
        entry["autosave_newer"] = bool(slot is not None and slot.stat().st_mtime > st.st_mtime + 1)
    except Exception:  # noqa: BLE001
        entry["autosave_newer"] = False
    return entry


def _thumbnail(p: Path, st):
    """(texture key, w, h, RGBA) of the picture a save keeps in the archive."""
    import zipfile
    try:
        with zipfile.ZipFile(p) as archive:
            info = archive.getinfo("thumbnail.png")
            if info.file_size > 4 * 1024 * 1024:
                return None
            data = archive.read(info)
    except (OSError, KeyError, zipfile.BadZipFile, ValueError):
        return None
    image = QImage()
    if not image.loadFromData(data, "PNG"):
        return None
    if blank_picture(image):
        # A save that grabbed the hidden GL view (Metal on) kept a black
        # frame: the placeholder says more than a black tile.
        return None
    if image.width() > THUMB_PX:
        image = image.scaledToWidth(THUMB_PX, Qt.SmoothTransformation)
    image = image.convertToFormat(QImage.Format_RGBA8888)
    w, h = image.width(), image.height()
    rgba = bytes(image.constBits())[: w * h * 4]
    if image.bytesPerLine() != w * 4:
        rgba = b"".join(bytes(image.constScanLine(y))[: w * 4] for y in range(h))
    key = "thumb:" + hashlib.sha1(f"{p}|{st.st_mtime_ns}|{st.st_size}".encode()).hexdigest()[:16]
    return key, w, h, rgba


#: A picture is blank when this share of it is one colour (a black frame
#: with a few stray pixels of chrome or a far speck of model counts).
BLANK_SHARE = 0.985


def blank_picture(image) -> bool:
    """True when ``image`` (a QImage) shows nothing: empty, or all (or
    nearly all) one colour. Read on a 64 × 64 sample, so it costs the same
    for any size; safe off the GUI thread."""
    if image is None or image.isNull() or image.width() < 1 or image.height() < 1:
        return True
    small = image.scaled(64, 64, Qt.IgnoreAspectRatio, Qt.FastTransformation)
    small = small.convertToFormat(QImage.Format_RGBA8888)
    w, h = small.width(), small.height()
    raw = bytes(small.constBits())
    stride = small.bytesPerLine()
    counts: dict = {}
    for y in range(h):
        row = raw[y * stride: y * stride + w * 4]
        for x in range(0, w * 4, 4):
            # Quantised: a JPEG-ish wobble or dither is still one colour.
            key = (row[x] >> 3, row[x + 1] >> 3, row[x + 2] >> 3)
            counts[key] = counts.get(key, 0) + 1
    return max(counts.values()) >= BLANK_SHARE * w * h


def forget_cache() -> None:
    """Tests: start from nothing known."""
    with _LOCK:
        _CACHE.clear()
        _NEWS[0] = False


# ---- Frames, keys, actions (called from views/wgpu_ui.py) ---------------------
def before_frame(view, win) -> None:
    p = win.__dict__.get("_start_page") if win is not None else None
    if p is not None:
        p.before_frame(view)


def action(win, arg: str) -> None:
    """``<verb>[\\x1f<arg>]``: the page's verbs (the sheet, the shell's
    menu and the tests call them by name)."""
    verb, _, rest = arg.partition(SEP)
    p = page(win)
    if verb == "show":
        show(win)
    elif verb == "close":
        if can_return(win):
            hide(win)
    elif verb == "new":
        new_model(win)
    elif verb == "project":
        new_project(win)
    elif verb == "openfile":
        open_dialog(win)
    elif verb == "open" and rest:
        open_recent(win, rest)
    elif verb == "continue":
        if can_return(win):
            hide(win)
        else:
            path = last_file()
            if path:
                open_recent(win, path)
    elif verb == "compare" and rest:
        # Opening asks which to keep (the file or its auto-saved copy).
        open_recent(win, rest)
    elif verb == "pin" and rest:
        row = next((r for k in ("files", "projects") for r in recent.entries(k) if r["path"] == rest), None)
        recent.remember(rest, pinned=not (row or {}).get("pinned", False))
        p.changed()
    elif verb == "forget" and rest:
        # Off the list only; the file stays where it is.
        recent.forget(rest)
        p.changed()
    elif verb == "clearmissing":
        from ingetrazo_render import start_missing
        gone = start_missing(_state(p.sheet, False))
        if gone:
            recent.forget(gone)
        p.changed()
    elif verb == "template" and rest:
        from_template(win, rest)
    elif verb == "nav" and rest == "libraries":
        fn = _libraries()
        if fn is not None:
            hide(win)
            try:
                fn(win)
            except TypeError:
                fn()
    elif verb == "cmd":
        run = {"prefs": "_on_preferences", "extensions": "_on_manage_extensions",
               "help": "_on_documentation"}.get(rest)
        fn = getattr(win, run, None) if run else None
        if callable(fn):
            fn()

# ---- Verbs -------------------------------------------------------------------
def new_model(win, ask_units: bool = True) -> None:
    win._on_new()
    hide(win)
    if ask_units:
        from views.units_question import offer_at_start
        offer_at_start(win)


def new_project(win) -> None:
    from views.filedialogs import file_dialogs
    name, _ = file_dialogs.getSaveFileName(win, "New project", "", "Project (*.igzproj)")
    if not name:
        return
    from core.scene import Scene
    from core import project
    path = Path(name).with_suffix(".igzproj")
    try:
        project.create(Scene(), path)
        win.open_project(path)
    except (OSError, ValueError) as exc:
        from views import chrome_dialogs
        chrome_dialogs.warning(win, "Project", str(exc))
        return
    hide(win)


def open_dialog(win) -> None:
    from formats import built
    from views.filedialogs import file_dialogs
    name, _ = file_dialogs.getOpenFileName(
        win, "Open", "", f"Documents (*{built.BUILT_SUFFIX} *.igz *.igzproj)")
    if name:
        open_recent(win, name)


def open_recent(win, path) -> bool:
    """Open a recent file or project, offering to locate it if it moved
    (as the Home dialog did). The page goes once it is in."""
    from formats import built
    from views.filedialogs import file_dialogs
    original = str(path)
    path = Path(path)
    if not path.is_file():
        kind = "Project" if path.suffix.lower() == ".igzproj" else (
            "Built" if built.is_built(path) else "IngeTrazo")
        located, _ = file_dialogs.getOpenFileName(win, "Locate " + path.name, "", f"{kind} (*{path.suffix})")
        if not located:
            return False
        path = Path(located)
    if not win.open_path(path):
        return False
    if win.__dict__.get("_pending_open"):
        from views.document_open import after_open
        after_open(win, path, lambda doc: recent.remember(path, replace=original))
        return True
    recent.remember(path, replace=original)
    hide(win)
    return True


def from_template(win, tid: str) -> None:
    t = next((t for t in TEMPLATES if t[0] == tid), None)
    if t is None:
        return
    _, _, _, nodes, active, units = t
    new_model(win, ask_units=units is None)
    if nodes is None and units is None:
        return
    scene = win.viewport.scene
    if units:
        from core import units as _units
        _units.apply_units(scene, units)
    if nodes:
        apply_structure(scene, nodes, active)
    vp = win.viewport
    vp.update()
    for view in (_view(win),):
        if view is not None:
            view._ui_tree_sig = None
            view.update()


def apply_structure(scene, nodes, active=None) -> bool:
    """Seed a new document's structure (no undo step: like its units, the
    template is where the document starts, not an edit)."""
    import copy
    from core import bim_registry
    registry = bim_registry.get(scene)
    registry["structure"] = {"nodes": copy.deepcopy(nodes)}
    if active:
        registry["structure"]["active"] = active
    if bim_registry.validate(registry):
        return False
    scene.plugin_data["bim_registry"] = registry
    return True


def wants_at_start(argv_has_file: bool) -> bool:
    """main.py: the window opens on the page when no file is given."""
    return not argv_has_file and os.environ.get("BUILT_START", "1") != "0"

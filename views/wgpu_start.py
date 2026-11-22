# SPDX-License-Identifier: GPL-3.0-or-later
"""The start page: a full-window state of the Built chrome (egui, in the
Metal frame), drawn by ``rust/ingetrazo_render/src/ui/start.rs``.

It replaces the Qt Home dialog (``views/home.py``) for a cold start
without a file and for File ▸ New Window, and it comes back from File ▸
Home, the document menu of the top bar and ⌘K. The Qt dialog stays when
the chrome cannot draw it: ``BUILT_UI=0``, the GL renderer, headless and
test runs, or no frame drawn within :data:`WATCHDOG_MS` of asking.

What it shows (Python owns the data, Rust the layout, the filter and
the focus):

* a side column: Recent · Projects · Templates (and Libraries when a
  library browser is there), the recent projects with their files,
  Preferences · Extensions · Help at the bottom;
* the main column: New model / New project… / Open…; a Continue card for
  the last document (its picture, project, where it was left, its open
  problems, a newer auto-save); the recent files as tiles; templates;
  the engine and renderer as a quiet footer.

Nothing here blocks the first frame. Recents come from QSettings
(``core.recent``) at once; whether each file still exists, its picture
(the ``thumbnail.png`` a save puts in the archive) and each project's
files are read by a worker thread and cached by (path, mtime, size). The
page shows what it has and fills in as the worker reports.

Opening goes through ``MainWindow.open_path``, so the egui progress
capsule (``views/chrome_dialogs``) shows over the page while it loads;
the page goes when the document is in.
"""
from __future__ import annotations

import hashlib
import os
import threading
import time
from pathlib import Path

from PySide6.QtCore import QSettings, Qt, QTimer
from PySide6.QtGui import QGuiApplication, QImage

from core import recent

SEP = "\x1f"
#: Asked for and not drawn this long after: the Qt dialog instead.
WATCHDOG_MS = 2000
#: Thumbnails wider than this are scaled down before upload (points × 2).
THUMB_PX = 512

# ---- Templates ---------------------------------------------------------------
# Stubs until Phase 4 of docs/project-structure.md: a new model with a
# structure preset (core/structure.py nodes with recipes) and units.
_STEEL = {
    "steel": {"name": "Steel", "parent": None, "order": 0},
    "steel.columns": {"name": "Columns", "parent": "steel", "order": 0,
                      "recipe": {"tag": "Steel Columns", "class": "IfcColumn"}},
    "steel.beams": {"name": "Beams", "parent": "steel", "order": 1,
                    "recipe": {"tag": "Steel Beams", "class": "IfcBeam"}},
    "steel.bracing": {"name": "Bracing", "parent": "steel", "order": 2,
                      "recipe": {"tag": "Steel Bracing", "class": "IfcMember"}},
    "steel.plates": {"name": "Plates", "parent": "steel", "order": 3,
                     "recipe": {"tag": "Steel Plates", "class": "IfcPlate"}},
    "steel.fasteners": {"name": "Fasteners", "parent": "steel", "order": 4,
                        "recipe": {"tag": "Fasteners", "class": "IfcMechanicalFastener"}},
}
_SURVEY = {
    "existing": {"name": "Existing", "parent": None, "order": 0},
    "existing.structure": {"name": "Structure", "parent": "existing", "order": 0,
                           "recipe": {"tag": "Existing Structure"}},
    "existing.envelope": {"name": "Envelope", "parent": "existing", "order": 1,
                          "recipe": {"tag": "Existing Envelope"}},
    "existing.services": {"name": "Services", "parent": "existing", "order": 2,
                          "recipe": {"tag": "Existing Services"}},
    "reference": {"name": "Reference", "parent": None, "order": 1},
    "reference.scan": {"name": "Scan and survey points", "parent": "reference", "order": 0,
                       "recipe": {"tag": "Reference Scan"}},
    "reference.context": {"name": "Context", "parent": "reference", "order": 1,
                          "recipe": {"tag": "Reference Context"}},
}
_ARCH = {
    "building": {"name": "Building", "parent": None, "order": 0},
    "building.walls": {"name": "Walls", "parent": "building", "order": 0,
                       "recipe": {"tag": "Walls", "class": "IfcWall"}},
    "building.slabs": {"name": "Slabs", "parent": "building", "order": 1,
                       "recipe": {"tag": "Slabs", "class": "IfcSlab"}},
    "building.doors": {"name": "Doors", "parent": "building", "order": 2,
                       "recipe": {"tag": "Doors", "class": "IfcDoor"}},
    "building.windows": {"name": "Windows", "parent": "building", "order": 3,
                         "recipe": {"tag": "Windows", "class": "IfcWindow"}},
    "building.roof": {"name": "Roof", "parent": "building", "order": 4,
                      "recipe": {"tag": "Roof", "class": "IfcRoof"}},
}
#: (id, name, description, structure nodes or None, active node, units or None)
TEMPLATES = [
    ("survey", "As-built survey", "Existing structure, envelope, services; reference · m",
     _SURVEY, "existing.structure", {"length": "m", "precision": 3}),
    ("steel", "Steel fabrication", "Columns, beams, bracing, plates, fasteners · mm",
     _STEEL, "steel.beams", {"length": "mm", "precision": 0}),
    ("arch_mm", "Architectural mm", "Walls, slabs, doors, windows, roof · mm",
     _ARCH, "building.walls", {"length": "mm", "precision": 0}),
    ("blank", "Blank", "An empty model in your usual units", None, None, None),
]


# ---- Can the chrome draw it? -------------------------------------------------
def available() -> bool:
    """The Built chrome will draw the page: the UI on, a real screen and
    the Metal renderer wanted (the GL view has no chrome)."""
    from views import wgpu_ui
    if not wgpu_ui.ENABLED:
        return False
    if QGuiApplication.platformName() in ("offscreen", "minimal"):
        return False
    try:
        from core import renderer_settings
        return bool(renderer_settings.wanted())
    except Exception:  # noqa: BLE001 — no renderer setting: no chrome
        return False


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
    """Put the page up. False when the chrome cannot (the caller then
    shows the Qt Home dialog)."""
    if not available():
        return False
    p = page(win)
    p.open()
    return True


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
    """One window's start page: what to show, what was sent, the loader."""

    def __init__(self, win):
        self.win = win
        self.wanted = False
        self.drawn = False
        self.sent = None            # (shown, can_return) last sent
        self.rows_sig = None
        self.dirty = True
        self.docs_sig = None
        self.icons_sent = set()     # texture keys uploaded (per renderer)
        self.renderer = None
        self.loader = None
        self.poll = None
        self.dog = None
        self.focused = False
        self.asked = 0.0

    # -- state ------------------------------------------------------------
    def open(self) -> None:
        win = self.win
        for doc in list(getattr(win, "_documents", []) or []):
            if getattr(doc, "path", None) is not None:
                note(win, doc.path, doc)
        self.dirty = True
        if self.wanted:
            # Already up (asked again at the end of a cold start): fresh
            # rows, nothing typed lost.
            self.load()
            self.repaint()
            return
        self.wanted = True
        self.drawn = False
        self.focused = False
        self.asked = time.monotonic()
        self.docs_sig = _docs_sig(win)
        self.load()
        self.repaint()
        view = _view(win)
        if view is not None:
            try:
                view.setFocus(Qt.OtherFocusReason)
            except RuntimeError:
                pass
        if self.dog is None:
            self.dog = QTimer(win, singleShot=True)
            self.dog.timeout.connect(self._watchdog)
        self.dog.start(WATCHDOG_MS)

    def close(self) -> None:
        self.wanted = False
        if self.dog is not None:
            self.dog.stop()
        self.repaint()

    def _watchdog(self) -> None:
        # Nothing drew the page (no Metal view after all, the surface lost):
        # the Qt Home dialog, so the window never sits on an Untitled model
        # with nothing to choose from.
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
        _qt_home(win)

    def repaint(self) -> None:
        view = _view(self.win)
        if view is not None:
            try:
                view.update()
            except RuntimeError:
                pass

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
            self.dirty = True
            self.repaint()
        if not busy:
            self.poll.stop()
            if _take_news():
                self.dirty = True
                self.repaint()

    # -- a frame ----------------------------------------------------------
    def before_frame(self, view) -> None:
        renderer = view.renderer
        if renderer is None or not hasattr(renderer, "ui_show_start"):
            return
        if renderer is not self.renderer:
            self.renderer = renderer
            self.icons_sent = set()
            self.sent = None
            self.rows_sig = None
        win = self.win
        if self.wanted and _docs_sig(win) != self.docs_sig:
            # A document came in (⌘O, a drop, the OS): the page has done its job.
            self.close()
        back = can_return(win)
        state = (self.wanted, back)
        if state != self.sent:
            self.sent = state
            renderer.ui_show_start(self.wanted, back)
        if not self.wanted:
            return
        self.drawn = True
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
        if self.dirty:
            self.dirty = False
            rows = self.rows(back)
            icons = rows.pop("icons")
            fresh = [i for i in icons if i[0] not in self.icons_sent]
            if fresh:
                renderer.ui_set_icons(fresh)
                self.icons_sent.update(i[0] for i in fresh)
            sig = repr(rows)
            if sig != self.rows_sig:
                self.rows_sig = sig
                renderer.ui_set_start(rows["files"], rows["projects"], rows["templates"],
                                      rows["cont"], rows["footer"], rows["nav"], rows["links"])

    def rows(self, back: bool) -> dict:
        # Each picture once: the Continue card and its tile share one. Two
        # loads of a key between frames upload and free a texture in the
        # same frame, and egui-wgpu destroys it with its upload pending.
        icons = {}

        def thumb(path):
            c = _cached(path)
            if c is None or c.get("thumb") is None:
                return ""
            key, w, h, rgba = c["thumb"]
            icons[key] = (key, w, h, rgba)
            return key

        files = []
        for r in recent.entries("files"):
            path = r["path"]
            c = _cached(path) or {}
            m = recent.meta(path)
            files.append(([path, Path(path).name, _meta_line(m, c), m.get("project", ""), thumb(path)],
                          [int(bool(r.get("pinned"))), int(c.get("exists") is False)]))
        projects = []
        for r in recent.entries("projects"):
            path = r["path"]
            c = _cached(path) or {}
            members = c.get("files") or []
            n = len(members)
            if c.get("exists") is False:
                meta = "Not found"
            elif c.get("exists") and c.get("files") is None:
                meta = "Can't read this project"
            elif c.get("exists"):
                meta = f"{n} file" + ("s" if n != 1 else "")
            else:
                meta = ""
            projects.append(([path, Path(path).stem, meta],
                             [int(bool(r.get("pinned"))), int(c.get("exists") is False)],
                             [(f, Path(f).name, role) for f, role in members]))
        templates = [(t[0], t[1], t[2]) for t in TEMPLATES]
        return dict(files=files, projects=projects, templates=templates,
                    cont=self.continue_row(back, thumb), footer=footer(), nav=nav(),
                    links=[("prefs", "Preferences"), ("extensions", "Extensions"), ("help", "Help")],
                    icons=list(icons.values()))

    def continue_row(self, back, thumb) -> list:
        win = self.win
        if back:
            doc = getattr(win, "_doc", None)
            path = getattr(doc, "path", None)
            name = doc.display_name() if doc is not None and hasattr(doc, "display_name") else "Untitled"
            m = recent.meta(path) if path else {}
            place = " · ".join(x for x in (m.get("project", ""), _folder(path) if path else "") if x)
            return [str(path or ""), name, place, _last_in(m) or "In Model", _problems(m), "",
                    thumb(str(path)) if path else "", "1"]
        path = last_file()
        if not path:
            return []
        c = _cached(path) or {}
        if c.get("exists") is False:
            return []
        m = recent.meta(path)
        place = " · ".join(x for x in (m.get("project", ""), _folder(path)) if x)
        auto = "Auto-save is newer than the file." if c.get("autosave_newer") else ""
        return [path, Path(path).name, place, _last_in(m), _problems(m), auto, thumb(path), "0"]


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


def _folder(path) -> str:
    """The folder, short: under the home folder from ~, and only its last
    two levels when deeper (the full path is the card's tooltip)."""
    try:
        parent = Path(path).parent
        home = Path.home()
        parts = parent.relative_to(home).parts if parent.is_relative_to(home) else parent.parts[1:]
        head = "~/" if parent.is_relative_to(home) else "/"
        if len(parts) > 2:
            return "…/" + "/".join(parts[-2:])
        return head + "/".join(parts)
    except (TypeError, ValueError):
        return ""


def _ago(mtime: float) -> str:
    s = max(0.0, time.time() - mtime)
    if s < 90:
        return "just now"
    if s < 3600:
        return f"{int(s // 60)} min ago"
    if s < 86400:
        return f"{int(s // 3600)} h ago"
    if s < 86400 * 14:
        return f"{int(s // 86400)} d ago"
    return time.strftime("%d %b %Y", time.localtime(mtime))


def _meta_line(m: dict, c: dict) -> str:
    parts = []
    if c.get("exists") is False:
        parts.append("Not found — click to locate")
    elif c.get("mtime"):
        parts.append(_ago(c["mtime"]))
    if m.get("project"):
        parts.append(m["project"])
    n = int(m.get("links") or 0)
    if n:
        parts.append(f"links {n} file" + ("s" if n != 1 else ""))
    if m.get("imported"):
        parts.append("from " + {"skp": ".skp", "dwg": "DWG", "dxf": "DXF", "ifc": "IFC",
                                "rfa": "Revit"}.get(m["imported"], m["imported"].upper()))
    n = int(m.get("unfiled") or 0)
    if n:
        parts.append(f"{n} unfiled")
    return " · ".join(parts)


def _last_in(m: dict) -> str:
    where = m.get("workspace") or ""
    return f"Last in {where}" if where else ""


def _problems(m: dict) -> str:
    n = int(m.get("problems") or 0)
    return f"{n} item" + ("s" if n != 1 else "") + " to fix" if n else ""


def footer() -> str:
    try:
        from core._native import engine
        from core import renderer_settings
        return f"Engine: {engine()} · Renderer: {renderer_settings.status_text()}"
    except Exception:  # noqa: BLE001
        return ""


def nav() -> list:
    out = [("recent", "Recent"), ("projects", "Projects"), ("templates", "Templates")]
    if _libraries() is not None:
        out.append(("libraries", "Libraries"))
    return out


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


_NAMES = {Qt.Key_Escape: "Escape", Qt.Key_Return: "Enter", Qt.Key_Enter: "Enter",
          Qt.Key_Backspace: "Backspace", Qt.Key_Tab: "Tab", Qt.Key_Up: "ArrowUp",
          Qt.Key_Down: "ArrowDown", Qt.Key_Left: "ArrowLeft", Qt.Key_Right: "ArrowRight"}


def _page_key(view, event):
    """(name, text) when the key is the page's, else None. ⌘ and ⌃ keys
    stay the window's (⌘K, ⌘O, ⌘N…)."""
    from views import wgpu_ui
    win = wgpu_ui._window(view)
    if not shown(win):
        return None
    if event.modifiers() & (Qt.ControlModifier | Qt.MetaModifier):
        return None
    name = _NAMES.get(event.key(), "")
    text = event.text() if not name else ""
    if not name and not (text and text.isprintable()):
        return None
    return name, text


def claims_key(view, event) -> bool:
    """ShortcutOverride: the page's keys fire no window shortcut."""
    return _page_key(view, event) is not None


def key(view, event, pressed: bool) -> bool:
    """A key to the page first. True: it was the page's."""
    got = _page_key(view, event)
    renderer = getattr(view, "renderer", None)
    if got is None or renderer is None or not hasattr(renderer, "ui_start_key"):
        return False
    if not pressed:
        # The release goes where the press went: to the command palette's
        # field when it is open over the page, else nowhere.
        return not renderer.ui_claims_key(got[0])
    taken = renderer.ui_start_key(*got)
    if taken:
        view.update()
    return taken


def action(win, arg: str) -> None:
    """``start:<verb>[\\x1f<arg>]`` from the chrome."""
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
        p.dirty = True
        p.repaint()
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
        from views.units_start_dialog import offer_at_start
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


# ---- The Qt dialog, where the chrome cannot draw ------------------------------
def _qt_home(win) -> None:
    from views.home import Home
    win._home = Home(win)
    win._home.show()
    win._home.raise_()


def wants_at_start(argv_has_file: bool) -> bool:
    """main.py: the window opens on the page when no file is given."""
    return not argv_has_file and available() and os.environ.get("BUILT_START", "1") != "0"

"""Documents in the shell: open, save, close, recover, import, link, copy.

What MainWindow does for these is borrowed where it only needs the
document (the unsaved-changes question, File ▸ New, the recent list, the
import pipelines, the links verbs, Select All…: ``ShellWindow.__getattr__``).
What it does through widgets is redone here: the open (no trays to sync,
the shell's own progress turn), the save (the Metal view's picture), the
file dialogs (NSOpenPanel/NSSavePanel through ``app_shell.native``), the
DXF unit question (on the sheet instead of a QDialog), the .skp parse
thread (waited for with the modal pump instead of a QEventLoop), the
pasteboard.

Everything here answers through ``views/chrome_dialogs`` (the egui sheet,
synchronous thanks to ``app_shell.modal``), so the code paths are the Qt
app's own.

Commands (:func:`command`) come from the menu bar (rust/built_app/src/
menu.rs), the start page and the chrome's document ▾.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import threading
from pathlib import Path

from core.i18n import tr

#: What open_path reads as a document; the rest are imports.
IMPORTS = (".skp", ".dae", ".dxf", ".dwg", ".rfa", ".glb", ".gltf")


class _Abort(Exception):
    pass


# ---- the file dialogs: NSOpenPanel / NSSavePanel ----------------------------
class ShellFileDialog:
    """``QFileDialog``'s static pickers, as views/filedialogs.file_dialogs
    calls them, answered by the native panels (rfd)."""

    @staticmethod
    def _split(directory):
        directory = str(directory or "")
        p = Path(directory)
        if directory and not p.is_dir() and p.suffix:
            return str(p.parent), p.name
        return directory, ""

    @staticmethod
    def getOpenFileName(parent=None, caption="", dir="", filter="", *args, **kwargs):  # noqa: A002,N802
        folder, _ = ShellFileDialog._split(dir)
        from app_shell import native
        got = native.open_files(caption or "Open", filter, folder or None)
        return (got[0] if got else "", filter)

    @staticmethod
    def getOpenFileNames(parent=None, caption="", dir="", filter="", *args, **kwargs):  # noqa: A002,N802
        folder, _ = ShellFileDialog._split(dir)
        from app_shell import native
        return (native.open_files(caption or "Open", filter, folder or None, multiple=True), filter)

    @staticmethod
    def getSaveFileName(parent=None, caption="", dir="", filter="", *args, **kwargs):  # noqa: A002,N802
        folder, name = ShellFileDialog._split(dir)
        from app_shell import native
        got = native.save_file(caption or "Save", name, filter, folder or None)
        return (got or "", filter)

    @staticmethod
    def getExistingDirectory(parent=None, caption="", dir="", *args, **kwargs):  # noqa: A002,N802
        from app_shell import native
        return native.pick_folder(caption or "Choose a folder", str(dir or "") or None) or ""


def install(win) -> None:
    """The shell's pieces in the modules the borrowed code calls."""
    from views import filedialogs
    filedialogs.QFileDialog = ShellFileDialog
    # The shell draws with Metal by construction: no probe process, no
    # GL fallback to report (the start page's footer, the chrome's checks).
    from core import renderer_settings
    if not getattr(renderer_settings, "_shell_installed", False):
        renderer_settings.capability = lambda: renderer_settings.Capability(True)
        renderer_settings.status_text = lambda: "wgpu (native app)"
        renderer_settings._shell_installed = True


# ---- commands ------------------------------------------------------------------
def command(win, name, *args):
    """A command by name (menu bar, start page, document ▾). Returns what
    the command returns (open/save: whether it happened)."""
    if name in ("about", "extensions", "cutlist"):
        from views.report_sheets import show
        return show(win, name)
    if name == "delivery_export":
        from views.wgpu_deliveries import export_form
        return export_form(win, one_off=True)
    kind, _, arg = name.partition(":")
    table = {
        "new": lambda: new_model(win),
        "new_window": lambda: new_window(win),
        "move_to_window": lambda: move_to_new_window(win, args[0] if args else None),
        "open": lambda: open_dialog(win),
        "home": lambda: show_start(win),
        "close_tab": lambda: close_document(win, args[0] if args else None),
        "save": lambda: save(win),
        "save_as": lambda: save_as(win),
        "save_copy": lambda: save_copy(win),
        "export_image": lambda: win._on_export_image(),
        "soften": lambda: win._on_soften_edges(),
        "export_igz": lambda: export_igz(win),
        "link": lambda: link_document(win),
        "reload_links": lambda: win._reload_all_links(),
        "classic": lambda: open_in_classic(win),
        "prefs": lambda: open_settings(win),
        "library": lambda: open_library(win),
        "cut": lambda: clipboard_cut(win),
        "copy": lambda: clipboard_copy(win),
        "paste": lambda: clipboard_paste(win, in_place=False),
        "paste_in_place": lambda: clipboard_paste(win, in_place=True),
        "select_all": lambda: win._on_select_all(),
        "select_none": lambda: win._on_select_none(),
        "next_doc": lambda: step_document(win, 1),
        "prev_doc": lambda: step_document(win, -1),
        # The workspaces (views/wgpu_sheets.py): the top bar's workspace ▾.
        "sheets": lambda: win._to_sheets(),
        "model": lambda: win._show_model(),
    }
    if kind == "import":
        return import_dialog(win, arg)
    if kind == "recent":
        return open_recent_index(win, arg)
    if kind == "open_path":
        return open_path(win, Path(arg))
    # Scripted runs and harnesses: the same paths without the panels.
    if kind == "save_to":
        return do_save(win, Path(arg))
    if kind == "save_copy_to":
        return do_save(win, Path(arg), copy=True)
    if kind == "export_image_to":
        return win._export_image_to(Path(arg))
    if kind == "export_igz_to":
        return export_igz(win, Path(arg))
    if kind == "sheet_export_to":           # sheet_export_to:pdf:/path/file.pdf
        from views import wgpu_sheets
        what, _, path = arg.partition(":")
        ws = wgpu_sheets.workspace_of(win.view)
        return ws._export_to(what, path) if ws is not None else None
    if kind == "link_source":
        from views import wgpu_tabs
        return wgpu_tabs.open_link_source(win, Path(arg))
    if kind == "ctx_demo":
        from app_shell import harness
        return harness.context_menu_for(win, arg)
    if kind == "close_doc":
        docs = win._documents
        return close_document(win, docs[int(arg)] if arg else None)
    if kind == "activate":
        docs = win._documents
        i = int(arg)
        if 0 <= i < len(docs):
            win.activate_document(docs[i])
        return True
    fn = table.get(name)
    if fn is None:
        return None
    return fn()


def open_library(win) -> bool:
    """Window ▸ Library: the Library sheet (views/wgpu_library.py)."""
    from views import wgpu_library
    return wgpu_library.open_library(win)


def open_settings(win) -> bool:
    from views import settings_sheet
    return settings_sheet.show(win)


def step_document(win, step):
    nxt = win._project_tabs.neighbour(win._doc, step)
    if nxt is not None:
        win.activate_document(nxt)


def new_model(win) -> None:
    """File ▸ New (MainWindow._on_new: an untouched untitled one starts
    over, else a new project), and the start page goes."""
    win._on_new()
    win._dismiss_home()


def new_window(win) -> bool:
    """File ▸ New Window (⇧⌘N): a window on the start page, in this process
    (app_shell.windows; Rust makes it and hands it the token)."""
    from app_shell import windows
    windows.request(win._host, "new")
    return True


def move_to_new_window(win, doc=None) -> bool:
    """Window ▸ Move Project to New Window, and the tab menu's row."""
    from app_shell import windows
    return windows.move_to_new_window(win, doc or win._doc) is not None


def show_start(win) -> None:
    from views import wgpu_start
    wgpu_start.show(win)


# ---- open ------------------------------------------------------------------------
def open_dialog(win) -> bool:
    from formats import built
    from views.filedialogs import file_dialogs
    win.viewport.end_group_edit()
    path, _ = file_dialogs.getOpenFileName(
        win, tr("Open Built document"), "",
        built.DOCUMENT_FILTER + ";;Imports (*.skp *.dae *.dxf *.dwg *.rfa *.glb *.gltf)")
    if not path:
        return False
    return open_path(win, Path(path))


def open_recent_index(win, arg) -> bool:
    if arg == "clear":
        from PySide6.QtCore import QSettings
        QSettings().setValue("recent_files", [])
        win._host.recent_changed()
        return True
    paths = [p for p in win._recent_paths() if Path(p).exists()]
    try:
        path = Path(paths[int(arg)])
    except (ValueError, IndexError):
        return False
    from views import wgpu_start
    return wgpu_start.open_recent(win, path)


def open_path(win, path: Path) -> bool:
    """MainWindow.open_path for the shell: a file already open is shown;
    an untouched untitled project in front takes it, else a project of
    its own; imports go through their pipelines into it."""
    path = Path(path)
    host = win._host
    suffix = path.suffix.lower()
    if suffix == ".igzproj":
        return open_in_classic(win, path, why="Projects (.igzproj)")
    if win._closed:
        return False
    for doc in win._documents:
        if doc.same_file(path):
            win.activate_document(doc)
            win._dismiss_home()
            return True
    # A file opens only once: open in another window, that window and its
    # tab come to the front.
    from app_shell import windows
    other, doc = windows.find_document(path, exclude=win)
    if other is not None:
        other.window.activate_document(doc)
        other.window._dismiss_home()
        other.focus_requested = True
        other.request_frame()
        return True
    if not path.is_file():
        from views import chrome_dialogs as cd
        cd.warning(win, tr("Open"), tr("{name} is no longer there.", name=path.name))
        return False
    previous, created = win._doc, None
    if not win._doc_is_blank():
        created = win.new_document()
    if suffix in IMPORTS:
        ok = import_path(win, path)
    else:
        ok = open_document(win, path)
    if not ok and created is not None and created in win._documents and win._doc_is_blank(created):
        win.activate_document(previous, closing=True)
        win._retire_document(created)
    if ok:
        win._dismiss_home()
    host.request_frame()
    return ok


def open_document(win, path: Path) -> bool:
    """Read a .built/.igz into the project in front: recovery offer, the
    progress capsule with Cancel, camera and views home, the resident pack
    under the same capsule, links and the heap after the first picture
    (MainWindow._open_path_here's steps that are not about Qt panels)."""
    from PySide6.QtWidgets import QMessageBox
    from core import autosave
    from formats import igz as igz_format
    from views import chrome_dialogs as cd
    host = win._host
    vp = win.viewport
    doc = win._doc
    view = doc.stand_in
    recovered = False
    slot = autosave.pending(path)
    defer_recovery = slot is not None and cd.live(win) is not None
    if slot is not None and not defer_recovery:
        answer = cd.question(
            win, tr("Recover the auto-saved drawing?"),
            tr("A session with '{name}' ended without saving (auto-saved "
               "copy found). Recover it instead of the file on disk?", name=path.name),
            QMessageBox.Yes | QMessageBox.No, QMessageBox.Yes)
        recovered = answer == QMessageBox.Yes
        if not recovered:
            autosave.clear(path)
    bar, cb = win._import_progress(tr("Opening {name}…", name=path.name), monotonic=True)
    chrome = isinstance(bar, cd.ChromeBar)
    win._open_cancelled = False
    win._open_aborted = False
    if chrome:
        bar.canceled.connect(win._on_open_cancelled)
        bar.set_cancellable(True)

    def guarded(frac, text):
        if win._open_cancelled or win._open_aborted:
            raise _Abort()
        cb(frac, text)

    win._open_in_progress = True
    win._opening_document = True
    if hasattr(view, "blank") and view.renderer is not None:
        view.blank()
    try:
        try:
            igz_format.load_into(vp.scene, slot if recovered else path,
                                 progress=lambda f, t: guarded(0.6 * f, t))
        except _Abort:
            bar.close()
            win._finish_cancelled_open()
            return False
        except Exception as exc:  # noqa: BLE001 — any IO/parse error goes to the user
            cd.close_failed(bar, exc)
            host._error("open")
            cd.critical(win, tr("Open failed"), str(exc))
            return False
        import gc
        gc.set_threshold(win.GC_THRESHOLD0, 10, 10)
        if chrome:
            bar.set_cancellable(False)          # it is in: packing cannot stop
        vp.history.clear()
        try:
            vp.reset_texture_cache()
        except Exception:  # noqa: BLE001
            host._error("reset_texture_cache")
        win._forget_coordinate()
        vp.inference.reset()
        win._apply_camera_dict(getattr(vp.scene, "camera_home", None))
        if not isinstance(getattr(vp.scene, "camera_home", None), dict):
            win._on_zoom_extents()
        win._current_path = path
        win._remember_recent(path)
        host.recent_changed()
        win._import_name = None
        win._autosaved_version = None
        # A recovered drawing is not the file on disk yet: dirty until saved.
        win._saved_version = -1 if recovered else vp.scene.version
        repaired = getattr(vp.scene, "load_repairs", 0)
        if repaired:
            win._saved_version = -1
            cd.notify(win, tr("Document repaired"), tr(
                "{n} damaged pieces (a coordinate that is not a number) were left out so "
                "the rest of “{name}” could open. Save it to keep the repaired document.",
                n=repaired, name=path.name), tone="warning")
        vp.notify_scene_changed()
        clean = not win._is_dirty()
        # The pack and upload, under the same capsule (23 s on the ride
        # with nothing saying so, when the bar closed before it).
        version = vp.scene.version
        view.set_scene(vp.scene, progress=lambda f, t: cb(0.6 + 0.38 * f, t), opening=True)
        view._shell_resident_version = version
        if clean:
            win._saved_version = vp.scene.version
        host.after_picture(win._after_links_loaded)
        for job in list(getattr(view, "after_open", ()) or ()):
            host.after_picture(job)
        view.after_open = []
        host.after_picture(win.settle_heap)
        win._refresh_scene_tabs()
        win._update_title()
        vp._pick_warm_due = True
        cb(1.0, "Document loaded")
        cd.opened(bar, win, path)
        if defer_recovery:
            cd.offer_recovery(win, path, slot)
        return True
    finally:
        win._opening_document = False
        win._open_in_progress = False
        bar.close()
        host.request_frame()


# ---- the progress capsule ------------------------------------------------------
class NullBar:
    """The ChromeBar surface when no chrome can show it (headless)."""

    class _Sig:
        def connect(self, *_):
            pass

        def emit(self, *_):
            pass

    def __init__(self):
        self.canceled = NullBar._Sig()

    def setValue(self, *_):  # noqa: N802
        pass

    def value(self):
        return 0

    def setLabelText(self, *_):  # noqa: N802
        pass

    def setRange(self, *_):  # noqa: N802
        pass

    def setCancelButtonText(self, *_):  # noqa: N802
        pass

    def set_cancellable(self, *_):
        pass

    def wasCanceled(self):  # noqa: N802
        return False

    def isVisible(self):  # noqa: N802
        return False

    def reset(self):
        pass

    def hide(self):
        pass

    def close(self):
        return True

    def deleteLater(self):  # noqa: N802
        pass


def import_progress(win, title, *, monotonic=False):
    """MainWindow._import_progress: the capsule in the top bar, whose
    callback turns the modal pump so it moves and Cancel works."""
    from views import chrome_dialogs as cd
    from app_shell import modal
    if cd.live(win) is None:
        return NullBar(), (lambda frac, text: None)
    bar = cd.ChromeBar(win, title)
    last = [0]

    def cb(frac, text):
        value = max(0, min(100, int(frac * 100)))
        last[0] = max(last[0], value) if monotonic else value
        bar.setValue(last[0])
        bar.setLabelText(tr(text))
        modal.tick()
    return bar, cb


def parse_skp_threaded(win, skp, cb):
    """MainWindow._parse_skp_threaded without a QEventLoop: the parse on a
    Python thread, its progress relayed on the main thread while the modal
    pump keeps the window drawing."""
    from formats import skp as skp_format
    from app_shell import modal
    result: dict = {}
    latest: list = []

    def run():
        try:
            result["payload"] = skp_format.parse_skp(
                skp, progress=lambda f, t: latest.append((f, t)))
            result["exc"] = None
        except Exception as exc:  # noqa: BLE001 — reported to the caller
            result["payload"] = None
            result["exc"] = exc

    worker = threading.Thread(target=run, name="skp-parse", daemon=True)
    worker.start()

    def done():
        if latest:
            f, t = latest[-1]
            latest.clear()
            cb(f, t)
        return not worker.is_alive()
    if not modal.run_until(done):
        worker.join()
    while latest:
        cb(*latest.pop(0))
    return result.get("payload"), result.get("exc")


# ---- imports --------------------------------------------------------------------
_IMPORT_FILTERS = {
    "skp": ("Import .skp", ".skp (*.skp)"),
    "dae": ("Import COLLADA", "COLLADA (*.dae)"),
    "dxf": ("Import DXF", "AutoCAD DXF (*.dxf)"),
    "dwg": ("Import DWG", "AutoCAD DWG (*.dwg)"),
    "rfa": ("Import Revit family", "Revit family (*.rfa)"),
    "glb": ("Import glTF/GLB", "glTF (*.glb *.gltf)"),
    "igz": ("Import a model as a group", "Built documents (*.built *.igz)"),
}


def import_dialog(win, kind) -> bool:
    title, filt = _IMPORT_FILTERS.get(kind, ("Import", "All files (*)"))
    from views.filedialogs import file_dialogs
    win.viewport.end_group_edit()
    path, _ = file_dialogs.getOpenFileName(win, tr(title), "", filt)
    if not path:
        return False
    if kind == "igz":
        return bool(win.import_igz_path(Path(path)))
    return import_path(win, Path(path))


def import_path(win, path: Path) -> bool:
    """Into the project in front, through MainWindow's own pipelines (one
    undo step, the capsule, the import report)."""
    suffix = path.suffix.lower()
    fn = {".rfa": win._import_rfa_path, ".dae": win._import_dae_path,
          ".skp": win.import_skp_path, ".dxf": win._import_dxf_path,
          ".dwg": win._import_dwg_path, ".glb": win._import_glb_path,
          ".gltf": win._import_glb_path}.get(suffix)
    if fn is None:
        win._host.not_here(f"{suffix} import")
        return False
    ok = bool(fn(path))
    win._host.request_frame()
    return ok


def dxf_options(win, path, header_scale=None):
    """MainWindow._dxf_options on the sheet: the unit, preselected from the
    header or the last answer; faces as last chosen (the classic app's
    dialog has the checkbox)."""
    from PySide6.QtCore import QSettings
    from views import chrome_dialogs as cd
    keys = ["mm", "cm", "m", "in", "ft"]
    factors = {"mm": 0.001, "cm": 0.01, "m": 1.0, "in": 0.0254, "ft": 0.3048}
    labels = {"mm": tr("Millimetres"), "cm": tr("Centimetres"), "m": tr("Metres"),
              "in": tr("Inches"), "ft": tr("Feet")}
    by_scale = {v: k for k, v in factors.items()}
    last = by_scale.get(header_scale) or str(QSettings().value("import/dxf_unit", "mm") or "mm")
    if last not in keys:
        last = "mm"
    faces = QSettings().value("import/dxf_make_faces", False, type=bool)
    got = cd.ask(win, tr("What unit is this drawing in?"),
                 tr("What unit is this drawing in? (CAD headers often lie)") + "\n" +
                 (tr("Faces from closed outlines: on") if faces else tr("Faces from closed outlines: off")),
                 [("cancel", tr("Cancel"))] + [(k, labels[k]) for k in keys],
                 default=last, cancel="cancel")
    if got is None:
        got = last                      # headless: the preselected unit
    if got == "cancel":
        return None
    QSettings().setValue("import/dxf_unit", got)
    return factors[got], bool(faces)


def link_document(win) -> bool:
    """File ▸ Link a Document…: another document placed as a live link,
    with its own nested links (the classic dialog's checkbox, on)."""
    from formats import built
    from views.filedialogs import file_dialogs
    path, _ = file_dialogs.getOpenFileName(win, tr("Link a document"), "", built.DOCUMENT_FILTER)
    if not path:
        return False
    try:
        return bool(win.link_igz_path(Path(path)))
    except Exception:  # noqa: BLE001
        win._host._error("link")
        return False


# ---- save ----------------------------------------------------------------------
def save(win) -> bool:
    from formats import built
    win._end_group_edits()
    path = win._current_path
    if path is None or not built.is_built(path):
        return save_as(win)
    return do_save(win, Path(path))


def _ask_save_path(win, caption, default):
    from formats import built
    from views.filedialogs import file_dialogs
    chosen, _ = file_dialogs.getSaveFileName(win, caption, str(default), built.BUILT_FILTER)
    if not chosen:
        return None
    path = Path(chosen)
    if not built.is_built(path):
        path = path.with_suffix(built.BUILT_SUFFIX)
    return path


def save_as(win) -> bool:
    from formats import built
    win._end_group_edits()
    default = (win._current_path or Path(win._doc.display_name())).with_suffix(built.BUILT_SUFFIX)
    path = _ask_save_path(win, tr("Save Built document"), default)
    if path is None:
        return False
    return do_save(win, path)


def save_copy(win) -> bool:
    """File ▸ Save a Copy…: the document as it is, to another .built; the
    window keeps its own file, name and unsaved state."""
    from formats import built
    win._end_group_edits()
    base = win._current_path or Path(win._doc.display_name())
    default = base.with_name(base.stem + " copy" + built.BUILT_SUFFIX)
    path = _ask_save_path(win, tr("Save a Copy"), default)
    if path is None:
        return False
    return do_save(win, path, copy=True)


def _picture_png(win, doc):
    """The picture a save keeps in the file (MainWindow._document_picture
    under Metal): one small offscreen frame of the resident model."""
    from views.wgpu_start import blank_picture
    view = getattr(doc, "stand_in", None)
    if view is None or getattr(view, "renderer", None) is None or getattr(view, "packed", None) is None:
        return None
    try:
        image = view.render_image(win.SAVE_PICTURE_PX)
    except Exception:  # noqa: BLE001 — a missing picture only
        return None
    if image is None or blank_picture(image):
        return None
    from PySide6.QtCore import QBuffer, QByteArray, QIODevice
    data = QByteArray()
    buffer = QBuffer(data)
    buffer.open(QIODevice.WriteOnly)
    image.save(buffer, "PNG")
    return bytes(data)


def _save_progress(job):
    if job is None:
        return None
    from app_shell import modal

    def update(fraction, phase):
        job.update(fraction, tr(phase))
        modal.tick()

    return update


def do_save(win, path: Path, copy: bool = False) -> bool:
    """MainWindow._do_save: the project's own scene and camera, links
    relative to where it goes, its picture; the previous file kept as
    .bak by the save itself (formats.igz)."""
    from core import autosave
    from core import links as _links
    from formats import igz as igz_format
    from views import chrome_dialogs as cd
    doc = win._doc
    vp = doc.viewport
    scene = vp.scene
    try:
        scene.camera_home = win._camera_dict(vp)
        scene.views_home = None
        _links.refresh_paths(scene, path)
        png = _picture_png(win, doc)
        if png is not None:
            scene._home_thumbnail_png = png
        with cd.busy(win, tr("Saving {name}…", name=path.name), output_path=path, exclusive=True) as job:
            if job is None:
                stats = igz_format.save_scene(scene, path) or {}
            else:
                stats = igz_format.save_scene(scene, path, progress=_save_progress(job)) or {}
    except Exception as exc:  # noqa: BLE001
        win._host._error("save")
        cd.save_failed(win, exc, lambda: save_copy(win) if copy else save_as(win))
        return False
    finally:
        if copy and doc.path is not None:
            try:
                _links.refresh_paths(scene, Path(doc.path))
            except Exception:  # noqa: BLE001
                pass
    missing = int(stats.get("missing", 0))
    embedded = int(stats.get("embedded", 0))
    if missing:
        cd.warning(win, tr("Saved with missing textures"), tr(
            "{n} texture image(s) could not be read, so they were not packed into the "
            "document — those faces will lose their texture on another computer.", n=missing))
    elif stats.get("backup_failed"):
        win.statusBar().showMessage(tr("Could not write the backup copy."), 4000)
    if copy:
        win.statusBar().showMessage(tr("Saved a copy as {name}", name=path.name), 4000)
        return True
    # A clean save ends the recovery window for this document and for what
    # it was called until now (untitled, or the old name of a Save As).
    autosave.clear(path)
    autosave.clear(doc.path, untitled=doc.untitled_no)
    if not missing and not stats.get("backup_failed"):
        win.statusBar().showMessage(
            tr("Saved — {n} texture(s) packed into the document.", n=embedded) if embedded
            else tr("Saved {name}", name=path.name), 4000)
    doc.path = path
    win._remember_recent(path)
    win._host.recent_changed()
    win._saved_version = scene.version
    win._update_title()
    return True


def export_igz(win, path=None) -> bool:
    """File ▸ Save a Copy as IngeTrazo (.igz): float32 geometry, after
    saying how far that rounding moves points."""
    import copy as _copy
    from PySide6.QtWidgets import QMessageBox
    from formats import igz as igz_format
    from views import chrome_dialogs as cd
    from views.filedialogs import file_dialogs
    win._end_group_edits()
    doc = win._doc
    if path is None:
        default = (doc.path or Path(doc.display_name())).with_suffix(".igz")
        chosen, _ = file_dialogs.getSaveFileName(win, tr("Save a Copy as IngeTrazo"), str(default),
                                                 "IngeTrazo (*.igz)")
        if not chosen:
            return False
        path = Path(chosen)
    path = Path(path).with_suffix(".igz")

    def confirm(mm):
        return cd.question(win, tr("Save the rounded copy?"), tr(
            "Float32 rounding moves points by up to {mm} mm. Save the copy?",
            mm=f"{mm:.6g}")) == QMessageBox.Yes

    try:
        scene = _copy.copy(doc.viewport.scene)
        scene.camera_home = win._camera_dict(doc.viewport)
        scene.views_home = None
        png = _picture_png(win, doc)
        if png is not None:
            scene._home_thumbnail_png = png
        activity = cd.busy(win, tr("Saving {name}…", name=path.name), exclusive=True)
        with activity as job:
            if job is None:
                stats = igz_format.export_igz(scene, path, confirm)
            else:
                stats = igz_format.export_igz(scene, path, confirm, progress=_save_progress(job))
            if stats is None and job is not None:
                job.cancelled = True
            elif stats:
                activity.output_path = path
        if stats and stats.get("missing"):
            cd.warning(win, tr("Saved with missing textures"), tr("Some texture images could not be packed."))
    except Exception as exc:  # noqa: BLE001
        win._host._error("export igz")
        cd.save_failed(win, exc, win._on_export_igz)
        return False
    if stats is None:
        return False                    # the precision question said no
    win.statusBar().showMessage(tr("Saved a copy as {name}", name=path.name), 4000)
    return True


# ---- close and quit ---------------------------------------------------------------
def close_document(win, doc=None) -> bool:
    """Close a project (⌘W, document ▾), asking about ITS unsaved changes.
    The last one closed leaves an empty project behind the start page."""
    doc = doc or win._doc
    if doc not in win._documents:
        return True
    from views import chrome_dialogs as cd
    if cd.current_sheet(win) is not None or cd.modal(win):
        return False
    if win._doc_is_dirty(doc):
        if doc is not win._doc:
            win.activate_document(doc)
        win.viewport.end_group_edit()
        if not win._confirm_discard(tr("Close “{name}”?", name=doc.display_name())):
            return False
    last = len(win._documents) == 1
    if last:
        win.new_document(activate=False)
    if doc is win._doc:
        docs = win._documents
        i = docs.index(doc)
        nxt = docs[i + 1] if i + 1 < len(docs) else docs[i - 1]
        win.activate_document(nxt, closing=True)
    win._retire_document(doc)
    try:
        win._link_watcher().refresh()
    except Exception:  # noqa: BLE001
        pass
    if last:
        win._start_blank_document()
        show_start(win)
    win._update_title()
    return True


def request_close(win) -> bool:
    """The window's close and Quit (MainWindow.closeEvent's checks): no
    while a question waits or a file is coming in; one question for
    every project with unsaved changes; then the recovery slots go."""
    from core import autosave
    from views import chrome_dialogs as cd
    if cd.current_sheet(win) is not None:
        return False
    if win._open_in_progress:
        win._open_aborted = True        # the load unwinds at its next step
        return False
    if not win._confirm_quit():
        return False
    for doc in win._documents:
        try:
            autosave.clear(doc.path, untitled=doc.untitled_no)
        except Exception:  # noqa: BLE001
            pass
    auto = win.__dict__.get("_background_autosave")
    if auto is not None:
        auto.cancel()
    win._closed = True
    return True


# ---- the start of a session ---------------------------------------------------------
def cold_start(win) -> None:
    """No file given: the untitled drawings an earlier session left in its
    auto-save slots are offered back, then the start page (unless one of
    them came back)."""
    try:
        win._offer_untitled_recovery()
    except Exception:  # noqa: BLE001
        win._host._error("untitled recovery")
    if win._doc_is_blank():
        win._start_blank_document()
        show_start(win)


# ---- the classic app --------------------------------------------------------------
def classic_command(path=None) -> list:
    """How to start the Qt app on ``path``: ``BUILT_CLASSIC`` (a command
    line) when set, else the checkout's main.py with its venv's Python."""
    from app_shell import host as _host
    extra = [str(path)] if path else []
    env = os.environ.get("BUILT_CLASSIC")
    if env:
        import shlex
        return shlex.split(env) + extra
    root = Path(_host.ROOT or Path(__file__).resolve().parent.parent)
    venv = _host.VENV
    python = None
    if venv:
        for name in ("python3", "python"):
            cand = Path(venv) / "bin" / name
            if cand.exists():
                python = str(cand)
                break
    python = python or shutil.which("python3") or sys.executable
    return [python, str(root / "main.py"), "--new-window", *extra]


def open_in_classic(win, path=None, why: str | None = None) -> bool:
    """Open the document (or ``path``) in the classic Qt app: what the
    native app cannot do yet stays one click away. Unsaved changes are
    saved first, when the user says so: the classic app reads the file."""
    from views import chrome_dialogs as cd
    if path is None:
        doc = win._doc
        if doc is not None and win._doc_is_dirty(doc):
            got = cd.ask(win, tr("Save before opening in the classic app?"),
                         tr("The classic app opens the file on disk. Save your changes first?"),
                         [("discard", tr("Open Without Saving")), ("cancel", tr("Cancel")),
                          ("save", tr("Save and Open"))],
                         default="save", cancel="cancel", discard="discard")
            if got == "cancel":
                return False
            if got == "save" and not save(win):
                return False
        path = getattr(doc, "path", None)
    cmd = classic_command(path)
    try:
        subprocess.Popen(cmd, start_new_session=True, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL, cwd=str(Path(cmd[1]).parent) if len(cmd) > 1 else None)
    except OSError as exc:
        cd.warning(win, tr("Open in Classic App"), str(exc))
        return False
    what = f"{why} open" if why else "Opening"
    win.statusBar().showMessage(f"{what} in the classic app…", 4000)
    return True


# ---- the clipboard: the Mac's pasteboard -------------------------------------------------
#: The pasteboard type Qt's macOS clipboard gives formats.clip.MIME, so
#: the classic app and the native one paste each other's copies.
PASTEBOARD_TYPE = "com.trolltech.anymime.application--x-ingetrazo-clipboard"
_OWN_COUNT = [None]
#: The clipboard dict that write was made from (the project that copied).
_OWN_CLIP = [None]


def _publish(win) -> None:
    from app_shell import native
    from formats import clip as clip_transfer
    clip = getattr(win.viewport, "clipboard", None)
    if not clip or not native.available():
        return
    try:
        data = clip_transfer.encode(clip)
    except Exception:  # noqa: BLE001 — the in-app copy still works
        win._host._error("clipboard encode")
        return
    if native.pasteboard_write([(PASTEBOARD_TYPE, data)]):
        _OWN_COUNT[0] = native.pasteboard_types()[1]
        _OWN_CLIP[0] = clip


def clipboard_copy(win) -> None:
    if _field_has_focus(win):
        return
    if win.viewport.copy_selection():
        _publish(win)


def clipboard_cut(win) -> None:
    if _field_has_focus(win):
        return
    if win.viewport.cut_selection():
        _publish(win)


def sync_foreign_clipboard(win) -> None:
    """MainWindow._sync_foreign_clipboard over the pasteboard: a copy made
    in another Built (this app or the classic one) wins over this
    window's older one; a copy in another project of this window is
    adopted through the same encoding."""
    from app_shell import native
    from formats import clip as clip_transfer
    types, count = native.pasteboard_types()
    if PASTEBOARD_TYPE not in types:
        return
    vp = win.viewport
    if count == _OWN_COUNT[0] and _OWN_CLIP[0] is not None and (
            vp.clipboard is _OWN_CLIP[0]
            or getattr(vp, "_clip_adopted_from", None) is _OWN_CLIP[0]):
        return                          # this project's copy, or adopted already
    data = native.pasteboard_read(PASTEBOARD_TYPE)
    other = clip_transfer.decode(data) if data else None
    if other is None:
        return
    old = vp.clipboard
    drop = getattr(vp, "_drop_clip_protos", None)
    if old and callable(drop):
        drop(old)
    vp.clipboard = other
    if count == _OWN_COUNT[0]:
        vp._clip_adopted_from = _OWN_CLIP[0]   # another project's copy here
    else:
        # Another app's copy: from now on it is the one to compare with.
        _OWN_COUNT[0], _OWN_CLIP[0] = count, other
        vp._clip_adopted_from = None


def _field_has_focus(win) -> bool:
    view = win._doc.stand_in if win._doc is not None else None
    renderer = getattr(view, "renderer", None)
    try:
        return bool(renderer is not None and renderer.ui_claims_key(""))
    except Exception:  # noqa: BLE001
        return False


def clipboard_paste(win, in_place: bool) -> None:
    from app_shell import native
    view = win._doc.stand_in
    if _field_has_focus(win):
        data = native.pasteboard_read("public.utf8-plain-text")
        if data:
            view.renderer.ui_text(data.decode("utf-8", "replace"))
            view.update()
        return
    sync_foreign_clipboard(win)
    if in_place:
        win._on_paste_in_place()
    else:
        win._on_paste()
        for action in win._tool_actions.values():
            action.setChecked(False)
    view.tools.sync()
    view.update()

"""Documents in the shell: open, save, close, recover, import, link, copy.

The shell binds document commands to the active view, native file panels and
the model clipboard. Import, link and recovery operations also call the
existing document backends and the shared chrome dialogs.

Commands (:func:`command`) come from the menu bar (rust/built_app/src/
menu.rs), the start page and the chrome's document ▾.
"""
from __future__ import annotations

import threading
from collections import deque
from pathlib import Path

from ingetrazo_render import ModelClipboard, cold_start_document as cold_start, request_close

from core.i18n import tr

_clipboard = ModelClipboard()
publish_clipboard = _clipboard.publish

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
    """Install the shell’s bindings for document commands."""
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
        from ingetrazo_render import show_report_sheet as show
        return show(win, name)
    if name == "delivery_export":
        from ingetrazo_render import delivery_export_form
        return delivery_export_form(win, one_off=True)
    if name in ("cut", "copy", "paste", "paste_in_place"):
        return _clipboard.run(win, name)
    kind, _, arg = name.partition(":")
    if kind == "extension":
        return win._extension_menus.trigger(name)
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
        "units": lambda: win._on_units(),
        "check_updates": lambda: win._on_check_updates(),
        "check_updates_quiet": lambda: win.check_updates_quietly(),
        "export_view_svg": lambda: win._on_export_view_svg(),
        "export_image": lambda: win._on_export_image(),
        "soften": lambda: win._on_soften_edges(),
        "export_igz": lambda: export_igz(win),
        "link": lambda: link_document(win),
        "reload_links": lambda: win._reload_all_links(),
        "prefs": lambda: open_settings(win),
        "library": lambda: open_library(win),
        "undo": lambda: win._on_undo(),
        "redo": lambda: win._on_redo(),
        "select_all": lambda: win._on_select_all(),
        "select_none": lambda: win._on_select_none(),
        "next_doc": lambda: step_document(win, 1),
        "prev_doc": lambda: step_document(win, -1),
        # The workspaces (views/wgpu_sheets.py): the top bar's workspace ▾.
        "sheets": lambda: win._to_sheets(),
        "model": lambda: win._show_model(),
    }
    if kind == "import" and arg in _IMPORT_FILTERS:
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
        from ingetrazo_render import context_menu_demo
        return context_menu_demo(win, arg)
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
    if fn is not None:
        return fn()
    from ingetrazo_render import run_model_menu
    return run_model_menu(win, name) or None


def open_library(win) -> bool:
    """Window ▸ Library: the Library sheet (ingetrazo_render.wgpu_library)."""
    from ingetrazo_render import wgpu_library
    return wgpu_library.open_library(win)


def open_settings(win) -> bool:
    from ingetrazo_render import settings_sheet
    return settings_sheet.show(win)


def step_document(win, step):
    nxt = win._project_tabs.neighbour(win._doc, step)
    if nxt is not None:
        win.activate_document(nxt)


def new_model(win) -> None:
    """File ▸ New (MainWindow._on_new: an untouched untitled one starts
    over, else a new project), and the start page goes."""
    from views import wgpu_start
    if win._workspace is not None:
        new = getattr(win._workspace, "new", None)
        if new is not None:
            new()
        return
    if wgpu_start.shown(win):
        return wgpu_start.new_model(win)
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
    opener = getattr(win._workspace, "open", None)
    if opener is not None:
        opener()
        return True
    from formats import built
    from views.filedialogs import file_dialogs
    win.viewport.end_group_edit()
    path, _ = file_dialogs.getOpenFileName(
        win, tr("Open Built document"), "",
        built.DOCUMENT_FILTER + ";;Built projects (*.builtproject *.igzproj)"
        ";;Imports (*.skp *.dae *.dxf *.dwg *.rfa *.glb *.gltf *.obj *.stl)")
    if not path:
        return False
    return open_path(win, Path(path))


def open_recent_index(win, arg) -> bool:
    if arg == "clear":
        from core import recent
        recent.clear()
        win._host.recent_changed()
        return True
    paths = win._recent_paths()
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
    if win._closed:
        return False
    opener = win.file_openers.get(suffix)
    if opener is not None:
        return bool(opener(path))
    if win._workspace is not None and not win.leave_workspace():
        return False
    if suffix in (".builtproject", ".igzproj"):
        # Built projects open here (app_shell/project.py); an .igzproj
        # is converted in place first.
        from app_shell import project
        return project.open_project(win, path)
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
    if suffix in (".obj", ".stl"):
        # Unitless meshes need an answer before an Open creates a tab.
        return win._mesh_import_options(path, open_tab=lambda load: _open_in_tab(win, load)) == "chrome"
    if suffix == ".skp" and host.ns_view:
        from views.document_open import install_skp, start
        return start(win, path, install_skp)
    if suffix not in IMPORTS and host.ns_view:
        from views.document_open import start
        from functools import partial
        from views.wgpu_ui import free_area
        # A Finder/CLI open can arrive before the first frame. Its camera
        # fit uses the initial aperture, before the dock and tool rail appear.
        initial_frame = None
        if free_area(win) is None and not host._picture_seen:
            initial_frame = (win.viewport.camera.aspect, win.view.width(), win.view.height())
        return start(win, path, partial(_install_open, initial_frame=initial_frame))
    loader = import_path if suffix in IMPORTS else open_document
    return _open_in_tab(win, lambda: loader(win, path))


def _open_in_tab(win, load):
    previous, created = win._doc, None
    if not win._doc_is_blank():
        created = win.new_document()
    ok = load()
    if not ok and created is not None and created in win._documents and win._doc_is_blank(created):
        win.activate_document(previous, closing=True)
        win._retire_document(created)
    if ok:
        win._dismiss_home()
    win._host.request_frame()
    return ok


def open_document(win, path: Path) -> bool:
    """Read a .built/.igz into the project in front: recovery offer, the
    progress capsule with Cancel, camera and views home, the resident pack
    under the same capsule, links and the heap after the first picture
    (MainWindow._open_path_here's steps that are not about Qt panels)."""
    from core import autosave
    from formats import igz as igz_format
    from views import chrome_dialogs as cd
    host = win._host
    doc = win._doc
    vp = doc.viewport
    view = doc.stand_in
    recovered = False
    slot = autosave.pending(path)
    defer_recovery = slot is not None and cd.live(win) is not None
    if slot is not None and not defer_recovery:
        from ingetrazo_render import named_recovery
        recovered = named_recovery(win, path)
        if recovered is None:
            return False
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
        return _finish_open(win, path, recovered, slot if defer_recovery else None, bar, cb)
    finally:
        win._opening_document = False
        win._open_in_progress = False
        bar.close()
        host.request_frame()


def _install_open(win, path, bar, result, *, initial_frame=None):
    from core import autosave
    scene, packed, tracker, tree = result
    doc = win._doc
    doc.model_views.suspend(doc)
    history = doc.viewport.history
    history.scene = scene
    doc.viewport.set_document(scene, history)
    initial_aspect = initial_frame[0] if initial_frame and initial_frame[1:] == (win.view.width(), win.view.height()) else None
    return _finish_open(win, path, False, autosave.pending(path), bar, bar.job.update,
                        (packed, tracker, tree), initial_aspect)


def _finish_open(win, path, recovered, slot, bar, cb, prepared=None, initial_aspect=None):
    from views import chrome_dialogs as cd
    win._doc.model_views.restore(win._doc)
    host, vp, view = win._host, win._doc.viewport, win.view
    import gc
    gc.set_threshold(win.GC_THRESHOLD0, 10, 10)
    vp.history.clear()
    try:
        vp.reset_texture_cache()
    except Exception:  # noqa: BLE001
        host._error("reset_texture_cache")
    win._forget_coordinate()
    vp.inference.reset()
    win._apply_camera_dict(getattr(vp.scene, "camera_home", None))
    if not isinstance(getattr(vp.scene, "camera_home", None), dict):
        if initial_aspect is None:
            win._on_zoom_extents()
        else:
            from views.wgpu_ui import free_area
            bounds = win._extents_bounds()
            area = free_area(win)
            if bounds is not None:
                camera = vp.camera
                aspect, camera.aspect = camera.aspect, initial_aspect
                try:
                    camera.fit_box(*bounds, free=(-1., area[1], 1., area[3]) if area else None)
                finally:
                    camera.aspect = aspect
    win._current_path = path
    if prepared is None:
        win._remember_recent(path)
        host.recent_changed()
    else:
        doc = win._doc
        def remember():
            if doc in win.documents() and doc.path == path:
                win._remember_recent(path, doc=doc)
                host.recent_changed()
        host.after_picture(remember)
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
    if prepared is None:
        view.set_scene(vp.scene, progress=lambda f, t: cb(0.6 + 0.38 * f, t), opening=True)
    else:
        view.set_scene(vp.scene, opening=True, prepared=prepared)
    view._shell_resident_version = version
    if clean:
        win._saved_version = vp.scene.version
    host.after_picture(win._after_links_loaded)
    for job in list(getattr(view, "after_open", ()) or ()):
        host.after_picture(job)
    view.after_open = []
    if prepared is None:
        host.after_picture(win.settle_heap)
    win._refresh_scene_tabs()
    win._update_title()
    vp._pick_warm_due = True
    cb(1.0, "Document loaded")
    cd.opened(bar, win, path)
    if slot is not None:
        cd.offer_recovery(win, path, slot)
    return True


# ---- the progress capsule ------------------------------------------------------
class NullBar:
    """The ChromeBar surface when no chrome can show it (headless)."""

    class _Sig:
        def connect(self, *_):
            pass

        disconnect = connect

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


def parse_threaded(parser, path, cb, **options):
    """Parse a mesh on a worker and deliver progress on the window thread."""
    from app_shell import modal
    result: dict = {}
    latest = deque(maxlen=1)

    def run():
        try:
            result["payload"] = parser(path, progress=lambda f, t: latest.append((f, t)), **options)
            result["exc"] = None
        except Exception as exc:  # noqa: BLE001 — reported to the caller
            result["payload"] = None
            result["exc"] = exc

    worker = threading.Thread(target=run, name=parser.__name__, daemon=True)
    worker.start()

    def done():
        if latest:
            cb(*latest.popleft())
        return not worker.is_alive()
    if not modal.run_until(done):
        worker.join()
    while latest:
        cb(*latest.popleft())
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
    """Run the selected importer against the active document."""
    method = {".rfa": "_import_rfa_path", ".dae": "_import_dae_path",
              ".skp": "import_skp_path", ".dxf": "_import_dxf_path",
              ".dwg": "_import_dwg_path", ".glb": "_import_glb_path",
              ".gltf": "_import_glb_path"}.get(path.suffix.lower())
    if method is None:
        win._host.not_here(f"{path.suffix} import")
        return False
    ok = bool(getattr(win, method)(path))
    win._host.request_frame()
    return ok


def link_document(win) -> bool:
    """Choose a source file, then ask which of its contents to place."""
    from formats import built
    from views.filedialogs import file_dialogs
    path, _ = file_dialogs.getOpenFileName(win, tr("Link a document"), "", built.DOCUMENT_FILTER)
    if not path:
        return False
    win._choose_link_contents(path)
    return True


# ---- save ----------------------------------------------------------------------
def save(win) -> bool:
    if win._workspace is not None:
        result = win._workspace.save()
        win._update_title()
        return result is not False
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
    if win._workspace is not None:
        result = win._workspace.save_as()
        win._update_title()
        return result is not False
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
    from ingetrazo_render import archive_picture_png
    view = getattr(doc, "stand_in", None)
    if view is None or getattr(view, "renderer", None) is None or getattr(view, "packed", None) is None:
        return None
    try:
        return archive_picture_png(*view.render_pixels(win.SAVE_PICTURE_PX))
    except Exception:  # noqa: BLE001 — a missing picture only
        return None


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
    from app_shell import project
    if win._workspace is not None:
        return False
    doc = win._doc
    vp = doc.viewport
    scene = vp.scene
    try:
        if not copy:
            project.before_save(win, doc, path)
        scene.camera_home = win._camera_dict(vp)
        scene.views_home = win._views_dict(doc)
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
    project.saved(win, doc)
    win._update_title()
    return True


def export_igz(win, path=None) -> bool:
    """File ▸ Save a Copy as IngeTrazo (.igz): float32 geometry, after
    saying how far that rounding moves points."""
    import copy as _copy
    from views.chrome_dialogs import QMessageBox
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
        scene.views_home = win._views_dict(doc)
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
def close_document(win, doc=None, ask=True) -> bool:
    """Close a project (⌘W, document ▾), asking about ITS unsaved changes.
    The last one closed leaves an empty project behind the start page."""
    doc = doc or win._doc
    if doc not in win._documents:
        return True
    from views import chrome_dialogs as cd
    if cd.current_sheet(win) is not None or cd.modal(win):
        return False
    from ingetrazo_render import confirm_close_tab
    if not confirm_close_tab(win, doc, ask):
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


def sync_foreign_clipboard(win):
    _clipboard.adopt(win)

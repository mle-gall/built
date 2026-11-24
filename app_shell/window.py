"""What views/wgpu_ui.py and the tools ask of "the window", without one.

The egui chrome reads the window's tool actions (checked state, text,
shortcut), its status bar, and runs ``_activate_tool``. :class:`ShellWindow`
answers those from the same tool registry MainWindow builds, so the chrome
code runs unchanged.

**MainWindow's own Python, on demand.** Every method, property and
constant of ``views.main_window.MainWindow`` (and its worksession mixin)
that the shell does not define is borrowed the first time something asks
for it (:meth:`ShellWindow.__getattr__`): Select All,
Undo, the links menu's verbs, the unsaved-changes question, File ▸ New…
run as the Qt app runs them. What they reach for and the shell lacks (a
dock, a tray, a Qt menu) raises inside them, and the host reports it as
"not in the shell yet" with the classic app as the way out. What was
looked up and is nowhere is still counted in
:data:`views.view_host.MISSING`.

**Documents.** Each open project is a ``views.documents.Document`` with
its own controller (``DocumentController``: scene, history, camera) and its own
view object (``ShellView``: the packed scene, picking, inference index,
tool dispatch). One renderer, the window's surface, is handed to the
document in front; a switch uploads that document's packed arrays again
(no repack) and resends the chrome's state. The egui top bar's document
▾ lists them (``_project_tabs`` below answers what views/wgpu_ui.py reads);
no new tab strip yet (it waits for the design review in
build/shell-design/).
"""
from __future__ import annotations

import types

from PySide6.QtCore import QEvent, QObject, Qt, Signal

from app_shell.command_action import ShellAction
from views.view_host import MISSING
from views.document_commands import WindowDocumentCommands


class _Label:
    def __init__(self):
        self._text = ""

    def text(self):
        return self._text

    def setText(self, text):
        self._text = text

    def setToolTip(self, *_):
        pass

    def setMaximumWidth(self, *_):
        pass


class ShellStatusBar:
    """``statusBar()``: the base message (``name — hint``) and a timed
    flash, as views/wgpu_ui.before_frame reads them."""

    def __init__(self, host):
        self._host = host
        self._base = ""
        self._msg = _Label()
        self._timer = host.timers.timer(host.request_frame)

    def showMessage(self, text, msec=0):
        if msec:
            self._msg.setText(text)
            self._timer.start(msec)
        else:
            self._base = text
        self._host.request_frame()

    def currentMessage(self):
        return self._msg.text() if self._timer.remainingTime() > 0 else self._base

    def clearMessage(self):
        self._timer.stop()

    # The chrome hides the Qt status bar; there is none to hide here.
    def _noop(self, *args, **kwargs):
        return None

    hide = show = setMaximumWidth = setMaximumHeight = addPermanentWidget = _noop

    def isVisible(self):
        return False


class _WindowHandle:
    """``windowHandle()``: the chrome's title-strip drag."""

    def __init__(self, host):
        self._host = host

    def startSystemMove(self):
        self._host.window_drag_requested = True
        return True


class _MenuBar:
    def actions(self):
        return []


class _Relay:
    """A signal-shaped callback: ``emit(*args)`` calls ``fn``."""

    def __init__(self, fn):
        self._fn = fn

    def emit(self, *args):
        self._fn(*args)

    def connect(self, *_):
        pass


class _Tabs:
    """The QTabBar surface wgpu_ui._projects reads (text, data per index)."""

    def __init__(self, bar):
        self._bar = bar

    def tabText(self, i):
        win = self._bar._win
        doc = self._bar.documents()[i]
        dirty = win._doc_is_dirty(doc)
        return doc.display_name() + (self._bar.DIRTY_MARK if dirty else "")

    def tabData(self, i):
        docs = self._bar.documents()
        return docs[i] if 0 <= i < len(docs) else None


class ShellProjectTabs:
    """``window._project_tabs`` as the egui top bar's document ▾ reads it
    (views/wgpu_ui._projects and _project_action)."""

    DIRTY_MARK = " •"

    def __init__(self, win):
        self._win = win
        self.tabs = _Tabs(self)
        self.activated = _Relay(lambda doc: win.activate_document(doc))
        self.newRequested = _Relay(lambda: win.run_command("new"))
        self.closeRequested = _Relay(lambda doc: win.run_command("close_tab", doc))
        self.viewsRequested = _Relay(win._pick_view_layout)

    def documents(self):
        return list(self._win._documents)

    def count(self):
        return len(self._win._documents)

    def current(self):
        return self._win._doc

    def index_of(self, doc):
        docs = self.documents()
        return docs.index(doc) if doc in docs else -1

    def neighbour(self, doc, step):
        docs = self.documents()
        if doc not in docs or len(docs) < 2:
            return None
        return docs[(docs.index(doc) + step) % len(docs)]

    def hide(self):
        pass

    def setMaximumHeight(self, *_):
        pass


class _SceneRow:
    """``window._scene_tabs`` (the scene ‹ › chips read which one is marked)."""

    def __init__(self):
        self.tabs = self
        self._marked = None

    def marked_view(self):
        return self._marked

    def mark_view(self, view):
        self._marked = view

    def setMaximumHeight(self, *_):
        pass


class ShellScenes:
    """The Scenes panel's verbs the bottom bar's scene chips call
    (views/tray.py ScenesPanel), over core's own scene commands."""

    def __init__(self, win):
        self._win = win

    def _vp(self):
        return self._win.viewport

    def _touch(self):
        vp = self._vp()
        vp.scene.version += 1
        vp.notify_scene_changed()
        vp.update()
        view = self._win._doc.stand_in
        if view is not None:
            view._ui_tabs = None

    def show_view(self, view, animate: bool = True):
        player = self._win.scene_player
        player.cancel()
        if view is None:
            return
        vp = self._vp()
        seconds = vp.scene.animation.glide_seconds() if animate else 0.0
        if (self._win.view.renderer is not None and seconds > 0
                and player.glide_to(view, seconds, lambda v: self.finish_view(v, False))):
            self._win._scene_tabs.mark_view(view)
            return
        self.finish_view(view)

    def finish_view(self, view, camera=True):
        if view is None:
            return
        from core.saved_views import shown_state
        vp = self._vp()
        scene = vp.scene
        before = shown_state(scene)
        if camera:
            view.apply(scene, vp.camera)
        else:
            view.apply_state(scene)
        self._win._scene_tabs.mark_view(view)
        if shown_state(scene) != before:
            self._touch()
        else:
            vp.update()
            stand_in = self._win._doc.stand_in
            if stand_in is not None:
                stand_in._ui_tabs = None
                stand_in.update()

    activate_view = finish_view

    def add_scene(self):
        from core.i18n import tr
        from core.protocol.commands import edit_saved_scene
        vp = self._vp()
        marked = self._win._scene_tabs.marked_view()
        result = edit_saved_scene(vp.history, "add", name=tr("Scene"), camera=vp.camera,
                                  style=self._win.active_style(), folder=getattr(marked, "folder", None))
        if not result["ok"]:
            return None
        view = vp.scene.saved_views[result["index"]]
        self._win._scene_tabs.mark_view(view)
        self._touch()
        return view

    def duplicate_view(self, view):
        from core.protocol.commands import edit_saved_scene
        vp = self._vp()
        result = edit_saved_scene(vp.history, "duplicate", views=[view])
        if result["ok"]:
            copy = vp.scene.saved_views[result["index"]]
            self._win._scene_tabs.mark_view(copy)
            self._touch()
            return copy

    def update_view(self, view):
        from core.protocol.commands import edit_saved_scene
        vp = self._vp()
        if edit_saved_scene(vp.history, "update", views=[view], camera=vp.camera,
                            style=self._win.active_style())["ok"]:
            self._touch()

    def rename_view(self, view, name):
        from core.protocol.commands import edit_saved_scene
        result = edit_saved_scene(self._vp().history, "rename", views=[view], name=name or "")
        if result["ok"]:
            self._touch()
        elif result["rejected"]:
            self._win.statusBar().showMessage(f"Another scene is already called «{name}»", 3500)
        return result["ok"]

    def delete_views(self, views):
        from core.protocol.commands import edit_saved_scene
        if edit_saved_scene(self._vp().history, "delete", views=views)["ok"]:
            if self._win._scene_tabs.marked_view() in views:
                self._win._scene_tabs.mark_view(None)
            self._touch()

    def move_view(self, view, index):
        from core.protocol.commands import edit_saved_scene
        if edit_saved_scene(self._vp().history, "move", views=[view], index=index)["ok"]:
            self._touch()


class _Tray:
    def __init__(self, win):
        from ingetrazo_core import PaintRecents
        from tools.paint import PaintTool
        self.scenes = ShellScenes(win)
        self.materials = PaintRecents(PaintTool)


def _rebind(fn, cls, depth=0):
    """``fn`` with every ``__class__`` cell (its own and its closures')
    pointing at ``cls``."""
    if not isinstance(fn, types.FunctionType) or depth > 3:
        return fn
    code = fn.__code__
    if not code.co_freevars:
        return fn
    cells = []
    changed = False
    for name, cell in zip(code.co_freevars, fn.__closure__):
        if name == "__class__":
            cells.append(types.CellType(cls))
            changed = True
            continue
        try:
            value = cell.cell_contents
        except ValueError:
            cells.append(cell)
            continue
        new = _rebind(value, cls, depth + 1)
        if new is not value:
            cells.append(types.CellType(new))
            changed = True
        else:
            cells.append(cell)
    if not changed:
        return fn
    out = types.FunctionType(code, fn.__globals__, fn.__name__, fn.__defaults__, tuple(cells))
    out.__kwdefaults__ = fn.__kwdefaults__
    out.__dict__.update(fn.__dict__)
    out.__qualname__ = fn.__qualname__
    out.__doc__ = fn.__doc__
    return out


#: MainWindow members the shell defines itself and never borrows (they
#: build Qt widgets, or the shell has its own version).
_OWN = {"viewport", "documents", "active_document", "activate_document",
        "close_document", "close_documents", "_new_document", "_retire_document",
        "_install_wgpu_display", "_choose_document_renderer", "open_path",
        "_open_path_here", "_import_progress", "_parse_skp_threaded",
        "_parse_stl_threaded", "show_viewport_context_menu", "closeEvent",
        "_update_title", "_update_tab", "_refresh_scene_tabs", "setWindowTitle",
        "_on_save", "_on_save_as", "_do_save", "_on_export_igz", "_document_picture",
        "_show_empty_area", "_leave_empty_area", "_dismiss_home", "_on_new_window",
        "_sync_foreign_clipboard", "_on_open", "_open_recent", "_fill_recent_menu",
        # Optional per-instance attachments and QWidget probes. A lookup
        # before installation must not load the Classic window class.
        "_host_window", "_doc_area", "_lib_installed", "_mac_chrome",
        "_panel_view", "_vcb_name", "_vcb_value", "isWindow", "scenes_dock"}

#: What a borrowed method may reach for that the shell has nothing for:
#: answered with a no-op, as a window with that panel closed would.
_QUIET = ("_sync_style_menu", "_sync_section_menu", "_refresh_layer_views",
          "_refresh_sheet_tabs",
          "_light_views_button", "_sync_projection_buttons", "_show_renderer_note",
          "_sync_component_edit_menu", "_refresh_vcb", "_forget_panel_caches", "_show_panels_again",
          "on_active_view_changed", "keep_other_views_style", "settle_heap_later")


def _lazy(name):
    """MainWindow's ``name`` copied onto ShellWindow, or None."""
    if name in _OWN:
        return None
    try:
        from views.main_window import MainWindow
    except Exception:  # noqa: BLE001 — no Qt app module: nothing to borrow
        return None
    for klass in MainWindow.__mro__:
        mod = getattr(klass, "__module__", "") or ""
        if mod.startswith(("PySide6", "Shiboken", "shiboken6")) or klass is object:
            continue
        if name not in vars(klass):
            continue
        value = vars(klass)[name]
        if isinstance(value, Signal):
            return None
        if isinstance(value, types.FunctionType):
            value = _rebind(value, ShellWindow)
        elif isinstance(value, property):
            value = property(_rebind(value.fget, ShellWindow), _rebind(value.fset, ShellWindow),
                             _rebind(value.fdel, ShellWindow), value.__doc__)
        elif isinstance(value, (staticmethod, classmethod)):
            value = type(value)(_rebind(value.__func__, ShellWindow))
        setattr(ShellWindow, name, value)
        BORROWED.add(name)
        return value
    return None


#: MainWindow members borrowed so far (diagnostics, the smoke test).
BORROWED: set = set()


def _doc_attr(name):
    def get(self):
        doc = self.__dict__.get("_doc")
        return getattr(doc, name) if doc is not None else None

    def put(self, value):
        setattr(self._doc, name, value)
    return property(get, put)


class ShellWindow(WindowDocumentCommands, QObject):

    def __init__(self, host):
        super().__init__()
        self._host = host
        from core.extensions import discover_plugins
        from core.plugin_panels import install
        from core.extension_manager import extension_startup_state
        self._extension_startup_state = extension_startup_state()
        plugins, errors = discover_plugins()
        from ingetrazo_render import ExtensionMenus
        self._extension_menus = ExtensionMenus()
        self._pending_extensions = plugins
        install(self, plugins)
        self._running_extensions = {p.stem for p in plugins if p.panel}
        self._extension_load_errors = {e.stem: {"error": e.error, "path": str(e.path)} for e in errors}
        from ingetrazo_core import WindowCommands
        self._command_store = WindowCommands()
        self._command_actions = {}
        from ingetrazo_render import build_tools
        self._tools = build_tools()
        self._act_eyedropper = ShellAction(self, "Sample material", checkable=True,
                                           toggled=self._on_toggle_eyedropper)
        self._tool_actions = {}
        self._style_before_xray = (None, None)
        self._rest_shown_mode = "fade"
        self._status = ShellStatusBar(host)
        self._tool_filter = None
        self._doc = None
        self._documents = []
        self._project_tabs = ShellProjectTabs(self)
        self._scene_tabs = _SceneRow()
        self.tray = _Tray(self)
        from views.scene_playback import WindowPlayback
        self.scene_player = WindowPlayback(self, host.timers)
        # MainWindow's own state the borrowed methods read.
        self._workspace = None
        self._empty_on = False
        self._session_path = None
        self._opening_session = False
        self._opening_document = False
        self._open_in_progress = False
        self._open_aborted = False
        self._open_cancelled = False
        self._closed = False
        self.file_openers = {}
        self._status_pending = {}
        self._coord_label = _Label()
        self._tool_label = _Label()
        self._vcb_live = ""
        self._active_file = None
        self._session_unsaved_doc = None
        from core.i18n import tr
        for key, tool in self._tools.items():
            shortcut = getattr(tool, "shortcut", "") or ""
            if key == "select":
                shortcut = "Space"          # bound by MainWindow, not the tool
            action = ShellAction(self, tr(getattr(tool, "name", key)), checkable=True)
            alternate = getattr(tool, "shortcut_alt", None)
            action.setShortcuts([s for s in (shortcut, alternate) if s])
            self._tool_actions[key] = action
        self._tool_defaults = [(k, a.text(), a.state.portable_keys())
                              for k, a in self._tool_actions.items()]
        self._registry_actions = {}
        self.refresh_commands()
        self._act_bridge = self._registry_actions["menu:bridge"]
        self._act_make_faces = self._registry_actions["menu:make_faces"]
        self._act_3dtext = self._registry_actions["menu:text3d"]

    def _on_player_active(self, on):
        self._host.request_frame()

    def _sync_animation_menu(self):
        self._host.request_frame()

    def _setup_autosave(self):
        self._configure_recovery(self._autosave_timer)

    @property
    def _last_command(self):
        return self._command_store.last_command

    def _remember_command(self, label, run):
        self._command_store.remember(label, run)

    def childEvent(self, event):
        # A plugin may attach an action or an entire submenu to this QObject.
        if event.type() in (QEvent.ChildAdded, QEvent.ChildRemoved):
            self._action_tree_changed = True
        super().childEvent(event)

    def refresh_commands(self):
        from app_shell.command_bindings import refresh
        refresh(self)

    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)
        if name in _QUIET:
            return lambda *a, **k: None
        value = _lazy(name)
        if value is not None:
            return getattr(self, name)
        MISSING["window." + name] += 1
        raise AttributeError(name)

    # ---- the documents' state, as MainWindow keeps it --------------------
    _current_path = _doc_attr("path")
    _import_name = _doc_attr("import_name")
    _saved_content = _doc_attr("saved_content")
    _autosaved_version = _doc_attr("autosaved_version")
    _composer = _doc_attr("composer")
    _last_sheet = _doc_attr("last_sheet")

    @property
    def viewport(self):
        doc = self.__dict__.get("_doc")
        if doc is None:
            return None
        return doc.front_view if doc.front_view is not None else doc.viewport

    @property
    def view(self):
        doc = self.__dict__.get("_doc")
        return doc.stand_in if doc is not None else None

    def documents(self):
        return list(self._documents)

    def active_document(self):
        return self._doc

    def all_viewports(self):
        return [d.viewport for d in self._documents if d.viewport is not None]

    def new_document(self, activate: bool = True):
        """A new, empty project with its own controller and view, right of
        the one in front."""
        from app_shell.model_views import ShellDocument
        from views.document_controller import DocumentController
        from app_shell.view import ShellView
        host = self._host
        doc = ShellDocument()
        doc.untitled_no = self._free_untitled_no()
        vp = DocumentController(host)
        try:
            from core import general_settings
            vp.history.max_steps = int(general_settings.value("undo_steps", 200))
        except Exception:  # noqa: BLE001
            pass
        view = ShellView(host)
        view.source_viewport = vp
        view.camera = vp.camera
        vp.keys_stand_in = view
        doc.viewport = vp
        doc.stand_in = view
        doc._renderer_wgpu = True
        doc.saved_content = vp.scene.content_version
        self._watch_document(doc)
        at = self._documents.index(self._doc) + 1 if self._doc in self._documents else len(self._documents)
        self._documents.insert(at, doc)
        vp.set_active_tool(None)
        if self._doc is None:
            self._doc = doc
        if activate:
            self.activate_document(doc)
        return doc

    # MainWindow's name for it (File ▸ New, open_path).
    _new_document = new_document

    def activate_document(self, doc, closing: bool = False) -> None:
        """Bring ``doc`` to the front: the tool is dropped and a group left
        in the one behind; the renderer goes to its view, which uploads its
        packed scene again and resends the chrome."""
        if doc is None or doc not in self._documents:
            return
        old = self._doc
        if doc is old:
            return
        self.scene_player.cancel()
        if old is not None:
            old.model_views.suspend(old)
        if old is not None and old.viewport is not None:
            from views.inspector_bridge import leave_camera
            leave_camera(old)
            try:
                old.viewport.set_active_tool(None)
                old.viewport.end_group_edit()
            except Exception:  # noqa: BLE001
                pass
        renderer = getattr(getattr(old, "stand_in", None), "renderer", None)
        if old is not None and not closing and renderer is not None:
            # Its tab's picture on hover, as it is left: the one renderer
            # goes to the project in front (views/wgpu_tabs.py).
            from views import wgpu_tabs
            try:
                wgpu_tabs.remember_picture(old)
            except Exception:  # noqa: BLE001 — a missing picture only
                pass
        if old is not None and old.stand_in is not None:
            old.stand_in.renderer = None
        self._doc = doc
        view = doc.stand_in
        if renderer is not None:
            self._host.hand_renderer(view, renderer)
        from core import axes as _axes
        from core import units as _units
        _units.bind_scene(doc.viewport.scene)
        _axes.sync(getattr(doc.viewport.scene, "drawing_frame", None))
        self._scene_tabs.mark_view(None)
        self._activate_tool("select")
        doc.viewport.prune_stale_selection()
        doc.viewport.update()
        doc.viewport._document_watch.observe(force=True)
        from app_shell import project
        project.persist(self)
        self._host.request_frame()

    def _watch_document(self, doc):
        from ingetrazo_core import DocumentWatch
        vp = doc.viewport
        vp._document_watch = DocumentWatch(doc)
        vp._unsubscribe_changes = vp.history.feed.subscribe(vp._document_watch)

    def _release_document_watch(self, doc):
        vp = doc.viewport
        unsubscribe = vp.__dict__.pop("_unsubscribe_changes", None)
        if unsubscribe is not None:
            unsubscribe()
        vp.__dict__.pop("_document_watch", None)

    def _document_changed(self, doc, version):
        if doc is self._doc:
            self._update_title()
            for fn in tuple(self.__dict__.get("_doc_change_hooks", ())):
                self._host._guard("extension document changed", fn)
        else:
            self._update_tab(doc)
        if doc.composer is not None:
            doc.composer._on_model_version(version)
        self._host.request_frame()

    def _retire_document(self, doc) -> None:
        from core import autosave
        if doc not in self._documents:
            return
        doc.model_views.suspend(doc)
        self._release_document_watch(doc)
        self._documents.remove(doc)
        from app_shell import project
        project.closed(self, doc)
        project.persist(self)
        for fn in tuple(self.__dict__.get("_doc_close_hooks", ())):
            self._host._guard("extension document closed", fn, doc.viewport.scene)
        try:
            autosave.clear(doc.path, untitled=doc.untitled_no)
        except Exception:  # noqa: BLE001
            pass
        view = doc.stand_in
        if view is not None:
            view.renderer = None
            try:
                view.closeEvent(None)
            except Exception:  # noqa: BLE001
                pass
        doc.viewport = None
        doc.stand_in = None

    def bind(self, viewport, view):
        """The first document (Host.__init__ made its controller and view)."""
        from app_shell.model_views import ShellDocument
        doc = ShellDocument()
        doc.viewport = viewport
        doc.stand_in = view
        doc._renderer_wgpu = True
        doc.saved_content = viewport.scene.content_version
        self._watch_document(doc)
        self._documents = [doc]
        self._doc = doc

    # ---- what the chrome and tools call --------------------------------
    def statusBar(self):
        return self._status

    def menuBar(self):
        return _MenuBar()

    def window(self):
        return self

    def windowHandle(self):
        return _WindowHandle(self._host)

    def windowState(self):
        return Qt.WindowNoState

    def isVisible(self):
        return bool(self._host.ns_view)

    def isMinimized(self):
        return False

    def isActiveWindow(self):
        return True

    def dockWidgetArea(self, _dock):
        return Qt.NoDockWidgetArea

    def setWindowTitle(self, title):
        self._host.set_title(title)

    def _active_view(self):
        return self.viewport

    def _pick_view_layout(self):
        from app_shell.model_views import choose
        choose(self)

    def set_view_layout(self, key):
        self._doc.model_views.choose(self._doc, key)

    def _views_dict(self, doc):
        return doc.model_views.archive()

    def _on_zoom_extents(self):
        bounds = self._extents_bounds()
        if bounds is not None:
            self._doc.model_views.fit_bounds(self, bounds)

    def _on_zoom_selection(self):
        from core.i18n import tr
        bounds = self.viewport.scene.selection_bounds()
        if bounds[0] is None:
            self.statusBar().showMessage(tr("Nothing selected"), 2500)
        else:
            self._doc.model_views.fit_bounds(self, bounds)

    def _maximize_model_view(self):
        self._doc.model_views.maximize(self._doc)

    def active_style(self):
        return self.viewport._view_style()

    def own_style(self):
        vp = self.viewport
        if vp is self._doc.viewport:
            self.keep_other_views_style()
            return vp.scene.display_style
        if vp.view_style is None and vp.scene.display_style is not None:
            vp.view_style = vp.scene.display_style.copy()
        return vp.view_style

    def keep_other_views_style(self):
        self._doc.model_views.preserve_peer_styles(self._doc)

    def set_active_style(self, style):
        if self.viewport is self._doc.viewport:
            self.keep_other_views_style()
            self.viewport.scene.display_style = style
        else:
            self.viewport.view_style = style
        self._host.request_frame()

    def _activate_nav(self, key):
        self.viewport.set_nav_mode(key)
        self._update_status_hint()
        display = self.viewport.keys_stand_in
        display.tools.sync()
        display.tools._apply_tool_cursor()
        self.view.update()

    def _activate_tool(self, key: str) -> None:
        """MainWindow._activate_tool's Metal path."""
        tool = self._tools.get(key)
        if tool is None:
            self._status.showMessage(f"{key} is not in the native app yet — File ▸ Open in Classic App", 4000)
            return
        self.viewport.set_active_tool(tool)
        display = self.viewport.keys_stand_in
        display.tools.pick_up()
        display.set_scene(self.viewport.scene)
        display.update()
        for k, action in self._tool_actions.items():
            action.setChecked(k == key)
        name = getattr(tool, "name", key)
        hint = getattr(tool, "description", "") or ""
        self._status.showMessage(f"{name} — {hint}" if hint else name)
        # What Repeat (⇧R, the right-click's first row) runs again, as
        # MainWindow._activate_tool records it.
        if key not in ("select", "texture_position", "change_axes", "scale_manager"):
            self._remember_command(name, lambda k=key: self._activate_tool(k))

    def _show_model(self):
        from views import wgpu_sheets
        wgpu_sheets.to_model(self)

    def _to_sheets(self):
        # Sheets is a workspace of the window here too (views/wgpu_sheets.py).
        from views import wgpu_sheets
        wgpu_sheets.to_sheets(self)

    def show_viewport_context_menu(self, global_pos=None, locked_image=None, locked_group=None):
        """The right-click menu (views/context_menu.py). With the chrome on,
        WgpuView.contextMenuEvent hands it to the chrome before this is
        reached; without a surface (headless) the menu is only built, for
        the scripted runs and tests that read it."""
        from views import context_menu
        view = self.view
        menu = context_menu.build(self, view, locked_image=locked_image, locked_group=locked_group)
        px = getattr(self.viewport, "_context_pixel", None) or (0.0, 0.0)
        menu.at = (float(px[0]), float(px[1]))
        if view is not None:
            view._ctx_menu = menu
            context_menu.send(view, menu)
        return menu

    # ---- a Built project (app_shell/project.py) ------------------------
    def open_project(self, path):
        from app_shell import project
        return project.open_project(self, path)

    def set_active_file(self, path):
        from app_shell import project
        return project.set_active_file(self, path)

    def query_worksession(self):
        from app_shell import project
        return project.query(self)

    def persist_worksession(self):
        from app_shell import project
        project.persist(self)

    def _refresh_worksession(self):
        from app_shell import project
        project.refresh(self)

    def close_document(self, doc=None, ask: bool = True) -> bool:
        from app_shell import files
        return files.close_document(self, doc)

    def close_documents(self, docs) -> bool:
        """Close these projects one by one, each asking about its own
        changes; a Cancel stops there."""
        for doc in list(docs):
            if doc in self._documents and not self.close_document(doc):
                return False
        return True

    # ---- the title and the dirty mark ----------------------------------
    def _doc_is_dirty(self, doc) -> bool:
        vp = getattr(doc, "viewport", None)
        if vp is None:
            return False
        if doc is self._doc:
            return self._is_dirty()
        return vp.scene.content_version != doc.saved_content

    def _update_title(self) -> None:
        doc = self._doc
        if doc is None or doc.viewport is None:
            return
        marker = " *" if self._is_dirty() else ""
        self.setWindowTitle(f"{doc.display_name()}{marker} — Built")
        view = doc.stand_in
        if view is not None:
            view._ui_docs = None            # the document ▾ shows the • too

    def _update_tab(self, doc=None) -> None:
        view = getattr(self._doc, "stand_in", None)
        if view is not None:
            view._ui_docs = None

    def _refresh_scene_tabs(self) -> None:
        view = getattr(self._doc, "stand_in", None)
        if view is not None:
            view._ui_tabs = None

    def _show_empty_area(self) -> None:
        from views import wgpu_start
        wgpu_start.show(self)

    def _leave_empty_area(self) -> None:
        pass

    def _dismiss_home(self) -> None:
        from views import wgpu_start
        wgpu_start.hide(self)

    # Commands from the menu bar, the chrome and the start page land here
    # (app_shell.files).
    def run_command(self, name, *args):
        from app_shell import files
        return files.command(self, name, *args)

    # MainWindow's names for what app_shell.files does in the shell: the
    # borrowed code (the unsaved-changes question's Save, the start page,
    # the import pipelines, Paste) calls these.
    def open_path(self, path):
        from app_shell import files
        return files.open_path(self, path)

    def _on_open(self):
        from app_shell import files
        return files.open_dialog(self)

    def _on_save(self):
        from app_shell import files
        files.save(self)

    def _on_save_as(self):
        from app_shell import files
        files.save_as(self)

    def _on_export_igz(self):
        from app_shell import files
        files.export_igz(self)

    def _on_new_window(self):
        from app_shell import files
        files.new_window(self)

    def _on_import_orthophoto(self):
        from views.survey_import import import_orthophoto
        import_orthophoto(self)

    def _on_import_photomesh(self):
        from views.survey_import import import_photomesh
        import_photomesh(self)

    def _load_photomesh_threaded(self, path, datum, cb):
        from app_shell.files import parse_threaded
        from ingetrazo_render import read_survey_mesh
        result, error = parse_threaded(read_survey_mesh, path, cb, datum=datum,
                                       limit=self.viewport.max_texture_size())
        return (*result, None) if error is None else (None, None, error)

    def _on_import_georef(self):
        from views.survey_import import import_alignment
        import_alignment(self)

    def _on_import_survey_points(self):
        from views.survey_import import import_points
        import_points(self)

    def _set_survey_capture(self, datum, lo, hi):
        from georef.tiles import DEFAULT_SOURCE_ID, PRESETS, TileLayer
        scene = self.viewport.scene
        layer = scene.tile_layer or TileLayer(PRESETS[DEFAULT_SOURCE_ID])
        layer.capture_bounds(datum, lo.x(), lo.y(), hi.x(), hi.y())
        scene.tile_layer = layer
        self.viewport.reset_tiles()

    @staticmethod
    def _pump_file_command():
        from app_shell.modal import tick
        tick()

    def _import_progress(self, title, *, monotonic=False):
        from app_shell import files
        return files.import_progress(self, title, monotonic=monotonic)

    def _parse_stl_threaded(self, path, scale, simplify_mode, cb, report=None):
        from app_shell import files
        from formats.stl import parse_stl
        return files.parse_threaded(parse_stl, path, cb, scale=scale,
                                    simplify_mode=simplify_mode, report=report)

    def _dxf_options(self, path, header_scale=None):
        from app_shell import files
        return files.dxf_options(self, path, header_scale)

    @staticmethod
    def _clipboard_available():
        from ingetrazo_render import ModelClipboard
        return ModelClipboard.available()

    def _sync_foreign_clipboard(self):
        from app_shell import files
        files.sync_foreign_clipboard(self)

    def _open_recent(self, path):
        from app_shell import files
        return files.open_path(self, path)

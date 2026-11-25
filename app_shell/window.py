"""The document window presented to native commands, chrome and Python extensions.

Each open document owns its history, controller, panes and packed scene.
The winit window gives its render surface to the front document.
"""
from __future__ import annotations

import ingetrazo_render as native


from core.input_types import QEvent, Qt

from ingetrazo_render import shell_action as ShellAction
from views.view_host import MISSING
from ingetrazo_render.document_commands import WindowDocumentCommands


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
        self.newRequested = _Relay(win._on_new_tab)
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


class ShellScenes:
    """Bind the scene chooser to the current camera and document commands."""

    def __init__(self, win):
        self._win = win

    def _vp(self):
        return self._win._active_view()

    def _touch(self):
        vp = self._vp()
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
        from ingetrazo_core import presentation
        from core.saved_views import shown_state
        vp = self._vp()
        scene = vp.scene
        before = shown_state(scene)
        if camera:
            view.apply(scene, vp.camera, self._win.set_active_style)
        else:
            view.apply_state(scene, self._win.set_active_style)
        presentation.recall_scene_display(vp, view)
        self._win._scene_tabs.mark_view(view)
        if shown_state(scene) != before:
            scene.version += 1
            self._touch()
        else:
            vp.update()
            stand_in = self._win._doc.stand_in
            if stand_in is not None:
                stand_in._ui_tabs = None
                stand_in.update()

    activate_view = finish_view

    def add_scene(self):
        marked = self._win._scene_tabs.marked_view()
        return self.add_scene_in_folder(getattr(marked, "folder", None))

    def add_scene_in_folder(self, folder):
        from core.i18n import tr
        from core.protocol.commands import edit_saved_scene
        vp = self._vp()
        result = edit_saved_scene(vp.history, "add", name=tr("Scene"), camera=vp.camera,
                                  style=self._win.active_style(), folder=folder or None)
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


_QUIET = ("_sync_style_menu", "_sync_section_menu",
          "_refresh_sheet_tabs",
          "_sync_projection_buttons", "_show_renderer_note",
          "_sync_component_edit_menu", "_refresh_vcb", "_forget_panel_caches", "_show_panels_again",
          "settle_heap_later")


def _doc_attr(name):
    def get(self):
        doc = self.__dict__.get("_doc")
        return getattr(doc, name) if doc is not None else None

    def put(self, value):
        setattr(self._doc, name, value)
    return property(get, put)


def discover_window_plugins():
    from plugins._native import discover_plugins
    return discover_plugins(native_panels={
        "render_blender": "plugins._blender_panel", "python_console": "plugins.python_console",
        "ai_assistant": None, "ai_bridge": None, "model_info": None, "solid_inspector": None,
    })


def new_shell_window(host):
    return ShellWindow(host, discover_window_plugins())


from core.host_callbacks import Signal


class ShellWindow(WindowDocumentCommands):
    _extension_closed = Signal()

    def __init__(self, host, plugins=None):
        super().__init__()
        self._host = host
        from core.plugin_panels import install
        from core.extension_manager import extension_startup_state
        self._extension_startup_state = extension_startup_state()
        plugins, errors = discover_window_plugins() if plugins is None else plugins
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
        self._reclaim_timer = host.timers.timer(self._reclaim_closed_documents)
        self._project_tabs = ShellProjectTabs(self)
        self.tray = _Tray(self)
        from views.scene_playback import WindowPlayback
        self.scene_player = WindowPlayback(self, host.timers)
        self.file_openers = {}
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
        self._tool_label = _Label()
        from core.i18n import tr
        for key, tool in self._tools.items():
            shortcut = getattr(tool, "shortcut", "") or ""
            if key == "select":
                shortcut = "Space"          # bound by MainWindow, not the tool
            action = ShellAction(self, tr(getattr(tool, "name", key)), checkable=True)
            action.setProperty("statusTip", tr(getattr(tool, "description", "") or ""))
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
        from ingetrazo_render import bind_window_commands
        bind_window_commands(self)

    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)
        if name in _QUIET:
            return lambda *a, **k: None
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
        if self._workspace is not None or doc is old:
            return
        self.scene_player.cancel()
        if old is not None:
            old.model_views.suspend(old)
        if old is not None and old.viewport is not None:
            from ingetrazo_render import leave_camera
            leave_camera(old)
            try:
                old.viewport.set_active_tool(None)
                old.viewport.end_group_edit()
            except Exception:  # noqa: BLE001
                pass
        renderer = getattr(getattr(old, "stand_in", None), "renderer", None)
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
        native.attach_viewport_extensions(self, vp)
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
        self._host.timers.cancel_context(doc.viewport)
        doc.front_view = None
        doc.viewport = None
        doc.stand_in = None

        self._reclaim_timer.start(0)

    def _reclaim_closed_documents(self):
        from ingetrazo_render import reclaim_closed_documents
        reclaim_closed_documents()

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
        self._apply_new_document_units()

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
        return self._doc.model_views.command_view(self._doc) if self._doc is not None else None

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
        return self._active_view()._view_style()

    def own_style(self):
        vp = self._active_view()
        if vp is self._doc.viewport:
            self.keep_other_views_style()
            return vp.scene.display_style
        if vp.view_style is None and vp.scene.display_style is not None:
            vp.view_style = vp.scene.display_style.copy()
        return vp.view_style

    def keep_other_views_style(self):
        self._doc.model_views.preserve_peer_styles(self._doc)

    def set_active_style(self, style):
        vp = self._active_view()
        if vp is self._doc.viewport:
            self.keep_other_views_style()
            vp.scene.display_style = style
        else:
            vp.view_style = style
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
            self._status.showMessage(f"{key} is unavailable in this window", 4000)
            return
        if self._workspace is not None and not self._tool_actions[key].isEnabled():
            return
        self.viewport.set_active_tool(tool)
        display = self.viewport.keys_stand_in
        display.tools.pick_up()
        display.set_scene(self.viewport.scene)
        display.update()
        for k, action in self._tool_actions.items():
            action.setChecked(k == key)
        name = getattr(tool, "name", key)
        self._update_status_hint()
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
        """The right-click menu (viewport_menu.rs)."""
        from plugins._context_menu import context_menu_show
        return context_menu_show(self, locked_image, locked_group)

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
        return files.close_document(self, doc, ask)

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
        if self._workspace is not None:
            self.setWindowTitle(f"Built — {self._workspace.title()}{marker}")
        else:
            self.setWindowTitle(f"{doc.display_name()}{marker} — Built")
        view = doc.stand_in
        if view is not None:
            view._ui_docs = None            # the document ▾ shows the • too

    def _update_tab(self, doc=None) -> None:
        view = getattr(self._doc, "stand_in", None)
        if view is not None:
            view._ui_docs = None

    def _show_empty_area(self) -> None:
        from ingetrazo_render import wgpu_start
        wgpu_start.show(self)

    def _leave_empty_area(self) -> None:
        pass

    def _dismiss_home(self) -> None:
        from ingetrazo_render import wgpu_start
        wgpu_start.hide(self)

    # Commands from the menu bar, the chrome and the start page land here
    # (app_shell.files).
    def run_command(self, name, *args):
        from app_shell import files
        return files.command(self, name, *args)

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


    def _wake_link_watch(self):
        self._host.request_frame()

    def workspace(self):
        return self._workspace

    def enter_workspace(self, workspace):
        return native.enter_workspace(self, workspace)

    def leave_workspace(self):
        return native.leave_workspace(self)

    def _on_standard_view(self, key):
        native.standard_view(self._active_view(), key)

    def _set_projection(self, mode):
        self._active_view().set_projection(mode)

    def _apply_display_style(self, preset):
        native.display_style(self, preset)

    def _toggle_xray(self):
        native.toggle_xray(self)

    def _set_style_field(self, name, value):
        native.style_field(self, name, value)

    def repaint_styled(self):
        self._host.request_frame()

    def _set_scene_visibility(self, attr, on):
        setattr(self.viewport.scene, attr, bool(on))
        self.viewport.update()

    def _set_hidden_view(self, attr, on):
        native.hidden_view(self.viewport, attr, on)

    def _on_toggle_shadows(self, on):
        native.set_shadows(self.viewport, on)

    def _on_smart_zoom(self):
        native.smart_zoom(self)

    def _update_status_hint(self):
        native.tool_hint(self)

    def _rebuild_surfaces(self):
        from ingetrazo_render import rebuild_path_surfaces
        rebuild_path_surfaces(self.viewport)

    def _on_preferences(self):
        return self.run_command("prefs")

    def _on_manage_extensions(self):
        return self.run_command("extensions")

    def release_eyedropper(self):
        self._on_toggle_eyedropper(False)

    def _set_survey_capture(self, datum, lo, hi):
        from georef.tiles import TileLayer
        from ingetrazo_render import selected_tile_source
        scene = self.viewport.scene
        layer = scene.tile_layer or TileLayer(selected_tile_source())
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
        from ingetrazo_core import parse_stl_threaded
        return parse_stl_threaded(path, scale, simplify_mode, cb, report)

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

"""The calls the Rust shell makes into Python, one object per window.

Everything runs on the main thread, under the GIL the Rust side takes for
each call; between calls Rust holds no GIL, so the snap-index worker and
other Python threads progress while winit waits for events.

Coordinates are logical pixels (Qt's), y down, from the window's top-left.
Modifier masks and key codes are already Qt's (rust/built_app/src/keys.rs).

The window may hold several documents (app_shell.window): ``viewport`` and
``view`` are always the ones in front; the renderer moves with them
(:meth:`Host.hand_renderer`).
"""
from __future__ import annotations

import os
import sys
import time
import traceback
from pathlib import Path

from app_shell import qt_host

#: Qt cursor shape → winit CursorIcon name (rust/built_app/src/main.rs).
_CURSORS = {
    0: "default", 1: "default", 2: "crosshair", 3: "wait", 4: "text",
    5: "ns-resize", 6: "ew-resize", 7: "nesw-resize", 8: "nwse-resize",
    9: "move", 10: "default", 11: "row-resize", 12: "col-resize",
    13: "pointer", 14: "not-allowed", 15: "help", 16: "progress",
    17: "grab", 18: "grabbing", 20: "copy", 21: "alias",
}
#: Qt.BitmapCursor: a pixmap cursor (the tools' icons).
_BITMAP = 24

#: The repository and venv the interpreter was started on (bootstrap).
ROOT = None
VENV = None


#: ``verb:<name>`` menu commands: MainWindow verbs whose questions are
#: chrome forms (they answer on a later turn, nothing blocks).
_VERBS = {"divide": "_on_divide", "new_tag": "_assign_new_layer",
          "image_size": "_on_image_size", "image_opacity": "_on_image_opacity"}


def _log(*parts):
    print("[built]", *parts, file=sys.stderr, flush=True)


class Host:
    def __init__(self, ns_view: int, width: float, height: float, scale: float,
                 settings: str | None = None, temporary: bool = False,
                 display: int = 0, platform: str = ""):
        t0 = time.perf_counter()
        self.settings_dir = qt_host.start(settings, temporary=temporary)
        self.ns_view = int(ns_view)
        self.surface_handles = dict(display=display, platform=platform) if platform else {}
        self.width = max(1, int(round(width)))
        self.height = max(1, int(round(height)))
        self.scale = float(scale)
        self._frame_wanted = True
        from ingetrazo_render import WindowTimers
        self.timers = WindowTimers()
        self._wake_timer = self.timers.timer(self.request_frame)
        self._cursor = None
        self._title = None
        self._custom_cursors = {}
        self._after_picture = []
        self._picture_seen = False
        self._recent_dirty = True
        self._preedit = ""
        self._ime_area = None
        self.quit_now = False
        self.errors = []
        self.not_here_log = []
        #: Set by the window handle's startSystemMove (the chrome's empty
        #: top strip); Rust starts the native window drag and clears it.
        self.window_drag_requested = False
        #: Windows this one asks Rust for (app_shell.windows): tokens that
        #: come back in :meth:`adopt` on the new window's host.
        self.window_requests = []
        #: This window should come to the front (a file already open here
        #: was opened from another window, the ›› n menu).
        self.focus_requested = False
        from views.document_controller import DocumentController
        from app_shell.view import ShellView
        from app_shell.window import ShellWindow
        from app_shell import files, modal
        from ingetrazo_core import WindowInput
        self.input_state = WindowInput()
        self.window = ShellWindow(self)
        viewport = DocumentController(self)
        try:
            from core import general_settings
            viewport.history.max_steps = int(general_settings.value("undo_steps", 200))
        except Exception:
            pass
        view = ShellView(self)
        view.source_viewport = viewport
        view.camera = viewport.camera
        viewport.keys_stand_in = view
        self.window.bind(viewport, view)
        self._device = None
        viewport.set_active_tool(None)
        modal.install(self)
        files.install(self.window)
        from app_shell import windows
        windows.register(self)
        guard = qt_host.widget_guard()
        if guard is not None:
            guard.on_blocked = lambda title: self.not_here(f"“{title}”")
        try:
            self.window._autosave_timer = self.timers.timer(self.window._on_autosave_tick, repeat=True)
            self.window._setup_autosave()
        except Exception:
            self._error("autosave setup")
        self.startup_ms = (time.perf_counter() - t0) * 1000
        self.window._update_title()

    # ---- the document in front ---------------------------------------------
    @property
    def viewport(self):
        return self.window.viewport

    @property
    def view(self):
        return self.window.view

    @property
    def path(self):
        return self.window._current_path

    def hand_renderer(self, view, renderer):
        """The window's renderer to ``view`` (a document switch): its packed
        scene goes up again (no repack), and everything the chrome was last
        sent is sent again for it."""
        view.renderer = renderer
        if renderer is not None and hasattr(renderer, 'set_reference_layers'):
            renderer.set_reference_layers([], [])
            renderer.set_center_cue(None, 0., (0., 0.), 1.)
        view._reference_cache = None
        view._had_center_cue = False
        if getattr(view, "_ui_timer", None) is None:
            view._ui_timer = view._view_timer(view.update)
        # What the chrome was last sent for this view is no longer what the
        # renderer shows: the signatures go (the defaults come back), the
        # view's own state (expanded rows, filters, mode) stays.
        resend = ("_ui_ptabs", "_ui_docs", "_ui_tabs", "_ui_opts", "_ui_palette",
                  "_ui_issues", "_ui_cursor_shape", "_ui_was_over")
        for key in [k for k in view.__dict__ if k.startswith("_ui_")]:
            if key.endswith(("_sig", "_sent", "_cv")) or key in resend:
                del view.__dict__[key]
        view._ui_attached = True
        view._ui_tools_sent = False
        view._ui_sheet_sig = False
        view._ui_jobs_sig = False
        view._ui_units_sent = False
        if view.source_viewport is not None:
            view.source_viewport._chrome_owns_labels = True
        view._physical_size = None
        view._selection_signature = None
        view._tint_signature = None
        view._last_camera = None
        view._finish_resize()
        if getattr(view, "packed", None) is not None:
            view._upload(force=True)
        else:
            view.set_scene(self.viewport.scene)
        view._shell_resident_version = self.viewport.scene.version
        self.request_frame()

    # ---- the renderer --------------------------------------------------
    def attach_surface(self):
        """Make the renderer on the window's NSView (first frame)."""
        view = self.view
        view.set_scene(self.viewport.scene)
        if self.ns_view:
            view._upload(force=True)
        self.window._activate_tool("select")
        return view.renderer

    @property
    def renderer(self):
        return self.view.renderer

    # ---- what the view and status bar ask of the host ------------------
    def request_frame(self):
        self._frame_wanted = True

    def wake_after(self, msec):
        left = self._wake_timer.remainingTime()
        if left < 0 or left > msec:
            self._wake_timer.start(msec)

    @staticmethod
    def tool_cursor(key, plus=False):
        from core import general_settings
        from ingetrazo_render import tool_cursor_name
        return tool_cursor_name(key, plus, str(general_settings.value("cursor_style", "tool")))

    def cursor_changed(self, cursor):
        if isinstance(cursor, str):
            self._cursor = cursor
            return
        try:
            shape = cursor.shape() if hasattr(cursor, "shape") else cursor
            value = int(getattr(shape, "value", shape))
            if value == _BITMAP and hasattr(cursor, "pixmap"):
                name = self._bitmap_cursor(cursor)
            else:
                name = _CURSORS.get(value, "crosshair")
        except Exception:
            name = "default"
        self._cursor = name

    def _bitmap_cursor(self, cursor):
        """A tool's pixmap cursor as a native one: its picture at the
        cursor's logical size (winit sizes a custom cursor by its pixels)."""
        from PySide6.QtCore import Qt
        from PySide6.QtGui import QImage
        pm = cursor.pixmap()
        if pm.isNull():
            return "crosshair"
        key = str(pm.cacheKey())
        if key not in self._custom_cursors:
            dpr = pm.devicePixelRatio() or 1.0
            w = max(1, int(round(pm.width() / dpr)))
            h = max(1, int(round(pm.height() / dpr)))
            img = pm.toImage().scaled(w, h, Qt.IgnoreAspectRatio, Qt.SmoothTransformation)
            img = img.convertToFormat(QImage.Format_RGBA8888)
            data = bytes(img.constBits())[: w * h * 4]
            hot = cursor.hotSpot()
            hx = min(w - 1, max(0, int(hot.x())))
            hy = min(h - 1, max(0, int(hot.y())))
            self._custom_cursors[key] = (w, h, hx, hy, data)
        return "custom:" + key

    def custom_cursor(self, key):
        return self._custom_cursors.get(key)

    def set_title(self, title):
        self._title = title

    def _set_title(self):
        self.window._update_title()

    def recent_changed(self):
        self._recent_dirty = True

    def after_picture(self, fn):
        """Run ``fn`` after the next picture, one per loop turn (the panels
        and indexes the Qt app leaves until the model is on screen)."""
        self._after_picture.append(fn)
        self._picture_seen = False

    def not_here(self, feature):
        """Say that ``feature`` needs the classic app (and how to get there)."""
        self.not_here_log.append(feature)
        self.window.statusBar().showMessage(
            f"{feature} is not in the native app yet — File ▸ Open in Classic App", 6000)

    # ---- one turn of the loop ------------------------------------------
    def idle(self):
        """Qt's posted events and timers; then what the window should do:
        ``(frame_wanted, wake_in_ms or None, cursor or None, title or None)``."""
        qt_host.pump()
        self._guard("window timers", self.timers.run_due)
        pending = self.window.__dict__.get("_pending_open")
        if pending and time.monotonic() >= pending.next_poll:
            pending.next_poll = time.monotonic() + 0.032
            pending.poll()
            self.request_frame()
        view = self.view
        scene = self.viewport.scene
        self.window._doc.viewport._document_watch.observe()
        # MainWindow._install_wgpu_display's present(): a scene whose
        # version moved since the resident pack is packed again.
        if view.renderer is not None and (scene is not getattr(view, "scene", None)
                                          or scene.version != getattr(view, "_shell_resident_version", None)):
            self._frame_wanted = True
        delay = max(0., (pending.next_poll - time.monotonic()) * 1000) if pending else None

        def sooner(ms):
            nonlocal delay
            if ms is not None:
                delay = ms if delay is None else min(delay, ms)

        # Remaining plugin/tool Qt callbacks still need their event pump.
        sooner(qt_host.next_timer_ms())
        if getattr(view, "_inference_future", None) is not None or getattr(view, "_inference_queued", False):
            sooner(33)
        # File changes of linked documents arrive as Qt posted events,
        # which winit does not see: look for them once a second.
        watch = self.window.__dict__.get("_links_watch")
        if watch is not None and getattr(watch, "_fs", None) is not None and watch._fs.files():
            sooner(1000)
        if self._after_picture and self._picture_seen:
            fn = self._after_picture.pop(0)
            self._guard("after picture", fn)
            sooner(16)
        cursor, self._cursor = self._cursor, None
        title, self._title = self._title, None
        wanted, self._frame_wanted = self._frame_wanted, False
        sooner(self.timers.next_ms())
        return wanted, delay, cursor, title

    def extras(self):
        """What else the window should do this turn (rust/built_app
        python.rs ``Extras``)."""
        recent = None
        if self._recent_dirty:
            self._recent_dirty = False
            try:
                recent = self.window._recent_paths()
            except Exception:
                recent = []
        new_window = self.window_requests.pop(0) if self.window_requests else None
        focus, self.focus_requested = self.focus_requested, False
        return {"exit": bool(self.quit_now), "ime_area": self._ime_area, "recent": recent,
                "new_window": new_window, "focus": bool(focus)}

    def adopt(self, token):
        """This new window was asked for by ``token`` (app_shell.windows):
        the start page for New Window, or the project moved here."""
        from app_shell import files, windows
        self._front()
        what = windows.take(token)
        if what is None:
            return False
        kind, payload = what
        if kind == "move":
            src, doc = payload
            ok = bool(self._guard("move project", windows.transfer, src, doc, self))
        else:
            ok = True
            self._guard("new window", files.cold_start, self.window)
        self.request_frame()
        return ok

    def _front(self):
        """This window is the one being used: questions it asks show here
        (app_shell.modal), as do the projects of the shared menu bar."""
        from app_shell import modal
        modal._HOST = self

    def frame(self):
        """Draw one frame. ``(cpu_ms, gpu_ms, wall_ms)`` or None."""
        view = self.view
        scene = self.viewport.scene
        try:
            if scene is not getattr(view, "scene", None) or scene.version != getattr(
                    view, "_shell_resident_version", None):
                version = scene.version
                view.set_scene(scene)
                view._shell_resident_version = version
            grid = getattr(self.window._doc, "model_views", None)
            out = grid.frame() if grid is not None else view.draw_frame()
            if out is not None:
                self._picture_seen = True
            return out
        except Exception:
            self._error("frame")
            return None

    def modal_frame(self):
        """A frame while a question or a job holds the loop (app_shell.modal)."""
        self._guard("window timers", self.timers.run_due)
        if self._frame_wanted:
            self._frame_wanted = False
            self.frame()

    def _error(self, where):
        text = traceback.format_exc()
        seen = self.__dict__.setdefault("_seen_errors", set())
        if text in seen:
            return
        seen.add(text)
        if len(self.errors) < 50:
            self.errors.append((where, text))
        _log(f"error in {where}:\n{text}")

    # ---- documents -----------------------------------------------------
    def open_path(self, path):
        """A file from the command line, Finder (Open With, a double-click,
        the Dock), a drop on the window, or another `built` handing over."""
        from app_shell import files
        self._front()
        t0 = time.perf_counter()
        ok = bool(self._guard("open", files.open_path, self.window, Path(path)))
        field = "dispatch" if self.window.__dict__.get("_pending_open") else "total"
        self.open_ms = {field: (time.perf_counter() - t0) * 1000}
        self.request_frame()
        return ok

    #: The name the first slice and its tests use.
    open = open_path

    def cold_start(self):
        from app_shell import files
        self._guard("cold start", files.cold_start, self.window)

    def request_close(self):
        """Close the window / quit: True once nothing unsaved stands in the
        way (the sheet asked, through the modal pump)."""
        from app_shell import files
        from views import chrome_dialogs as cd
        self._front()
        if cd.modal(self.window):
            return False
        ok = self._guard("close", files.request_close, self.window)
        return bool(ok)

    # ---- input ---------------------------------------------------------
    def _guard(self, where, fn, *args):
        try:
            return fn(*args)
        except Exception:
            self._error(where)
            return None

    def _modal(self):
        from views import chrome_dialogs as cd
        return cd.modal(self.window)

    def _pane_input(self, kind, method, event, held):
        grid = getattr(self.window._doc, "model_views", None)
        if grid is None or not grid.visible:
            return self.view
        from views.wgpu_view import _ui_claims
        if not grid.layout.captured and _ui_claims(self.view, method, event):
            return None
        pos = event.position()
        primary = getattr(event, "button", lambda: None)()
        cursor = grid.layout.divider(kind, pos.x(), pos.y(), getattr(primary, "value", 0) == 1, grid.snap_pixels)
        if cursor:
            self.cursor_changed(cursor)
            return None
        hit = grid.pointer(kind, pos.x(), pos.y(), held)
        if hit is None:
            return None
        pane, x, y = hit
        event.set_local_position(x, y)
        return pane

    def pointer(self, kind, x, y, button=0, held=0, modifiers=0, timestamp=0):
        from app_shell.events import ContextMenuEvent, MouseEvent
        self.input_state.pointer(x, y, held, modifiers)
        if kind in ("press", "double"):
            self.window.scene_player.stop()
            self._front()
        view = self.view
        if kind == "leave":
            return self._guard("leave", view.leaveEvent, None)
        ev = MouseEvent(kind, x, y, button, held, modifiers, timestamp=timestamp)
        method = {"press": "mousePressEvent", "release": "mouseReleaseEvent",
                  "move": "mouseMoveEvent", "double": "mouseDoubleClickEvent"}[kind]
        view = self._pane_input(kind, method, ev, held)
        if view is None:
            self.request_frame()
            return
        self._guard(f"pointer {kind}", getattr(view, method), ev)
        if kind == "press" and button == 2 and not self._modal():
            # Qt on the Mac sends the context menu on the right press.
            self._guard("context menu", view.contextMenuEvent, ContextMenuEvent(ev.x(), ev.y(), modifiers))
        self.request_frame()

    def wheel(self, x, y, pixel_dx, pixel_dy, angle_dx, angle_dy, phase="none",
              modifiers=0, held=0, inverted=True, touch_screen=False):
        from app_shell.events import WheelEvent
        self.input_state.pointer(x, y, held, modifiers)
        if self._modal():
            return
        self.window.scene_player.stop()
        ev = WheelEvent(x, y, pixel_dx, pixel_dy, angle_dx, angle_dy, phase, modifiers, held, inverted, touch_screen)
        view = self._pane_input("wheel", "wheelEvent", ev, held)
        if view is not None:
            self._guard("wheel", view.wheelEvent, ev)
        self.request_frame()

    def gesture(self, kind, value, x, y, modifiers=0):
        from app_shell.events import NativeGestureEvent
        self.input_state.pointer(x, y, self.input_state.buttons, modifiers)
        if self._modal():
            return
        self.window.scene_player.stop()
        ev = NativeGestureEvent(kind, value, x, y, modifiers=modifiers)
        view = self._pane_input("gesture", "event", ev, self.input_state.buttons)
        if view is not None:
            self._guard("gesture", view.event, ev)
        self.request_frame()

    def key(self, pressed, key, modifiers=0, text="", repeat=False):
        """A key, as Qt delivers it: ShortcutOverride first (the chrome, the
        value box and the tool may claim it), then the window's shortcuts
        (the tools' letters), else the view's key handler. While a sheet
        or an exclusive job holds the window, only they hear keys."""
        from PySide6.QtCore import Qt
        from app_shell.events import KeyEvent, ShortcutOverride
        from views import chrome_dialogs as cd
        self.input_state.modifiers = modifiers
        view = self.view
        if pressed:
            self._front()
        pending = self.window.__dict__.get("_pending_open")
        if pending and pressed and key == Qt.Key_Escape and not self._modal():
            pending.job.cancel()
            return True
        if self._modal():
            if cd.current_sheet(self.window) is None and cd._form_up(self.window):
                # A form on the chrome (views/chrome_forms.py): its fields
                # type, Enter and Esc answer it; nothing else hears keys.
                from views import wgpu_ui
                ev = KeyEvent(pressed, key, modifiers, text, repeat)
                self._guard("form key", wgpu_ui.key, view, ev, pressed)
                self.request_frame()
                return True
            if pressed and not repeat:
                ev = KeyEvent(True, key, modifiers, text, repeat)
                if cd.current_sheet(self.window) is not None:
                    self._guard("sheet key", cd.sheet_key, self.window, ev)
                else:
                    self._guard("job key", cd.job_key, self.window, ev)
            self.request_frame()
            return True
        grid = getattr(self.window._doc, "model_views", None)
        if pressed and key == Qt.Key_Escape and grid and grid.layout.cancel_divider():
            self.request_frame()
            return True
        if pressed and key == Qt.Key_Escape and self.window.scene_player.is_active():
            self.window.scene_player.stop()
            return True
        if pressed:
            override = ShortcutOverride(key, modifiers, text, repeat)
            if grid and grid.visible:
                from views import wgpu_ui
                if wgpu_ui.claims_key(view, override):
                    override.accept()
                else:
                    self._guard("shortcut override", self.viewport.keys_stand_in.event, override)
            else:
                self._guard("shortcut override", view.event, override)
            if not override.isAccepted() and not repeat and self._shortcut(key, modifiers):
                self.request_frame()
                return True
        ev = KeyEvent(pressed, key, modifiers, text, repeat)
        if grid is not None and grid.visible:
            from views.wgpu_view import _ui_claims
            if _ui_claims(view, "keyPressEvent" if pressed else "keyReleaseEvent", ev):
                self.request_frame()
                return True
            view = self.viewport.keys_stand_in
        self._guard("key", view.keyPressEvent if pressed else view.keyReleaseEvent, ev)
        self.request_frame()
        return True

    def refresh_commands(self):
        self.window.refresh_commands()

    def _shortcut(self, key, modifiers):
        win = self.window
        slot = win._command_store.resolve(int(key), int(modifiers))
        if slot is not None:
            win._command_actions[slot].trigger()
            return True
        if vars(win).get("_action_tree_changed"):
            from app_shell.legacy_actions import sync
            sync(win)
        action = win._command_store.plugin_action(int(key), int(modifiers))
        if action is None:
            return False
        action.trigger()
        return True

    def text(self, text):
        """Committed IME text: the chrome's focused field, else the tools
        hear it as key presses carrying the text."""
        self._preedit_done()
        renderer = self.view.renderer
        if renderer is not None and renderer.ui_claims_key(""):
            if hasattr(renderer, "ui_ime"):
                renderer.ui_ime("commit", text)
            else:
                renderer.ui_text(text)
            self.request_frame()
            return
        for ch in text:
            code = ord(ch.upper()) if ch.isalpha() else ord(ch)
            self.key(True, code, 0, ch)
            self.key(False, code, 0, ch)

    def preedit(self, text, cursor=0):
        """The composition being typed (dead keys, an input method): drawn
        by the chrome's focused field as egui does (underlined, in place),
        else shown in the hint line until it commits."""
        renderer = self.view.renderer
        was = self._preedit
        self._preedit = text or ""
        field = renderer is not None and renderer.ui_claims_key("")
        if field and hasattr(renderer, "ui_ime"):
            if not was and self._preedit:
                renderer.ui_ime("enabled", "")
            renderer.ui_ime("preedit", self._preedit)
            rect = renderer.ui_ime_rect() if hasattr(renderer, "ui_ime_rect") else None
            self._ime_area = tuple(rect) if rect else None
        elif self._preedit:
            self.window.statusBar().showMessage(f"Composing: {self._preedit}", 4000)
        else:
            self.window.statusBar().clearMessage()
        self.request_frame()

    def _preedit_done(self):
        if self._preedit:
            self._preedit = ""
            self.window.statusBar().clearMessage()

    def drag_hover(self, path):
        """A file dragged over the window (or gone again)."""
        if path:
            self.window.statusBar().showMessage(f"Drop to open {Path(path).name}", 3000)
        else:
            self.window.statusBar().clearMessage()
        self.request_frame()

    def resize(self, width, height, scale):
        self.width = max(1, int(round(width)))
        self.height = max(1, int(round(height)))
        self.scale = float(scale)
        self._guard("resize", self.view.resizeEvent, None)
        self.request_frame()

    def focus(self, on):
        if on:
            self._front()
            from views.ndof_input import shared_input
            if self._device is None and (device := shared_input()).backend_name:
                self._device = device
                device.motion.connect(self.window._on_ndof_motion)
                device.button.connect(self.window._on_ndof_button)
        elif self._device is not None:
            self._device.motion.disconnect(self.window._on_ndof_motion)
            self._device.button.disconnect(self.window._on_ndof_button)
            self._device = None
        if not on:
            grid = getattr(self.window._doc, "model_views", None)
            if grid is not None:
                grid.layout.cancel()
            self.input_state.focus_lost()
            self._guard("focus out", self.view.focusOutEvent, None)

    # ---- menus ---------------------------------------------------------
    def menu(self, action):
        """A menu command from the Rust menu bar (rust/built_app/src/menu.rs)."""
        w = self.window
        self._front()
        if self._modal():
            return None                 # a question or a job holds the window
        kind, _, arg = action.partition(":")
        out = None
        if kind == "tool":
            self._guard("tool", w._activate_tool, arg)
        elif kind == "panel":
            from core.plugin_panels import open_panel
            self._guard("open panel", open_panel, w, arg)
        elif action == "undo":
            self._guard("undo", w._on_undo)
        elif action == "redo":
            self._guard("redo", w._on_redo)
        elif action == "zoom_extents" or kind == "view":
            from views.view_commands import run
            self._guard("camera command", run, w, "view_" + arg if kind == "view" else action)
            if kind == "view":
                bounds = w._extents_bounds()
                if bounds:
                    from views.wgpu_ui import free_area
                    grid = getattr(w._doc, "model_views", None)
                    self.viewport.camera.fit_box(*bounds, free=None if grid and grid.visible else free_area(w))
        elif kind == "verb" and arg in _VERBS:
            # The ported prompts (views/chrome_forms.py), by name: the
            # window's own MainWindow methods, borrowed.
            self._guard(f"verb {arg}", getattr(w, _VERBS[arg]))
        else:
            out = self._guard(f"menu {action}", w.run_command, action)
        view = self.view
        if view is not None and kind != "tool":
            self._guard("tool sync", view.tools.sync)
        self.request_frame()
        return out

    def close(self):
        from app_shell import windows
        from views import chrome_forms, chrome_sheet
        if self._device is not None:
            self.focus(False)
        chrome_sheet.close(self.window)
        chrome_forms.close_all(self.window)
        self.window._release_check = None
        windows.unregister(self)
        if vars(self.window).get("_action_observer") is not None:
            from app_shell.legacy_actions import close
            close(self.window)
        self.window.scene_player.cancel()
        self.timers.cancel_all()
        for doc in list(self.window._documents):
            if grid := getattr(doc, "model_views", None):
                grid.suspend()
            self.window._release_document_watch(doc)
            view = doc.stand_in
            if view is not None:
                self._guard("close", view.closeEvent, None)

    # ---- diagnostics -----------------------------------------------------
    def state(self):
        """A few facts for scripted runs and the smoke test."""
        from app_shell import windows
        from app_shell.window import BORROWED
        from views import wgpu_tabs
        scene = self.viewport.scene
        mesh = scene.mesh
        tool = self.viewport.active_tool
        win = self.window
        guard = qt_host.widget_guard()
        return {
            "opening": [(j.label, j.fraction, j.phase) for j in win.__dict__.get("_chrome_jobs", ())],
            "instances": len(self.view.packed.instances) if self.view.packed is not None else 0,
            "path": str(self.path) if self.path else None,
            "version": scene.version,
            "faces": len(getattr(mesh, "faces", ()) or ()),
            "edges": len(getattr(mesh, "edges", ()) or ()),
            "groups": len(getattr(scene, "groups", ()) or ()),
            "input": self.view.renderer.ui_input_state() if hasattr(self.view.renderer, "ui_input_state") else {},
            "filter": self.view.__dict__.get("_ui_filter", ""),
            "qt_modules": sorted(name for name in sys.modules if name.split(".")[0] in ("PySide6", "shiboken6")),
            "model_layout": win._doc.view_layout,
            "model_pane": grid.layout.active if (grid := getattr(win._doc, "model_views", None)) else 1,
            "model_dividers": grid.layout.shares if grid else None,
            "view_targets": getattr(self.renderer, "target_count", 0),
            "dock_lens": self.view.__dict__.get("_ui_lens", 0),
            "dock_mode": self.view.__dict__.get("_ui_mode", ""),
            "tool_busy": self.viewport._tool_busy_for_delete(tool),
            "edit_group": getattr(scene.edit_group, "name", None),
            "selection": len(getattr(scene, "selection", ()) or ()),
            "cluster": [key for key, _ in getattr(self.renderer, "ui_cluster_buttons", lambda: [])()],
            "nav_mode": self.viewport.nav_mode,
            "camera_distance": self.viewport.camera.distance,
            "camera_forward": self.viewport.camera.forward().toTuple(),
            "camera_eye": self.viewport.camera.eye().toTuple(),
            "tool": type(tool).__name__ if tool is not None else None,
            "status": win.statusBar().currentMessage(),
            "documents": [d.display_name() for d in win._documents],
            "windows": len(windows.hosts()),
            "link_tabs": [d.display_name() for d in win._documents
                          if wgpu_tabs.link_host(win, d)[0] is not None],
            "tab_rects": self._tab_rects(),
            "context_menu": self._context_rows(),
            "active": win._documents.index(win._doc) if win._doc in win._documents else -1,
            "dirty": bool(win._is_dirty()),
            "large_sheet": self.renderer.ui_large_sheet_state()[0] if self.renderer is not None else "",
            "form_open": self.renderer.ui_dialog_open() if self.renderer is not None else False,
            "start_page": bool(getattr(win.__dict__.get("_start_page"), "wanted", False)),
            "errors": len(self.errors),
            "widget_gaps": self.missing(),
            "borrowed": len(BORROWED),
            "blocked_widgets": list(guard.blocked) if guard is not None else [],
            "not_here": list(self.not_here_log),
            "custom_cursors": len(self._custom_cursors),
            "preedit": self._preedit,
            "ime_area": self._ime_area,
            "clipboard": bool(getattr(self.viewport, "clipboard", None)),
            "pasteboard": self._pasteboard_types(),
            "project": win.query_worksession() if win.__dict__.get("_project") is not None else None,
            "read_only": [bool(getattr(d, "read_only", False)) for d in win._documents],
        }

    def _tab_rects(self):
        """The project tabs as the chrome last laid them out (scripted runs)."""
        renderer = getattr(self.view, "renderer", None)
        if renderer is None or not hasattr(renderer, "ui_project_tab_rects"):
            return []
        return [list(r) if r is not None else None for r in renderer.ui_project_tab_rects()]

    def _context_rows(self):
        """The right-click menu's rows on screen, or [] (scripted runs)."""
        renderer = getattr(self.view, "renderer", None)
        if renderer is None or not hasattr(renderer, "ui_context_menu_rows"):
            return []
        return list(renderer.ui_context_menu_rows())

    @staticmethod
    def _pasteboard_types():
        from app_shell import native
        return [t for t in native.pasteboard_types()[0] if "ingetrazo" in t]

    def missing(self):
        """Qt widget/window API the borrowed code asked for and the shell
        lacks (probes of the code's own lazily set attributes left out)."""
        from PySide6.QtWidgets import QWidget
        from PySide6.QtWidgets import QMainWindow
        from views.view_host import MISSING
        out = {}
        for name, n in MISSING.most_common():
            bare = name.split(".", 1)[-1]
            if hasattr(QWidget, bare) or hasattr(QMainWindow, bare):
                out[name] = n
        return out


def bootstrap(root: str, venv: str | None):
    """sys.path for the embedded interpreter: the repository and the venv's
    site-packages (Rust calls this before importing anything else)."""
    global ROOT, VENV
    import site
    ROOT, VENV = root, venv
    if venv:
        packages = ("Lib/site-packages" if sys.platform == "win32" else "lib/python3*/site-packages")
        for sp in sorted(Path(venv).glob(packages)):
            site.addsitedir(str(sp))
        # Embedded, sys.executable is the `built` binary: code that runs
        # "sys.executable -m module" (the Metal probe, helpers) would start
        # another app window. It is the venv's Python, as in the Qt app.
        binaries = ["Scripts/python.exe"] if sys.platform == "win32" else ["bin/python3", "bin/python"]
        for name in binaries:
            python = Path(venv) / name
            if python.exists():
                sys.executable = str(python)
                break
    if root not in sys.path:
        sys.path.insert(0, root)
    os.chdir(root)

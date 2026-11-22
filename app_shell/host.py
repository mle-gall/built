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


def _log(*parts):
    print("[built]", *parts, file=sys.stderr, flush=True)


class Host:
    def __init__(self, ns_view: int, width: float, height: float, scale: float,
                 settings: str | None = None):
        t0 = time.perf_counter()
        self.settings_dir = qt_host.start(settings)
        self.ns_view = int(ns_view)
        self.width = max(1, int(round(width)))
        self.height = max(1, int(round(height)))
        self.scale = float(scale)
        self._frame_wanted = True
        self._wake_at = None
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
        from app_shell.document import ShellViewport
        from app_shell.view import ShellView
        from app_shell.window import ShellWindow
        from app_shell import files, modal
        self.window = ShellWindow(self)
        viewport = ShellViewport(self)
        try:
            viewport.history.max_steps = 200
        except Exception:
            pass
        view = ShellView(self)
        view.source_viewport = viewport
        view.camera = viewport.camera
        viewport.keys_stand_in = view
        self.window.bind(viewport, view)
        viewport.set_active_tool(None)
        modal.install(self)
        files.install(self.window)
        guard = qt_host.widget_guard()
        if guard is not None:
            guard.on_blocked = lambda title: self.not_here(f"“{title}”")
        try:
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
        from PySide6.QtCore import QTimer
        view.renderer = renderer
        if getattr(view, "_ui_timer", None) is None:
            view._ui_timer = QTimer(view, singleShot=True)
            view._ui_timer.timeout.connect(view.update)
        # What the chrome was last sent for this view is no longer what the
        # renderer shows: the signatures go (the defaults come back), the
        # view's own state (expanded rows, filters, mode) stays.
        resend = ("_ui_docs", "_ui_tabs", "_ui_opts", "_ui_palette", "_ui_colset",
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
        at = time.monotonic() + msec / 1000.0
        self._wake_at = at if self._wake_at is None else min(self._wake_at, at)

    def cursor_changed(self, cursor):
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
        view = self.view
        scene = self.viewport.scene
        # MainWindow._install_wgpu_display's present(): a scene whose
        # version moved since the resident pack is packed again.
        if view.renderer is not None and (scene is not getattr(view, "scene", None)
                                          or scene.version != getattr(view, "_shell_resident_version", None)):
            self._frame_wanted = True
        delay = None

        def sooner(ms):
            nonlocal delay
            if ms is not None:
                delay = ms if delay is None else min(delay, ms)

        ui_timer = getattr(view, "_ui_timer", None)
        if ui_timer is not None and ui_timer.isActive():
            sooner(max(0, ui_timer.remainingTime()))
        if self._wake_at is not None:
            wait = max(0.0, (self._wake_at - time.monotonic()) * 1000)
            sooner(wait)
            if wait == 0:
                self._wake_at = None
                self._frame_wanted = True
        # Qt timers fire only while we pump: wake when the first is due
        # (QTimer.start / singleShot from Python, app_shell.qt_host), and
        # poll the view's own at the Qt app's 33 ms present cadence.
        sooner(qt_host.next_timer_ms())
        if self._qt_timers_pending():
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
        return wanted, delay, cursor, title

    def extras(self):
        """What else the window should do this turn (rust/built_app
        python.rs ``Extras``)."""
        recent = None
        if self._recent_dirty:
            self._recent_dirty = False
            try:
                recent = [p for p in self.window._recent_paths() if Path(p).exists()]
            except Exception:
                recent = []
        return {"exit": bool(self.quit_now), "ime_area": self._ime_area, "recent": recent}

    def _qt_timers_pending(self):
        view = self.view
        for name in ("_settle_timer", "_resize_timer", "_ui_timer"):
            t = getattr(view, name, None)
            if t is not None and t.isActive():
                return True
        return (getattr(view, "_inference_future", None) is not None
                or getattr(view, "_inference_queued", False))

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
            out = view.draw_frame()
            if out is not None:
                self._picture_seen = True
            return out
        except Exception:
            self._error("frame")
            return None

    def modal_frame(self):
        """A frame while a question or a job holds the loop (app_shell.modal)."""
        ui_timer = getattr(self.view, "_ui_timer", None)
        due = ui_timer is not None and ui_timer.isActive() and ui_timer.remainingTime() <= 0
        if self._frame_wanted or due:
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
        t0 = time.perf_counter()
        ok = bool(self._guard("open", files.open_path, self.window, Path(path)))
        self.open_ms = {"total": (time.perf_counter() - t0) * 1000}
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

    def pointer(self, kind, x, y, button=0, held=0, modifiers=0):
        from app_shell.events import ContextMenuEvent, MouseEvent
        qt_host.INPUT.update(modifiers=int(modifiers), buttons=int(held), pos=(x, y))
        view = self.view
        if kind == "leave":
            return self._guard("leave", view.leaveEvent, None)
        ev = MouseEvent(kind, x, y, button, held, modifiers)
        handler = {"press": view.mousePressEvent, "release": view.mouseReleaseEvent,
                   "move": view.mouseMoveEvent, "double": view.mouseDoubleClickEvent}[kind]
        self._guard(f"pointer {kind}", handler, ev)
        if kind == "press" and button == 2 and not self._modal():
            # Qt on the Mac sends the context menu on the right press.
            self._guard("context menu", view.contextMenuEvent, ContextMenuEvent(x, y, modifiers))
        self.request_frame()

    def wheel(self, x, y, pixel_dx, pixel_dy, angle_dx, angle_dy, phase="none",
              modifiers=0, held=0, inverted=True):
        from app_shell.events import WheelEvent
        qt_host.INPUT["modifiers"] = int(modifiers)
        if self._modal():
            return
        ev = WheelEvent(x, y, pixel_dx, pixel_dy, angle_dx, angle_dy, phase, modifiers, held, inverted)
        self._guard("wheel", self.view.wheelEvent, ev)
        self.request_frame()

    def gesture(self, kind, value, x, y, modifiers=0):
        from app_shell.events import NativeGestureEvent
        if self._modal():
            return
        ev = NativeGestureEvent(kind, value, x, y, modifiers=modifiers)
        self._guard("gesture", self.view.event, ev)
        self.request_frame()

    def key(self, pressed, key, modifiers=0, text="", repeat=False):
        """A key, as Qt delivers it: ShortcutOverride first (the chrome, the
        value box and the tool may claim it), then the window's shortcuts
        (the tools' letters), else the view's key handler. While a sheet
        or an exclusive job holds the window, only they hear keys."""
        from app_shell.events import KeyEvent, ShortcutOverride
        from views import chrome_dialogs as cd
        qt_host.INPUT["modifiers"] = int(modifiers)
        view = self.view
        if self._modal():
            if pressed and not repeat:
                ev = KeyEvent(True, key, modifiers, text, repeat)
                if cd.current_sheet(self.window) is not None:
                    self._guard("sheet key", cd.sheet_key, self.window, ev)
                else:
                    self._guard("job key", cd.job_key, self.window, ev)
            self.request_frame()
            return True
        if pressed:
            override = ShortcutOverride(key, modifiers, text, repeat)
            self._guard("shortcut override", view.event, override)
            if not override.isAccepted() and not repeat and self._shortcut(key, modifiers):
                self.request_frame()
                return True
        ev = KeyEvent(pressed, key, modifiers, text, repeat)
        self._guard("key", view.keyPressEvent if pressed else view.keyReleaseEvent, ev)
        self.request_frame()
        return True

    def _shortcut(self, key, modifiers):
        from PySide6.QtGui import QKeySequence
        try:
            combo = QKeySequence(int(modifiers) | int(key))
        except Exception:
            return False
        for action in list(self.window._tool_actions.values()):
            seq = action.shortcut()
            if not seq.isEmpty() and seq.matches(combo) == QKeySequence.ExactMatch:
                action.trigger()
                return True
        return False

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
        if not on:
            self._guard("focus out", self.view.focusOutEvent, None)

    # ---- menus ---------------------------------------------------------
    def menu(self, action):
        """A menu command from the Rust menu bar (rust/built_app/src/menu.rs)."""
        w = self.window
        if self._modal():
            return None                 # a question or a job holds the window
        kind, _, arg = action.partition(":")
        out = None
        if kind == "tool":
            self._guard("tool", w._activate_tool, arg)
        elif action == "undo":
            self._guard("undo", w._on_undo)
        elif action == "redo":
            self._guard("redo", w._on_redo)
        elif action == "zoom_extents":
            self._guard("zoom extents", w._on_zoom_extents)
        elif kind == "view":
            self.viewport.camera.set_view(arg)
            self.viewport.camera.fit_box(*w._extents_bounds()) if w._extents_bounds() else None
        else:
            out = self._guard(f"menu {action}", w.run_command, action)
        view = self.view
        if view is not None and kind != "tool":
            self._guard("tool sync", view.tools.sync)
        self.request_frame()
        return out

    def close(self):
        for doc in list(self.window._documents):
            view = doc.stand_in
            if view is not None:
                self._guard("close", view.closeEvent, None)

    # ---- diagnostics -----------------------------------------------------
    def state(self):
        """A few facts for scripted runs and the smoke test."""
        from app_shell.window import BORROWED
        scene = self.viewport.scene
        mesh = scene.mesh
        tool = self.viewport.active_tool
        win = self.window
        guard = qt_host.widget_guard()
        return {
            "path": str(self.path) if self.path else None,
            "version": scene.version,
            "faces": len(getattr(mesh, "faces", ()) or ()),
            "edges": len(getattr(mesh, "edges", ()) or ()),
            "groups": len(getattr(scene, "groups", ()) or ()),
            "selection": len(getattr(scene, "selection", ()) or ()),
            "tool": type(tool).__name__ if tool is not None else None,
            "status": win.statusBar().currentMessage(),
            "documents": [d.display_name() for d in win._documents],
            "active": win._documents.index(win._doc) if win._doc in win._documents else -1,
            "dirty": bool(win._is_dirty()),
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
        }

    @staticmethod
    def _pasteboard_types():
        from app_shell import native
        return [t for t in native.pasteboard_types()[0] if "ingetrazo" in t]

    def missing(self):
        """Qt widget/window API the borrowed code asked for and the shell
        lacks (probes of the code's own lazily set attributes left out)."""
        from PySide6.QtOpenGLWidgets import QOpenGLWidget
        from PySide6.QtWidgets import QMainWindow
        from app_shell.borrow import MISSING
        out = {}
        for name, n in MISSING.most_common():
            bare = name.split(".", 1)[-1]
            if hasattr(QOpenGLWidget, bare) or hasattr(QMainWindow, bare):
                out[name] = n
        return out


def bootstrap(root: str, venv: str | None):
    """sys.path for the embedded interpreter: the repository and the venv's
    site-packages (Rust calls this before importing anything else)."""
    global ROOT, VENV
    import site
    ROOT, VENV = root, venv
    if venv:
        for sp in sorted(Path(venv).glob("lib/python3*/site-packages")):
            site.addsitedir(str(sp))
        # Embedded, sys.executable is the `built` binary: code that runs
        # "sys.executable -m module" (the Metal probe, helpers) would start
        # another app window. It is the venv's Python, as in the Qt app.
        for name in ("python3", "python"):
            python = Path(venv) / "bin" / name
            if python.exists():
                sys.executable = str(python)
                break
    if root not in sys.path:
        sys.path.insert(0, root)
    os.chdir(root)

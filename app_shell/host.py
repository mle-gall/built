"""The calls the Rust shell makes into Python, one object per window.

Everything runs on the main thread, under the GIL the Rust side takes for
each call; between calls Rust holds no GIL, so the snap-index worker and
other Python threads progress while winit waits for events.

Coordinates are logical pixels (Qt's), y down, from the window's top-left.
Modifier masks and key codes are already Qt's (rust/built_app/src/keys.rs).
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
        self._resident_version = None
        self.errors = []
        #: Set by the window handle's startSystemMove (the chrome's empty
        #: top strip); Rust starts the native window drag and clears it.
        self.window_drag_requested = False
        from app_shell.document import ShellViewport
        from app_shell.view import ShellView
        from app_shell.window import ShellWindow
        self.window = ShellWindow(self)
        self.viewport = ShellViewport(self)
        try:
            self.viewport.history.max_steps = 200
        except Exception:
            pass
        self.view = ShellView(self)
        self.view.source_viewport = self.viewport
        self.view.camera = self.viewport.camera
        self.viewport.keys_stand_in = self.view
        self.window.bind(self.viewport, self.view)
        self.viewport.set_active_tool(None)
        self.path = None
        self.startup_ms = (time.perf_counter() - t0) * 1000
        self._set_title()

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
            # A tool-shaped pixmap cursor (QCursor(QPixmap)) is Qt's 24.
            name = _CURSORS.get(int(getattr(shape, "value", shape)), "crosshair")
        except Exception:
            name = "default"
        self._cursor = name

    def _set_title(self):
        name = self.path.name if self.path else "Untitled"
        self._title = f"{name} — Built (Rust shell)"

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
                                          or scene.version != self._resident_version):
            self._frame_wanted = True
        delay = None
        ui_timer = getattr(view, "_ui_timer", None)
        if ui_timer is not None and ui_timer.isActive():
            delay = max(0, ui_timer.remainingTime())
        if self._wake_at is not None:
            wait = max(0.0, (self._wake_at - time.monotonic()) * 1000)
            delay = wait if delay is None else min(delay, wait)
            if wait == 0:
                self._wake_at = None
                self._frame_wanted = True
        # QTimers (singleShot from the borrowed code, the settle timer)
        # fire only while we pump: poll at the Qt app's own 33 ms present
        # cadence while any is pending, else sleep until an event.
        if self._qt_timers_pending():
            delay = 33 if delay is None else min(delay, 33)
        cursor, self._cursor = self._cursor, None
        title, self._title = self._title, None
        wanted, self._frame_wanted = self._frame_wanted, False
        return wanted, delay, cursor, title

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
            if scene is not getattr(view, "scene", None) or scene.version != self._resident_version:
                version = scene.version
                view.set_scene(scene)
                self._resident_version = version
            return view.draw_frame()
        except Exception:
            self._error("frame")
            return None

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
    def open(self, path):
        """Read a .built/.igz into the document (MainWindow._open_path_here,
        without recovery prompts, links UI or trays)."""
        path = Path(path)
        t0 = time.perf_counter()
        from formats import igz as igz_format
        vp = self.viewport
        suffix = path.suffix.lower()
        if suffix not in (".built", ".igz"):
            self.window.statusBar().showMessage(
                f"{suffix} import is not in the shell yet; open a .built or .igz", 6000)
            return False
        view = self.view
        if hasattr(view, "blank"):
            view.blank()
        try:
            igz_format.load_into(vp.scene, path)
        except Exception as exc:
            self._error("open")
            self.window.statusBar().showMessage(f"Open failed: {exc}", 8000)
            return False
        loaded = time.perf_counter()
        vp.history.clear()
        try:
            vp.reset_texture_cache()
        except Exception:
            self._error("reset_texture_cache")
        vp.inference.reset()
        self.window._apply_camera_dict(getattr(vp.scene, "camera_home", None))
        if not isinstance(getattr(vp.scene, "camera_home", None), dict):
            self.window._on_zoom_extents()
        self.path = path
        self.window._current_path = path
        vp.notify_scene_changed()
        version = vp.scene.version
        view.set_scene(vp.scene)
        self._resident_version = version
        self.open_ms = {"read": (loaded - t0) * 1000,
                        "pack": (time.perf_counter() - loaded) * 1000}
        self._set_title()
        self.request_frame()
        return True

    # ---- input ---------------------------------------------------------
    def _guard(self, where, fn, *args):
        try:
            return fn(*args)
        except Exception:
            self._error(where)
            return None

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
        if kind == "press" and button == 2:
            # Qt on the Mac sends the context menu on the right press.
            self._guard("context menu", view.contextMenuEvent, ContextMenuEvent(x, y, modifiers))
        self.request_frame()

    def wheel(self, x, y, pixel_dx, pixel_dy, angle_dx, angle_dy, phase="none",
              modifiers=0, held=0, inverted=True):
        from app_shell.events import WheelEvent
        qt_host.INPUT["modifiers"] = int(modifiers)
        ev = WheelEvent(x, y, pixel_dx, pixel_dy, angle_dx, angle_dy, phase, modifiers, held, inverted)
        self._guard("wheel", self.view.wheelEvent, ev)
        self.request_frame()

    def gesture(self, kind, value, x, y, modifiers=0):
        from app_shell.events import NativeGestureEvent
        ev = NativeGestureEvent(kind, value, x, y, modifiers=modifiers)
        self._guard("gesture", self.view.event, ev)
        self.request_frame()

    def key(self, pressed, key, modifiers=0, text="", repeat=False):
        """A key, as Qt delivers it: ShortcutOverride first (the chrome, the
        value box and the tool may claim it), then the window's shortcuts
        (the tools' letters), else the view's key handler."""
        from app_shell.events import KeyEvent, ShortcutOverride
        qt_host.INPUT["modifiers"] = int(modifiers)
        view = self.view
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
        from PySide6.QtCore import Qt
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
        renderer = self.view.renderer
        if renderer is not None and renderer.ui_claims_key(""):
            renderer.ui_text(text)
            self.request_frame()
            return
        for ch in text:
            code = ord(ch.upper()) if ch.isalpha() else ord(ch)
            self.key(True, code, 0, ch)
            self.key(False, code, 0, ch)

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
        kind, _, arg = action.partition(":")
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
        self.request_frame()

    def close(self):
        self._guard("close", self.view.closeEvent, None)

    # ---- diagnostics -----------------------------------------------------
    def state(self):
        """A few facts for scripted runs and the smoke test."""
        scene = self.viewport.scene
        mesh = scene.mesh
        tool = self.viewport.active_tool
        return {
            "path": str(self.path) if self.path else None,
            "version": scene.version,
            "faces": len(getattr(mesh, "faces", ()) or ()),
            "edges": len(getattr(mesh, "edges", ()) or ()),
            "groups": len(getattr(scene, "groups", ()) or ()),
            "selection": len(getattr(scene, "selection", ()) or ()),
            "tool": type(tool).__name__ if tool is not None else None,
            "status": self.window.statusBar().currentMessage(),
            "errors": len(self.errors),
            "widget_gaps": self.missing(),
        }

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
    import site
    if venv:
        for sp in sorted(Path(venv).glob("lib/python3*/site-packages")):
            site.addsitedir(str(sp))
    if root not in sys.path:
        sys.path.insert(0, root)
    os.chdir(root)

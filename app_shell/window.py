"""What views/wgpu_ui.py and the tools ask of "the window", without one.

The egui chrome reads the window's tool actions (checked state, text,
shortcut), its status bar, and runs ``_activate_tool``. :class:`ShellWindow`
answers those from the same tool registry MainWindow builds, so the chrome
code runs unchanged. Everything else MainWindow does (docks, trays, sheets,
dialogs) is not here yet; what the chrome and tools looked up and did not
find is counted in :data:`app_shell.borrow.MISSING`.
"""
from __future__ import annotations

import time

from PySide6.QtCore import Qt
from PySide6.QtGui import QKeySequence

from app_shell.borrow import MISSING


def build_tools() -> dict:
    """MainWindow's tool registry (views/main_window.py ``self._tools``)."""
    from tools.arc import ArcTool, CenterArcTool, PieTool, ThreePointArcTool
    from tools.circle import CircleTool, PolygonTool
    from tools.dimension import DimensionTool
    from tools.eraser import EraserTool
    from tools.fillet import FilletTool
    from tools.flip import FlipTool
    from tools.followme import FollowMeTool
    from tools.freehand import FreehandTool
    from tools.line import LineTool
    from tools.move import MoveTool
    from tools.offset import OffsetTool
    from tools.paint import PaintTool
    from tools.protractor import ProtractorTool
    from tools.pushpull import PushPullTool
    from tools.rectangle import RectangleTool
    from tools.rotate import RotateTool
    from tools.rotated_rectangle import RotatedRectangleTool
    from tools.scale import ScaleTool
    from tools.select import SelectTool
    from tools.tape import TapeMeasureTool
    from tools.text import TextTool
    tools = {
        "select": SelectTool(), "line": LineTool(), "freehand": FreehandTool(),
        "rectangle": RectangleTool(), "rotated_rect": RotatedRectangleTool(),
        "circle": CircleTool(), "polygon": PolygonTool(), "arc": ArcTool(),
        "arc3": ThreePointArcTool(), "center_arc": CenterArcTool(), "pie": PieTool(),
        "pushpull": PushPullTool(), "offset": OffsetTool(), "move": MoveTool(),
        "rotate": RotateTool(), "scale": ScaleTool(), "flip": FlipTool(),
        "followme": FollowMeTool(), "fillet": FilletTool(), "paint": PaintTool(),
        "dimension": DimensionTool(), "eraser": EraserTool(), "tape": TapeMeasureTool(),
        "protractor": ProtractorTool(), "text": TextTool(),
    }
    for key, tool in tools.items():
        if not getattr(tool, "icon", None):
            tool.icon = key
    return tools


class ShellAction:
    """The QAction surface the chrome reads."""

    def __init__(self, text, shortcut="", run=None, checkable=True):
        self._text = text
        self._shortcut = QKeySequence(shortcut) if shortcut else QKeySequence()
        self._run = run
        self._checked = False
        self._checkable = checkable
        self._enabled = True

    def text(self):
        return self._text

    def shortcut(self):
        return self._shortcut

    def isChecked(self):
        return self._checked

    def setChecked(self, on):
        self._checked = bool(on)

    def isCheckable(self):
        return self._checkable

    def isEnabled(self):
        return self._enabled

    def setEnabled(self, on):
        self._enabled = bool(on)

    def isVisible(self):
        return True

    def trigger(self):
        if self._run is not None:
            self._run()

    def toolTip(self):
        return self._text

    def statusTip(self):
        return ""

    def menu(self):
        return None


class _Label:
    def __init__(self):
        self._text = ""

    def text(self):
        return self._text

    def setText(self, text):
        self._text = text

    def setMaximumWidth(self, *_):
        pass


class _Timer:
    def __init__(self):
        self._until = 0.0

    def isActive(self):
        return time.monotonic() < self._until


class ShellStatusBar:
    """``statusBar()``: the base message (``name — hint``) and a timed
    flash, as views/wgpu_ui.before_frame reads them."""

    def __init__(self, host):
        self._host = host
        self._base = ""
        self._msg = _Label()
        self._timer = _Timer()

    def showMessage(self, text, msec=0):
        if msec:
            self._msg.setText(text)
            self._timer._until = time.monotonic() + msec / 1000.0
            self._host.wake_after(msec)
        else:
            self._base = text
        self._host.request_frame()

    def currentMessage(self):
        return self._msg.text() if self._timer.isActive() else self._base

    def clearMessage(self):
        self._timer._until = 0.0

    # The chrome hides the Qt status bar; there is none to hide here.
    def _noop(self, *args, **kwargs):
        return None

    hide = show = setMaximumWidth = setMaximumHeight = _noop

    def isVisible(self):
        return False


class _Document:
    def __init__(self, viewport, view):
        self.viewport = viewport
        self.stand_in = view
        self.view_grid = None
        self._renderer_wgpu = True


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


class ShellWindow:
    def __init__(self, host):
        if "_on_undo" not in vars(ShellWindow):
            borrow_main_window()
        self._host = host
        self._tools = build_tools()
        self._tool_actions = {}
        self._nav_actions = {}
        self._status = ShellStatusBar(host)
        self._current_path = None
        self._tool_filter = None
        self.viewport = None
        self._doc = None
        self._documents = []
        self.tray = None
        for key, tool in self._tools.items():
            shortcut = getattr(tool, "shortcut", "") or ""
            if key == "select":
                shortcut = "Space"          # bound by MainWindow, not the tool
            self._tool_actions[key] = ShellAction(
                getattr(tool, "name", key), shortcut, run=lambda k=key: self._activate_tool(k))

    def bind(self, viewport, view):
        self.viewport = viewport
        self._doc = _Document(viewport, view)
        self._documents = [self._doc]

    def __getattr__(self, name):
        if not name.startswith("__"):
            MISSING["window." + name] += 1
        raise AttributeError(name)

    # ---- what the chrome and tools call --------------------------------
    def statusBar(self):
        return self._status

    def menuBar(self):
        return _MenuBar()

    def findChildren(self, *_args, **_kwargs):
        return []

    def windowHandle(self):
        return _WindowHandle(self._host)

    def windowState(self):
        return Qt.WindowNoState

    def dockWidgetArea(self, _dock):
        return Qt.NoDockWidgetArea

    def active_style(self):
        return getattr(self.viewport.scene, "display_style", None)

    def _activate_tool(self, key: str) -> None:
        """MainWindow._activate_tool's Metal path."""
        tool = self._tools.get(key)
        if tool is None:
            self._status.showMessage(f"{key} is not in the shell yet", 3000)
            return
        self.viewport.set_active_tool(tool)
        display = self._doc.stand_in
        display.tools.sync()
        display.set_scene(self.viewport.scene)
        display.update()
        for k, action in self._tool_actions.items():
            action.setChecked(k == key)
        name = getattr(tool, "name", key)
        hint = getattr(tool, "description", "") or ""
        self._status.showMessage(f"{name} — {hint}" if hint else name)

    def show_viewport_context_menu(self, *args, **kwargs):
        self._status.showMessage("Context menus are not in the shell yet", 2000)



#: MainWindow methods that only reach ``self.viewport`` and ``self._doc``:
#: borrowed as they are (app_shell.borrow).
_FROM_MAIN_WINDOW = ("_CAMERA_FIELDS", "_apply_camera_dict", "_apply_camera_home",
                     "_on_undo", "_on_redo", "_on_zoom_extents", "_extents_bounds",
                     "_figure_bounds", "_image_bounds", "_on_zoom_selection")


def borrow_main_window():
    from app_shell.borrow import borrow
    from views.main_window import MainWindow
    borrow(MainWindow, ShellWindow, skip=set(vars(MainWindow)) - set(_FROM_MAIN_WINDOW))

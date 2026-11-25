"""Window geometry, frame requests, focus and cursor services for document hosts."""
from __future__ import annotations

from collections import Counter

from core.input_types import PixelPoint, ScreenPoint, ScreenRect, ScreenSize, Input

#: Widget attributes the copied code looked up and the shim did not have.
MISSING: Counter = Counter()


class WidgetShim:
    """The window services exposed to document and plugin callbacks."""

    _shim_visible = False

    def __init__(self, parent=None, host=None):
        super().__init__()
        self._host = host
        self._cursor = Input.ArrowCursor
        self._focus = False
        self._enabled = True

    def __getattr__(self, name):
        if not name.startswith("__"):
            MISSING[name] += 1
        raise AttributeError(name)

    # ---- geometry -----------------------------------------------------
    def width(self):
        return self._host.width if self._host else 1280

    def height(self):
        return self._host.height if self._host else 800

    def size(self):
        return ScreenSize(self.width(), self.height())

    def rect(self):
        return ScreenRect(0, 0, self.width(), self.height())

    def devicePixelRatioF(self):
        return float(self._host.scale) if self._host else 2.0

    def devicePixelRatio(self):
        return self.devicePixelRatioF()

    def mapToGlobal(self, p):
        return ScreenPoint(p) if isinstance(p, ScreenPoint) else PixelPoint(p)

    def mapFromGlobal(self, p):
        return ScreenPoint(p) if isinstance(p, ScreenPoint) else PixelPoint(p)

    def mapTo(self, _other, p):
        return p

    def mapFrom(self, _other, p):
        return p

    def window(self):
        return self._host.window if self._host else None

    def windowHandle(self):
        return None

    def parentWidget(self):
        return None

    def isVisible(self):
        return self._shim_visible

    def isHidden(self):
        return not self._shim_visible

    def isEnabled(self):
        return self._enabled

    def setEnabled(self, on):
        self._enabled = bool(on)

    def updatesEnabled(self):
        return True

    def underMouse(self):
        return True

    # ---- painting -----------------------------------------------------
    def update(self, *args):
        if self._host is not None:
            self._host.request_frame()

    repaint = update

    # ---- cursor and focus ---------------------------------------------
    def cursor(self):
        return self._cursor

    def setCursor(self, cursor):
        self._cursor = cursor
        if self._host is not None:
            self._host.cursor_changed(cursor)

    def unsetCursor(self):
        self.setCursor(Input.ArrowCursor)

    def setFocus(self, *args):
        self._focus = True

    def clearFocus(self):
        self._focus = False

    def hasFocus(self):
        return True

    # ---- setters the widgets' __init__ and tools call: no-ops ------------
    def _noop(self, *args, **kwargs):
        return None

    def setAttribute(self, attr, on=True):
        self.__dict__.setdefault("_shim_attrs", {})[attr] = bool(on)

    def testAttribute(self, attr):
        if attr == Input.WA_SetCursor:
            return self._cursor is not Input.ArrowCursor
        return self.__dict__.get("_shim_attrs", {}).get(attr, False)

    setFormat = setMinimumSize = setFocusPolicy = setMouseTracking = _noop
    setLayoutDirection = setUpdatesEnabled = setToolTip = _noop
    setAcceptDrops = setContextMenuPolicy = setSizePolicy = grabKeyboard = _noop
    releaseKeyboard = grabMouse = releaseMouse = raise_ = activateWindow = _noop
    setStatusTip = setWhatsThis = show = hide = _noop

    def event(self, e):
        return False
    # ---- default event handlers (what super() reaches) -------------------
    keyPressEvent = keyReleaseEvent = mousePressEvent = mouseMoveEvent = _noop
    mouseReleaseEvent = mouseDoubleClickEvent = wheelEvent = focusOutEvent = _noop
    focusInEvent = leaveEvent = enterEvent = showEvent = hideEvent = _noop
    resizeEvent = closeEvent = paintEvent = contextMenuEvent = _noop

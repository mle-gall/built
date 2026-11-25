"""Unshown document controller and renderer-owned capture for interaction tests."""
from types import SimpleNamespace
from weakref import proxy

from core.input_codes import Type
from core.input_types import NATIVE
from ingetrazo_core import WindowInput, capture_document_frame
from ingetrazo_render import WindowTimers
from views.document_interaction import DocumentInteraction
from views.view_host import WidgetShim


class _InputSurface(WidgetShim):
    _event_handlers = {
        Type.KeyPress: "keyPressEvent",
        Type.KeyRelease: "keyReleaseEvent",
        Type.Wheel: "wheelEvent",
        Type.MouseMove: "mouseMoveEvent",
        Type.MouseButtonPress: "mousePressEvent",
        Type.MouseButtonRelease: "mouseReleaseEvent",
        Type.MouseButtonDblClick: "mouseDoubleClickEvent",
        Type.ContextMenu: "contextMenuEvent",
    }

    def event(self, event):
        handler = self._event_handlers.get(event.type())
        if handler is None:
            return super().event(event)
        getattr(self, handler)(event)
        return True


class ControllerSurface(DocumentInteraction, _InputSurface):
    def __init__(self, parent=None):
        host = SimpleNamespace(width=640, height=480, scale=1., ns_view=0,
                               timers=WindowTimers(), input_state=WindowInput(), window=proxy(self),
                               request_frame=lambda: None, cursor_changed=lambda cursor: None)
        if NATIVE:
            from app_shell.host import Host
            host.tool_cursor = Host.tool_cursor
        super().__init__(parent, host=host)

    def resize(self, width, height=None):
        if height is None:
            width, height = width.width(), width.height()
        self._host.width, self._host.height = width, height

    def _on_smart_zoom(self):
        self.smartZoomRequested.emit()

    def close(self):
        self.closeEvent(None)
        self._host.timers.cancel_context(self)
        self._shim_visible = False
        return True

    def _depth_world_at(self, x, y):
        return None

    def reset_texture_cache(self):
        self.reset_document_caches()

class _PixelCount(int):
    # Existing frame callers use width(), while raster callers read width.
    __call__ = int.__int__


class _DocumentImage:
    """Expose the renderer-owned capture to both frame and raster assertions."""

    def __init__(self, controller):
        self._frame = capture_document_frame(controller)
        self.width = _PixelCount(self._frame.width())
        self.height = _PixelCount(self._frame.height())

    def __getattr__(self, name):
        return getattr(self._frame, name)


live_image = _DocumentImage

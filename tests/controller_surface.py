"""Unshown document controller and renderer-owned capture for interaction tests."""
from types import SimpleNamespace
from weakref import proxy

from core.input_codes import Type
from ingetrazo_core import capture_document_frame as live_image
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
                               timers=WindowTimers(), window=proxy(self),
                               request_frame=lambda: None, cursor_changed=lambda cursor: None)
        super().__init__(parent, host=host)

    def resize(self, width, height=None):
        if height is None:
            width, height = width.width(), width.height()
        self._host.width, self._host.height = width, height

    def _on_smart_zoom(self):
        self.smartZoomRequested.emit()

    def close(self):
        self._shim_visible = False
        return True

    def _depth_world_at(self, x, y):
        return None

    def reset_texture_cache(self):
        self.reset_document_caches()

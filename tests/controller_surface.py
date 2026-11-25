"""Unshown document controller and renderer-owned capture for interaction tests."""
from types import SimpleNamespace
from weakref import proxy

from core.input_codes import Type
from ingetrazo_core import WindowInput
from ingetrazo_render import SurveyRaster, WindowTimers
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
        from ingetrazo_render import Host
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

def live_image(controller):
    """Read back a rendered document frame at the surface's physical size."""
    display = controller.keys_stand_in
    display.set_scene(controller.scene)
    display._prepare_pick_frame()
    display.renderer.request_capture()
    display.draw_frame()
    rgba = bytes(display.renderer.take_capture())
    width = round(display.width() * display.devicePixelRatioF())
    assert width > 0 and rgba and len(rgba) % (4 * width) == 0
    return SurveyRaster(width, len(rgba) // (4 * width), rgba)

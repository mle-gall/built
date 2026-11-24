"""A document with native input and window timers, independent of a window system."""
from types import SimpleNamespace
from ingetrazo_render import WindowTimers, SurveyRaster
from ingetrazo_core import WindowInput
from views.view_host import WidgetShim
from views.document_interaction import DocumentInteraction


class ControllerSurface(DocumentInteraction, WidgetShim):
    def __init__(self, parent=None):
        from app_shell.host import Host
        services = SimpleNamespace(width=640, height=480, scale=1., window=None,
                                   tool_cursor=Host.tool_cursor, request_frame=lambda: None,
                                   cursor_changed=lambda cursor: None,
                                   timers=WindowTimers(), input_state=WindowInput())
        super().__init__(parent, host=services)

    def resize(self, width, height):
        self._host.width, self._host.height = width, height

    def close(self):
        self.closeEvent(None)
        self._host.timers.cancel_context(self)
        return True

    def _depth_world_at(self, x, y):
        return None

    def reset_texture_cache(self):
        self.reset_document_caches()


def live_image(controller):
    """Capture the document's own wgpu surface at its physical resolution."""
    display = controller.keys_stand_in
    display.set_scene(controller.scene)
    display._prepare_pick_frame()
    display.renderer.request_capture()
    display.draw_frame()
    pixels = bytes(display.renderer.take_capture())
    width = round(display.width() * display.devicePixelRatioF())
    assert pixels and len(pixels) % (width * 4) == 0
    return SurveyRaster(width, len(pixels) // (width * 4), pixels)

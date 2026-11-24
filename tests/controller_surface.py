"""Unshown document controller and renderer-owned capture for interaction tests."""
from ingetrazo_core import capture_document_frame as live_image
from views.document_interaction import DocumentInteraction
from views.view_host import WidgetShim


class ControllerSurface(DocumentInteraction, WidgetShim):
    _width, _height = 640, 480

    def resize(self, width, height):
        self._width, self._height = width, height

    def width(self):
        return self._width

    def height(self):
        return self._height

    def close(self):
        self._shim_visible = False
        return True

    def _depth_world_at(self, x, y):
        return None

    def reset_texture_cache(self):
        self.reset_document_caches()

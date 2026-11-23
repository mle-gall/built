"""An unshown QWidget for tests that send Qt events to document interaction."""
from PySide6.QtWidgets import QWidget
from views.document_interaction import DocumentInteraction


class ControllerSurface(DocumentInteraction, QWidget):
    def _depth_world_at(self, x, y):
        return None

    def reset_texture_cache(self):
        self.reset_document_caches()


def live_image(controller):
    """Capture the document's own wgpu surface at its physical resolution."""
    from PySide6.QtGui import QImage
    display = controller.keys_stand_in
    display.set_scene(controller.scene)
    display._prepare_pick_frame()
    display.renderer.request_capture()
    display.draw_frame()
    pixels = bytes(display.renderer.take_capture())
    width = round(display.width() * display.devicePixelRatioF())
    assert pixels and len(pixels) % (width * 4) == 0
    frame = QImage(pixels, width, len(pixels) // (width * 4),
                   width * 4, QImage.Format_RGBA8888).copy()
    frame.setDevicePixelRatio(display.devicePixelRatioF())
    return frame

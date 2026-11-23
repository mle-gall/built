"""Document editing over a Metal window supplied by either application."""
from views.view_host import WidgetShim
from views.viewport import DocumentInteraction


class DocumentController(DocumentInteraction, WidgetShim):
    def __init__(self, host=None):
        super().__init__()
        self._host = host
        self._enabled = False

    @classmethod
    def for_widget(cls, display):
        controller = cls(QtViewHost(display))
        controller.setParent(display)
        controller.keys_stand_in = display
        display.source_viewport = controller
        display.camera = controller.camera
        return controller

    def render_image(self, *args, **kwargs):
        return self.keys_stand_in.render_image(*args, **kwargs)


class QtViewHost:
    """Window services used by tools attached to a Qt Metal surface."""

    def __init__(self, display):
        self.display = display

    @property
    def window(self):
        return self.display.window()

    @property
    def width(self):
        return self.display.width()

    @property
    def height(self):
        return self.display.height()

    @property
    def scale(self):
        return self.display.devicePixelRatioF()

    def request_frame(self):
        self.display.update()

    def cursor_changed(self, cursor):
        self.display.setCursor(cursor)

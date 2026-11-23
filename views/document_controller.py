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

    def mapToGlobal(self, point):
        return self.keys_stand_in.mapToGlobal(point)

    def mapFromGlobal(self, point):
        return self.keys_stand_in.mapFromGlobal(point)

    def upload_terrain(self, terrain):
        self._reference_revision = getattr(self, '_reference_revision', 0) + 1
        self.update()

    def upload_photo_mesh(self, mesh, images=None):
        if mesh is not None and images is not None:
            mesh.images = images
        self._reference_revision = getattr(self, '_reference_revision', 0) + 1
        self.update()

    def release_photo_textures(self):
        self._reference_revision = getattr(self, '_reference_revision', 0) + 1
        self.update()

    def max_texture_size(self):
        renderer = getattr(self.keys_stand_in, 'renderer', None)
        return renderer.max_texture_size() if renderer is not None else 4096

    def _photo_showing(self):
        mesh = getattr(self.scene, 'photo_mesh', None)
        return bool(mesh and mesh.visible and mesh.triangle_count and self.scene.entity_visible(mesh))

    def render_image(self, width_px, height_px=None, overlays=True, annotations_only=False,
                     sharp_shadows=False, transparent=False, crop_to_frame=False):
        if crop_to_frame and height_px is None:
            return self._render_framed_image(width_px, overlays, annotations_only,
                                             sharp_shadows, transparent)
        return self.keys_stand_in.render_image(
            width_px, height_px, overlays=overlays, sharp_shadows=sharp_shadows,
            transparent=transparent, annotations_only=annotations_only)


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

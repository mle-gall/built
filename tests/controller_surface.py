"""An unshown QWidget for tests that send Qt events to document interaction."""
from PySide6.QtWidgets import QWidget
from views.document_interaction import DocumentInteraction


class ControllerSurface(DocumentInteraction, QWidget):
    def _depth_world_at(self, x, y):
        return None

    def reset_texture_cache(self):
        self.reset_document_caches()

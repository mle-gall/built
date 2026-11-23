"""The native window's non-widget document controller."""
from app_shell.borrow import WidgetShim
from views.viewport import DocumentInteraction


class ShellViewport(DocumentInteraction, WidgetShim):
    def __init__(self, host=None):
        super().__init__()
        self._host = host
        self._enabled = False

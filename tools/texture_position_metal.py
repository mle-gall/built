"""Submit the placement machine's UV map to the editable face's material rows."""

from ingetrazo_core import register_tool_adapter
from time import perf_counter

from ingetrazo_core import TextureToolRows


@register_tool_adapter
class TexturePreview:
    def __init__(self, view):
        self.view = view
        self._packed = self._rows = self._target = self.signature = None
        self.overlay_only = False
        self.samples_ms = []

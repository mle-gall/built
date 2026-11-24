"""Submit the placement machine's UV map to the editable face's material rows."""
from time import perf_counter

from ingetrazo_core import TextureToolRows


class TexturePreview:
    def __init__(self, view):
        self.view = view
        self._packed = self._rows = self._target = self.signature = None
        self.overlay_only = False
        self.samples_ms = []

    def update(self, tool):
        if not tool.active:
            self.restore()
            return
        view = self.view
        if view.renderer is None or getattr(view, '_instance_updates', None) is None:
            return
        target = (tool.face.key, tool.side)
        if self._packed is not view.packed or self._target != target:
            self.restore()
            self._rows = TextureToolRows.prepare(view, tool.face, tool.side)
            self._packed, self._target = view.packed, target
        uvw = tool.map.uvw()
        if uvw is None or uvw == self.signature or self.overlay_only and tool.gesture_active:
            return
        start = perf_counter()
        self._rows.upload(view.renderer, uvw)
        self.samples_ms.append((perf_counter() - start) * 1000)
        self.signature = uvw

    def restore(self):
        if (self._rows is not None and self.view.renderer is not None
                and self._packed is self.view.packed):
            self._rows.upload(self.view.renderer, None)
        self._packed = self._rows = self._target = self.signature = None
        self.overlay_only = False

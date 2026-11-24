# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""More Metal views of one document (views/multi_view.py), sharing its
resident scene.

The document's first view (``WgpuView``, the window's stand-in) packs the
scene and sends it to the graphics card once. Every other view of the grid
is a :class:`WgpuPeerView`: a native surface of its own on the SAME
renderer (``Renderer.add_target``), with its own camera, style, overlay,
guides, tool preview, section caps and pick IDs, and nothing else — no
second pack, no second upload, no second snap index. A view's calls go
through a :class:`TargetRenderer`, which makes that view's surface the live
one first (``Renderer.bind_target``); the document's buffers (instances,
geometry, materials, textures, the sun's depth map) are the same for all.

What the peer reads of the document — the pack, its rows, the snap index,
the containers' boxes, the edit context — is the first view's
(:data:`SHARED`). Its camera and style come from its own (hidden) GL model
viewport, the cell's ``viewport``, exactly as the first view's come from
the document's viewport; so styles, projections, standard views, Zoom
Extents and the layout's state work as with OpenGL.
"""
from __future__ import annotations

from PySide6.QtWidgets import QWidget
from views.wgpu_view import WgpuView


from views.resident_peer import ResidentPeer, TargetRenderer, _release, raw_renderer


class WgpuPeerView(ResidentPeer, WgpuView):
    # ---- its surface -------------------------------------------------------
    def _attach(self) -> bool:
        """This view's surface on the first view's renderer, made once."""
        if self.renderer is not None:
            return True
        primary = self.primary
        if primary.renderer is None:
            primary._upload(force=True)
        raw = raw_renderer(primary.renderer)
        if raw is None or not hasattr(raw, "add_target"):
            return False
        window, handles = self._window_handles()
        self._target = raw.add_target(window, max(1, self.width()),
                                      max(1, self.height()),
                                      self.devicePixelRatioF(), **handles)
        self.renderer = TargetRenderer(raw, self._target)
        self._physical_size = None
        # Let the surface go with the widget, not with the Python object.
        target = self._target
        self.destroyed.connect(lambda *_a, r=raw, t=target: _release(r, t))
        return True

    def draw_frame(self):
        # Nothing resident yet (the document still packing): nothing to draw.
        if self.primary.packed is None:
            return None
        return super().draw_frame()

    def showEvent(self, event):
        QWidget.showEvent(self, event)
        self._upload()

    def closeEvent(self, event):
        # Not the document's tracker, worker or renderer: only its surface.
        self._closed = True
        self.release()
        QWidget.closeEvent(self, event)

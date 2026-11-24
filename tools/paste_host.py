# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Paste tool: drop a copied set of geometry, placing it with the cursor.

After Copy/Cut fills ``viewport.clipboard``, Paste activates this tool: the copied
faces, edges and groups follow the cursor as a live preview (snapping like any
draw), and a click stamps them into the scene ONCE and returns to Select.
The clipboard is kept — paste again for another copy.
"""
from __future__ import annotations
from ingetrazo_core import Vec3 as QVector3D

from tools.base import Tool


class PasteHost(Tool):
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = True
    _clipboard = None
    _preview_target = None

    @property
    def _clip(self):
        return self._clipboard

    @_clip.setter
    def _clip(self, clip):
        self._clipboard = clip
        self._machine.set_paste_clipboard(clip or None)

    def on_activate(self, vp):
        self._clip = getattr(vp, "clipboard", None)
        self._send(vp, {"type": "activate"})

    def on_hover(self, ctx):
        self._send(ctx.viewport, self._pointer("hover", ctx))

    def on_click(self, ctx):
        if self._send(ctx.viewport, self._pointer("click", ctx))["committed"]:
            # The handoff can synchronously deactivate Paste.
            window = getattr(ctx.viewport, "window", None)
            window = window() if callable(window) else None
            if window is not None and hasattr(window, "_activate_tool"):
                window._activate_tool("select")
            ctx.viewport.update()

    def on_cancel(self, vp):
        self._send(vp, {"type": "cancel"})
        self._clipboard = None
        vp.update()

    def on_deactivate(self, vp):
        self._send(vp, {"type": "deactivate"})
        self._clipboard = None

    @classmethod
    def in_place(cls, vp):
        tool = cls()
        tool._clip = getattr(vp, "clipboard", None)
        if not tool._clip:
            return False
        ok = tool.stamp(vp, QVector3D())
        vp.update()
        return ok

    def stamp(self, vp, off):
        return self._send(vp, {"type": "paste_at", "offset": off})["committed"]

    def preview_faces(self):
        if self._clip is None:
            return []
        return self._machine.paste_faces(self._clip.get("faces", ()), self._offset)

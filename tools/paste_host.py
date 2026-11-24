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

from core.group import translated_attrs
from core.texture import face_uv_axes


def _preview_attrs(attrs, off, face):
    """Attrs for a preview face at the cursor offset. A POSITIONED texture's
    world-anchored uvw map is shifted to follow the drag. A PLANAR texture
    (no uvw — hand-painted faces, billboard figures) would be re-projected
    at every cursor position and the image SWIMS through the model; the
    renderer's planar projection is baked into a uvw here (same axes, rot,
    sw/sh) so the image rides with the drag looking exactly as it did at
    the original spot."""
    if not attrs:
        return None
    t = attrs.get("texture")
    if t and t.get("path") and not t.get("uvw"):
        gu, cu, gv, cv = face_uv_axes(t, face.normal())   # the renderer's map
        uvw = [gu.x(), gu.y(), gu.z(), cu, gv.x(), gv.y(), gv.z(), cv]
        attrs = {**attrs, "texture": {**t, "uvw": uvw}}
    return translated_attrs(attrs, off)


from tools.base import Tool


class PasteHost(Tool):
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = True
    _clipboard = None
    _preview_on = False

    @property
    def _clip(self):
        return self._clipboard

    @_clip.setter
    def _clip(self, clip):
        self._clipboard = clip
        self._machine.set("clipboard", None if not clip else {
            "ref": clip.get("ref", (0., 0., 0.)), "faces": [(loop, *holes) for loop, holes, *_ in clip["faces"]],
            "edges": [(a, b) for a, b, *_ in clip["edges"]]})

    def on_activate(self, vp):
        self._clip = getattr(vp, "clipboard", None)
        self._send(vp, {"type": "activate"})

    def host_begin_paste_preview(self, vp, _):
        groups = (self._clip or {}).get("groups")
        begin = getattr(vp, "begin_groups_preview", None)
        self._preview_on = bool(groups and callable(begin))
        if self._preview_on:
            begin(groups, external=True)

    def host_end_paste_preview(self, vp, _):
        if self._preview_on:
            end = getattr(vp, "end_groups_preview", None)
            if end:
                end()
            self._preview_on = False

    def host_pose_paste_preview(self, vp, offset):
        if self._preview_on:
            vp.set_groups_preview_offset(QVector3D(*offset))
        else:
            vp.update()

    def host_can_stamp(self, vp, _):
        return bool(self._clip and any(self._clip.get(k) for k in ("faces", "edges", "groups")))

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
        faces = super().preview_faces()
        for face, (_, _, *attrs) in zip(faces, self._clip["faces"]):
            if attrs:
                face.attrs = _preview_attrs(attrs[0], self._offset, face)
        return faces

# SPDX-License-Identifier: GPL-3.0-or-later
"""Placement bounds, projected mesh previews and host selection handoff."""
from ingetrazo_core import Vec3 as QVector3D, ToolMachine

class PlaceGroupHost:
    name = "Place component"
    uses_snap = True
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = False      # the pending component floats on top

    def __init__(self, group, align_to_face: bool = False,
                 anchor: QVector3D | None = None) -> None:
        super().__init__()
        self._group = group
        from ingetrazo_core import placement_template
        self._anchor, self._segments = placement_template(group, align_to_face, anchor)
        self._align = align_to_face
        self._attachment = None
        self._machine.set("load", (group.uid, self._anchor.toTuple(), self._align, self._segments))

    @staticmethod
    def _face_frame(normal):
        return tuple(QVector3D(*p) for p in ToolMachine("place_group").query("basis", normal.toTuple()))

    def on_click(self, ctx):
        if self._group is not None and self._align and self._group.text3d:
            from core.annotation_anchor import picked_attachment
            self._attachment = picked_attachment(ctx)
        self._drop(ctx.viewport, self._pointer("click", ctx))

    def place_at(self, viewport, world, *, attachment=None):
        self._attachment = attachment
        self._drop(viewport, {"type": "paste_at", "offset": world.toTuple()})

    def _drop(self, vp, event):
        from core.i18n import tr
        if self._send(vp, event)["committed"]:
            self._group = None
            vp.flash_status(tr("Component placed — Move (M) adjusts it"))
            window = vp.window()
            if hasattr(window, "_activate_tool"):
                window._activate_tool("select")
            vp.update()

    def on_cancel(self, vp):
        self._group = None
        super().on_cancel(vp)

    def on_deactivate(self, vp):
        self._group = None
        super().on_deactivate(vp)

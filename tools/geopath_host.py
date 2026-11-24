# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the geopath tool gestures."""
from tools.rust_tool_base import RustTool
from ingetrazo_core import Vec3


class GeoPathTool(RustTool):
    kind = "geopath"
    _saved_view = None
    work_plane = (Vec3(), Vec3(0, 0, 1))
    nodes = property(lambda self: [Vec3(*p) for p in self._state("nodes")])
    hover_point = property(lambda self: self._point("hover_point"))
    hover_elevation = property(lambda self: self._state("elevation"))

    def host_geo_plan(self, vp, enter):
        from core.i18n import tr
        if enter and vp.has_ground_surface():
            cam = vp.camera
            self._saved_view = (cam.perspective, cam.yaw, cam.pitch)
            cam.perspective = False
            cam.set_view("top")
            vp.flash_status(tr("Top view, parallel — so the trace lands where you click. Restored when you leave the tool."))
            vp.update()
        elif not enter and self._saved_view is not None:
            vp.camera.perspective, vp.camera.yaw, vp.camera.pitch = self._saved_view
            self._saved_view = None
            vp.update()

    def host_geo_preview(self, vp, args):
        key, i, point = args
        path = self._handles.get(key)
        if path in vp.scene.geo_paths and i < len(path.points):
            path.points[i] = Vec3(*point)
            vp.scene.version += 1

    def on_key(self, vp, key, modifiers):
        from PySide6.QtCore import Qt
        if key not in (Qt.Key_Enter, Qt.Key_Return):
            return False
        self._send(vp, {"type": "key", "key": "Enter"})
        vp.update()
        return True

    def on_double_click(self, ctx):
        self._send(ctx.viewport, self._pointer("double_click", ctx))
        ctx.viewport.update()

    def value_label(self):
        from core.units import fmt_len
        label = self._state("label")
        if label is None:
            return None
        at, run, elevation, rise, grade = label
        parts = [fmt_len(v) for v in (run, elevation) if v is not None]
        if rise is not None:
            parts.append(f"{'+' if rise >= 0 else ''}{fmt_len(rise)} ({grade:+.1f}%)")
        return ("  ·  ".join(parts), Vec3(*at)) if parts else None

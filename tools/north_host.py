# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the north tool gestures."""
from tools.rust_tool_base import RustTool
from ingetrazo_core import ToolMachine, Vec3
from tools.rust_tool_base import _tuple


COMPASS_PX = 70.0


def refresh_north_display(viewport) -> None:
    """After the north changed on a located model: the map is redrawn from
    the new datum and the terrain rebuilt (``tilesChanged`` → the window's
    terrain build). Without a location only the sun moved: a repaint."""
    scene = getattr(viewport, "scene", None)
    if getattr(scene, "georef", None) is not None:
        reset = getattr(viewport, "reset_tiles", None)
        if callable(reset):
            reset()
        changed = getattr(viewport, "tilesChanged", None)
        if changed is not None:
            changed.emit()
    notify = getattr(viewport, "notify_scene_changed", None)
    if callable(notify):
        notify()


def north_deg_towards(centre, point):
    return ToolMachine("north").query("angle", [_tuple(centre), _tuple(point)])


def compass_segments(centre, radius, north_deg):
    return [(Vec3(*a), Vec3(*b)) for a, b in ToolMachine("north").query("compass", [_tuple(centre), radius, north_deg])]


class NorthTool(RustTool):
    kind = icon = "north"
    wireframe_color = (0.80, 0.16, 0.14, 1.0)
    centre = property(lambda self: self._point("centre"))
    hover_point = property(lambda self: self._point("hover_point"))
    _radius = property(lambda self: self._state("radius"))

    def value_is_unitless(self):
        return True

    def vcb_caption(self):
        return "Angle"

    @property
    def work_plane(self):
        return (self.centre, Vec3(0, 0, 1)) if self.centre is not None else None

    @work_plane.setter
    def work_plane(self, _):
        pass  # North keeps the compass horizontal.

    def pending_deg(self):
        return self._state("pending")

    def _send(self, vp, event):
        self._scene = getattr(vp, "scene", None)
        return super()._send(vp, event)

    def rubber_band_lines(self):
        from core.sun import scene_north_deg
        if self._during is None:
            self._machine.set("north", scene_north_deg(getattr(self, "_scene", None)))
        return super().rubber_band_lines()

    def host_north_angle(self, vp, _):
        from core.sun import scene_north_deg
        return scene_north_deg(getattr(vp, "scene", None))

    def host_north_pixels(self, vp, pts):
        project = getattr(vp, "_world_to_pixel", None)
        return [project(Vec3(*p)) for p in pts] if project else None

    def host_north_done(self, vp, result):
        from core.i18n import tr
        from core.units import fmt_angle_deg
        deg, changed = result
        if changed:
            refresh_north_display(vp)
        if hasattr(vp, "flash_status"):
            vp.flash_status(tr("North: {a}").format(a=fmt_angle_deg(deg)), 4000)
        vp.update()

    def commit(self, vp, deg):
        self.on_value(vp, deg)

    def value_label(self):
        from core.units import fmt_angle_deg
        deg = self.pending_deg()
        if deg is not None and self.hover_point is not None:
            return fmt_angle_deg(deg), self.hover_point

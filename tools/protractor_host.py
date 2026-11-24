# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Viewport and extension bindings for the shared Rust protractor cursor."""
from __future__ import annotations


from core.input_types import Qt
from ingetrazo_core import Vec3 as QVector3D

from core.guide import Guide
from ingetrazo_core import ToolDisc
from core.snap import COLOR_AXIS_X, COLOR_AXIS_Y, COLOR_AXIS_Z
from core.axes import plane_axes  # drawing axes (#44)
from tools.base import Tool, ToolContext

# The disc keeps a fixed SCREEN size; ticks every 15 degrees.
DISC_PX = 60.0
TICK_DEG = 15.0
# The drawing axes (core.axes): the open group's own inside it (#44).
from core.axes import AXES as _AXES  # noqa: E402
_AXIS_RGBA = {"x": (*COLOR_AXIS_X, 1.0), "y": (*COLOR_AXIS_Y, 1.0),
              "z": (*COLOR_AXIS_Z, 1.0)}
_OFF_AXIS_RGBA = (0.24, 0.27, 0.32, 1.0)


class ProtractorBase(ToolDisc, Tool):
    """Shared protractor state + behaviour (see module docstring)."""

    def value_is_unitless(self) -> bool:
        """The typed value is an ANGLE: the document's length unit must
        not scale it («45» was 0.045° in a millimetre model, #176)."""
        return True

    #: The Line tool's axis magnet for the arms (@pacaeiro, issue #41): a
    #: base arm within 3° of an axis that lies in the disc's plane lands
    #: on it. Only the world half — the screen detector would hand back a
    #: point on an axis OUTSIDE the disc's plane, which an arm cannot be.
    magnetic_axis_deg = 3.0
    #: The arms are directions from the centre, not lines being drawn: the
    #: snap engine leaves out 'through point', 'extension' and 'from point'
    #: for them, so the axis magnet is not outvoted (issue #140).
    radial_arm = True

    def __init__(self) -> None:
        self.start_point: QVector3D | None = None   # the protractor centre
        self.ref_point: QVector3D | None = None     # base (0°) direction
        self.hover_point: QVector3D | None = None

    def on_key(self, viewport, key, modifiers):
        axis = {Qt.Key_Right: "x", Qt.Key_Left: "y", Qt.Key_Up: "z"}.get(key)
        return self.lock_disc(viewport, axis) if axis else False

    # ---- Plane / colour -----------------------------------------------------
    @property
    def wireframe_color(self):  # type: ignore[override]
        # The disc is coloured by its rotation axis: red/green/blue
        # on an axis plane, dark on an arbitrary face plane.
        n = self._axis()
        for axis, v in _AXES.items():
            if abs(QVector3D.dotProduct(n, v)) > 0.999:
                return _AXIS_RGBA[axis]
        return _OFF_AXIS_RGBA

    def _infer_plane(self, ctx):
        from views.array_bake import reading_only
        with reading_only():
            self.infer_disc(ctx, bool(ctx.modifiers & Qt.ShiftModifier))

    # ---- Disc rendering -----------------------------------------------------
    def _disc_lines(self, sweep=None):
        centre = self.start_point if self.start_point is not None else self.hover_point
        if centre is None:
            return []
        tilt = self._axis_drag_live if self._axis_drag_armed else None
        basis = plane_axes(tilt if tilt is not None else self._axis())
        return self._machine.cursor_lines(centre, [p.toTuple() for p in basis], self._disc_r,
                                          self.hover_point.toTuple() if self.hover_point is not None else None, sweep)


class ProtractorHost(ProtractorBase):
    accepts_angle_ratio = True
    _held_guide = None

    @property
    def cursor_plus(self) -> bool:
        return self._guides

    def status_clause(self) -> str:
        """Kept on screen while the tool is active. Two
        modes here: the Protractor cannot drop a guide POINT."""
        from core.i18n import tr as _tr
        guia, medir = _tr("guide"), _tr("measure")
        return ("Ctrl = " + (f"[{guia}] / {medir}" if self._guides
                             else f"{guia} / [{medir}]"))

    def on_activate(self, viewport):
        self._send(viewport, {"type": "activate"})
        self._axis_pick = self._held_guide = None

    def on_deactivate(self, viewport):
        self._send(viewport, {"type": "deactivate"})
        self._axis_pick = self._held_guide = self.hover_point = None

    def on_cancel(self, viewport):
        self._send(viewport, {"type": "cancel"})
        self._axis_pick = self._held_guide = None
        viewport.update()

    def _send_basis(self, viewport):
        self._send(viewport, {"type": "protractor_frame", "basis": self._basis(),
                              "near": self._snap_ticks, "exact": self._exact_snap})

    def on_click(self, ctx):
        if self.ref_point is not None:
            self._note_snap(ctx)
            self._send_basis(ctx.viewport)
        self._held_guide = None
        self._send(ctx.viewport, self._pointer("click", ctx))
        ctx.viewport.update()

    def on_hover(self, ctx):
        self.hover_point = ctx.world
        self._track_axis_drag(ctx.viewport)
        self._infer_plane(ctx)
        self._update_screen_metrics(ctx)
        self._note_snap(ctx)
        self._send(ctx.viewport, self._pointer("hover", ctx))
        ctx.viewport.update()

    def on_release(self, viewport) -> None:
        # Click-drag from the vertex: the protractor's axis follows the drag
        # (issue #10) — the guide then lies in the tilted plane.
        if self._release_axis_drag(viewport):
            from core.i18n import tr
            viewport.flash_status(tr("Protractor axis set along the drag"))

    def on_value(self, viewport, value):
        if not isinstance(value, (int, float)):
            return False
        self._send_basis(viewport)
        self._machine.set("hover_point", self.hover_point)
        result = self._send(viewport, {"type": "value", "value": value})
        if result["redraw"]:
            viewport.update()
        return result["consumed"]

    def _toggle_guides(self, viewport, say=True):
        self._send(viewport, {"type": "key", "key": "ToggleGuides" if say else "ToggleGuidesQuiet"})

    def on_key(self, viewport, key, modifiers):
        if key == Qt.Key_Control:
            return self._send(viewport, {"type": "key", "key": "Control"})["consumed"]
        return super().on_key(viewport, key, modifiers)

    def on_key_release(self, viewport, key):
        if key != Qt.Key_Control:
            return False
        return self._send(viewport, {"type": "key_release", "key": "Control",
                                     "tapped": getattr(viewport, "ctrl_tapped", lambda: True)()})["consumed"]

    def host_reset_disc(self, viewport, _):
        self._reset_protractor()

    def host_guide_toggle(self, viewport, args):
        from core.i18n import tr
        guides, say = args
        self._during["guides"] = guides
        if say:
            viewport.flash_status(tr("Create guides: on") if guides else tr("Create guides: off — measure only"))
        for name in ("_apply_tool_cursor", "refresh_status_hint"):
            callback = getattr(viewport, name, None)
            if callback:
                callback()
        viewport.update()

    def host_disc_centre(self, viewport, point):
        self._centre_plane = (QVector3D(*point), QVector3D(self._axis()))
        self._axis_drag_armed = True

    def host_remember_guide(self, viewport, _):
        self._held_guide = viewport.scene.guides[-1]

    def host_live_guide_index(self, viewport, _):
        return next((i for i, guide in enumerate(viewport.scene.guides) if guide is self._held_guide), None)

    def host_measured_angle(self, viewport, degrees):
        from core.i18n import tr
        from core.units import fmt_angle_deg
        viewport.flash_status(tr("Angle: {a}").format(a=fmt_angle_deg(degrees, signed=True)), 4000)

    def rubber_band_lines(self):
        return self._disc_lines()

    def guide_preview_lines(self):
        """The angled guide the next click leaves, through the vertex."""
        if (not self._guides or self.start_point is None
                or self.ref_point is None or self.hover_point is None):
            return []
        deg = self._commit_deg(self.hover_point)
        if deg is None:
            return []
        return [Guide(self.start_point, self._direction_at(deg)).segment()]

    def value_label(self):
        if self.ref_point is None or self.hover_point is None:
            return None
        deg = self._display_deg(self.hover_point)
        if deg is None:
            return None
        from core.units import fmt_angle_deg
        return (fmt_angle_deg(deg, signed=True), self.hover_point)

    def vcb_caption(self) -> str:
        return "Angle"

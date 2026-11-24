# SPDX-License-Identifier: GPL-3.0-or-later
"""Both hosts' bindings for the annotation tool gestures."""
from tools.rust_tool_base import RustTool
from ingetrazo_core import Vec3
from tools.rust_tool_base import _point_property, _tuple


from tools.base import AxisMagnet


class _AnnotationTool(AxisMagnet, RustTool):
    work_plane = None
    _ctx = _vp = None

    def _pointer(self, kind, ctx, t=0.0):
        self._ctx, self._vp = ctx, ctx.viewport
        return super()._pointer(kind, ctx, t)

    def host_annotation_pick(self, vp, point):
        from core.annotation_anchor import picked_attachment
        if self._ctx.world == Vec3(*point):
            binding = picked_attachment(self._ctx)
            return binding.archive() if binding is not None else None

    def _send(self, vp, event):
        result = super()._send(vp, event)
        if result["committed"] or event["type"] in ("activate", "cancel", "deactivate"):
            self.work_plane = None
        return result


class TextTool(_AnnotationTool):
    kind = "text"
    anchor = start_point = _point_property("start_point")
    hover_point = _point_property("hover_point")

    def host_text_default(self, vp, _):
        return self._describe(self._ctx)

    def host_text_prompt(self, vp, args):
        from core.i18n import tr
        from core.protocol.commands import run
        from views import chrome_forms
        self.work_plane = None
        def place(text):
            if text and text.strip():
                run(vp.history, "add_text_label", **{**args, "text": text.strip()})
            vp.update()
        chrome_forms.ask_text(vp.window(), tr("Text"), tr("Label text:"), args["text"], done=place, multiline=True)

    def value_label(self):
        if self.anchor is not None and self.hover_point is not None:
            return self._state("description") or "…", self.hover_point

    def _describe(self, ctx) -> str:
        """The default label: what did the anchor click land on?"""
        from core.units import fmt_area, fmt_len
        viewport = ctx.viewport
        sx, sy = ctx.screen.x(), ctx.screen.y()
        try:
            edge = viewport.pick_edge(sx, sy)
        except Exception:
            edge = None
        if edge is not None:
            return fmt_len((Vec3(edge.b) - Vec3(edge.a)).length())
        try:
            face = viewport.pick_face_any(sx, sy)
        except Exception:
            face = None
        if face is not None:
            f = face[0] if isinstance(face, tuple) else face
            # The label quotes the surface as drawn: net of openings
            # when the face knows them (#742).
            net = getattr(f, "net_area", None)
            if callable(net):
                return fmt_area(net())
            area = getattr(f, "area", None)
            if callable(area):
                return fmt_area(f.area())
        p = ctx.world
        return f"({p.x():.2f}, {p.y():.2f}, {p.z():.2f})"


class DimensionTool(_AnnotationTool):
    kind = "dimension"
    a = start_point = _point_property("a")
    b = _point_property("b")
    hover_point = _point_property("hover_point")

    def magnet_on(self):
        return self.b is None

    def owns_shift_lock(self):
        return self.b is not None

    def _shift_held(self):
        from PySide6.QtCore import Qt
        from views import input_state
        return bool(input_state.modifiers(self._vp) & Qt.ShiftModifier)

    def _pointer(self, kind, ctx, t=0.0):
        from PySide6.QtCore import Qt
        from core.snap import project_to_view_plane
        event = super()._pointer(kind, ctx, t)
        camera = getattr(ctx.viewport, "camera", None)
        if self.a is not None and camera and (kind == "hover" or self.b is None):
            event["world"] = project_to_view_plane(ctx.world, self.a, camera.target - camera.eye()).toTuple()
        if self._shift_held():
            event["mods"] |= Qt.ShiftModifier.value
        return event

    def host_dimension_edge(self, vp, _):
        from core.annotation_anchor import picked_edge, Attachment
        snap = getattr(self._ctx, "snap", None)
        edge = getattr(snap, "edge", None)
        if getattr(snap, "kind", None) == "on_edge" and edge is not None and edge.a is not None and edge.b is not None:
            hit, owner = picked_edge(vp.scene, edge)
            ends = [Attachment.capture(vp.scene, hit, owner, p) if hit else None for p in (edge.a, edge.b)]
            return _tuple(edge.a), _tuple(edge.b), [e.archive() if e else None for e in ends]

    _free = property(fset=lambda self, value: self._machine.set("free", (_tuple(value[0]), value[1]) if value else None))

    def _proposed(self, cursor):
        from core.dimension import Dimension
        self._machine.set("propose", (_tuple(cursor), getattr(self._vp, "axis_lock", None), self._shift_held()))
        offset, axis = self._state("proposal")
        return Dimension(self.a, self.b, Vec3(*offset), axis=axis)

    def rubber_band_lines(self):
        if self.a is None or self.hover_point is None:
            return []
        if self.b is None:
            return [(self.a, self.hover_point)]
        p, q = self._proposed(self.hover_point).line_points()
        return [(self.a, p), (self.b, q), (p, q)]

    def value_label(self):
        from core.units import fmt_len
        if self.a is None or self.hover_point is None:
            return None
        if self.b is None:
            return fmt_len((self.a - self.hover_point).length()), (self.a + self.hover_point) * .5
        dim = self._proposed(self.hover_point)
        return fmt_len(dim.value()), dim.midpoint()

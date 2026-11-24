# SPDX-License-Identifier: GPL-3.0-or-later
"""Viewport picking and wireframe bindings for an Eraser stroke."""
from core.input_types import Qt
from core.dimension import Dimension
from core.group import Group
from core.guide import Guide, is_guide
from core.mesh import Edge
from core.textlabel import TextLabel
from georef.geopath import GeoPath


class EraserHost:
    wireframe_color = (0.90, 0.20, 0.15, 1.0)
    _viewport = None

    # A long stroke retains every swept edge until its one-step commit.
    handle_limit = None

    _stroke = property(lambda self: self._state("stroke"),
                       lambda self, value: self._machine.set("stroke", value))
    _hide = property(lambda self: self._state("hide"),
                     lambda self, hide: self._machine.set("hide", bool(hide)))

    def _mark(self, vp, x, y):
        self._send(vp, {"type": "click", "px": (x, y), "mods": Qt.ShiftModifier.value if self._hide else 0})

    marked = property(lambda self: {self._handles.get(i) for i in self._state("marked")},
                      lambda self, items: self._machine.set("marked", [self._handles.id(e) for e in items]))

    def _send(self, vp, event):
        self._viewport = vp
        reply = super()._send(vp, event)
        if event["type"] in ("activate", "deactivate", "cancel", "release") or (event["type"] == "hover" and not self._stroke):
            self._handles.clear()
        return reply

    def on_release(self, vp):
        self._send(vp, {"type": "release"})
        vp.update()

    def rubber_band_lines(self):
        """The marked stroke, drawn in the tool's red wireframe colour."""
        segs = []
        for m in self.marked:
            if isinstance(m, Edge):
                segs.append((m.a, m.b))
            elif isinstance(m, Group):
                segs.extend(self._group_box_segments(m))
            elif isinstance(m, Guide):
                segs.append(m.segment())
            elif getattr(m, "is_circle", False):
                pts = m.sample_points(24)
                segs.extend((pts[i], pts[(i + 1) % len(pts)])
                            for i in range(len(pts)))
            elif isinstance(m, Dimension):
                segs.append(m.line_points())
            elif isinstance(m, GeoPath):
                segs.extend(m.segments())
            elif isinstance(m, TextLabel):
                segs.append((m.anchor, m.position()))
        return segs

    # ---- Internals ----------------------------------------------------------
    def _group_box_segments(self, group) -> list:
        """The twelve edges of the group's oriented box — its stroke mark,
        the same box Select draws around it."""
        vp = getattr(self, "_viewport", None)
        obb = getattr(vp, "_group_obb", None)
        if obb is None:
            return []
        from core.group import oriented_box_corners
        try:
            c = oriented_box_corners(*obb(group))
        except Exception:  # noqa: BLE001 — a box that cannot be built
            return []
        if len(c) != 8:
            return []
        pairs = ((0, 1), (1, 3), (3, 2), (2, 0), (4, 5), (5, 7), (7, 6),
                 (6, 4), (0, 4), (1, 5), (2, 6), (3, 7))
        return [(c[i], c[j]) for i, j in pairs]

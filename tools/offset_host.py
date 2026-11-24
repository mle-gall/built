# SPDX-License-Identifier: GPL-3.0-or-later
"""Offset viewport bindings. The machine owns the source geometry and gesture."""
from tools.overlay_painter import draw_overlay
from ingetrazo_core import Vec3 as QVector3D, ordered_edge_path as _run_from_edges, outline_normal as _newell

from core.i18n import tr
from core.mesh import Edge
from core.units import fmt_len
from tools.rust_tool_base import _point_property


def _field(key):
    return property(lambda self: self._state(key), lambda self, v: self._machine.set(key, v))


class OffsetHost:
    _READ_DURING = ("dragging", "distance", "ref_point", "ref_inward", "reference")
    hovered_face = hovered_edge = None
    _base_face = None
    dragging = _field("dragging")
    distance = _field("distance")
    _armed = _field("armed")
    _closed = _field("closed")
    _ref_point = _point_property("ref_point")
    _ref_inward = _point_property("ref_inward")
    allow_inferences = False
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = True
    allowed_snap_kinds = frozenset((
        "endpoint", "midpoint", "arc_midpoint", "center", "origin",
        "component_origin", "intersection", "intersection_edge",
        "intersection_face",
    ))

    @property
    def uses_snap(self) -> bool:
        """Only during the drag, whose distance snaps to named model points.
        Before it the click picks a face or a run of edges, not a point."""
        return self.dragging


    @property
    def _chain(self):
        return self._state("chain")

    @_chain.setter
    def _chain(self, chain):
        if chain:
            self.base_face = None

    @property
    def base_face(self):
        return self._base_face

    @base_face.setter
    def base_face(self, face):
        self._base_face = face
        self._machine.set("face", str(face.key) if face is not None else None)

    @property
    def _loop(self):
        return [QVector3D(*p) for p in self._machine.query("points", None)]

    @_loop.setter
    def _loop(self, points):
        self._machine.set("points", [p.toTuple() for p in points])

    @property
    def _normal(self):
        return QVector3D(*self._machine.query("normal", None))

    @_normal.setter
    def _normal(self, n):
        self._machine.set("normal", n.toTuple())

    @property
    def _offset_reference(self):
        points = self._state("reference")
        return None if points is None else tuple(QVector3D(*p) for p in points)

    def _pointer(self, kind, ctx, t=0.0):
        event = super()._pointer(kind, ctx, t)
        if event["snap_kind"] in self.allowed_snap_kinds:
            event["world"] = ctx.snap.point.toTuple()
        else:
            event["snap_kind"] = None
        return event

    def on_value(self, viewport, value):
        if not isinstance(value, (int, float)):
            return False
        answer = self._send(viewport, {"type": "value", "value": value})
        if answer["redraw"]:
            viewport.update()
        return answer["consumed"]

    def _commit(self, viewport):
        self._send(viewport, {"type": "key", "key": "CommitOffset"})
        viewport.update()

    def value_label(self):
        label = self._machine.query("label", None) if self._during is None else None
        if label is not None:
            return fmt_len(label[0]), QVector3D(*label[1])

    def host_clear_offset_picks(self, viewport, _):
        self.hovered_face = self.hovered_edge = self._base_face = None

    def host_clear_offset_hover(self, viewport, _):
        viewport.set_hover(None)

    def host_hover_outline(self, viewport, px):
        self.hovered_face = viewport.pick_face(*px)
        self.hovered_edge = None
        if self.hovered_face is None:
            pick = getattr(viewport, "pick_edge", None)
            self.hovered_edge = pick(*px) if pick else None
        viewport.set_hover(self.hovered_face or self.hovered_edge)
        return self._picked(viewport, False)

    def host_picked_outline(self, viewport, _):
        self._base_face = self.hovered_face
        return self._picked(viewport, True)

    def _picked(self, viewport, report):
        f = self.hovered_face
        if f is not None:
            if len(f.vertices) >= 3:
                return dict(points=f.vertices, normal=f.normal(), origin=f.centroid(),
                            face=str(f.key), closed=True)
        elif self.hovered_edge is not None:
            return self._run_payload(viewport, _run_from_edge(self.hovered_edge), report)

    def host_selected_outline(self, viewport, _):
        selection = list(getattr(getattr(viewport, "scene", None), "selection", ()))
        if len(selection) < 2 or not all(isinstance(e, Edge) for e in selection):
            return None
        run = _run_from_edges(selection)
        if run is not None:
            source = self._run_payload(viewport, run, True)
            if source:
                viewport.flash_status(tr("{n} edges selected — click to start the offset",
                                         n=len(selection)), 4000)
            return source

    def _run_payload(self, viewport, run, report):
        if run is not None:
            points, closed = run
            normal = _newell(points)
            if normal is not None:
                return dict(points=points, closed=closed, normal=normal)
        if report:
            if run is None:
                message = "Offset needs one connected run of edges — a polyline, or a closed loop."
            elif len(points) == 2 and not closed:
                message = ("This line is on its own — nothing is joined to its ends, so there is no "
                           "chain to offset. Draw the missing line, or select the edges to offset.")
                if self.hovered_edge is not None:
                    viewport.scene.select([self.hovered_edge])
                viewport.flash_status(tr(message), 6000)
                return None
            else:
                message = ("Offset needs at least two edges that are not in line — a straight "
                           "run has no plane to offset in.")
            viewport.flash_status(tr(message), 5000)

    def host_offset_refusal(self, viewport, args):
        from core.topology import max_offset_distance
        error, distance, points, normal = args
        d = fmt_len(abs(distance))
        if error == "hole":
            message = tr("{d} runs into a hole of this face — try a smaller offset", d=d)
        elif error == "collapsed_chain":
            message = tr("{d} collapses this run — try a smaller offset", d=d)
        elif error == "collapsed_face":
            sign = -1 if distance < 0 else 1
            room = max_offset_distance([QVector3D(*p) for p in points], QVector3D(*normal), sign)
            side = tr("inward") if sign > 0 else tr("outward")
            if room <= 1e-4:
                message = tr("{d} closes this face — it takes no offset {side}", d=d, side=side)
            else:
                message = tr("{d} closes this face — {side} it takes at most {max}",
                             d=d, side=side, max=fmt_len(room))
        else:
            message = error
        if message:
            viewport.flash_status(message, 5000)

    overlay_kind = "offset"
    draw_overlay = draw_overlay


def _run_from_edge(edge: Edge):
    """The run ``edge`` belongs to, walked both ways through vertices
    where exactly two edges meet (a corner of a polyline); a junction or a
    free end stops the walk. ``(points, closed)``."""
    mesh = getattr(edge, "_m", None)
    store = mesh.__dict__.get("_store") if mesh is not None else None
    if store is not None:
        return store.edge_run(edge.key)

    def walk(v, e):
        out = []
        while len(v.edges) == 2:
            e = next(x for x in v.edges if x != e)
            if e == edge:
                return out, True              # came round: a closed loop
            v = e.other(v)
            out.append(QVector3D(v.position))
        return out, False
    fwd, closed = walk(edge.v1, edge)
    if closed:
        return [QVector3D(edge.v0.position), QVector3D(edge.v1.position)] + fwd[:-1], True
    back, _ = walk(edge.v0, edge)
    return list(reversed(back)) + [QVector3D(edge.v0.position),
                                   QVector3D(edge.v1.position)] + fwd, False

# SPDX-License-Identifier: GPL-3.0-or-later
"""Tool parity scenarios (docs/rust-tools.md §11).

Each scenario drives a tool through its public API (``on_click``,
``on_hover``, ``on_value``, ``on_undo``, ``on_cancel``, the box and the
double click) on a synthetic document, and records after every step what
the user would see: the geometry, the undo depth, the selection and the
state the inference reads. The fixtures in ``tests/fixtures/tool_parity``
were recorded from the Python tools before they moved to Rust
(``python -m tests.tool_parity record``); ``test_tool_parity.py`` replays
them against whatever implementation ``tools`` exports now.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

FIXTURES = Path(__file__).with_name("fixtures") / "tool_parity"


def _r(x):
    return round(float(x), 9) + 0.0


def _pt(p):
    if p is None:
        return None
    t = p.toTuple() if hasattr(p, "toTuple") else tuple(p)
    return [_r(v) for v in t]


class _Snap:
    def __init__(self, kind):
        self.kind = kind


# ---- Line --------------------------------------------------------------

class LineView:
    """The viewport a Line commit needs: scene, history, a camera eye."""

    def __init__(self, far=False):
        from core.history import History
        from core.scene import Scene
        self.scene = Scene()
        self.history = History(self.scene)
        self.offset = (650000.0, 6860000.0, 0.0) if far else (0.0, 0.0, 0.0)

    def update(self):
        pass

    def flash_status(self, *a, **k):
        pass


def _line_snapshot(view, tool):
    mesh = view.scene.mesh
    edges = sorted(sorted((_pt(e.a), _pt(e.b))) for e in mesh.edges)
    return {"edges": edges, "faces": len(mesh.faces),
            "undo": len(view.history.undo_stack),
            "start": _pt(tool.start_point),
            "first": _pt(tool.chain_first_point),
            "chain": len(tool.chain_vertices),
            "lines": [[_pt(a), _pt(b)] for a, b in tool.rubber_band_lines()]}


LINE_SCENARIOS = {
    "triangle_closes_on_its_first_point": [
        ("click", (0, 0, 0)), ("hover", (2, 0, 0)), ("click", (2, 0, 0)),
        ("click", (2, 2, 0)), ("click", (0, 0, 0), "close")],
    "square_closes_by_making_a_face": [
        ("click", (0, 0, 0)), ("click", (1, 0, 0)), ("click", (1, 1, 0)),
        ("click", (0, 1, 0)), ("click", (0, 0, 0)), ("click", (5, 5, 0))],
    "typed_length_along_the_band_and_negative": [
        ("click", (0, 0, 0)), ("hover", (0, 3, 0)), ("value", 2.0),
        ("hover", (1, 2, 0)), ("value", -1.5), ("value", 0.0)],
    "typed_delta_and_pair_refused": [
        ("click", (1, 1, 0)), ("value", (3.0, 4.0, 5.0)), ("value", (2.0, 1.0)),
        ("value", (0.0, 0.0, 1.0))],
    "retype_twice_keeps_one_step": [
        ("click", (0, 0, 0)), ("hover", (1, 0, 0)), ("click", (1, 0, 0)),
        ("value", 4.0), ("value", 2.5), ("hover", (2.5, 1, 0)), ("value", 1.0)],
    "retype_closes_window_after_moving_away": [
        ("click", (0, 0, 0)), ("hover", (1, 0, 0)), ("click", (1, 0, 0)),
        ("hover", (1, 2, 0)), ("value", 3.0)],
    "retype_the_closing_segment": [
        ("click", (0, 0, 0)), ("click", (2, 0, 0)), ("click", (2, 2, 0)),
        ("click", (0, 0, 0), "close"), ("value", (0.0, -2.0, 0.0))],
    "undo_mid_chain_steps_back": [
        ("click", (0, 0, 0)), ("click", (2, 0, 0)), ("click", (2, 3, 0)),
        ("undo",), ("hover", (2, -1, 0)), ("value", (0.0, -1.0, 0.0)),
        ("undo",), ("undo",), ("undo",), ("undo",)],
    "click_on_the_start_draws_nothing": [
        ("click", (0, 0, 0)), ("click", (0, 0, 0)), ("click", (1, 0, 0))],
    "cancel_mid_chain": [
        ("click", (0, 0, 0)), ("click", (1, 0, 0)), ("cancel",),
        ("click", (3, 3, 0)), ("click", (3, 4, 0))],
    "cutting_a_face_ends_the_chain": [
        ("face", [(0, 0, 0), (4, 0, 0), (4, 4, 0), (0, 4, 0)]),
        ("click", (2, 0, 0)), ("click", (2, 4, 0)), ("click", (3, 4, 0))],
    "far_from_the_origin": [
        ("click", (0.01, 0.02, 0)), ("hover", (0.03, 0.02, 0)), ("value", 0.02),
        ("hover", (0.03, 0.05, 0)), ("value", 0.02), ("value", (-0.02, 0.0, 0.0)),
        ("click", (0.01, 0.02, 0), "close")],
}


def run_line(steps, far=False):
    from ingetrazo_core import Vec3
    from ingetrazo_core import ScreenPoint, Input
    from tools.base import ToolContext
    from tools.rust_tools import LineTool
    view = LineView(far=far)
    off = view.offset
    tool = LineTool()
    tool.on_activate(view)

    def at(p):
        return Vec3(*(float(v) + o for v, o in zip(p, off)))

    out = []
    for step in steps:
        kind = step[0]
        result = None
        if kind == "face":
            view.scene.mesh.add_face([tuple(float(v) + o for v, o in zip(p, off)) for p in step[1]])
        elif kind in ("click", "hover"):
            snap = _Snap(step[2] if len(step) > 2 else "on_face")
            ctx = ToolContext(view, at(step[1]), ScreenPoint(0, 0), Input.NoModifier, snap)
            (tool.on_click if kind == "click" else tool.on_hover)(ctx)
        elif kind == "value":
            v = step[1]
            result = bool(tool.on_value(view, tuple(map(float, v)) if isinstance(v, (list, tuple)) else float(v)))
        elif kind == "undo":
            result = bool(tool.on_undo(view))
        elif kind == "cancel":
            tool.on_cancel(view)
        snap = _line_snapshot(view, tool)
        if far:
            snap = _unshift(snap, off)
        snap["result"] = result
        out.append(snap)
    return out


def _unshift(snap, off):
    def un(p):
        return None if p is None else [_r(v - o) for v, o in zip(p, off)]
    snap = dict(snap)
    snap["edges"] = sorted(sorted((un(a), un(b))) for a, b in snap["edges"])
    for k in ("start", "first"):
        if k in snap:
            snap[k] = un(snap[k])
    snap["lines"] = [[un(a), un(b)] for a, b in snap["lines"]]
    return snap


def line_cases():
    for name, steps in LINE_SCENARIOS.items():
        yield name, steps, name == "far_from_the_origin"


# ---- Select ------------------------------------------------------------

class SelectView:
    """A view whose picks are scripted: ``hits[(x, y)]`` names what lies
    under a pixel, ``box`` what a box would catch. Records the edit-context
    calls; the selection is the scene's own."""

    def __init__(self):
        from core.group import Group
        from core.history import History
        from core.mesh import Mesh
        from core.scene import Scene
        self.scene = Scene()
        self.history = History(self.scene)
        mesh = self.scene.mesh
        self.face = mesh.add_face([(0., 0., 0.), (1., 0., 0.), (1., 1., 0.), (0., 1., 0.)])
        self.edge = mesh.find_edge(self.face.loop[0], self.face.loop[1])
        self.group = Group(Mesh())
        self.group.uid = "g1"
        self.group.name = "Post"
        self.scene.groups.append(self.group)
        self.hits = {}
        self.calls = []
        self.edit_box = False
        self.extension_pick = None

    def names(self, entities):
        out = []
        for e in entities:
            if e is self.face:
                out.append("face")
            elif e is self.group:
                out.append("group")
            elif e is self.edge:
                out.append("edge")
            else:
                out.append(type(e).__name__)
        return sorted(out)

    def _hit(self, x, y, kind):
        near = [h for (hx, hy), h in self.hits.items() if abs(hx - x) <= 3 and abs(hy - y) <= 3]
        hit = near[0] if near else {}
        return {"face": self.face, "group": self.group, "edge": self.edge}.get(hit.get(kind))

    def pick_group(self, x, y):
        return self._hit(x, y, "group")

    def pick_edge(self, x, y):
        return self._hit(x, y, "edge")

    def pick_face(self, x, y):
        return self._hit(x, y, "face")

    def pick_dimension(self, x, y):
        return None

    def pick_geopath(self, x, y):
        return None

    def begin_group_edit(self, group):
        self.calls.append("begin_group_edit")

    def end_one_group_edit(self):
        self.calls.append("end_one_group_edit")

    def edit_group_box_contains_pixel(self, x, y):
        return self.edit_box

    def set_hover(self, entity, placement=None):
        self.calls.append("hover:" + ",".join(self.names([entity]) if entity is not None else []))

    def update(self):
        pass

    def flash_status(self, text, *a, **k):
        self.calls.append("flash:" + text)


SELECT_SCENARIOS = {
    "click_replace_and_modifiers": [
        ("hits", 10, 10, {"face": "face"}), ("hits", 20, 20, {"group": "group"}),
        ("click", 10, 10, "none"), ("click", 20, 20, "ctrl"), ("click", 10, 10, "shift"),
        ("click", 20, 20, "shift+ctrl"), ("click", 50, 50, "ctrl"), ("click", 50, 50, "none")],
    "edge_over_group_takes_the_edge": [
        ("hits", 10, 10, {"edge": "edge", "face": "face"}), ("click", 10, 10, "none")],
    "repeated_click_cycles": [
        ("hits", 10, 10, {"group": "group", "face": "face"}),
        ("click", 10, 10, "none"), ("click", 11, 10, "none"), ("click", 10, 11, "none"),
        ("hover", 40, 40), ("click", 10, 10, "none")],
    "double_click_face_takes_its_edges_then_group_opens": [
        ("hits", 10, 10, {"face": "face"}), ("hits", 20, 20, {"group": "group"}),
        ("double", 10, 10, "none"), ("double", 20, 20, "none")],
    "triple_click_takes_the_connected": [
        ("hits", 10, 10, {"edge": "edge"}), ("triple", 10, 10, "none")],
    "empty_click_in_an_edit_context": [
        ("editing", True), ("click", 70, 70, "none"),
        ("hits", 10, 10, {"face": "face"}), ("click", 10, 10, "none"),
        ("click", 70, 70, "none"), ("edit_box", True), ("click", 70, 70, "none")],
    "hover_marks_edges_and_groups_never_faces": [
        ("hits", 10, 10, {"face": "face"}), ("hits", 20, 20, {"group": "group"}),
        ("hover", 10, 10), ("hover", 20, 20), ("hover", 60, 60)],
}

_MODS = {"none": 0, "shift": 0x02000000, "ctrl": 0x04000000, "shift+ctrl": 0x06000000}


def run_select(steps):
    from ingetrazo_core import Vec3
    from ingetrazo_core import ScreenPoint, Input
    from tools.base import ToolContext
    from tools.rust_tools import SelectTool
    view = SelectView()
    tool = SelectTool()
    tool.on_activate(view)
    out = []
    for step in steps:
        kind = step[0]
        if kind == "hits":
            view.hits[(step[1], step[2])] = step[3]
            continue
        if kind == "editing":
            # Any non-None edit group: the tool only asks whether one is open.
            view.scene.edit_group = view.group if step[1] else None
            continue
        if kind == "edit_box":
            view.edit_box = step[1]
            continue
        view.calls.clear()
        mods = Input.KeyboardModifier(_MODS[step[3]]) if len(step) > 3 else Input.NoModifier
        ctx = ToolContext(view, Vec3(), ScreenPoint(step[1], step[2]), mods, None)
        {"click": tool.on_click, "double": tool.on_double_click,
         "triple": tool.on_triple_click, "hover": tool.on_hover}[kind](ctx)
        out.append({"selection": view.names(view.scene.selection), "calls": list(view.calls)})
    return out


# ---- Rectangle ------------------------------------------------------------

class RectView(LineView):
    """A Line view that also keeps what the tool flashed."""

    def __init__(self, far=False):
        super().__init__(far)
        self.flashed = []

    def flash_status(self, text, *a, **k):
        self.flashed.append(text)


def _rect_snapshot(view, tool):
    snap = _line_snapshot_base(view, tool)
    label = tool.value_label()
    snap.update(label=None if label is None else [label[0], _pt(label[1])],
                plane_lock=tool.plane_lock,
                work_plane=None if tool.work_plane is None else [_pt(p) for p in tool.work_plane],
                flashed=list(view.flashed))
    view.flashed.clear()
    return snap


def _line_snapshot_base(view, tool):
    mesh = view.scene.mesh
    edges = sorted(sorted((_pt(e.a), _pt(e.b))) for e in mesh.edges)
    return {"edges": edges, "faces": len(mesh.faces), "undo": len(view.history.undo_stack),
            "start": _pt(tool.start_point),
            "lines": [[_pt(a), _pt(b)] for a, b in tool.rubber_band_lines()]}


RECT_SCENARIOS = {
    "two_corners": [("click", (0, 0, 0)), ("hover", (2, 1, 0)), ("click", (2, 1, 0))],
    "square_cue_snaps_the_far_corner": [
        ("click", (0, 0, 0)), ("hover", (2, 2.05, 0)), ("click", (2, 2.05, 0))],
    "shift_square_lock": [
        ("click", (0, 0, 0)), ("hover", (2, 2.02, 0), "shift"), ("hover", (3, 1, 0), "shift"),
        ("click", (3, 1, 0), "shift")],
    "down_arrow_square_lock": [
        ("click", (0, 0, 0)), ("hover", (2, 2.03, 0)), ("key", "down"), ("hover", (3, 1, 0)),
        ("click", (3, 1, 0))],
    "from_the_centre_with_ctrl": [
        ("key", "ctrl"), ("key_release", "ctrl", True), ("click", (1, 1, 0)), ("hover", (2, 1.5, 0)),
        ("click", (2, 1.5, 0))],
    "ctrl_in_a_chord_takes_itself_back": [
        ("key", "ctrl"), ("key_release", "ctrl", False), ("click", (0, 0, 0)), ("hover", (1, 2, 0)),
        ("click", (1, 2, 0))],
    "typed_width_and_height": [
        ("click", (0, 0, 0)), ("hover", (-1, 1, 0)), ("value", (3.0, 2.0))],
    "a_lone_value_sets_the_side_being_stretched": [
        ("click", (0, 0, 0)), ("hover", (1, 0.5, 0)), ("hover", (2, 0.5, 0)), ("value", 4.0),
        ("click", (5, 5, 0)), ("hover", (5.2, 6, 0)), ("hover", (5.2, 7, 0)), ("value", 3.0)],
    "retype_after_drawing": [
        ("click", (0, 0, 0)), ("hover", (2, 1, 0)), ("click", (2, 1, 0)), ("value", (4.0, 3.0)),
        ("value", 5.0), ("value", (1.0, 1.0))],
    "a_flat_rectangle_draws_nothing": [
        ("click", (0, 0, 0)), ("hover", (2, 0, 0)), ("click", (2, 0, 0))],
    "plane_lock_after_the_first_corner": [
        ("click", (0, 0, 0)), ("key", "right"), ("hover", (0, 2, 1)), ("click", (0, 2, 1))],
    "cancel_mid_rectangle": [
        ("click", (0, 0, 0)), ("hover", (1, 1.5, 0)), ("cancel",), ("hover", (2, 2, 0)),
        ("click", (3, 3, 0)), ("hover", (4, 5, 0)), ("click", (4, 5, 0))],
    "typed_far_from_the_origin": [
        ("click", (0.01, 0.02, 0)), ("hover", (0.03, 0.05, 0)), ("value", (0.022345678912, 0.038912345678))],
}

_KEYS = {"down": "Key_Down", "right": "Key_Right", "ctrl": "Key_Control"}


def run_rect(steps, far=False):
    from ingetrazo_core import Vec3
    from ingetrazo_core import ScreenPoint, Input
    from tools.base import ToolContext
    from tools.rust_tools import RectangleTool
    view = RectView(far=far)
    off = view.offset
    tool = RectangleTool()
    tool.on_activate(view)

    def at(p):
        return Vec3(*(float(v) + o for v, o in zip(p, off)))

    out = []
    for step in steps:
        kind = step[0]
        result = None
        if kind in ("click", "hover"):
            mods = Input.ShiftModifier if len(step) > 2 and step[2] == "shift" else Input.NoModifier
            ctx = ToolContext(view, at(step[1]), ScreenPoint(0, 0), mods, _Snap("on_face"))
            (tool.on_click if kind == "click" else tool.on_hover)(ctx)
        elif kind == "value":
            v = step[1]
            result = bool(tool.on_value(view, tuple(map(float, v)) if isinstance(v, (list, tuple)) else float(v)))
        elif kind == "key":
            result = bool(tool.on_key(view, getattr(Input, _KEYS[step[1]]), Input.NoModifier))
        elif kind == "key_release":
            tapped = step[2]
            view.ctrl_tapped = lambda: tapped
            result = bool(tool.on_key_release(view, getattr(Input, _KEYS[step[1]])))
        elif kind == "cancel":
            tool.on_cancel(view)
        snap = _rect_snapshot(view, tool)
        if far:
            snap = _unshift(snap, off)
            if snap["label"] is not None:
                snap["label"][1] = [_r(v - o) for v, o in zip(snap["label"][1], off)]
            snap["work_plane"] = None if snap["work_plane"] is None else [
                [_r(v - o) for v, o in zip(snap["work_plane"][0], off)], snap["work_plane"][1]]
        snap["result"] = result
        out.append(snap)
    return out


def rect_cases():
    for name, steps in RECT_SCENARIOS.items():
        yield name, steps, name == "typed_far_from_the_origin"


# ---- Circle and Polygon ---------------------------------------------------

def _circle_snapshot(view, tool):
    snap = _line_snapshot_base(view, tool)
    label = tool.value_label()
    snap.update(label=None if label is None else [label[0], _pt(label[1])],
                plane_lock=tool.plane_lock,
                work_plane=None if tool.work_plane is None else [_pt(p) for p in tool.work_plane],
                flashed=list(view.flashed), sides=int(tool.sides),
                caption=tool.vcb_caption(), unitless=bool(tool.value_is_unitless()),
                guide=bool(tool._guide_mode),
                guide_lines=[[_pt(a), _pt(b)] for a, b in tool.guide_preview_lines()],
                guides=[[_pt(g.center), _pt(g.normal), _r(g.radius)] for g in view.scene.guides])
    view.flashed.clear()
    return snap


CIRCLE_SCENARIOS = {
    "centre_and_rim": [("click", (0, 0, 0)), ("hover", (2, 1, 0)), ("click", (2, 1, 0))],
    "typed_radius_keeps_the_direction": [
        ("click", (1, 1, 0)), ("hover", (1, 3, 0)), ("value", 0.5)],
    "sides_before_the_centre": [
        ("value", 8.0), ("value", 2.0), ("click", (0, 0, 0)), ("hover", (1, 0, 0)), ("click", (1, 0, 0))],
    "segments_redraw_the_last": [
        ("click", (0, 0, 0)), ("hover", (1, 0, 0)), ("click", (1, 0, 0)), ("segments", 12),
        ("segments", 2), ("value", 2.0), ("segments", 5)],
    "retype_closes_on_undo": [
        ("click", (0, 0, 0)), ("hover", (1, 0, 0)), ("click", (1, 0, 0)), ("undo",), ("value", 3.0)],
    "guide_circle_with_ctrl": [
        ("key", "ctrl"), ("key_release", "ctrl", True), ("click", (0, 0, 0)), ("hover", (0, 2, 0)),
        ("click", (0, 2, 0)), ("click", (1, 1, 0)), ("hover", (1, 2, 0)), ("value", 0.25)],
    "ctrl_in_a_chord_takes_itself_back": [
        ("key", "ctrl"), ("key_release", "ctrl", False), ("click", (0, 0, 0)), ("hover", (1, 1, 0)),
        ("click", (1, 1, 0))],
    "a_rim_on_the_centre_draws_nothing": [
        ("click", (0, 0, 0)), ("hover", (0, 0, 0)), ("click", (0, 0, 0)), ("value", -1.0)],
    "plane_lock_arrow": [
        ("key", "right"), ("click", (0, 0, 0)), ("hover", (0, 1, 1)), ("click", (0, 1, 1))],
    "cancel_mid_circle": [
        ("click", (0, 0, 0)), ("hover", (1, 1, 0)), ("cancel",), ("click", (3, 3, 0)),
        ("hover", (4, 3, 0)), ("click", (4, 3, 0)), ("cancel",), ("value", 2.0)],
    "two_circles_overlap": [
        ("click", (0, 0, 0)), ("hover", (1, 0, 0)), ("click", (1, 0, 0)),
        ("click", (1, 0, 0)), ("hover", (2, 0, 0)), ("click", (2, 0, 0))],
    "tuple_values_are_refused": [("click", (0, 0, 0)), ("hover", (1, 0, 0)), ("value", (1.0, 2.0))],
    "typed_far_from_the_origin": [
        ("click", (0.01, 0.02, 0)), ("hover", (0.03, 0.05, 0)), ("value", 0.022345678912)],
}


def run_circle(steps, polygon=False, far=False):
    from ingetrazo_core import Vec3
    from ingetrazo_core import ScreenPoint, Input
    from tools.base import ToolContext
    from tools.circle import CircleTool, PolygonTool
    view = RectView(far=far)
    off = view.offset
    tool = (PolygonTool if polygon else CircleTool)()
    tool.on_activate(view)

    def at(p):
        return Vec3(*(float(v) + o for v, o in zip(p, off)))

    out = []
    for step in steps:
        kind = step[0]
        result = None
        if kind in ("click", "hover"):
            ctx = ToolContext(view, at(step[1]), ScreenPoint(0, 0), Input.NoModifier, _Snap("on_face"))
            (tool.on_click if kind == "click" else tool.on_hover)(ctx)
        elif kind == "value":
            v = step[1]
            result = bool(tool.on_value(view, tuple(map(float, v)) if isinstance(v, (list, tuple)) else float(v)))
        elif kind == "segments":
            result = bool(tool.on_segments_value(view, step[1]))
        elif kind == "key":
            result = bool(tool.on_key(view, getattr(Input, _KEYS[step[1]]), Input.NoModifier))
        elif kind == "key_release":
            tapped = step[2]
            view.ctrl_tapped = lambda: tapped
            result = bool(tool.on_key_release(view, getattr(Input, _KEYS[step[1]])))
        elif kind == "cancel":
            tool.on_cancel(view)
        elif kind == "undo":
            view.history.undo()
        snap = _circle_snapshot(view, tool)
        if far:
            snap = _unshift(snap, off)
            if snap["label"] is not None:
                snap["label"][1] = [_r(v - o) for v, o in zip(snap["label"][1], off)]
            snap["work_plane"] = None if snap["work_plane"] is None else [
                [_r(v - o) for v, o in zip(snap["work_plane"][0], off)], snap["work_plane"][1]]
        snap["result"] = result
        out.append(snap)
    return out


def circle_cases():
    for name, steps in CIRCLE_SCENARIOS.items():
        for polygon in (False, True):
            yield f"{'polygon' if polygon else 'circle'}:{name}", steps, polygon, name == "typed_far_from_the_origin"


# ---- Rotated Rectangle ----------------------------------------------------

class RotView(RectView):
    """A Rect view with an oblique parallel camera (pixels, rays), the
    axis lock and a wall face the first click may land on."""

    def __init__(self):
        super().__init__()
        from core.protocol.commands import run
        run(self.history, "add_face", outer=[[5, 0, 0], [5, 2, 0], [5, 2, 2], [5, 0, 2]])
        self.history.undo_stack.clear()
        self.axis_lock = None
        self.on_wall = False

    def _world_to_pixel(self, p):
        return (400.0 + 100.0 * (p.x() + 0.3 * p.z()), 300.0 - 100.0 * (p.y() + 0.2 * p.z()))

    def _pixel_to_ray(self, x, y):
        from ingetrazo_core import Vec3
        wx, wy = (x - 400.0) / 100.0, (300.0 - y) / 100.0
        return Vec3(wx - 3.0, wy - 2.0, 10.0), Vec3(0.3, 0.2, -1.0).normalized()

    def pick_face_any(self, x, y):
        if not self.on_wall:
            return None, None
        return self.scene.mesh.faces[0], None


def _rot_snapshot(view, tool):
    snap = _line_snapshot_base(view, tool)
    label = tool.value_label()
    snap.update(label=None if label is None else [label[0], _pt(label[1])],
                base=_pt(tool.base_point), angle=_r(tool.angle),
                work_plane=None if tool.work_plane is None else [_pt(p) for p in tool.work_plane],
                flashed=list(view.flashed), vcb=tool.vcb_label, magnet=bool(tool.magnet_on()),
                diagonal=bool(tool.diagonal_span), plane_lock=tool.plane_lock)
    view.flashed.clear()
    return snap


ROT_SCENARIOS = {
    "three_clicks_flat": [
        ("click", (0, 0, 0)), ("hover", (2, 0.3, 0)), ("click", (2, 0.3, 0)), ("hover", (2.2, 1.5, 0)),
        ("click", (2.2, 1.5, 0))],
    "first_protractor_ticks_near_the_rim": [
        ("click", (0, 0, 0)), ("hover", (0.5, 0.1, 0)), ("click", (0.5, 0.1, 0)), ("hover", (0.6, 0.8, 0)),
        ("click", (0.6, 0.8, 0))],
    "shift_holds_the_edge_direction": [
        ("click", (0, 0, 0)), ("hover", (3, 1, 0), "shift"), ("hover", (1, 2, 0), "shift"),
        ("click", (1, 2, 0), "shift"), ("hover", (0, 3, 0)), ("click", (0, 3, 0))],
    "typed_edge_then_width": [
        ("click", (0, 0, 0)), ("hover", (1, 1, 0)), ("value", 3.0), ("hover", (1, 3, 0)), ("value", 2.0)],
    "typed_edge_with_angle_then_standing_width": [
        ("click", (1, 1, 0)), ("hover", (2, 1, 0)), ("value", (2.0, 30.0)), ("hover", (2, 3, 0)),
        ("value", (1.5, 90.0))],
    "negative_and_zero_values": [
        ("click", (0, 0, 0)), ("hover", (1, 0, 0)), ("value", 0.0), ("value", -2.0), ("hover", (-1, 1, 0)),
        ("value", 0.0), ("value", (-1.0, 0.0)), ("value", (1.0, 2.0, 3.0)), ("value", -1.0)],
    "the_width_reads_the_protractor_plane": [
        ("click", (0, 0, 0)), ("hover", (3, 0, 0)), ("click", (3, 0, 0)), ("hover", (3.4, 1.2, 0)),
        ("click", (3.4, 1.2, 0))],
    "a_point_snap_is_taken_as_is": [
        ("click", (0, 0, 0)), ("hover", (3, 0, 0)), ("click", (3, 0, 0)),
        ("hover", (3, 1, 1), "endpoint"), ("click", (3, 1, 1), "endpoint")],
    "axis_lock_sets_the_width": [
        ("click", (0, 0, 0)), ("hover", (2, 0, 0)), ("click", (2, 0, 0)), ("axis", "z"),
        ("hover", (2.5, 0.7, 0)), ("click", (2.5, 0.7, 0)), ("axis", None)],
    "shift_holds_the_width_angle": [
        ("click", (0, 0, 0)), ("hover", (2, 0, 0)), ("click", (2, 0, 0)), ("hover", (2, 1, 0.2)),
        ("hover", (2, 1, 0.2), "shift"), ("hover", (2.5, 0.3, 1.5), "shift"), ("click", (2.5, 0.3, 1.5), "shift")],
    "zero_width_says_why": [
        ("click", (0, 0, 0)), ("hover", (2, 0, 0)), ("click", (2, 0, 0)), ("hover", (1, 0, 0), "endpoint"),
        ("click", (1, 0, 0), "endpoint")],
    "the_wall_under_the_first_click": [
        ("wall", True), ("click", (5, 0.5, 0.5)), ("wall", False), ("hover", (5, 1.5, 0.5), "endpoint"),
        ("click", (5, 1.5, 0.5), "endpoint"), ("hover", (5, 1.5, 1.5), "endpoint"),
        ("click", (5, 1.5, 1.5), "endpoint")],
    "edge_square_to_the_plane_is_refused": [
        ("key", "right"), ("click", (0, 0, 0)), ("hover", (1, 0, 0), "endpoint"),
        ("click", (1, 0, 0), "endpoint"), ("hover", (0, 1, 0)), ("click", (0, 1, 0))],
    "cancel_and_start_again": [
        ("click", (0, 0, 0)), ("hover", (1, 0, 0)), ("click", (1, 0, 0)), ("cancel",),
        ("hover", (2, 2, 0)), ("click", (2, 2, 0)), ("hover", (3, 2, 0)), ("click", (3, 2, 0)),
        ("hover", (3, 3, 0)), ("click", (3, 3, 0))],
}


def run_rot(steps):
    from ingetrazo_core import Vec3
    from ingetrazo_core import ScreenPoint, Input
    from tools.base import ToolContext
    from tools.rotated_rectangle import RotatedRectangleTool
    view = RotView()
    tool = RotatedRectangleTool()
    tool.on_activate(view)
    out = []
    for step in steps:
        kind = step[0]
        result = None
        if kind in ("click", "hover"):
            w = Vec3(*map(float, step[1]))
            opt = step[2] if len(step) > 2 else None
            mods = Input.ShiftModifier if opt == "shift" else Input.NoModifier
            snap = _Snap("endpoint" if opt == "endpoint" else "on_face")
            x, y = view._world_to_pixel(w)
            ctx = ToolContext(view, w, ScreenPoint(x, y), mods, snap)
            (tool.on_click if kind == "click" else tool.on_hover)(ctx)
        elif kind == "value":
            v = step[1]
            result = bool(tool.on_value(view, tuple(map(float, v)) if isinstance(v, (list, tuple)) else float(v)))
        elif kind == "key":
            result = bool(tool.on_key(view, getattr(Input, _KEYS[step[1]]), Input.NoModifier))
        elif kind == "axis":
            view.axis_lock = step[1]
        elif kind == "wall":
            view.on_wall = step[1]
        elif kind == "cancel":
            tool.on_cancel(view)
        snap = _rot_snapshot(view, tool)
        snap["result"] = result
        out.append(snap)
    return out


# ---- Arcs -------------------------------------------------------------------

class ArcView(RotView):
    """A Rot view whose square face (0..4 × 0..4 on the ground) has corners
    to round; the wall face stays out of the way at x = 5."""

    def __init__(self):
        super().__init__()
        from core.protocol.commands import run
        run(self.history, "add_face", outer=[[0, 0, 0], [4, 0, 0], [4, 4, 0], [0, 4, 0]])
        self.history.undo_stack.clear()


def _arc_snapshot(view, tool):
    snap = _line_snapshot_base(view, tool)
    label = tool.value_label() if hasattr(tool, "value_label") else None
    second = next((getattr(tool, k) for k in ("end_point", "mid_point", "arm_point") if hasattr(tool, k)), None)
    colour = getattr(tool, "wireframe_color", None)
    snap.update(label=None if label is None else [label[0], _pt(label[1])],
                second=_pt(second), flashed=list(view.flashed),
                segments=getattr(tool, "segments", None),
                caption=tool.vcb_caption() if hasattr(tool, "vcb_caption") else None,
                unitless=bool(tool.value_is_unitless()),
                colour=None if colour is None else [_r(c) for c in colour],
                work_plane=None if tool.work_plane is None else [_pt(p) for p in tool.work_plane],
                plane_lock=tool.plane_lock)
    view.flashed.clear()
    return snap


ARC_SCENARIOS = {
    "arc:bulge_by_click": [
        ("click", (6, 1, 0)), ("hover", (8, 1, 0)), ("click", (8, 1, 0)), ("hover", (7, 1.4, 0)),
        ("click", (7, 1.4, 0))],
    "arc:segments_before_and_retyped_after": [
        ("value", 6.0), ("value", 1.0), ("click", (6, 1, 0)), ("hover", (8, 1, 0)), ("click", (8, 1, 0)),
        ("hover", (7, 0.5, 0)), ("click", (7, 0.5, 0)), ("segments", 4), ("segments", 1), ("segments", 10)],
    "arc:typed_bulge_and_tuple": [
        ("click", (6, 1, 0)), ("hover", (8, 1, 0)), ("click", (8, 1, 0)), ("hover", (7, 0.8, 0)),
        ("value", (1.0, 2.0)), ("value", 0.3)],
    "arc:typed_radius": [
        ("click", (6, 1, 0)), ("hover", (8, 1, 0)), ("click", (8, 1, 0)), ("hover", (7, 1.2, 0)),
        ("radius", 0.5), ("radius", 1.5)],
    "arc:half_circle_snap": [
        ("click", (6, 1, 0)), ("hover", (8, 1, 0)), ("click", (8, 1, 0)), ("hover", (7, 1.98, 0)),
        ("click", (7, 1.98, 0))],
    "arc:fillet_on_a_face_corner": [
        ("click", (1, 0, 0)), ("hover", (0.5, 0.5, 0)), ("hover", (0.02, 1.03, 0)), ("click", (0.02, 1.03, 0)),
        ("hover", (0.3, 0.3, 0)), ("click", (0.3, 0.3, 0))],
    "arc:fillet_with_alt_keeps_the_corner": [
        ("click", (1, 0, 0)), ("hover", (0.02, 1.03, 0)), ("click", (0.02, 1.03, 0)),
        ("hover", (0.3, 0.3, 0)), ("click", (0.3, 0.3, 0), "alt")],
    "arc:fillet_radius_typed": [
        ("click", (1, 0, 0)), ("hover", (0.02, 1.03, 0)), ("click", (0.02, 1.03, 0)),
        ("hover", (0.5, 0.6, 0)), ("value", 0.5)],
    "arc:fillet_radius_too_big": [
        ("click", (1, 0, 0)), ("hover", (0.02, 1.03, 0)), ("click", (0.02, 1.03, 0)),
        ("hover", (0.5, 0.6, 0)), ("value", 9.0), ("cancel",)],
    "arc:fillet_then_double_click_repeats_it": [
        ("click", (1, 0, 0)), ("hover", (0.02, 1.03, 0)), ("click", (0.02, 1.03, 0)),
        ("hover", (0.3, 0.3, 0)), ("click", (0.3, 0.3, 0)),
        ("click", (4.02, 0.03, 0)), ("double", (4.02, 0.03, 0))],
    "arc:a_rounded_corner_still_offers_the_fillet": [
        ("click", (1, 0, 0)), ("hover", (0.02, 1.03, 0)), ("click", (0.02, 1.03, 0)),
        ("hover", (0.3, 0.3, 0)), ("click", (0.3, 0.3, 0)),
        ("click", (2, 0, 0)), ("hover", (0.03, 2.02, 0)), ("click", (0.03, 2.02, 0)), ("hover", (0.6, 0.6, 0)),
        ("click", (0.6, 0.6, 0))],
    "arc:tangent_at_the_end_of_an_arc": [
        ("click", (6, 1, 0)), ("hover", (8, 1, 0)), ("click", (8, 1, 0)), ("hover", (7, 1.5, 0)),
        ("click", (7, 1.5, 0)), ("click", (8, 1, 0)), ("hover", (10, 1, 0)), ("click", (10, 1, 0)),
        ("hover", (9, 0.6, 0)), ("hover", (9, 0.55, 0)), ("click", (9, 0.55, 0))],
    "arc:a_zero_chord_and_cancel": [
        ("click", (6, 1, 0)), ("click", (6, 1, 0)), ("hover", (7, 2, 0)), ("cancel",), ("value", 3.0)],
    "arc:plane_lock_arrow": [
        ("key", "right"), ("click", (6, 1, 0)), ("hover", (6, 2, 1)), ("click", (6, 2, 1)),
        ("hover", (6, 1.5, 2)), ("click", (6, 1.5, 2))],
    "arc3:three_points": [
        ("click", (6, 1, 0)), ("hover", (7, 2, 0)), ("click", (7, 2, 0)), ("hover", (8, 1, 0)),
        ("click", (8, 1, 0))],
    "arc3:segments_and_values": [
        ("value", 5.0), ("value", (1.0, 1.0)), ("segments", 3), ("click", (6, 1, 0)), ("value", 2.0),
        ("click", (6, 1, 0)), ("click", (7, 1, 0)), ("hover", (8, 1, 0)), ("click", (8, 1, 0))],
    "center_arc:sweep": [
        ("click", (6, 1, 0)), ("hover", (7, 1, 0)), ("click", (7, 1, 0)), ("hover", (6, 2, 0)),
        ("click", (6, 2, 0))],
    "center_arc:typed_angle_and_segments": [
        ("value", 4.0), ("click", (6, 1, 0)), ("hover", (7, 1.5, 0)), ("click", (7, 1.5, 0)),
        ("hover", (6, 0, 0)), ("value", 120.0)],
    "center_arc:arm_value_refused": [
        ("click", (6, 1, 0)), ("hover", (7, 1, 0)), ("value", 2.0), ("click", (6, 1, 0)), ("cancel",)],
    "pie:wedge": [
        ("click", (6, 1, 0)), ("hover", (7, 1, 0)), ("click", (7, 1, 0)), ("hover", (6.2, 2, 0)),
        ("click", (6.2, 2, 0))],
}


def run_arc(name, steps):
    from ingetrazo_core import Vec3
    from ingetrazo_core import ScreenPoint, Input
    from tools.base import ToolContext
    from tools.arc import ArcTool, CenterArcTool, PieTool, ThreePointArcTool
    kind = name.split(":")[0]
    cls = {"arc": ArcTool, "arc3": ThreePointArcTool, "center_arc": CenterArcTool, "pie": PieTool}[kind]
    ArcTool.last_fillet_d = None
    view = ArcView()
    tool = cls()
    tool.on_activate(view)
    out = []
    for step in steps:
        what = step[0]
        result = None
        if what in ("click", "hover", "double"):
            w = Vec3(*map(float, step[1]))
            opt = step[2] if len(step) > 2 else None
            mods = Input.AltModifier if opt == "alt" else Input.NoModifier
            x, y = view._world_to_pixel(w)
            ctx = ToolContext(view, w, ScreenPoint(x, y), mods, _Snap("on_face"))
            {"click": tool.on_click, "hover": tool.on_hover,
             "double": getattr(tool, "on_double_click", tool.on_click)}[what](ctx)
        elif what == "value":
            v = step[1]
            result = bool(tool.on_value(view, tuple(map(float, v)) if isinstance(v, (list, tuple)) else float(v)))
        elif what == "segments":
            result = bool(tool.on_segments_value(view, step[1]))
        elif what == "radius":
            result = bool(tool.on_radius_value(view, step[1]))
        elif what == "key":
            result = bool(tool.on_key(view, getattr(Input, _KEYS[step[1]]), Input.NoModifier))
        elif what == "cancel":
            tool.on_cancel(view)
        snap = _arc_snapshot(view, tool)
        snap["result"] = result
        snap["last_fillet_d"] = None if ArcTool.last_fillet_d is None else _r(ArcTool.last_fillet_d)
        out.append(snap)
    ArcTool.last_fillet_d = None
    return out


# ---- Move -----------------------------------------------------------------

class MoveView(RectView):
    """A view with a loose square and a box group; picks scripted by the
    scenario (the face, the group, or nothing)."""

    def __init__(self):
        super().__init__()
        from ingetrazo_core import Matrix4, Vec3
        from core.group import Group
        from core.mesh import Mesh
        square = [Vec3(0, 0, 0), Vec3(1, 0, 0), Vec3(1, 1, 0), Vec3(0, 1, 0)]
        self.scene.mesh.add_face(square)
        box = Mesh()
        box.add_face(square)
        self.group = Group(box)
        self.group.uid = "box"
        self.group.xform = Matrix4()
        self.group.xform.translate(5.0, 0.0, 0.0)
        self.scene.groups.append(self.group)
        self.under = None

    def pick_group(self, x, y):
        return self.group if self.under == "group" else None

    def pick_face(self, x, y):
        return self.scene.mesh.faces[0] if self.under == "face" else None

    def pick_edge(self, x, y):
        return None

    def pick_dimension(self, x, y):
        return None


def _move_snapshot(view, tool):
    mesh = view.scene.mesh
    faces = sorted(sorted(_pt(v) for v in f.vertices) for f in mesh.faces)
    label = tool.value_label()
    return {"faces": faces, "groups": sorted(_pt(g.xform.map(__import__("ingetrazo_core").Vec3())) for g in view.scene.groups),
            "undo": len(view.history.undo_stack), "start": _pt(tool.start_point),
            "copy": bool(tool._copy), "array": bool(tool.array_pending(view)),
            "lines": len(tool.rubber_band_lines()),
            "label": None if label is None else [label[0], _pt(label[1])],
            "selection": len(view.scene.selection), "flashed": list(view.flashed)}


MOVE_SCENARIOS = {
    "move_the_face_under_the_cursor": [
        ("under", "face"), ("click", (0.5, 0.5, 0)), ("hover", (0.5, 2.5, 0)), ("click", (0.5, 2.5, 0))],
    "move_the_group_by_a_typed_distance": [
        ("under", "group"), ("click", (5.5, 0.5, 0)), ("hover", (7.5, 0.5, 0)), ("value", 3.0)],
    "typed_delta": [
        ("under", "face"), ("click", (0.5, 0.5, 0)), ("value", (0.0, 0.0, 2.0)), ("value", 1.0)],
    "ctrl_copy_then_an_array": [
        ("under", "group"), ("key", "ctrl"), ("key_release", "ctrl", True), ("click", (5.5, 0.5, 0)),
        ("hover", (5.5, 2.5, 0)), ("click", (5.5, 2.5, 0)), ("array", 3, "x", None),
        ("array", 4, "/", None), ("array", 2, "x", 5.0)],
    "ctrl_in_a_chord_takes_itself_back": [
        ("under", "group"), ("key", "ctrl"), ("key_release", "ctrl", False), ("click", (5.5, 0.5, 0)),
        ("hover", (6.5, 0.5, 0)), ("click", (6.5, 0.5, 0))],
    "copy_during_the_drag_with_spacing": [
        ("under", "face"), ("click", (0.5, 0.5, 0)), ("key", "ctrl"), ("key_release", "ctrl", True),
        ("hover", (2.5, 0.5, 0)), ("array", 3, "x", 1.5)],
    "cancel_puts_it_back": [
        ("under", "face"), ("click", (0.5, 0.5, 0)), ("hover", (3, 3, 0)), ("cancel",),
        ("hover", (4, 4, 0)), ("click", (4, 4, 0))],
    "nothing_under_the_cursor": [("under", None), ("click", (9, 9, 0)), ("value", 2.0)],
    "the_array_window_closes_after_an_undo": [
        ("under", "group"), ("key", "ctrl"), ("key_release", "ctrl", True), ("click", (5.5, 0.5, 0)),
        ("hover", (5.5, 1.5, 0)), ("click", (5.5, 1.5, 0)), ("undo_history",), ("array", 3, "x", None)],
}


def run_move(steps):
    from ingetrazo_core import Vec3
    from ingetrazo_core import ScreenPoint, Input
    from tools.base import ToolContext
    from tools.move import MoveTool
    view = MoveView()
    tool = MoveTool()
    tool.on_activate(view)
    out = []
    for step in steps:
        kind = step[0]
        result = None
        if kind == "under":
            view.under = step[1]
            continue
        if kind in ("click", "hover"):
            ctx = ToolContext(view, Vec3(*map(float, step[1])), ScreenPoint(0, 0), Input.NoModifier, _Snap("on_face"))
            (tool.on_click if kind == "click" else tool.on_hover)(ctx)
        elif kind == "value":
            v = step[1]
            result = bool(tool.on_value(view, tuple(map(float, v)) if isinstance(v, (list, tuple)) else float(v)))
        elif kind == "array":
            result = bool(tool.on_array_value(view, step[1], step[2], step[3]))
        elif kind == "key":
            result = bool(tool.on_key(view, getattr(Input, _KEYS[step[1]]), Input.NoModifier))
        elif kind == "key_release":
            tapped = step[2]
            view.ctrl_tapped = lambda: tapped
            result = bool(tool.on_key_release(view, getattr(Input, _KEYS[step[1]])))
        elif kind == "cancel":
            tool.on_cancel(view)
        elif kind == "undo_history":
            view.history.undo()
        snap = _move_snapshot(view, tool)
        view.flashed.clear()
        snap["result"] = result
        out.append(snap)
    return out


SCALE_SCENARIOS = {
    "corner_retype_and_undo": [("click", 400, 300), ("value", 2.0),
                               ("value", 0.5), ("undo",)],
    "absolute_and_mirror": [("click", 400, 150), ("value", ("abs_len", 6.0)),
                            ("value", -1.0), ("undo",)],
    "center_uniform": [("click", 400, 150), ("key", "Control"),
                       ("key", "Shift"), ("value", 2.0), ("undo",)],
    "invalid_then_pair": [("click", 400, 300), ("value", 0.0),
                          ("value", (1.0, 2.0, 3.0)), ("value", (1.5, 2.0))],
    "drag_release": [("click", 400, 150), ("release",), ("hover", 600, 150),
                     ("release",), ("undo",)],
    "cancel_preview": [("click", 400, 300), ("hover", 800, 600), ("cancel",)],
}


def run_scale(steps):
    from ingetrazo_core import Vec3
    from ingetrazo_core import ScreenPoint, Input
    from tools.base import ToolContext
    from tools.scale import ScaleTool

    view = LineView()
    view._world_to_pixel = lambda p: (100 * p.x(), 100 * p.y())
    face = view.scene.mesh.add_face([Vec3(0, 0, 0), Vec3(4, 0, 0),
                                     Vec3(4, 3, 0), Vec3(0, 3, 0)])
    view.scene.select([face])
    tool = ScaleTool()
    tool.on_activate(view)
    out = []
    for kind, *args in steps:
        result = None
        if kind in ("click", "hover"):
            x, y = args
            point = Vec3(x / 100, y / 100, 0)
            snap = _Snap("endpoint")
            snap.point = point
            ctx = ToolContext(view, point, ScreenPoint(x, y), Input.NoModifier, snap)
            (tool.on_click if kind == "click" else tool.on_hover)(ctx)
        elif kind == "key":
            result = tool.on_key(view, getattr(Input, "Key_" + args[0]), Input.NoModifier)
        elif kind == "release":
            tool.on_release(view)
        elif kind == "value":
            result = tool.on_value(view, args[0])
        elif kind == "cancel":
            tool.on_cancel(view)
        elif kind == "undo":
            view.history.undo()
        box = tool.scale_box_state()
        out.append({"vertices": sorted(_pt(v.position) for v in view.scene.mesh.vertices),
                    "grips": [[_pt(p), role] for p, role in box["grips"]],
                    "snap": tool.uses_snap, "caption": tool.vcb_caption(),
                    "undo": len(view.history.undo_stack), "result": result})
    return out


ROTATE_SCENARIOS = {
    "click_then_retype": [("click", 0, 0), ("click", 1, 0), ("hover", 0, 2),
                          ("click", 0, 2), ("value", -45.0), ("undo",)],
    "cancel_live_preview": [("click", 0, 0), ("click", 2, 0),
                            ("hover", 1, 1), ("cancel",)],
    "coincident_arm_then_value": [("click", 0, 0), ("click", 0, 0),
                                   ("click", 1, 0), ("value", 90.0)],
    "copy_polar_array": [("group",), ("copy",), ("click", 0, 0),
                         ("click", 1, 0), ("value", 45.0),
                         ("array", 3, "x"), ("array", 3, "/"), ("undo",)],
}


def run_rotate(steps):
    from ingetrazo_core import ScreenPoint, Input
    from ingetrazo_core import Vec3
    from tools.base import ToolContext
    from tools.rotate import RotateTool

    view = MoveView()
    view.scene.select(view.scene.mesh.faces)
    tool = RotateTool()
    tool.on_activate(view)
    out = []
    for kind, *args in steps:
        result = None
        if kind in ("click", "hover"):
            ctx = ToolContext(view, Vec3(*args, 0.), ScreenPoint(), Input.NoModifier, None)
            (tool.on_click if kind == "click" else tool.on_hover)(ctx)
        elif kind == "copy":
            result = tool.on_key(view, Input.Key_Control, Input.NoModifier)
        elif kind == "group":
            view.scene.select([view.group])
        elif kind == "value":
            result = tool.on_value(view, args[0])
        elif kind == "array":
            result = tool.on_array_value(view, *args)
        elif kind == "cancel":
            tool.on_cancel(view)
        elif kind == "undo":
            view.history.undo()
        out.append({"start": _pt(tool.start_point), "reference": _pt(tool.ref_point),
                    "copy": tool.cursor_plus, "array": tool.array_pending(view),
                    "vertices": sorted(_pt(v.position) for v in view.scene.mesh.vertices),
                    "placements": sorted(_pt(g.xform.map(Vec3())) for g in view.scene.groups),
                    "undo": len(view.history.undo_stack), "result": result})
    return out


CHANGE_AXES_SCENARIOS = {
    "tilted_frame": [("click", 1, 2, 3), ("hover", 2, 2, 4), ("click", 2, 2, 4),
                     ("hover", 1, 3, 4), ("click", 1, 3, 4), ("undo",)],
    "degenerate_and_cancel": [("click", 1, 0, 0), ("click", 1, 0, 0), ("click", 3, 0, 0),
                              ("click", 2, 0, 0), ("value", 4), ("cancel",)],
}


def run_change_axes(steps):
    from core.group import frame_axes, group_frame
    from core.scene import Scene
    from tests.test_parity_tools import _Vp, _ctx
    from tests.test_group_axes import _slab
    from tools.change_axes import ChangeAxesTool
    vp, tool = _Vp(Scene()), ChangeAxesTool()
    group = _slab()
    vp.scene.groups.append(group)
    tool.target = group
    tool.on_activate(vp)
    out = []
    for kind, *args in steps:
        result = None
        if kind in ("click", "hover"):
            getattr(tool, "on_" + kind)(_ctx(vp, *args))
        elif kind == "undo":
            vp.history.undo()
        elif kind == "value":
            result = tool.on_value(vp, args[0])
        else:
            tool.on_cancel(vp)
        frame = group_frame(group)
        out.append({"start": _pt(tool.start_point), "red": _pt(tool.red_point), "target": tool.target is group,
                    "band": [[_pt(a), _pt(b)] for a, b in tool.rubber_band_lines()],
                    "frame": [_pt(p) for p in frame_axes(frame)] if frame is not None else None,
                    "undo": len(vp.history.undo_stack), "result": result})
    return out


FOLLOWME_SCENARIOS = {
    "preselection": ["select_edges", "activate", "profile", "click", "undo"],
    "drag_release": ["activate", "profile", "click", "release", "edge0", "edge1", "release", "undo"],
    "rollback": ["activate", "profile", "click", "edge0", "edge1", "edge1", "value", "cancel", "release"],
    "perimeter": ["activate", "profile", "click", "alt", "click", "undo"],
}


def run_followme(steps):
    from ingetrazo_core import Input
    from core.scene import Scene
    from tests.test_followme import _Vp2, _ctx, V
    from tools.followme import FollowMeTool
    vp, tool = _Vp2(Scene()), FollowMeTool()
    mesh = vp.scene.mesh
    profile = mesh.add_face([V(0, 0, .5), V(-1, 0, .5), V(-1, 0, 1.5), V(0, 0, 1.5)])
    perimeter = mesh.add_face([V(0, 0, 0), V(6, 0, 0), V(6, 6, 0), V(0, 6, 0)])
    edges = [mesh.add_edge(V(0, 0, 0), V(0, 3, 0)), mesh.add_edge(V(0, 3, 0), V(2, 3, 0))]
    out = []
    for step in steps:
        result = None
        if step == "select_edges":
            vp.scene.selection.update(edges)
        elif step in ("profile", "alt"):
            vp._pick = profile if step == "profile" else perimeter
            tool.on_hover(_ctx(vp, 20, 10, mods=Input.NoModifier if step == "profile" else Input.AltModifier))
        elif step.startswith("edge"):
            vp._edge = edges[int(step[-1])]
            tool.on_hover(_ctx(vp, 80, 10))
        elif step == "click":
            tool.on_click(_ctx(vp, 20, 10))
        elif step == "undo":
            vp.history.undo()
        elif step == "value":
            result = tool.on_value(vp, 2)
        else:
            getattr(tool, "on_" + step)(vp)
        out.append({"band": [[_pt(a), _pt(b)] for a, b in tool.rubber_band_lines()],
                    "preview": len(tool.preview_faces()), "faces": len(mesh.faces),
                    "vertices": len(mesh.vertices), "undo": len(vp.history.undo_stack), "result": result})
    return out


FREEHAND_SCENARIOS = {
    "simplified": [("click", 0, 0), ("hover", .01, 0), ("hover", .5, .01),
                   ("hover", 1, 0), ("hover", 1, 1), ("release",), ("undo",)],
    "closed": [("click", 1, 1), ("hover", 3, 1), ("hover", 3, 2), ("hover", 1.05, 1),
               ("release",), ("undo",)],
    "short_cancel": [("click", 0, 0), ("hover", .05, .05), ("release",),
                     ("click", 1, 1), ("hover", 2, 1), ("value", 3), ("cancel",), ("release",)],
}


def run_freehand(steps):
    from core.scene import Scene
    from tests.test_parity_tools import _Vp, _ctx
    from tools.freehand import FreehandTool
    vp, tool = _Vp(Scene()), FreehandTool()
    tool.on_activate(vp)
    out = []
    for kind, *args in steps:
        result = None
        if kind in ("click", "hover"):
            getattr(tool, "on_" + kind)(_ctx(vp, *args))
        elif kind == "value":
            result = tool.on_value(vp, args[0])
        elif kind == "undo":
            vp.history.undo()
        else:
            getattr(tool, "on_" + kind)(vp)
        snap = _line_snapshot_base(vp, tool)
        snap.update(curves=len({e.curve for e in vp.scene.mesh.edges}), result=result)
        out.append(snap)
    return out


FLIP_SCENARIOS = {
    "lock_and_mirror": ["x", "click", "undo", "y", "cancel"],
    "copy": ["control", "x", "click", "undo"],
    "copy_chord": ["control", "chord", "y", "click", "undo"],
    "direct_nested": ["group", "direct", "undo", "clear", "direct"],
}


def run_flip(steps):
    from types import SimpleNamespace
    from ingetrazo_core import Input
    from core.scene import Scene
    from core.group import placement_points
    from tests.test_flip_nested_groups import _vp, _cabinet, V
    from tools.flip import FlipTool
    scene = Scene()
    face = scene.mesh.add_face([V(1, 0, 0), V(3, 0, 0), V(1, 2, 0)])
    scene.select([face])
    vp, tool = _vp(scene), FlipTool()
    tool.on_activate(vp)
    out = []
    for kind in steps:
        result = None
        if kind in ("x", "y", "control"):
            key = {"x": Input.Key_Right, "y": Input.Key_Left, "control": Input.Key_Control}[kind]
            result = tool.on_key(vp, key, Input.NoModifier)
        elif kind == "chord":
            vp.ctrl_tapped = lambda: False
            result = tool.on_key_release(vp, Input.Key_Control)
        elif kind == "click":
            tool.on_click(SimpleNamespace(viewport=vp))
        elif kind == "cancel":
            tool.on_cancel(vp)
        elif kind == "undo":
            vp.history.undo()
        elif kind == "group":
            _cabinet(scene)
        elif kind == "clear":
            scene.clear_selection()
        elif kind == "direct":
            result = tool.flip(vp, "x")
        out.append({"axis": tool._lock_axis, "copy": tool.cursor_plus, "result": result,
                    "faces": len(scene.mesh.faces), "undo": len(vp.history.undo_stack),
                    "vertices": sorted(_pt(v.position) for v in scene.mesh.vertices),
                    "groups": [sorted(placement_points(g).round(6).tolist()) for g in scene.groups]})
    return out


PASTE_SCENARIOS = {
    "face_and_edge": [("hover", 3, 4), ("click", 3, 4), ("undo",), ("redo",)],
    "in_place": [("in_place",), ("undo",)],
    "group": [("group",), ("hover", 2, 1), ("click", 2, 1), ("undo",)],
    "cancel": [("hover", -3, 0), ("value", 2), ("cancel",), ("click", 1, 1)],
}


def run_paste(steps):
    from ingetrazo_core import ScreenPoint, Input
    from core.history import History
    from core.scene import Scene
    from tests.test_paste import _PreviewVP, _make_group, V
    from tools.base import ToolContext
    from tools.paste import PasteTool
    scene = Scene()
    clip = {"ref": V(1, 2), "edges": [(V(1, 2), V(1, 3), False, None)],
            "faces": [([V(1, 2), V(2, 2), V(2, 3)], [], {"layer": "Layer0"})]}
    vp, tool = _PreviewVP(scene, History(scene), clip), PasteTool()
    tool.on_activate(vp)
    out = []
    for kind, *args in steps:
        result = None
        if kind in ("hover", "click"):
            getattr(tool, "on_" + kind)(ToolContext(vp, V(*args), ScreenPoint(), Input.NoModifier, None))
        elif kind == "group":
            clip["groups"] = [_make_group(scene)]
            tool.on_activate(vp)
        elif kind == "in_place":
            result = tool.in_place(vp)
        elif kind == "value":
            result = tool.on_value(vp, args[0])
        elif kind == "cancel":
            tool.on_cancel(vp)
        elif kind in ("undo", "redo"):
            getattr(vp.history, kind)()
        out.append({"ref": _pt(tool.start_point), "offset": _pt(tool._offset), "ended": vp.ended,
                    "preview": [[_pt(a), _pt(b)] for a, b in tool.rubber_band_lines()],
                    "vertices": sorted(_pt(v.position) for v in scene.mesh.vertices),
                    "faces": len(scene.mesh.faces), "groups": len(scene.groups),
                    "selected": len(scene.selection), "undo": len(vp.history.undo_stack), "result": result})
    return out


TAPE_SCENARIOS = {
    "axis_offset": [("axis", "x"), ("click", 2, .02, None), ("hover", 3, 1, None),
                    ("value", -.75), ("undo",)],
    "source_point": [("click", 2, 1, "endpoint"), ("hover", 2, 4, None), ("value", 2.5),
                     ("click", 0, 0, "origin"), ("click", 3, 4, "endpoint")],
    "mode_cycle": [("control",), ("click", 1, 2, None), ("click", 1, 2, None), ("control",),
                   ("click", 0, 0, None), ("hover", 3, 4, None), ("value", 4),
                   ("click", 3, 4, None), ("undo",)],
    "guide_source": [("guide",), ("click", 2, 1, "on_edge"), ("hover", 3, 2, None),
                     ("click", 3, 2, None), ("click", 2, 1, "on_edge"), ("cancel",), ("undo",)],
}


def run_tape(steps):
    from ingetrazo_core import Input
    from core.guide import Guide
    from core.scene import Scene
    from tests.test_tape_axis_source_and_guide_segment import _Vp, _ctx, V
    from tools.tape import TapeMeasureTool
    view, tool = _Vp(Scene()), TapeMeasureTool()
    tool.on_activate(view)
    out = []
    for kind, *args in steps:
        result = None
        if kind == "axis":
            view._axis = args[0]
        elif kind == "guide":
            source = Guide(V(0, 1), V(1, 0))
            view.scene.guides.append(source)
            view.pick_guide = lambda *_: source
        elif kind in ("hover", "click"):
            ctx = _ctx(view, *args[:2], kind=args[2])
            getattr(tool, "on_" + kind)(ctx)
        elif kind == "control":
            result = tool.on_key_release(view, Input.Key_Control)
        elif kind == "value":
            result = tool.on_value(view, args[0])
        elif kind == "cancel":
            tool.on_cancel(view)
        elif kind == "undo":
            view.history.undo()
        label = tool.value_label()
        out.append({"mode": tool._mode, "start": _pt(tool.start_point), "measured": tool._measured,
                    "label": [label[0], _pt(label[1])] if label else None,
                    "band": [[_pt(a), _pt(b)] for a, b in tool.rubber_band_lines()],
                    "preview": [[_pt(a), _pt(b)] for a, b in tool.guide_preview_lines()],
                    "guides": [[_pt(g.point), _pt(g.direction), _pt(g.origin)] for g in view.scene.guides],
                    "undo": len(view.history.undo_stack), "result": result})
    return out


FILLET_SCENARIOS = {
    "cursor_round": [("hover", 1, 0, 1), ("click", 1, 0, 1), ("hover", 1, .2, 1),
                     ("click", 1, .2, 1), ("undo",)],
    "selected_edge": [("select",), ("segments", 4), ("profile", .4), ("value", .15), ("undo",)],
    "corner": [("affect", "vertices"), ("hover", .1, 0, 1), ("click", .1, 0, 1),
               ("hover", .2, .1, 1), ("value", .15), ("undo",)],
    "refused_size": [("hover", 1, 0, 1), ("click", 1, 0, 1), ("value", -1),
                     ("value", 5), ("cancel",)],
}


def run_fillet(steps):
    from contextlib import ExitStack
    from unittest.mock import patch
    from tests.test_fillet_tool import _box_scene, _ctx, edge_at
    from tools.fillet import FilletTool
    view = RectView()
    mesh = _box_scene(view)
    edge = edge_at(mesh, (0, 0, 1), (2, 0, 1))
    out = []
    with ExitStack() as changes:
        for key, val in dict(radius=.1, segments=8, profile=.5, measure="radius",
                             corners="round", shape=None, affect="edges").items():
            changes.enter_context(patch.object(FilletTool, key, val))
        tool = FilletTool()
        tool.on_activate(view)
        view._hover_edge = edge
        for kind, *args in steps:
            result = None
            if kind in ("hover", "click"):
                getattr(tool, "on_" + kind)(_ctx(view, args))
            elif kind == "select":
                view.scene.select([edge])
                tool.on_activate(view)
            elif kind == "affect":
                tool.set_affect(view, args[0])
            elif kind in ("value", "profile", "segments"):
                method = "on_value" if kind == "value" else f"on_{kind}_value"
                result = getattr(tool, method)(view, args[0])
            elif kind == "cancel":
                tool.on_cancel(view)
            elif kind == "undo":
                view.history.undo()
            label = tool.value_label()
            out.append({"sizing": tool.sizing, "targets": len(tool.edges),
                        "radius": round(FilletTool.radius, 6), "label": label[0] if label else None,
                        "preview": len(tool.preview_faces()), "lines": len(tool.rubber_band_lines()),
                        "faces": len(mesh.faces), "vertices": sorted(_pt(v.position) for v in mesh.vertices),
                        "undo": len(view.history.undo_stack), "result": result})
    return out


PROTRACTOR_SCENARIOS = {
    "click_and_reaim": [("click", 2, 1, 0), ("click", 5, 1, 0), ("hover", 2, 4, 0),
                        ("click", 2, 4, 0), ("value", 30), ("value", -60), ("undo",)],
    "typed_clockwise": [("click", 0, 0, 0), ("click", 2, 0, 0), ("hover", 1, -1, 0),
                        ("value", 45), ("value", 20), ("undo",), ("undo",), ("value", 10)],
    "measure_only": [("control",), ("click", 0, 0, 0), ("click", 1, 0, 0),
                     ("hover", .4, .3, 0), ("click", .4, .3, 0), ("value", 20)],
    "tilt_plane_and_cancel": [("click", 0, 0, 0), ("hover", 2, 2, 1), ("release",),
                              ("click", 2, 0, 0), ("hover", 0, 1, 1), ("cancel",)],
}


def run_protractor(steps):
    from ingetrazo_core import Input
    from core.scene import Scene
    from tests.test_protractor import _VpScreen, _ctx
    from tools.protractor import ProtractorTool
    view, tool = _VpScreen(Scene()), ProtractorTool()
    tool.on_activate(view)
    out = []
    for kind, *args in steps:
        result = None
        if kind in ("hover", "click"):
            (tool.on_hover if kind == "hover" else tool.on_click)(_ctx(view, *args))
        elif kind == "control":
            result = tool.on_key(view, Input.Key_Control, Input.NoModifier)
        elif kind == "release":
            tool.on_release(view)
        elif kind == "value":
            result = tool.on_value(view, args[0])
        elif kind == "undo":
            view.history.undo()
        elif kind == "cancel":
            tool.on_cancel(view)
        label = tool.value_label()
        out.append({"centre": _pt(tool.start_point), "arm": _pt(tool.ref_point),
                    "axis": _pt(tool._axis()), "plus": tool.cursor_plus,
                    "label": label[0] if label else None,
                    "guide_preview": [[_pt(a), _pt(b)] for a, b in tool.guide_preview_lines()],
                    "guides": [[_pt(g.point), _pt(g.direction)] for g in view.scene.guides],
                    "undo": len(view.history.undo_stack), "result": result})
    return out


PAINT_SCENARIOS = {
    "front_selection_and_undo": ["front", "select", "blue", "front", "undo", "undo"],
    "back_sample_and_repaint": ["back", "blue", "sample_back", "other", "default", "back", "undo"],
    "sample_default_and_miss": ["front", "sample_empty", "front", "miss", "undo"],
    "closed_container": ["container", "front", "blue", "sample_group", "undo"],
}


def run_paint(steps):
    from contextlib import ExitStack
    from unittest.mock import patch
    from ingetrazo_core import Input, ScreenPoint
    from ingetrazo_core import Vec3
    from core.group import Group
    from core.mesh import Mesh
    from tests.test_paint_recent import _FakeViewport, _quad, _use
    from tools.base import ToolContext
    from tools.paint import PaintTool
    view = _FakeViewport()
    a, b = _quad(view.scene.mesh), _quad(view.scene.mesh, 1.)
    group = Group(Mesh(), name="Room")
    painted = _quad(group.mesh)
    view.scene.groups.append(group)
    container = False
    hit = (a, None)
    side = -1.
    view.pick_face_any = lambda x, y: hit
    view.pick_group = lambda x, y: group if container else None
    view._pixel_to_ray = lambda x, y: (Vec3(0, 0, 5), Vec3(0, 0, side))
    with ExitStack() as saved:
        for key in ("recent", "sample_armed", "current_texture_plane", "current_color",
                    "current_texture", "current_opacity", "current_is_default", "current_material"):
            saved.enter_context(patch.object(PaintTool, key, getattr(PaintTool, key)))
        PaintTool.recent, PaintTool.sample_armed, PaintTool.current_texture_plane = [], False, None
        _use(color=(.8, .3, .2), name="Brick", opacity=.65)
        tool, rows = PaintTool(), []
        for step in steps:
            if step == "container":
                container = True
            elif step == "select":
                view.scene.selection = {a, b}
            elif step == "undo":
                view.history.undo()
            elif step == "default":
                PaintTool.current_is_default = True
            elif step == "blue":
                _use(color=(.1, .3, .8), name="Blue")
            else:
                sample = step.startswith("sample")
                PaintTool.sample_armed = sample
                side = 1. if step in ("back", "sample_back") else -1.
                hit = ((None, None) if step == "miss" else (painted, group) if container
                       else (b if step in ("other", "sample_empty") else a, None))
                ctx = ToolContext(view, Vec3(), ScreenPoint(), Input.NoModifier, None)
                tool.on_hover(ctx)
                tool.on_click(ctx)
            rows.append(json.loads(json.dumps({
                "faces": [dict(f.attrs) for f in (a, b, painted)], "group": group.material,
                "materials": sorted(view.scene.materials), "armed": PaintTool.sample_armed,
                "default": PaintTool.current_is_default, "active": PaintTool._current_as_material(),
                "recent": PaintTool.recent, "undo": len(view.history.undo_stack)})))
    return rows


OFFSET_SCENARIOS = {
    "inward_retype_refuse_and_undo": [("hover", 2, .1), ("click", 2, .1), ("hover", 3, .5),
                                      ("value", 1.), ("value", 3.), ("value", .25), ("undo",)],
    "outward_click": [("hover", .1, 2), ("click", .1, 2), ("hover", -.5, 2), ("click", -.5, 2)],
    "zero_and_cancel": [("hover", 2, .1), ("click", 2, .1), ("click", 2, 0), ("value", 0.),
                        ("hover", 3, .25), ("cancel",), ("value", .5)],
    "armed_edge_chain": [("arm",), ("hover", 2, .1), ("click", 2, .1), ("hover", 2, .5),
                         ("value", .25), ("value", .5), ("undo",)],
}


def run_offset(steps):
    from tests.test_offset_reference import Viewport, context
    from tools.offset import OffsetTool
    view, tool = Viewport(), OffsetTool()
    tool.on_activate(view)
    out = []
    for kind, *args in steps:
        result = None
        if kind in ("hover", "click"):
            ctx = context(view, *args)
            (tool.on_hover if kind == "hover" else tool.on_click)(ctx)
        elif kind == "arm":
            view.scene.selection = set(view.scene.mesh.edges[:2])
            tool.on_activate(view)
        elif kind == "value":
            result = tool.on_value(view, *args)
        elif kind == "cancel":
            tool.on_cancel(view)
        elif kind == "undo":
            view.history.undo()
        label = tool.value_label()
        ref = [_pt(p) for p in tool._offset_reference or ()]
        # An edge selected in the opposite order draws the same segment.
        reference = [*sorted(ref[:2]), ref[2]] if ref else []
        out.append({"distance": round(tool.distance, 9), "drag": tool.dragging,
                    "lines": sorted(sorted([_pt(a), _pt(b)]) for a, b in tool.rubber_band_lines()),
                    "reference": reference,
                    "label": None if label is None else [label[0], _pt(label[1])],
                    "areas": sorted(round(f.area(), 8) for f in view.scene.mesh.faces),
                    "vertices": sorted(_pt(v.position) for v in view.scene.mesh.vertices),
                    "undo": len(view.history.undo_stack), "result": result})
    return out


WALKTHROUGH_SCENARIOS = {
    "place_drag_offset": ("PositionCameraTool", [("click", (3, 2, 0), (100, 100)),
        ("hover", (4, 6, 0), (106, 100)), ("hover", (4, 6, 0), (107, 100)),
        ("release",), ("value", 2.5), ("value", -1.0)]),
    "place_cancel": ("PositionCameraTool", [("click", (1, 2, 3), (100, 100)),
        ("hover", (3, 4, 5), (130, 150)), ("cancel",), ("release",)]),
    "turn_and_height": ("LookAroundTool", [("click", (0, 0, 0), (100, 100)),
        ("hover", (0, 0, 0), (350, 100)), ("hover", (0, 0, 0), (350, 30)),
        ("release",), ("hover", (0, 0, 0), (200, 100)), ("value", 1.25)]),
    "walk": ("WalkTool", [("walk", 1, 2, "none"), ("walk", 0, 150, "none"),
        ("walk", 75, 150, "run"), ("walk", 100, 50, "vertical"),
        ("walk", -100, -150, "through"), ("value", 2.0)]),
    "first_person": ("FirstPersonTool", [("keys", "WD", "none"), ("keys", "WS", "none"),
        ("keys", "AE", "run"), ("look", 250, -125), ("keys", "W", "through"),
        ("value", 1.9)]),
}


def run_walkthrough(tool_name, steps):
    from unittest.mock import patch
    from ingetrazo_core import ScreenPoint, Input
    from ingetrazo_core import Vec3
    from core.camera import OrbitCamera
    from tools.base import ToolContext
    from tools import walkthrough

    view = LineView()
    view.camera = OrbitCamera()
    view.camera.look_from(Vec3(1, 2, 1.68), Vec3(0, 1, 0))
    view.width = lambda: 1000
    view.height = lambda: 700
    tool = getattr(walkthrough, tool_name)()
    tool.on_activate(view)
    out = []
    with (patch.object(walkthrough, "eye_height", return_value=1.68),
          patch.object(walkthrough, "set_eye_height"),
          patch.object(walkthrough, "look_sensitivity", return_value=25)):
        for kind, *args in steps:
            result = None
            if kind in ("click", "hover"):
                ctx = ToolContext(view, Vec3(*args[0]), ScreenPoint(*args[1]), Input.NoModifier, None)
                (tool.on_click if kind == "click" else tool.on_hover)(ctx)
            elif kind == "value":
                result = tool.on_value(view, args[0])
            elif kind == "release":
                tool.on_release(view)
            elif kind == "cancel":
                tool.on_cancel(view)
            elif kind == "look":
                tool.on_look(view, *args)
            elif kind in ("walk", "keys"):
                mods = {"none": Input.NoModifier, "through": Input.AltModifier,
                        "vertical": Input.ShiftModifier, "run": Input.ControlModifier
                        if kind == "walk" else Input.ShiftModifier}[args[-1]]
                if kind == "walk":
                    tool.step(view, args[0], args[1], 0.1, mods)
                else:
                    tool.step(view, {getattr(Input, "Key_" + k) for k in args[0]}, 0.1, mods)
            out.append({"eye": _pt(view.camera.eye()), "direction": _pt(view.camera.forward()),
                        "lines": [[_pt(a), _pt(b)] for a, b in tool.rubber_band_lines()],
                        "undo": len(view.history.undo_stack), "result": result})
    return out


def _compact(fixture) -> str:
    """One scenario step per line: the protractors' segments make an
    indented file several times larger than the data."""
    rows = []
    for name, steps in fixture.items():
        body = ",\n  ".join(json.dumps(step, separators=(",", ":")) for step in steps)
        rows.append(f"{json.dumps(name)}: [\n  {body}]")
    return "{\n" + ",\n".join(rows) + "\n}\n"


def record(which=("line", "select", "rectangle", "move", "circle", "rotated_rectangle", "arc")):
    FIXTURES.mkdir(parents=True, exist_ok=True)
    if "line" in which:
        line = {name: run_line(steps, far) for name, steps, far in line_cases()}
        (FIXTURES / "line.json").write_text(json.dumps(line, indent=1) + "\n")
    if "select" in which:
        select = {name: run_select(steps) for name, steps in SELECT_SCENARIOS.items()}
        (FIXTURES / "select.json").write_text(json.dumps(select, indent=1) + "\n")
    if "move" in which:
        move = {name: run_move(steps) for name, steps in MOVE_SCENARIOS.items()}
        (FIXTURES / "move.json").write_text(json.dumps(move, indent=1) + "\n")
    if "rectangle" in which:
        rect = {name: run_rect(steps, far) for name, steps, far in rect_cases()}
        (FIXTURES / "rectangle.json").write_text(json.dumps(rect, indent=1) + "\n")
    if "circle" in which:
        circle = {name: run_circle(steps, polygon, far) for name, steps, polygon, far in circle_cases()}
        (FIXTURES / "circle.json").write_text(_compact(circle))
    if "rotated_rectangle" in which:
        rot = {name: run_rot(steps) for name, steps in ROT_SCENARIOS.items()}
        (FIXTURES / "rotated_rectangle.json").write_text(_compact(rot))
    if "change_axes" in which:
        cases = {name: run_change_axes(steps) for name, steps in CHANGE_AXES_SCENARIOS.items()}
        (FIXTURES / "change_axes.json").write_text(_compact(cases))
    from tests import remaining_tool_parity as remaining
    for kind in set(which).intersection(remaining.CASES):
        cases = {name: remaining.run(kind, steps) for name, steps in remaining.CASES[kind].items()}
        (FIXTURES / f"{kind}.json").write_text(_compact(cases))
    if "solid_tools" in which:
        cases = {name: remaining.run_solid(*args) for name, args in remaining.solid_cases().items()}
        (FIXTURES / "solid_tools.json").write_text(_compact(cases))
    if "followme" in which:
        cases = {name: run_followme(steps) for name, steps in FOLLOWME_SCENARIOS.items()}
        (FIXTURES / "followme.json").write_text(_compact(cases))
    if "freehand" in which:
        cases = {name: run_freehand(steps) for name, steps in FREEHAND_SCENARIOS.items()}
        (FIXTURES / "freehand.json").write_text(_compact(cases))
    if "flip" in which:
        cases = {name: run_flip(steps) for name, steps in FLIP_SCENARIOS.items()}
        (FIXTURES / "flip.json").write_text(_compact(cases))
    if "paste" in which:
        cases = {name: run_paste(steps) for name, steps in PASTE_SCENARIOS.items()}
        (FIXTURES / "paste.json").write_text(_compact(cases))
    if "tape" in which:
        cases = {name: run_tape(steps) for name, steps in TAPE_SCENARIOS.items()}
        (FIXTURES / "tape.json").write_text(_compact(cases))
    if "fillet" in which:
        cases = {name: run_fillet(steps) for name, steps in FILLET_SCENARIOS.items()}
        (FIXTURES / "fillet.json").write_text(_compact(cases))
    if "protractor" in which:
        cases = {name: run_protractor(steps) for name, steps in PROTRACTOR_SCENARIOS.items()}
        (FIXTURES / "protractor.json").write_text(_compact(cases))
    if "paint" in which:
        cases = {name: run_paint(steps) for name, steps in PAINT_SCENARIOS.items()}
        (FIXTURES / "paint.json").write_text(_compact(cases))
    if "offset" in which:
        cases = {name: run_offset(steps) for name, steps in OFFSET_SCENARIOS.items()}
        (FIXTURES / "offset.json").write_text(_compact(cases))
    if "walkthrough" in which:
        cases = {name: run_walkthrough(*case) for name, case in WALKTHROUGH_SCENARIOS.items()}
        (FIXTURES / "walkthrough.json").write_text(_compact(cases))
    if "rotate" in which:
        rotate = {name: run_rotate(steps) for name, steps in ROTATE_SCENARIOS.items()}
        (FIXTURES / "rotate.json").write_text(_compact(rotate))
    if "scale" in which:
        scale = {name: run_scale(steps) for name, steps in SCALE_SCENARIOS.items()}
        (FIXTURES / "scale.json").write_text(_compact(scale))
    if "arc" in which:
        arcs = {name: run_arc(name, steps) for name, steps in ARC_SCENARIOS.items()}
        (FIXTURES / "arc.json").write_text(_compact(arcs))


def load(tool):
    return json.loads((FIXTURES / f"{tool}.json").read_text())


if __name__ == "__main__":
    if sys.argv[1:2] == ["record"]:
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        record(tuple(sys.argv[2:]) or ("line", "select", "rectangle", "move"))

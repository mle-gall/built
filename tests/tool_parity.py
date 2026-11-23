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
import os
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
    from PySide6.QtCore import QPointF, Qt
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
            ctx = ToolContext(view, at(step[1]), QPointF(0, 0), Qt.NoModifier, snap)
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
    from PySide6.QtCore import QPointF, Qt
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
        mods = Qt.KeyboardModifier(_MODS[step[3]]) if len(step) > 3 else Qt.NoModifier
        ctx = ToolContext(view, Vec3(), QPointF(step[1], step[2]), mods, None)
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
    from PySide6.QtCore import QPointF, Qt
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
            mods = Qt.ShiftModifier if len(step) > 2 and step[2] == "shift" else Qt.NoModifier
            ctx = ToolContext(view, at(step[1]), QPointF(0, 0), mods, _Snap("on_face"))
            (tool.on_click if kind == "click" else tool.on_hover)(ctx)
        elif kind == "value":
            v = step[1]
            result = bool(tool.on_value(view, tuple(map(float, v)) if isinstance(v, (list, tuple)) else float(v)))
        elif kind == "key":
            result = bool(tool.on_key(view, getattr(Qt, _KEYS[step[1]]), Qt.NoModifier))
        elif kind == "key_release":
            tapped = step[2]
            view.ctrl_tapped = lambda: tapped
            result = bool(tool.on_key_release(view, getattr(Qt, _KEYS[step[1]])))
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


def record(which=("line", "select", "rectangle")):
    FIXTURES.mkdir(parents=True, exist_ok=True)
    if "line" in which:
        line = {name: run_line(steps, far) for name, steps, far in line_cases()}
        (FIXTURES / "line.json").write_text(json.dumps(line, indent=1) + "\n")
    if "select" in which:
        select = {name: run_select(steps) for name, steps in SELECT_SCENARIOS.items()}
        (FIXTURES / "select.json").write_text(json.dumps(select, indent=1) + "\n")
    if "rectangle" in which:
        rect = {name: run_rect(steps, far) for name, steps, far in rect_cases()}
        (FIXTURES / "rectangle.json").write_text(json.dumps(rect, indent=1) + "\n")


def load(tool):
    return json.loads((FIXTURES / f"{tool}.json").read_text())


if __name__ == "__main__":
    if sys.argv[1:2] == ["record"]:
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        record(tuple(sys.argv[2:]) or ("line", "select", "rectangle"))

# SPDX-License-Identifier: GPL-3.0-or-later
"""'From point' on a constrained line (owner report, 2026-11-26).

Drawing along red, the end of the line must be able to line up with any
point in the plane perpendicular to the line there, e.g. a base plate's
corner offset in Y and Z, without the line touching it:

* under a lock (arrow key, Shift), hovering ANY inference point (endpoint,
  midpoint, centre, face centre, intersection) puts the end at that point's
  foot on the line, even when the foot is behind geometry;
* on an axis without a lock, a point hovered lately (picked up on the way,
  ~1.5 s) lines the end up when the cursor comes back near its foot, even
  after the cursor crossed other points on the way back;
* the cue: a dotted guide from the point to the end, in the colour of the
  axis it runs along or else of the line's axis, the point marked, "From
  point";
* Line, Move, Copy, Tape, Rectangle (second corner) and Push/Pull honour it,
  and a hover stays cheap on a million faces.

The view cases drive an unshown ``WgpuView`` with the real tools (the fixture
of ``test_inference_and_face_cutting``)."""
import math
import time

import numpy as np
import pytest
from built_core import Input as Qt
from built_core import Matrix4 as QMatrix4x4
from built_core import Vec3 as V
from built_core.group import Group
from built_core.inference import InferenceEngine
from built_core.mesh import Mesh
from built_core.scene import Scene
from built_core.snap import AXIS_COLORS, compute_snap
from tests.headless_gpu import key, mouse, tap
from tests.test_inference_and_face_cutting import click, refresh, use, view  # noqa: F401
from tests.test_wgpu_axis_lock_hidden_refs_1085 import _box
from tools.rust_tools import MoveTool, RectangleTool, TapeMeasureTool

CORNER = V(3, 1.5, 1.1)        # the reference box's top corner, off red in Y and Z
FOOT = V(3, 0, 0)              # its foot on red from the origin
OTHER = V(3.6, 0.6, 0.3)       # a corner the cursor crosses on the way back


def _same(a, b, tol=1e-4):
    return (a - b).length() <= tol


# ---- the projection itself (compute_snap, no view) --------------------------

def _w2p(p):
    """An oblique projection: no axis is end-on."""
    return (400 + 100 * p.x() - 45 * p.y(), 300 - 100 * p.z() + 35 * p.y() - 10 * p.x())


def _locked(ref, start, direction=None, axis=None, centers=None):
    scene = Scene()
    if ref is not None:
        scene.mesh.add_edge(ref, ref + V(0.0, 0.0, 0.5))     # ref is an endpoint
    target = ref if ref is not None else centers[0][0]

    def project(s, d):
        return s + d * V.dotProduct(target - s, d)

    return compute_snap(
        candidate_world=target, candidate_pixel=_w2p(target), scene=scene,
        world_to_pixel=_w2p, threshold_px=10.0, edge_threshold_px=12.0,
        project_onto_line=project, start_point=start, axis_lock=axis,
        shift_lock_dir=direction, face_centers=centers)


@pytest.mark.parametrize("axis", ["x", "y", "z"])
def test_an_axis_lock_takes_the_foot_of_the_hovered_point(axis):
    start = V(0.3, -0.2, 0.1)
    ref = V(2.0, 1.25, 0.7)
    r = _locked(ref, start, axis=axis)
    u = {"x": V(1, 0, 0), "y": V(0, 1, 0), "z": V(0, 0, 1)}[axis]
    assert r.kind == "from_point", r
    assert _same(r.point, start + u * V.dotProduct(ref - start, u), 1e-9)
    assert _same(r.guide[0], ref, 1e-9) and _same(r.guide[1], r.point, 1e-9)
    assert r.source_kind == "endpoint"


@pytest.mark.parametrize("direction", [
    V(1, 1, 0), V(1, 2, 3), V(-0.4, 0.1, 0.9), V(0.2, -1, 0.05)])
def test_a_locked_direction_takes_the_foot_of_the_hovered_point(direction):
    u = direction.normalized()
    start = V(0.1, 0.2, 0.0)
    ref = V(1.6, 0.9, 1.3)
    r = _locked(ref, start, direction=u)
    assert r.kind == "from_point", r
    foot = start + u * V.dotProduct(ref - start, u)
    assert _same(r.point, foot, 1e-9)
    assert abs(V.dotProduct(r.point - ref, u)) < 1e-9      # the perpendicular plane


def test_the_guide_takes_the_colour_of_its_axis_or_of_the_line():
    # Off red in Y only: the guide runs along green.
    r = _locked(V(2, 1.5, 0), V(0, 0, 0), axis="x")
    assert r.kind == "from_point" and tuple(r.guide_color) == AXIS_COLORS["y"]
    # Off red in Y and Z: the guide runs along no axis, it takes red's.
    r = _locked(V(2, 1.5, 0.8), V(0, 0, 0), axis="x")
    assert r.kind == "from_point" and tuple(r.guide_color) == AXIS_COLORS["x"]


def test_a_locked_line_takes_the_foot_of_a_face_centre():
    centre = V(2.2, 1.4, 0.9)
    r = _locked(None, V(0, 0, 0), axis="x", centers=[(centre, None)])
    assert r.kind == "from_point", r
    assert _same(r.point, V(2.2, 0, 0), 1e-9)
    assert r.source_kind == "face_center"


# ---- the remembered points --------------------------------------------------

def test_recent_points_are_few_fresh_and_newest_first():
    e = InferenceEngine()
    for i in range(8):
        e.note_reference(V(i, 0, 0))
    e.note_reference(V(3, 0, 0))                    # again: moves to the front
    pts = e._recent_points
    assert len(pts) == 6
    assert _same(pts[0], V(3, 0, 0)) and _same(pts[1], V(7, 0, 0))
    e._age_references(1.0)
    assert len(e._recent_points) == 6               # still fresh after a second
    e._age_references(1.0)
    assert e._recent_points == []                   # gone after two
    e.note_reference(V(1, 1, 1), "midpoint")
    (p, kind), = e._recent_references
    assert _same(p, V(1, 1, 1)) and kind == "midpoint"
    e.clear_references()
    assert e._recent_points == []


# ---- the view ---------------------------------------------------------------

def _scene(view, extra=()):
    _box(view.scene.mesh, 2, 1.5, 0.5, 1, 1, 0.6)        # the reference box
    _box(view.scene.mesh, 3.6, 0.6, 0.0, 0.3, 0.3, 0.3)   # OTHER's box
    for b in extra:
        _box(view.scene.mesh, *b)
    view._host.resize(1000, 700, 1.)
    cam = view.camera
    cam.aspect = 1000 / 700
    cam.target = V(1.5, 1, 0.5)
    cam.distance = 9
    cam.yaw = math.radians(-30)
    cam.pitch = math.radians(30)
    refresh(view)


def _move(view, p, mods=0, dx=0.0, dy=0.0):
    x, y = view._world_to_pixel(p)
    mouse(view, "move", x + dx, y + dy, modifiers=mods)
    return view.last_snap


def _engine(view):
    return view.source_viewport.inference


def _pick_up(view, *points):
    for p in points:
        assert _move(view, p).kind in ("endpoint", "midpoint"), p


def test_a_corner_picked_up_lines_the_end_up_after_crossing_another(view):
    _scene(view)
    click(view, V(0, 0, 0))
    assert _move(view, V(1.5, 0, 0)).kind in ("axis", "axis_inference")
    _pick_up(view, CORNER, OTHER)
    # OTHER is the acquired point now; CORNER is still remembered.
    assert _same(_engine(view)._acquired_point, OTHER)
    snap = _move(view, FOOT, dx=2, dy=-2)
    assert snap.kind == "from_point", (snap.kind, snap.point)
    assert _same(snap.point, FOOT)
    assert _same(snap.guide[0], CORNER, 1e-6) and _same(snap.guide[1], FOOT)
    assert tuple(snap.guide_color) == AXIS_COLORS["x"]
    assert snap.source_kind == "endpoint"


def test_a_picked_up_corner_is_forgotten_after_a_while(view):
    _scene(view)
    click(view, V(0, 0, 0))
    _pick_up(view, CORNER, OTHER)
    _engine(view)._age_references(2.0)
    snap = _move(view, FOOT, dx=2, dy=-2)
    assert not (snap.kind == "from_point" and _same(snap.guide[0], CORNER, 1e-6))


def test_a_midpoint_is_picked_up_too(view):
    _scene(view)
    click(view, V(0, 0, 0))
    mid = V(2.5, 1.5, 1.1)
    _pick_up(view, mid, OTHER)
    snap = _move(view, V(2.5, 0, 0), dx=2, dy=-2)
    assert snap.kind == "from_point", (snap.kind, snap.point)
    assert _same(snap.point, V(2.5, 0, 0))
    assert snap.source_kind == "midpoint"


def test_before_the_first_click_a_hovered_point_lines_nothing_up(view):
    """Remembered points serve an operation's line only: before the first
    click the cursor near a corner's axis line stays where it is."""
    _scene(view)
    assert _move(view, CORNER).kind == "endpoint"
    snap = _move(view, V(3, 0.2, 1.1), dx=2, dy=2)      # on green through it
    assert snap.kind not in ("from_point", "aligned"), (snap.kind, snap.point)


def test_an_arrow_lock_takes_a_midpoint_and_says_from_point(view):
    _scene(view)
    click(view, V(0, 0, 0))
    _engine(view).axis_lock = "x"
    _move(view, V(1, 0, 0))
    snap = _move(view, V(2.5, 1.5, 1.1))
    assert snap.kind == "from_point", (snap.kind, snap.point)
    assert _same(snap.point, V(2.5, 0, 0))
    assert snap.source_kind == "midpoint"


def test_an_arrow_lock_keeps_a_foot_hidden_behind_geometry(view):
    from built_core import occluded
    _scene(view)
    foot = V(2.5, 0, 0)
    origin, ray = view._pixel_to_ray(*view._world_to_pixel(foot))
    q = foot - ray.normalized() * 1.0                 # a block between the eye and the foot
    _box(view.scene.mesh, q.x() - 0.2, q.y() - 0.2, q.z() - 0.2, 0.4, 0.4, 0.4)
    view.inference_index = None
    refresh(view)
    assert occluded(view.inference_index, view, foot)
    click(view, V(0, 0, 0))
    _engine(view).axis_lock = "x"
    _move(view, V(1, 0, 0))
    snap = _move(view, V(2.5, 1.5, 1.1))
    assert snap.kind == "from_point", (snap.kind, snap.point)
    assert _same(snap.point, foot)


def test_a_shift_lock_takes_the_corner(view):
    _scene(view)
    click(view, V(0, 0, 0))
    assert _move(view, V(1.5, 0, 0)).kind in ("axis", "axis_inference")
    key(view, True, Qt.Key_Shift, Qt.ShiftModifier)
    snap = _move(view, CORNER, mods=Qt.ShiftModifier)
    key(view, False, Qt.Key_Shift)
    assert snap.kind == "from_point", (snap.kind, snap.point)
    assert _same(snap.point, FOOT)


def test_the_cue_marks_the_source_point():
    from views.wgpu_overlay import draw_snap_indicator
    r = _locked(V(2, 1.5, 0.8), V(0, 0, 0), axis="x")
    calls = []

    class Painter:
        def draw_snap(self, kind, at, rgb, guides, label):
            calls.append((kind, tuple(at), list(guides), label))

    draw_snap_indicator(Painter(), r, _w2p, lambda a, b: (a, b), labels=False)
    kinds = [c[0] for c in calls]
    assert kinds == ["endpoint", "from_point"], kinds
    assert calls[0][1] == pytest.approx(_w2p(V(2, 1.5, 0.8)))   # the source
    (a, b, colour), = calls[1][2]                              # the guide
    assert tuple(colour) == pytest.approx(AXIS_COLORS["x"])


# ---- every tool with a constrained direction --------------------------------

def _back_to_the_foot(view):
    _move(view, V(1.0, 0, 0))
    _pick_up(view, CORNER, OTHER)
    return _move(view, FOOT, dx=2, dy=-2)


@pytest.mark.parametrize("tool", ["line", "move", "copy", "tape"])
def test_each_drawing_tool_lines_up_with_a_picked_up_corner(view, tool):
    _scene(view, extra=[(-0.2, -0.2, 0.0, 0.2, 0.2, 0.2)])  # something to move
    if tool in ("move", "copy"):
        use(view, MoveTool())
        if tool == "copy":
            tap(view, Qt.Key_Control, Qt.ControlModifier)
    elif tool == "tape":
        use(view, TapeMeasureTool())
    click(view, V(0, 0, 0))
    assert _same(view.tools.start_point, V(0, 0, 0))
    snap = _back_to_the_foot(view)
    assert snap.kind == "from_point", (tool, snap.kind, snap.point)
    assert _same(snap.point, FOOT)


def test_a_rectangle_corner_lines_up_with_a_picked_up_corner(view):
    _scene(view)
    use(view, RectangleTool())
    click(view, V(0, -1, 0))
    _move(view, V(1.0, -2, 0))
    _pick_up(view, CORNER, OTHER)
    snap = _move(view, V(3.0, -2.0, 0.0), dx=2, dy=1)
    assert snap.kind == "aligned", (snap.kind, snap.point)
    assert snap.point.x() == pytest.approx(3.0, abs=1e-6)
    assert snap.point.z() == pytest.approx(0.0, abs=1e-6)


@pytest.mark.parametrize("target,kind", [(CORNER, "vertex"), (V(2.5, 1.5, 1.1), "midpoint")])
def test_push_pull_lines_the_face_up_with_a_hovered_point(view, target, kind):
    from tools.pushpull import PushPullTool
    _scene(view)
    view.scene.mesh.add_face([V(0, -2, 0), V(1, -2, 0), V(1, -1, 0), V(0, -1, 0)])
    refresh(view)
    tool = PushPullTool()
    use(view, tool)
    _move(view, V(0.5, -1.5, 0))
    click(view, V(0.5, -1.5, 0))
    assert tool.dragging
    _move(view, target)
    assert tool._inference_kind == kind
    assert tool.extrusion == pytest.approx(1.1, abs=1e-6)


# ---- a million faces --------------------------------------------------------

def _sheet(n, size):
    i, j = np.meshgrid(np.arange(n), np.arange(n), indexing="ij")
    i, j = i.ravel(), j.ravel()
    d = size / n
    corners = np.stack([np.stack([(i + a) * d, (j + b) * d, np.zeros(i.shape)], -1)
                        for a, b in ((0, 0), (1, 0), (1, 1), (0, 1))], 1)
    m = Mesh()
    m.add_faces_bulk(corners.reshape(-1, 3), np.full(n * n, 4), np.ones(n * n, int))
    return m


def test_a_hover_stays_cheap_on_a_million_faces(view):
    m = _sheet(1000, 10.0)
    assert len(m.faces) == 1_000_000
    g = Group(m, name="floor")
    xf = QMatrix4x4()
    xf.translate(-5.0, 2.0, 0.0)
    g.xform = xf
    view.scene.groups.append(g)
    _scene(view)
    cam = view.camera
    cam.target = V(1.5, 3, 0.5)
    cam.distance = 12
    refresh(view)
    click(view, V(0, 0, 0))

    def timed(points):
        spent = []
        for p in points:
            x, y = view._world_to_pixel(p)
            t = time.perf_counter()
            mouse(view, "move", x, y)
            spent.append(time.perf_counter() - t)
        return 1000 * sorted(spent)[len(spent) // 2]

    # On red, with corners picked up on the dense floor and off it.
    free = timed([CORNER if k % 3 == 0 else V(-4.0 + 0.13 * k, 2.0 + 0.11 * k, 0)
                  if k % 3 == 1 else V(0.2 * k, 0, 0) for k in range(60)])
    assert view.last_snap is not None
    _engine(view).axis_lock = "x"
    locked = timed([V(-4.0 + 0.13 * k, 2.0 + 0.11 * k, 0) for k in range(60)])
    assert view.last_snap.kind == "from_point"
    assert free < 2.0 and locked < 2.0, (free, locked)

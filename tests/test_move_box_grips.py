# SPDX-License-Identifier: GPL-3.0-or-later
"""Bounding-box grips on groups and components, Move and Rotate (.skp parity).

Hovering a group with Move or Rotate shows its box, in the box's own axes,
with grips: the 8 corners by default; Alt (Option on the Mac; ⌘ is Move's
copy key here) cycles corners → edge midpoints → side centres → centre while
the object is selected and no grip is under the cursor. A click on a grip
picks the object up by that very point (Move) or puts the rotation centre or
start point on it (Rotate). Move's red rotation handles show on the sides
facing the view within a 150° cone, step aside near box grips, and a
preference turns them off.
"""
from __future__ import annotations

import math

import pytest
from built_core import Input, Matrix4 as QMatrix4x4, ScreenPoint
from built_core import Vec3 as QVector3D

from built_core.group import Group
from built_core.history import History
from built_core.mesh import Mesh
from built_core.scene import Scene
from tests.controller_surface import ControllerSurface
from tools.base import ToolContext
from tools.move import MoveTool
from tools.rotate import RotateTool


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


def _box_mesh(sx=4.0, sy=1.0, sz=1.0):
    m = Mesh()
    p = [V(0, 0, 0), V(sx, 0, 0), V(sx, sy, 0), V(0, sy, 0),
         V(0, 0, sz), V(sx, 0, sz), V(sx, sy, sz), V(0, sy, sz)]
    for idx in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5),
                (2, 3, 7, 6), (3, 0, 4, 7)):
        m.add_face([p[i] for i in idx])
    return m


def _box(scene, sx=4.0, sy=1.0, sz=1.0, at=(0, 0, 0), name="Bench"):
    g = Group(_box_mesh(sx, sy, sz), name=name)
    if any(at):
        x = QMatrix4x4()
        x.translate(V(*at))
        g.xform = x
    scene.groups.append(g)
    return g


class _Cam:
    def __init__(self, eye):
        self._eye = eye

    def eye(self):
        return self._eye


class _Vp:
    """A top-down orthographic view (1 m = 100 px, y flipped) whose boxes
    are the real ones (``_compute_obb``: the placement's own axes)."""

    def __init__(self, scene, eye=V(2, 0.5, 50)):
        self.scene = scene
        self.history = History(scene)
        self.camera = _Cam(eye)
        self.flashed = []
        self.hidden = set()

    def update(self):
        pass

    def flash_status(self, msg, *a, **k):
        self.flashed.append(msg)

    def pick_group(self, x, y):
        wx, wy = x / 100.0, -y / 100.0
        for g in self.scene.groups:
            (u, v, w), lo, hi = self._group_obb(g)
            if lo[0] - 1e-9 <= wx <= hi[0] + 1e-9 and lo[1] - 1e-9 <= wy <= hi[1] + 1e-9:
                return g
        return None

    def _group_obb(self, g):
        return ControllerSurface._compute_obb(g)

    def _world_to_pixel(self, p):
        return (p.x() * 100.0, -p.y() * 100.0)

    def _occluded_on_screen(self, p):
        return (round(p.x(), 3), round(p.y(), 3), round(p.z(), 3)) in self.hidden

    def pick_edge(self, x, y):
        return None

    def pick_face(self, x, y):
        return None


def _ctx(vp, wx, wy, wz=1.0, mods=Input.NoModifier):
    return ToolContext(viewport=vp, world=V(wx, wy, wz),
                       screen=ScreenPoint(wx * 100.0, -wy * 100.0),
                       modifiers=mods, snap=None)


def _corners(g):
    from built_core.group import world_mesh
    return sorted((round(p.x(), 4), round(p.y(), 4), round(p.z(), 4))
                  for p in (v.position for v in world_mesh(g).vertices))


def _pts(points):
    return sorted((round(p.x(), 4), round(p.y(), 4), round(p.z(), 4)) for p in points)


def _setup(tool_class=MoveTool, **box):
    scene = Scene()
    g = _box(scene, **box)
    vp = _Vp(scene)
    tool = tool_class()
    tool.on_activate(vp)
    return scene, g, vp, tool


# ---- positions ----------------------------------------------------------------

def test_hover_shows_the_eight_corners_of_the_box():
    scene, g, vp, tool = _setup()
    tool.on_hover(_ctx(vp, 2.0, 0.5))
    assert tool._grip_group is g
    assert _pts(tool._box_grips) == _pts(
        V(x, y, z) for x in (0, 4) for y in (0, 1) for z in (0, 1))
    assert tool._hot_box is None


def test_grips_of_a_turned_scaled_component_sit_on_its_own_box():
    from built_core import box_grip_points
    scene = Scene()
    g = Group(_box_mesh(1, 1, 1), name="Turned")
    x = QMatrix4x4()
    x.translate(V(5, 5, 0))
    x.rotate(30.0, V(0, 0, 1))
    x.scale(2.0, 3.0, 1.0)
    g.xform = x
    scene.groups.append(g)
    obb = ControllerSurface._compute_obb(g)
    grips = _pts(box_grip_points(obb, 0))
    # The placed corners themselves, not the model-axis box round them.
    assert grips == _corners(g)
    centre = box_grip_points(obb, 3)[0]
    c, s = math.cos(math.radians(30)), math.sin(math.radians(30))
    assert (centre.x(), centre.y(), centre.z()) == pytest.approx(
        (5 + c * 1.0 - s * 1.5, 5 + s * 1.0 + c * 1.5, 0.5))


def test_grips_of_a_nested_group_include_its_children():
    from built_core import box_grip_points
    scene = Scene()
    leaf = Group(_box_mesh(1, 1, 1), name="Leaf")
    x = QMatrix4x4()
    x.translate(V(3, 0, 0))
    leaf.xform = x
    outer = Group(_box_mesh(1, 1, 1), name="Outer")
    outer.children = [leaf]
    y = QMatrix4x4()
    y.translate(V(0, 2, 0))
    outer.xform = y
    scene.groups.append(outer)
    grips = _pts(box_grip_points(ControllerSurface._compute_obb(outer), 0))
    assert (0.0, 2.0, 0.0) in grips and (4.0, 3.0, 1.0) in grips
    assert len(grips) == 8


def test_alt_cycles_the_sets_on_a_selected_object():
    scene, g, vp, tool = _setup()
    tool.on_hover(_ctx(vp, 2.0, 0.5))
    # Not selected: Alt does nothing here.
    assert not tool.on_key(vp, Input.Key_Alt, Input.NoModifier)
    scene.selection.add(g)
    tool.on_hover(_ctx(vp, 2.0, 0.5))
    counts = []
    for _ in range(4):
        assert tool.on_key(vp, Input.Key_Alt, Input.NoModifier)
        counts.append(len(tool._box_grips))
    assert counts == [12, 6, 1, 8]
    assert tool.on_key(vp, Input.Key_Alt, Input.NoModifier)
    assert _pts(tool._box_grips)[0] == (0.0, 0.0, 0.5)   # an edge midpoint
    # The set stays for the next hover; Ctrl is still the copy key.
    tool.on_hover(_ctx(vp, 2.0, 0.6))
    assert len(tool._box_grips) == 12
    assert tool.on_key(vp, Input.Key_Control, Input.NoModifier)
    assert tool._copy


def test_alt_with_a_grip_under_the_cursor_does_not_cycle():
    scene, g, vp, tool = _setup()
    scene.selection.add(g)
    tool.on_hover(_ctx(vp, 4.0, 1.0))
    assert tool._hot_box is not None
    assert not tool.on_key(vp, Input.Key_Alt, Input.NoModifier)
    assert len(tool._box_grips) == 8


def test_a_selection_of_groups_has_one_union_box():
    scene = Scene()
    a = _box(scene, 1, 1, 1)
    b = _box(scene, 1, 1, 2, at=(3, 2, 0), name="B")
    vp = _Vp(scene, eye=V(2, 1.5, 50))
    tool = MoveTool()
    tool.on_activate(vp)
    scene.selection.add(a)
    scene.selection.add(b)
    tool.on_hover(_ctx(vp, 0.5, 0.5))
    assert set(tool._grip_groups) == {a, b}
    assert _pts(tool._box_grips) == _pts(
        V(x, y, z) for x in (0, 4) for y in (0, 3) for z in (0, 2))
    # Something else selected as well: no grips.
    scene.selection.add(scene.mesh.add_face([V(9, 9), V(10, 9), V(10, 10)]))
    tool.on_hover(_ctx(vp, 0.5, 0.5))
    assert tool._box_grips == [] and tool._grip_group is None


def test_hidden_grips_are_dimmed_in_the_overlay_but_still_taken():
    from built_core import tool_overlay_marks
    scene, g, vp, tool = _setup()
    vp.hidden = {(4.0, 1.0, 0.0)}           # the bottom corner, under the box
    tool.on_hover(_ctx(vp, 4.0, 1.0))
    assert tool._hot_box is not None
    marks = tool_overlay_marks("move", tool, vp)
    rects = [m for m in marks if m[0] == "rect"]
    assert len(rects) == 8
    assert any(m[2] & 0xff == 0x66 for m in rects)          # dimmed
    assert sum(1 for m in marks if m[0] == "line") >= 12    # the box


# ---- Move ----------------------------------------------------------------------

def test_a_corner_click_picks_the_object_up_by_that_corner():
    scene, g, vp, tool = _setup()
    tool.on_hover(_ctx(vp, 3.9, 0.9))         # on the object: its grips
    tool.on_hover(_ctx(vp, 4.05, 1.03))       # beside the corner, in the air
    assert tool._hot_box is not None
    tool.on_click(_ctx(vp, 4.05, 1.03, wz=7.0))
    assert tool.start_point is not None
    grab = tool.start_point
    # Exactly on the corner, whatever the snap found under the cursor.
    assert (round(grab.x(), 6), round(grab.y(), 6)) == (4.0, 1.0)
    assert tool._groups == [g]
    tool.on_click(_ctx(vp, 10.0, 1.0, wz=grab.z()))
    assert min(c[0] for c in _corners(g)) == pytest.approx(6.0)
    vp.history.undo()
    assert min(c[0] for c in _corners(g)) == pytest.approx(0.0)


def test_away_from_grips_the_geometry_is_grabbed_as_before():
    scene, g, vp, tool = _setup()
    scene.selection.add(g)
    tool.on_hover(_ctx(vp, 0.5, 0.5))
    assert tool._hot_box is None and tool._hot_grip is None
    tool.on_click(_ctx(vp, 0.5, 0.5))
    assert (tool.start_point.x(), tool.start_point.y()) == (0.5, 0.5)
    tool.on_click(_ctx(vp, 0.5, 2.5))
    assert min(c[1] for c in _corners(g)) == pytest.approx(2.0)


def test_a_selection_moves_together_from_a_grip_of_its_box():
    scene = Scene()
    a = _box(scene, 1, 1, 1)
    b = _box(scene, 1, 1, 1, at=(3, 0, 0), name="B")
    vp = _Vp(scene, eye=V(2, 0.5, 50))
    tool = MoveTool()
    tool.on_activate(vp)
    scene.selection.add(a)
    scene.selection.add(b)
    tool.on_hover(_ctx(vp, 4.0, 1.0))
    assert tool._hot_box is not None
    tool.on_click(_ctx(vp, 4.0, 1.0))
    tool.on_click(_ctx(vp, 4.0, 3.0))
    assert min(c[1] for c in _corners(a)) == pytest.approx(2.0)
    assert min(c[1] for c in _corners(b)) == pytest.approx(2.0)


def test_a_grip_copy_and_array_still_work():
    scene, g, vp, tool = _setup()
    tool.on_hover(_ctx(vp, 4.0, 1.0))
    tool.on_click(_ctx(vp, 4.0, 1.0))
    tool._toggle_copy(vp)
    tool.on_click(_ctx(vp, 9.0, 1.0))
    assert len(scene.groups) == 2
    assert tool.on_array_value(vp, 3, "x")
    assert len(scene.groups) == 4


def test_esc_after_a_grip_pick_puts_it_back():
    scene, g, vp, tool = _setup()
    before = _corners(g)
    tool.on_hover(_ctx(vp, 4.0, 1.0))
    tool.on_click(_ctx(vp, 4.0, 1.0))
    tool.on_hover(_ctx(vp, 7.0, 3.0))
    tool.on_cancel(vp)
    assert tool.start_point is None
    assert _corners(g) == before


# ---- red rotation handles ----------------------------------------------------

def test_rotation_handles_step_aside_near_box_grips():
    # A small box on screen: its handles crowd its corners and give way.
    scene, g, vp, tool = _setup(sx=0.12, sy=0.12, sz=0.12)
    tool.on_hover(_ctx(vp, 0.06, 0.06))
    assert tool._box_grips
    assert tool._grips == []
    scene2, g2, vp2, tool2 = _setup()
    tool2.on_hover(_ctx(vp2, 2.0, 0.5))
    assert len(tool2._grips) == 4


def test_box_grip_wins_over_a_handle_at_the_same_spot():
    scene, g, vp, tool = _setup()
    tool.on_hover(_ctx(vp, 4.0, 1.0))
    assert tool._hot_box is not None and tool._hot_grip is None


def test_the_preference_turns_the_rotation_handles_off(monkeypatch):
    from built_core import settings as prefs
    st = prefs.settings()
    st.setValue("modelling/move_rotation_grips", "0")
    try:
        scene, g, vp, tool = _setup()
        tool.on_hover(_ctx(vp, 2.0, 0.5))
        assert tool._grips == []
        assert len(tool._box_grips) == 8      # the box grips stay
    finally:
        st.setValue("modelling/move_rotation_grips", "1")
    scene, g, vp, tool = _setup()
    tool.on_hover(_ctx(vp, 2.0, 0.5))
    assert len(tool._grips) == 4


def test_handles_follow_the_view_cone():
    # Looking down steeply: the long front side faces the eye at ~80°, out
    # of the 150° cone, so only the top keeps its handles.
    scene = Scene()
    g = _box(scene)
    eye = V(2, 0.5 - 50 * math.tan(math.radians(10)), 50.5)
    vp = _Vp(scene, eye=eye)
    tool = MoveTool()
    tool.on_activate(vp)
    tool.on_hover(_ctx(vp, 2.0, 0.5))
    assert {round(n.z()) for _p, _c, n in tool._grips} == {1}


# ---- Rotate --------------------------------------------------------------------

def test_rotate_shows_the_same_grips_and_cycles():
    scene, g, vp, tool = _setup(RotateTool)
    tool.on_hover(_ctx(vp, 2.0, 0.5))
    assert len(tool._box_grips) == 8 and tool._grips == []
    scene.selection.add(g)
    tool.on_hover(_ctx(vp, 2.0, 0.5))
    assert tool.on_key(vp, Input.Key_Alt, Input.NoModifier)
    assert len(tool._box_grips) == 12


def test_rotate_takes_its_centre_and_start_on_grips():
    scene, g, vp, tool = _setup(RotateTool)
    tool.on_hover(_ctx(vp, 0.02, 0.98))       # the far-left corner, in the air
    assert tool._hot_box is not None
    tool.on_click(_ctx(vp, 0.02, 0.98, wz=3.0))
    centre = tool.start_point
    assert centre is not None
    assert (round(centre.x(), 6), round(centre.y(), 6)) == (0.0, 1.0)
    tool.on_hover(_ctx(vp, 4.0, 1.0))
    assert tool._hot_box is not None
    tool.on_click(_ctx(vp, 4.0, 1.0, wz=centre.z()))
    ref = tool.ref_point
    assert (round(ref.x(), 6), round(ref.y(), 6)) == (4.0, 1.0)
    # The swing has begun: no grips while it turns.
    tool.on_hover(_ctx(vp, 0.0, 5.0, wz=centre.z()))
    assert tool._box_grips == []
    assert tool.on_value(vp, 90.0)
    xs = [c[0] for c in _corners(g)]
    assert (min(xs), max(xs)) == pytest.approx((0.0, 1.0), abs=1e-6)

# SPDX-License-Identifier: GPL-3.0-or-later
"""Every inference snap the GL view offers, on the Metal view too.

The GL viewport offered four snaps the Metal hover never produced, and two
cues it never ran:

* face meets face with no edge between them (#819, «Intersection» along
  the line where a box sinks into a floor) — GL's
  ``Viewport._face_crossing_point``; Metal handed no ``face_crossing``;
* an edge piercing the face under the cursor (``built_core.snap._edge_face_snap``)
  — Metal handed no ``face_outline``;
* the «Arc midpoint» of an open arc — GL's ``Viewport._arc_midpoints``;
  the Metal snap index fitted the arcs' centres only;
* the extension snap providers (``ExtensionApp.add_snap_provider``) and the
  axis-source cue (the square on a model axis a Line or the Tape starts
  from) — ``InferenceEngine.finish_snap`` ran them with ``cues=False``;
* a planar shape re-planning onto the face under the cursor (#545) —
  Metal's ``work_point`` kept the plane the first click captured.

Each case drives an unshown ``WgpuView`` with the real tools (the fixture of
``test_inference_and_face_cutting``: no renderer, no window, no settings) and
asks the GL viewport behind it the same pixel: both must answer the same
kind, point and ScreenTip context — on loose geometry, in groups and
components, from inside an open group and on a linked file's geometry.
"""
import math

import numpy as np
import pytest
from built_core import Input
from built_core import Matrix4 as QMatrix4x4, Vec3 as V

from built_core.group import Group
from built_core.mesh import Mesh
from built_core.snap import SnapResult
from tests.headless_gpu import mouse
from tests.test_inference_and_face_cutting import (  # noqa: F401
    click, move, refresh, use, view)
from tools.rust_tools import RectangleTool
from tools.tape import TapeMeasureTool


# ---- scenes -----------------------------------------------------------------

_QUADS = ([0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5],
          [2, 3, 7, 6], [3, 0, 4, 7])


def _box(mesh, lo, hi):
    (x0, y0, z0), (x1, y1, z1) = lo, hi
    p = [V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0), V(x0, y1, z0),
         V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)]
    for q in _QUADS:
        mesh.add_face([p[i] for i in q])


def _floor(mesh, z=0.0, s=5.0):
    mesh.add_face([V(-s, -s, z), V(s, -s, z), V(s, s, z), V(-s, s, z)])


def _component(mesh, dx=0.0, dy=0.0, dz=0.0, name=None):
    g = Group(mesh, name)
    m = QMatrix4x4()
    m.translate(dx, dy, dz)
    g.xform = m
    g.component = True
    return g


def _linked(group):
    group.ext = {"link": {"path": "/nonexistent/source.igz"}}
    return group


def _arc(mesh, cx, cy, z=0.0, r=1.0, curve=7):
    pts = [V(cx + r * math.cos(math.pi * k / 8), cy + r * math.sin(math.pi * k / 8), z)
           for k in range(9)]
    for a, b in zip(pts, pts[1:]):
        mesh.add_edge(a, b).curve = curve


def _look(view, target, distance=9.0, pitch=30.0, yaw=-60.0):
    cam = view.camera
    cam.target = target
    cam.distance = distance
    cam.pitch = math.radians(pitch)
    cam.yaw = math.radians(yaw)


def _open(view, group):
    view.source_viewport.begin_group_edit(group)
    view.inference_index = None
    refresh(view)


def _both(view, world, dx=0.0, dy=0.0, modifiers=Input.NoModifier):
    """The Metal hover's snap and GL's InferenceEngine.snap at one pixel."""
    x, y = view._world_to_pixel(world)
    mouse(view, "move", x + dx, y + dy, modifiers=modifiers)
    metal = view.last_snap
    gl = view.source_viewport.inference.snap(x + dx, y + dy, modifiers)
    return metal, gl


def _same(metal, gl, kind, near=None, tol=1e-4):
    """Both views give ``kind`` at the same point with the same tip."""
    assert metal is not None and gl is not None
    assert gl.kind == kind, ("GL", gl.kind, gl.point)
    assert metal.kind == kind, ("Metal", metal.kind, metal.point, "GL", gl.point)
    assert (metal.point - gl.point).length() <= tol, (metal.point, gl.point)
    assert metal.context == gl.context, (metal.context, gl.context)
    assert tuple(metal.color) == tuple(gl.color)
    if near is not None:
        assert (metal.point - near).length() <= 0.05, metal.point


# ---- 1. face meets face (#819) ----------------------------------------------

def _sunk_box(view, where):
    """A floor and a box sunk through it, with no edge where they meet.
    Returns the group to open for ``where == 'inside open group'``."""
    scene = view.scene
    floor, box = Mesh(), Mesh()
    _floor(floor)
    _box(box, (0, 0, -0.5), (1, 1, 1.5))
    opened = None
    if where == "loose":
        _floor(scene.mesh)
        _box(scene.mesh, (0, 0, -0.5), (1, 1, 1.5))
    elif where == "group":
        _floor(scene.mesh)
        scene.groups.append(Group(box, "box"))
    elif where == "sibling components":
        scene.groups.append(_component(floor, name="floor"))
        scene.groups.append(_component(box, name="box"))
    elif where == "linked":
        _floor(scene.mesh)
        scene.groups.append(_linked(_component(box, name="linked box")))
    elif where == "inside open group":
        opened = Group(floor, "floor")
        scene.groups.append(opened)
        scene.groups.append(_component(box, name="box"))
    _look(view, V(0.5, 0.5, 0.3), distance=5.0, yaw=-60.0)
    refresh(view)
    if opened is not None:
        _open(view, opened)


WHERE = ["loose", "group", "sibling components", "linked", "inside open group"]


@pytest.mark.parametrize("where", WHERE)
@pytest.mark.parametrize("on_line", [V(0.5, 0.0, 0.0), V(1.0, 0.4, 0.0)])
def test_face_crossing_is_offered_on_metal(view, where, on_line):
    _sunk_box(view, where)
    for d in (0.0, 2.0, 4.0):
        metal, gl = _both(view, on_line, d, d)
        _same(metal, gl, "intersection")
        assert abs(metal.point.z()) < 1e-4            # on the floor's plane
        assert (abs(metal.point.y()) < 1e-4 or abs(metal.point.x() - 1) < 1e-4)


def test_face_crossing_is_not_offered_where_the_faces_do_not_meet(view):
    """A box standing ON the floor (its bottom on it) crosses nothing: the
    seam is the box's own edge; away from it the face stays «On face»."""
    scene = view.scene
    _floor(scene.mesh)
    box = Mesh()
    _box(box, (0, 0, 0.5), (1, 1, 1.5))               # floating above it
    scene.groups.append(Group(box))
    _look(view, V(0.5, 0.5, 0.3), distance=5.0)
    refresh(view)
    metal, gl = _both(view, V(-1.5, -1.5, 0.0))
    _same(metal, gl, "on_face")


# ---- 2. an edge piercing a face --------------------------------------------

def _pierced(view, where):
    """A face, and a line through it whose crossing is not on any edge.
    Returns the crossing."""
    scene = view.scene
    a, b = V(1, 1, -1), V(1.2, 1.1, 1)
    crossing = a + (b - a) * 0.5
    slab = Mesh()
    _floor(slab)
    if where == "loose":
        _floor(scene.mesh)
        scene.mesh.add_edge(a, b)
    elif where == "group":
        scene.groups.append(Group(slab, "slab"))
        scene.mesh.add_edge(a, b)
    elif where == "sibling components":
        line = Mesh()
        line.add_edge(a, b)
        scene.groups.append(_component(slab, name="slab"))
        scene.groups.append(_component(line, name="post"))
    elif where == "linked":
        scene.groups.append(_linked(_component(slab, name="linked slab")))
        scene.mesh.add_edge(a, b)
    elif where == "inside open group":
        line = Mesh()
        line.add_edge(a, b)
        opened = Group(line, "post")
        _floor(scene.mesh)
        scene.groups.append(opened)
    _look(view, V(1, 1, 0), distance=8.0, pitch=35.0, yaw=-50.0)
    refresh(view)
    if where == "inside open group":
        _open(view, opened)
    return crossing


@pytest.mark.parametrize("where", WHERE)
def test_an_edge_piercing_a_face_is_offered_on_metal(view, where):
    crossing = _pierced(view, where)
    for d in (0.0, 3.0, 6.0):
        metal, gl = _both(view, crossing, d, -d)
        _same(metal, gl, "intersection", near=crossing)


def test_a_line_through_a_hole_pierces_nothing(view):
    """Inside a hole the ray meets no triangle of the face: no crossing,
    as GL's loop test says."""
    m = view.scene.mesh
    s, h = 5.0, 0.05
    m.add_face([V(-s, -s, 0), V(s, -s, 0), V(s, s, 0), V(-s, s, 0)],
               [[V(1 - h, 1 - h, 0), V(1 + h, 1 - h, 0), V(1 + h, 1 + h, 0),
                 V(1 - h, 1 + h, 0)]])
    assert any(f.holes for f in m.faces)
    m.add_edge(V(1.0, 1.0, -1.0), V(1.0, 1.0, 3.0))   # crosses at (1, 1, 0)
    _look(view, V(1, 1, 0), distance=5.0, pitch=60.0, yaw=-50.0)
    refresh(view)
    crossing = V(1, 1, 0)
    beside = V(1 + h + 0.02, 1.0, 0.0)                # on the face, past the rim
    x, y = view._world_to_pixel(crossing)
    bx, by = view._world_to_pixel(beside)
    assert math.hypot(bx - x, by - y) < 12.0          # within the edge reach
    metal, gl = _both(view, beside)
    assert gl.kind != "intersection"
    _same(metal, gl, gl.kind)                         # a hole corner, here
    # …and the same line through the face itself (no hole) does pierce it.
    m2 = Mesh()
    _floor(m2)
    view.scene.groups.append(_component(m2, 0, 0, 1.0))
    view.scene.mesh.add_edge(V(-2.0, -2.0, 0.5), V(-2.0, -2.0, 2.0))
    refresh(view)
    metal, gl = _both(view, V(-2.0, -2.0, 1.0), 4.0, 0.0)
    _same(metal, gl, "intersection", near=V(-2.0, -2.0, 1.0))


# ---- 3. arc midpoint --------------------------------------------------------

def _arcs(view, where):
    """An open half-circle arc; returns its sweep midpoint."""
    scene = view.scene
    arc = Mesh()
    _arc(arc, 0, 0)
    if where == "loose":
        _arc(scene.mesh, 0, 0)
        mid = V(0, 1, 0)
    elif where == "group":
        g = Mesh()
        _arc(g, 2, 0, z=0.5)
        scene.groups.append(Group(g, "arc"))
        mid = V(2, 1, 0.5)
    elif where == "sibling components":
        scene.groups.append(_component(arc, 2, 0, 0.5, name="arc A"))
        scene.groups.append(_component(arc, -2, 0, 0.5, name="arc B"))
        mid = V(-2, 1, 0.5)
    elif where == "linked":
        scene.groups.append(_linked(_component(arc, 1, -1, 0.25, name="linked arc")))
        mid = V(1, 0, 0.25)
    elif where == "inside open group":
        # The arc is outside the open group: «outside» on both views.
        _arc(scene.mesh, 0, 0)
        other = Mesh()
        other.add_edge(V(3, 3, 0), V(4, 3, 0))
        opened = Group(other, "other")
        scene.groups.append(opened)
        mid = V(0, 1, 0)
    _look(view, V(0, 0.5, 0), distance=8.0, pitch=50.0, yaw=-70.0)
    refresh(view)
    if where == "inside open group":
        _open(view, opened)
    return mid


@pytest.mark.parametrize("where", WHERE)
def test_arc_midpoint_is_offered_on_metal(view, where):
    mid = _arcs(view, where)
    for d in (0.0, 3.0, 5.0):
        metal, gl = _both(view, mid, d, d)
        _same(metal, gl, "arc_midpoint", near=mid, tol=1e-5)
    if where == "inside open group":
        assert metal.context == "outside"


def test_a_closed_circle_has_no_arc_midpoint(view):
    m = view.scene.mesh
    pts = [V(math.cos(2 * math.pi * k / 16), math.sin(2 * math.pi * k / 16), 0)
           for k in range(16)]
    for a, b in zip(pts, pts[1:] + pts[:1]):
        m.add_edge(a, b).curve = 3
    _look(view, V(0, 0, 0), distance=8.0, pitch=50.0)
    refresh(view)
    metal, gl = _both(view, V(-1, 0, 0), 3.0, 3.0)
    assert metal.kind != "arc_midpoint" and gl.kind != "arc_midpoint"


# ---- 4. extension snap providers --------------------------------------------

def _provider_scene(view):
    _floor(view.scene.mesh, s=2.0)
    _look(view, V(0, 0, 0), distance=10.0, pitch=40.0)
    refresh(view)


def test_extension_snap_providers_run_on_metal(view):
    _provider_scene(view)
    src = view.source_viewport
    seen = []

    def level(vp, snap, px, py):
        seen.append(vp)
        if snap.kind not in ("none", "on_face"):
            return None
        return SnapResult(V(snap.point.x(), snap.point.y(), 3.0), "reference",
                          (0.2, 0.4, 0.9), label="Level 1")

    src._ext_snap_providers.append(level)
    for world in (V(0.5, 0.5, 0.0), V(4.0, -4.0, 0.0)):   # on the face, off it
        metal, gl = _both(view, world)
        _same(metal, gl, "reference")
        assert metal.label == gl.label == "Level 1"
    # The provider is handed the document's viewport, as on GL.
    assert seen and all(vp is src for vp in seen)
    # A named point is never overridden.
    metal, gl = _both(view, V(2.0, 2.0, 0.0))
    _same(metal, gl, "endpoint")


def test_a_provider_that_raises_does_not_break_the_metal_hover(view):
    _provider_scene(view)

    def broken(*_a):
        raise ValueError("plugin bug")

    view.source_viewport._ext_snap_providers.append(broken)
    metal, gl = _both(view, V(0.5, 0.5, 0.0))
    _same(metal, gl, "on_face")


# ---- 5. axis-source cue -----------------------------------------------------

@pytest.mark.parametrize("tool", ["line", "tape"])
def test_axis_source_cue_on_metal(view, tool):
    view.scene.mesh.add_edge(V(-3, 2, 0), V(-3, 3, 0))
    _look(view, V(0, 0, 0), distance=14.0, pitch=40.0)
    refresh(view)
    if tool == "tape":
        use(view, TapeMeasureTool())
    metal, gl = _both(view, V(2.5, 0, 0), 2.0, 1.0)
    _same(metal, gl, "on_axis")
    assert metal.axis == gl.axis == "x"


def test_no_axis_source_cue_once_the_line_has_started(view):
    view.scene.mesh.add_edge(V(-3, 2, 0), V(-3, 3, 0))
    _look(view, V(0, 0, 0), distance=14.0, pitch=40.0)
    refresh(view)
    click(view, V(-3, 3, 0))
    metal, _gl = _both(view, V(2.5, 0, 0), 2.0, 1.0)
    assert metal.kind != "on_axis"


# ---- 6. planar re-plan onto the face under the cursor (#545) ---------------

def test_a_rectangle_replans_onto_the_face_under_the_cursor(view):
    _box(view.scene.mesh, (0, 0, 0), (2, 2, 2))
    _look(view, V(1, 1, 1), distance=9.0, pitch=30.0, yaw=-70.0)
    refresh(view)
    tool = RectangleTool()
    use(view, tool)
    click(view, V(0.5, 0.5, 2.0))                     # on the top face
    captured = tool.work_plane
    assert captured is not None and abs(captured[1].z()) > 0.999
    front = V(1.5, 0.0, 1.0)                          # the front face, y = 0
    snap = move(view, front)
    assert snap.kind == "on_face"
    metal_plane = tool.work_plane
    assert abs(metal_plane[1].y()) > 0.999, metal_plane[1]
    assert abs(metal_plane[0].y()) < 1e-4
    # GL's _current_work_plane re-plans the same way from the same capture.
    tool.work_plane = captured
    gl_plane = view.source_viewport._current_work_plane(view._world_to_pixel(front))
    assert abs(QVector3D_dot(gl_plane[1], metal_plane[1])) > 0.999
    assert abs(QVector3D_dot(gl_plane[0] - metal_plane[0], gl_plane[1])) < 1e-4


def test_a_line_keeps_its_plane(view):
    """Only a planar shape re-plans: a Line keeps its chain where it was."""
    from tools.rust_tools import LineTool
    _box(view.scene.mesh, (0, 0, 0), (2, 2, 2))
    _look(view, V(1, 1, 1), distance=9.0, pitch=30.0, yaw=-70.0)
    refresh(view)
    tool = LineTool()
    use(view, tool)
    tool.work_plane = (V(0, 0, 2), V(0, 0, 1))
    view.tools.work_point(*view._world_to_pixel(V(1.5, 0.0, 1.0)))
    assert abs(tool.work_plane[1].z()) > 0.999


def QVector3D_dot(a, b):
    return a.x() * b.x() + a.y() * b.y() + a.z() * b.z()


# ---- cost: the new work stays under the cursor ------------------------------

def test_face_rules_ask_only_the_placements_under_the_cursor(view, monkeypatch):
    """The face crossing and the pierce test ask the snap index for the
    placements whose screen box holds the cursor (or a probe beside it),
    never for the whole scene."""
    scene = view.scene
    proto = Mesh()
    _box(proto, (0, 0, 0), (0.6, 0.6, 0.6))
    for i in range(400):
        scene.groups.append(_component(proto, (i % 20) * 1.0, (i // 20) * 1.0, 0))
    _look(view, V(10, 10, 0), distance=12.0, pitch=45.0)
    refresh(view)
    index = view.inference_index
    asked = []
    original = type(index).ray_cands

    def spy(self, rows, *args):
        asked.append(len(np.asarray(rows).reshape(-1)))
        return original(self, rows, *args)

    monkeypatch.setattr(type(index), "ray_cands", spy)
    x, y = view._world_to_pixel(V(10.3, 10.3, 0.6))   # a box's top face
    mouse(view, "move", x, y)
    assert view.last_snap is not None
    # The test's own ID-pass stand-in (_pick) asks one row at a time; the
    # occlusion rays ask all rows through the native BVH. Neither is new.
    import built_core
    asked.clear()
    # Observe the rows that actually reach every BVH ray, including side probes.
    built_core.face_crossing(
        index, view, x, y, view.hovered_row, view.last_snap.point, V(0, 0, 1),
        built_core.eligible_rows(index, view.packed),
        view._world_to_pixel, view.scene.edit_group)
    assert asked and max(asked) <= 8, asked


def test_acquired_arc_center_survives_crossing_the_empty_interior(view):
    _arc(view.scene.mesh, 2., 1., r=2.)
    _look(view, V(2., 1., 0.), distance=5., pitch=math.pi / 2 - .001)
    refresh(view)
    move(view, V(2., 3., 0.))
    ref = view.source_viewport._valid_center_ref()
    assert ref is not None and (ref[0] - V(2., 1., 0.)).length() < 1e-5
    move(view, V(2., 1.5, 0.))
    assert move(view, V(2., 1., 0.)).kind == 'center'
    view.scene.mesh.clear()
    refresh(view)
    assert view.source_viewport._valid_center_ref() is None

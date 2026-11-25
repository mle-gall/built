"""CAD-style inference and face cutting, as the Metal view delivers them.

Reported 2026-11-18: «we're still missing some of .skp intelligence like
perpendicular snapping, face cut detection», «snapping sometimes doesn't
work (not finding points)». The rules existed in ``core.snap``; what was
missing:

* the Metal view handed ``compute_snap`` a short list of its keywords: no
  acquired edge or face, no Shift lock, no plane rules, no rectangle
  diagonal (``InferenceEngine.snap_options`` is now the one list);
* «perpendicular to the edge you started on» only looked at the edges near
  the cursor, so it died once the line was longer than that radius;
* a hovered edge offered parallel but never perpendicular;
* on a sloped face the axis magnet carried the point off the face, so a loop
  drawn on it floated and cut nothing;
* the Metal view never captured the clicked face's plane, so a rectangle
  dragged past a sloped face's border left it whole;
* one nearest-48 edge query for every group let dense geometry behind a
  corner push the corner's own edges out.

The end-to-end cases drive an unshown ``WgpuView`` with the real tools (no
renderer, no window, no settings); the rest call ``compute_snap`` directly.
"""
import math
from types import SimpleNamespace

import numpy as np
import pytest
from built_core import Input as Qt
from built_render import MouseEvent
from built_core import Matrix4 as QMatrix4x4, Vec3 as V

from core.group import Group
from core.history import History
from core.mesh import Mesh
from core.scene import Scene
from core.snap import compute_snap
from tools.rust_tools import LineTool
from tools.rust_tools import RectangleTool
from tests.controller_surface import ControllerSurface as Viewport
from built_core import resident_prepare as prepare, surface


# ---- An unshown Metal view driving the real tools --------------------------

@pytest.fixture
def view(native_host):
    source = native_host.viewport
    source.scene = Scene()
    source.history = History(source.scene)
    source.flash_status = lambda *a: None
    source.notify_scene_changed = lambda: None
    source.active_tool = LineTool()
    native_host.resize(800, 600, 1.)
    v = native_host.view
    v.source_viewport = source
    v.camera = source.camera
    v.camera.target = V(1, 1, 0)
    v.camera.pitch = math.pi / 2 - .001      # looking down
    v.camera.distance = 8
    v.camera.aspect = 800 / 600
    v.set_scene(source.scene)
    v._upload = lambda **kwargs: None
    v.pick_row = lambda x, y: _pick(v, x, y)
    yield v
    v.renderer = None


def _pick(view, x, y):
    """The ID pass's answer without a renderer: the nearest placement the
    cursor ray hits."""
    index = view.inference_index
    if index is None or not len(view.packed.instances):
        return None
    origin, direction = view._pixel_to_ray(x, y)
    best = (math.inf, None)
    for row in range(len(view.packed.instances)):
        hit = surface(index, row, origin, direction)
        if hit is not None and (hit - origin).length() < best[0]:
            best = ((hit - origin).length(), row)
    return best[1]


def refresh(view):
    view.scene.version += 1
    view.set_scene(view.scene)
    if view.inference_index is None:
        view.inference_index = prepare(view.packed, view.scene)


def _event(view, kind, p, buttons=Qt.NoButton, button=Qt.NoButton,
           modifiers=Qt.NoModifier):
    x, y = view._world_to_pixel(p)
    handler = {"move": view.mouseMoveEvent, "press": view.mousePressEvent, "release": view.mouseReleaseEvent}[kind]
    handler(MouseEvent(kind, x, y, button, buttons, modifiers))


def move(view, p, modifiers=Qt.NoModifier):
    _event(view, "move", p, modifiers=modifiers)
    return view.last_snap


def click(view, p):
    _event(view, "press", p, Qt.LeftButton, Qt.LeftButton)
    _event(view, "release", p, Qt.NoButton, Qt.LeftButton)
    refresh(view)


def use(view, tool):
    view.source_viewport.active_tool = tool
    view.tools.sync()


def same(a, b, tol=2e-3):
    return (a - b).length() <= tol


_C, _S = math.cos(math.radians(30)), math.sin(math.radians(30))


def turned(x, y):
    """A frame turned 30° about Z (no edge runs along an axis)."""
    return V(_C * x - _S * y + 1, _S * x + _C * y, 0)


def sloped(x, y, slope=0.4):
    """A roof rising along X."""
    return V(x, y, slope * x)


def areas(mesh):
    return sorted(round(f.net_area(), 3) for f in mesh.faces)


# ---- Perpendicular ----------------------------------------------------------

def test_perpendicular_to_the_edge_the_line_started_on_survives_a_long_line(view):
    view.scene.mesh.add_face([turned(0, 0), turned(2, 0), turned(2, 2), turned(0, 2)])
    refresh(view)
    click(view, turned(0.6, 0))
    # A metre away (~90 px): the start edge is nowhere near the cursor.
    snap = move(view, turned(0.66, 1.0))
    assert snap.kind == "reference", (snap.kind, snap.point)
    assert same(snap.point, turned(0.6, 1.0)), snap.point
    # …and where that perpendicular meets the far edge.
    snap = move(view, turned(0.64, 2.0))
    assert snap.kind == "intersection" and same(snap.point, turned(0.6, 2.0))
    click(view, turned(0.64, 2.0))
    assert areas(view.scene.mesh) == [1.2, 2.8]


def test_a_hovered_edge_gives_parallel_and_perpendicular(view):
    view.scene.mesh.add_edge(turned(0, 0), turned(2, 0))
    refresh(view)
    click(view, turned(1, -1))
    move(view, turned(1.5, 0))                       # acquire the edge
    perp = move(view, turned(1.02, -0.3))
    assert perp.kind == "reference" and same(perp.point, turned(1, -0.3)), perp.point
    move(view, turned(1.5, 0))
    par = move(view, turned(1.6, -1.02))
    assert par.kind == "reference" and same(par.point, turned(1.6, -1)), par.point


def test_perpendicular_to_a_hovered_edge_in_compute_snap():
    edge = SimpleNamespace(a=turned(0, 0), b=turned(2, 0))
    start, cand = turned(1, -1), turned(1.02, -0.3)
    w2p = lambda p: (p.x() * 100.0, p.y() * 100.0)

    def project(s, d):
        d = d.normalized()
        return s + d * V.dotProduct(cand - s, d)

    snap = compute_snap(cand, w2p(cand), SimpleNamespace(edges=[]), w2p, 9.0,
                        start_point=start, project_onto_line=project,
                        acquired_edge=edge)
    assert snap.kind == "reference" and same(snap.point, turned(1, -0.3))
    # An axis-aligned perpendicular stays the axis's own cue.
    flat = SimpleNamespace(a=V(0, 0, 0), b=V(2, 0, 0))
    cand = V(1.0, 0.7, 0)
    snap = compute_snap(cand, w2p(cand), SimpleNamespace(edges=[]), w2p, 9.0,
                        start_point=V(1, 0, 0), project_onto_line=project,
                        acquired_edge=flat)
    assert snap.kind != "reference"


def test_start_edges_feed_the_perpendicular_rule():
    """``compute_snap`` sees the edge the line started on even when the
    caller's nearby edges leave it out."""
    wall = SimpleNamespace(a=turned(0, 0), b=turned(2, 0))
    start, cand = turned(0.6, 0), turned(0.66, 1.0)
    w2p = lambda p: (p.x() * 100.0, p.y() * 100.0)

    def project(s, d):
        d = d.normalized()
        return s + d * V.dotProduct(cand - s, d)

    kw = dict(start_point=start, project_onto_line=project)
    scene = SimpleNamespace(edges=[])
    assert compute_snap(cand, w2p(cand), scene, w2p, 9.0, **kw).kind != "reference"
    snap = compute_snap(cand, w2p(cand), scene, w2p, 9.0, start_edges=[wall], **kw)
    assert snap.kind == "reference" and same(snap.point, turned(0.6, 1.0))


def test_shift_holds_a_magenta_lock_on_metal(view):
    view.scene.mesh.add_edge(turned(0, 0), turned(2, 0))
    refresh(view)
    click(view, turned(1, -1))
    perp = (turned(1, 0) - turned(1, -1)).normalized()
    view.source_viewport.inference._shift_lock = (perp, (0.85, 0.3, 0.8))
    snap = move(view, turned(1.4, -0.3), Qt.ShiftModifier)  # well off it
    assert snap.kind == "reference" and same(snap.point, turned(1, -0.3), 5e-3)


def test_metal_hands_compute_snap_every_engine_keyword(view, monkeypatch):
    import core.snap as snap_rules
    seen = {}
    real = snap_rules.compute_snap

    def spy(*a, **k):
        seen.update(k)
        return real(*a, **k)

    monkeypatch.setattr(snap_rules, "compute_snap", spy)
    view.scene.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    refresh(view)
    use(view, RectangleTool())
    click(view, V(0.5, 0.5, 0))
    move(view, V(1.5, 0.52, 0))
    wanted = set(view.source_viewport.inference.snap_options(
        view.source_viewport.active_tool, Qt.NoModifier))
    assert wanted <= set(seen), wanted - set(seen)
    assert seen["diagonal_span"] is True
    assert seen["work_plane_normal"] is not None


# ---- Point snaps ------------------------------------------------------------

def square(mesh, x0, y0, w=2.0):
    return mesh.add_face([V(x0, y0, 0), V(x0 + w, y0, 0),
                          V(x0 + w, y0 + w, 0), V(x0, y0 + w, 0)])


@pytest.mark.parametrize("where", ["loose", "open group", "sibling group",
                                   "component", "open component"])
def test_point_snaps_in_every_context(view, where):
    if where == "loose":
        square(view.scene.mesh, 3, 0)
    else:
        mine, other = Mesh(), Mesh()
        square(mine, 0, 0)
        square(other, 3, 0)
        g1, g2 = Group(mine), Group(other)
        if "component" in where:
            g2 = Group(Mesh())
            square(g2.mesh, 0, 0)
            m = QMatrix4x4()
            m.translate(3, 0, 0)
            g2.xform, g2.component = m, True
        view.scene.groups += [g1, g2]
        refresh(view)
        if where in ("open group", "sibling group"):
            view.scene.begin_group_edit(g1 if where == "sibling group" else g2)
        elif where == "open component":
            view.scene.begin_group_edit(g2)
    refresh(view)
    cases = [(V(5, 2, 0), V(5, 2, 0), "endpoint"),
             (V(5.04, 2.03, 0), V(5, 2, 0), "endpoint"),     # 3 px beside it
             (V(4, 0, 0), V(4, 0, 0), "midpoint"),
             (V(3.6, 0, 0), V(3.6, 0, 0), "on_edge"),
             (V(3.6, 0.7, 0), V(3.6, 0.7, 0), "on_face")]
    for at, want, kind in cases:
        snap = move(view, at)
        assert snap.kind == kind and same(snap.point, want), (where, kind, snap.kind, snap.point)


def test_a_corner_in_front_of_dense_geometry_is_found(view):
    front = Mesh()
    front.add_face([V(0, 0, 1), V(1, 0, 1), V(1, 1, 1), V(0, 1, 1)])
    dense = Mesh()
    for i in range(1201):                    # six lines a pixel behind
        t = -0.5 + 3.0 * i / 1200
        dense.add_edge(V(t, -0.5, 0), V(t, 2.5, 0))
        dense.add_edge(V(-0.5, t, 0), V(2.5, t, 0))
    view.scene.groups += [Group(front), Group(dense)]
    refresh(view)
    for d in (0.0, 0.05, 0.1):               # up to ~6 px from the corner, on it
        snap = move(view, V(1 - d, 1 - d / 2, 1))
        assert snap.kind == "endpoint" and same(snap.point, V(1, 1, 1)), (d, snap.kind)


def test_crossing_lines_on_a_face_meet_and_split(view):
    square(view.scene.mesh, 0, 0)
    refresh(view)
    click(view, V(0.5, 0, 0))
    click(view, V(0.5, 2, 0))
    click(view, V(0, 1.2, 0))
    click(view, V(2, 1.2, 0))
    assert areas(view.scene.mesh) == [0.4, 0.6, 1.2, 1.8]
    snap = move(view, V(0.52, 1.21, 0))
    assert snap.kind == "endpoint" and same(snap.point, V(0.5, 1.2, 0))


# ---- Face cuts on a sloped face ----------------------------------------------

def _roof(view):
    view.camera.target = sloped(1, 1)
    view.scene.mesh.add_face([sloped(0, 0), sloped(2, 0), sloped(2, 2), sloped(0, 2)])
    refresh(view)


_K = math.sqrt(1 + 0.4 ** 2)          # area factor of the 0.4 slope


def _a(x):
    return round(x * _K, 3)


def test_a_loop_drawn_on_a_sloped_face_cuts_it(view):
    _roof(view)
    for p in [(0.5, 0.5), (1.5, 0.5), (1.5, 1.5), (0.5, 1.5), (0.5, 0.5)]:
        click(view, sloped(*p))
    assert areas(view.scene.mesh) == [_a(1), _a(3)]
    for f in view.scene.mesh.faces:          # nothing floats off the roof
        assert all(abs(v.z() - 0.4 * v.x()) < 1e-3 for v in f.vertices)


def test_a_rectangle_over_a_sloped_faces_border_splits_it(view):
    _roof(view)
    use(view, RectangleTool())
    click(view, sloped(1, 1))
    click(view, sloped(3, 3))
    assert areas(view.scene.mesh) == [_a(1), _a(3), _a(3)]


def test_the_magnet_keeps_a_line_on_the_sloped_face_it_is_drawn_on():
    """compute_snap alone: the start on the roof, the cursor on the roof one
    step along red; the red line runs under the roof (the roof climbs
    along X), so the point stays on the face."""
    n = V.crossProduct(V(1, 0, 0.4), V(0, 1, 0)).normalized()
    start, cand = sloped(0.5, 1.0), sloped(1.5, 1.01)
    w2p = lambda p: (p.x() * 100.0, p.y() * 100.0)

    def project(s, d):                   # the cursor ray is vertical
        d = d.normalized()
        flat = V(d.x(), d.y(), 0)
        if flat.lengthSquared() < 1e-12:
            return V(s)
        t = V.dotProduct(V(cand.x(), cand.y(), 0) - V(s.x(), s.y(), 0), flat) / flat.lengthSquared()
        return s + d * t

    kw = dict(start_point=start, project_onto_line=project,
              magnetic_axis_deg=3.0, screen_axis_px=9.0)
    free = compute_snap(cand, w2p(cand), SimpleNamespace(edges=[]), w2p, 9.0, **kw)
    assert free.kind == "axis"           # the control: the magnet engages
    snap = compute_snap(cand, w2p(cand), SimpleNamespace(edges=[]), w2p, 9.0,
                        face_under_cursor=True, work_plane_normal=n, **kw)
    assert snap.kind == "on_face" and same(snap.point, cand), (snap.kind, snap.point)
    # A post rising from the roof (blue, steep to it) still engages.
    up = start + V(0, 0, 1)
    oblique = lambda p: (p.x() * 100.0 + p.z() * 30.0, p.y() * 100.0 - p.z() * 80.0)
    kw["project_onto_line"] = lambda s, d: s + d.normalized() * V.dotProduct(
        up - s, d.normalized())
    snap = compute_snap(up, oblique(up), SimpleNamespace(edges=[]), oblique, 9.0,
                        face_under_cursor=True, work_plane_normal=n, **kw)
    assert snap.kind == "axis" and snap.axis == "z"

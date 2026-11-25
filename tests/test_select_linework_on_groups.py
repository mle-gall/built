# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Lines lying on a grouped face are clicked, on both views.

Maxence, 2026-11-22: selecting linework is hard when the lines lie on a face
that belongs to a group, loose or grouped; sometimes there is no hitbox at
all and only a selection box gets them. The causes, reproduced here:

- a group of lines only (a hatch) had no hitbox but its lines in the ID
  pass (Metal: the 1-px line itself, lost to the face below at a grazing
  angle or a hair under it) or the face test (OpenGL: a face under the
  cursor always went first, so a hatch on a slab was never picked);
- a line a couple of millimetres under the face z-fights it — drawn at one
  angle, gone at the next — and the strict occlusion test gave the click to
  the slab.

The lines now come first within the pick tolerance (the modeller's rule: an
edge before the face under the cursor), with a small depth slack. Between
the lines of a hatch the slab under the cursor still wins; a hatch with
nothing under it is picked by its box. A plain click again at the same spot
cycles through what lies there (line, the slab, the hatch's box…), the
status bar naming it.

The Metal cases run device-free: the ID pass is stood in for by the CPU ray
test over the resident rows (``_front_at``). The GPU fixture also exercises the native ID pass.
"""
from __future__ import annotations

import math

import pytest
from ingetrazo_core import ScreenPoint, Input
from ingetrazo_core import Vec3 as QVector3D

from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401
from core.group import Group
from core.history import History
from core.mesh import Edge, Mesh
from core.scene import Scene
from tools.base import ToolContext
from tools.rust_tools import SelectTool

V = QVector3D
ANGLES = {"top": (0.3, math.pi / 2 - .02), "oblique": (0.6, math.radians(45)),
          "low": (0.4, math.radians(18))}


def _slab(name="slab", z=0.0):
    m = Mesh()
    m.add_face([V(0, 0, z), V(4, 0, z), V(4, 4, z), V(0, 4, z)])
    return Group(m, name=name)


def _lines(mesh, dz):
    return [mesh.add_edge(V(1, y, dz), V(3, y, dz)) for y in (1.5, 2.0, 2.5)]


def build(kind, dz=0.0, slab=True):
    """``(scene, slab_pick, targets)``: a 4 m slab group at z=0 (inside an
    outer group for ``nested``) with three 2 m lines on it, ``dz`` above
    it — loose (``loose``, ``nested``) or a group of lines (``hatch``)."""
    scene = Scene()
    pick = None
    if slab:
        s = _slab()
        if kind == "nested":
            outer = Group(Mesh(), name="outer")
            outer.children = [s]
            scene.groups.append(outer)
            pick = outer
        else:
            scene.groups.append(s)
            pick = s
    if kind == "hatch":
        hatch = Group(Mesh(), name="hatch")
        _lines(hatch.mesh, dz)
        scene.groups.append(hatch)
        targets = [hatch]
    else:
        targets = _lines(scene.mesh, dz)
    scene.version += 1
    return scene, pick, targets


def _gl(scene, vp=None):
    from tests.controller_surface import ControllerSurface as Viewport
    if vp is None:
        vp = Viewport(None)
        vp.resize(800, 600)
    vp.scene = scene
    vp.history = History(scene)
    vp.flashed = []
    vp.flash_status = lambda text, *a: vp.flashed.append(text)
    vp.notify_scene_changed = lambda: None
    vp.active_tool = SelectTool()
    return vp


class GL:
    """The OpenGL viewport: its own picks."""

    def __init__(self, scene):
        self.vp = _gl(scene)
        self.picks = self.vp
        self.camera = self.vp.camera
        self.flashed = self.vp.flashed

    def pixel(self, p):
        return self.vp._world_to_pixel(p)

    def close(self):
        self.vp.close()


class Metal:
    """The Metal view over the same document; ``native`` uses the real
    renderer's ID pass, otherwise the CPU ray test stands in for it."""

    def __init__(self, scene, host, native=False):
        from views.wgpu_inference import prepare
        host.resize(800, 600, 1.)
        self.source = _gl(scene, host.window.viewport)
        self.flashed = self.source.flashed
        v = self.view = host.view
        v.source_viewport = self.source
        v.camera = self.source.camera
        self.camera = v.camera
        v.set_scene(scene)
        if native:
            v._upload(force=True)
        v.inference_index = prepare(v.packed)
        if not native:
            v._upload = lambda *a, **k: None
            v.pick_row = self._cpu_row
        self.picks = v.tools
        self.picks.sync()

    def _cpu_row(self, x, y):
        from views.wgpu_inference import _front_at, eligible_rows
        v = self.view
        mvp = v.camera.projection_matrix() * v.camera.view_matrix()
        hit = _front_at(v.inference_index, v, x, y,
                        eligible_rows(v.inference_index, v.packed), mvp)
        return None if hit is None else hit[2]

    def pixel(self, p):
        return self.view._world_to_pixel(p)

    def close(self):
        # The host fixture owns the renderer and source viewport.
        pass


@pytest.fixture(params=["document", "resident", "gpu"])
def make(request):
    if request.param == "document":
        return GL
    native = request.param == "gpu"
    host = request.getfixturevalue("native_gpu_host" if native else "native_host")
    return lambda scene: Metal(scene, host, native)


def _aim(view, angle, distance=9.0):
    yaw, pitch = ANGLES[angle]
    cam = view.camera
    cam.perspective = True
    cam.yaw, cam.pitch = yaw, pitch
    cam.target = V(2, 2, 0)
    cam.distance = distance
    cam.aspect = 800 / 600
    if isinstance(view, Metal):
        view.view._pick_frame_dirty = True


def _click(view, tool, px, modifiers=Input.NoModifier):
    """What mouseReleaseEvent does for a click on either view."""
    tool.on_click(ToolContext(view.picks, QVector3D(), ScreenPoint(*px),
                              modifiers, None))
    return set(view.picks.scene.selection)


CASES = [(kind, dz, angle)
         for kind in ("loose", "hatch", "nested")
         for dz in (0.0, 0.002, -0.002)
         for angle in ("top", "oblique", "low")
         # 2 mm under at 18°: the face lies 6.5 mm before the line along
         # the sight line, past the slack at 9 m (4.5 mm).
         if not (dz < 0 and angle == "low")]


@pytest.mark.parametrize("kind,dz,angle", CASES)
def test_a_line_on_a_grouped_face_takes_the_click(make, kind, dz, angle):
    scene, slab, targets = build(kind, dz)
    view = make(scene)
    try:
        _aim(view, angle)
        tool = SelectTool()
        on = view.pixel(V(2, 2.0, dz))
        for px in (on, (on[0], on[1] + 4)):          # on it, 4 px off it
            picked = SelectTool().pick_at(view.picks, *px)
            assert any(picked == t for t in targets), (px, picked)
        # The face away from the lines still selects the slab's object.
        away = view.pixel(V(2, 0.5, 0))
        assert SelectTool().pick_at(view.picks, *away) is slab
    finally:
        view.close()


@pytest.mark.parametrize("kind", ["loose", "hatch"])
def test_a_line_well_under_the_face_does_not_steal_the_click(make, kind):
    scene, slab, targets = build(kind, -0.05)       # 5 cm under the slab
    view = make(scene)
    try:
        _aim(view, "top")
        px = view.pixel(V(2, 2.0, -0.05))
        assert SelectTool().pick_at(view.picks, *px) is slab
    finally:
        view.close()


def test_between_the_lines_of_a_hatch_in_the_air_its_box_picks_it(make):
    scene, _slab_pick, (hatch,) = build("hatch", slab=False)
    view = make(scene)
    try:
        _aim(view, "top")
        tool = SelectTool()
        between = view.pixel(V(2, 1.75, 0))
        assert SelectTool().pick_at(view.picks, *between) is hatch
        outside = view.pixel(V(3.6, 3.6, 0))
        assert SelectTool().pick_at(view.picks, *outside) is None
    finally:
        view.close()


def test_repeated_clicks_cycle_through_what_lies_under_the_cursor(make):
    scene, slab, (hatch,) = build("hatch")
    view = make(scene)
    try:
        _aim(view, "top")
        tool = SelectTool()
        on = view.pixel(V(2, 2.0, 0))
        assert _click(view, tool, on) == {hatch}
        assert _click(view, tool, (on[0] + 1, on[1])) == {slab}
        assert "2 of 2" in view.flashed[-1] and "slab" in view.flashed[-1]
        assert _click(view, tool, on) == {hatch}            # and back
        # Between the lines the slab comes first, the hatch's box next.
        between = view.pixel(V(2, 1.75, 0))
        tool.hover_at(view.picks, *between, 0)
        assert _click(view, tool, between) == {slab}
        assert _click(view, tool, between) == {hatch}
    finally:
        view.close()


def test_a_loose_line_cycles_to_the_group_and_back(make):
    scene, slab, lines = build("loose")
    view = make(scene)
    try:
        _aim(view, "oblique")
        tool = SelectTool()
        on = view.pixel(V(2, 2.0, 0))
        first = _click(view, tool, on)
        assert len(first) == 1 and isinstance(next(iter(first)), Edge)
        assert _click(view, tool, on) == {slab}
        assert _click(view, tool, on) == first
    finally:
        view.close()


def test_without_the_repeat_a_click_picks_as_before(make):
    scene, slab, (hatch,) = build("hatch")
    view = make(scene)
    try:
        _aim(view, "top")
        tool = SelectTool()
        on = view.pixel(V(2, 2.0, 0))
        assert _click(view, tool, on) == {hatch}
        # Leaving the spot ends the cycle: back there, the first pick again.
        tool.hover_at(view.picks, on[0] + 40, on[1], 0)
        tool.hover_at(view.picks, *on, 0)
        assert _click(view, tool, on) == {hatch}
        # The selection changed in between (Esc): no cycle either.
        scene.clear_selection()
        assert _click(view, tool, on) == {hatch}
        # Shift toggles the same pick, as it always did.
        assert _click(view, tool, on, Input.ShiftModifier) == set()
        assert _click(view, tool, on, Input.ShiftModifier) == {hatch}
        # A plain click on the slab, away from the lines: the slab, and
        # only one candidate, so nothing to announce.
        before = len(view.flashed)
        away = view.pixel(V(0.5, 0.5, 0))
        assert _click(view, tool, away) == {slab}
        assert len(view.flashed) == before
    finally:
        view.close()


def test_metal_click_event_takes_the_hatch(native_host):
    """Through the Metal view's own mouse events."""
    scene, slab, (hatch,) = build("hatch")
    view = Metal(scene, native_host)
    try:
        _aim(view, "top")
        x, y = view.pixel(V(2, 2.0, 0))
        x, y = round(x), round(y) + 3
        native_host.pointer("press", x, y, Input.LeftButton, Input.LeftButton)
        native_host.pointer("release", x, y, Input.LeftButton)
        assert scene.selection == {hatch}
    finally:
        view.close()


def test_inside_an_open_group_its_hatch_beats_its_slab(make):
    """Editing a group holding a slab and a hatch lying on it: the hatch's
    line takes the click, the slab's face away from it takes the slab."""
    scene = Scene()
    slab = _slab()
    hatch = Group(Mesh(), name="hatch")
    _lines(hatch.mesh, 0.0)
    outer = Group(Mesh(), name="outer")
    outer.children = [slab, hatch]
    scene.groups.append(outer)
    scene.version += 1
    view = make(scene)
    try:
        _aim(view, "oblique")
        view.picks.begin_group_edit(outer)
        if isinstance(view, Metal):
            view.view.set_scene(scene)
            view.view.sync_edit_context()
        assert scene.edit_group is outer
        tool = SelectTool()
        on = view.pixel(V(2, 2.0, 0))
        assert SelectTool().pick_at(view.picks, on[0], on[1] + 3) is hatch
        assert SelectTool().pick_at(view.picks, *view.pixel(V(2, 0.5, 0))) is slab
    finally:
        view.close()

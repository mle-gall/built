# SPDX-License-Identifier: GPL-3.0-or-later
"""Metal: a double click of the wheel recentres like GL (#979, #1154).

On the model, the point the Metal pick finds under the cursor goes to the
centre: a slide, no turn, perspective keeps the point's depth. On sky the
pixel goes to the centre at the depth of the view's centre. The tool in
use never hears of it and no orbit starts. Device free: the pick is the
offscreen stand-in the other test_wgpu_* files use.
"""
import math
from types import SimpleNamespace

import pytest
from ingetrazo_core import Vec3 as V

from core.mesh import Mesh
from core.scene import Scene
from views.wgpu_inference import prepare
from views.wgpu_scene import pack_scene
from core.input_codes import MouseButton
from tests.headless_gpu import HeadlessView, mouse

WHEEL = MouseButton.MiddleButton


def _double_click(view, x, y, button=WHEEL):
    """What the window delivers: press, release, double click, release."""
    mouse(view, "press", x, y, button, button)
    mouse(view, "release", x, y, button, 0)
    mouse(view, "double", x, y, button, button)
    mouse(view, "release", x, y, button, 0)


@pytest.fixture
def view():
    """A wall 20 m wide in the plane y = 0, seen from the front."""
    scene = Scene()
    scene.guides.clear()
    scene.mesh.add_face([V(-10, 0, -10), V(10, 0, -10), V(10, 0, 10), V(-10, 0, 10)])
    v = HeadlessView()
    v.resize(800, 600)
    v.scene = scene
    v.packed = pack_scene(scene)
    v.uid_rows = {uid: [i] for i, uid in enumerate(v.packed.uids)}
    v.inference_index = prepare(v.packed)
    v.hit = True
    v.renderer = SimpleNamespace(set_highlight=lambda *a: None, set_rest=lambda *a: None,
                                 pick=lambda *a: 0 if v.hit else -1,
                                 pick_rect=lambda *a: [0])
    v._prepare_pick_frame = lambda: None
    cam = v.camera
    cam.set_view('front')
    cam.target = V(0, 0, 0)
    cam.distance = 10.0
    cam.aspect = 800 / 600
    yield v
    v.renderer = None
    v.close()


def _projected(view, point):
    n = (view.camera.projection_matrix() * view.camera.view_matrix()).map(point)
    return (n.x() + 1) / 2 * view.width(), (1 - n.y()) / 2 * view.height()


@pytest.mark.parametrize('perspective', [True, False], ids=['perspective', 'parallel'])
def test_the_point_under_the_cursor_goes_to_the_centre(view, perspective):
    view.camera.perspective = perspective
    x, y = 460, 260
    seen, point = view._depth_world_at(x, y)
    assert seen == 'hit' and abs(point.y()) < 1e-4
    yaw, pitch, distance = view.camera.yaw, view.camera.pitch, view.camera.distance
    _double_click(view, x, y)
    cam = view.camera
    assert (cam.target - point).length() < 1e-4
    px, py = _projected(view, point)
    assert abs(px - 400) < 1 and abs(py - 300) < 1
    assert (cam.yaw, cam.pitch) == (yaw, pitch)
    assert cam.distance == pytest.approx(distance, rel=1e-3)


def test_on_sky_it_slides_at_the_centre_depth(view):
    view.hit = False
    x, y = 460, 260
    expected = view._point_at_centre_depth(x, y)
    distance = view.camera.distance
    _double_click(view, x, y)
    assert (view.camera.target - expected).length() < 1e-4
    assert view.camera.distance == distance


def test_the_tool_never_hears_of_it(view, monkeypatch):
    calls = []
    monkeypatch.setattr(view.tools, 'sync', lambda: calls.append('sync') or 'select')
    _double_click(view, 460, 260)
    # mousePressEvent syncs the tools; the double click itself does not.
    assert calls.count('sync') == 1


def test_no_orbit_after_the_double_click(view):
    _double_click(view, 460, 260)
    yaw, pitch = view.camera.yaw, view.camera.pitch
    mouse(view, "move", 520, 300, 0, WHEEL)
    assert (view.camera.yaw, view.camera.pitch) == (yaw, pitch)


def test_a_wheel_drag_still_orbits(view):
    x, y = 460, 260
    mouse(view, "press", x, y, WHEEL, WHEEL)
    yaw = view.camera.yaw
    mouse(view, "move", x + 60, y, 0, WHEEL)
    assert not math.isclose(view.camera.yaw, yaw)

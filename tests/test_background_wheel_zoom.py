"""Empty-space zoom stays at view depth; high-resolution ticks stay fractional."""
from types import SimpleNamespace
import pytest
from core.input_types import QPoint, QPointF, Qt
from ingetrazo_render import WheelEvent
from ingetrazo_core import Vec3 as V
from core.camera import OrbitCamera
from tests.controller_surface import ControllerSurface as Viewport


@pytest.mark.parametrize('slope', [-.001, 0., .001])
def test_background_near_horizon_uses_focal_depth(slope):
    camera = SimpleNamespace(target=V(0, 20, 10), eye=lambda: V(0, 0, 10))
    ray = V(0, 1, slope).normalized()
    vp = SimpleNamespace(camera=camera,
                         _pixel_to_ray=lambda *a: (camera.eye(), ray),
                         _depth_world_at=lambda *a: ('background', None))
    focus = Viewport._world_under_cursor(vp, 0, 0)
    assert focus.y() == pytest.approx(20)
    assert focus.z() == pytest.approx(10 + 20 * slope)
    assert (focus - camera.eye()).length() < 21


def test_real_surface_hit_keeps_its_actual_depth():
    hit = V(0, 200, 9)
    vp = SimpleNamespace(_pixel_to_ray=lambda *a: (V(0, 0, 10), V(0, 1, 0)),
                         _depth_world_at=lambda *a: ('hit', hit))
    assert Viewport._world_under_cursor(vp, 0, 0) is hit


def wheel(delta):
    return WheelEvent(100, 100, 0, 0, 0, delta, "move")


def viewport():
    camera = OrbitCamera()
    camera.target = V(0, 0, 0)
    camera.distance = .02
    # The ticks carry a scroll phase, like a finger slide; the preference
    # keeps them zooming (views.trackpad_nav) so the fractions reach the
    # wheel's zoom.
    vp = SimpleNamespace(camera=camera, scene=SimpleNamespace(version=0),
                         _invert_wheel=False, _trackpad_slide="zoom", _touch_slide="zoom",
                         update=lambda: None,
                         _zoom_out_step=lambda: 1.0,
                         _world_under_cursor=lambda *a: V(0, 0, 0))
    vp._zoom_at = lambda steps, pos: Viewport._zoom_at(vp, steps, pos)
    return vp


def test_many_fractional_ticks_do_not_each_receive_a_full_notch_floor():
    vp = viewport()
    for _ in range(120):
        Viewport.wheelEvent(vp, wheel(-1))
    assert vp.camera.distance == pytest.approx(1.02, abs=1e-5)
    notch = viewport()
    Viewport.wheelEvent(notch, wheel(-120))
    assert notch.camera.distance == pytest.approx(vp.camera.distance, abs=1e-5)


def test_zero_delta_does_not_pick_or_change_camera():
    vp = viewport()
    def unexpected(*args):
        pytest.fail('zero-delta event must not pick')
    vp._world_under_cursor = unexpected
    Viewport.wheelEvent(vp, wheel(0))
    assert vp.camera.distance == .02


def test_a_slide_gesture_picks_the_point_under_the_cursor_once():
    """Per-event cost, as an operation count: 200 slide events with the
    hand resting near one spot zoom toward ONE pick, the first event's.
    A pick per event is a GPU readback each (the zoom-lag suspect)."""
    vp = viewport()
    vp.camera.distance = 50.0
    picks = []
    vp._world_under_cursor = lambda x, y: picks.append((x, y)) or V(1, 2, 0)
    for i in range(200):
        x = 100 + i % 7
        event = WheelEvent(x, 100, 0, 1, 0, -2 if i < 100 else 2, "move")
        Viewport.wheelEvent(vp, event)
    assert len(picks) == 1
    assert vp.camera.distance != 50.0


def test_recentering_and_zoom_keep_their_grazing_ray_boundaries():
    view = SimpleNamespace(
        camera=SimpleNamespace(target=V(1, 0, 0), eye=lambda: V(0, 0, 0)),
        _pixel_to_ray=lambda x, y: (V(0, 0, 0), V(1e-6, 1, 0)),
        _depth_world_at=lambda x, y: ('miss', None),
    )
    assert Viewport._point_at_centre_depth(view, 0, 0).toTuple() == (1.0, 1e6, 0.0)
    assert Viewport._world_under_cursor(view, 0, 0) is None


@pytest.mark.parametrize("value", [
    -0.0, -1e-15, 0.0000005, -0.0000015, 0.0000000005, -0.0000000015,
    4503599627.370497, -4503599627370.497, 1e100,
])
def test_zoom_focus_preserves_public_rounded_pose(value):
    view = SimpleNamespace(
        camera=SimpleNamespace(yaw=value, pitch=value, distance=value,
                               target=V(value, value, value), zoom=lambda steps: None),
        scene=SimpleNamespace(version=0), _world_under_cursor=lambda x, y: None,
    )
    at = QPointF(17, 23)
    assert Viewport._zoom_at(view, 1., at) == (False, False)
    expected = (round(value, 9),) * 2 + (round(value, 6),) * 4
    assert [x.hex() for x in view._zoom_focus[0]] == [x.hex() for x in expected]
    assert Viewport._zoom_at(view, 1., at) == (False, True)

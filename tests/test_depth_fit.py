"""The perspective near/far fitted to what the view shows (port of
5e61c559 onto the trunk camera): a near plane 0.1 m from the eye leaves
plates a few millimetres thick fighting at 50 m on the GL depth buffer.

The raster check needs a real GL context: run this file with
QT_QPA_PLATFORM=cocoa on macOS (no window is left open).
"""
import math


import numpy as np
import pytest
from built_core import Vec3 as V


from built_core.camera import OrbitCamera  # noqa: E402


def camera():
    c = OrbitCamera()
    c.distance = 20
    c.look_from(V(0, 0, 0), V(0, 1, 0))
    return c


@pytest.mark.parametrize('distance', [10, 50, 200])
def test_depth_fit_resolves_one_mm(distance):
    c = camera()
    c.fit_depth_range([[-1, distance, -1]], [[1, distance + .005, 1]])
    n, f = c.near_plane(), c.far_plane()
    assert 0 < n < distance < f
    step = distance ** 2 * (f - n) / (f * n * (2 ** 24 - 1))
    assert step < .001 / 10


def test_inside_unknown_and_changed_camera_are_conservative():
    c = camera()
    c.fit_depth_range([[-1, -1, -1]], [[1, 20, 1]])
    assert c.near_plane() == .1           # a box around the eye
    c.fit_depth_range([[-1, 10, -1]], [[1, 11, 1]])
    assert c.near_plane() > 1
    c.move_eye(V(0, 10, 0))
    assert c.near_plane() == .1           # a stale fit never clips a later view
    c.fit_depth_range([[-np.inf] * 3], [[np.inf] * 3])
    assert c.near_plane() == .1


def test_side_culling_does_not_use_old_far_plane():
    c = camera()
    c.fit_depth_range([[1e6, 1, -1], [-1, 20000, -1]], [[1e6 + 1, 2, 1], [1, 20001, 1]])
    assert c.near_plane() > 10000
    assert c.far_plane() > 20001


def test_two_point_and_parallel():
    c = camera()
    c.two_point = True
    c.pitch = math.radians(30)
    p = c.target
    c.fit_depth_range([p.toTuple()], [p.toTuple()])
    assert c.near_plane() == pytest.approx(20 * math.cos(c.pitch) * .95, abs=1e-4)
    c.perspective = False
    c.fit_depth_range([p.toTuple()], [p.toTuple()])
    assert c._fitted_depth_range() is None


def test_clipping_set_by_hand_wins():
    """Camera Settings ▸ Clipping: the planes the user typed are used as
    they are; the fit neither replaces them nor survives setting them."""
    c = camera()
    c.fit_depth_range([[-1, 50, -1]], [[1, 51, 1]])
    assert c.near_plane() > 40
    c.clip_near = 0.5
    assert c.near_plane() == 0.5          # the old fit lapses with the key
    c.fit_depth_range([[-1, 50, -1]], [[1, 51, 1]])
    assert c.near_plane() == 0.5
    c.clip_near, c.clip_far = None, 30.
    c.fit_depth_range([[-1, 50, -1]], [[1, 51, 1]])
    assert c.far_plane() == 30. and c.near_plane() == .1


@pytest.mark.parametrize('empty', [False, True])
def test_unknown_bounds_are_cached_and_empty_groups_do_not_block_fit(empty):
    from types import SimpleNamespace as NS
    from built_core.mesh import Mesh
    from tests.controller_surface import ControllerSurface as Viewport
    mesh = Mesh()
    if not empty:
        mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    group = NS(mesh=mesh, xform=object())
    scene = NS(edit_group=None, guides=[], shadows=NS(enabled=False),
               mesh=Mesh(), content_version=0, version=0, image_planes=[],
               entity_visible=lambda g: True)
    calls = []

    def box(g):
        calls.append(g)
        return None if g is group else ((-1, 10, -1), (1, 11, 1))
    vp = NS(camera=camera(), scene=scene, _placements_epoch=lambda: 0,
            _placements=lambda: [group, NS(mesh=Mesh(), xform=object())],
            _placement_bbox=box, _base_map_showing=lambda: False,
            _terrain_showing=lambda: False, _photo_showing=lambda: False)
    vp._depth_fit_refused = lambda: Viewport._depth_fit_refused(vp)
    vp._depth_fit_boxes = lambda: Viewport._depth_fit_boxes(vp)
    Viewport._fit_scene_depth(vp)
    count = len(calls)
    Viewport._fit_scene_depth(vp)
    assert len(calls) == count            # one walk per content epoch
    assert (vp.camera.near_plane() > 1) == empty
    scene.content_version += 1
    Viewport._fit_scene_depth(vp)
    assert len(calls) == 2 * count


@pytest.mark.parametrize('two_point', [False, True])
def test_depth_fit_keeps_all_editing_rays_exact(two_point):
    from types import SimpleNamespace as NS
    from tests.controller_surface import ControllerSurface as Viewport
    c = camera()
    c.two_point = two_point
    vp = NS(camera=c, width=lambda: 1124, height=lambda: 835)
    points = [(x, y) for x in (0, 50, 562, 1000, 1123) for y in (0, 80, 417, 750, 834)]

    def rays():
        return [tuple(v.toTuple() for v in Viewport._pixel_to_ray(vp, x, y))
                for x, y in points]
    before = rays()
    c.fit_depth_range([[-30, 150, -30]], [[30, 300, 30]])
    assert c.near_plane() > 100
    fitted = c._depth_range
    assert rays() == before
    assert c._depth_range is fitted


def test_metal_view_fits_the_shown_box_and_refuses_a_mesh_preview():
    from types import SimpleNamespace as NS
    from tests.headless_gpu import HeadlessView
    c = camera()
    source = NS(camera=c, scene=NS(edit_group=None, guides=[], shadows=NS(enabled=False)),
                _base_map_showing=lambda: False, _terrain_showing=lambda: False,
                _photo_showing=lambda: False, active_tool=None)
    packed = NS(instances=np.zeros(1), world_bounds=np.array([[-1., 50, -1], [1, 51, 1]]))
    view = NS(camera=c, source_viewport=source, packed=packed, tools=NS(mode='select'))
    HeadlessView._fit_depth(view)
    assert c.near_plane() > 40
    view.tools.mode = 'mesh'
    HeadlessView._fit_depth(view)
    assert c.near_plane() == .1
    view.tools.mode = 'select'
    source.scene.guides = [object()]
    HeadlessView._fit_depth(view)
    assert c.near_plane() == .1

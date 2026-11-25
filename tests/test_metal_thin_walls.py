"""Thin plates on the Metal view: the back face must never show through.

A plate a few millimetres thick is two faces: the top one seen from its
front, the bottom one from its back. With too little depth precision the
bottom face wins some pixels, and they flicker while orbiting. Each case
renders two frames 0.01 degrees apart and counts pixels that come out in
the back colour.

The renderer uses an offscreen wgpu target.
``THIN_WALLS_REPORT=1`` prints the table instead of only asserting.
"""
import importlib.util
import math
import os


import numpy as np
import pytest
from built_core import Vec3 as V


from core.camera import OrbitCamera  # noqa: E402
from core.scene import Scene  # noqa: E402

HAS_RENDERER = importlib.util.find_spec("built_render") is not None
SIZE = 256
THICKNESS_MM = (1, 2, 5)
DISTANCE_M = (10, 50, 200)


@pytest.fixture(scope="module")
def native():
    from built_render import Renderer
    renderer = Renderer(0, SIZE, SIZE, 1., platform="offscreen")
    renderer.set_antialias(False)
    renderer.set_edges(False)
    return renderer


def plate_scene(distance, thickness, top=True, bottom=True):
    """A square plate filling most of the view at ``distance``: the top face
    looks up (+z), the bottom one down (-z), as a closed plate's faces do."""
    scene = Scene()
    m = scene.loose_mesh
    h = distance * 0.4
    if top:
        m.add_face([V(-h, -h, thickness), V(h, -h, thickness),
                    V(h, h, thickness), V(-h, h, thickness)])
    if bottom:
        m.add_face([V(-h, -h, 0), V(-h, h, 0), V(h, h, 0), V(h, -h, 0)])
    return scene


def camera_at(distance, yaw_deg=-37.0):
    c = OrbitCamera()
    c.aspect = 1.0
    c.target = V(0, 0, 0)
    c.distance = distance
    c.yaw = math.radians(yaw_deg)
    c.pitch = math.radians(40.0)
    return c


def render(renderer, scene, camera, fit=False):
    from built_core import pack_scene
    from views.wgpu_style import style_payload
    packed = pack_scene(scene)
    packed.upload(renderer)
    if fit:
        # What the Metal view does each frame (WgpuView._fit_depth).
        lo, hi = np.asarray(packed.world_bounds, float)
        camera.fit_depth_range(lo[None], hi[None])
        assert camera._fitted_depth_range() is not None
    renderer.set_camera(list(camera.view_matrix().data()),
                        list(camera.projection_matrix().data()))
    style = style_payload(scene)
    style.update(sky=False, edges=False, profiles=False, shadows=False, ao=False)
    renderer.set_style(style)
    renderer.request_capture()
    renderer.render(False)
    rgba = np.frombuffer(bytes(renderer.take_capture()), np.uint8)
    return rgba.reshape(SIZE, SIZE, 4)[..., :3].astype(int)


def back_pixels(renderer, distance, thickness_mm, fit=False):
    t = thickness_mm / 1000.0
    total = 0
    for yaw in (-37.0, -37.01):
        cam = camera_at(distance, yaw)
        front = render(renderer, plate_scene(distance, t, bottom=False), cam, fit)
        back = render(renderer, plate_scene(distance, t, top=False), cam, fit)
        both = render(renderer, plate_scene(distance, t), cam, fit)
        total += count_back(front, back, both)
    return total


def count_back(front, back, both):
    """Pixels of the plate that came out closer to its back face's colour
    than to its front's, inside the top face's interior: two pixels in
    from its outline, since along the near edge the bottom face really
    shows past the top one."""
    cover = np.abs(front - front[0, 0]).sum(axis=2) > 8
    for _ in range(2):
        cover[1:-1, 1:-1] &= (cover[:-2, 1:-1] & cover[2:, 1:-1]
                              & cover[1:-1, :-2] & cover[1:-1, 2:])
    cover &= np.abs(front - back).sum(axis=2) > 30
    assert cover.sum() > front.shape[0] * front.shape[1] // 8, "the plate must fill the view"
    to_back = np.abs(both - back).sum(axis=2)
    to_front = np.abs(both - front).sum(axis=2)
    return int((cover & (to_back < to_front)).sum())


@pytest.mark.parametrize("fit", [False, True])
@pytest.mark.parametrize("antialias", [False, True])
def test_thin_plates_never_show_their_back_face(native, antialias, fit):
    """Reverse-Z alone (``fit=False``) must hold; the near/far fit to the
    shown box (``fit=True``) must not disturb it."""
    native.set_antialias(antialias)
    table = {(t, d): back_pixels(native, d, t, fit)
             for t in THICKNESS_MM for d in DISTANCE_M}
    if os.environ.get("THIN_WALLS_REPORT"):
        print(f"\nback-colour pixels over two frames 0.01 deg apart, antialias={antialias}, fit={fit}")
        print("thickness  " + "  ".join(f"{d:>7} m" for d in DISTANCE_M))
        for t in THICKNESS_MM:
            print(f"{t:>6} mm  " + "  ".join(f"{table[t, d]:>9}" for d in DISTANCE_M))
    assert all(v == 0 for v in table.values()), table

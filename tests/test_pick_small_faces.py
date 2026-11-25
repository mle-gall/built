# SPDX-License-Identifier: GPL-3.0-or-later
"""A face of 1 mm can be picked and pushed (#1294).

The ray/triangle test refused |det| < 1e-6, but det is (e1 × e2)·d: it
shrinks with the triangle's area. A 1 mm square met head-on has det ≈ 6e-7,
so no tool found it under the cursor, however close the camera came. A
small det is now read against the triangle's own size — the angle between
the ray and its plane — and grazing rays are still refused."""
from __future__ import annotations

import math


import numpy as np  # noqa: E402
from built_core import Vec3 as V


from views.document_interaction import _moller_trumbore, _ray_triangle
from tests.controller_surface import ControllerSurface as Viewport


def _vp(side):
    vp = Viewport(None)
    vp.resize(1400, 850)
    vp.camera.set_aspect(1400, 850)
    vp.flash_status = lambda *a, **k: None
    vp.camera.target = V(side / 2, side / 2, 0)
    vp.camera.distance = 20 * side
    vp.camera.pitch = math.radians(35)
    vp.camera.yaw = math.radians(-40)
    return vp


def test_a_millimetre_square_is_picked():
    for side in (0.001, 0.0003):
        vp = _vp(side)
        f = vp.scene.mesh.add_face([V(0, 0, 0), V(side, 0, 0),
                                    V(side, side, 0), V(0, side, 0)])
        vp.scene.version += 1
        x, y = vp._world_to_pixel(V(side / 2, side / 2, 0))
        assert vp.pick_face(x, y) == f, side
        assert vp.pick_face_any(x, y) == (f, None), side


def test_a_grazing_ray_is_still_a_miss():
    o = np.array([-1.0, 0.25, 1e-9])
    d = np.array([1.0, 0.0, 0.0])                 # along the plane
    v0 = np.array([[0.0, 0.0, 0.0]])
    e1 = np.array([[1.0, 0.0, 0.0]])
    e2 = np.array([[0.0, 1.0, 0.0]])
    _t, hit = _moller_trumbore(o, d, v0, e1, e2)
    assert not hit[0]
    assert _ray_triangle(V(-1, 0.25, 1e-9), V(1, 0, 0), V(0, 0, 0),
                         V(1, 0, 0), V(0, 1, 0)) is None


def test_the_scalar_and_the_batched_test_agree_on_a_small_triangle():
    s = 0.001
    o = np.array([s / 3, s / 3, 1.0])
    d = np.array([0.1, -0.2, -1.0])
    d = d / np.linalg.norm(d)
    o = o - d * 0.0                              # head-on enough
    v0 = np.array([[0.0, 0.0, 0.0]])
    e1 = np.array([[s, 0.0, 0.0]])
    e2 = np.array([[0.0, s, 0.0]])
    # The ray from above the centroid, through the plane near it.
    o = np.array([s / 3, s / 3, 0.0]) - d * 1.0
    t, hit = _moller_trumbore(o, d, v0, e1, e2)
    assert hit[0]
    ts = _ray_triangle(V(*o), V(*d), V(0, 0, 0), V(s, 0, 0), V(0, s, 0))
    assert ts is not None and abs(ts - t[0]) < 1e-9


def test_ray_batches_keep_row_order_across_array_layouts():
    origin, direction = [0.25, 0.25, 3], (0, 0, -1)
    corners = np.array([[0, 0, 0], [4, 4, 1], [0, 0, 2]], dtype=np.float32)
    edges = np.tile(np.eye(3)[:2], (3, 1, 1))

    for rows in (corners, np.asfortranarray(corners), corners.ravel().tolist(),
                 np.repeat(corners, 2, axis=1)[:, ::2]):
        times, hits = _moller_trumbore(origin, direction, rows, edges[:, 0], edges[:, 1])
        np.testing.assert_array_equal(times, [3, 0, 1])
        np.testing.assert_array_equal(hits, [True, False, True])

    times, hits = _moller_trumbore(origin, direction, [], [], [])
    assert times.shape == hits.shape == (0,)

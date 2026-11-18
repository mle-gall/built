# SPDX-License-Identifier: GPL-3.0-or-later
"""What the Metal hover must not pay for on every move, and the faster
paths giving the same answers as the ones they replace.

Measured on synthetic scenes (scratch bench, unshown WgpuView, Line tool,
CPU inference only), 2026-11-18:

* a Line hovering a 12 000-face loose mesh before its first click: ~120 ms a
  move — ``work_point`` triangulated every face of the mesh in Python to
  find the face under the cursor — now under 1 ms (the face plane comes
  from the snap index's triangle under the cursor);
* the face pick for «Face center» (``pick_face_placement``) did the same
  walk, a second time: one vectorised ray test over the mesh's cached
  triangles now;
* the first hover after a camera move over 30 000 placements: ~18 ms, ~14 of
  them projecting every placement's box — now ~7 ms (world boxes kept,
  corners in eight contiguous passes).
"""
import math
from types import SimpleNamespace

import numpy as np
from PySide6.QtCore import QPointF, Qt
from PySide6.QtGui import QMouseEvent, QVector3D as V
from PySide6.QtWidgets import QApplication

from core.mesh import Face, Mesh
from tests.test_.skp_inference_gaps import refresh, view  # noqa: F401
from tests.test_wgpu_inference_parity import _box, _component, _look


def _hover(view, world):
    x, y = view._world_to_pixel(world)
    q = QPointF(x, y)
    QApplication.sendEvent(view, QMouseEvent(
        QMouseEvent.MouseMove, q, q, Qt.NoButton, Qt.NoButton, Qt.NoModifier))
    return view.last_snap


def test_the_work_plane_does_not_walk_the_mesh(view, monkeypatch):
    for i in range(400):
        _box(view.scene.mesh, ((i % 20) * 2.0, (i // 20) * 2.0, 0),
             ((i % 20) * 2.0 + 1, (i // 20) * 2.0 + 1, 1))
    _look(view, V(20, 20, 0), distance=30.0, pitch=45.0)
    refresh(view)
    calls = []
    original = Face.triangulate

    def counting(self, *a, **k):
        calls.append(1)
        return original(self, *a, **k)

    monkeypatch.setattr(Face, "triangulate", counting)
    # 2400 faces: under the «Face center» pick's cap, so the first hover
    # builds the mesh's triangles once; the moves after it walk nothing.
    _hover(view, V(10.5, 10.5, 1.0))
    first = len(calls)
    assert first <= len(view.scene.mesh.faces)
    for k in range(5):
        snap = _hover(view, V(10.5 + 0.05 * k, 10.5, 1.0))
    assert len(calls) == first, "a hover walked the mesh's faces again"
    assert snap.kind in ("on_face", "face_center")


def test_the_vectorised_face_pick_is_the_walk(view):
    """``pick_face_placement`` names the face the per-face Python walk
    named, in loose geometry and through a component's matrix, a face a
    tool is moving left out."""
    from views.viewport import _ray_triangle
    m = view.scene.mesh
    _box(m, (0, 0, 0), (2, 2, 2))
    proto = Mesh()
    _box(proto, (0, 0, 0), (1, 1, 1))
    view.scene.groups.append(_component(proto, 3, 0, 0))
    _look(view, V(2, 1, 1), distance=10.0, pitch=35.0, yaw=-60.0)
    refresh(view)
    tools = view.tools

    def walk(x, y):
        mesh, group, matrix = tools._picked_mesh(x, y)
        o, d = tools._pixel_to_ray(x, y)
        inv, _ok = matrix.inverted()
        o, d = inv.map(o), inv.mapVector(d)
        best, dist = None, math.inf
        moving = getattr(tools.tool, "moving_face_ids", lambda: set())()
        for f in mesh.faces:
            if f.key in moving or getattr(f, "hidden", False):
                continue
            for tri in f.triangulate():
                t = _ray_triangle(o, d, *tri)
                if t is not None and t < dist:
                    best, dist = f, t
        return best

    points = [V(0.5, 0.5, 2.0), V(1.5, 0.0, 1.0), V(2.0, 1.0, 0.5),
              V(3.5, 0.5, 1.0), V(3.5, 0.0, 0.5), V(4.0, 0.5, 0.5)]
    for p in points:
        x, y = view._world_to_pixel(p)
        face, place = tools.pick_face_placement(x, y)
        assert face is walk(x, y), p
        assert (place is not None) == (p.x() > 2.5)
    # A face a tool is moving is not picked; the one behind it is.
    x, y = view._world_to_pixel(V(0.5, 0.5, 2.0))
    top, _ = tools.pick_face_placement(x, y)
    real = tools.tool
    tools.tool = SimpleNamespace(moving_face_ids=lambda: {top.key})
    try:
        behind, _ = tools.pick_face_placement(x, y)
        assert behind is not None and behind is not top
        assert behind is walk(x, y)
    finally:
        tools.tool = real


def test_screen_boxes_are_the_eight_projected_corners(view):
    """The cached world boxes projected corner by corner give the boxes
    the old one-shot projection gave, eye-plane crossings included."""
    from views import wgpu_inference as W
    proto = Mesh()
    _box(proto, (0, 0, 0), (1, 1, 1))
    for i in range(300):
        g = _component(proto, (i % 20) * 1.5 - 15, (i // 20) * 1.5 - 10, 0)
        if i % 7 == 0:
            g.xform.rotate(30.0 * (i % 5), 0.3, 0.2, 1.0)
            g.xform.scale(1.0, 2.0, 0.5)
        view.scene.groups.append(g)
    refresh(view)
    index = view.inference_index
    packed = view.packed

    def reference(mvp, width, height):
        values = packed.instances
        matrices = values['world'].astype(float).transpose(0, 2, 1)
        bounds = packed.bounds[values['info'][:, 0], :, :3].astype(float)
        center = bounds.mean(axis=1)
        extent = (bounds[:, 1] - bounds[:, 0]) * .5
        wc = np.einsum('nij,nj->ni', matrices[:, :3, :3], center) + matrices[:, :3, 3]
        we = np.einsum('nij,nj->ni', abs(matrices[:, :3, :3]), extent)
        signs = np.array([[a, b, c] for a in (-1, 1) for b in (-1, 1) for c in (-1, 1)])
        corners = wc[:, None, :] + we[:, None, :] * signs
        matrix = np.asarray(mvp.data()).reshape(4, 4).T
        clip = corners @ matrix[:, :3].T + matrix[:, 3]
        front = clip[:, :, 3] > 1e-3
        denom = np.where(front, clip[:, :, 3], 1.)
        xy = clip[:, :, :2] / denom[:, :, None]
        xy = (xy * np.array([1., -1.]) + 1.) * np.array([width / 2, height / 2])
        low, high = xy.min(axis=1), xy.max(axis=1)
        crossing = front.any(axis=1) & ~front.all(axis=1)
        low[crossing], high[crossing] = -np.inf, np.inf
        low[~front.any(axis=1)], high[~front.any(axis=1)] = np.inf, np.inf
        return low, high

    crossings = 0
    for target, distance, pitch, yaw in ((V(0, 0, 0), 40.0, 40.0, -30.0),
                                         (V(0, 0, 0.5), 2.0, 5.0, 10.0),
                                         (V(5, 3, 0.5), 0.5, 2.0, 120.0)):
        _look(view, target, distance, pitch, yaw)
        mvp = view.camera.projection_matrix() * view.camera.view_matrix()
        want = reference(mvp, view.width(), view.height())
        lo, hi, _grid, _moved = W.screen_boxes(index, packed, mvp, view.width(), view.height())
        np.testing.assert_allclose(lo, want[0], rtol=1e-9, atol=1e-6)
        np.testing.assert_allclose(hi, want[1], rtol=1e-9, atol=1e-6)
        crossings += int(np.isneginf(lo[:, 0]).sum())
    assert crossings > 0                     # the eye-plane case was met
    # A moved placement: its world box is redone, the others kept.
    packed.instances['world'][5, 3, 0] += 2.0
    index.update([5], packed)
    mvp = view.camera.projection_matrix() * view.camera.view_matrix()
    lo, hi, _grid, _moved = W.screen_boxes(index, packed, mvp, view.width(), view.height())
    want = reference(mvp, view.width(), view.height())
    np.testing.assert_allclose(lo, want[0], rtol=1e-9, atol=1e-6)


def test_edge_grid_near_is_unique_sorted():
    from core.edge_grid import EdgeGrid
    rng = np.random.default_rng(3)
    ax, ay = rng.uniform(0, 400, 500), rng.uniform(0, 300, 500)
    bx, by = ax + rng.uniform(-60, 60, 500), ay + rng.uniform(-60, 60, 500)
    grid = EdgeGrid(ax, ay, bx, by, np.ones(500, bool), 400, 300, 64.)
    for px, py in ((10, 10), (200, 150), (399, 299)):
        got = grid.near(px, py, 48.)
        assert np.all(np.diff(got) > 0)
        # Every segment within reach is among them.
        lo_x, hi_x = np.minimum(ax, bx), np.maximum(ax, bx)
        lo_y, hi_y = np.minimum(ay, by), np.maximum(ay, by)
        gap = np.maximum(np.maximum(lo_x - px, px - hi_x), 0) ** 2 + \
            np.maximum(np.maximum(lo_y - py, py - hi_y), 0) ** 2
        assert set(np.flatnonzero(gap <= 48. ** 2)) <= set(got.tolist())

# SPDX-License-Identifier: GPL-3.0-or-later
"""Hovering an edge snaps to it at any zoom, in a model of any extent.

Reported on a large model: zoomed very close to a line, the Line tool no
longer snapped to it and Select no longer highlighted it. Close up, the
line's far end lies behind the eye; every edge search projected both ends
and dropped a segment with an end behind the eye.
"""
import math

import pytest
from built_core import Vec3 as V

from built_core.mesh import Mesh
from tests.headless_gpu import mouse
from tests.test_inference_and_face_cutting import refresh, view  # noqa: F401
from tests.test_wgpu_inference_parity import _box, _component, _look

DISTANCES = [1.0, 0.1, 0.01, 0.001]


def _scene(view, remote):
    m = view.scene.mesh
    # A long, thin slab: its 40 m edges run along x, 20 m above the origin.
    _box(m, (-20, 0, 20), (20, 0.3, 20.2))
    if remote:
        part = Mesh()
        _box(part, (0, 0, 0), (5, 5, 5))
        for x, y in [(3000, 0), (0, -2500), (-4000, 1500)]:
            view.scene.groups.append(_component(part, x, y, 0))
    refresh(view)


def _hover(view, world, dx=0.0, dy=0.0):
    x, y = view._world_to_pixel(world)
    assert x is not None
    mouse(view, "move", x + dx, y + dy)
    return view.last_snap


@pytest.mark.parametrize("remote", [False, True], ids=["near", "remote"])
@pytest.mark.parametrize("distance", DISTANCES)
def test_line_snaps_to_an_edge_close_up(view, distance, remote):
    _scene(view, remote)
    target = V(1.37, 0.0, 20.2)
    # Looking along the edge, down onto the slab: the edge's far end is in
    # front, its near end behind the eye.
    _look(view, target, distance=distance, pitch=35.0, yaw=-75.0)
    view.camera.aspect = 800 / 600
    snap = _hover(view, target, 0.0, -3.0)
    assert snap is not None and snap.kind == "on_edge", (distance, snap and snap.kind)
    assert abs(snap.point.y()) < 1e-6 and abs(snap.point.z() - 20.2) < 1e-6


@pytest.mark.parametrize("distance", DISTANCES)
def test_select_finds_a_loose_edge_close_up(view, distance):
    """Select's hover: the store's edges near the cursor (``MeshStore.nearest_edges``)."""
    _scene(view, remote=True)
    target = V(-3.21, 0.0, 20.2)
    _look(view, target, distance=distance, pitch=35.0, yaw=-75.0)
    x, y = view._world_to_pixel(target)
    edge, pixels = view.tools.pick_linework_edge(x, y - 3.0)
    assert edge is not None, distance
    assert {(round(p.y(), 6), round(p.z(), 6)) for p in (edge.a, edge.b)} == {(0.0, 20.2)}
    assert pixels < 4.0


@pytest.mark.parametrize("distance", DISTANCES)
def test_select_finds_a_component_edge_close_up(view, distance):
    """Select's hover over a placement: the snap index's edges (``reference_edge``)."""
    part = Mesh()
    _box(part, (-20, 0, 0), (20, 0.3, 0.2))
    view.scene.groups.append(_component(part, 0, 0, 20))
    far = Mesh()
    _box(far, (0, 0, 0), (5, 5, 5))
    view.scene.groups.append(_component(far, 3000, 0, 0))
    refresh(view)
    target = V(2.5, 0.0, 20.2)
    _look(view, target, distance=distance, pitch=35.0, yaw=-75.0)
    x, y = view._world_to_pixel(target)
    edge = view.tools.pick_edge_any(x, y - 3.0)
    assert edge is not None, distance
    # The snap index keeps placed ends in float32.
    assert {(round(p.y(), 4), round(p.z(), 4)) for p in (edge.a, edge.b)} == {(0.0, 20.2)}

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Tape Measure guides no longer slow every frame down (#816).

Each guide line was re-dashed in a Python loop on EVERY repaint — about
2 ms a guide, so forty guides took a frame on the Plaza model from 18 to
110 ms even with the cursor merely hovering. The dashes are built in one
NumPy pass now, and reused (with their upload) until the camera, the
guides or their selection change."""
from __future__ import annotations

from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401


from tests.raster_samples import pixels
from tests.controller_surface import live_image
import numpy as np  # noqa: E402
import pytest  # noqa: E402
from built_core import Vec3 as V


import views.document_interaction as vpmod  # noqa: E402
from built_core.guide import Guide  # noqa: E402


def test_dashes_match_a_point_by_point_construction():
    g = Guide(V(1, 2, 3), V(1, 1, 0))
    coords, spans = vpmod._guide_vertices([g], 0.5, centre=V(0, 0, 0),
                                          reach=10.0)
    a = np.frombuffer(coords, np.float32).reshape(-1, 2, 3)
    d = g.direction
    tf = V.dotProduct(V(0, 0, 0) - g.point, d)
    for k in (0, 7, len(a) - 1):
        t = tf - 10.0 + k * 0.5
        want0 = g.point + d * t
        want1 = g.point + d * (t + 0.25)
        assert np.allclose(a[k, 0], (want0.x(), want0.y(), want0.z()),
                           atol=1e-5)
        assert np.allclose(a[k, 1], (want1.x(), want1.y(), want1.z()),
                           atol=1e-5)
    assert spans == [(0, 2 * len(a), False)]


def test_the_edit_box_sharing_the_buffer_does_not_eat_the_guides(native_gpu_host):
    """The dashed box of the group being edited reuses the guides' buffer;
    a cached frame must not draw the box where the guides were."""
    from built_core.group import Group
    from built_core.mesh import Mesh
    win = native_gpu_host.window
    vp = win.viewport
    m = Mesh()
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    g = Group(m, "g")
    vp.scene.groups.append(g)
    vp.scene.guides.append(Guide(V(0, 3, 0), V(1, 0, 0)))
    vp.camera.target, vp.camera.distance = V(0.5, 1.5, 0), 8.0
    vp.camera.set_view("top")
    vp.scene.begin_group_edit(g)
    vp.scene.version += 1
    first = pixels(live_image(vp))
    second = pixels(live_image(vp))
    assert (first == second).all()
    # And the guide is really there: a row of guide-blue pixels.
    vp.scene.guides[:] = []
    bare = pixels(live_image(vp))
    assert (first != bare).any()

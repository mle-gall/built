# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""X-ray fades the edges that sit behind a face.

Every edge used to come out equally dark, so looking down into a box you
could not tell whether it had a lid: the form read the same open or closed.
Now an edge with a face in front of it is washed toward the background and
one with nothing in front keeps its full colour — the form is kept and a
face shows where it is, without the faces losing their transparency.
"""
from __future__ import annotations

import numpy as np
from ingetrazo_core import Vec3 as V
from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401


def _pixels(vp):
    width, height, rgba = vp.keys_stand_in.render_pixels(300, 300, overlays=False)
    return np.frombuffer(rgba, np.uint8).reshape(height, width, 4)[..., :3].astype(int)


def _contrast(before, after) -> int:
    """How much an edge changed the picture — a difference, since MSAA
    blends a one-pixel line with whatever lies behind it."""
    return int(np.abs(after - before).sum())


def test_an_edge_under_a_face_is_fainter_than_one_in_the_open(native_gpu_host):
    from ingetrazo_core.style import style_by_name
    vp = native_gpu_host.viewport
    scene = vp.scene
    # A lid over the left half (x < 0), two metres up; nothing on the
    # right. Seen from above.
    scene.mesh.add_face([V(-60, -60, 2), V(0, -60, 2), V(0, 60, 2),
                         V(-60, 60, 2)])
    scene.version += 1
    cam = vp.camera
    cam.target = V(0, 0, 0)
    cam.distance, cam.yaw, cam.pitch = 30.0, 0.0, 1.5
    cam.perspective = False
    vp.style_override = style_by_name("X-ray")
    bare = _pixels(vp)

    under = scene.add_edge(V(-5, -4, 0), V(-5, 4, 0))
    scene.version += 1
    hidden = _contrast(bare, _pixels(vp))
    scene.mesh.remove_edge(under)

    scene.add_edge(V(5, -4, 0), V(5, 4, 0))
    scene.version += 1
    shown = _contrast(bare, _pixels(vp))

    assert hidden > 0, "X-ray must still show the edge through the lid"
    assert hidden < 0.7 * shown, (
        f"the edge under the lid must read fainter "
        f"(hidden={hidden}, open={shown})")

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""An opaque reference image hides what lies BEHIND it, and still shows
what is traced ON it (a user's video, 25-09): a triangle traced on a photo
on the ground, pushed DOWN into a prism, painted its sides over the photo
seen from above — the image wrote no depth at all, so anything drawn after
it won. It writes depth now, pushed back by a polygon offset.

Requires a native wgpu surface for pixel readback."""
from __future__ import annotations

from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401


import numpy as np
import pytest
from tests.raster_samples import pixels
from ingetrazo_core import Vec3 as QVector3D

PHOTO = (0x30, 0xC0, 0x40)          # a green the model never uses


def _photo_share(img) -> float:
    px = pixels(img)[..., :3].astype(int)
    near = np.abs(px - np.array(PHOTO)).sum(axis=2) < 40
    return float(near.mean())


def test_a_box_below_an_opaque_image_does_not_paint_over_it(native_gpu_host, tmp_path):
    from core.image_plane import ImagePlane

    win = native_gpu_host.window
    vp = win.viewport
    scene = vp.scene
    V = QVector3D
    from tests.raster_samples import solid_raster
    pic = solid_raster(8, 8, PHOTO)
    path = tmp_path / "photo.png"
    pic.save(str(path))
    scene.image_planes.append(ImagePlane(
        str(path), V(0, 0, 0), V(10, 0, 0), V(0, 10, 0)))
    scene.version += 1
    vp.camera.set_view("iso")                  # from above the ground
    vp.camera.fit_to(V(-2, -2, -5), V(12, 12, 0))
    alone = _photo_share(vp.render_raster(400, 300, overlays=False))
    if alone < 0.05:
        # a context that draws nothing textured (the offscreen platform
        # inside the full suite): nothing here could be measured
        pytest.skip("this GL context renders no textures")
    # a box hanging BELOW the photo, wider than it, seen from above
    lo, hi = -2.0, 12.0
    p = [V(lo, lo, -5), V(hi, lo, -5), V(hi, hi, -5), V(lo, hi, -5),
         V(lo, lo, -1), V(hi, lo, -1), V(hi, hi, -1), V(lo, hi, -1)]
    for idx in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5),
                (2, 3, 7, 6), (3, 0, 4, 7)):
        scene.mesh.add_face([p[i] for i in idx])
    scene.version += 1
    under = _photo_share(vp.render_raster(400, 300, overlays=False))
    assert under == pytest.approx(alone, abs=0.01)   # the photo still shows
    # …and a face traced ON the photo still covers it
    scene.mesh.add_face([V(0, 0, 0), V(10, 0, 0), V(10, 10, 0),
                         V(0, 10, 0)])
    scene.version += 1
    traced = _photo_share(vp.render_raster(400, 300, overlays=False))
    assert traced < alone * 0.2

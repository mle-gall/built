# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A faded reference image must not switch the frame's blending off: the
shadow catcher that follows writes alpha 0 where the sun reaches, and
without blending that is a transparent hole the size of the model's
shadow square — white on screen, and only on frames that REUSE the
cached shadow map (orbit, zoom), since a rebuild re-enables blending on
its way out. Marco, 2026-09-14: «puse sombras con el tif activo y orbité».

Requires a native wgpu surface for pixel readback."""
from __future__ import annotations

from tests.raster_samples import solid_raster
from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401


import numpy as np
import pytest
from tests.raster_samples import pixels
from built_core import Vec3 as QVector3D


def _alpha_zero_share(img) -> float:
    return float((pixels(img)[..., 3] == 0).mean())


def test_a_translucent_image_leaves_blending_on_for_the_shadow_catcher(native_gpu_host, tmp_path):
    from built_core.image_plane import ImagePlane

    win = native_gpu_host.window
    vp = win.viewport
    scene = vp.scene
    V = QVector3D
    scene.mesh.add_face([V(0, 0, 0), V(4, 0, 0), V(4, 4, 0), V(0, 4, 0)])
    scene.mesh.add_face([V(1, 1, 2), V(3, 1, 2), V(3, 3, 2), V(1, 3, 2)])
    pic = solid_raster(8, 8, (48, 96, 160))
    path = tmp_path / "scan.png"
    pic.save(str(path))
    scene.image_planes.append(ImagePlane(
        str(path), V(-20, -20, 0), V(40, 0, 0), V(0, 40, 0), opacity=0.5))
    scene.shadows.enabled = True
    scene.shadows.hour = 12
    scene.version += 1
    vp.camera.set_view("iso")
    vp.camera.fit_to(V(-20, -20, 0), V(20, 20, 2))
    first = vp.render_raster(160, 120, overlays=False)     # rebuilds the map
    second = vp.render_raster(160, 120, overlays=False)    # reuses it
    assert first is not None and second is not None
    assert _alpha_zero_share(first) == 0.0
    assert _alpha_zero_share(second) == 0.0

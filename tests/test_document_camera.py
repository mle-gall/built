# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #60 (@pacaeiro): «If I do a New drawing, or open a drawing, the
Camera stays in the position where it was before». The .igz now keeps the
camera it was saved with; New goes back to the default view."""
from __future__ import annotations

import math

import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401

from ingetrazo_core import Vec3 as QVector3D
from core.scene import Scene  # noqa: E402
from formats import igz  # noqa: E402


def test_the_camera_travels_in_the_igz(tmp_path):
    scene = Scene()
    scene.camera_home = {"target": [1.0, 2.0, 3.0], "distance": 7.5,
                         "yaw": 0.3, "pitch": 0.4, "fov_deg": 50.0,
                         "perspective": False}
    path = tmp_path / "cam.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.camera_home == scene.camera_home
    scene.clear()
    assert scene.camera_home is None


def test_open_restores_the_authors_view_and_new_resets_it(native_host, tmp_path):
    from ingetrazo_render.shell_files import do_save
    win = native_host.window
    cam = win.viewport.camera
    cam.target = QVector3D(12.0, -4.0, 1.5)
    cam.distance = 33.0
    cam.yaw = 1.1
    cam.pitch = 0.2
    cam.perspective = False
    path = tmp_path / "doc.igz"
    assert do_save(win, path)
    assert win.viewport.scene.camera_home["distance"] == 33.0
    # Move the camera away, then reopen: back where the author left it.
    cam.target = QVector3D(0, 0, 0)
    cam.distance = 5.0
    cam.yaw = -0.7
    cam.perspective = True
    # The same file would only bring its tab to the front (#1066): open
    # a copy, which comes in a tab of its own with its author's view.
    import shutil
    copy = tmp_path / "copy.igz"
    shutil.copy(path, copy)
    assert win.open_path(copy)
    cam = win.viewport.camera
    assert (cam.target - QVector3D(12.0, -4.0, 1.5)).length() < 1e-6
    assert cam.distance == 33.0 and abs(cam.yaw - 1.1) < 1e-9
    assert cam.perspective is False
    # New: the default view, whatever the last document did.
    win._on_new()
    cam = win.viewport.camera
    assert cam.distance == 20.0 and cam.perspective is True
    assert abs(cam.yaw - math.radians(-45.0)) < 1e-9
    assert (cam.target - QVector3D(0, 0, 0)).length() < 1e-9


def test_the_loader_reports_progress(tmp_path):
    """Issue #59: opening a document reports milestones for the bar."""
    from ingetrazo_core import Matrix4 as QMatrix4x4
    from core.group import Group
    from core.mesh import Mesh
    scene = Scene()
    for i in range(3):
        m = Mesh()
        m.add_face([QVector3D(i, 0, 0), QVector3D(i + 1, 0, 0),
                    QVector3D(i + 1, 1, 0), QVector3D(i, 1, 0)])
        g = Group(m, f"g{i}")
        g.xform = QMatrix4x4()
        scene.groups.append(g)
    path = tmp_path / "grupos.igz"
    igz.save_scene(scene, path)
    seen = []
    back = Scene()
    igz.load_into(back, path, progress=lambda f, t: seen.append((f, t)))
    fracs = [f for f, _t in seen]
    assert fracs == sorted(fracs) and fracs[0] < 0.1 and fracs[-1] >= 0.95
    assert any("groups" in t.lower() for _f, t in seen)
    assert len(back.groups) == 3
    igz.load_into(Scene(), path)                      # no callback: as before


def test_a_broken_camera_never_blocks_the_save(native_host, tmp_path):
    # #383: a NaN in the live camera made the save refuse the whole
    # document («invalid coordinates»), so the user's work could not be
    # saved. The geometry is what matters; the file keeps its last view.
    from ingetrazo_render.shell_files import do_save
    win = native_host.window
    cam = win.viewport.camera
    path = tmp_path / "doc.igz"
    assert do_save(win, path)
    good = dict(win.viewport.scene.camera_home)
    cam.target = QVector3D(float("nan"), 0.0, 0.0)
    assert do_save(win, path)
    assert win.viewport.scene.camera_home == good
    back = Scene()
    igz.load_into(back, path)
    assert back.camera_home == good


@pytest.mark.parametrize("extension", [".built", ".igz"])
def test_document_camera_precision(tmp_path, extension):
    from types import SimpleNamespace
    import numpy as np
    from core.camera import OrbitCamera
    from ingetrazo_render import camera_record, apply_camera_record
    cam = OrbitCamera()
    cam.target = (650000.012345678, 6860000.023456789, .01)
    cam.up = (.00123456789, .00234567891, .999999)
    cam.distance = .07
    scene = Scene()
    vp = SimpleNamespace(camera=cam, scene=scene, update=lambda: None)
    scene.camera_home = camera_record(vp)
    path = tmp_path / ("eye" + extension)
    igz.save_scene(scene, path)
    restored = Scene()
    igz.load_into(restored, path)
    vp.camera = OrbitCamera()
    apply_camera_record(vp, restored.camera_home)
    for attr in ("target", "up"):
        expected = np.array(getattr(cam, attr).toTuple())
        if extension == ".igz":
            expected = expected.astype(np.float32)
        np.testing.assert_array_equal(getattr(vp.camera, attr).toTuple(), expected)
    if extension == ".built":
        np.testing.assert_array_equal(vp.camera.eye().toTuple(), cam.eye().toTuple())

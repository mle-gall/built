# SPDX-License-Identifier: GPL-3.0-or-later
"""Metal: the ground's shadows are not drawn from below the ground (#1301).

GL's catcher pass returns when ``camera.sees_from_below(-0.005)``; the
Metal view sends ``ground_shadows`` in its style payload, so that payload
must read the view's camera the same way.
"""
import math
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from ingetrazo_core import Vec3 as QVector3D
from core.camera import OrbitCamera
from core.scene import Scene
from views.wgpu_style import SHADOW_CATCHER_Z, style_payload, sun_payload


def _scene():
    scene = Scene()
    scene.loose_mesh.add_face([QVector3D(0, 0, 0), QVector3D(1, 0, 0),
                               QVector3D(0, 1, 1)])
    scene.shadows.enabled = True
    scene.shadows.hour = 12
    return scene


def _camera(pitch_deg, perspective=True, target_z=0.0):
    cam = OrbitCamera()
    cam.target = QVector3D(0.0, 0.0, target_z)
    cam.distance = 20.0
    cam.pitch = math.radians(pitch_deg)
    cam.perspective = perspective
    return cam


def test_catcher_sits_just_below_ground():
    assert SHADOW_CATCHER_Z == -0.005

def test_ground_shadows_on_from_above():
    p = sun_payload(_scene(), camera=_camera(30.0))
    assert p['shadows'] and p['ground_shadows']


def test_ground_shadows_off_from_below():
    p = sun_payload(_scene(), camera=_camera(-30.0))
    # Shadows on the model's own faces stay.
    assert p['shadows'] and not p['ground_shadows']


def test_parallel_judges_by_view_direction():
    assert sun_payload(_scene(), camera=_camera(30.0, False, -50.0))['ground_shadows']
    assert not sun_payload(_scene(), camera=_camera(-30.0, False, 50.0))['ground_shadows']


def test_style_payload_passes_the_camera():
    scene = _scene()
    assert style_payload(scene, camera=_camera(30.0))['ground_shadows']
    assert not style_payload(scene, camera=_camera(-30.0))['ground_shadows']
    # No camera (tests, headless callers): unchanged default.
    assert style_payload(scene)['ground_shadows']

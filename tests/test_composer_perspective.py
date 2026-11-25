# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A sheet frame in PERSPECTIVE, with its own sun (Marco, 2026-09-17:
«debería haber otra vista 3d con opción de sombras o algo así que se vea
como un 3d de verdad como si viera en campo … la vista 3d como
perspectiva»).

Until this existed ``apply_frame_camera`` forced ``camera.perspective =
False`` on every frame, so a sheet could only ever hold parallel drawings —
an axonometric was the closest thing to a 3D, and a scene saved in
perspective rendered flat anyway. These fix the three things that makes
true: the camera, the page projection that follows from it, and the fact
that such a frame has no scale to print."""
from __future__ import annotations

import math
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from ingetrazo_core import Vec3 as QVector3D

from core.camera import OrbitCamera
from core.composition import (Composicion, MarcoVista, apply_frame_camera,
                              fit_distance_for_scene, frame_page_projector,
                              model_height_for_frame)
from core.saved_views import SavedView
from core.scene import Scene
from tests.test_composer_canvas import _FakeViewport
V = QVector3D


def _scene_with_a_box() -> Scene:
    scene = Scene()
    scene.mesh.add_face([V(0, 0, 0), V(6, 0, 0), V(6, 0, 3), V(0, 0, 3)])
    return scene


# ── The camera ──────────────────────────────────────────────────────────────

def test_a_parallel_frame_still_frames_by_its_exact_scale():
    """The contract every existing sheet rests on, unchanged."""
    cam, scene = OrbitCamera(), _scene_with_a_box()
    frame = MarcoVista(w_mm=100.0, h_mm=80.0, scale_n=50.0)
    apply_frame_camera(cam, frame, None, scene)
    assert cam.perspective is False
    shown = 2.0 * cam.distance * math.tan(math.radians(cam.fov_deg) / 2.0)
    assert shown == pytest.approx(model_height_for_frame(80.0, 50.0))


def test_a_perspective_frame_gets_a_vanishing_point_and_its_own_eye():
    cam, scene = OrbitCamera(), _scene_with_a_box()
    frame = MarcoVista(w_mm=100.0, h_mm=80.0, perspective=True,
                       cam_distance=12.5, cam_fov=35.0)
    apply_frame_camera(cam, frame, None, scene)
    assert cam.perspective is True
    assert cam.distance == pytest.approx(12.5)
    assert cam.fov_deg == pytest.approx(35.0)


def test_a_perspective_frame_bound_to_a_scene_stands_where_the_scene_did():
    """The scene is the natural source of the eye: the user already walked
    there in the model."""
    cam, scene = OrbitCamera(), _scene_with_a_box()
    view = SavedView("De campo", target=(3.0, 0.0, 1.6), distance=22.0,
                     fov_deg=60.0, perspective=True)
    scene.saved_views.append(view)
    frame = MarcoVista(view_key="scene:De campo", perspective=True)
    apply_frame_camera(cam, frame, view, scene)
    assert cam.perspective is True
    assert cam.distance == pytest.approx(22.0)
    assert cam.fov_deg == pytest.approx(60.0)


def test_a_perspective_frame_with_no_eye_at_all_fits_the_model():
    """std:iso turned perspective has no distance anywhere — it must not
    open standing inside a wall."""
    cam, scene = OrbitCamera(), _scene_with_a_box()
    frame = MarcoVista(view_key="std:iso", perspective=True)
    apply_frame_camera(cam, frame, None, scene)
    assert cam.perspective is True
    assert cam.distance == pytest.approx(
        fit_distance_for_scene(scene, cam.fov_deg))
    assert cam.distance > 3.0             # outside the 6×3 m box


def test_the_frames_own_turn_still_applies_in_perspective():
    cam, scene = OrbitCamera(), _scene_with_a_box()
    frame = MarcoVista(perspective=True, cam_distance=10.0,
                       cam_yaw=0.25, cam_pitch=0.1)
    apply_frame_camera(cam, frame, None, scene)
    assert cam.yaw == pytest.approx(0.25)
    assert cam.pitch == pytest.approx(0.1)


# ── The page projection ─────────────────────────────────────────────────────

def test_the_parallel_projector_is_the_scale_arithmetic_it_replaced():
    frame = MarcoVista(w_mm=100.0, h_mm=80.0, scale_n=50.0)
    to_page = frame_page_projector(frame, OrbitCamera())
    model_h = model_height_for_frame(80.0, 50.0)
    k = 80.0 / model_h
    half_h, half_w = model_h / 2.0, (model_h / 2.0) * (100.0 / 80.0)
    px, py = to_page(1.25, -0.5, 30.0)          # the depth is ignored
    assert px == pytest.approx((1.25 + half_w) * k)
    assert py == pytest.approx((half_h + 0.5) * k)


def test_the_perspective_projector_divides_by_depth():
    """The whole difference between an axonometric and standing there: the
    same object, twice as far, draws half the size."""
    frame = MarcoVista(w_mm=100.0, h_mm=80.0, perspective=True)
    cam = OrbitCamera()
    cam.fov_deg = 45.0
    to_page = frame_page_projector(frame, cam)
    near_x, near_y = to_page(1.0, 1.0, 10.0)
    far_x, far_y = to_page(1.0, 1.0, 20.0)
    centre_x, centre_y = to_page(0.0, 0.0, 10.0)
    assert (centre_x, centre_y) == pytest.approx((50.0, 40.0))  # dead centre
    assert near_x - centre_x == pytest.approx(2.0 * (far_x - centre_x))
    assert centre_y - near_y == pytest.approx(2.0 * (centre_y - far_y))


def test_a_point_behind_the_eye_leaves_the_page_by_a_bounded_amount():
    """It has no projection at all; a polyline crossing the eye plane must
    still head off the frame rather than to the next galaxy."""
    frame = MarcoVista(w_mm=100.0, h_mm=80.0, perspective=True)
    px, py = frame_page_projector(frame, OrbitCamera())(1.0, 1.0, -5.0)
    assert not (0.0 <= px <= 100.0 and 0.0 <= py <= 80.0)
    assert abs(px) < 1e4 and abs(py) < 1e4


def test_the_projector_takes_arrays_whole():
    import numpy as np
    frame = MarcoVista(w_mm=100.0, h_mm=80.0, perspective=True)
    to_page = frame_page_projector(frame, OrbitCamera())
    px, py = to_page(np.array([0.0, 1.0]), np.array([0.0, 1.0]),
                     np.array([10.0, 10.0]))
    assert px.shape == (2,) and py.shape == (2,)
    assert (px[0], py[0]) == pytest.approx((50.0, 40.0))


# ── No scale on paper ───────────────────────────────────────────────────────


# ── The frame's own sun ─────────────────────────────────────────────────────


# ── The composer around it ──────────────────────────────────────────────────


def test_the_perspective_switch_survives_a_save_and_a_load():
    comp = Composicion(name="Lámina 4")
    comp.frames = [MarcoVista(perspective=True, cam_distance=18.0,
                              cam_fov=35.0, shadows=True, sun_hour=10.0)]
    back = Composicion.from_dict(comp.to_dict()).frames[0]
    assert back.perspective is True
    assert back.cam_distance == pytest.approx(18.0)
    assert back.cam_fov == pytest.approx(35.0)
    assert back.shadows is True
    assert back.sun_hour == pytest.approx(10.0)


def test_an_old_sheet_opens_parallel_even_pointing_at_a_perspective_scene():
    """No silent inheritance: a document made before this existed must draw
    exactly what it drew (half the scenes of a model are saved in
    perspective from modelling)."""
    frame = Composicion.from_dict(
        {"frames": [{"view_key": "scene:Escena 9", "scale_n": 417.7}]}
    ).frames[0]
    assert frame.perspective is False
    cam, scene = OrbitCamera(), _scene_with_a_box()
    view = SavedView("Escena 9", distance=40.0, perspective=True)
    scene.saved_views.append(view)
    apply_frame_camera(cam, frame, view, scene)
    assert cam.perspective is False




def test_projector_broadcasts_each_coordinate_with_depth():
    import numpy as np
    from types import SimpleNamespace

    frame = MarcoVista(w_mm=120., h_mm=60., perspective=True)
    project = frame_page_projector(frame, SimpleNamespace(fov_deg=60.))
    x = np.array([[.2], [-.3], [.5]])
    y = np.array([.4, .8])
    depths = np.array([[1., -1.]])
    px, py = project(x, y, depths)
    half_height = np.tan(np.deg2rad(30.)) * np.maximum(depths, .0001)
    np.testing.assert_allclose(px, np.clip((x / (2 * half_height) + 1) * 60., -1200., 1320.))
    np.testing.assert_allclose(py, np.clip((1 - y / half_height) * 30., -600., 660.))
    assert px.shape == (3, 2) and py.shape == (1, 2)
    assert np.isnan(project(1., 2., float("nan"))[0])
    with pytest.raises(ValueError, match="depth"):
        project(1., 2.)

    frame.perspective = False
    px, py = frame_page_projector(frame, None)(x.astype("float32"), y)
    assert px.shape == x.shape and py.shape == y.shape
    assert px.dtype == np.float32

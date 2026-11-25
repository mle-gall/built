# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Editing a frame's view in place (the usual convention for sheets:
double-click the viewport, then pan / orbit / zoom; Zoom Extents recentres the model)."""
from __future__ import annotations

import math

import pytest
from built_core import Vec3 as QVector3D

from core.composition import MarcoVista, apply_frame_camera


class _Cam:
    def __init__(self):
        self.fov_deg = 45.0
        self.perspective = True
        self.distance = 7.0
        self.aspect = 1.6
        self.yaw = 0.0
        self.pitch = 0.0
        self.up = QVector3D(0, 0, 1)
        self.target = QVector3D(0, 0, 0)

    def set_view(self, key):
        self.yaw, self.pitch = {"front": (math.radians(-90.0), 0.0),
                                "top": (math.radians(-90.0),
                                        math.radians(89.0))}[key]


class _Scene:
    def bounds(self):
        return QVector3D(0, 0, 0), QVector3D(4, 4, 2)


def test_frame_camera_overrides_apply_after_the_view():
    cam = _Cam()
    f = MarcoVista(view_key="std:front", scale_n=100.0,
                   cam_target=[1.0, 2.0, 3.0], cam_yaw=0.3, cam_pitch=0.2)
    apply_frame_camera(cam, f, saved_view=None, scene=_Scene())
    assert (cam.target.x(), cam.target.y(), cam.target.z()) == (1, 2, 3)
    assert (cam.yaw, cam.pitch) == (0.3, 0.2)
    # Orbiting off a plan view: Z is up again (the plan swapped it to +Y).
    cam = _Cam()
    f = MarcoVista(view_key="std:top", scale_n=100.0, cam_pitch=0.5)
    apply_frame_camera(cam, f, saved_view=None, scene=_Scene())
    assert cam.pitch == 0.5 and cam.up.z() == 1.0
    cam = _Cam()
    apply_frame_camera(cam, MarcoVista(view_key="std:top"), scene=_Scene())
    assert cam.up.y() == 1.0                      # untouched plan keeps +Y


def test_view_edits_travel_in_the_document(tmp_path):
    from core.composition import Composicion
    c = Composicion()
    f = MarcoVista(cam_target=[1.0, 2.0, 3.0], cam_yaw=0.1, scale_n=75.0)
    c.frames.append(f)
    back = Composicion.from_dict(c.to_dict())
    g = back.frames[-1]
    assert list(g.cam_target) == [1.0, 2.0, 3.0]
    assert g.cam_yaw == 0.1 and g.cam_pitch is None and g.scale_n == 75.0



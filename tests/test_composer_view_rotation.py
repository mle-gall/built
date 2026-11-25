# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Turning a model view on the sheet (Marco, 2026-09-07: «en compositor de
láminas, debería poder rotar un model view»).

The frame does not move: the DRAWING inside it turns, clockwise on paper like
the north arrow's angle, because ``rot_deg`` is a roll of the frame's camera.
Everything projected through that camera — the render, the hidden-line pass,
snap points, anchored cotas — turns with it by construction.
"""
from __future__ import annotations

import math

import pytest
from ingetrazo_core import ScreenPoint as QPointF
from ingetrazo_core import Vec3 as QVector3D

from core.composition import (Composicion, MarcoVista, apply_frame_camera,
                              roll_camera)


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


def test_a_frame_without_a_turn_leaves_the_camera_exactly_as_it_was():
    cam = _Cam()
    apply_frame_camera(cam, MarcoVista(view_key="std:top"), scene=_Scene())
    assert (cam.up.x(), cam.up.y(), cam.up.z()) == (0.0, 1.0, 0.0)


def test_a_quarter_turn_on_a_plan_swaps_the_up_vector():
    cam = _Cam()
    f = MarcoVista(view_key="std:top", rot_deg=90.0)
    apply_frame_camera(cam, f, scene=_Scene())
    # Plan: forward −Z, up +Y, right +X. A clockwise quarter turn of the
    # DRAWING points the camera's up at −X, so east lands at the bottom.
    assert cam.up.x() == pytest.approx(-1.0, abs=1e-6)
    assert cam.up.y() == pytest.approx(0.0, abs=1e-6)
    assert cam.up.z() == pytest.approx(0.0, abs=1e-6)


def test_the_roll_keeps_the_up_vector_square_to_the_sight_line():
    cam = _Cam()
    cam.yaw, cam.pitch = math.radians(37.0), math.radians(21.0)
    roll_camera(cam, 33.0)
    cp, sp = math.cos(cam.pitch), math.sin(cam.pitch)
    cy, sy = math.cos(cam.yaw), math.sin(cam.yaw)
    fwd = (-cp * cy, -cp * sy, -sp)
    up = (cam.up.x(), cam.up.y(), cam.up.z())
    # QVector3D stores single precision, hence the 1e-6.
    assert sum(f * u for f, u in zip(fwd, up)) == pytest.approx(0.0, abs=1e-6)
    assert math.sqrt(sum(u * u for u in up)) == pytest.approx(1.0, abs=1e-6)


def test_the_turn_survives_the_igz_round_trip():
    c = Composicion()
    c.frames = [MarcoVista(rot_deg=-37.5)]
    back = Composicion.from_dict(c.to_dict())
    assert back.frames[0].rot_deg == -37.5
    # a sheet written before this version simply has no turn
    d = c.to_dict()
    d["frames"][0].pop("rot_deg")
    assert Composicion.from_dict(d).frames[0].rot_deg == 0.0


# ---- In the composer, on a real model ---------------------------------------


def _centre(frame):
    return (frame.x_mm + frame.w_mm / 2.0, frame.y_mm + frame.h_mm / 2.0)



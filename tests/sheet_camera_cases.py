# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Frames and views of the frame-camera parity goldens
(tests/test_sheet_camera_parity.py): the camera ``core.composition.
apply_frame_camera`` points at each frame, from a default OrbitCamera, in a
scene of three boxes. Written before the camera moved to Rust
(docs/rust-sheets.md step 4)."""
from __future__ import annotations

import math


def scene():
    from ingetrazo_core import Vec3 as V
    from core.saved_views import SavedView
    from core.scene import Scene
    s = Scene()
    for (x0, y0, z0, x1, y1, z1) in ((0, 0, 0, 12, 8, 0.3), (1, 1, 0.3, 5, 4, 3.2), (6, 1.5, 0.3, 11, 6, 2.6)):
        p = [V(x, y, z) for z in (z0, z1) for y in (y0, y1) for x in (x0, x1)]
        for loop in ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4), (1, 3, 7, 5), (3, 2, 6, 7), (2, 0, 4, 6)):
            s.mesh.add_face([p[i] for i in loop])
    s.saved_views.append(SavedView("Iso", target=(6.0, 4.0, 1.5), distance=24.0, perspective=False))
    s.saved_views.append(SavedView("Eye", target=(5.0, 3.0, 1.2), distance=14.0, yaw=0.4, pitch=0.2,
                                   fov_deg=60.0, perspective=True))
    s.saved_views.append(SavedView("Rise", target=(5.0, 3.0, 2.0), distance=18.0, yaw=-1.2, pitch=0.05,
                                   perspective=True, two_point=True,
                                   lens={"shift_y": 0.15, "roll_deg": 4.0, "clip_near": 2.0}))
    s.saved_views.append(SavedView("Plan", target=(6.0, 4.0, 0.0), distance=30.0, yaw=-math.pi / 2,
                                   pitch=math.radians(89.0), perspective=False,
                                   lens={"shift_x": -0.1, "clip_near": -5.0, "clip_far": 40.0}))
    return s


def frames():
    """``(name, MarcoVista kwargs)``."""
    out = []
    for key in ("std:top", "std:bottom", "std:front", "std:back", "std:right", "std:left", "std:iso",
                "__current__", "scene:Iso", "scene:Eye", "scene:Rise", "scene:Plan"):
        out.append((f"{key}", dict(view_key=key)))
    out += [
        ("rot30-top", dict(view_key="std:top", rot_deg=30.0)),
        ("rot-90-front", dict(view_key="std:front", rot_deg=-90.0)),
        ("wide-iso-1:50", dict(view_key="std:iso", w_mm=260.0, h_mm=120.0, scale_n=50.0)),
        ("tiny-2:1", dict(view_key="std:right", w_mm=30.0, h_mm=45.0, scale_n=0.5)),
        ("override-orbit", dict(view_key="std:top", cam_yaw=0.3, cam_pitch=0.6,
                                cam_target=[3.0, 2.0, 1.0])),
        ("override-plan", dict(view_key="scene:Iso", cam_pitch=math.radians(90.0),
                               cam_target=[4.0, 4.0, 0.0], rot_deg=15.0)),
        ("persp-fit", dict(view_key="std:iso", perspective=True)),
        ("persp-view", dict(view_key="scene:Eye", perspective=True, rot_deg=10.0)),
        ("persp-own", dict(view_key="scene:Iso", perspective=True, cam_fov=30.0, cam_distance=40.0)),
        ("persp-clamped", dict(view_key="std:front", perspective=True, cam_fov=200.0, cam_distance=-3.0)),
    ]
    return out


def python_camera(scn, kw):
    """``(view 16, projection 16, fields)`` of the frame's camera."""
    from core.camera import OrbitCamera
    from core.composition import MarcoVista, apply_frame_camera
    frame = MarcoVista(**kw)
    view = None
    if frame.view_key.startswith("scene:"):
        view = next(v for v in scn.saved_views if v.name == frame.view_key[6:])
    cam = OrbitCamera()
    apply_frame_camera(cam, frame, view, scn)
    t, u = cam.target, cam.up
    fields = {"target": [t.x(), t.y(), t.z()], "up": [u.x(), u.y(), u.z()], "distance": cam.distance,
              "fov_deg": cam.fov_deg, "perspective": cam.perspective, "aspect": cam.aspect,
              "yaw": cam.yaw, "pitch": cam.pitch}
    return list(cam.view_matrix().data()), list(cam.projection_matrix().data()), fields

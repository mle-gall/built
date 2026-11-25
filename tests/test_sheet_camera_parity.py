# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The frame camera in Rust (built_core sheets::camera) is the one
built_core.composition.apply_frame_camera pointed: the same view and projection
matrices and camera fields for standard views, saved views (lens shift,
roll, clips, two-point), the frame's own orbit, turn and scale, and
perspective frames. Goldens from the Python, before it moved
(tests/sheet_camera_cases.py)."""
from __future__ import annotations

import gzip
import json
import os
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402

import built_core as NATIVE  # noqa: E402
from tests.sheet_camera_cases import frames, scene  # noqa: E402
GOLDENS = json.loads(gzip.decompress(
    (Path(__file__).parent / "fixtures" / "sheet_camera" / "goldens.json.gz").read_bytes()))
FRAMES = frames()


def test_every_frame_has_a_golden():
    assert sorted(GOLDENS) == sorted(n for n, _ in FRAMES)


@pytest.mark.parametrize("name,kw", FRAMES, ids=[n for n, _ in FRAMES])
def test_the_rust_frame_camera_is_the_python_one(name, kw):
    from built_core.composition import MarcoVista
    s = scene()
    frame = MarcoVista(**kw)
    view = next((v for v in s.saved_views if f"scene:{v.name}" == frame.view_key), None)
    lo, hi = s.bounds()
    got = NATIVE.sheet_frame_camera(frame, view, None, (lo.toTuple(), hi.toTuple()))
    want_view, want_proj, want = GOLDENS[name]
    assert got["view"] == pytest.approx(want_view, abs=1e-9)
    assert got["projection"] == pytest.approx(want_proj, rel=1e-12, abs=1e-12)
    for k in ("distance", "fov_deg", "aspect", "yaw", "pitch"):
        assert got[k] == pytest.approx(want[k], abs=1e-12), k
    assert got["perspective"] == want["perspective"]
    assert list(got["target"]) == pytest.approx(want["target"], abs=1e-12)
    assert list(got["up"]) == pytest.approx(want["up"], abs=1e-12)

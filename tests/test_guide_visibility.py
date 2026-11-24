# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Guide lines must survive perspective: the ±10 km 'infinite' segment often
has an endpoint BEHIND the camera, where _world_to_pixel returns None — the
whole guide used to vanish from render, snap and pick (the "protractor doesn't
mark a guide I can draw on" report)."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import Vec3 as QVector3D


def _window_with_guide(native_host):
    from core.guide import Guide

    win = native_host.window
    vp = win.viewport
    native_host.resize(1200, 800, 1.0)
    # A guide running from the origin TOWARD the eye: its far endpoint lands
    # behind the camera (the everyday case after orbiting to draw on it).
    d = (vp.camera.eye() - QVector3D(0, 0, 0)).normalized()
    g = Guide(QVector3D(0, 0, 0), d)
    vp.scene.guides.append(g)
    return win, vp, g


def test_clip_segment_front_recovers_projectable_span(native_host):
    win, vp, g = _window_with_guide(native_host)
    a, b = g.segment()
    raw = [vp._world_to_pixel(a), vp._world_to_pixel(b)]
    assert None in raw                      # the bug's precondition holds
    seg = vp._clip_segment_front(a, b)
    assert seg is not None
    pa, pb = vp._world_to_pixel(seg[0]), vp._world_to_pixel(seg[1])
    assert pa is not None and pb is not None


def test_snap_scene_feeds_clipped_guides_and_points(native_host):
    from core.guide import Guide
    win, vp, g = _window_with_guide(native_host)
    vp.scene.guides.append(Guide(QVector3D(2, 3, 0)))    # a guide point
    snap_scene = vp._snap_scene(600, 400)
    extra = [e for e in snap_scene.edges]
    projectable = [
        e for e in extra
        if vp._world_to_pixel(e.a) is not None
        and vp._world_to_pixel(e.b) is not None
    ]
    # The clipped guide line and the guide point both made it in with
    # endpoints the snap engine can project.
    assert len(projectable) >= 2
    pts = [e for e in extra if (e.a - QVector3D(2, 3, 0)).length() < 1e-9]
    assert pts                              # the guide point pseudo-edge


def test_on_guide_snap_in_perspective(native_host):
    from core.snap import compute_snap
    win, vp, g = _window_with_guide(native_host)
    # A visible point ON the guide, a bit in front of the camera.
    p = g.point + g.direction * 3.0
    px = vp._world_to_pixel(p)
    assert px is not None
    snap = compute_snap(
        candidate_world=p,
        candidate_pixel=px,
        scene=vp._snap_scene(*px),
        world_to_pixel=vp._world_to_pixel,
        threshold_px=vp.snap_threshold_px,
        edge_threshold_px=vp.edge_snap_threshold_px,
    )
    assert snap.kind != "none"              # the guide is snappable again
    # And the snapped point lies on the guide line.
    off = snap.point - g.point
    along = QVector3D.dotProduct(off, g.direction)
    assert (off - g.direction * along).length() < 1e-3


def test_pixel_clip_preserves_border_and_endpoint_order():
    from types import SimpleNamespace
    from views.document_interaction import DocumentInteraction
    view = SimpleNamespace(width=lambda: 100, height=lambda: 50)
    clip = DocumentInteraction._clip_pixel_line
    assert clip(view, (200, 25), (-100, 25), 0) == ((100, 25), (0, 25))
    assert clip(view, (0, -50), (0, 80), 0) == ((0, 0), (0, 50))
    assert clip(view, (-1, 20), (-1, 40), 0) is None
    assert clip(view, (42, 8), (42, 8), 0) == ((42, 8), (42, 8))

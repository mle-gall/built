# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A guide behind a wall stays behind it (issue #23, @pacaeiro).

«Guide lines are always in front of other objects, even if they are behind
the object, in the scene.» They were painted in the QPainter overlay, which
runs AFTER the render and has no depth buffer — on top was the only place
they could be.

The cheap fix does not exist: asking the pick index per sample point
(``_is_occluded``, what the snap engine uses) measured **618 µs a ray** on
the Plaza Yanque model, so ten guides would have cost 200 ms a frame. The
depth buffer already knows, for free, so the lines moved into the GL pass —
same recipe as the axes, world-space dashes and depth-write off.
"""
from __future__ import annotations

from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401

import numpy as np
import pytest
from ingetrazo_render import SurveyRaster
from ingetrazo_core import Vec3 as QVector3D

from core.guide import Guide
from views.document_interaction import _guide_vertices

V = QVector3D


# ── The dash generator (no GL needed) ───────────────────────────────────────

def test_the_generator_lays_dashes_along_the_guide():
    g = Guide(V(0, 0, 1), V(1, 0, 0))            # a line along +X at z = 1
    coords, spans = _guide_vertices([g], spacing=1.0)
    assert len(spans) == 1
    start, count, selected = spans[0]
    assert start == 0 and count > 2 and count % 2 == 0   # pairs: GL_LINES
    assert selected is False
    pts = np.asarray(coords, dtype=float).reshape(-1, 3)
    assert np.allclose(pts[:, 1], 0.0)          # never leaves the guide…
    assert np.allclose(pts[:, 2], 1.0)          # …in either direction
    # gaps, not one solid line: consecutive dashes do not touch
    assert pts[1, 0] < pts[2, 0] - 1e-9


def test_a_guide_point_produces_no_dashes():
    """Points are screen-space crosses and stay in the overlay."""
    coords, spans = _guide_vertices([Guide(V(1, 2, 3))], spacing=1.0)
    assert spans == [] and len(coords) == 0


def test_the_selected_guide_is_flagged_for_its_own_colour():
    g1, g2 = Guide(V(0, 0, 0), V(1, 0, 0)), Guide(V(0, 0, 0), V(0, 1, 0))
    _coords, spans = _guide_vertices([g1, g2], spacing=1.0, selection=[g2])
    assert [sel for _s, _c, sel in spans] == [False, True]


def test_the_dash_count_is_capped_however_long_the_guide():
    """A guide is half a million metres long: at a fine spacing that would
    be millions of segments a frame. The spacing grows instead."""
    g = Guide(V(0, 0, 0), V(1, 0, 0))
    _coords, spans = _guide_vertices([g], spacing=0.001)
    _start, count, _sel = spans[0]
    assert count <= 8000                        # 4000 dashes × 2 vertices


def test_a_guide_circle_lays_dashes_on_its_rim():
    from core.guide import GuideCircle
    g = GuideCircle(V(1, 1, 0), V(0, 0, 1), 2.0)   # radius 2, in the XY plane
    coords, spans = _guide_vertices([g], spacing=0.5)
    assert len(spans) == 1
    _start, count, selected = spans[0]
    assert count >= 4 and count % 2 == 0 and selected is False
    pts = np.asarray(coords, dtype=float).reshape(-1, 3)
    r = np.hypot(pts[:, 0] - 1.0, pts[:, 1] - 1.0)
    assert np.all(np.abs(r - 2.0) < 1e-4)          # every vertex on the rim
    assert np.all(np.abs(pts[:, 2]) < 1e-6)        # and in the circle's plane


def test_a_zero_radius_guide_circle_produces_nothing():
    from core.guide import GuideCircle
    coords, spans = _guide_vertices(
        [GuideCircle(V(0, 0, 0), V(0, 0, 1), 0.0)], spacing=1.0)
    assert spans == [] and len(coords) == 0


def test_pick_guide_picks_a_guide_circle():
    import types
    from types import SimpleNamespace
    from core.guide import GuideCircle
    from core.scene import Scene
    from tests.controller_surface import ControllerSurface as Viewport

    scene = Scene()
    g = GuideCircle(V(0, 0, 0), V(0, 0, 1), 2.0)
    scene.guides.append(g)

    def w2p(p):
        return (p.x() * 100.0, p.y() * 100.0)

    stub = SimpleNamespace(scene=scene, _world_to_pixel=w2p,
                           pick_threshold_px=8.0)
    stub._circle_pick_distance = types.MethodType(
        Viewport._circle_pick_distance, stub)
    assert Viewport.pick_guide(stub, 200.0, 0.0) is g    # on the rim at (2,0)
    assert Viewport.pick_guide(stub, 0.0, 0.0) is None   # far from the rim


def test_a_small_guide_circle_still_gets_enough_dashes():
    """A circle smaller than one dash period must not collapse to a single
    camera-sized dash: its dashes shrink so the ring stays visibly dashed."""
    from core.guide import GuideCircle
    from views.document_interaction import _MIN_CIRCLE_DASHES
    coords, spans = _guide_vertices(
        [GuideCircle(V(0, 0, 0), V(0, 0, 1), 0.02)], spacing=1.0)
    assert len(spans) == 1
    _start, count, _sel = spans[0]
    assert count >= 2 * _MIN_CIRCLE_DASHES and count % 2 == 0


# ── The real thing: pixels ──────────────────────────────────────────────────


def _pixels(raster):
    return np.frombuffer(raster.rgba(), np.uint8).reshape(raster.height, raster.width, 4)[..., :3].astype(int)


def _live_pixels(controller):
    display = controller.keys_stand_in
    display.set_scene(controller.scene)
    display._prepare_pick_frame()
    display.renderer.request_capture()
    display.draw_frame()
    pixels = bytes(display.renderer.take_capture())
    width = round(display.width() * display.devicePixelRatioF())
    assert pixels and len(pixels) % (width * 4) == 0
    return _pixels(SurveyRaster(width, len(pixels) // (width * 4), pixels))


def _changed(before, after) -> int:
    """Pixels the guide altered. Measured as a DIFFERENCE, not by matching
    its colour: a one-pixel GL line under 4× MSAA resolves to a 50 % blend
    with whatever is behind it, so there is no single «guide colour» on
    screen to look for."""
    return int((np.abs(after - before).sum(axis=2) > 8).sum())


def _scene_looking_down_minus_y(native_gpu_host):
    """A wall in the XZ plane at y = 0, seen from the eye at y = +20. So
    y > 0 is IN FRONT of the wall and y < 0 is behind it.

    The wall is WIDER than the view on purpose: a 10 m one covers only the
    middle of a ~17 m window, and the ends of the guide poking past its
    edges are legitimately visible — they would read as a failure when they
    are the tool working."""
    win, vp = native_gpu_host.window, native_gpu_host.viewport
    scene = vp.scene
    scene.mesh.add_face([V(-60, 0, -60), V(60, 0, -60), V(60, 0, 60),
                         V(-60, 0, 60)])
    scene.version += 1
    cam = vp.camera
    cam.target = V(0, 0, 0)
    cam.distance, cam.yaw, cam.pitch = 20.0, 1.5707963, 0.0
    cam.perspective = False
    return win, vp


def test_a_wall_hides_the_guide_behind_it(native_gpu_host):
    win, vp = _scene_looking_down_minus_y(native_gpu_host)
    assert vp.camera.eye().y() > 0          # the eye really is on +Y
    bare = _live_pixels(vp)

    vp.scene.guides[:] = [Guide(V(0, 4, 0), V(1, 0, 0))]     # in front
    shown = _changed(bare, _live_pixels(vp))

    vp.scene.guides[:] = [Guide(V(0, -4, 0), V(1, 0, 0))]    # behind
    hidden = _changed(bare, _live_pixels(vp))

    assert shown > 50, "a guide in front of the wall must be drawn"
    assert hidden == 0, (
        f"the wall must hide the guide behind it "
        f"(hidden={hidden}px, shown={shown}px)")


def test_a_sheet_render_still_carries_no_guides(native_gpu_host):
    """A styled composer frame never drew the scaffolding and must not
    start now that the pass moved into GL."""
    win, vp = _scene_looking_down_minus_y(native_gpu_host)
    from ingetrazo_core.style import style_by_name
    vp.style_override = style_by_name("Architectural")
    try:
        bare = _pixels(vp.render_raster(300, 300, overlays=False))
        vp.scene.guides[:] = [Guide(V(0, 4, 0), V(1, 0, 0))]
        after = _pixels(vp.render_raster(300, 300, overlays=False))
    finally:
        vp.style_override = None
    assert _changed(bare, after) == 0


def test_mixed_guide_spans_preserve_segment_end_and_circle_phase():
    from core.guide import GuideCircle
    line = Guide(V(2, 0, 0), V(1, 0, 0))
    segment = Guide(V(1.25, 0, 0), origin=V(0, 0, 0))
    circle = GuideCircle(V(0, 0, 0), V(0, 0, 1), 1.0)
    coords, spans = _guide_vertices([line, Guide(V()), segment, circle], 1.0,
                                    reach=1.0, selection=[circle])
    assert spans == [(0, 4, False), (4, 4, False), (8, 48, True)]
    pts = np.frombuffer(coords, np.float32).reshape(-1, 3)
    assert pts[:8, 0].tolist() == [-1, -.5, 0, .5, 0, .5, 1, 1.25]
    from core.axes import plane_axes
    u, _ = plane_axes(circle.normal)
    assert np.allclose(pts[8], u.toTuple())

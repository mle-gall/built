# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Ambient occlusion of the 3D view (#954): off, the picture is the one it
always was; on, the foot of a box on a floor darkens and the open floor
does not; a flat face seen obliquely gets no vertical bands (#989); the
foliage mark rides the recorded instanced passes (the native replay gained
glStencilFunc for it, in the same order on both sides).

The raster cases read the hidden native wgpu surface."""
from __future__ import annotations

import math

import numpy as np
import pytest
from built_core import Vec3 as QVector3D
from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401

from built_render import ao_settings  # noqa: E402


def test_it_is_off_unless_asked(monkeypatch):
    monkeypatch.delenv("BUILT_AO", raising=False)
    assert ao_settings() is None
    monkeypatch.setenv("BUILT_AO", "0")
    assert ao_settings() is None


def test_the_settings_are_kept_in_range(monkeypatch):
    monkeypatch.setenv("BUILT_AO", "1")
    monkeypatch.setenv("BUILT_AO_RADIUS", "-3")
    monkeypatch.setenv("BUILT_AO_SAMPLES", "500")
    monkeypatch.setenv("BUILT_AO_INTENSITY", "nada")
    s = ao_settings()
    assert s["radius"] > 0 and s["samples"] == 64 and s["intensity"] == 1.0


def _lum(frame) -> np.ndarray:
    width, height, data = frame
    return np.frombuffer(data, np.uint8).reshape(height, width, 4)[..., :3].astype(
        np.float32).mean(axis=2)


@pytest.fixture
def box_on_floor(native_gpu_host):
    vp = native_gpu_host.viewport
    V = QVector3D
    scene = vp.scene
    scene.mesh.add_face([V(-10, -10, 0), V(10, -10, 0), V(10, 10, 0),
                         V(-10, 10, 0)])
    p = [V(-1, -1, 0), V(1, -1, 0), V(1, 1, 0), V(-1, 1, 0),
         V(-1, -1, 2), V(1, -1, 2), V(1, 1, 2), V(-1, 1, 2)]
    for idx in ((4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5),
                (2, 3, 7, 6), (3, 0, 4, 7)):
        scene.mesh.add_face([p[i] for i in idx])
    scene.version += 1
    vp.camera.set_view("iso")
    vp.camera.fit_to(V(-4, -4, 0), V(4, 4, 2))
    return vp



@pytest.fixture
def step_block(native_gpu_host):
    """The report's step (#989): a slab with a wall set back on it. Seen
    from an oblique camera, the wall's left face is a flat surface whose AO
    used to come out in vertical bands."""
    vp = native_gpu_host.viewport
    V = QVector3D
    scene = vp.scene

    def box(x0, x1, y0, y1, z0, z1):
        p = [V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0),
             V(x0, y1, z0), V(x0, y0, z1), V(x1, y0, z1),
             V(x1, y1, z1), V(x0, y1, z1)]
        for idx in ((4, 5, 6, 7), (0, 3, 2, 1), (0, 1, 5, 4),
                    (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)):
            scene.mesh.add_face([p[i] for i in idx])

    box(-3, 3, -2, 2, 0, 2)
    box(-3, 0, 0, 2, 2, 5)
    scene.version += 1
    vp.camera.set_view("iso")
    vp.camera.yaw = math.radians(-135.0)
    vp.camera.pitch = math.radians(20.0)
    vp.camera.fit_to(V(-3.2, -2.2, 0), V(3.2, 2.2, 5.2))
    # The export is 900x700; give the camera that aspect so the face's
    # projected corners below land where the pixels do.
    vp.camera.set_aspect(900, 700)
    return vp



def _render(vp, monkeypatch, on: str) -> np.ndarray:
    monkeypatch.setenv("BUILT_AO", on)
    monkeypatch.setenv("BUILT_AO_RADIUS", "0.5")
    return _lum(vp.keys_stand_in.render_pixels(400, 300, overlays=False))


def _render_at(vp, monkeypatch, on: str, w: int, h: int) -> np.ndarray:
    monkeypatch.setenv("BUILT_AO", on)
    monkeypatch.setenv("BUILT_AO_RADIUS", "0.5")
    return _lum(vp.keys_stand_in.render_pixels(w, h, overlays=False))


def _pixel(vp, p: QVector3D, w: int, h: int) -> tuple[float, float]:
    """Where a world point lands in the exported image, in pixels."""
    m = vp.camera.projection_matrix() * vp.camera.view_matrix()
    q = m.map(p)
    return ((q.x() * 0.5 + 0.5) * w, (1.0 - (q.y() * 0.5 + 0.5)) * h)


def test_the_foot_of_a_box_darkens_and_the_open_floor_does_not(
        box_on_floor, monkeypatch):
    vp = box_on_floor
    off = _render(vp, monkeypatch, "0")
    # A GPU that does not draw the same frame twice cannot say whether
    # ambient occlusion leaves anything behind; one that does, must.
    deterministic = np.array_equal(_render(vp, monkeypatch, "0"), off)
    on = _render(vp, monkeypatch, "1")
    darker = off - on
    assert (darker > 8).mean() > 0.002          # the corner band shows
    assert (on > off + 1).mean() < 0.001        # it never lightens
    assert (np.abs(darker) < 1).mean() > 0.6    # most of the picture is as it was
    if not deterministic:
        pytest.skip("this GPU does not draw the same frame twice, so the frame after "
                    "ambient occlusion is turned off again cannot be compared")
    # Off again: the very same picture as before it was ever on.
    assert np.array_equal(_render(vp, monkeypatch, "0"), off)


def test_wireframe_has_none(box_on_floor, monkeypatch):
    vp = box_on_floor
    vp.scene.display_style.face_mode = "wireframe"                   # the wireframe face mode
    assert np.array_equal(_render(vp, monkeypatch, "0"),
                          _render(vp, monkeypatch, "1"))


def test_a_flat_face_seen_obliquely_gets_no_vertical_bands(
        step_block, monkeypatch):
    """#989: the half-size AO pass read the full-size depth at texel corners,
    so a flat face's normal wobbled and the face self-occluded in bands."""
    vp = step_block
    off = _render_at(vp, monkeypatch, "0", 900, 700)
    on = _render_at(vp, monkeypatch, "1", 900, 700)
    darker = off - on
    assert darker.max() > 5.0                  # the pass did something
    # The upper wall's left face (x = -3): flat, vertical, seen obliquely.
    V = QVector3D
    corners = [V(-3, 0, 2), V(-3, 2, 2), V(-3, 2, 5), V(-3, 0, 5)]
    pts = np.array([_pixel(vp, c, 900, 700) for c in corners])
    x0, y0 = pts.min(axis=0)
    x1, y1 = pts.max(axis=0)
    # Inset well clear of the face's edges and of the corner darkening.
    ix = (x1 - x0) * 0.28
    iy = (y1 - y0) * 0.22
    region = darker[int(y0 + iy):int(y1 - iy),
                    int(x0 + ix):int(x1 - ix)]
    assert region.size > 1000
    # One flat face has ONE occlusion value; the bug swung it ~28 levels.
    assert np.ptp(region) < 4.0, (region.min(), region.max(), region.mean())


@pytest.fixture
def corner_blocks(native_gpu_host):
    """Two blocks meeting in an inner vertical corner, the view of the
    0.5.11 round: a block hanging against the side of a taller one. Returns
    a function that builds it with the faces in the colour given."""
    from built_core.materials import Material
    def build(color):
        vp = native_gpu_host.viewport
        vp.scene.clear()
        V = QVector3D
        attrs = Material("pared", color=color).face_attrs()

        def box(x0, x1, y0, y1, z0, z1):
            p = [V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0),
                 V(x0, y1, z0), V(x0, y0, z1), V(x1, y0, z1),
                 V(x1, y1, z1), V(x0, y1, z1)]
            for idx in ((4, 5, 6, 7), (0, 3, 2, 1), (0, 1, 5, 4),
                        (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)):
                vp.scene.mesh.add_face([p[i] for i in idx]).attrs.update(attrs)

        box(0, 3, 0, 3, 0, 3.5)
        box(-3, 0, -1.5, 1.5, 1.2, 3.5)
        vp.scene.version += 1
        vp.camera.yaw = math.radians(-60.0)
        vp.camera.pitch = math.radians(12.0)
        vp.camera.fit_to(V(-3, -1.5, 0), V(3, 3, 3.5))
        vp.camera.set_aspect(1000, 800)
        return vp

    return build


def _corner_light(vp, monkeypatch) -> float:
    """How much light the darkest of the corner keeps, on over off."""
    off = _render_at(vp, monkeypatch, "0", 1000, 800)
    on = _render_at(vp, monkeypatch, "1", 1000, 800)
    lit = off > 20
    return float(np.percentile(on[lit] / off[lit], 0.1))


def test_a_light_inner_corner_is_a_soft_grey_not_a_black_bar(
        corner_blocks, monkeypatch):
    """The 0.5.11 contrast curve, ao^(2.5 * intensity), sent a 90-degree
    corner down to half its light in a hard bar. Measured as horizons and
    with the light its walls bounce, a light corner keeps most of it."""
    light = _corner_light(corner_blocks((0.85, 0.85, 0.82)), monkeypatch)
    assert 0.6 < light < 0.85, light


#: The light corner's share of its unshaded light in the probe frame, as
#: recorded on Apple M-series hardware (M5 Max, 4 samples).
REFERENCE_CORNER = 0.842


def reference_frame(corner_blocks, monkeypatch):
    """Skip where the GPU does not reproduce a recorded frame.

    The probe is the light corner drawn without and with ambient occlusion,
    then without again. A GPU that returns a different picture the second
    time, or a corner away from the recorded one, cannot measure how the
    corner follows the wall's colour. (A hosted paravirtual GPU measured
    0.797 here and gave light and dark walls the same corner.)"""
    vp = corner_blocks((0.85, 0.85, 0.82))
    off, on, again = (_render_at(vp, monkeypatch, enabled, 1000, 800) for enabled in ("0", "1", "0"))
    if not np.array_equal(off, again):
        pytest.skip("this GPU does not return the same frame once ambient occlusion is off again")
    lit = off > 60
    ratio = on[lit] / off[lit]
    corner = float(np.median(ratio[ratio < 0.9])) if (ratio < 0.9).sum() > 100 else float("nan")
    if not abs(corner - REFERENCE_CORNER) <= 0.02:
        pytest.skip(f"this GPU draws the probe frame's corner at {corner:.3f} of its light; "
                    f"the recorded frame has {REFERENCE_CORNER}")


def test_a_dark_corner_stays_darker_than_a_light_one(
        corner_blocks, monkeypatch):
    """The bounce follows the face's colour: dark walls pass on little
    light, so their corner keeps more of its darkening."""
    from built_render import store_and_resolve_probe
    if store_and_resolve_probe() is False:
        pytest.skip("this GPU does not hand on the colours of a multisampled pass that also keeps its samples "
                    "(resolved into the display view of the frame, then copied); ambient occlusion reads "
                    "the faces' colours from that copy")
    reference_frame(corner_blocks, monkeypatch)

    def difference(antialias):
        def samples(color):
            vp = corner_blocks(color)
            renderer = vp.keys_stand_in.renderer
            if renderer is not None and not antialias:
                renderer.set_msaa(False)
            return [_render_at(vp, monkeypatch, enabled, 1000, 800) for enabled in ("0", "1")] + [renderer]

        light_off, light_on, renderer = samples((0.85, 0.85, 0.82))
        dark_off, dark_on, _ = samples((0.25, 0.25, 0.25))
        if renderer is not None:
            renderer.set_msaa(True)
        # Compare the same wall pixels in both images. Selecting the darkest
        # percentile independently can measure an unchanged ground or edge.
        walls = (light_off > dark_off + 30) & (dark_off > 20)
        light = light_on[walls] / light_off[walls]
        dark = dark_on[walls] / dark_off[walls]
        corner = light < 0.9
        counts = renderer.msaa_levels() if renderer is not None else None
        return (float(np.median(light[corner] - dark[corner])) if corner.sum() else None, int(corner.sum()),
                round(float(np.median(light[corner])), 4), round(float(np.median(dark[corner])), 4), counts)

    seen = difference(True)
    assert seen[1] > 100, seen
    assert seen[0] > 0.025, {"antialiased": seen, "plain": difference(False)}


def test_intensity_zero_leaves_the_picture_untouched(
        box_on_floor, monkeypatch):
    vp = box_on_floor
    off = _render(vp, monkeypatch, "0")
    monkeypatch.setenv("BUILT_AO_INTENSITY", "0")
    assert np.array_equal(_render(vp, monkeypatch, "1"), off)


# ---- The style's switch (Styles panel) -------------------------------------

def test_the_style_carries_it_and_every_preset_has_it_off():
    from built_core.style import BUILTIN_STYLES, Style
    assert all(not p.ambient_occlusion for p in BUILTIN_STYLES)
    s = Style(ambient_occlusion=True, ao_distance=1.25, ao_intensity=1.5)
    back = Style.from_dict(s.to_dict())
    assert (back.ambient_occlusion, back.ao_distance, back.ao_intensity) \
        == (True, 1.25, 1.5)


def test_a_file_from_before_reads_it_off_and_bad_values_are_kept_in_range():
    from built_core.style import AO_DISTANCE_RANGE, Style
    old = Style().to_dict()
    for k in ("ambient_occlusion", "ao_distance", "ao_intensity"):
        old.pop(k)
    assert not Style.from_dict(old).ambient_occlusion
    odd = Style.from_dict({"ambient_occlusion": True, "ao_distance": 1e9,
                           "ao_intensity": "mucho"})
    assert odd.ao_distance == AO_DISTANCE_RANGE[1] and odd.ao_intensity == 1.0


def test_the_frame_follows_the_style_unless_the_environment_forces_it(
        monkeypatch):
    from built_core.style import Style
    monkeypatch.delenv("BUILT_AO", raising=False)
    assert ao_settings(Style()) is None
    got = ao_settings(Style(ambient_occlusion=True, ao_distance=2.0,
                                ao_intensity=0.5))
    assert got["radius"] == 2.0 and got["intensity"] == 0.5
    monkeypatch.setenv("BUILT_AO", "0")
    assert ao_settings(Style(ambient_occlusion=True)) is None


# ---- Stable (0.5.11): silhouettes, sky and flat faces ------------------------

def test_the_sky_round_a_silhouette_is_never_darkened(native_gpu_host, monkeypatch):
    """The occlusion is worked out at half size. Stretched back without
    looking at depth, the dark value of a frame's corner leaked onto the
    sky next to it: a dark, toothed halo round every slat and column (the
    Windows round of 0.5.10.1). Every pixel that shows sky with nothing in
    the model must show the very same sky with the occlusion on."""
    vp = native_gpu_host.viewport
    V = QVector3D
    vp.camera.set_view("iso")
    vp.camera.fit_to(V(-3, -3, 0), V(3, 3, 3))
    empty = _render(vp, monkeypatch, "0")
    # A frame of three posts and a lintel: inner corners, against sky.
    mesh = vp.scene.mesh

    def box(x0, y0, z0, x1, y1, z1):
        p = [V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0), V(x0, y1, z0),
             V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)]
        for idx in ((3, 2, 1, 0), (4, 5, 6, 7), (0, 1, 5, 4),
                    (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)):
            mesh.add_face([p[i] for i in idx])

    for x in (-2.0, 0.0, 2.0):
        box(x - 0.15, -0.15, 0, x + 0.15, 0.15, 2.4)
    box(-2.3, -0.2, 2.4, 2.3, 0.2, 2.7)
    vp.scene.version += 1
    off = _render(vp, monkeypatch, "0")
    on = _render(vp, monkeypatch, "1")
    sky = empty == off
    assert sky.mean() > 0.3
    # Softly since the horizons and the bounce, but they do.
    assert (off - on > 2).mean() > 0.0005      # the corners still darken
    assert np.array_equal(on[sky], off[sky])

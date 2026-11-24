# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Line weights by class and the poché of the vector style (professional
sheets, 2026-09-05): the hidden-line pass tells section cuts, profiles
(the usual sense — silhouettes and outlines against the background) and plain
edges apart, chains the cut chords into closed rings, and the composer
inks each class with its own pen and fills the rings under the lines."""
from __future__ import annotations

import math
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
import pytest
from PySide6.QtGui import QColor, QImage, QPainter
from ingetrazo_core import Vec3 as QVector3D
from PySide6.QtWidgets import QApplication

from core.camera import OrbitCamera
from core.composition import Composicion, MarcoVista, apply_frame_camera
from core.hlr import (KIND_CUT, KIND_EDGE, KIND_PROFILE, hlr_drawing,
                      hlr_view, section_loops)
from core.scene import Scene
from core.section import SectionPlane

_app = QApplication.instance() or QApplication([])
V = QVector3D


def _box(m, x0, y0, z0, x1, y1, z1):
    """A closed axis-aligned box of six faces."""
    m.add_face([V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0), V(x0, y1, z0)])
    m.add_face([V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)])
    m.add_face([V(x0, y0, z0), V(x1, y0, z0), V(x1, y0, z1), V(x0, y0, z1)])
    m.add_face([V(x0, y1, z0), V(x1, y1, z0), V(x1, y1, z1), V(x0, y1, z1)])
    m.add_face([V(x0, y0, z0), V(x0, y1, z0), V(x0, y1, z1), V(x0, y0, z1)])
    m.add_face([V(x1, y0, z0), V(x1, y1, z0), V(x1, y1, z1), V(x1, y0, z1)])


def _camera(scene, view_key="std:top"):
    cam = OrbitCamera()
    f = MarcoVista(view_key=view_key, scale_n=100.0, w_mm=100.0, h_mm=100.0)
    apply_frame_camera(cam, f, saved_view=None, scene=scene)
    return cam


def _ring_area(ring) -> float:
    x, y = ring[:, 0], ring[:, 1]
    return 0.5 * abs(float(np.dot(x, np.roll(y, -1)) - np.dot(y, np.roll(x, -1))))


# ---- section_loops ---------------------------------------------------------

def test_section_loops_close_the_ring_of_a_cut_solid():
    from core.hlr import clip_to_section, collect_geometry
    scene = Scene()
    _box(scene.mesh, 0, 0, 0, 2, 3, 2)
    tris, hard, soft = collect_geometry(scene)
    plane = SectionPlane(V(0, 0, 1), V(0, 0, 1))          # hide z > 1
    _t, _h, _s, cuts = clip_to_section(tris, hard, soft, plane,
                                       split_cuts=True)
    loops = section_loops(cuts)
    assert len(loops) == 1
    ring = loops[0]
    assert np.allclose(ring[:, 2], 1.0)                     # lies in the plane
    assert _ring_area(ring[:, :2]) == pytest.approx(6.0)    # the 2×3 section


def test_section_loops_keep_a_hollow_wall_hollow_and_skip_open_chains():
    from core.hlr import clip_to_section, collect_geometry
    scene = Scene()
    _box(scene.mesh, 0, 0, 0, 4, 4, 3)                      # outer skin
    _box(scene.mesh, 1, 1, 0, 3, 3, 3)                      # inner skin (a hollow)
    # a lone vertical face beside it: an open surface, never filled
    scene.mesh.add_face([V(6, 0, 0), V(8, 0, 0), V(8, 0, 3), V(6, 0, 3)])
    tris, hard, soft = collect_geometry(scene)
    plane = SectionPlane(V(0, 0, 1.5), V(0, 0, 1))
    _t, _h, _s, cuts = clip_to_section(tris, hard, soft, plane,
                                       split_cuts=True)
    loops = section_loops(cuts)
    areas = sorted(_ring_area(r[:, :2]) for r in loops)
    assert areas == pytest.approx([4.0, 16.0])              # two rings, no chain


# ---- line classes -------------------------------------------------------------

def test_a_cut_box_from_above_draws_cut_lines_and_fills_the_ring():
    scene = Scene()
    _box(scene.mesh, 0, 0, 0, 2, 3, 2)
    sp = SectionPlane(V(0, 0, 1), V(0, 0, 1))
    scene.section_planes.append(sp)
    scene.set_active_section(sp)
    cam = _camera(scene, "std:top")
    d = hlr_drawing(scene, cam)
    assert len(d) == len(d.kinds) == len(d.world)
    cut_len = sum(math.hypot(x1 - x0, y1 - y0)
                  for (x0, y0, x1, y1), k in zip(d.segs, d.kinds)
                  if k == KIND_CUT)
    # the whole perimeter of the section is inked as cut (chords of the
    # triangulated faces overlap the box's outline, so ≥ the perimeter)
    assert cut_len >= 2 * (2 + 3) - 1e-6
    assert len(d.loops) == 1
    assert _ring_area(d.loops[0]) == pytest.approx(6.0)
    # and the plain view (no fills) still agrees with hlr_view
    segs = hlr_view(scene, cam)
    assert segs.shape == d.segs.shape


def test_profiles_outline_a_box_seen_at_an_angle():
    scene = Scene()
    _box(scene.mesh, 0, 0, 0, 2, 2, 2)
    cam = OrbitCamera()
    cam.perspective = False
    cam.target = V(1, 1, 1)
    cam.yaw = math.radians(35.0)
    cam.pitch = math.radians(30.0)
    d = hlr_drawing(scene, cam)
    prof = float(sum(math.hypot(x1 - x0, y1 - y0)
                     for (x0, y0, x1, y1), k in zip(d.segs, d.kinds)
                     if k == KIND_PROFILE))
    edge = float(sum(math.hypot(x1 - x0, y1 - y0)
                     for (x0, y0, x1, y1), k in zip(d.segs, d.kinds)
                     if k == KIND_EDGE))
    # Three faces show: the six outline edges are profiles, the three
    # edges meeting at the near corner run between two faces → thin.
    assert prof > 0 and edge > 0
    assert prof > edge
    assert KIND_CUT not in set(d.kinds.tolist())
    # profiles off: every line is a plain edge
    flat = hlr_drawing(scene, cam, profiles=False)
    assert set(flat.kinds.tolist()) == {KIND_EDGE}
    assert flat.segs.shape == d.segs.shape


def test_a_lone_face_is_all_profile_from_the_front():
    scene = Scene()
    scene.mesh.add_face([V(0, 0, 0), V(6, 0, 0), V(6, 0, 3), V(0, 0, 3)])
    d = hlr_drawing(scene, _camera(scene, "std:front"))
    assert len(d) >= 4
    assert set(d.kinds.tolist()) == {KIND_PROFILE}


# ---- the composer inks them -----------------------------------------------------

def test_frame_pens_and_poche_survive_the_document_roundtrip():
    c = Composicion()
    f = MarcoVista(style="vectorial", pen_cut_mm=0.7, pen_profile_mm=0.4,
                   pen_edge_mm=0.13, profiles=False, cut_fill="hatch",
                   cut_fill_color="#112233", cut_hatch_mm=2.0)
    c.frames.append(f)
    back = Composicion.from_dict(c.to_dict()).frames[-1]
    assert (back.pen_cut_mm, back.pen_profile_mm, back.pen_edge_mm) == (
        0.7, 0.4, 0.13)
    assert back.profiles is False
    assert (back.cut_fill, back.cut_fill_color, back.cut_hatch_mm) == (
        "hatch", "#112233", 2.0)
    # an old document without the fields gets the plan defaults
    legacy = Composicion.from_dict({"frames": [{"style": "vectorial"}]})
    g = legacy.frames[0]
    assert (g.pen_cut_mm, g.pen_profile_mm, g.pen_edge_mm) == (0.5, 0.35, 0.18)
    assert g.profiles is True and g.cut_fill == "solid"


# ---- clean lines -------------------------------------------------------------

def test_chords_across_a_triangle_diagonal_merge_and_end_on_verticals_vanish():
    scene = Scene()
    _box(scene.mesh, 0, 0, 0, 10, 0.25, 3)                  # one wall
    sp = SectionPlane(V(5, 4, 1.2), V(0, 0, 1))
    scene.section_planes.append(sp)
    scene.set_active_section(sp)
    d = hlr_drawing(scene, _camera(scene, "std:top"))
    lens = [math.hypot(x1 - x0, y1 - y0) for x0, y0, x1, y1 in d.segs]
    assert min(lens) > 1e-6                     # no zero-length dots
    cut = sorted(round(L, 6) for L, k in zip(lens, d.kinds) if k == KIND_CUT)
    # the two long faces are ONE chord each (not split at the diagonal),
    # the two short ends one each
    assert cut == [0.25, 0.25, 10.0, 10.0]
    # world endpoints still re-project onto the merged 2D segments
    from core.hlr import _to_cam, camera_basis
    cam = _camera(scene, "std:top")
    eye, right, up, fwd = camera_basis(cam)
    for (x0, y0, x1, y1), (w0, w1) in zip(d.segs, d.world):
        c0 = _to_cam(np.asarray([w0]), eye, right, up, fwd)[0]
        c1 = _to_cam(np.asarray([w1]), eye, right, up, fwd)[0]
        assert (c0[0], c0[1]) == pytest.approx((x0, y0), abs=1e-9)
        assert (c1[0], c1[1]) == pytest.approx((x1, y1), abs=1e-9)


def test_dxf_export_puts_each_line_class_on_its_own_layer(tmp_path):
    from formats.dxf_out import save_dxf_layers, save_dxf_lines
    path = tmp_path / "vista.dxf"
    n = save_dxf_layers(path, [
        ("Planta", [(0, 0, 1, 0), (1, 0, 1, 1)]),
        ("Planta-PERFIL", [(0, 0, 0, 1)]),
        ("Planta-CORTE", [])])
    assert n == 3
    text = path.read_text()
    assert text.count("0\nLAYER\n2\n") == 3
    assert "2\nPLANTA-CORTE\n" in text and "2\nPLANTA-PERFIL\n" in text
    assert text.count("8\nPLANTA\n") == 2 and text.count("8\nPLANTA-PERFIL\n") == 1
    # the single-layer writer is the same file with one group
    assert save_dxf_lines(tmp_path / "one.dxf", [(0, 0, 1, 1)], layer="x") == 1


# ---- the panel ------------------------------------------------------------------


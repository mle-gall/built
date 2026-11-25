# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Sheets: a copied frame shows ITS scene's layers, on screen and on export.

GitHub #1450 — in Sheets, copying a view portal and changing the copy's
assigned scene left it showing the ORIGINAL portal's layer visibility; the
picture looked right at first and reverted on PDF export. Two caches were
to blame, both keyed without the visibility: the composer's shared
geometry (``_scene_geometry``) was built once under the live layers and
served again after the live layers moved, and the pick index behind it
omits hidden layers outright. The shared geometry is now keyed on the
visibility and re-collected from the layers as they are switched when it
moved; a frame's scene also no longer leaks its hidden objects into the
live model.
"""
import copy
import uuid

import numpy as np
import pytest
from ingetrazo_core import Vec3 as V

from core.group import Group
from core.layers import Layer
from core.mesh import Mesh
from core.scene import Scene
from core.saved_views import SavedView, visibility_sig
from tests.controller_surface import ControllerSurface as Viewport


def _box(m, x0, y0, z0, x1, y1, z1):
    m.add_face([V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0), V(x0, y1, z0)])
    m.add_face([V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)])
    m.add_face([V(x0, y0, z0), V(x1, y0, z0), V(x1, y0, z1), V(x0, y0, z1)])
    m.add_face([V(x0, y1, z0), V(x1, y1, z0), V(x1, y1, z1), V(x0, y1, z1)])
    m.add_face([V(x0, y0, z0), V(x0, y1, z0), V(x0, y1, z1), V(x0, y0, z1)])
    m.add_face([V(x1, y0, z0), V(x1, y1, z0), V(x1, y1, z1), V(x1, y0, z1)])


def _build_scene(only=None):
    """Two layers, one box on each (different world-x); S1 hides B, S2
    hides A. ``only`` builds a single-box control scene."""
    scene = Scene()
    scene.layers += [Layer("Members", key="A", folder="West", color=(1.0, 0.0, 0.0)),
                     Layer("Members", key="B", folder="East", color=(0.0, 0.0, 1.0))]
    if only != "B":
        ga = Group(Mesh(), name="BoxA")
        ga.layer = "A"
        _box(ga.mesh, 0, 0, 0, 2, 2, 2)
        scene.groups.append(ga)
    if only != "A":
        gb = Group(Mesh(), name="BoxB")
        gb.layer = "B"
        _box(gb.mesh, 10, 0, 0, 12, 2, 2)
        scene.groups.append(gb)
    kw = dict(target=(6.0, 1.0, 1.0), distance=20.0,
              yaw=-0.7853981633974483, pitch=0.5235987755982988,
              perspective=False)
    scene.layer_folders = ["West", "East", "Building"]
    scene.folder_parents = {"West": "Building"}
    s1 = SavedView("S1", hidden_layers=["B"], hidden_folders=[], **kw)
    s2 = SavedView("S2", hidden_layers=[], hidden_folders=["Building"],
                   **kw)
    scene.saved_views = [s1, s2]
    return scene, s1, s2


def _prep_frame(frame, view_key):
    frame.view_key = view_key
    frame.style = "vectorial"
    frame.scale_n = 100.0
    frame.w_mm, frame.h_mm = 200.0, 100.0


def _segs(composer, frame):
    composer.compute_hlr(frame)
    out = composer.hlr_cache.get(id(frame))
    return None if out is None else np.asarray(out, dtype=float)


def _same(a, b):
    if a is None or b is None:
        return a is b
    if a.shape != b.shape:
        return False
    return bool(np.allclose(np.sort(a.ravel()), np.sort(b.ravel())))


def test_visibility_sig_moves_with_layers_and_hidden_objects():
    scene, s1, s2 = _build_scene()
    base = visibility_sig(scene)
    scene.layer("B").visible = False
    assert visibility_sig(scene) != base
    scene.layer("B").visible = True
    assert visibility_sig(scene) == base
    gb = next(g for g in scene.groups if g.name == "BoxB")
    gb.hidden = True
    assert visibility_sig(scene) != base
    gb.hidden = False
    assert visibility_sig(scene) == base



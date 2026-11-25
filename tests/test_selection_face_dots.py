# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A selected face is drawn as a dot pattern whose colour reads on its paint.

The old cue was a 35% orange wash, which over the blue-grey back side looked
like just another back face. Now selected faces get opaque dots: orange as
usual, blue on a side that is itself orange/red, where orange would vanish.
"""
from __future__ import annotations


from ingetrazo_core import Vec3 as QVector3D


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


def _viewport():
    from tests.controller_surface import ControllerSurface as Viewport
    vp = Viewport(None)
    vp.resize(800, 600)
    return vp


def test_dot_colour_switches_on_orange_and_red_surfaces():
    from views.document_interaction import SELECTION_DOT_ALT_COLOR, SELECTION_DOT_COLOR, selection_dot_color
    assert selection_dot_color((0.96, 0.95, 0.925)) == SELECTION_DOT_COLOR
    assert selection_dot_color((0.62, 0.70, 0.78)) == SELECTION_DOT_COLOR
    assert selection_dot_color(None) == SELECTION_DOT_COLOR
    assert selection_dot_color((0.95, 0.45, 0.16)) == SELECTION_DOT_ALT_COLOR
    assert selection_dot_color((0.85, 0.15, 0.10)) == SELECTION_DOT_ALT_COLOR
    # A pale, washed-out peach is not "orange" enough to hide the dots.
    assert selection_dot_color((0.95, 0.90, 0.85)) == SELECTION_DOT_COLOR


def test_each_side_is_judged_against_its_own_paint():
    from views.document_interaction import SELECTION_DOT_ALT_COLOR, SELECTION_DOT_COLOR
    vp = _viewport()
    face = vp.scene.mesh.add_face([V(0, 0), V(1, 0), V(1, 1), V(0, 1)])
    # Default cream front, default blue-grey back: orange on both.
    assert vp._selection_dot_colors(face) == (SELECTION_DOT_COLOR,
                                              SELECTION_DOT_COLOR)
    face.attrs["color"] = (0.95, 0.50, 0.15)            # orange front only
    assert vp._selection_dot_colors(face) == (SELECTION_DOT_ALT_COLOR,
                                              SELECTION_DOT_COLOR)
    face.attrs["back"] = True                           # both sides orange
    assert vp._selection_dot_colors(face) == (SELECTION_DOT_ALT_COLOR,
                                              SELECTION_DOT_ALT_COLOR)


def test_selected_faces_are_split_into_runs_per_dot_colour():
    vp = _viewport()
    plain = vp.scene.mesh.add_face([V(0, 0), V(1, 0), V(1, 1), V(0, 1)])
    orange = vp.scene.mesh.add_face([V(2, 0), V(3, 0), V(3, 1), V(2, 1)])
    orange.attrs["color"] = (0.95, 0.45, 0.16)
    vp.scene.selection.update({plain, orange})
    vp.scene.version += 1
    from types import SimpleNamespace
    from ingetrazo_core import pack_scene, VERTEX
    from views.wgpu_parity import face_marks
    view = SimpleNamespace(scene=vp.scene, source_viewport=vp, camera=vp.camera,
                           packed=pack_scene(vp.scene))
    rgba = face_marks(view, [plain, orange]).view(VERTEX)['color']
    assert len(rgba) == 12
    assert len(set(map(tuple, rgba))) == 2

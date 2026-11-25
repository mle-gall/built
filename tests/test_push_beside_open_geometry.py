# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""An open piece elsewhere must not leave a new solid's face reversed
(#230, @Rainjalin).

A cube with a face deleted sits in the model; a rectangle drawn apart and
pushed into a second cube came out with its bottom face reversed. The
orientation pass decided whether it may turn faces by asking if the WHOLE
mesh was closed, so one open piece anywhere stopped it everywhere — group
the open cube and the bug went away, explode it and it came back.
"""
from __future__ import annotations

from built_core import Vec3 as V

from core.edits import build_add_edges
from core.history import EraseSelectionCommand, History
from core.scene import Scene
from tests.test_coplanar_merge import _push


def _rect_face(scene, hist, x0):
    c = [V(x0, 0, 0), V(x0 + 1, 0, 0), V(x0 + 1, 1, 0), V(x0, 1, 0)]
    hist.execute(build_add_edges(scene, [(c[i], c[(i + 1) % 4])
                                         for i in range(4)]))
    return next(f for f in scene.mesh.faces
                if all(x0 - 1e-6 <= v.x() <= x0 + 1 + 1e-6
                       for v in f.vertices)
                and all(abs(v.z()) < 1e-6 for v in f.vertices))


def _faces_pointing_in(scene, x0):
    """The faces of the unit cube at ``x0`` whose normal points inside."""
    centre = V(x0 + 0.5, 0.5, 0.5)
    return [f for f in scene.mesh.faces
            if all(v.x() >= x0 - 1e-6 for v in f.vertices)
            and V.dotProduct(f.normal(), f.centroid() - centre) <= 0]


def _open_cube(scene, hist):
    """A cube at x = 0 with its top deleted: open, non-watertight."""
    _push(scene, hist, _rect_face(scene, hist, 0.0), 1.0)
    top = next(f for f in scene.mesh.faces
               if all(abs(v.z() - 1) < 1e-6 for v in f.vertices))
    hist.execute(EraseSelectionCommand([], faces=[top]))


def test_a_new_cube_beside_an_open_one_faces_out():
    scene = Scene()
    hist = History(scene)
    _open_cube(scene, hist)
    _push(scene, hist, _rect_face(scene, hist, 3.0), 1.0)
    assert _faces_pointing_in(scene, 3.0) == [], \
        "a face of the new cube points inside"


def test_the_open_cube_itself_is_left_as_it_was():
    """The fix decides per solid: the open cube has no inside, so its
    faces are never turned."""
    scene = Scene()
    hist = History(scene)
    _open_cube(scene, hist)
    before = {f.key: f.normal() for f in scene.mesh.faces}
    _push(scene, hist, _rect_face(scene, hist, 3.0), 1.0)
    for f in scene.mesh.faces:
        if f.key in before:
            assert V.dotProduct(f.normal(), before[f.key]) > 0.999



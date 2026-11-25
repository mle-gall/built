# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A closed skin wound one way is oriented by asking a few of its faces.

Every edit re-decided every face of the solid it touched: two thousand
faces, each with its rays, three times per circle drawn and pushed — 0.5 s
of every operation on a slab with sixty cylinders. A skin whose every edge
is walked by two faces, once each way, has no partitions and cannot
disagree with itself: its faces all look out or all look in, and a
handful of them say which.

What is not such a skin (a partition, one face turned over) still goes
through the pass that asks every face, and gets the answer it always got.
"""
from __future__ import annotations

from ingetrazo_core import Vec3 as QVector3D

from ingetrazo_core import orient
from core.history import History, MoveVerticesCommand
from core.mesh import Mesh
from ingetrazo_core.orient import orient_outward, signed_volume
from core.scene import Scene
from core.topology import _key
from tests.test_pushpull_large_solid_cost import (_circle,
                                                  _slab_with_cylinders)
from tests.test_pushpull_ux import _cube, _push


def V(x, y, z):
    return QVector3D(x, y, z)


def _box_loops(x0, y0, z0, x1, y1, z1):
    """The six faces of a box, each wound to look out."""
    return [
        [V(x0, y0, z0), V(x0, y1, z0), V(x1, y1, z0), V(x1, y0, z0)],  # down
        [V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)],  # up
        [V(x0, y0, z0), V(x1, y0, z0), V(x1, y0, z1), V(x0, y0, z1)],  # front
        [V(x0, y1, z0), V(x0, y1, z1), V(x1, y1, z1), V(x1, y1, z0)],  # back
        [V(x0, y0, z0), V(x0, y0, z1), V(x0, y1, z1), V(x0, y1, z0)],  # left
        [V(x1, y0, z0), V(x1, y1, z0), V(x1, y1, z1), V(x1, y0, z1)],  # right
    ]


def _box(mesh, *corners, turned=()):
    faces = []
    for i, loop in enumerate(_box_loops(*corners)):
        faces.append(mesh.add_face(loop[::-1] if i in turned else loop))
    return faces


def test_an_open_mesh_is_marked_but_never_turned():
    """As before: without a closed mesh, a pass over everything turns
    nothing — a closed box wound inwards beside a loose face included."""
    mesh = Mesh()
    _box(mesh, 0, 0, 0, 1, 1, 1, turned=range(6))
    mesh.add_face([V(5, 0, 0), V(6, 0, 0), V(6, 1, 0.5)])
    assert orient_outward(mesh) == []
    assert signed_volume(mesh) < 0



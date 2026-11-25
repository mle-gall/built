# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Curved surfaces shade round in the viewport, not faceted (#131).

The viewport bakes its matte shading into the vertex colours, one colour
per face from the face's own normal: a sphere or a cylinder joined by soft
edges hid its lines and still showed every facet. Each corner on a curved
surface now takes the colour of the SURFACE's normal there — the smooth
normals the exports write — so neighbouring faces meet in one colour and
the GPU blends across them. A hard edge keeps its crease, and a model
without soft edges is drawn exactly as before."""
from __future__ import annotations

import math
import os
from collections import defaultdict

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np                                                # noqa: E402
import pytest                                                     # noqa: E402
from built_core import Vec3 as QVector3D

from core.group import Group                                      # noqa: E402
from core.mesh import Mesh                                        # noqa: E402
from core.scene import Scene                                      # noqa: E402
from views.document_interaction import DocumentInteraction

SIDES = 12
# A light from the side: every side face of the cylinder gets a clearly
# different flat shade, and the caps (normal ±Z) the darkest one.
SIDE_LIGHT = QVector3D(1.0, 0.3, 0.0).normalized()


def V(x, y, z):
    return QVector3D(float(x), float(y), float(z))


def _cylinder(mesh, soft=True):
    ring0 = [V(math.cos(2 * math.pi * i / SIDES),
               math.sin(2 * math.pi * i / SIDES), 0) for i in range(SIDES)]
    ring1 = [V(p.x(), p.y(), 2) for p in ring0]
    sides = [mesh.add_face([ring0[i], ring0[(i + 1) % SIDES],
                            ring1[(i + 1) % SIDES], ring1[i]])
             for i in range(SIDES)]
    mesh.add_face(list(reversed(ring0)))
    mesh.add_face(ring1)
    if soft:
        for e in mesh.edges:
            if len(e.faces) == 2 and all(f in sides for f in e.faces):
                e.soft = True
    return mesh


def _box(mesh):
    c = [V(x, y, z) for z in (0, 1) for y in (0, 1) for x in (0, 1)]
    for q in ([0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4],
              [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]):
        mesh.add_face([c[i] for i in q])
    return mesh


class _VP(DocumentInteraction):
    def __init__(self, scene):
        self.scene = scene

    def width(self):
        return 100

    def height(self):
        return 100


def _stub(scene):
    """A document surface with no window or disk cache."""
    vp = _VP(scene)
    vp.active_tool = None
    vp._chunk_cache_load = None
    vp._chunk_cache_store = None
    vp._LIGHT = SIDE_LIGHT
    return vp


def _chunk_rows(mesh):
    scene = Scene()
    g = Group(mesh, name="cilindro")
    scene.groups.append(g)
    vp = _stub(scene)
    return np.frombuffer(vp._group_chunk(g)["vcol"],
                         dtype=np.float32).reshape(-1, 6)


def _colours_at(rows):
    """``{position: {rgb, …}}`` over every drawn corner."""
    out = defaultdict(set)
    for r in rows:
        out[tuple(np.round(r[:3], 5))].add(tuple(np.round(r[3:], 5)))
    return out


def test_a_soft_cylinder_meets_in_one_colour_at_each_seam():
    at = _colours_at(_chunk_rows(_cylinder(Mesh())))
    assert len(at) == 2 * SIDES
    # The side surface brings ONE colour to each corner, the cap its own.
    assert all(len(cols) == 2 for cols in at.values())


def test_hard_seams_keep_one_colour_per_face():
    at = _colours_at(_chunk_rows(_cylinder(Mesh(), soft=False)))
    # Two side faces and a cap meet at each corner, each in its flat colour.
    assert max(len(cols) for cols in at.values()) == 3


def test_a_soft_side_face_shades_across_its_width():
    rows = _chunk_rows(_cylinder(Mesh()))
    side = rows[:6]                     # the first side face, two triangles
    assert len({tuple(np.round(r[3:], 5)) for r in side}) == 2


def test_the_cap_keeps_its_flat_colour_against_a_soft_side():
    mesh = _cylinder(Mesh())
    flat = _stub(Scene())._shaded_color(DocumentInteraction.DEFAULT_FACE_COLOR,
                                        QVector3D(0, 0, 1))
    rows = _chunk_rows(mesh)
    caps = [r for r in rows if abs(r[2]) < 1e-6 or abs(r[2] - 2) < 1e-6]
    on_ring = defaultdict(list)
    for r in caps:
        on_ring[tuple(np.round(r[:3], 5))].append(r[3:])
    # Every ring corner shows the cap's flat colour among its two.
    assert len(on_ring) == 2 * SIDES
    assert all(any(np.allclose(c, flat, atol=1e-6) for c in cols)
               for cols in on_ring.values())


def test_a_box_is_drawn_exactly_as_before():
    rows = _chunk_rows(_box(Mesh()))
    tri = rows.reshape(-1, 3, 6)
    # Every triangle in one flat colour, quantised as the face shade is.
    assert np.all(tri[:, :, 3:] == tri[:, :1, 3:])
    assert np.allclose(rows[:, 3:] * 64.0, np.round(rows[:, 3:] * 64.0),
                       atol=1e-4)


def test_coplanar_faces_joined_by_a_soft_edge_keep_the_flat_colour():
    mesh = Mesh()
    a = mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    b = mesh.add_face([V(0, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    for e in mesh.edges:
        if a in e.faces and b in e.faces:
            e.soft = True
    rows = _chunk_rows(mesh)
    assert len({tuple(r[3:]) for r in rows}) == 1


def test_loose_geometry_shades_round_too():
    from built_core import pack_scene
    scene = Scene()
    _cylinder(scene.mesh)
    packed = pack_scene(scene)
    by_position = defaultdict(set)
    cmd = packed.commands[0]
    indices = packed.indices[int(cmd['first']):int(cmd['first'] + cmd['count'])]
    for vertex in packed.vertices[indices + cmd['base']]:
        by_position[tuple(vertex['pos'])].add(tuple(np.round(vertex['normal'], 5)))
    assert len(by_position) == 2 * SIDES
    assert all(len(normals) == 2 for normals in by_position.values())

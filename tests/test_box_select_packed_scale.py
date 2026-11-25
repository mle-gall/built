# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Box selection in the Metal view over a big linked-file-like object.

The Metal view's tool facade has no GL chunk caches, so the box used to
walk every vertex of every placement in Python, one projection each: on the
ride composite (91k placements, 14M faces) the main thread stayed in
``mouseReleaseEvent`` for 30–95 s and the app was force-quit (hang reports
of 2026-11-18). The box now reads the packed scene the view draws: one
numpy pass over the instances' boxes, the exact test only for instances
that straddle the rectangle, and the same answer as the walk.
"""
from __future__ import annotations

import math
import time

import numpy as np
import pytest
from built_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from core.camera import OrbitCamera
from core.group import Group
from core.mesh import Edge, Face, Mesh, Vertex
from core.scene import Scene
import tools.select_host as select
from tools.rust_tools import SelectTool
from built_render import MeshToolAccess
from views.document_interaction import DocumentInteraction
from built_core import pack_scene

W, H = 800, 600


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


def _box(n=2, size=0.8, height=0.5):
    """A box with each side split n×n (6·n² quads)."""
    m = Mesh()
    s = (size, size, height)

    def side(o, u, v):
        for i in range(n):
            for j in range(n):
                pts = [[o[k] + u[k] * (i + a) / n + v[k] * (j + b) / n
                        for k in range(3)]
                       for a, b in ((0, 0), (1, 0), (1, 1), (0, 1))]
                m.add_face([V(*p) for p in pts])
    side((0, 0, 0), (s[0], 0, 0), (0, s[1], 0))
    side((0, 0, s[2]), (0, s[1], 0), (s[0], 0, 0))
    side((0, 0, 0), (0, 0, s[2]), (s[0], 0, 0))
    side((0, s[1], 0), (s[0], 0, 0), (0, 0, s[2]))
    side((0, 0, 0), (0, s[1], 0), (0, 0, s[2]))
    side((s[0], 0, 0), (0, 0, s[2]), (0, s[1], 0))
    return m


def _sheet(x0, y0, z, size=0.6, n=6):
    """Unique geometry: an n×n quad sheet at (x0, y0, z), built in bulk."""
    i, j = np.meshgrid(np.arange(n), np.arange(n), indexing="ij")
    i, j = i.ravel(), j.ravel()
    d = size / n
    corners = np.stack([np.stack([x0 + (i + a) * d, y0 + (j + b) * d,
                                  np.full(i.shape, z)], -1)
                        for a, b in ((0, 0), (1, 0), (1, 1), (0, 1))], 1)
    m = Mesh()
    m.add_faces_bulk(corners.reshape(-1, 3), np.full(n * n, 4),
                     np.ones(n * n, int))
    return m


def _placed(mesh, x, y, name):
    g = Group(mesh, name=name)
    g.component = True
    xf = QMatrix4x4()
    xf.translate(float(x), float(y), 0.0)
    g.xform = xf
    return g


def _site(parts=10, per_part=100, sheets=20, side=None):
    """A linked-file-like component: ``parts`` groups of ``per_part``
    instances of one box plus ``sheets`` unique sheets each, on a grid."""
    proto = _box()
    total = parts * (per_part + sheets)
    side = side or int(math.sqrt(total)) + 1
    k = 0
    kids = []
    for p in range(parts):
        members = []
        for _ in range(per_part):
            members.append(_placed(proto, k % side, k // side, f"piece {k}"))
            k += 1
        for _ in range(sheets):
            members.append(Group(_sheet(k % side + 0.2, k // side + 0.2, 0.7),
                                 name=f"sheet {k}"))
            k += 1
        part = Group(Mesh(), name=f"part {p}")
        part.adopt(members)
        kids.append(part)
    site = Group(Mesh(), name="Site")
    site.adopt(kids)
    site.component = True
    site.xform = QMatrix4x4()
    return site, side


class _View:
    """What the facade reads of the Metal view: camera, size, the pack."""

    def __init__(self, scene, camera):
        self.camera = camera
        self.scene = scene
        self.packed = pack_scene(scene)

    def width(self):
        return W

    def height(self):
        return H

    def _world_to_pixel(self, point):
        return DocumentInteraction._world_to_pixel(self, point)


class _Facade(MeshToolAccess):
    """The Metal view's tool facade (built_render.NativeTools), minus
    the window: no GL chunks, projection through the view's camera."""

    def __init__(self, scene, camera):
        self.scene = scene
        self.view = _View(scene, camera)
        self.projected = 0

    @property
    def camera(self):
        return self.view.camera

    def width(self):
        return W

    def height(self):
        return H

    def update(self):
        pass

    def _world_to_pixel(self, point, mvp=None):
        self.projected += 1
        return super()._world_to_pixel(point, mvp)


def _camera(cx, cy, distance, pitch=60.0, yaw=-60.0):
    cam = OrbitCamera()
    cam.aspect = W / H
    cam.target = V(cx, cy, 0)
    cam.distance = distance
    cam.pitch = math.radians(pitch)
    cam.yaw = math.radians(yaw)
    return cam


def _box_select(vp, rect, crossing, packed=True):
    """SelectTool's box with (default) or without the packed path."""
    keep = (select._box_groups_packed, select._box_loose_arrays)
    if not packed:
        select._box_groups_packed = lambda *a, **k: None
        select._box_loose_arrays = lambda *a, **k: None
    try:
        SelectTool().on_box_select(vp, rect, crossing, mode="replace")
    finally:
        select._box_groups_packed, select._box_loose_arrays = keep
    # Elements by key (a store element is a view per read), objects by id.
    return {e.key if isinstance(e, (Vertex, Edge, Face)) else id(e)
            for e in vp.scene.selection}


def _scene(**kw):
    scene = Scene()
    site, side = _site(**kw)
    scene.groups.append(site)
    # Two objects of the host's own beside the site.
    scene.groups.append(_placed(_box(), -3, -3, "host A"))
    scene.groups.append(Group(_sheet(-3, side + 1, 0), name="host B"))
    return scene, side


def test_whole_scene_box_projects_no_vertex_one_by_one():
    """The scale test: a window and a crossing box over a 2 400-placement
    object answer from the pack — the old walk projected every vertex of
    every placement through the facade (about 72 000 calls here)."""
    scene, side = _scene(parts=20, per_part=100, sheets=20)
    vp = _Facade(scene, _camera(side / 2, side / 2, side * 2.2))
    rect = (0.0, 0.0, float(W), float(H))
    for crossing in (False, True):
        vp.projected = 0
        start = time.perf_counter()
        got = _box_select(vp, rect, crossing)
        took = time.perf_counter() - start
        assert id(scene.groups[0]) in got, "the site is taken whole"
        assert vp.projected < 1000, vp.projected
        assert took < 2.0, took


@pytest.mark.parametrize("crossing", [False, True])
@pytest.mark.parametrize("rect", [
    (0, 0, W, H),                  # everything
    (0, 0, W / 2, H),              # half: the site straddles
    (W * .3, H * .3, W * .7, H * .7),
    (W * .45, H * .45, W * .5, H * .5),   # inside one object's box
    (2, 2, 40, 30),                # a corner, maybe empty
])
def test_packed_answer_matches_the_walk(rect, crossing):
    scene, side = _scene(parts=3, per_part=30, sheets=5)
    vp = _Facade(scene, _camera(side / 2, side / 2, side * 2.0))
    assert _box_select(vp, rect, crossing) == \
        _box_select(vp, rect, crossing, packed=False)


@pytest.mark.parametrize("crossing", [False, True])
def test_open_group_children_match_the_walk(crossing):
    """Inside an open group the box takes its children, as the walk does."""
    scene, side = _scene(parts=3, per_part=30, sheets=5)
    part = scene.groups[0].children[1]
    scene.begin_group_edit(scene.groups[0])
    scene.begin_group_edit(part)
    vp = _Facade(scene, _camera(side / 2, side / 2, side * 1.2))
    for rect in ((0, 0, W, H), (0, 0, W / 2, H), (W * .3, H * .2, W * .6, H * .7)):
        new = _box_select(vp, rect, crossing)
        old = _box_select(vp, rect, crossing, packed=False)
        assert new == old
        assert new <= {id(c) for c in part.children}


@pytest.mark.parametrize("crossing", [False, True])
def test_camera_inside_the_model_matches_the_walk(crossing):
    """Instances partly behind the eye take the exact test."""
    scene, side = _scene(parts=3, per_part=30, sheets=5)
    cam = _camera(side / 2, side / 2, 2.0, pitch=10.0)
    vp = _Facade(scene, cam)
    for rect in ((0, 0, W, H), (W * .2, H * .2, W * .8, H * .8)):
        assert _box_select(vp, rect, crossing) == \
            _box_select(vp, rect, crossing, packed=False)


def test_hidden_object_is_not_taken():
    scene, side = _scene(parts=2, per_part=10, sheets=2)
    scene.groups[1].hidden = True
    vp = _Facade(scene, _camera(side / 2, side / 2, side * 2.2))
    got = _box_select(vp, (0, 0, W, H), True)
    assert id(scene.groups[1]) not in got
    assert id(scene.groups[0]) in got


def _loose_scene():
    scene = Scene()
    m = _sheet(0, 0, 0, size=4.0, n=30)
    scene.mesh = m
    return scene


@pytest.mark.parametrize("crossing", [False, True])
def test_loose_store_mesh_matches_the_walk(crossing):
    from built_core.store_mesh import StoreMesh
    scene = _loose_scene()
    if scene.mesh.__class__ is not StoreMesh:
        pytest.skip("mesh store off")
    from core.mesh import element_identity
    vp = _Facade(scene, _camera(2, 2, 7.0))
    for rect in ((0, 0, W, H), (W * .3, H * .3, W * .6, H * .7)):
        _box_select(vp, rect, crossing)
        new = {element_identity(e) for e in scene.selection}
        vp.projected = 0
        _box_select(vp, rect, crossing, packed=False)
        old = {element_identity(e) for e in scene.selection}
        assert new == old and new

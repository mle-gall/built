# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A sheet frame showing other layers than the live model takes its
geometry from the placements' cached arrays (``hlr_geometry_now``), not
from the per-face walk that kept Marco waiting ~20 s on the Queñuamarca
plaza before the first dimension snapped (2026-11-12). Same geometry, read
with the layers as they are switched at the time of asking."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401

import numpy as np  # noqa: E402
from built_core import Vec3 as V

from built_core.layers import Layer  # noqa: E402


def _box(scene, layer, x0):
    from built_core.group import Group
    from built_core.mesh import Mesh
    group = Group(Mesh(), name=f"box {layer}")
    group.layer = layer
    p = [V(x0, 0, 0), V(x0 + 6, 0, 0), V(x0 + 6, 2, 0), V(x0, 2, 0),
         V(x0, 0, 3), V(x0 + 6, 0, 3), V(x0 + 6, 2, 3), V(x0, 2, 3)]
    for f in ((0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7),
              (4, 5, 6, 7), (3, 2, 1, 0)):
        group.mesh.add_face([p[i] for i in f])
    scene.groups.append(group)


def _edge_set(arr):
    return sorted(tuple(sorted((tuple(np.round(a, 4)), tuple(np.round(b, 4)))))
                  for a, b in arr)


def _area(tris):
    if not len(tris):
        return 0.0
    return float(np.linalg.norm(np.cross(tris[:, 1] - tris[:, 0],
                                         tris[:, 2] - tris[:, 0]),
                                axis=1).sum() / 2)


def test_the_arrays_match_the_walk_with_the_layers_switched_now(native_host):
    from built_core.hlr import collect_geometry
    win = native_host.window
    vp = win.viewport
    scene = vp.scene
    scene.layers.append(Layer("OG", visible=False))
    scene.layers.append(Layer("EG"))
    _box(scene, "OG", 0.0)
    _box(scene, "EG", 10.0)
    scene.loose_mesh.add_edge(V(0, -5, 0), V(4, -5, 0))
    scene.version += 1
    vp.hlr_geometry()                     # the live view: OG hidden
    for og, eg in ((True, False), (True, True), (False, True)):
        scene.layer("OG").visible = og
        scene.layer("EG").visible = eg
        tris, hard, soft, _n = vp.hlr_geometry_now()
        w_tris, w_hard, w_soft = collect_geometry(scene)
        w_hard = np.asarray(w_hard, dtype=float).reshape(-1, 2, 3)
        assert _edge_set(hard) == _edge_set(w_hard)
        assert len(soft) == len(w_soft)
        w_tris = np.asarray(w_tris, dtype=float).reshape(-1, 3, 3)
        assert abs(_area(tris) - _area(w_tris)) < 1e-6
        # both boxes or one: 2 × (5 × 18 + 6 × 12)… just: not empty
        assert len(hard) >= 12 * (og + eg)


def _canon(arrays):
    tris, hard, soft, normals = arrays
    def rows(a, k):
        return sorted(tuple(np.round(r.reshape(-1), 5)) for r in a.reshape(-1, k))
    nn = np.nan_to_num(normals, nan=9.0)
    return (rows(tris, 9), rows(hard, 6), rows(soft, 6), rows(nn, 6))


def test_copies_not_baked_yet_come_out_as_their_bakes(native_host):
    """Copies of a component the view has not baked are placed in one go
    per prototype (1.2 s → a few ms on the plaza's 3 400): the very
    triangles, edges and soft edges with their normals that baking each
    copy gives — turned, mirrored and scaled ones too."""
    from built_core import Matrix4 as QMatrix4x4
    from built_core.group import Group
    from built_core.mesh import Mesh
    win = native_host.window
    vp = win.viewport
    scene = vp.scene
    proto = Mesh()
    p = [V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0),
         V(0, 0, 1), V(1, 0, 1), V(1, 1, 1), V(0, 1, 1)]
    for f in ((0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7),
              (4, 5, 6, 7), (3, 2, 1, 0)):
        proto.add_face([p[i] for i in f])
    # A soft edge with its two faces: its normals must follow the copy.
    e = proto.find_edge(proto.vertex_at(p[1]), proto.vertex_at(p[5]))
    e.soft = True
    mats = []
    m = QMatrix4x4(); m.translate(5, 0, 0); mats.append(m)
    m = QMatrix4x4(); m.translate(0, 7, 2); m.rotate(33, 0.3, 0.2, 1); mats.append(m)
    m = QMatrix4x4(); m.translate(-4, 1, 0); m.scale(-1, 2, 0.5); mats.append(m)
    for i, m in enumerate(mats):
        g = Group(proto, name=f"copy {i}")
        g.xform = m
        scene.groups.append(g)
    scene.version += 1
    import views.document_interaction as VP
    calls = []
    real = VP._batch_instances
    VP._batch_instances = lambda base, xfs, *a: (calls.append(len(xfs)),
                                                 real(base, xfs, *a))
    try:
        batched = vp.hlr_geometry_now()
    finally:
        VP._batch_instances = real
    assert calls == [3]                      # one batch, three copies
    assert len(batched[0]) == 3 * 12 and len(batched[2]) == 3
    # Bake each copy the usual way; then every copy is read baked.
    for g in vp._placements():
        vp._group_chunk(g)
    baked = vp.hlr_geometry_now()
    a, b = _canon(batched), _canon(baked)
    for x, y in zip(a, b):
        assert len(x) == len(y)
        assert np.allclose(np.array(x), np.array(y), atol=1e-5)

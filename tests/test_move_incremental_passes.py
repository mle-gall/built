# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A Move updates the placement passes row by row (#158, «sin lag»).

Picking an object up (the preview) and putting it down (one matrix) made
every per-placement pass start over: on the plant of #158 each was ~1 s.
The placements epoch now remembers what it was made of, so a pass can ask
whether only matrices moved since it was built — and then bring just those
rows up to date. Every answer here is compared with a pass made afresh."""
from __future__ import annotations

import math

import numpy as np  # noqa: E402
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V


def _prisma(n=12, r=1.0, h=2.0):
    """A smooth n-sided prism: soft vertical edges, a curved side."""
    from core.mesh import Mesh
    m = Mesh()
    ring = [(r * math.cos(2 * math.pi * i / n), r * math.sin(2 * math.pi * i / n))
            for i in range(n)]
    for i in range(n):
        (x0, y0), (x1, y1) = ring[i], ring[(i + 1) % n]
        m.add_face([V(x0, y0, 0), V(x1, y1, 0), V(x1, y1, h), V(x0, y0, h)])
    for e in m.edges:
        if abs(e.a.z() - e.b.z()) > 1e-9:
            e.soft = True
    return m


def _scene(native_host):
    from core.group import Group

    win = native_host.window
    vp = win.viewport
    protos = [_prisma(12), _prisma(7, r=0.5, h=3.0)]
    gs = []
    for k in range(10):
        m = QMatrix4x4()
        m.translate(2.5 * k, (k % 3) * 1.7, 0.2 * k)
        m.rotate(23.0 * k, 0.1 * (k % 2), 0.3, 1.0)
        g = Group(protos[k % 2], name=f"P{k}")
        g.xform = m
        vp.scene.groups.append(g)
        gs.append(g)
    vp.scene.version += 1
    return win, vp, gs


def test_an_edit_of_a_mesh_is_not_only_matrices(native_host):
    win, vp, gs = _scene(native_host)
    old = vp._placements_epoch()
    gs[0].mesh.add_face([V(0, 0, 5), V(1, 0, 5), V(1, 1, 5)])
    vp.scene.version += 1
    assert not vp._epoch_only_matrices(old)


def test_a_new_layer_state_is_not_only_matrices(native_host):
    win, vp, gs = _scene(native_host)
    old = vp._placements_epoch()
    gs[4].hidden = True
    vp.scene.version += 1
    assert not vp._epoch_only_matrices(old)


def test_moving_a_plain_group_is_only_matrices_unless_its_mesh_is_a_prototype(native_host):
    from core.group import Group
    win, vp, gs = _scene(native_host)
    plain = Group(_prisma(5), name="plain")             # no matrix
    vp.scene.groups.append(plain)
    vp.scene.version += 1
    old = vp._placements_epoch()
    plain.mesh.add_face([V(0, 0, 9), V(1, 0, 9), V(1, 1, 9)])   # edited
    vp.scene.version += 1
    assert vp._epoch_only_matrices(old)
    # the same mesh drawn by a placement too: its edit is not a move
    shared = Group(gs[0].mesh, name="shared")
    vp.scene.groups.append(shared)
    vp.scene.version += 1
    old = vp._placements_epoch()
    shared.mesh.add_face([V(0, 0, 7), V(1, 0, 7), V(1, 1, 7)])
    vp.scene.version += 1
    assert not vp._epoch_only_matrices(old)

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Resolving a dimension's vertex anchor does not build the lazy meshes
it walks past. On Marco's Queñuamarca plaza (3 400 placements) each
anchored cota on a sheet built all 164 deferred meshes to ask their
registries — 3.6 s on opening the composer (2026-11-12)."""
from __future__ import annotations

from PySide6.QtGui import QVector3D as V

from core import mesh as M
from core.dimension import resolve_vertex_anchor
from tests.test_deferred_mesh import _open, binary  # noqa: F401
from tests.test_igz_format3 import _scene
from formats import igz


def test_an_anchor_builds_only_the_mesh_it_sits_on(binary, monkeypatch,
                                                   tmp_path):
    path = tmp_path / "doc.igz"
    igz.save_scene(_scene(tmp_path), path)
    lazy = _open(monkeypatch, path, True)
    meshes = {id(g.mesh): g.mesh for g in lazy.groups}
    deferred = [m for m in meshes.values() if type(m) is M.DeferredMesh]
    assert deferred, "the groups should open deferred"
    # A point on no group: nothing is built, no anchor.
    assert resolve_vertex_anchor(lazy, V(55, 55, 55)) is None
    assert all(type(m) is M.DeferredMesh for m in deferred)
    # The corner of the second chair (placed at x = 5, y = 20): found,
    # and only the chairs' prototype is built.
    mesa = next(g.mesh for g in lazy.groups if g.name == "Mesa")
    mesa_lazy = type(mesa) is M.DeferredMesh
    a = resolve_vertex_anchor(lazy, V(6, 21, 0))
    assert a is not None
    assert (type(mesa) is M.DeferredMesh) == mesa_lazy   # left alone
    # The classic group's corner is found too, after the cache of
    # placements was made.
    assert resolve_vertex_anchor(lazy, V(2, 30, 0)) is not None


def test_shared_placements_are_filtered_before_vertex_lookup(monkeypatch):
    from PySide6.QtGui import QMatrix4x4
    from core.group import Group
    from core.scene import Scene
    mesh = M.Mesh()
    mesh.add_edge(V(0, 0, 0), V(1, 0, 0))
    scene = Scene()
    for i in range(64):
        group = Group(mesh)
        group.xform = QMatrix4x4()
        group.xform.translate(4*i, 0, 0)
        group.xform.rotate(45, 0, 0, 1)
        scene.groups.append(group)
    checks = []
    lookup = mesh.vertex_at
    def counted(point):
        checks.append(point)
        return lookup(point)
    monkeypatch.setattr(mesh, "vertex_at", counted)
    anchor = resolve_vertex_anchor(scene, V(252, 0, 0))
    assert anchor.chain == (scene.groups[-1],)
    assert len(checks) == 1
    # A growing shared prototype gets its new bounds on the next lookup.
    mesh.add_edge(V(0, 5, 0), V(1, 5, 0))
    p = scene.groups[-1].xform.map(V(0, 5, 0))
    assert resolve_vertex_anchor(scene, p).chain == (scene.groups[-1],)

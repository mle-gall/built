# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""``ingetrazo_core.scene_mesh_states`` (and ``scene_group_states`` for
placements) answers, for every face and edge of
a mesh at once, what ``Scene.entity_visible``, ``entity_selectable`` and
``edge_snappable`` answer one by one (``model/scene_mesh_states.rs``): on
the seeded scenes of ``tests/test_scene_visibility_parity.py`` (whose
per-entity answers are pinned against the Python at ``61fe6326``), at the
model and inside open groups and components, for the loose mesh, the open
context's mesh and every placement's prototype."""
from __future__ import annotations

import itertools
import random

import ingetrazo_core as N

from tests import test_scene_visibility_parity as V


def _check(scene):
    meshes = [scene.loose_mesh, scene.mesh] + [g.mesh for g in scene.groups] + \
        [c.mesh for g in scene.groups for c in g.children]
    seen = set()
    n = 0
    for mesh in meshes:
        if id(mesh) in seen:
            continue
        seen.add(id(mesh))
        faces, edges = list(mesh.faces), list(mesh.edges)
        (fv, fs), (ev, es, esel) = N.scene_mesh_states(scene, mesh, faces, edges)
        assert fv == [bool(scene.entity_visible(f)) for f in faces]
        assert fs == [bool(scene.entity_selectable(f)) for f in faces]
        assert ev == [bool(scene.entity_visible(e)) for e in edges]
        assert es == [bool(scene.edge_snappable(e)) for e in edges]
        assert esel == [bool(scene.entity_selectable(e)) for e in edges]
        n += len(faces) + len(edges)
    groups = [g for g in scene.groups] + [c for g in scene.groups for c in g.children]
    states = N.scene_group_states(scene, groups)
    assert states == [(bool(scene.entity_visible(g)), bool(scene.group_snappable(g)),
                       bool(scene.entity_selectable(g))) for g in groups]
    return n + len(groups)


def test_one_call_per_mesh_answers_as_the_rules_one_by_one():
    from core import layers as L
    counter = itertools.count()
    real = L.new_uid
    L.new_uid = lambda: f"k{next(counter)}"
    try:
        rng = random.Random(20261125)
        total = 0
        for _ in range(60):
            scene = V._scene(rng)
            total += _check(scene)
            opened = [g for g in scene.groups if not g.locked]
            if opened:
                g = rng.choice(opened)
                scene.begin_group_edit(g)
                total += _check(scene)
                if g.children:
                    scene.begin_group_edit(g.children[0])
                    total += _check(scene)
                scene.end_group_edit()
        assert total > 2000
    finally:
        L.new_uid = real

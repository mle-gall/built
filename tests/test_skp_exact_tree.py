# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The .skp tree arrives as it is drawn (#exact-tree).

asbuilt.skp: «Groupe#5992» (a beam frame) places a 52-face beam
twice and, inside it, a 6-face plate. The sharing thresholds (60 polygons,
400 saved) melted both copies — and the plate inside — into the frame's own
mesh, so the beam could no longer be selected, tagged or painted as itself.
Every placement must come in as its own object at its own depth, small or
not, once or many times."""
from __future__ import annotations

from types import SimpleNamespace as NS


import pytest  # noqa: E402

pytest.importorskip("openskp")

from built_core.scene import Scene  # noqa: E402
from formats import skp, skp_reader  # noqa: E402

from tests.test_skp_nested_parts_866 import _ID, _box  # noqa: E402


def _model():
    plate = _box(5, "Plate", 1, 1, 0.1)
    beam = _box(6, "Beam", 10, 1, 1)
    beam.instances = [NS(ref_idx=5, matrix=_ID[:9] + [0, 0, 1, 1])]
    frame = NS(id=7, name="Frame", vertices={}, edges={}, faces={},
               instances=[NS(ref_idx=6, matrix=_ID),
                          NS(ref_idx=6, matrix=_ID[:9] + [0, 5, 0, 1])])
    root = NS(id=0, name="ROOT_MODEL", vertices={}, edges={}, faces={},
              instances=[NS(ref_idx=7, matrix=_ID)])
    return NS(definitions={d.id: d for d in (root, plate, beam, frame)})


def _tree(g):
    return (g.name, len(g.mesh.faces), sorted(_tree(c) for c in g.children))


def test_small_shared_components_keep_their_place_in_the_tree():
    scene = Scene()
    skp.apply_payload(scene, skp_reader._adapt(_model(), "exact_tree"))
    [frame] = scene.groups
    assert _tree(frame) == ("Frame", 0, [("Beam", 6, [("Plate", 6, [])]),
                                         ("Beam", 6, [("Plate", 6, [])])])
    a, b = frame.children
    assert a.mesh is b.mesh                    # the two copies share one mesh
    assert a.component and b.component


def test_the_flag_still_restores_the_old_flattening(monkeypatch):
    monkeypatch.setattr(skp_reader, "EXACT_TREE", False)
    scene = Scene()
    skp.apply_payload(scene, skp_reader._adapt(_model(), "exact_tree"))
    [frame] = scene.groups
    assert len(frame.mesh.faces) == 24 and not frame.children

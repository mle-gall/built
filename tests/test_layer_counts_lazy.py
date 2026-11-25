# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Layers panel counts what each layer carries without building the
group meshes the loader left lazy (release_check 0.5.11: counting through
``mesh.faces`` built all 888 of plaza's, 95 MB more on every open). The
count read from a lazy mesh's source is the count of the built mesh."""
from __future__ import annotations

import random
from collections import Counter

import pytest

import core._native as _native
from built_core import mesh as M
from built_core.layers import DEFAULT_LAYER, layer_of
from built_core.purge import layer_carriers, layer_counts, used_layers
from built_core.scene import Scene
from formats import igz
from tests.test_deferred_mesh import _meshes, _random_scene


@pytest.fixture
def binary(monkeypatch):
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    monkeypatch.setattr(_native, "wanted", lambda: True)


def _labelled(rng) -> Scene:
    scene = _random_scene(rng)
    names = ("Muros", "Losa", "Mobiliario")
    for m in _meshes(scene):
        for f in m.faces:
            if rng.random() < 0.5:
                f.attrs["layer"] = rng.choice(names)
        for e in m.edges:
            if rng.random() < 0.4:
                e.layer = rng.choice(names)
    for g in scene.groups:
        if rng.random() < 0.5:
            g.layer = rng.choice(names)
    return scene


@pytest.mark.parametrize("seed", range(6))
def test_counting_a_lazy_document_builds_nothing_and_counts_the_same(
        binary, tmp_path, seed):
    path = tmp_path / "doc.igz"
    igz.save_scene(_labelled(random.Random(seed)), path)
    scene = Scene()
    igz.load_into(scene, path)
    lazy = [m for m in _meshes(scene) if type(m) is M.DeferredMesh]
    assert lazy, "the fixture must leave group meshes lazy"

    got = layer_counts(scene)
    used = used_layers(scene)

    assert all(type(m) is M.DeferredMesh for m in lazy)
    assert all("_deferred_plan" not in m.__dict__ for m in lazy)
    built = Counter(layer_of(e) for e in layer_carriers(scene))
    assert got == dict(built)
    assert used == {DEFAULT_LAYER} | set(built)


def test_an_eager_document_counts_as_before(tmp_path):
    scene = _labelled(random.Random(7))
    built = Counter(layer_of(e) for e in layer_carriers(scene))
    assert layer_counts(scene) == dict(built)


def test_welded_duplicate_last_layer_and_missing_boundary_edges():
    import numpy as np
    from built_core.purge import _count_edges

    pos = np.array([[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 0]])
    edges = np.array([[0, 1], [1, 3], [0, 3], [0, 1]])
    src = dict(pos=pos, edges=edges, flags=np.zeros(4), layers=np.array([0, 1, 0, -1]),
               rings=np.array([0, 1, 2]), sizes=np.array([3]), meta=dict(edge_layers=['a', 'b']))
    counts = Counter()
    _count_edges(src, counts)
    assert counts == {None: 2, 'b': 1}


def test_native_inspector_publishes_only_a_complete_current_count(monkeypatch):
    from time import sleep
    from built_core import purge
    from built_core.history import History
    from built_render import TagCounts

    def steps(scene, *, history):
        assert history.scene is scene
        version = scene.content_version
        sleep(.008)
        yield None
        yield {"tag": version}

    monkeypatch.setattr(purge, "layer_count_steps", steps)
    scene = Scene()
    history = History(scene)
    counts = TagCounts()
    assert counts.tick(history) and counts.count("tag") is None
    scene.version += 1
    assert counts.tick(history) and counts.count("tag") is None
    assert not counts.tick(history) and counts.count("tag") == scene.content_version
    assert counts.count("empty") == 0

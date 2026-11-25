# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Layer state, selection membership and marks in the Drawing lens."""
from __future__ import annotations

import ingetrazo_core
import random

import pytest
from tests.test_project_drawing_filters import Project

from core import layers as L
from core.group import Group
from core.layers import DEFAULT_LAYER, Layer, layer_of
from core.mesh import Mesh
from core.scene import Scene


def drawing():
    project = Project()
    scene = project.scene
    scene.layers += [Layer("A"), Layer("B")]
    ga = Group(Mesh(), name="ga")
    ga.layer = "A"
    gb = Group(Mesh(), name="gb")
    gb.layer = "B"
    scene.groups += [ga, gb]
    project.action("lens", "1")
    return project, ga, gb


def test_is_isolated_reads_the_default_layer_as_always_on():
    scene = Scene()
    assert not ingetrazo_core.tag_edits.is_isolated(scene, DEFAULT_LAYER)
    scene.layers.append(Layer("Solo"))
    assert not ingetrazo_core.tag_edits.is_isolated(scene, "Solo")         # nothing else to hide
    scene.layers.append(Layer("Otra", visible=False))
    assert ingetrazo_core.tag_edits.is_isolated(scene, "Solo")
    scene.layer("Solo").visible = False
    assert not ingetrazo_core.tag_edits.is_isolated(scene, "Solo")


def marked(project):
    return {key: state[10] for key, (_, state) in project.rows().items()
            if key.startswith("preview-") and state[10]}


def test_drawing_marks_selection_tags_without_changing_the_active_tag():
    project, a, b = drawing()
    scene = project.scene
    L.set_active(scene, "B")
    scene.select([a])
    assert marked(project) == {"preview-tag:A": 1}
    assert L.active_layer(scene) == "B"
    scene.select([a, b])
    assert marked(project) == {"preview-tag:A": 1, "preview-tag:B": 1}
    scene.clear_selection()
    assert marked(project) == {}


def test_a_folded_folder_carries_the_mark():
    project, a, _ = drawing()
    scene = project.scene
    ingetrazo_core.tag_edits.move_to_folder(scene, ["A"], "Arq")
    scene.select([a])
    assert marked(project) == {"preview-folder:Arq": 1}
    project.action("toggle", "preview-folder:Arq")
    assert marked(project) == {"preview-folder:Arq": 2, "preview-tag:A": 1}
    project.action("toggle", "preview-folder:Arq")
    assert marked(project) == {"preview-folder:Arq": 1}


@pytest.mark.parametrize("seed", range(10))
def test_selection_layers_matches_random_selections(seed):
    from ingetrazo_core import Vec3 as V
    rng = random.Random(seed)
    scene = Scene()
    names = [f"L{i}" for i in range(4)]
    scene.layers += [Layer(n) for n in names]
    ents = []
    for i in range(8):
        g = Group(Mesh(), name=f"g{i}")
        g.layer = rng.choice(names + [None])
        scene.groups.append(g)
        ents.append(g)
    f = scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    ents.append(f)
    pick = rng.sample(ents, rng.randint(0, len(ents)))
    scene.select(pick)
    assert L.selection_layers(scene) == {layer_of(e) for e in pick}


@pytest.mark.parametrize("seed", range(15))
def test_layer_state_round_trips(seed):
    rng = random.Random(seed)
    scene = Scene()
    scene.layers += [Layer(f"L{i}") for i in range(rng.randint(1, 6))]
    names = [ly.name for ly in scene.layers]
    for ly in scene.layers:
        ly.visible, ly.locked = rng.random() < 0.7, rng.random() < 0.3
    L.set_active(scene, rng.choice(names[1:]))
    saved = ingetrazo_core.tag_edits.layer_state(scene)
    before = [(ly.name, ly.visible, ly.locked) for ly in scene.layers]
    active = scene.active_layer
    for ly in scene.layers:
        ly.visible, ly.locked = rng.random() < 0.5, rng.random() < 0.5
    scene.active_layer = None
    ingetrazo_core.tag_edits.apply_layer_state(scene, saved)
    assert [(ly.name, ly.visible, ly.locked) for ly in scene.layers] == before
    assert scene.active_layer == active


def test_layer_state_skips_layers_made_or_deleted_since():
    scene = Scene()
    scene.layers += [Layer("A", visible=False), Layer("B")]
    L.set_active(scene, "B")
    saved = ingetrazo_core.tag_edits.layer_state(scene)
    scene.layers.remove(scene.layer("B"))
    scene.layers.append(Layer("C", visible=False))
    scene.layer("A").visible = True
    ingetrazo_core.tag_edits.apply_layer_state(scene, saved)
    assert not scene.layer("A").visible
    assert not scene.layer("C").visible               # untouched
    assert scene.active_layer is None                  # B is gone

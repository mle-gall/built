# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Layer state, selection membership and marks in the legacy dock."""
from __future__ import annotations

import os
import random

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PySide6.QtWidgets import QApplication

_app = QApplication.instance() or QApplication([])

from core import layers as L
from core.group import Group
from core.layers import DEFAULT_LAYER, Layer, layer_of
from core.mesh import Mesh
from core.scene import Scene


@pytest.fixture
def win():
    from views.main_window import MainWindow
    w = MainWindow()
    w.show()
    _app.processEvents()
    yield w
    w._saved_version = w.viewport.scene.version
    w.close()


def _doc(win):
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B")]
    ga = Group(Mesh(), name="ga")
    ga.layer = "A"
    gb = Group(Mesh(), name="gb")
    gb.layer = "B"
    scene.groups += [ga, gb]
    win.viewport.notify_scene_changed()
    return scene, ga, gb


def test_is_isolated_reads_the_default_layer_as_always_on():
    scene = Scene()
    assert not L.is_isolated(scene, DEFAULT_LAYER)
    scene.layers.append(Layer("Solo"))
    assert not L.is_isolated(scene, "Solo")         # nothing else to hide
    scene.layers.append(Layer("Otra", visible=False))
    assert L.is_isolated(scene, "Solo")
    scene.layer("Solo").visible = False
    assert not L.is_isolated(scene, "Solo")


def _holds(panel):
    from views.layers_panel import ROLE_HOLDS
    return sorted(panel._key(it)[1] for it in panel._all_items()
                  if it.data(0, ROLE_HOLDS))


def test_the_panel_marks_the_selections_layer_and_not_the_active(win):
    scene, ga, gb = _doc(win)
    panel = win.layers_dock.panel
    L.set_active(scene, "B")
    scene.select([ga])
    win.viewport.notify_scene_changed()
    _app.processEvents()                              # the rebuild is deferred
    assert _holds(panel) == ["A"]
    assert L.active_layer(scene) == "B"               # untouched
    scene.select([ga, gb])
    win.viewport.notify_scene_changed()
    _app.processEvents()
    assert _holds(panel) == ["A", "B"]                # several: all of them
    scene.clear_selection()
    win.viewport.notify_scene_changed()
    _app.processEvents()
    assert _holds(panel) == []


def test_a_folded_folder_carries_the_mark(win):
    scene, ga, _gb = _doc(win)
    panel = win.layers_dock.panel
    L.move_to_folder(scene, ["A"], "Arq")
    scene.select([ga])
    win.viewport.notify_scene_changed()
    _app.processEvents()                              # the rebuild is deferred
    folder = panel.item_for("Arq", "folder")
    assert _holds(panel) == ["A"]                     # open: the layer
    folder.setExpanded(False)
    assert _holds(panel) == ["A", "Arq"]              # folded: the folder too
    folder.setExpanded(True)
    assert _holds(panel) == ["A"]


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
    saved = L.layer_state(scene)
    before = [(ly.name, ly.visible, ly.locked) for ly in scene.layers]
    active = scene.active_layer
    for ly in scene.layers:
        ly.visible, ly.locked = rng.random() < 0.5, rng.random() < 0.5
    scene.active_layer = None
    L.apply_layer_state(scene, saved)
    assert [(ly.name, ly.visible, ly.locked) for ly in scene.layers] == before
    assert scene.active_layer == active


def test_layer_state_skips_layers_made_or_deleted_since():
    scene = Scene()
    scene.layers += [Layer("A", visible=False), Layer("B")]
    L.set_active(scene, "B")
    saved = L.layer_state(scene)
    scene.layers.remove(scene.layer("B"))
    scene.layers.append(Layer("C", visible=False))
    scene.layer("A").visible = True
    L.apply_layer_state(scene, saved)
    assert not scene.layer("A").visible
    assert not scene.layer("C").visible               # untouched
    assert scene.active_layer is None                  # B is gone

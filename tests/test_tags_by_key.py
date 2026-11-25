# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Tags and tag folders are known by a key; their names are labels
(#tags-by-key).

.skp wants tag names unique, so a model's tags came in as «Observation
Deck - Steel - Beams», «Preshow 1 - Steel - Beams»… — the folder path spelt
out in every name. Entities now carry the tag's KEY (the name it was made
with, stable), and the name is only what the panels show: free to repeat
(«Beams» in every folder) and to change without relabelling anything."""
from __future__ import annotations

import ingetrazo_core
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from ingetrazo_core import Vec3 as QVector3D
from PySide6.QtWidgets import QApplication  # noqa: E402

_app = QApplication.instance() or QApplication([])

from core import layers as L  # noqa: E402
from core.group import Group  # noqa: E402
from core.layers import DEFAULT_LAYER, Layer  # noqa: E402
from core.mesh import Mesh  # noqa: E402
from core.scene import Scene  # noqa: E402


def _square():
    m = Mesh()
    m.add_face([QVector3D(0, 0, 0), QVector3D(1, 0, 0), QVector3D(1, 1, 0),
                QVector3D(0, 1, 0)])
    return m


def _two_parts():
    """Two folders, each holding a tag labelled «Beams», each tag carried
    by one group."""
    scene = Scene()
    deck = L.add_folder(scene, "Deck")
    hall = L.add_folder(scene, "Hall")
    L.rename_folder(scene, deck, "Deck")
    L.rename_folder(scene, hall, "Hall")
    scene.layers += [Layer("Deck - Steel - Beams", folder=deck),
                     Layer("Hall - Steel - Beams", folder=hall)]
    a, b = Group(_square(), name="a"), Group(_square(), name="b")
    a.layer, b.layer = "Deck - Steel - Beams", "Hall - Steel - Beams"
    scene.groups += [a, b]
    assert L.rename_layer(scene, "Deck - Steel - Beams", "Beams")
    assert L.rename_layer(scene, "Hall - Steel - Beams", "Beams")
    return scene, a, b, deck, hall


def test_labels_repeat_and_entities_keep_their_key():
    scene, a, b, deck, hall = _two_parts()
    assert [ly.name for ly in scene.layers[1:]] == ["Beams", "Beams"]
    assert a.layer == "Deck - Steel - Beams" and b.layer == "Hall - Steel - Beams"
    assert L.path_label(scene, a.layer) == "Deck/Beams"
    assert L.path_label(scene, b.layer) == "Hall/Beams"


def test_hiding_one_beams_tag_leaves_the_other_shown():
    scene, a, b, *_ = _two_parts()
    ingetrazo_core.tag_edits.set_visible(scene, [a.layer], False)
    assert not scene._layer_state(a)[0] and scene._layer_state(b)[0]


def test_keys_and_labels_survive_the_file(tmp_path):
    from formats import igz
    scene, a, b, deck, hall = _two_parts()
    L.rename_folder(scene, deck, "Observation Deck")
    path = tmp_path / "tags.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    by_name = {g.name: g for g in back.groups}
    assert by_name["a"].layer == "Deck - Steel - Beams"
    assert back.layer_by_key("Deck - Steel - Beams").name == "Beams"
    assert L.folder_label(back, deck) == "Observation Deck"
    assert L.path_label(back, by_name["a"].layer) == "Observation Deck/Beams"


def test_a_file_from_before_keys_reads_names_as_keys(tmp_path):
    """No «key» field: the name IS the key, as it always was."""
    raw = Layer.from_dict({"name": "Muros", "uid": "u1"})
    assert raw.key == "Muros" and raw.name == "Muros"
    assert "key" not in Layer("Muros").to_dict()


def test_exports_name_tags_by_their_folder_path():
    scene, a, b, *_ = _two_parts()
    labels = L.export_labels(scene)
    assert labels[a.layer] == "Deck/Beams" and labels[b.layer] == "Hall/Beams"
    assert labels[DEFAULT_LAYER] == DEFAULT_LAYER
    rows, swatches = L.legend_rows(scene)
    assert "Deck/Beams" in rows and "Hall/Beams" in rows


def test_assign_by_label_lands_on_the_key():
    from core.history import AssignLayerCommand, History
    scene, a, b, *_ = _two_parts()
    g = Group(_square(), name="c")
    scene.groups.append(g)
    history = History(scene)
    history.execute(AssignLayerCommand([g], "Hall - Steel - Beams"))
    assert g.layer == "Hall - Steel - Beams"
    history.undo()
    assert g.layer in (None, DEFAULT_LAYER)


def test_renaming_does_not_recolour():
    scene = Scene()
    ly = Layer("Muros")
    scene.layers.append(ly)
    before = L.layer_color(ly)
    L.rename_layer(scene, "Muros", "Walls")
    assert L.layer_color(ly) == before

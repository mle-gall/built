# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Ordering the Browser's rows by layer: a way of reading the tree, not a
change to the model. Siblings take the layers' order — the Layers tab's
own, folders included: the layers of a folder stay together where the
folder sits — and a group stays inside its parent, so drag, rename,
isolate and delete go on working on the same groups.

Every model here is generated, as in tests/test_outliner.py.
"""
from __future__ import annotations

import time

from ingetrazo_core import Vec3 as QVector3D

from core import layers as L
from core import outliner
from core.group import Group
from core.layers import DEFAULT_LAYER, Layer
from core.mesh import Mesh
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.native_tree import action
from tests.test_project_drawing_filters import Project

_QUADS = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6),
          (0, 2, 6, 4), (1, 5, 7, 3)]


def _box(at=(0.0, 0.0, 0.0), size=0.1) -> Mesh:
    mesh = Mesh()
    x0, y0, z0 = at
    c = [QVector3D(x0 + dx * size, y0 + dy * size, z0 + dz * size)
         for dz in (0, 1) for dy in (0, 1) for dx in (0, 1)]
    for q in _QUADS:
        mesh.add_face([c[k] for k in q])
    return mesh


def _group(name, layer=None, at=(0.0, 0.0, 0.0)) -> Group:
    g = Group(_box(at), name=name)
    g.layer = layer
    return g


def _container(name, kids, layer=None) -> Group:
    g = Group(name=name)
    g.adopt(kids)
    g.component = False
    g.layer = layer
    return g


# ---- The order itself, without a window -------------------------------------

def test_siblings_are_ordered_by_layer_and_stay_inside_their_parent():
    walls = _group("Walls", "Walls")
    roof = _group("Roof", "Roof")
    plain = _group("Plain")
    kid = _group("Kid", "Roof")
    host = _container("Host", [kid])
    groups = [walls, host, plain, roof]          # the document's own order
    rank = {DEFAULT_LAYER: 0, "Walls": 1, "Roof": 2}

    rows = outliner.walk_by_layer(groups, rank)
    assert [g.name for g, _p, _d in rows] == [
        "Host", "Kid", "Plain", "Walls", "Roof"]
    by_name = {g.name: (g, p, d) for g, p, d in rows}
    assert by_name["Kid"][1] is host              # inside its parent still
    assert by_name["Kid"][2] == 1
    # The model itself is left alone: this is a way of reading it.
    assert [g.name for g in groups] == ["Walls", "Host", "Plain", "Roof"]
    assert host.children == [kid]


def test_the_default_layer_comes_first_and_the_rest_as_the_tab_lists_them():
    scene = type("Scene", (), {
        "layers": [Layer(DEFAULT_LAYER), Layer("Roof"), Layer("Walls")]})()
    rank = outliner.layer_rank(scene)
    assert rank == {DEFAULT_LAYER: 0, "Roof": 1, "Walls": 2}
    assert rank[DEFAULT_LAYER] < rank["Roof"] < rank["Walls"]


def test_a_layer_that_no_longer_exists_sorts_after_every_one_of_them():
    scene = type("Scene", (), {"layers": [Layer(DEFAULT_LAYER)]})()
    gone = _group("Gone", "Deleted layer")
    plain = _group("Plain")
    rows = outliner.walk_by_layer([gone, plain], outliner.layer_rank(scene))
    assert [g.name for g, _p, _d in rows] == ["Plain", "Gone"]


def test_ten_thousand_rows_order_quickly():
    groups = [_group(f"G{i}", ("Walls", "Roof", None)[i % 3])
              for i in range(10_000)]
    scene = type("Scene", (), {
        "layers": [Layer(DEFAULT_LAYER), Layer("Walls"), Layer("Roof")]})()
    t0 = time.perf_counter()
    rows = outliner.walk_by_layer(groups, outliner.layer_rank(scene))
    elapsed = time.perf_counter() - t0
    assert len(rows) == 10_000
    assert rows[0][0].layer is None               # the default layer first
    assert elapsed < 1.0


# ---- Folder folders: the layers of a folder stay together --------------------

def _scene_with(*names, folders=(), parents=None):
    scene = type("Scene", (), {
        "layers": [Layer(DEFAULT_LAYER)] + [Layer(n) for n in names]})()
    scene.layer_folders = list(folders)
    scene.folder_parents = dict(parents or {})
    return scene


def test_the_layers_of_a_folder_are_ordered_together():
    """A layer filed into a folder takes the folder's place, however far
    apart the layers were made: the rows on them stay together."""
    scene = _scene_with("Muros", "Acero", "Vidrio", "Tabique",
                        folders=("Acabados", "Obra gris"))
    scene.layers[2].folder = "Acabados"          # Acero
    scene.layers[3].folder = "Acabados"          # Vidrio
    scene.layers[4].folder = "Obra gris"         # Tabique, made last
    assert L.ordered_names(scene) == [
        DEFAULT_LAYER, "Acero", "Vidrio", "Tabique", "Muros"]
    rank = outliner.layer_rank(scene)
    assert rank["Acero"] < rank["Vidrio"] < rank["Tabique"] < rank["Muros"]


def test_a_folder_inside_a_folder_holds_its_layers_first():
    scene = _scene_with("A", "B", "C", folders=("Estructura", "Hormigón"),
                        parents={"Hormigón": "Estructura"})
    scene.layers[1].folder = "Estructura"        # A
    scene.layers[2].folder = "Hormigón"          # B
    assert L.ordered_names(scene) == [DEFAULT_LAYER, "B", "A", "C"]


def test_filing_moves_only_the_tag_row_and_undo_restores_its_position():
    import json
    from views.wgpu_ui import SEP
    p = Project(Layer("Walls"), Layer("Steel", folder="Structure"), Layer("Glass", folder="Structure"))
    wall, column, glass = [_group(name, tag) for name, tag in
                           [("Wall", "Walls"), ("Column", "Steel"), ("Glass panel", "Glass")]]
    p.scene.groups[:] = [wall, column, glass]
    p.scene.version += 1
    p.action("lens", "1")
    p.action("toggle", "preview-folder:Structure")

    def visible():
        return [(key, state[0]) for key, (_, state) in p.rows().items() if key.startswith("preview-")]

    before = visible()
    folder = ("preview-folder:Structure", 0)
    assert before.index(folder) < before.index(("preview-tag:Steel", 1))
    assert before.index(folder) < before.index(("preview-tag:Glass", 1))
    p.action("tagfolder", "file" + SEP + json.dumps(dict(tags=["Walls"], parent="Structure")))
    assert ("preview-tag:Walls", 1) in visible()
    assert p.win.viewport.history.undo()
    assert visible() == before
    assert p.scene.groups == [wall, column, glass]


def test_sort_choice_is_restored_and_returning_to_project_order_keeps_the_model(native_host):
    from core.settings import settings
    p = Project(Layer("Roof"), Layer("Walls"), scene=native_host.viewport.scene)
    p.scene.groups[:] = [_group("Site"), _group("House", "Walls")]
    original = list(p.scene.groups)
    for mode in (1, 0):
        p.action("sort", str(mode))
        new_view = Project(scene=p.scene)
        new_view.rows()
        assert new_view.view.renderer.calls["ui_set_tree_sort"] == (mode,)
        assert settings().value("ui/project_tree_sort", -1, type=int) == mode
    assert p.scene.groups == original


def test_tag_assignment_and_drop_resolve_the_same_placement_after_sort(native_host):
    from core.protocol.commands import set_tag
    vp = native_host.viewport
    wall, desk = _group("Wall", "Walls"), _group("Desk")
    room = _container("Room", [wall], layer="Walls")
    vp.scene.groups[:] = [room, desk]
    vp.scene.layers.extend([Layer("Walls"), Layer("Furniture")])
    vp.scene.version += 1
    action(native_host, "sort", "1")
    action(native_host, "into", "i:"+desk.uid, "i:"+room.uid)
    assert room.children == [wall, desk] and desk not in vp.scene.groups
    assert wall.layer == "Walls" and desk.layer is None
    assert set_tag(vp.history, [desk.uid], "Furniture")["ok"]
    assert desk.layer == "Furniture" and desk in room.children
    assert vp.history.undo() and desk.layer is None
    assert vp.history.undo() and desk in vp.scene.groups and room.children == [wall]

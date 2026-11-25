# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Purge unused (layers / materials) — the sweep an import makes necessary.

Marco imported a surveyed plaza as a reference, kept one flat drawing and
deleted the rest; the document still carried the big drawing's 13 layers and
47 materials. Layers are labels, not owners, so the delete could not take
them — an explicit sweep must, and it must be undoable.
"""
from __future__ import annotations

from built_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401
from built_core.group import Group
from built_core.history import History, PurgeUnusedCommand
from built_core.layers import DEFAULT_LAYER, Layer, assign_layer
from built_core.materials import Material
from built_core.mesh import Mesh
from built_core.purge import (unused_layers, unused_materials, used_layers,
                        used_materials)
from built_core.saved_views import SavedView
from built_core.scene import Scene


def V(x, y, z=0.0):
    return QVector3D(x, y, z)


def _quad(mesh, x0=0.0):
    return mesh.add_face([V(x0, 0), V(x0 + 2, 0), V(x0 + 2, 2), V(x0, 2)])


def test_empty_layer_is_unused():
    scene = Scene()
    _quad(scene.mesh)
    scene.layers.append(Layer("CVL_PUNTO"))
    assert [ly.name for ly in unused_layers(scene)] == ["CVL_PUNTO"]


def test_layer_worn_by_a_face_survives():
    scene = Scene()
    f = _quad(scene.mesh)
    scene.layers.append(Layer("VEREDA"))
    assign_layer(f, "VEREDA")
    assert "VEREDA" in used_layers(scene)
    assert unused_layers(scene) == []


def test_the_default_layer_is_never_swept():
    scene = Scene()
    assert scene.layers[0].name == DEFAULT_LAYER
    assert unused_layers(scene) == []


def test_a_layer_used_only_inside_a_group_survives():
    """The flat-list walk deleted this one out from under the group."""
    scene = Scene()
    inner = Mesh()
    f = _quad(inner)
    scene.groups.append(Group(inner, name="Banca"))
    scene.layers.append(Layer("Topografía"))
    assign_layer(f, "Topografía")
    assert unused_layers(scene) == []


def test_a_layer_used_only_inside_a_NESTED_placement_survives():
    scene = Scene()
    deep = Mesh()
    f = _quad(deep)
    child = Group(deep, name="Silla")
    child.xform = QMatrix4x4()
    parent = Group(Mesh(), name="Mesa")
    parent.adopt([child])
    scene.groups.append(parent)
    scene.layers.append(Layer("Muebles"))
    assign_layer(f, "Muebles")
    assert "Muebles" in used_layers(scene)
    assert unused_layers(scene) == []


def test_a_layer_a_scene_hides_is_kept():
    scene = Scene()
    scene.layers.append(Layer("Anotaciones"))
    scene.saved_views.append(SavedView("Planta", hidden_layers=["Anotaciones"]))
    assert unused_layers(scene) == []


def test_purge_removes_and_undo_restores_at_the_same_index():
    scene = Scene()
    _quad(scene.mesh)
    for name in ("BUZON", "CANAL", "limite"):
        scene.layers.append(Layer(name))
    history = History(scene)
    history.execute(PurgeUnusedCommand(layers=True, materials=False))
    assert [ly.name for ly in scene.layers] == [DEFAULT_LAYER]
    history.undo()
    assert [ly.name for ly in scene.layers] == [
        DEFAULT_LAYER, "BUZON", "CANAL", "limite"]
    history.redo()
    assert [ly.name for ly in scene.layers] == [DEFAULT_LAYER]


def test_redo_removes_what_the_first_run_chose_not_a_fresh_survey():
    """A layer created after the purge must not vanish on redo."""
    scene = Scene()
    _quad(scene.mesh)
    scene.layers.append(Layer("CVL_CURV_D"))
    history = History(scene)
    history.execute(PurgeUnusedCommand(layers=True, materials=False))
    history.undo()
    scene.layers.append(Layer("Recien creada"))
    history.redo()
    assert [ly.name for ly in scene.layers] == [DEFAULT_LAYER, "Recien creada"]


def test_unused_materials_are_the_ones_no_face_wears():
    scene = Scene()
    f = _quad(scene.mesh)
    scene.materials["Concreto"] = Material("Concreto", color=(0.5, 0.5, 0.5))
    scene.materials["Celtis_australis"] = Material("Celtis_australis",
                                                   color=(0.1, 0.4, 0.1))
    f.attrs["mat"] = "Concreto"
    assert used_materials(scene) == {"Concreto"}
    assert unused_materials(scene) == ["Celtis_australis"]


def test_a_material_worn_inside_a_group_survives():
    scene = Scene()
    inner = Mesh()
    f = _quad(inner)
    scene.groups.append(Group(inner, name="Pileta"))
    scene.materials["Marble_07_1K"] = Material("Marble_07_1K",
                                               color=(0.9, 0.9, 0.9))
    f.attrs["mat"] = "Marble_07_1K"
    assert unused_materials(scene) == []


def test_a_material_only_a_GROUP_wears_survives():
    """A group's own paint lives on the group, not on its faces: Purge used
    to delete a material that only a group wore (issue #133) — nested at
    any depth too."""
    scene = Scene()
    inner = Mesh()
    _quad(inner)
    box = Group(inner, name="Caja")
    box.material = {"color": [0.8, 0.2, 0.2], "mat": "Rojo"}
    child = Group(Mesh(), name="Hijo")
    _quad(child.mesh, 3.0)
    child.material = {"color": [0.2, 0.2, 0.8], "mat": "Azul"}
    outer = Group(Mesh(), name="Fila")
    outer.adopt([child])
    scene.groups.extend([box, outer])
    for name, rgb in (("Rojo", (0.8, 0.2, 0.2)), ("Azul", (0.2, 0.2, 0.8)),
                      ("Suelto", (0.5, 0.5, 0.5))):
        scene.materials[name] = Material(name, color=rgb)
    assert used_materials(scene) == {"Rojo", "Azul"}
    assert unused_materials(scene) == ["Suelto"]


def test_purging_materials_is_undoable_with_the_recipe_intact():
    scene = Scene()
    _quad(scene.mesh)
    mat = Material("Brick_Soldier_Bond_1K", color=(0.7, 0.3, 0.2))
    scene.materials["Brick_Soldier_Bond_1K"] = mat
    history = History(scene)
    history.execute(PurgeUnusedCommand(layers=False, materials=True))
    assert scene.materials == {}
    history.undo()
    assert scene.materials["Brick_Soldier_Bond_1K"] is mat


def test_flags_scope_the_sweep():
    scene = Scene()
    _quad(scene.mesh)
    scene.layers.append(Layer("Trees"))
    scene.materials["grass"] = Material("grass", color=(0.2, 0.6, 0.2))
    history = History(scene)
    history.execute(PurgeUnusedCommand(layers=True, materials=False))
    assert [ly.name for ly in scene.layers] == [DEFAULT_LAYER]
    assert "grass" in scene.materials


def test_deleting_a_layer_reassigns_faces_INSIDE_groups(native_question_host):
    from built_core.layers import layer_of
    from tests.native_tree import action
    from built_render import chrome_dialogs
    scene = native_question_host.viewport.scene
    inner = Mesh()
    f = _quad(inner)
    deep = Mesh()
    g = _quad(deep)
    child = Group(deep, name="Silla")
    child.xform = QMatrix4x4()
    parent = Group(inner, name="Mesa")
    parent.adopt([child])
    scene.groups.append(parent)
    scene.layers.append(Layer("CANAL"))
    assign_layer(f, "CANAL")
    assign_layer(g, "CANAL")
    action(native_question_host, "deletetag", "preview-tag:CANAL")
    assert chrome_dialogs.answer(native_question_host.window, "delete")
    assert scene.layer("CANAL") is None
    assert layer_of(f) == layer_of(g) == DEFAULT_LAYER


def test_the_drawing_purge_removes_only_unused_tags_and_undo_restores_order(native_question_host):
    from tests.native_tree import action
    from built_render import chrome_dialogs
    host = native_question_host
    scene = host.viewport.scene
    names = [DEFAULT_LAYER, "CVL_PUNTO", "BUZON", "Trees"]
    scene.layers.extend(Layer(name) for name in names[1:])
    scene.materials["Grass"] = Material("Grass")
    action(host, "purgetags", "")
    sheet = chrome_dialogs.current_sheet(host.window)
    assert "3" in sheet.text and "CVL_PUNTO, BUZON, Trees" in sheet.text
    assert chrome_dialogs.answer(host.window, "delete")
    assert [tag.name for tag in scene.layers] == [DEFAULT_LAYER] and "Grass" in scene.materials
    assert host.viewport.history.undo()
    assert [tag.name for tag in scene.layers] == names

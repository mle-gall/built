# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Renaming and recolouring a material of the model.

A colour picked from the library entered the model under its library
name and kept it for good: there was no way to rename it, and editing its
colour left «RAL 7035» on a colour that is not RAL 7035. Now the model's
material can be renamed (on every face and group that wears it, one
undo), and a library colour edited in the model becomes a new material —
the library file itself never changes."""
from __future__ import annotations


import pytest
from ingetrazo_core import Vec3 as QVector3D


from core.group import Group                                      # noqa: E402
from core.history import (History, RestampMaterialCommand,        # noqa: E402
                          NativeCommand)
from core.materials import Material, free_name                    # noqa: E402
from core.mesh import Mesh                                        # noqa: E402
from core.scene import Scene                                      # noqa: E402
from tools.paint import PaintTool, rename_recent                  # noqa: E402


def _quad(mesh, z=0.0):
    return mesh.add_face([QVector3D(0, 0, z), QVector3D(4, 0, z),
                          QVector3D(4, 4, z), QVector3D(0, 4, z)])


def _painted_scene():
    """«Azul» on a loose face, on a face inside a nested component and on
    a group painted whole; «Rojo» beside it."""
    scene = Scene()
    history = History(scene)
    azul = Material("Azul", color=(0.1, 0.4, 0.9), finish="matte")
    rojo = Material("Rojo", color=(0.9, 0.1, 0.1))
    scene.materials.update({"Azul": azul, "Rojo": rojo})
    loose = _quad(scene.mesh)
    loose.attrs.update(azul.face_attrs())
    red = _quad(scene.mesh, z=1.0)
    red.attrs.update(rojo.face_attrs())
    outer = Group(Mesh(), name="Caja")
    inner = Group(Mesh(), name="Tapa")
    deep = _quad(inner.mesh, z=5.0)
    deep.attrs.update(azul.face_attrs())
    outer.adopt([inner])
    scene.groups.append(outer)
    whole = Group(Mesh(), name="Bloque")
    _quad(whole.mesh, z=7.0)
    scene.groups.append(whole)
    history.execute(NativeCommand.paint([whole], azul.face_attrs()))
    return scene, history, loose, red, deep, whole


def test_renaming_restamps_every_face_and_group_and_undoes():
    scene, history, loose, red, deep, whole = _painted_scene()
    azul = scene.materials["Azul"]
    history.execute(RestampMaterialCommand("Azul", Material(
        "Fachada norte", color=azul.color, finish=azul.finish)))

    assert "Azul" not in scene.materials
    assert scene.materials["Fachada norte"].finish == "matte"
    for attrs in (loose.attrs, deep.attrs, whole.material):
        assert attrs["mat"] == "Fachada norte"
        assert tuple(attrs["color"]) == (0.1, 0.4, 0.9)
    assert red.attrs["mat"] == "Rojo"                 # untouched

    history.undo()
    assert scene.materials["Azul"] is azul
    assert "Fachada norte" not in scene.materials
    for attrs in (loose.attrs, deep.attrs, whole.material):
        assert attrs["mat"] == "Azul"

    history.redo()
    assert set(scene.materials) == {"Fachada norte", "Rojo"}
    assert loose.attrs["mat"] == "Fachada norte"


def test_undo_puts_back_an_entry_the_rename_landed_on():
    """The command itself does not refuse a taken name (the panel does);
    if it ever lands on one, undo must give that entry back intact."""
    scene, history, *_ = _painted_scene()
    rojo = scene.materials["Rojo"]
    history.execute(RestampMaterialCommand(
        "Azul", Material("Rojo", color=(0.5, 0.5, 0.5))))
    history.undo()
    assert scene.materials["Rojo"] is rojo
    assert "Azul" in scene.materials


def test_free_name():
    assert free_name("Muro", {}) == "Muro"
    assert free_name("Muro", {"Muro": 1}) == "Muro (2)"
    assert free_name("Muro", {"Muro", "Muro (2)"}) == "Muro (3)"


def test_recents_follow_the_rename():
    entries = [{"mat": "Azul", "color": [0.1, 0.4, 0.9]},
               {"mat": "Rojo", "color": [0.9, 0.1, 0.1]},
               {"color": [0.3, 0.3, 0.3]}]
    out = rename_recent(entries, "Azul", Material(
        "Fachada", color=(0.2, 0.2, 0.2)).face_attrs())
    assert out[0] == {"mat": "Fachada", "color": [0.2, 0.2, 0.2]}
    assert out[1:] == entries[1:]


# ---- The panel -------------------------------------------------------------

from tests.shell_isolation import native_window as window, restore_shell_patches  # noqa: F401
from tests.test_library_paint_selection import _model_paint


def test_naming_a_swatch_not_yet_in_the_model_names_the_next_paint(window):
    from ingetrazo_render import wgpu_library as library
    from tests.test_tree_links import _View
    name = "RAL 6011 Reseda green"
    library._pick_paint(window, recipe={"name": name, "color": [0.3, 0.6, 0.3]})
    library.edit_field(window, _View(), "brush_name:" + name, "Zócalo")
    assert PaintTool.current_material.name == "Zócalo"
    assert PaintTool.current_color == (0.3, 0.6, 0.3)
    assert "Zócalo" not in window.viewport.scene.materials   # until painted


def test_recolouring_a_library_colour_makes_a_new_material(window,
                                                          monkeypatch):
    """«RAL 7035» in another colour would be a lie: the faces take a new
    material «… (modified)», the library file stays byte for byte."""
    from ingetrazo_core.paths import app_root
    from ingetrazo_core import ral_rgb
    from tests.test_tree_links import _View
    from ingetrazo_render import wgpu_library as library
    palette = app_root() / "resources" / "colors" / "ral.json"
    ral = palette.read_bytes()
    entry = next(c for fam in __import__("json").loads(ral)["families"]
                 for c in fam["colors"] if c["code"] == "RAL 7035")
    name = "%s %s" % (entry["code"], entry["name"])
    face = _model_paint(window, name, tuple(entry["rgb"]))
    library.edit_field(window, _View(), "color:" + name, "#33cc33")
    new = face.attrs["mat"]
    assert new != name and new.startswith(name)
    assert tuple(face.attrs["color"]) == (0.2, 0.8, 0.2)
    scene = window.viewport.scene
    assert name not in scene.materials and new in scene.materials
    assert palette.read_bytes() == ral
    assert ral_rgb(name) == pytest.approx(tuple(entry["rgb"]))
    # A colour of the user's own keeps its name when recoloured.
    own = _model_paint(window, "Muro", (0.5, 0.5, 0.5))
    library.edit_field(window, _View(), "color:Muro", "#999999")
    assert own.attrs["mat"] == "Muro"


def test_rename_from_the_in_model_menu(window):
    from ingetrazo_render import wgpu_library as library
    face = _model_paint(window, "Azul", (.1, .4, .9))
    library.provider("materials").run(library.Ctx(window, window.view), "rename",
                                      {"key": "m:Azul", "name": "Azul"}, "Techo")
    assert face.attrs["mat"] == "Techo"

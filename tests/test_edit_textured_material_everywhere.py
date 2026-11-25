# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Editing a textured material changes it wherever it is worn.

A YouTube review of 0.6.1 (#1308): «when you edit an already applied
material, say change the colour, it does not affect the applied materials
in the model, almost as if it creates another material definition». The
Materials panel's Apply changed the texture to paint with next and the
faces SELECTED: with nothing selected the model kept the old look, with
one face selected the same name came out in two colours. Here two faces
and a component are painted with the Paint tool's clicks, the material is
picked again from «In model», given a colour and applied: everything that
wears it changes, each face keeps its own placement, one undo puts it all
back."""
from __future__ import annotations

import pytest
from tests.test_extension_previews import png
from built_core import Vec3 as V
from core.group import Group
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_paint_hover_container import _ctx
from tools.paint import PaintTool
from built_render import wgpu_library as library


@pytest.fixture
def scene(native_host, tmp_path, monkeypatch):
    win, vp = native_host.window, native_host.viewport
    sc = vp.scene
    def face(mesh, x):
        return mesh.add_face([V(x, 0, 0), V(x+2, 0, 0), V(x+2, 2, 0), V(x, 2, 0)])
    f1, f2 = face(sc.mesh, 0), face(sc.mesh, 4)
    comp = Group(name="Tile")
    inside = face(comp.mesh, 8)
    sc.groups.append(comp)
    sc.version += 1
    image = tmp_path / "brick.png"
    image.write_bytes(png((180, 80, 60)))
    library._pick_paint(win, recipe={"name": "Brick", "texture": {"path": str(image), "sw": 1., "sh": 1.}})
    for picked, owner in ((f1, None), (f2, None), (inside, comp)):
        monkeypatch.setattr(vp, "pick_face_any", lambda *_: (picked, owner))
        monkeypatch.setattr(vp, "pick_group", lambda *_: owner)
        PaintTool().on_click(_ctx(vp))
    return win, sc, (f1, f2, comp)


def _tex(worn):
    """The texture a face wears, or a component painted whole."""
    attrs = getattr(worn, "attrs", None)
    return ((attrs if attrs is not None else worn.material or {})
            .get("texture") or {})


def _tint(worn):
    t = _tex(worn).get("tint")
    return None if t is None else tuple(round(c, 3) for c in t)


BLUE_F = tuple(round(c, 3) for c in (40/255, 120/255, 220/255))


@pytest.mark.parametrize("selected", ["none", "one"])
def test_a_colour_given_to_a_textured_material_reaches_every_face(
        scene, selected):
    win, sc, faces = scene
    f1, f2, comp = faces
    assert f1.attrs.get("mat") == f2.attrs.get("mat") == "Brick"
    assert comp.material.get("mat") == "Brick"     # painted whole
    assert all(_tint(f) is None for f in faces)
    sc.selection.clear()
    if selected == "one":
        sc.select([faces[0]])
    library.edit_field(win, win.view, "color:Brick", "#2878dc")
    assert [_tint(f) for f in faces] == [BLUE_F] * 3
    reg = sc.materials["Brick"].texture
    assert tuple(round(c, 3) for c in reg["tint"]) == BLUE_F
    # One step back: the model as it was, the registry too.
    win.viewport.history.undo()
    assert [_tint(f) for f in faces] == [None] * 3
    assert not sc.materials["Brick"].texture.get("tint")


def test_each_face_keeps_its_own_placement_and_size(scene):
    win, sc, (f1, f2, _comp) = scene
    # A texture placed by hand on one face, another tile size on the next.
    f1.attrs["texture"] = {**f1.attrs["texture"], "uvw": [1, 2, 3, 4]}
    f2.attrs["texture"] = {**f2.attrs["texture"], "sw": 0.25, "sh": 0.5}
    sc.selection.clear()
    library.edit_field(win, win.view, "color:Brick", "#2878dc")
    assert f1.attrs["texture"]["uvw"] == [1, 2, 3, 4]
    assert (f2.attrs["texture"]["sw"], f2.attrs["texture"]["sh"]) == (0.25, 0.5)
    assert _tint(f1) == _tint(f2) == BLUE_F


def test_a_size_changed_in_the_panel_reaches_every_face(scene):
    win, sc, faces = scene
    sc.selection.clear()
    library.edit_field(win, win.view, "tile:Brick", "2.5")
    assert [_tex(f)["sw"] for f in faces] == [2.5] * 3
    assert sc.materials["Brick"].texture["sw"] == 2.5


def test_picking_a_tinted_swatch_again_and_resizing_keeps_its_colour(scene):
    win, sc, faces = scene
    sc.selection.clear()
    library.edit_field(win, win.view, "color:Brick", "#2878dc")
    # Picked again from «In model», as its swatch does: the image it shows.
    library.paint_from_strip(win, "m:Brick")
    library.edit_field(win, win.view, "tile:Brick", "3.0")
    assert [_tint(f) for f in faces] == [BLUE_F] * 3
    assert [_tex(f)["sw"] for f in faces] == [3.0] * 3

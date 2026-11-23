# SPDX-License-Identifier: GPL-3.0-or-later
"""Picking paint from the native Library and editing it in the inspector."""
from types import SimpleNamespace

import pytest

from core.group import Group
from core.history import History, CompoundCommand, SetFaceMaterialTagCommand, SetGroupMaterialCommand
from core.materials import Material
from core.scene import Scene
from tools.paint import PaintTool
from views import wgpu_library as library
from views import wgpu_ui


@pytest.fixture
def win(monkeypatch):
    for field in ("current_color", "current_opacity", "current_texture", "current_texture_plane",
                  "current_material", "current_is_default"):
        monkeypatch.setattr(PaintTool, field, getattr(PaintTool, field))
    scene = Scene()
    vp = SimpleNamespace(scene=scene, history=History(scene))
    selected = []
    return SimpleNamespace(viewport=vp, _activate_tool=selected.append, tools_selected=selected)


def test_strip_picks_full_registered_recipe_without_a_materials_panel(win):
    tex = {"path": "synthetic.png", "sw": 1.2, "sh": 0.4, "rot": 21.}
    mat = Material("Stained glass", texture=tex, color=(.1, .2, .3), opacity=.4, pbr={"roughness":.2})
    win.viewport.scene.materials[mat.name] = mat
    PaintTool.current_texture_plane = "previous plane"
    library.paint_from_strip(win, "m:Stained glass")
    assert win.tools_selected == ["paint"]
    assert PaintTool.current_material is mat
    assert PaintTool.current_texture_plane is None and PaintTool.current_texture == tex
    assert PaintTool.current_color == mat.color and PaintTool.current_opacity == .4
    assert PaintTool.current_material.pbr == mat.pbr
    assert not win.viewport.history.undo_stack


def test_bundled_image_and_ral_keep_their_recipes_until_the_first_stroke(win):
    wgpu_ui._paint_with(win, "lib:brick:0")
    assert PaintTool.current_texture["sw"] == 1
    assert PaintTool.current_texture["sh"] == .6
    assert PaintTool.current_texture["path"].endswith("brick/brick_red.png")
    wgpu_ui._paint_with(win, "ral:RAL 1000")
    assert PaintTool.current_material.name.startswith("RAL 1000 ")
    assert not PaintTool.current_texture
    assert PaintTool.current_color == pytest.approx((.8039,.7294,.5333))
    assert not win.viewport.scene.materials


def test_the_color_row_recolours_a_plain_colour_material(win):
    from tests.test_paint_materials import _quad

    vp = win.viewport
    g = Group(name="Caja")
    _quad(g.mesh, z=5)
    vp.scene.groups.append(g)
    face = _quad(vp.scene.mesh)
    library._pick_paint(win, recipe={"name":"Azul", "color":[.1,.4,.9]})
    paint = PaintTool._current_as_material()
    face.attrs.update(paint)
    mat = PaintTool.current_material
    vp.history.execute(CompoundCommand([
        SetFaceMaterialTagCommand([], mat.name, mat), SetGroupMaterialCommand(g, paint)]))
    library.provider("materials").edit(library.Ctx(win, None), {"name":"Azul","key":"m:Azul"},
                                       "color", "#00ff00")
    assert tuple(face.attrs["color"]) == (0,1,0) and tuple(g.material["color"]) == (0,1,0)
    assert vp.history.undo()
    for attrs in (g.material, face.attrs):
        assert tuple(attrs["color"]) == pytest.approx((.1,.4,.9))

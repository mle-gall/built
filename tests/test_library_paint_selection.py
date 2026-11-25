# SPDX-License-Identifier: GPL-3.0-or-later
"""Picking paint from the native Library and editing it in the inspector."""
from types import SimpleNamespace

import pytest

from core.group import Group
from core.history import History, CompoundCommand, SetFaceMaterialTagCommand, NativeCommand
from core.materials import Material
from core.scene import Scene
from tools.paint import PaintTool
from built_render import wgpu_library as library
from views import wgpu_ui


@pytest.fixture
def win(monkeypatch):
    monkeypatch.setattr(PaintTool, "_library_pick", getattr(PaintTool, "_library_pick", None), raising=False)
    for field in ("current_color", "current_opacity", "current_texture", "current_texture_plane",
                  "current_material", "current_is_default"):
        monkeypatch.setattr(PaintTool, field, getattr(PaintTool, field))
    scene = Scene()
    vp = SimpleNamespace(scene=scene, history=History(scene), flash_status=lambda *_: None,
                         refresh_after_edit=lambda: None)
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
        SetFaceMaterialTagCommand([], mat.name, mat), NativeCommand.paint([g], paint)]))
    library.provider("materials").edit(library.Ctx(win, None), {"name":"Azul","key":"m:Azul"},
                                       "color", "#00ff00")
    assert tuple(face.attrs["color"]) == (0,1,0) and tuple(g.material["color"]) == (0,1,0)
    assert vp.history.undo()
    for attrs in (g.material, face.attrs):
        assert tuple(attrs["color"]) == pytest.approx((.1,.4,.9))


def _model_paint(win, name, color):
    from tests.test_paint_materials import _quad
    scene = win.viewport.scene
    mat = Material(name, color=color)
    scene.materials[name] = mat
    face = _quad(scene.mesh, len(scene.mesh.faces))
    face.attrs.update(mat.face_attrs())
    library.paint_from_strip(win, "m:" + name)
    return face


def test_the_name_field_renames_the_active_material(win):
    from tests.test_tree_links import _View
    view = _View()
    face = _model_paint(win, "Azul", (.1,.4,.9))
    view._ui_mat_focus = "m:Azul"
    wgpu_ui._tree_action(win, view, wgpu_ui.SEP.join(("edit", "libmat:name:Azul", "Fachada norte")))
    assert face.attrs["mat"] == "Fachada norte" and PaintTool.current_material.name == "Fachada norte"
    assert "Azul" not in win.viewport.scene.materials
    assert library.inspector(view, win)[0] == "Fachada norte"
    assert win.viewport.history.undo()
    assert face.attrs["mat"] == "Azul"


def test_the_name_field_refuses_a_name_already_taken(win):
    from tests.test_tree_links import _View
    view = _View()
    _model_paint(win, "Rojo", (.9,.1,.1))
    face = _model_paint(win, "Azul", (.1,.4,.9))
    view._ui_mat_focus = "m:Azul"
    before = win.viewport.history.revision
    wgpu_ui._tree_action(win, view, wgpu_ui.SEP.join(("edit", "libmat:name:Azul", "Rojo")))
    assert face.attrs["mat"] == "Azul" and library.inspector(view, win)[0] == "Azul"
    assert win.viewport.scene.materials["Rojo"].color == (.9,.1,.1)
    assert win.viewport.history.revision == before


def test_next_stroke_uses_the_current_revision_of_a_library_pick(win):
    from tests.test_paint_materials import _FakeViewport, _quad, _click
    from tests.test_tree_links import _View
    win.viewport = vp = _FakeViewport()
    view = _View()
    face = _quad(vp.scene.mesh, 4)
    _model_paint(win, "Azul", (.1,.4,.9))
    library.edit_field(win, view, "opacity:Azul", "62")
    assert PaintTool.current_opacity == pytest.approx(.62)
    _click(vp, face)
    assert face.attrs["opacity"] == pytest.approx(.62)
    assert vp.history.undo() and vp.history.undo()
    face = next(f for f in vp.scene.mesh.faces if f.key == face.key)
    _click(vp, face)
    assert face.attrs.get("opacity", 1.) == 1.

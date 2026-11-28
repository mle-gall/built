# SPDX-License-Identifier: GPL-3.0-or-later
"""View templates and title blocks in the Library (docs/library.md): a
view's setup (camera kind, style, hidden tags, section planes) saved as a
view template, a sheet's title block saved as a Library item, both with
the same destination choice as every kind, both applied from the Library."""
from __future__ import annotations

import json
from types import SimpleNamespace

import pytest
from built_core import Vec3 as V
from built_core.layers import Layer
from built_core.section import SectionPlane
from built_core.style import Style

from built_render import chrome_forms
from built_render import wgpu_library as L
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_library_destinations import act, frame, in_project, shared, tiles, win  # noqa: F401
from tests.test_sheets_workspace import _run_actions, _sheets, host  # noqa: F401


def dress(win):
    """A view worth keeping: parallel, its own style, a tag hidden, a cut."""
    vp = win.viewport
    scene = vp.scene
    furniture = Layer("Furniture")
    furniture.visible = False
    scene.layers.extend([furniture, Layer("Structure")])
    vp.camera.perspective = False
    style = Style.from_dict(dict(scene.display_style.to_dict(), name="Working drawing", edges=True, face_mode="shaded"))
    win.set_active_style(style)
    plane = SectionPlane(V(0, 0, 1.2), V(0, 0, -1))
    plane.name = "Plan cut"
    scene.section_planes.append(plane)
    scene.set_active_section(plane)
    scene.show_section_planes = False
    return plane


def undress(win):
    """Back to an ordinary view of the same model."""
    vp = win.viewport
    scene = vp.scene
    for ly in scene.layers:
        ly.visible = True
    vp.camera.perspective = True
    win.set_active_style(Style.from_dict(dict(scene.display_style.to_dict(), name="Default", face_mode="textures")))
    scene.set_active_section(None)
    scene.show_section_planes = True


def test_a_view_template_holds_the_camera_kind_style_tags_and_section(win):
    plane = dress(win)
    data = L.view_template(win)
    assert data["format"] == "built.view_template" and data["v"] == 1
    assert data["camera"]["perspective"] is False and data["camera"]["two_point"] is False
    assert data["style"]["name"] == "Working drawing" and data["style"]["face_mode"] == "shaded"
    assert [t["name"] for t in data["hidden_tags"]] == ["Furniture"] and data["hidden_tags"][0]["key"]
    assert data["section"]["active"]["uid"] == plane.uid and data["section"]["active"]["name"] == "Plan cut"
    assert data["section"]["planes_shown"] is False and data["section"]["cuts_shown"] is True
    # Never where the camera stands: a template is a setup, not a scene.
    assert not {"target", "distance", "yaw", "pitch"} & set(data["camera"])


def test_save_view_as_template_takes_the_same_destinations_and_applies_from_the_library(win, tmp_path, monkeypatch):
    in_project(win, tmp_path)
    shared(tmp_path, "Team")
    shown = []
    monkeypatch.setattr(chrome_forms, "show", lambda parent, form: shown.append(form))
    dress(win)
    camera = win.viewport.camera
    stand = (camera.distance, camera.yaw, camera.pitch)
    L.save_view_template(win)
    form, = shown
    assert form.placement == "drawer" and form.title == "Save view as template"
    assert [o[0] for o in form.field("destination").options] == ["model", "project", "mine", "shared:Team"]
    assert form.value("name") == "Working drawing view"
    form.on_done("ok", dict(form.values(), name="Plan view", description="1:50 plans", tags="plans", destination="project"))
    row, = L.ViewTemplatesProvider().items(SimpleNamespace(scene=win.viewport.scene))
    assert (row["source"], row["name"]) == ("project", "Plan view")
    facts = {r[0]: r[1] for r in row["defaults"]}
    assert facts["Camera"] == "Parallel" and facts["Style"] == "Working drawing"
    assert facts["Visible tags"] == "Hidden: Furniture" and facts["Sections"] == "Cut by Plan cut · planes hidden"
    assert (tmp_path / "Library" / "scenes").is_dir()
    # Its tile is drawn from the setup itself (a box as the camera shows one, cut across).
    assert row["thumb"].startswith("sketch:")
    # The Library's own tab, with its header button and the apply verb.
    undress(win)
    scene = win.viewport.scene
    L.open_library(win, "scenes")
    sh = frame(win)
    assert "View templates" in json.dumps(sh)
    assert [(t["label"], t["place"]) for t in tiles(sh)] == [("Plan view", "This project")]
    notes = []
    monkeypatch.setattr(win, "notify", lambda tone, text, *a: notes.append((tone, text)))
    act(win, "open", tiles(sh)[0]["id"])
    assert notes[-1] == ("success", "Plan view applied to this view")
    assert camera.perspective is False and (camera.distance, camera.yaw, camera.pitch) == stand
    assert scene.layer("Furniture").visible is False and scene.layer("Structure").visible is True
    assert win.active_style().name == "Working drawing"
    assert scene.active_section().name == "Plan cut" and scene.show_section_planes is False


def test_a_template_applies_in_another_model_by_tag_name_and_brings_its_section_plane(win, monkeypatch):
    from built_core.scene import Scene
    dress(win)
    data = L.view_template(win)
    # Another model: its Furniture tag has another key, it has no Structure
    # tag and no section plane; a tag the template hides is missing.
    data["hidden_tags"].append({"key": "feedfacefeedface", "name": "Scaffolding"})
    win.new_document()
    scene = win.viewport.scene
    assert isinstance(scene, Scene) and not scene.section_planes
    scene.layers.extend([Layer("Furniture"), Layer("Walls")])
    steps = len(win.viewport.history.undo_stack)
    missing = L.apply_view_template(win, json.dumps(data))
    assert missing == "1 tag of the template is not in this model"
    assert scene.layer("Furniture").visible is False and scene.layer("Walls").visible is True
    assert win.viewport.camera.perspective is False
    plane, = scene.section_planes
    assert plane.name == "Plan cut" and scene.active_section() is plane
    assert tuple(round(c, 6) for c in (plane.point.x(), plane.point.y(), plane.point.z())) == (0, 0, 1.2)
    # The plane it brought is one step of this model's history.
    assert len(win.viewport.history.undo_stack) == steps + 1
    # Applied again: the same plane, not a second one.
    assert L.apply_view_template(win, json.dumps(data)) == missing
    assert len(scene.section_planes) == 1
    with pytest.raises(ValueError, match="Not a view template"):
        L.apply_view_template(win, json.dumps({"camera": {}}))


def test_a_title_block_saves_to_the_library_and_lands_on_another_sheet(host, tmp_path, monkeypatch):
    ws = _sheets(host)
    win = host.window
    comp = ws.composer.comp
    block = comp.cajetin
    assert block is not None
    block.campos = [["PROJECT", "Garage"], ["SHEET", "A-01"], ["SCALE", "1:50"]]
    block.columns = 2
    block.w_mm, block.h_mm = 150.0, 28.0
    data = L.title_block(win)
    assert data["format"] == "built.title_block"
    fields = data["fields"]
    assert fields["campos"][0] == ["PROJECT", "Garage"] and fields["columns"] == 2
    assert not {"x_mm", "y_mm", "z", "locked", "uid"} & set(fields), "never its place on this sheet"
    # Right-click the title block › Save to Library…: the shared drawer.
    shown = []
    monkeypatch.setattr(chrome_forms, "show", lambda parent, form: shown.append(form))
    _run_actions(host, "sheets:save_titleblock")
    form, = shown
    assert form.placement == "drawer" and form.value("name") == "Garage · title block"
    assert [o[0] for o in form.field("destination").options][:2] == ["model", "mine"]
    form.on_done("ok", dict(form.values(), name="Office block", destination="mine"))
    row, = L.TitleBlocksProvider().items(SimpleNamespace(scene=win.viewport.scene))
    assert (row["source"], row["name"]) == ("mine", "Office block")
    facts = {r[0]: r[1] for r in row["defaults"]}
    assert facts["Size"] == "150 × 28 mm" and facts["Rows"] == "3 · two columns"
    assert facts["Fields"] == "PROJECT, SHEET, SCALE"
    # Its tile is drawn from its rows and columns; another layout, another picture.
    assert row["thumb"].startswith("sketch:")
    L.save_item(win, "title_blocks", "mine", "One column", json.dumps(dict(data, fields=dict(fields, columns=1))))
    one = next(r for r in L.TitleBlocksProvider().items(SimpleNamespace(scene=win.viewport.scene)) if r["name"] == "One column")
    assert one["thumb"].startswith("sketch:") and one["thumb"] != row["thumb"]
    # A second sheet with its own, default-sized block in the page's corner.
    _run_actions(host, "sheets:add")
    other = ws.composer.comp
    assert other is not comp
    target = other.cajetin
    target.campos = [["PROJECT", "?"]]
    target.columns = 1
    target.w_mm, target.h_mm = 180.0, 33.0
    corner = (target.x_mm + target.w_mm, target.y_mm + target.h_mm)
    history = win.viewport.history
    steps = len(history.undo_stack)
    assert L.apply_title_block(win, json.dumps(data)) is True
    assert len(history.undo_stack) == steps + 1
    assert target.campos[1] == ["SHEET", "A-01"] and target.columns == 2
    assert (target.w_mm, target.h_mm) == (150.0, 28.0)
    # Still docked in its corner, whatever its new size.
    assert (target.x_mm + target.w_mm, target.y_mm + target.h_mm) == pytest.approx(corner)
    ws._undo()
    assert target.campos == [["PROJECT", "?"]] and target.w_mm == 180.0
    # A sheet without a title block gets one.
    other.cajetin = None
    assert L.title_block(win) is None
    assert L.apply_title_block(win, json.dumps(data)) is True
    assert other.cajetin is not None and other.cajetin.campos[0] == ["PROJECT", "Garage"]
    assert host.errors == []


def test_title_blocks_wait_for_a_sheet_and_say_so(win, monkeypatch):
    notes = []
    monkeypatch.setattr(win, "notify", lambda tone, text, *a: notes.append((tone, text)))
    # No sheet in front: nothing to save, nothing to put a block on.
    assert L.save_title_block(win) is False
    assert notes[-1] == ("info", "Open a sheet with a title block to save it to the Library")
    block = {"format": "built.title_block", "v": 1, "fields": {"campos": [["PROJECT", "Garage"]], "w_mm": 150, "h_mm": 28}}
    L.save_item(win, "title_blocks", "mine", "Office block", json.dumps(block))
    assert L.apply_title_block(win, json.dumps(block)) is False
    L.open_library(win, "title_blocks")
    sh = frame(win)
    assert [(t["label"], t["place"]) for t in tiles(sh)] == [("Office block", "My library")]
    act(win, "open", tiles(sh)[0]["id"])
    assert notes[-1] == ("info", "Open a sheet first: a title block goes on the sheet in front")
    with pytest.raises(ValueError, match="Not a title block"):
        L.apply_title_block(win, json.dumps({"fields": []}))

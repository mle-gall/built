# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Outliner (#219): the model's groups and components as a tree.

Every model here is generated: boxes, nested containers, component copies
and a seeded random tree of ten thousand nodes for the timing."""
from __future__ import annotations

import math
import time

import pytest

from built_core import Vec3 as QVector3D

from core import outliner
from core.group import Group
from core.mesh import Mesh
from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401
from tests.test_project_occurrences import _building, _group, _random_tree


def _filing_project(*roots):
    from tests.test_project_drawing_filters import Project
    project = Project()
    project.scene.groups[:] = roots
    project.scene.version += 1
    project.view._ui_exp = {"n:"}
    return project


def test_rows_update_in_place_and_rebuild_only_when_groups_change():
    building, tree = _building()
    p = _filing_project(building, tree)
    p.view._ui_exp.add("i:" + building.uid)
    key = "i:" + tree.uid
    assert p.rows()[key][0] == "Tree"
    tree.name = "Pine"
    tree.hidden = True
    p.scene.version += 1
    rows = p.rows()
    assert rows[key][0] == "Pine" and rows[key][1][5] == 0
    from core.history import InsertGroupCommand
    extra = _group("Fence", (9, 0, 0))
    p.win.viewport.history.execute(InsertGroupCommand(extra))
    rows = p.rows()
    assert rows["i:" + building.uid][1][8] == 1
    assert rows["i:" + extra.uid][0] == "Fence"
    assert p.win.viewport.history.undo()
    assert "i:" + extra.uid not in p.rows()


def test_a_folded_section_waits_until_it_shows():
    from views.wgpu_ui import _sync_project_rows
    p = _filing_project(*_building())
    p.rows()
    before = p.view.renderer.calls["ui_set_tree"]
    p.view.renderer.ui_project_hidden = lambda: True
    p.scene.groups.append(_group("Late"))
    p.scene.version += 1
    _sync_project_rows(p.view, p.win)
    assert p.view.renderer.calls["ui_set_tree"] is before
    assert "Late" in {name for name, _ in p.rows().values()}


def test_ten_thousand_nodes_build_once_and_refresh_quickly():
    roots = _random_tree(10_000)
    p = _filing_project(*roots)
    p.view._ui_pages = {"": len(roots)}
    stack = roots.copy()
    while stack:
        g = stack.pop()
        if g.children:
            p.view._ui_exp.update(("i:" + g.uid, "c:" + g.uid))
            stack.extend(g.children)
    t0 = time.process_time()
    rows = p.rows()
    assert len([k for k in rows if k.startswith(("i:", "c:"))]) == 10_000
    assert time.process_time() - t0 < 5.0
    previous = p.view.renderer.calls["ui_set_tree"]
    t0 = time.process_time()
    p.rows()
    assert time.process_time() - t0 < 1.0
    assert p.view.renderer.calls["ui_set_tree"] is previous


def _sheets_model(win):
    from core.composition import Composicion, MarcoVista
    from built_core.saved_views import SavedView
    scene = win.viewport.scene
    cam = win.viewport.camera
    scene.saved_views[:] = [SavedView.capture("Plan L1", scene, cam),
                            SavedView.capture("North elevation", scene, cam)]
    a101 = Composicion(name="A101 Plans", paper="A3")
    a101.frames = [MarcoVista(view_key="scene:Plan L1", scale_n=100.0),
                   MarcoVista(view_key="scene:Section A", scale_n=50.0)]
    a102 = Composicion(name="A102 Elevations")
    a102.frames = [MarcoVista(view_key="scene:Plan L1", scale_n=200.0),
                   MarcoVista(view_key="std:front", scale_n=100.0)]
    scene.compositions[:] = [a101, a102]
    return a101, a102


def test_renaming_a_scene_carries_its_sheet_views_along(native_host):
    win = native_host.window
    scene = win.viewport.scene
    a101, a102 = _sheets_model(win)
    view = scene.saved_views[0]
    assert outliner.rename_scene(scene, view, "Ground floor")
    assert a101.frames[0].view_key == "scene:Ground floor"
    assert a102.frames[0].view_key == "scene:Ground floor"
    assert not outliner.is_orphan(scene, a101.frames[0])
    # A name another scene has is refused, and nothing moves.
    assert not outliner.rename_scene(scene, view, "North elevation")
    assert view.name == "Ground floor"


def test_the_scenes_panel_rename_no_longer_orphans_sheet_views(native_host):
    from tests.scene_controls import choose, prompt
    win = native_host.window
    first, second = _sheets_model(win)
    choose(win, 0)
    prompt(win, "scene_rename", "yes", "Ground floor")
    for page in (first, second):
        assert page.frames[0].view_key == "scene:Ground floor"
        assert not outliner.is_orphan(win.viewport.scene, page.frames[0])


def _kinds(project):
    project.rows()
    return {text[0]: text[2] for text, _ in project.view.renderer.calls["ui_set_tree"][0]}


def test_an_extension_names_its_groups_in_the_type_column():
    from views.extension_api import ExtensionApp
    building, tree = _building()
    tree.ext = {"stairs": {"steps": 14}}
    project = _filing_project(building, tree)
    app = ExtensionApp(project.win, "stairs")
    app.add_outliner_kind(lambda g: {
        "type": "Stair", "props": [("Steps", g.ext["stairs"]["steps"])]}
        if "stairs" in (g.ext or {}) else None)
    rows = _kinds(project)
    assert rows["i:" + tree.uid].split("\x1e") == ["Stair", "Steps: 14"]
    assert rows["i:" + building.uid] == "Group"
    filtered = project.action("filter", "stair")
    assert "i:" + tree.uid in filtered and "i:" + building.uid not in filtered


def test_a_failing_kind_provider_is_skipped():
    building, tree = _building()
    project = _filing_project(building, tree)
    project.win._outliner_kinds = [lambda g: 1 / 0, lambda g: {"type": "Thing"}]
    assert _kinds(project)["i:" + tree.uid] == "Thing"
    assert outliner.custom_kind(tree, [lambda g: None]) is None
    assert outliner.custom_kind(tree, [lambda g: {"props": []}]) is None


def _plan(win, target, *, height=None, sheet=True, scale=100):
    from views import wgpu_ui
    from built_render import chrome_forms
    verb, prefix = ("groupplan", "i:") if height is not None else ("cutview", "section:")
    wgpu_ui._tree_action(win, win.view, wgpu_ui.SEP.join((verb, prefix + str(target.uid))))
    form = chrome_forms.current(win)
    assert form is not None
    if height is not None:
        form.receive("height", str(height))
    form.receive("scale", str(scale))
    form.receive("sheet", str(int(sheet)))
    form.receive("open_sheet", "0")
    form.press("ok")
    return win.viewport.scene.saved_views[-1]


def test_a_plan_of_a_group_is_a_cut_a_scene_and_a_sheet_in_one_step(native_question_host):
    building, tree = _building()
    level2 = building.children[1]                 # its wall stands on z = 3
    win = native_question_host.window
    scene = win.viewport.scene
    scene.groups[:] = [building, tree]
    scene.version += 1
    planes0, views0 = len(scene.section_planes), len(scene.saved_views)
    view = _plan(win, level2, height=1.2, sheet=True, scale=50.0)
    assert view is not None and view.name == "Plan Level 2"
    plane = scene.section_planes[-1]
    assert scene.active_section() is plane
    assert abs(plane.point.z() - 3.0 - 1.2) < 1e-6        # 1.20 over its base
    assert abs(plane.normal.z() - 1.0) < 1e-9
    assert view.perspective is False
    assert abs(view.pitch - math.radians(90.0)) < 1e-6      # looking down
    assert view.section["active"] == plane.uid
    sheet = scene.compositions[-1]
    assert sheet.frames[-1].view_key == "scene:Plan Level 2"
    assert sheet.frames[-1].scale_n == 50.0
    assert not outliner.is_orphan(scene, sheet.frames[-1])
    win.viewport.history.undo()                   # all three, at once
    assert len(scene.section_planes) == planes0
    assert len(scene.saved_views) == views0
    assert sheet not in scene.compositions


def test_a_second_plan_gets_its_own_name_and_no_sheet_when_asked(native_question_host):
    building, tree = _building()
    win = native_question_host.window
    scene = win.viewport.scene
    scene.groups[:] = [building, tree]
    scene.version += 1
    sheets0 = len(scene.compositions)
    _plan(win, building, height=1.2, sheet=False)
    second = _plan(win, building, height=1.5, sheet=False)
    assert second.name == "Plan Building 2"
    assert len(scene.compositions) == sheets0


def test_a_view_of_a_vertical_cut_looks_at_it_square_on(native_question_host):
    building, tree = _building()
    win = native_question_host.window
    scene = win.viewport.scene
    scene.groups[:] = [building, tree]
    scene.version += 1
    from built_core.section import SectionPlane
    cut = SectionPlane(QVector3D(4, 3, 0), QVector3D(1, 0, 0),
                       name="Cross section", symbol="C")
    scene.section_planes[:] = [cut]
    planes = list(scene.section_planes)
    view = _plan(win, cut, sheet=True, scale=50.0)
    assert view.name == "Cross section"
    assert scene.section_planes == planes            # no plane added
    assert scene.active_section() is cut
    assert view.perspective is False
    assert abs(view.yaw) < 1e-9 and abs(view.pitch) < 1e-9
    assert view.target[0] == 4.0                      # on the plane
    assert view.section["active"] == cut.uid
    sheet = scene.compositions[-1]
    assert sheet.frames[-1].view_key == "scene:Cross section"
    assert sheet.frames[-1].scale_n == 50.0
    win.viewport.history.undo()
    assert scene.section_planes == planes             # the cut stays
    assert sheet not in scene.compositions


def test_a_horizontal_cut_gives_a_plan(native_question_host):
    building, tree = _building()
    win = native_question_host.window
    scene = win.viewport.scene
    scene.groups[:] = [building, tree]
    scene.version += 1
    from built_core.section import SectionPlane
    cut = SectionPlane(QVector3D(0, 0, 1.4), QVector3D(0, 0, 1),
                       symbol="A")
    scene.section_planes[:] = [cut]
    view = _plan(win, cut, sheet=False)
    assert view.name == "Section A"
    assert abs(view.pitch - math.radians(90.0)) < 1e-9


def test_export_all_pdf_can_take_only_some_sheets(tmp_path, native_host):
    from built_render.sheet_workspace import composer_for

    win = native_host.window
    first, second = _sheets_model(win)
    for page in (first, second):
        page.frames.clear()
    path = tmp_path / "one.pdf"
    assert composer_for(win).export_all_pdf(str(path), [second]) == []
    assert path.read_bytes().count(b"/Type /Page ") == 1


def test_the_scale_fits_the_model_in_the_frame():
    from built_core.saved_views import SavedView
    from core.camera import OrbitCamera
    building, tree = _building()                     # 5.1 m wide, 6 m tall
    from core.scene import Scene
    scene = Scene()
    scene.groups[:] = [building, tree]
    cam = OrbitCamera()
    cam.set_view("front")
    cam.perspective = False
    view = SavedView.capture("Front", scene, cam)
    n = outliner.fit_scale(scene, view, 120.0, 90.0)
    assert n in outliner.STANDARD_SCALES
    lo, hi = scene.bounds()
    tall_mm = (hi.z() - lo.z()) * 1000.0 / n
    assert tall_mm <= 90.0                            # it fits …
    bigger = [s for s in outliner.STANDARD_SCALES if s < n]
    assert not bigger or (hi.z() - lo.z()) * 1000.0 / bigger[-1] > 90 * 0.85


def test_repeated_row_templates_keep_distinct_objects_and_children():
    """Identical presentation is shared, never the represented group/tree."""
    mesh = Mesh()
    first, second = Group(mesh, "First"), Group(mesh, "Second")
    first.adopt([Group(mesh, "Child A")])
    second.adopt([Group(mesh, "Child B"), Group(mesh, "Child C")])
    p = _filing_project(first, second)
    a, b = ("i:" + g.uid for g in (first, second))
    p.view._ui_exp.update((a, b))
    rows = p.rows()
    names = [name for k, (name, _) in rows.items() if k.startswith(("i:", "c:"))]
    assert names == ["First", "Child A", "Second", "Child B", "Child C"]
    assert all(rows["c:" + g.uid][1][0] == rows[a][1][0] + 1
               for g in (*first.children, *second.children))
    second.hidden = True
    p.scene.version += 1
    rows = p.rows()
    assert rows[a][1][5] == 1 and rows[b][1][5] == 0

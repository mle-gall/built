# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Outliner (#219): the model's groups and components as a tree.

Every model here is generated: boxes, nested containers, component copies
and a seeded random tree of ten thousand nodes for the timing."""
from __future__ import annotations

import math
import time

from PySide6.QtCore import Qt
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D
from PySide6.QtWidgets import QApplication

from core import outliner
from core.group import Group, copy_group
from core.mesh import Mesh
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_project_occurrences import _box, _building, _group, _random_tree, _two_planes
from views.outliner import COL_EYE, COL_LAYER, COL_TYPE

_app = QApplication.instance() or QApplication([])

def _window_with(*groups):
    from views.main_window import MainWindow
    win = MainWindow()
    scene = win.viewport.scene
    scene.groups[:] = groups        # without the template's scale figure
    scene.version += 1
    panel = win.project_browser.outliner
    panel.refresh()
    return win, scene, panel


def _close(win):
    win.viewport.scene.end_group_edit()
    win._saved_version = win.viewport.scene.version


def _views(win):
    """The «Views and sheets» panel — in the sheet composer."""
    return win._ensure_composer().views_panel


def _row(panel, group):
    return panel._items[id(group)]


def _eye(panel, group):
    """Click the row's eye."""
    panel._on_item_clicked(_row(panel, group), COL_EYE)


def _shown(panel, group) -> bool:
    return not _row(panel, group).data(COL_EYE, Qt.UserRole)


# ---- The tray panel --------------------------------------------------------

def test_the_tree_shows_the_whole_model_nested():
    building, tree = _building()
    win, scene, panel = _window_with(building, tree)
    t = panel.tree
    assert t.topLevelItemCount() == 2            # the document, the planes
    assert t.topLevelItem(1).isHidden()          # no planes: no folder
    doc = t.topLevelItem(0)
    assert doc.text(0) == "Untitled" and doc.isExpanded()
    assert doc.childCount() == 2
    top = doc.child(0)
    assert top.text(0) == "Building" and top.text(COL_TYPE) == "Group"
    assert top.text(COL_LAYER) == outliner.layer_name(building)
    assert [top.child(i).text(0) for i in range(top.childCount())] == [
        "Level 1", "Level 2"]
    wall = top.child(0).child(0)
    assert wall.text(0) == "Wall" and wall.text(COL_TYPE) == "IfcWall"
    assert wall.data(COL_TYPE, Qt.UserRole) == "ifc"
    assert top.data(COL_TYPE, Qt.UserRole) == "group"
    assert not top.icon(0).isNull() and not top.icon(COL_EYE).isNull()
    assert _shown(panel, building)
    _close(win)


def test_a_selection_in_the_view_is_found_in_the_tree():
    building, tree = _building()
    level2 = building.children[1]
    win, scene, panel = _window_with(building, tree)
    assert not _row(panel, building).isExpanded()
    scene.begin_group_edit(building)
    scene.begin_group_edit(level2)
    scene.selection.add(level2.children[0])
    scene.version += 1
    panel.refresh()
    row = _row(panel, level2.children[0])
    assert row.isSelected()
    assert _row(panel, building).isExpanded()
    assert _row(panel, level2).isExpanded()
    assert panel.tree.currentItem() is row
    _close(win)


def test_rows_update_in_place_and_rebuild_only_when_groups_change():
    building, tree = _building()
    win, scene, panel = _window_with(building, tree)
    row = _row(panel, tree)
    _row(panel, building).setExpanded(True)
    tree.name = "Pine"
    tree.hidden = True
    panel.refresh()
    assert _row(panel, tree) is row
    assert row.text(0) == "Pine" and not _shown(panel, tree)
    extra = _group("Fence", (9, 0, 0))
    scene.groups.append(extra)
    panel.refresh()
    assert _row(panel, tree) is not row             # rebuilt…
    assert _row(panel, building).isExpanded()       # …keeping what was open
    assert _row(panel, extra).text(0) == "Fence"
    scene.groups.remove(extra)
    panel.refresh()
    assert id(extra) not in panel._items
    _close(win)


def test_a_folded_section_waits_until_it_shows():
    building, tree = _building()
    win, scene, panel = _window_with(building, tree)
    assert not panel.isVisible()
    scene.groups.append(_group("Late"))
    win.project_browser.on_scene_changed()
    assert panel._stale
    assert panel.tree.topLevelItem(0).childCount() == 2
    panel.refresh()
    assert panel.tree.topLevelItem(0).childCount() == 3
    _close(win)


def test_an_empty_model_says_so():
    win, scene, panel = _window_with()
    assert panel._empty.isVisibleTo(panel)
    assert panel.tree.topLevelItem(0).childCount() == 0
    _close(win)


def test_ten_thousand_nodes_build_once_and_refresh_quickly():
    roots = _random_tree(10_000)
    win, scene, panel = _window_with()
    scene.groups.extend(roots)
    # This process's CPU time, not wall time: under pytest-xdist the other
    # workers' load is not the outliner's.
    t0 = time.process_time()
    panel.refresh()
    build = time.process_time() - t0
    assert len(panel._items) == 10_000
    t0 = time.process_time()
    panel.refresh()
    again = time.process_time() - t0
    print(f"\n10 000 nodes: build {build * 1000:.0f} ms, "
          f"refresh {again * 1000:.0f} ms")
    assert build < 5.0
    assert again < 1.0
    _close(win)


def test_the_filter_keeps_the_path_to_every_match():
    building, tree = _building()
    level1, level2 = building.children
    win, scene, panel = _window_with(building, tree)
    panel.search.setText("slab")
    assert not _row(panel, level1.children[1]).isHidden()     # the match
    assert not _row(panel, level1).isHidden()                 # its path
    assert _row(panel, level1).isExpanded()
    assert _row(panel, level1.children[0]).isHidden()
    assert _row(panel, level2).isHidden()
    assert _row(panel, tree).isHidden()
    panel.search.setText("ifcwall")                           # by class
    assert not _row(panel, level1.children[0]).isHidden()
    assert _row(panel, level1.children[1]).isHidden()
    panel.search.setText("")
    assert not any(item.isHidden() for item in panel._items.values())
    _close(win)


# ---- Sheets 2D -------------------------------------------------------------

def _sheets_model(win):
    from core.composition import Composicion, MarcoVista
    from core.saved_views import SavedView
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


# ---- Isolate, footer --------------------------------------------------------


def test_isolate_and_show_all_are_one_undo_step_each():
    building, tree = _building()
    level1, level2 = building.children
    wall, slab = level1.children
    win, scene, panel = _window_with(building, tree)
    panel.tree.setCurrentItem(_row(panel, wall))
    panel.isolate()
    assert slab.hidden and level2.hidden and tree.hidden
    assert not wall.hidden and not level1.hidden and not building.hidden
    win.viewport.history.undo()
    assert not (slab.hidden or level2.hidden or tree.hidden)
    panel.refresh()
    assert not panel._isolated_now()
    panel.isolate()
    panel.show_all()
    assert not any(g.hidden for g, _p, _d in outliner.walk(scene.groups))
    win.viewport.history.undo()
    assert tree.hidden
    _close(win)


def test_the_footer_counts_and_follows_the_filter():
    building, tree = _building()
    proto = Group(_box(), name="Chair")
    proto.xform = QMatrix4x4()
    tree.hidden = True
    win, scene, panel = _window_with(building, tree, proto)
    assert panel.summary.text() == "7 groups · 1 components · 1 hidden"
    panel.search.setText("wall")
    assert panel.summary.text() == "2 of 8 match"
    _close(win)


# ---- Rename, links, live refresh (2D) ----------------------------------------

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


# ---- Re-nesting (drag and drop, Tab / Shift+Tab) ----------------------------


# ---- Lock --------------------------------------------------------------------


# ---- Section planes ------------------------------------------------------------


def test_the_fill_colour_comes_from_the_styles_panel_picker():
    building, tree = _building()
    win, scene, panel = _window_with(building, tree)
    _two_planes(scene)
    panel.refresh()
    asked = []
    win.styles_panel._pick_color = lambda title, attr: asked.append(attr)
    panel.pick_section_fill_color()
    assert asked == ["section_fill_color"]
    _close(win)


# ---- A cut's own fill, renaming planes, column widths -----------------------

# ---- 8 · What an extension says a group is ------------------------------------

def test_an_extension_names_its_groups_in_the_type_column():
    from views.extension_api import ExtensionApp
    building, tree = _building()
    tree.ext = {"stairs": {"steps": 14}}
    win, scene, panel = _window_with(building, tree)
    app = ExtensionApp(win, "stairs")
    app.add_outliner_kind(lambda g: {
        "type": "Stair", "props": [("Steps", g.ext["stairs"]["steps"])]}
        if "stairs" in (g.ext or {}) else None)
    row = _row(panel, tree)
    assert row.text(COL_TYPE) == "Stair"
    assert row.toolTip(COL_TYPE) == "Steps: 14"
    assert _row(panel, building).text(COL_TYPE) == "Group"   # not its own
    panel.search.setText("stair")
    assert not row.isHidden() and _row(panel, building).isHidden()
    _close(win)


def test_a_failing_kind_provider_is_skipped():
    building, tree = _building()
    win, scene, panel = _window_with(building, tree)
    win._outliner_kinds = [lambda g: 1 / 0,
                           lambda g: {"type": "Thing"}]
    panel._shape = None
    panel.refresh()
    assert _row(panel, tree).text(COL_TYPE) == "Thing"
    assert outliner.custom_kind(tree, [lambda g: None]) is None
    assert outliner.custom_kind(tree, [lambda g: {"props": []}]) is None
    _close(win)


# ---- 9 · Columns and several rows at once -------------------------------------

def test_the_header_shows_and_hides_columns_and_remembers():
    from views.outliner import COL_COPIES, COL_IFC, COL_MATERIAL
    building, tree = _building()
    proto = Group(_box(), name="Chair")
    proto.xform = QMatrix4x4()
    other = copy_group(proto, QVector3D(1, 0, 0))
    tree.material = {"color": [1.0, 0.0, 0.0]}
    win, scene, panel = _window_with(building, tree, proto, other)
    t = panel.tree
    assert t.isColumnHidden(COL_MATERIAL) and t.isColumnHidden(COL_IFC)
    assert not t.isColumnHidden(COL_TYPE)
    panel.set_column_shown(COL_MATERIAL, True)
    panel.set_column_shown(COL_COPIES, True)
    panel.set_column_shown(COL_IFC, True)
    panel.set_column_shown(COL_LAYER, False)
    assert _row(panel, tree).text(COL_MATERIAL) == "#ff0000"
    assert _row(panel, proto).text(COL_COPIES) == "2"
    wall = building.children[0].children[0]
    assert _row(panel, wall).text(COL_IFC) == "IfcWall"
    _close(win)
    win2, scene2, panel2 = _window_with()
    assert not panel2.tree.isColumnHidden(COL_MATERIAL)
    assert panel2.tree.isColumnHidden(COL_LAYER)
    panel2.set_column_shown(COL_LAYER, True)           # leave it as found
    for col in (COL_MATERIAL, COL_COPIES, COL_IFC):
        panel2.set_column_shown(col, False)
    _close(win2)


# ---- 10 · A plan of a group ------------------------------------------------

def test_a_plan_of_a_group_is_a_cut_a_scene_and_a_sheet_in_one_step():
    building, tree = _building()
    level2 = building.children[1]                 # its wall stands on z = 3
    win, scene, panel = _window_with(building, tree)
    planes0, views0 = len(scene.section_planes), len(scene.saved_views)
    view = panel.create_plan(level2, 1.2, sheet=True, scale=50.0)
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
    _close(win)


def test_a_second_plan_gets_its_own_name_and_no_sheet_when_asked():
    building, tree = _building()
    win, scene, panel = _window_with(building, tree)
    sheets0 = len(scene.compositions)
    panel.create_plan(building, 1.2, sheet=False)
    second = panel.create_plan(building, 1.5, sheet=False)
    assert second.name == "Plan Building 2"
    assert len(scene.compositions) == sheets0
    _close(win)


def test_a_view_of_a_vertical_cut_looks_at_it_square_on():
    building, tree = _building()
    win, scene, panel = _window_with(building, tree)
    from core.section import SectionPlane
    cut = SectionPlane(QVector3D(4, 3, 0), QVector3D(1, 0, 0),
                       name="Cross section", symbol="C")
    scene.section_planes[:] = [cut]
    planes = list(scene.section_planes)
    view = panel.create_cut_view(cut, sheet=True, scale=50.0)
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
    _close(win)


def test_a_horizontal_cut_gives_a_plan():
    building, tree = _building()
    win, scene, panel = _window_with(building, tree)
    from core.section import SectionPlane
    cut = SectionPlane(QVector3D(0, 0, 1.4), QVector3D(0, 0, 1),
                       symbol="A")
    scene.section_planes[:] = [cut]
    view = panel.create_cut_view(cut, sheet=False)
    assert view.name == "Section A"
    assert abs(view.pitch - math.radians(90.0)) < 1e-9
    _close(win)


# ---- Sheets 2D as a place to work ----------------------------------------------


def test_export_all_pdf_can_take_only_some_sheets(tmp_path):
    win, scene, panel = _window_with()
    a101, a102 = _sheets_model(win)
    for c in (a101, a102):
        c.frames[:] = []                              # no GL renders
    comp_win = win._ensure_composer()
    path = tmp_path / "one.pdf"
    assert comp_win.export_all_pdf(str(path), [a102]) == []
    assert path.stat().st_size > 0                    # one page, written
    comp_win._saved_version = scene.version
    _close(win)


# ---- The «Views and sheets» panel, in the composer ------------------------------

def _composer_with_model(win):
    building, tree = _building()
    win.viewport.scene.groups[:] = [building, tree]
    win.viewport.scene.version += 1
    a101, a102 = _sheets_model(win)
    comp_win = win._ensure_composer()
    comp_win.show_sheet(1)                            # A102 open
    return comp_win, a101, a102


def test_the_scale_fits_the_model_in_the_frame():
    from core.saved_views import SavedView
    from core.camera import OrbitCamera
    building, tree = _building()                     # 5.1 m wide, 6 m tall
    win, scene, panel = _window_with(building, tree)
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
    _close(win)


def test_repeated_row_templates_keep_distinct_objects_and_children():
    """Identical presentation is shared, never the represented group/tree."""
    from views.outliner import COL_NAME
    mesh = Mesh()
    first = Group(mesh, 'First')
    second = Group(mesh, 'Second')
    first.adopt([Group(mesh, 'Child A')])
    second.adopt([Group(mesh, 'Child B'), Group(mesh, 'Child C')])
    win, scene, panel = _window_with(first, second)
    try:
        a, b = panel._items[id(first)], panel._items[id(second)]
        assert a.data(COL_NAME, Qt.UserRole) is first
        assert b.data(COL_NAME, Qt.UserRole) is second
        assert a.text(COL_NAME) == 'First'
        assert b.text(COL_NAME) == 'Second'
        assert a.childCount() == 1
        assert b.childCount() == 2
        second.hidden = True
        scene.version += 1
        panel.refresh()
        assert not a.data(COL_EYE, Qt.UserRole)
        assert b.data(COL_EYE, Qt.UserRole)
    finally:
        _close(win)
        win.close()



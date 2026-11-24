# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Layers tab and what came with it: active layer, colour and look per
layer, folders, merge, section planes on layers, layer lines on sheets and
in DXF, templates."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PySide6.QtCore import Qt
from ingetrazo_core import Vec3 as QVector3D
from PySide6.QtWidgets import QApplication, QMenu

_app = QApplication.instance() or QApplication([])

from core import layers as L
from core.group import Group
from core.layers import DEFAULT_LAYER, Layer, assign_layer
from core.mesh import Mesh
from core.saved_views import SavedView
from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401
from tests.test_drawing_records import _square
from core.scene import Scene


# ---- The panel -------------------------------------------------------------

@pytest.fixture
def win():
    from views.main_window import MainWindow
    w = MainWindow()
    w.show()
    _app.processEvents()
    yield w
    w._saved_version = w.viewport.scene.version
    w.close()
    # Closing saves the window's layout; some tests here move the Layers tab
    # to the other side, and the next window of the run (another test file)
    # would open with trays on both sides — wider than it may be (CI:
    # test_the_standing_hint_never_widens_the_window). Leave none behind.
    from PySide6.QtCore import QSettings
    QSettings().remove("ui/window_state")
    QSettings().remove("ui/window_geometry")


def test_add_opens_the_selected_folder_and_preserves_its_parent(native_host):
    from ingetrazo_core import edit_tag_folder
    from tests.native_tree import action
    import json
    scene = native_host.viewport.scene
    action(native_host, "newtag", "")
    assert scene.layers[-1].folder is None
    folder = edit_tag_folder(native_host.viewport.history, "new", json.dumps({"name": "Survey"}))
    inner = edit_tag_folder(native_host.viewport.history, "new", json.dumps({"name": "Roads", "parent": folder}))
    action(native_host, "newtag", "preview-folder:" + inner)
    assert scene.layers[-1].folder == inner and L.folder_parent(scene, inner) == folder
    assert {"preview-folder:" + inner, "preview-folder:" + folder} <= native_host.view._ui_exp


def test_purge_asks_for_the_count_and_accepts_only_a_current_confirmation(native_question_host):
    from tests.native_tree import action
    from views import chrome_dialogs
    host = native_question_host
    scene = host.viewport.scene
    scene.layers.extend([Layer("Vacia 1"), Layer("Vacia 2")])
    for answer in ("cancel", "stale", "readonly", "delete"):
        action(host, "purgetags", "preview-tag:Vacia 1")
        assert "2" in chrome_dialogs.current_sheet(host.window).text
        if answer == "stale":
            scene.version += 1
        scene.worksession_read_only = answer == "readonly"
        assert chrome_dialogs.answer(host.window, "cancel" if answer == "cancel" else "delete")
        assert len(scene.layers) == (1 if answer == "delete" else 3)
        scene.worksession_read_only = False
    action(host, "purgetags", "")
    assert chrome_dialogs.current_sheet(host.window) is None


def test_columns_and_indent_follow_the_browser(win):
    from views.layers_panel import (COL_ACTIVE, COL_COLOR, COL_COUNT,
                                    COL_EYE, COL_LOCK, COL_NAME)
    panel = win.layers_dock.panel
    head = panel.tree.header()
    order = sorted((COL_EYE, COL_LOCK, COL_NAME, COL_ACTIVE, COL_COLOR,
                    COL_COUNT), key=head.visualIndex)
    assert order == [COL_EYE, COL_LOCK, COL_NAME, COL_ACTIVE, COL_COLOR,
                     COL_COUNT]
    assert panel.tree.treePosition() == COL_NAME
    assert panel.tree.indentation() == 12


def test_the_top_bar_ends_with_expand_colour_by_layer_and_more(win):
    from PySide6.QtWidgets import QPushButton, QToolButton
    panel = win.layers_dock.panel
    bar = panel.layout().itemAt(0).layout()
    widgets = [bar.itemAt(i).widget() for i in range(bar.count())]
    assert widgets[-3:] == [panel.expand_btn, panel.color_by_layer,
                            panel.more_btn]
    assert not hasattr(panel, "isolate_btn")       # isolating is per row
    # No row of buttons under the list any more: the tree is the last.
    assert panel.layout().itemAt(panel.layout().count() - 1).widget()         is panel.tree
    assert not panel.findChildren(QPushButton)
    panel._fill_more_menu()
    assert any("template" in a.text() for a in panel.more_menu.actions())
    btn = panel.color_by_layer
    assert isinstance(btn, QToolButton) and btn.isCheckable()
    assert btn.autoRaise() and not btn.icon().isNull()
    btn.click()
    assert win.viewport.scene.display_style.color_by_layer
    btn.click()
    assert not win.viewport.scene.display_style.color_by_layer


def test_the_eye_and_padlock_headers_act_on_every_layer(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B", visible=False)])
    for verb, field, expected in [("headereye", "visible", True), ("headereye", "visible", False),
                                  ("headerlock", "locked", True), ("headerlock", "locked", False)]:
        action(native_host, verb, "tags")
        assert all(getattr(tag, field) == expected for tag in scene.layers)
    assert len(native_host.viewport.history.undo_stack) == 4
    assert native_host.viewport.history.undo() and all(tag.locked for tag in scene.layers)


def test_a_real_click_on_the_eye_of_a_selected_row_acts_on_all(win):
    """With the mouse, the press used to select the clicked row first, and
    only that one changed. Now the press acts on every selected row and
    leaves the selection as it was."""
    from PySide6.QtCore import QPoint
    from PySide6.QtTest import QTest
    from views.layers_panel import COL_EYE
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B"), Layer("C")]
    panel.refresh()
    for n in ("A", "B"):
        panel.item_for(n).setSelected(True)
    tree = panel.tree
    rect = tree.visualItemRect(panel.item_for("A"))
    x = tree.columnViewportPosition(COL_EYE) + tree.columnWidth(COL_EYE) // 2
    QTest.mouseClick(tree.viewport(), Qt.LeftButton, Qt.NoModifier,
                     QPoint(x, rect.center().y()))
    _app.processEvents()
    assert not scene.layer("A").visible and not scene.layer("B").visible
    assert scene.layer("C").visible
    assert {panel._key(it)[1] for it in tree.selectedItems()} == {"A", "B"}


def _eye_point(panel, name):
    from PySide6.QtCore import QPoint
    from views.layers_panel import COL_EYE
    tree = panel.tree
    rect = tree.visualItemRect(panel.item_for(name))
    x = tree.columnViewportPosition(COL_EYE) + tree.columnWidth(COL_EYE) // 2
    return QPoint(x, rect.center().y())


def test_a_wobble_while_clicking_an_eye_selects_no_row(win):
    """The press on an eye is the panel's, so Qt never saw it: a few
    pixels of movement with the button held drew a selection box from
    Qt's last press, and switching a layer on again left a run of layers
    selected (the user's video, 2026-10-19: «Cotas» and «Guías» blue)."""
    from PySide6.QtCore import QEvent, QPointF
    from PySide6.QtGui import QMouseEvent
    from PySide6.QtTest import QTest
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("Capa 1"), Layer("Cotas"), Layer("Guías")]
    panel.refresh()
    tree = panel.tree
    QTest.mouseClick(tree.viewport(), Qt.LeftButton, Qt.NoModifier,
                     _eye_point(panel, DEFAULT_LAYER))        # Qt's last press
    _app.processEvents()
    tree.clearSelection()
    start, end = _eye_point(panel, "Cotas"), _eye_point(panel, "Guías")
    QTest.mousePress(tree.viewport(), Qt.LeftButton, Qt.NoModifier, start)
    move = QMouseEvent(QEvent.MouseMove, QPointF(end),
                       QPointF(tree.viewport().mapToGlobal(end)),
                       Qt.NoButton, Qt.LeftButton, Qt.NoModifier)
    QApplication.sendEvent(tree.viewport(), move)
    QTest.mouseRelease(tree.viewport(), Qt.LeftButton, Qt.NoModifier, end)
    _app.processEvents()
    assert tree.selectedItems() == []
    assert not scene.layer("Cotas").visible and scene.layer("Guías").visible


def test_two_quick_clicks_on_an_eye_switch_it_off_and_on(win):
    """The second of two quick clicks reaches the list as a double click:
    it is a click on the eye too, never a double click that selects (or
    renames) the row."""
    from PySide6.QtCore import QEvent, QPointF
    from PySide6.QtGui import QMouseEvent
    from PySide6.QtTest import QTest
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("Cotas"), Layer("Guías")]
    panel.refresh()
    tree = panel.tree
    p = _eye_point(panel, "Cotas")
    QTest.mouseClick(tree.viewport(), Qt.LeftButton, Qt.NoModifier, p)
    double = QMouseEvent(QEvent.MouseButtonDblClick, QPointF(p),
                         QPointF(tree.viewport().mapToGlobal(p)),
                         Qt.LeftButton, Qt.LeftButton, Qt.NoModifier)
    QApplication.sendEvent(tree.viewport(), double)
    QTest.mouseRelease(tree.viewport(), Qt.LeftButton, Qt.NoModifier, p)
    _app.processEvents()
    assert scene.layer("Cotas").visible                   # off, then on
    assert tree.selectedItems() == []
    assert tree.state() != tree.State.EditingState


def test_the_target_column_sits_after_the_padlock(win):
    from views.layers_panel import (COL_EYE, COL_ISOLATE, COL_LOCK,
                                    COL_NAME, LayersPanel)
    panel = win.layers_dock.panel
    head = panel.tree.header()
    order = [head.logicalIndex(v) for v in range(head.count())]
    assert order[:4] == [COL_EYE, COL_LOCK, COL_ISOLATE, COL_NAME]
    assert list(LayersPanel.COLUMN_ORDER) == order
    zero = panel.item_for(DEFAULT_LAYER)
    assert zero.icon(COL_ISOLATE).isNull()          # always on: no target


def test_a_rows_target_isolates_and_a_second_click_brings_back_what_it_hid(win):
    from views.layers_panel import COL_ISOLATE
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B"), Layer("C"),
                     Layer("Oculta", visible=False)]
    panel.refresh()
    panel.click_cell(("layer", "A"), COL_ISOLATE)
    assert [ly.name for ly in scene.layers if ly.visible] == [
        DEFAULT_LAYER, "A"]
    assert panel.isolated_layers() == {"A"}
    filled = panel.item_for("A").icon(COL_ISOLATE).pixmap(18, 18).toImage()
    empty = panel.item_for("B").icon(COL_ISOLATE).pixmap(18, 18).toImage()
    assert filled != empty                           # A's target is filled
    panel.click_cell(("layer", "B"), COL_ISOLATE)    # another: moves to it
    assert panel.isolated_layers() == {"B"}
    panel.click_cell(("layer", "B"), COL_ISOLATE)    # the same: out
    assert all(scene.layer(n).visible for n in ("A", "B", "C"))
    assert not scene.layer("Oculta").visible         # hidden by hand: stays
    assert panel.isolated_layers() == set()


def test_the_target_acts_on_every_selected_row_and_on_folders(win):
    from views.layers_panel import COL_ISOLATE
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    arq = L.add_folder(scene, "Arq")
    scene.layers += [Layer("A"), Layer("B"), Layer("C", folder=arq),
                     Layer("D", folder=arq)]
    panel.refresh()
    for n in ("A", "B"):
        panel.item_for(n).setSelected(True)
    panel.click_cell(("layer", "A"), COL_ISOLATE)
    assert panel.isolated_layers() == {"A", "B"}
    panel.tree.clearSelection()
    panel.click_cell(("folder", arq), COL_ISOLATE)   # a folder: its layers
    assert panel.isolated_layers() == {"C", "D"}
    folder_icon = panel.item_for(arq, "folder").icon(COL_ISOLATE)
    assert not folder_icon.isNull()
    panel.click_cell(("folder", arq), COL_ISOLATE)
    assert all(ly.visible for ly in scene.layers)


def test_the_target_header_ends_the_isolation(win):
    from views.layers_panel import COL_ISOLATE
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B")]
    panel.refresh()
    panel.toggle_isolate(["A"])
    assert not scene.layer("B").visible
    panel._on_header_clicked(COL_ISOLATE)
    assert scene.layer("B").visible


def test_expand_button_folds_and_unfolds_every_folder(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    a = L.add_folder(scene, "A")
    L.add_folder(scene, "B", parent=a)
    panel.refresh()
    btn = panel.expand_btn
    assert btn.property("opened") and btn.toolTip() == "Collapse all"
    assert not btn.icon().isNull() and btn.text() == ""   # a drawn icon
    panel._expand_or_collapse()
    assert not any(it.isExpanded() for it in panel._all_items()
                   if panel._key(it)[0] == "folder")
    assert not btn.property("opened") and btn.toolTip() == "Expand all"
    panel._expand_or_collapse()
    assert btn.property("opened")


def test_drawing_quick_filters_and_inversion_follow_document_use_and_selection():
    from tests.test_project_drawing_filters import Project
    p = Project(Layer("Usada"), Layer("Oculta", visible=False),
                Layer("Fija", locked=True), Layer("Vacia", printable=False))
    p.view._ui_lens = 1
    group = Group(Mesh(), name="g")
    group.layer = "Usada"
    p.scene.groups.append(group)
    all_tags = {"Untagged", "Usada", "Oculta", "Fija", "Vacia"}
    for query, expected in [("", all_tags), ("is:visible", all_tags - {"Oculta"}),
                            ("is:hidden", {"Oculta"}), ("is:locked", {"Fija"}),
                            ("is:used", {"Untagged", "Usada"}), ("is:unused", {"Fija", "Oculta", "Vacia"}),
                            ("is:unprinted", {"Vacia"}), ("is:selected", set()),
                            ("! is:hidden", all_tags - {"Oculta"}), ("! is:visible is:locked", all_tags - {"Fija"})]:
        assert set(p.names(query, "preview-tag:")) == expected
    p.scene.select([group])
    assert p.names("is:selected", "preview-tag:") == ["Usada"]
    p.scene.saved_views.append(SavedView("Plan", hidden_layers=["Vacia"]))
    p.scene.version += 1
    assert set(p.names("is:unused", "preview-tag:")) == {"Oculta", "Fija"}
    assert set(p.names("", "preview-tag:")) == all_tags


def test_saved_scene_preference_and_make_active(native_host):
    from tests.native_tree import action
    from core.settings import settings
    scene = native_host.viewport.scene
    scene.saved_views.append(SavedView("Planta"))
    action(native_host, "newtaghidden", "1")
    action(native_host, "newtag", "")
    key = scene.layers[-1].key
    assert settings().value("layers/new_hidden_in_scenes", False, type=bool)
    assert list(scene.saved_views[0].hidden_layers) == [key]
    scene.layers[-1].locked = True
    action(native_host, "tagactive", "preview-tag:" + key)
    assert L.active_layer(scene) == key and not scene.layers[-1].locked
    action(native_host, "newtaghidden", "0")
    action(native_host, "newtag", "")
    assert list(scene.saved_views[0].hidden_layers) == [key]


def test_colour_by_layer_button_sets_the_style(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    panel.color_by_layer.setChecked(True)
    assert scene.display_style.color_by_layer
    panel.color_by_layer.setChecked(False)
    assert not scene.display_style.color_by_layer


def test_context_menu_offers_the_layer_actions(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B")]
    scene.saved_views.append(SavedView("S"))
    panel.refresh()
    menu = QMenu()
    panel.fill_menu(menu, panel.item_for("A"))
    texts = {a.text() for a in menu.actions()}
    assert {"Make active", "Move the selection here", "Show all layers",
            "Select content", "Colour…", "Line type", "Line weight",
            "Transparency", "Hide in every scene", "Merge into",
            "Move to folder", "Rename"} <= texts
    assert "Isolate" not in texts                    # the row's target is
    merge = next(a.menu() for a in menu.actions() if a.text() == "Merge into")
    next(a for a in merge.actions() if a.text() == "B").trigger()
    assert scene.layer("A") is None


def test_template_export_and_import_through_the_panel(win, tmp_path):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers.append(Layer("Ejes", linetype="dashdot"))
    path = str(tmp_path / "plantilla.json")
    assert panel.export_template(path)
    scene.layers[:] = [Layer(DEFAULT_LAYER)]
    assert panel.import_template(path)
    assert scene.layer("Ejes").linetype == "dashdot"


def test_right_click_hides_and_isolates_the_clicked_layer(win):
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B")]
    g = Group(Mesh(), name="g")
    g.layer = "A"
    scene.groups.append(g)
    scene.select([g])
    menu = QMenu()
    win._add_layer_submenu(menu, [g])
    sub = [a for a in menu.actions() if a.menu() is not None][0].menu()
    act = {a.text(): a for a in sub.actions()}
    act["Isolate this layer"].trigger()
    assert scene.layer("A").visible and not scene.layer("B").visible
    act["Hide this layer"].trigger()
    assert not scene.layer("A").visible


def test_the_default_layer_without_a_colour_shows_a_none_swatch():
    from views.tree_icons import NO_COLOUR_SLASH, layer_swatch, swatch_icon
    img = lambda icon: icon.pixmap(18, 18).toImage()
    zero = Layer(DEFAULT_LAYER)
    none = img(layer_swatch(zero))
    # A red slash from the top-left corner to the bottom-right…
    def red(x, y):
        c = none.pixelColor(x, y)
        return c.red() > 150 and c.green() < 120 and c.blue() < 120
    assert red(9, 9) and red(6, 6) and red(12, 12)
    assert not red(12, 6) and not red(6, 12)          # …not the other way
    assert none != img(swatch_icon(L.layer_color(zero)))
    assert NO_COLOUR_SLASH.red() > 150
    # …and a picked colour, or any other layer, is a plain swatch.
    zero.color = (0.2, 0.4, 0.6)
    assert img(layer_swatch(zero)) == img(swatch_icon((0.2, 0.4, 0.6)))
    other = Layer("Muros")
    assert img(layer_swatch(other)) == img(swatch_icon(L.layer_color(other)))


def test_back_in_the_drawing_the_picked_rows_let_go(win):
    panel = win.layers_dock.panel
    win.viewport.scene.layers.append(Layer("Muros"))
    panel.refresh()
    panel.tree.selectAll()
    assert panel.tree.selectedItems()
    panel._on_focus_changed(win.viewport.keys_stand_in, panel.more_btn)     # still in here
    assert panel.tree.selectedItems()
    panel._on_focus_changed(panel.tree, win.viewport.keys_stand_in)       # to the model
    assert not panel.tree.selectedItems()
    # A real focus change does it too.
    panel.tree.selectAll()
    win.activateWindow()
    panel.tree.setFocus()
    _app.processEvents()
    win.viewport.setFocus()
    _app.processEvents()
    if QApplication.focusWidget() is win.viewport.keys_stand_in:
        assert not panel.tree.selectedItems()


def test_the_delete_key_deletes_the_current_row(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers += [Layer("A"), Layer("B")]
    action(native_host, "deletetag", "preview-tag:A")
    assert scene.layer("A") is None and scene.layer("B") is not None
    assert native_host.viewport.history.undo() and scene.layer("A") is not None
    action(native_host, "deletetag", "preview-tag:" + DEFAULT_LAYER)
    assert scene.layer(DEFAULT_LAYER) is not None and not native_host.viewport.history.undo_stack


def test_line_type_and_weight_columns_follow_the_colour(win):
    from views.layers_panel import (COL_COLOR, COL_COUNT, COL_LINETYPE,
                                    COL_WEIGHT, ROLE_LINETYPE)
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A", linetype="dashed", lineweight=0.35),
                     Layer("B")]
    panel.refresh()
    head = panel.tree.header()
    order = [head.logicalIndex(v) for v in range(head.count())]
    from views.layers_panel import COL_CUT, COL_HATCH, COL_OPACITY, COL_PRINT
    assert order[-8:] == [COL_COLOR, COL_LINETYPE, COL_HATCH, COL_WEIGHT,
                          COL_CUT, COL_OPACITY, COL_PRINT, COL_COUNT]
    a, b = panel.item_for("A"), panel.item_for("B")
    assert a.data(COL_LINETYPE, ROLE_LINETYPE) == "dashed"
    assert a.text(COL_WEIGHT) == "0.35"
    assert b.data(COL_LINETYPE, ROLE_LINETYPE) == "continuous"
    assert b.text(COL_WEIGHT) == "—"                 # default, from the sheet
    panel.tree.grab()                                 # the delegate paints


def test_a_click_on_the_line_cells_opens_their_list_for_every_selected_row(
        win, monkeypatch):
    from views.layers_panel import COL_LINETYPE, COL_WEIGHT
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B"), Layer("C")]
    panel.refresh()
    for n in ("A", "B"):
        panel.item_for(n).setSelected(True)
    picks = {"pick": None}

    def fake_popup(menu, _at):
        picks["texts"] = [a.text() for a in menu.actions()]
        next(a for a in menu.actions() if a.text() == picks["pick"]).trigger()

    monkeypatch.setattr(panel, "_popup", fake_popup)
    picks["pick"] = "Dash-dot"
    panel.click_cell(("layer", "A"), COL_LINETYPE)
    assert "Continuous" in picks["texts"] and "Dotted" in picks["texts"]
    assert scene.layer("A").linetype == scene.layer("B").linetype == "dashdot"
    assert scene.layer("C").linetype == "continuous"
    picks["pick"] = "0.50 mm"
    panel.click_cell(("layer", "B"), COL_WEIGHT)
    assert "Default (from the sheet)" in picks["texts"]
    assert scene.layer("A").lineweight == scene.layer("B").lineweight == 0.5
    assert panel.item_for("A").text(COL_WEIGHT) == "0.50"
    picks["pick"] = "Default (from the sheet)"
    panel.click_cell(("layer", "A"), COL_WEIGHT)
    assert panel.item_for("A").text(COL_WEIGHT) == "—"


def test_the_line_type_list_shows_the_lines_not_their_names(win):
    from PySide6.QtCore import QPoint
    from PySide6.QtTest import QTest
    from PySide6.QtWidgets import QWidgetAction
    from views.layers_panel import _LineSample
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A", linetype="dashed")]
    panel.refresh()
    menu = QMenu()
    panel._fill_linetypes(menu, ["A"], scene.layer("A"))
    samples = [a.defaultWidget() for a in menu.actions()
               if isinstance(a, QWidgetAction)
               and isinstance(a.defaultWidget(), _LineSample)]
    from views.layers_panel import LayersPanel
    keys = [k for k, _l in LayersPanel.LINETYPE_LABELS]
    assert [s.key for s in samples] == keys
    assert all(isinstance(s, _LineSample) for s in samples)
    assert [s._checked for s in samples] == [k == "dashed" for k in keys]
    dashdot = samples[keys.index("dashdot")]
    assert dashdot.toolTip() == "Dash-dot"             # the name: a tooltip
    for s in samples:
        s.grab()                                        # each paints its line
    QTest.mouseClick(samples[keys.index("center")], Qt.LeftButton,
                     Qt.NoModifier, QPoint(40, 10))
    assert scene.layer("A").linetype == "center"


def test_dropped_menus_stay_inside_the_panel(win):
    """Every list opens the same way — its right edge on the cell's or the
    button's — and inside the panel, a margin from its sides."""
    from PySide6.QtCore import QRect
    from views.layers_panel import (COL_CUT, COL_WEIGHT, MENU_MARGIN,
                                    global_rect, place_menu)
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A")]
    panel.refresh()
    win.layers_dock.raise_()
    _app.processEvents()
    inside = global_rect(panel)
    seen = []
    panel._popup = lambda m, pos: seen.append((m.sizeHint().width(), pos))
    # The line type is a panel of its own now, centred like the hatch
    # picker: the weight lists drop from their cells.
    for col in (COL_CUT, COL_WEIGHT):
        panel.click_cell(("layer", "A"), col)
    panel._open_more_menu()
    for w, at in seen:
        assert at.x() >= inside.left() + MENU_MARGIN
        assert at.x() + w <= inside.right() + 1 - MENU_MARGIN             or at.x() == inside.left() + MENU_MARGIN
    # The two cell lists line up the same way on their cells.
    item = panel.item_for("A")
    row = panel.tree.visualItemRect(item)
    for (w, at), col in zip(seen[:2], (COL_CUT, COL_WEIGHT)):
        cell = QRect(panel.tree.columnViewportPosition(col), row.top(),
                     panel.tree.columnWidth(col), row.height())
        right = global_rect(panel.tree.viewport(), cell).right() + 1
        assert at.x() + w == right or at.x() == inside.left() + MENU_MARGIN             or at.x() + w == inside.right() + 1 - MENU_MARGIN
    # A made-up narrow panel: never left of it.
    menu = panel.more_menu
    tiny = QRect(inside.right() - 30, inside.top(), 30, 400)
    assert place_menu(menu, global_rect(panel.more_btn), tiny).x()         >= tiny.left() + MENU_MARGIN


def test_the_weight_list_looks_like_the_line_type_list(win):
    from PySide6.QtWidgets import QWidgetAction
    from views.layers_panel import _LineSample
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A", lineweight=0.35)]
    panel.refresh()
    menu = QMenu()
    panel._fill_weights(menu, ["A"], scene.layer("A"))
    rows = [a.defaultWidget() for a in menu.actions()
            if isinstance(a, QWidgetAction)]
    assert all(isinstance(r, _LineSample) for r in rows)
    assert rows[0].key is None                       # the default: no line
    assert [r._checked for r in rows].index(True) == 4   # 0.35 is ticked
    widths = [r._width for r in rows[1:]]
    assert widths == sorted(widths) and widths[0] < widths[-1]
    rows[1].grab()


# ---- Layers not printed ------------------------------------------------------


# ---- Order and where new rows go -------------------------------------------

def test_new_tag_at_a_tag_uses_its_folder_and_rejects_a_stale_target(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    arq = L.add_folder(scene, "Arq")
    scene.layers.extend([Layer("Muros", folder=arq), Layer("Suelta")])
    action(native_host, "newtag", "preview-tag:Muros")
    assert scene.layers[-1].folder == arq
    action(native_host, "newtag", "preview-tag:Suelta")
    assert scene.layers[-1].folder is None
    count = len(scene.layers)
    for key in ["preview-tag:missing", "preview-folder:missing"]:
        action(native_host, "newtag", key)
        assert len(scene.layers) == count


def test_behind_another_tab_the_panel_waits_until_shown(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    win.layers_dock.hide()
    _app.processEvents()
    assert not panel.isVisible()
    scene.layers += [Layer("Nueva")]
    win.viewport.notify_scene_changed()
    assert panel.item_for("Nueva") is None                 # not rebuilt hidden
    win.layers_dock.show()
    _app.processEvents()
    assert panel.item_for("Nueva") is not None
# ---- The rebuild waits for the event loop -----------------------------------

def _count_rebuilds(panel):
    """The panel's rebuild, counted. The deferred timer binds the spy, so a
    pending one is dropped first."""
    calls = []
    real = panel.refresh
    if getattr(panel, "_refresh_timer", None) is not None:
        panel._refresh_timer.stop()

    def spy():
        calls.append(1)
        real()
    panel._refresh_timer = None
    panel.refresh = spy
    return calls, real


def _stop_counting(panel, real):
    timer = getattr(panel, "_refresh_timer", None)
    if timer is not None:
        timer.stop()
    panel._refresh_timer = None                 # the next one binds the real
    panel.refresh = real


def test_a_scene_change_rebuilds_after_the_event_loop_not_inside_it(win):
    """``sceneVersionChanged`` reaches the panel from inside ``paintGL``
    (``_sync_edges``) and from the Layers toolbar's list while Qt is still
    closing it: clearing the rows there was an access violation. The burst
    of changes pays one rebuild, on the next turn."""
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    L.add_folder(scene, "Nueva")            # hierarchy changes require a full rebuild
    calls, real = _count_rebuilds(panel)
    try:
        win.viewport.notify_scene_changed()
        win.viewport.notify_scene_changed()
        win.viewport.notify_scene_changed()
        assert calls == []                  # not from inside paint nor a signal
        _app.processEvents()
        assert len(calls) == 1              # one rebuild for the burst
    finally:
        _stop_counting(panel, real)


def test_the_layer_of_what_is_picked_in_the_model_stands_out(win):
    """Picked in the model, its layer's row reads at a glance: tinted, its
    name in bold, the accent bar — and back to plain when nothing is."""
    from views.layers_panel import COL_NAME, ROLE_HOLDS
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("Muros")]
    g = Group(Mesh(), name="muro")
    g.layer = "Muros"
    scene.groups.append(g)
    panel.refresh()
    scene.selection.add(g)
    panel._mark_selection()
    item = panel.item_for("Muros")
    assert item.data(COL_NAME, ROLE_HOLDS)
    assert item.font(COL_NAME).bold()
    assert item.background(COL_NAME).color().alpha() >= 100
    scene.selection.clear()
    panel._mark_selection()
    assert not item.font(COL_NAME).bold()


def test_layer_signature_reads_folders_once(monkeypatch):
    from views.layers_panel import layers_signature
    scene = Scene()
    scene.layer_folders = ['one', 'two', 'three']
    scene.folder_parents = {'two': 'one', 'three': 'missing'}
    read = L.folders_of
    calls = []
    def counted(scene):
        calls.append(1)
        return read(scene)
    monkeypatch.setattr(L, 'folders_of', counted)
    assert layers_signature(scene)[2] == (None, 'one', None)
    assert len(calls) == 1

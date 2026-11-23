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
from core.layers import DEFAULT_LAYER, Layer, assign_layer, layer_of
from core.mesh import Mesh
from core.saved_views import SavedView
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


def test_the_layers_tab_sits_beside_the_browser_and_left_properties(win):
    order = [d.objectName() for d in win._default_tray_order()]
    assert order[:3] == ["tray_properties", "tray_project_browser",
                         "tray_layers"]
    assert not hasattr(win.tray, "layers")
    assert not win.layers_dock.isHidden()


def test_the_add_buttons_sit_left_of_the_filter_icon_only(win):
    from PySide6.QtWidgets import QPushButton, QToolButton
    panel = win.layers_dock.panel
    bar = panel.layout().itemAt(0).layout()
    widgets = [bar.itemAt(i).widget() for i in range(bar.count())]
    assert widgets[:3] == [panel.add_layer_btn, panel.add_folder_btn,
                           panel.search]
    for btn, tip in ((panel.add_layer_btn, "Add layer"),
                     (panel.add_folder_btn, "Add folder")):
        assert isinstance(btn, QToolButton) and btn.autoRaise()
        assert btn.text() == "" and not btn.icon().isNull()
        assert btn.toolTip() == tip
    texts = {b.text() for b in panel.findChildren(QPushButton)}
    assert "Purge" not in texts and "+ Layer" not in texts


def test_add_goes_into_the_selected_folder_with_the_name_open(win):
    from PySide6.QtWidgets import QAbstractItemView
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    panel.add_layer_btn.click()                       # nothing selected
    first = scene.layers[-1]
    assert first.folder is None
    assert panel.tree.state() == QAbstractItemView.EditingState
    assert panel.tree.currentItem() is panel.item_for(first.name)
    panel.add_folder_btn.click()
    folder = L.folders_of(scene)[-1]
    assert L.folder_parent(scene, folder) is None
    item = panel.item_for(folder, "folder")
    panel.tree.setCurrentItem(item)                   # select the folder
    panel.add_layer_btn.click()
    assert scene.layers[-1].folder == folder
    panel.tree.setCurrentItem(panel.item_for(folder, "folder"))
    panel.add_folder_btn.click()
    inner = L.folders_of(scene)[-1]
    assert L.folder_parent(scene, inner) == folder
    assert panel.item_for(inner, "folder").parent() is \
        panel.item_for(folder, "folder")


def test_purge_lives_in_the_right_click_and_asks_with_the_count(win,
                                                                monkeypatch):
    from PySide6.QtWidgets import QMessageBox
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("Vacia 1"), Layer("Vacia 2")]
    panel.refresh()
    asked = []

    def question(_parent, _title, text, *a, **k):
        asked.append(text)
        return QMessageBox.Yes

    monkeypatch.setattr(QMessageBox, "question", staticmethod(question))
    for item in (None, panel.item_for("Vacia 1")):
        menu = QMenu()
        panel.fill_menu(menu, item)
        assert "Purge unused layers…" in [a.text() for a in menu.actions()]
    next(a for a in menu.actions()
         if a.text() == "Purge unused layers…").trigger()
    assert asked and "2" in asked[0]
    assert [ly.name for ly in scene.layers] == [DEFAULT_LAYER]


def test_a_layers_tab_left_in_a_block_of_its_own_rejoins_the_tabs(win):
    """A layout saved before the tab existed put it apart from the other
    tabs; at start-up it goes back beside the Browser."""
    dock = win.layers_dock
    win.removeDockWidget(dock)
    win.addDockWidget(Qt.RightDockWidgetArea, dock)      # a block apart
    dock.show()
    _app.processEvents()
    assert win.project_browser not in win.tabifiedDockWidgets(dock)
    win._seat_new_trays()
    _app.processEvents()
    assert win.project_browser in win.tabifiedDockWidgets(dock)


def test_a_layers_tab_moved_alone_to_the_other_side_stays(win, monkeypatch):
    dock = win.layers_dock
    win.removeDockWidget(dock)
    win.addDockWidget(Qt.LeftDockWidgetArea, dock)
    dock.show()
    monkeypatch.setattr(win, "restoreDockWidget", lambda d: True)
    win._seat_new_trays()
    assert win.dockWidgetArea(dock) == Qt.LeftDockWidgetArea


def test_eye_on_a_selected_row_acts_on_every_selected_row(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B"), Layer("C")]
    panel.refresh()
    for name in ("A", "B"):
        panel.item_for(name).setSelected(True)
    panel._on_item_clicked(panel.item_for("A"), 2)          # COL_EYE
    _app.processEvents()
    assert [scene.layer(n).visible for n in "ABC"] == [False, False, True]
    panel.click_cell(("layer", "C"), 3)                     # COL_LOCK
    assert scene.layer("C").locked


def test_folder_row_shows_hides_and_counts(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A", folder="Arq"), Layer("B", folder="Arq")]
    f = _square(scene.mesh)
    assign_layer(f, "A")
    panel.refresh()
    folder = panel.item_for("Arq", "folder")
    assert folder.childCount() == 2
    panel.click_cell(("folder", "Arq"), 2)
    assert not L.effective_visible(scene, scene.layer("A"))
    assert not L.effective_visible(scene, scene.layer("B"))
    panel.click_cell(("folder", "Arq"), 2)
    assert L.effective_visible(scene, scene.layer("A"))
    panel.update_counts()
    assert panel.item_for("A").text(5) == "1"


def test_sorting_keeps_the_default_layer_first(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("zeta"), Layer("alfa")]
    panel.refresh()
    for order in (Qt.AscendingOrder, Qt.DescendingOrder):
        panel.tree.sortByColumn(0, order)
        names = [panel.tree.topLevelItem(i).text(0)
                 for i in range(panel.tree.topLevelItemCount())]
        assert names[0] == DEFAULT_LAYER
        assert names[1:] == (["alfa", "zeta"] if order == Qt.AscendingOrder
                             else ["zeta", "alfa"])


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


def test_the_eye_and_padlock_headers_act_on_every_layer(win):
    from views.layers_panel import COL_EYE, COL_LOCK, COL_NAME
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B", visible=False)]
    panel.refresh()
    panel._on_header_clicked(COL_EYE)            # not all visible → all on
    assert all(ly.visible for ly in scene.layers)
    panel._on_header_clicked(COL_EYE)            # all visible → all off
    assert not any(ly.visible for ly in scene.layers)
    panel._on_header_clicked(COL_LOCK)
    assert all(ly.locked for ly in scene.layers)
    panel._on_header_clicked(COL_LOCK)
    assert not any(ly.locked for ly in scene.layers)
    panel._on_header_clicked(COL_NAME)           # Name: sorts, descending
    names = [panel.tree.topLevelItem(i).text(0)
             for i in range(panel.tree.topLevelItemCount())]
    assert names == [DEFAULT_LAYER, "B", "A"]


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


def test_the_top_bar_buttons_share_one_size(win):
    """Icons, not text glyphs: the expand and templates buttons are as tall
    as the add buttons beside them."""
    panel = win.layers_dock.panel
    win.layers_dock.raise_()
    _app.processEvents()
    assert not panel.more_btn.icon().isNull() and panel.more_btn.text() == ""
    sizes = {b.sizeHint().height() for b in (
        panel.add_layer_btn, panel.add_folder_btn, panel.expand_btn,
        panel.color_by_layer, panel.more_btn)}
    assert len(sizes) == 1


def test_a_new_folder_shows_even_empty(win):
    """The add-folder button made a folder nobody could see: with no filter
    an empty folder had no row to be shown through."""
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    panel.add_folder_btn.click()
    name = L.folders_of(scene)[-1]
    item = panel.item_for(name, "folder")
    assert item is not None and not item.isHidden()
    panel.refresh()                                   # and after a rebuild
    assert not panel.item_for(name, "folder").isHidden()


def test_dropping_rows_files_them_in_a_folder(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B"), Layer("C", folder="Arq")]
    L.add_folder(scene, "Ext")
    scene.layer_folders = L.folders_of(scene)
    panel.refresh()
    panel.drop_rows([("layer", "A"), ("layer", "B"),
                     ("layer", DEFAULT_LAYER)], "Arq")
    assert scene.layer("A").folder == scene.layer("B").folder == "Arq"
    assert scene.layer(DEFAULT_LAYER).folder is None      # never filed
    ext = L.folders_of(scene)[-1]
    panel.drop_rows([("folder", "Arq"), ("layer", "C")], ext)
    assert L.folder_parent(scene, "Arq") == ext
    assert scene.layer("C").folder == "Arq"               # went with it
    panel.drop_rows([("folder", ext)], "Arq")             # into its own child
    assert L.folder_parent(scene, ext) is None
    panel.drop_rows([("layer", "A")], None)               # out of any
    assert scene.layer("A").folder is None


def test_the_drop_target_is_the_folder_under_the_pointer(win):
    from PySide6.QtCore import QPoint
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B", folder="Arq")]
    panel.refresh()
    tree = panel.tree
    for name, kind, expect in (("Arq", "folder", "Arq"),
                               ("B", "layer", "Arq"),
                               ("A", "layer", None)):
        rect = tree.visualItemRect(panel.item_for(name, kind))
        assert tree.target_folder(rect.center()) == expect
    # Empty space under the last row: out of any folder. The list is made
    # tall enough to have some, whatever window size the run left.
    tree.setMinimumHeight(400)
    _app.processEvents()
    rows = [panel.tree.topLevelItem(i)
            for i in range(panel.tree.topLevelItemCount())]
    below = max(tree.visualItemRect(it).bottom() for it in rows) + 10
    assert below < tree.viewport().height()
    assert tree.target_folder(QPoint(0, below)) is None


def test_a_real_drag_and_drop_moves_the_selected_rows(win):
    """The event path itself: a drop on a folder files every selected
    layer there, and Qt does not move rows on its own."""
    from PySide6.QtCore import QMimeData, QPointF
    from PySide6.QtGui import QDropEvent
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B"), Layer("C", folder="Arq")]
    panel.refresh()
    tree = panel.tree
    for n in ("A", "B"):
        panel.item_for(n).setSelected(True)
    pos = QPointF(tree.visualItemRect(panel.item_for("Arq", "folder")).center())

    class _Drop(QDropEvent):
        def source(self):
            return tree

    ev = _Drop(pos, Qt.MoveAction, QMimeData(), Qt.LeftButton, Qt.NoModifier)
    tree.dropEvent(ev)
    _app.processEvents()
    assert scene.layer("A").folder == scene.layer("B").folder == "Arq"


def test_several_selected_rows_take_one_colour_and_one_look(win,
                                                            monkeypatch):
    from PySide6.QtGui import QColor
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B"), Layer("C")]
    panel.refresh()
    for n in ("A", "B"):
        panel.item_for(n).setSelected(True)
    monkeypatch.setattr("views.color_dialog.get_color",
                        lambda *a, **k: QColor.fromRgbF(1.0, 0.0, 0.0))
    panel._on_item_clicked(panel.item_for("A"), 4)         # COL_COLOR
    _app.processEvents()
    assert scene.layer("A").color == scene.layer("B").color == (1.0, 0.0, 0.0)
    assert scene.layer("C").color is None
    panel.set_style(["A", "B"], linetype="dashed", lineweight=0.5,
                    opacity=0.5)
    assert {(scene.layer(n).linetype, scene.layer(n).lineweight,
             scene.layer(n).opacity) for n in "AB"} == {("dashed", 0.5, 0.5)}
    panel.click_cell(("layer", "A"), 2, ["A", "B"])        # eye on both
    assert not scene.layer("A").visible and not scene.layer("B").visible
    menu = QMenu()
    panel.fill_menu(menu, panel.item_for("A"))
    reset = next(a for a in menu.actions() if a.text() == "Automatic colour")
    reset.trigger()
    assert scene.layer("A").color is None and scene.layer("B").color is None


def _shown(panel):
    return sorted(panel._key(it)[1] for it in panel._all_items()
                  if panel._key(it)[0] == "layer" and not it.isHidden())


def test_the_funnels_quick_filters(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("Usada"), Layer("Oculta", visible=False),
                     Layer("Fija", locked=True), Layer("Vacia")]
    g = Group(Mesh(), name="g")
    g.layer = "Usada"
    scene.groups.append(g)
    panel.refresh()
    expect = {
        "all": [DEFAULT_LAYER, "Fija", "Oculta", "Usada", "Vacia"],
        "visible": [DEFAULT_LAYER, "Fija", "Usada", "Vacia"],
        "hidden": ["Oculta"],
        "locked": ["Fija"],
        "used": [DEFAULT_LAYER, "Usada"],
        "unused": ["Fija", "Oculta", "Vacia"],
        "selection": [],
    }
    for key, names in expect.items():
        panel.set_state_filter(key)
        assert _shown(panel) == sorted(names), key
    scene.select([g])
    win.viewport.notify_scene_changed()             # follows the selection
    _app.processEvents()                            # the rebuild is deferred
    assert _shown(panel) == ["Usada"]


def test_invert_and_the_funnel_says_it_is_on(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A", visible=False), Layer("B")]
    panel.refresh()
    assert "All layers" in panel.filter_btn.toolTip()
    panel.set_state_filter("hidden")
    assert _shown(panel) == ["A"]
    panel.set_invert_filter(True)
    assert _shown(panel) == sorted([DEFAULT_LAYER, "B"])
    assert "inverted" in panel.filter_btn.toolTip()
    panel.set_state_filter("all")
    panel.set_invert_filter(False)
    assert _shown(panel) == sorted([DEFAULT_LAYER, "A", "B"])


def test_the_filter_menu_lists_the_filters_and_invert(win):
    panel = win.layers_dock.panel
    panel._fill_filter_menu()
    texts = [a.text() for a in panel.filter_menu.actions() if not a.isSeparator()]
    assert texts == ["All layers", "Visible layers", "Hidden layers",
                     "Locked layers", "Layers not printed",
                     "Layers in use", "Unused layers",
                     "Layers of the selection", "Invert filter"]
    next(a for a in panel.filter_menu.actions()
         if a.text() == "Hidden layers").trigger()
    assert panel.state_filter == "hidden"


def test_active_column_and_new_layer_option(win, monkeypatch):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.saved_views.append(SavedView("Planta"))
    monkeypatch.setattr(type(panel), "new_layers_hidden_in_scenes",
                        classmethod(lambda cls: True))
    panel._on_add()
    name = scene.layers[-1].name
    assert scene.saved_views[0].hidden_layers == [name]
    panel.click_cell(("layer", name), 1)                    # COL_ACTIVE
    assert L.active_layer(scene) == name


def test_active_pencil_follows_the_chosen_accent(win):
    from views import theme

    panel = win.layers_dock.panel
    item = panel.item_for(DEFAULT_LAYER)

    def has(icon, colour):
        image = icon.pixmap(18, 18).toImage()
        return any(
            image.pixelColor(x, y).alpha() > 200
            and abs(image.pixelColor(x, y).red() - colour.red()) < 12
            and abs(image.pixelColor(x, y).green() - colour.green()) < 12
            and abs(image.pixelColor(x, y).blue() - colour.blue()) < 12
            for y in range(image.height()) for x in range(image.width())
        )

    blue = theme.accent_color("blue")
    try:
        theme.save_accent("blue")
        theme.apply_accent(_app)
        assert has(item.icon(1), blue)                       # COL_ACTIVE
        assert has(panel.tree.headerItem().icon(1), blue)
    finally:
        theme.save_accent(theme.ORANGE)
        theme.apply_accent(_app)


def test_row_icons_follow_a_live_theme_switch(win):
    # #1086: after switching the theme with the app open, the eye, padlock,
    # isolate and print marks stayed in the previous theme's ink
    from views import theme

    panel = win.layers_dock.panel
    win.viewport.scene.layers.append(Layer("A"))     # has an isolate target
    panel.refresh()

    def lightness(icon):
        image = icon.pixmap(18, 18).toImage()
        return [image.pixelColor(x, y).lightness()
                for y in range(image.height()) for x in range(image.width())
                if image.pixelColor(x, y).alpha() > 100]

    columns = (2, 3, 6, 10)               # COL_EYE, COL_LOCK, COL_ISOLATE, COL_PRINT
    try:
        for mode, other in ((theme.LIGHT, theme.dark_palette()),
                            (theme.DARK, theme.light_palette())):
            theme.apply_theme(_app, mode)
            _app.processEvents()
            ink = _app.palette().text().color().lightness()
            old = other.text().color().lightness()
            row, head = panel.item_for("A"), panel.tree.headerItem()
            for col in columns:
                for icon in (row.icon(col), head.icon(col)):
                    seen = lightness(icon)
                    # drawn, and every visible pixel in this theme's ink
                    assert seen, (mode, col)
                    assert all(abs(v - ink) < abs(v - old) for v in seen), (mode, col)
    finally:
        theme.apply_theme(_app, theme.DARK)


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
            "Move to folder", "Rename", "Delete"} <= texts
    assert "Isolate" not in texts                    # the row's target is
    merge = next(a.menu() for a in menu.actions() if a.text() == "Merge into")
    next(a for a in merge.actions() if a.text() == "B").trigger()
    assert scene.layer("A") is None


def test_look_menus_set_the_line_and_opacity(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers.append(Layer("A"))
    panel.set_style(["A"], linetype="dotted", lineweight=0.25, opacity=0.5)
    ly = scene.layer("A")
    assert (ly.linetype, ly.lineweight, ly.opacity) == ("dotted", 0.25, 0.5)


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
    from views.layers_panel import NO_COLOUR_SLASH, layer_swatch, swatch_icon
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
    panel._on_focus_changed(win.viewport.keys_stand_in, panel.search)     # still in here
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


def test_the_headers_take_a_real_mouse_click(win):
    """Switching sorting off makes Qt's header unclickable; the eye and
    padlock headers must still answer the mouse."""
    from PySide6.QtCore import QPoint
    from PySide6.QtTest import QTest
    from views.layers_panel import COL_EYE, COL_LOCK
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B")]
    panel.refresh()
    win.layers_dock.raise_()
    _app.processEvents()
    head = panel.tree.header()
    assert head.sectionsClickable()

    def click(col):
        x = head.sectionViewportPosition(col) + head.sectionSize(col) // 2
        QTest.mouseClick(head.viewport(), Qt.LeftButton, Qt.NoModifier,
                         QPoint(x, head.height() // 2))
        _app.processEvents()

    click(COL_EYE)
    assert not any(ly.visible for ly in scene.layers)
    click(COL_EYE)
    assert all(ly.visible for ly in scene.layers)
    click(COL_LOCK)
    assert all(ly.locked for ly in scene.layers)
    click(COL_LOCK)
    assert not any(ly.locked for ly in scene.layers)


def _drag_move(tree, pos):
    from PySide6.QtCore import QMimeData
    from PySide6.QtGui import QDragMoveEvent

    class _Move(QDragMoveEvent):
        def source(self):
            return tree

    ev = _Move(pos, Qt.MoveAction, QMimeData(), Qt.LeftButton, Qt.NoModifier)
    tree.dragMoveEvent(ev)
    return ev


def test_a_drag_outlines_the_folder_it_would_file_into(win):
    """With folders inside folders, the outline is the INNER folder's
    block, not its parent's — and it says where in words."""
    from core import layers as L
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    arq = L.add_folder(scene, "Arq")
    est = L.add_folder(scene, "Est", parent=arq)
    scene.layers += [Layer("A"), Layer("B", folder=arq),
                     Layer("C", folder=est)]
    panel.refresh()
    panel.tree.expandAll()
    _app.processEvents()
    tree = panel.tree
    rect = lambda n, k="layer": tree.visualItemRect(panel.item_for(n, k))
    # The outline spans the list's width; a row's middle may lie past it
    # when the columns scroll sideways, so its height is what counts.
    _rows_of = lambda box, r: box.top() <= r.center().y() <= box.bottom()

    ev = _drag_move(tree, rect("C").center())
    assert ev.isAccepted() and tree.drop_hint == (est,)
    box = tree.drop_rect(est)
    assert _rows_of(box, rect(est, "folder"))
    assert _rows_of(box, rect("C"))
    assert not _rows_of(box, rect(arq, "folder"))
    assert not _rows_of(box, rect("B"))

    _drag_move(tree, rect("B").center())
    assert tree.drop_hint == (arq,)
    box = tree.drop_rect(arq)
    for n, k in ((arq, "folder"), (est, "folder"), ("C", "layer"),
                 ("B", "layer")):
        assert _rows_of(box, rect(n, k))
    assert not _rows_of(box, rect("A"))

    _drag_move(tree, rect("A").center())
    assert tree.drop_hint == (None,)
    assert tree.drop_rect(None) == tree.viewport().rect()
    tree.grab()                                       # paints the hint
    tree._clear_hint()
    assert tree.drop_hint is None


def test_the_dragged_rows_are_stacked_see_through_and_counted(win):
    """Rows far apart are drawn one under the other with no empty band
    between them, half see-through; past a handful, the rest is counted."""
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer(n) for n in "ABCDEFGHIJ"]
    panel.refresh()
    tree = panel.tree
    for n in ("A", "D", "H"):
        panel.item_for(n).setSelected(True)
    pm, rows = tree.drag_pixmap()
    h = tree.visualItemRect(panel.item_for("A")).height()
    assert [panel._key(r)[1] for r in rows] == ["A", "D", "H"]
    assert round(pm.height() / pm.devicePixelRatio()) == 3 * h
    img = pm.toImage()
    alphas = {img.pixelColor(x, y).alpha()
              for x in range(5, img.width() - 5, 7)
              for y in range(3, img.height() - 3, 3)}
    assert alphas and max(alphas) < 255            # see-through
    for n in "BCEFGIJ":
        panel.item_for(n).setSelected(True)
    pm, rows = tree.drag_pixmap()
    assert len(rows) == 10
    assert round(pm.height() / pm.devicePixelRatio()) == (tree.DRAG_ROWS + 1) * h


def test_a_new_row_opens_its_name_without_a_half_lit_highlight(win):
    from PySide6.QtTest import QTest
    from PySide6.QtWidgets import QAbstractItemView
    panel = win.layers_dock.panel
    win.layers_dock.raise_()
    _app.processEvents()
    for add, kind in ((panel.add_layer, "layer"), (panel.add_folder, "folder")):
        name = add()
        _app.processEvents()
        tree = panel.tree
        assert tree.state() == QAbstractItemView.EditingState
        assert tree.currentItem() is panel.item_for(name, kind)
        assert tree.selectedItems() == []
        # Typed and Enter: the row is selected whole, under its new name.
        editor = tree.focusWidget()
        editor.setText(kind + " nueva")
        QTest.keyClick(editor, Qt.Key_Return)
        _app.processEvents()
        _app.processEvents()
        assert tree.state() != QAbstractItemView.EditingState
        # The row keeps its key (#tags-by-key); the typed text is its label.
        assert [panel._key(i) for i in tree.selectedItems()] == [(kind, name)]
        assert [i.text(0) for i in tree.selectedItems()] == [kind + " nueva"]
        made = name
    # The folder just made is selected: the next layer goes into it.
    inner = panel.add_layer()
    assert win.viewport.scene.layer(inner).folder == made


def test_the_delete_key_deletes_the_current_row(win, monkeypatch):
    from PySide6.QtTest import QTest
    from PySide6.QtWidgets import QMessageBox
    monkeypatch.setattr(QMessageBox, "question",
                        lambda *a, **k: QMessageBox.Yes)
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B")]
    panel.refresh()
    panel.tree.setCurrentItem(panel.item_for("A"))
    QTest.keyClick(panel.tree, Qt.Key_Delete)
    _app.processEvents()
    assert scene.layer("A") is None and scene.layer("B") is not None
    win.viewport.history.undo()                      # one undo step
    assert scene.layer("A") is not None
    panel.refresh()
    panel.tree.setCurrentItem(panel.item_for(DEFAULT_LAYER))
    QTest.keyClick(panel.tree, Qt.Key_Delete)        # the default stays
    assert scene.layer(DEFAULT_LAYER) is not None


def test_move_the_selection_here_from_the_rows_menu(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A")]
    g = Group(Mesh(), name="g")
    scene.groups.append(g)
    panel.refresh()
    menu = QMenu()
    panel.fill_menu(menu, panel.item_for("A"))
    act = next(a for a in menu.actions()
               if a.text() == "Move the selection here")
    assert not act.isEnabled()                       # nothing selected
    scene.select([g])
    menu = QMenu()
    panel.fill_menu(menu, panel.item_for("A"))
    act = next(a for a in menu.actions()
               if a.text() == "Move the selection here")
    assert act.isEnabled()
    act.trigger()
    assert layer_of(g) == "A"


def test_leaving_a_new_name_for_the_model_leaves_nothing_selected(win):
    """Typing a new layer's name and clicking straight into the drawing:
    the row used to get selected AFTER the drawing took the focus, and
    stayed lit while working in the model."""
    from PySide6.QtCore import QPoint
    from PySide6.QtTest import QTest
    panel = win.layers_dock.panel
    win.activateWindow()
    win.layers_dock.raise_()
    _app.processEvents()
    panel.add_layer()
    _app.processEvents()
    QTest.mouseClick(win.viewport.keys_stand_in, Qt.LeftButton, Qt.NoModifier,
                     QPoint(300, 300))
    for _ in range(3):
        _app.processEvents()
    if QApplication.focusWidget() is win.viewport.keys_stand_in:
        assert panel.tree.selectedItems() == []
    # Left with Enter instead (still in the panel): selected, as before.
    panel._just_made = None
    panel._select_just_made()                          # nothing pending
    assert panel.tree.selectedItems() == []


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


def test_the_columns_button_switches_columns_and_remembers(win):
    from PySide6.QtCore import QSettings
    from views.layers_panel import COL_COUNT, COL_LINETYPE, LayersPanel
    panel = win.layers_dock.panel
    QSettings().remove(LayersPanel.HIDDEN_COLUMNS_KEY)
    panel._apply_column_choice()
    # By default the weight, the print mark, the transparency, the count
    # and the cut weight are off.
    from views.layers_panel import COL_CUT, COL_OPACITY, COL_PRINT, COL_WEIGHT
    assert panel.hidden_columns() == {COL_WEIGHT, COL_PRINT, COL_OPACITY,
                                      COL_COUNT, COL_CUT}
    assert panel.tree.isColumnHidden(COL_COUNT)
    assert panel.tree.isColumnHidden(COL_OPACITY)
    assert panel.columns_button.text() == "+5"
    for c in (COL_COUNT, COL_OPACITY, COL_PRINT,
              COL_WEIGHT, COL_CUT):                # now chosen: all on
        panel.set_column_shown(c, True)
    win.layers_dock.raise_()
    _app.processEvents()
    b = panel.columns_button
    assert b.text() == "⋮" and not b.isHidden()
    head = panel.tree.header()
    # At the header's right end, level with it.
    assert b.geometry().left() == head.geometry().right() + 1
    panel.set_column_shown(COL_LINETYPE, False)
    panel.set_column_shown(COL_COUNT, False)
    assert panel.tree.isColumnHidden(COL_LINETYPE)
    assert panel.tree.isColumnHidden(COL_COUNT)
    assert b.text() == "+2"
    menu = panel._columns_menu()
    checks = {a.text(): a.isChecked() for a in menu.actions()}
    assert checks["Line type"] is False and checks["Colour"] is True
    next(a for a in menu.actions() if a.text() == "Line type").trigger()
    assert not panel.tree.isColumnHidden(COL_LINETYPE)
    # Remembered for the next window.
    assert LayersPanel.hidden_columns(panel) == {COL_COUNT}
    QSettings().remove(LayersPanel.HIDDEN_COLUMNS_KEY)
    panel._apply_column_choice()
    assert b.text() == "+5"                        # back to the default


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
    panel._open_filter_menu()
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


def test_the_columns_menu_stays_open_and_shows_all(win):
    from PySide6.QtCore import QPoint, QSettings
    from PySide6.QtTest import QTest
    from views.layers_panel import COL_COLOR, COL_COUNT, COL_WEIGHT, LayersPanel
    panel = win.layers_dock.panel
    QSettings().setValue(LayersPanel.HIDDEN_COLUMNS_KEY, "")   # all on
    QSettings().setValue(LayersPanel.KNOWN_COLUMNS_KEY, ",".join(
        str(c) for c, _l in LayersPanel.OPTIONAL_COLUMNS))
    panel._apply_column_choice()
    menu = panel._columns_menu()
    menu.popup(QPoint(100, 100))
    _app.processEvents()
    acts = {a.text(): a for a in menu.actions() if a.text()}

    def click(text):
        r = menu.actionGeometry(acts[text])
        QTest.mouseClick(menu, Qt.LeftButton, Qt.NoModifier, r.center())
        _app.processEvents()

    click("Colour")
    click("Line weight")                     # a second switch: still open
    assert menu.isVisible()
    assert panel.tree.isColumnHidden(COL_COLOR)
    assert panel.tree.isColumnHidden(COL_WEIGHT)
    assert panel.columns_button.text() == "+2"
    click("Show all columns")
    assert menu.isVisible()
    assert not any(panel.tree.isColumnHidden(c)
                   for c, _l in LayersPanel.OPTIONAL_COLUMNS)
    assert all(a.isChecked() for a in acts.values() if a.isCheckable())
    menu.close()
    assert panel.columns_button.text() == "⋮"
    assert not panel.tree.isColumnHidden(COL_COUNT)


def test_the_transparency_column_and_its_list(win):
    from PySide6.QtWidgets import QWidgetAction
    from views.layers_panel import COL_OPACITY, _LineSample
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A", opacity=0.5), Layer("B"), Layer("C")]
    panel.set_column_shown(COL_OPACITY, True)
    assert not panel.tree.isColumnHidden(COL_OPACITY)
    assert panel.item_for("A").text(COL_OPACITY) == "50 %"
    assert panel.item_for("B").text(COL_OPACITY) == "—"
    # With the column on, the name no longer repeats it.
    assert "50" not in panel.item_for("A").toolTip(0)
    menu = QMenu()
    panel._fill_opacities(menu, ["A"], scene.layer("A"))
    rows = [a.defaultWidget() for a in menu.actions()
            if isinstance(a, QWidgetAction)]
    assert all(isinstance(r, _LineSample) and r.swatch for r in rows)
    assert [r._checked for r in rows] == [False, False, True, False]
    rows[2].grab()                                    # paints the swatch
    for n in ("B", "C"):
        panel.item_for(n).setSelected(True)
    chosen = {}

    def fake_popup(m, _at):
        next(a for a in m.actions() if a.text() == "75 %").trigger()
        chosen["done"] = True

    panel._popup = fake_popup
    panel.click_cell(("layer", "B"), COL_OPACITY)
    assert chosen and scene.layer("B").opacity == scene.layer("C").opacity         == 0.25
    panel.set_column_shown(COL_OPACITY, False)
    assert "Transparency 50 %" in panel.item_for("A").toolTip(0)


# ---- Layers not printed ------------------------------------------------------


def test_the_print_column_and_its_header(win):
    from views.layers_panel import COL_PRINT
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B"), Layer("C")]
    panel.refresh()
    panel.set_column_shown(COL_PRINT, True)          # off by default
    for n in ("A", "B"):
        panel.item_for(n).setSelected(True)
    panel.click_cell(("layer", "A"), COL_PRINT)
    assert not scene.layer("A").printable and not scene.layer("B").printable
    assert scene.layer("C").printable and scene.layer("A").visible
    panel.set_state_filter("unprinted")
    shown = {panel._key(it)[1] for it in panel._all_items()
             if not it.isHidden() and panel._key(it)[0] == "layer"}
    assert shown == {"A", "B"}
    panel.set_state_filter("all")
    panel._on_header_clicked(COL_PRINT)                 # some off → all on
    assert all(ly.printable for ly in scene.layers)
    panel._on_header_clicked(COL_PRINT)                 # all on → all off
    assert not any(ly.printable for ly in scene.layers)


# ---- Order and where new rows go -------------------------------------------

def _layer_rows(panel):
    return [panel.tree.topLevelItem(i).text(0)
            for i in range(panel.tree.topLevelItemCount())
            if panel._key(panel.tree.topLevelItem(i))[0] == "layer"]


def test_names_sort_with_their_numbers_read_as_numbers(win):
    from PySide6.QtCore import QSettings
    from views.layers_panel import LayersPanel, natural_key
    QSettings().remove(LayersPanel.SORT_KEY)
    assert natural_key("Capa 2") < natural_key("Capa 10")
    assert natural_key("planta 9") < natural_key("Planta 12")
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("Capa 10"), Layer("Capa 2"), Layer("Capa 1"),
                     Layer("Alzado")]
    panel.refresh()
    assert _layer_rows(panel) == [DEFAULT_LAYER, "Alzado", "Capa 1",
                                  "Capa 2", "Capa 10"]


def test_sort_by_order_created_from_the_options_menu(win):
    from PySide6.QtCore import QSettings
    from views.layers_panel import LayersPanel
    QSettings().remove(LayersPanel.SORT_KEY)
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("Zeta"), Layer("Alfa"), Layer("Media")]
    panel.refresh()
    assert _layer_rows(panel)[1:] == ["Alfa", "Media", "Zeta"]
    panel._fill_more_menu()
    menu = panel.more_menu                           # the panel's options
    next(a for a in menu.actions() if a.text() == "Sort by order created").trigger()
    assert panel.sort_by == "created"
    assert _layer_rows(panel) == [DEFAULT_LAYER, "Zeta", "Alfa", "Media"]
    panel.add_layer(edit=False)                      # a new one: at the end
    assert _layer_rows(panel)[-1] == "Layer 1"
    panel._fill_more_menu()
    checks = {a.text(): a.isChecked() for a in panel.more_menu.actions()}
    assert checks["Sort by order created"] and not checks["Sort by name"]
    assert "Sort by name" not in [a.text()
                                  for a in panel._columns_menu().actions()]
    panel.set_sort_by("name")
    QSettings().remove(LayersPanel.SORT_KEY)


def _top_rows(panel):
    return [panel.tree.topLevelItem(i).text(0)
            for i in range(panel.tree.topLevelItemCount())]


def _child_rows(panel, folder):
    item = panel.item_for(folder, "folder")
    return [item.child(i).text(0) for i in range(item.childCount())]


@pytest.mark.parametrize("how, label, attr", [
    ("visible", "Sort by visible", "visible"),
    ("locked", "Sort by locked", "locked"),
])
def test_sort_by_visible_or_locked_from_the_options_menu(win, how, label,
                                                         attr):
    from PySide6.QtCore import QSettings
    from views.layers_panel import LayersPanel
    QSettings().remove(LayersPanel.SORT_KEY)
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    # Visible (or locked) layers go first, the others after.
    scene.layers += [Layer("Capa 10", **{attr: True}),
                     Layer("Capa 2", **{attr: False}),
                     Layer("Capa 1", **{attr: True}),
                     Layer("Alzado", **{attr: False})]
    panel.refresh()
    assert _layer_rows(panel)[1:] == ["Alzado", "Capa 1", "Capa 2",
                                      "Capa 10"]
    panel._fill_more_menu()
    next(a for a in panel.more_menu.actions() if a.text() == label).trigger()
    assert panel.sort_by == how
    # Layer 0 still first; then that state's layers, each group by name
    # with its numbers read as numbers.
    assert _layer_rows(panel) == [DEFAULT_LAYER, "Capa 1", "Capa 10",
                                  "Alzado", "Capa 2"]
    panel._fill_more_menu()
    checks = {a.text(): a.isChecked() for a in panel.more_menu.actions()}
    assert checks[label]
    assert sum(checks[t] for t in ("Sort by name", "Sort by order created",
                                   "Sort by visible", "Sort by locked")) == 1
    assert str(QSettings().value(LayersPanel.SORT_KEY)) == how   # remembered
    panel.set_sort_by("name")
    QSettings().remove(LayersPanel.SORT_KEY)


def test_sort_by_visible_moves_a_row_when_its_eye_is_clicked(win):
    from PySide6.QtCore import QSettings
    from views.layers_panel import COL_EYE, COL_LOCK, LayersPanel
    QSettings().remove(LayersPanel.SORT_KEY)
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A"), Layer("B"), Layer("C")]
    panel.set_sort_by("visible")
    assert _layer_rows(panel)[1:] == ["A", "B", "C"]
    panel.click_cell(("layer", "A"), COL_EYE)       # hide A: it goes down
    assert not scene.layer("A").visible
    assert _layer_rows(panel)[1:] == ["B", "C", "A"]
    panel.click_cell(("layer", "A"), COL_EYE)       # shown again: back up
    assert _layer_rows(panel)[1:] == ["A", "B", "C"]
    panel.set_sort_by("locked")
    panel.click_cell(("layer", "C"), COL_LOCK)      # locked: it goes up
    assert _layer_rows(panel)[1:] == ["C", "A", "B"]
    panel.set_sort_by("name")
    QSettings().remove(LayersPanel.SORT_KEY)


def test_sort_by_visible_keeps_folders_first_and_sorts_inside_them(win):
    from PySide6.QtCore import QSettings
    from views.layers_panel import LayersPanel
    QSettings().remove(LayersPanel.SORT_KEY)
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    hidden = L.add_folder(scene, "Apagada")
    shown = L.add_folder(scene, "Zona")
    scene.folder_visibility[hidden] = False    # the folder itself off
    scene.layers += [Layer("Muro", folder=hidden, visible=False),
                     Layer("Losa", folder=shown, visible=False),
                     Layer("Pilar", folder=shown),
                     Layer("Suelta", visible=False), Layer("Viga")]
    panel.set_sort_by("visible")
    # Layer 0, the folders (the one switched on first), the layers.
    assert _top_rows(panel) == [DEFAULT_LAYER, shown, hidden, "Viga",
                                "Suelta"]
    assert _child_rows(panel, shown) == ["Pilar", "Losa"]
    panel.set_sort_by("name")
    assert _top_rows(panel) == [DEFAULT_LAYER, hidden, shown, "Suelta",
                                "Viga"]
    assert _child_rows(panel, shown) == ["Losa", "Pilar"]
    QSettings().remove(LayersPanel.SORT_KEY)


def test_a_new_layer_goes_into_the_folder_of_the_selected_layer(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    arq = L.add_folder(scene, "Arq")
    scene.layers += [Layer("Muros", folder=arq), Layer("Suelta")]
    panel.refresh()
    panel.tree.setCurrentItem(panel.item_for("Muros"))
    name = panel.add_layer(edit=False)
    assert scene.layer(name).folder == arq
    panel.tree.setCurrentItem(panel.item_for("Muros"))
    sub = panel.add_folder(edit=False)                 # a folder too
    assert L.folder_parent(scene, sub) == arq
    panel.refresh()
    panel.tree.setCurrentItem(panel.item_for("Suelta"))
    top = panel.add_layer(edit=False)
    assert scene.layer(top).folder is None           # a loose one: the top


def test_default_columns_puts_the_daily_set_back(win):
    from PySide6.QtCore import QSettings
    from views.layers_panel import (COL_COLOR, COL_COUNT, COL_CUT,
                                    COL_LINETYPE, COL_OPACITY, COL_PRINT,
                                    COL_WEIGHT, LayersPanel)
    panel = win.layers_dock.panel
    QSettings().setValue(LayersPanel.HIDDEN_COLUMNS_KEY,
                         str(COL_COLOR))                # someone's own choice
    panel._apply_column_choice()
    menu = panel._columns_menu()
    next(a for a in menu.actions() if a.text() == "Default columns").trigger()
    hidden = panel.hidden_columns()
    assert hidden == {COL_WEIGHT, COL_PRINT, COL_OPACITY, COL_COUNT, COL_CUT}
    for col in (COL_COLOR, COL_LINETYPE):
        assert not panel.tree.isColumnHidden(col)
    assert panel.tree.isColumnHidden(COL_WEIGHT)
    assert panel.columns_button.text() == "+5"


def test_the_line_weight_column_starts_off(win):
    """Nothing chosen yet: the weight is set once per layer and only shows
    on sheets, so it waits in the columns menu; the line type stays."""
    from PySide6.QtCore import QSettings
    from views.layers_panel import COL_LINETYPE, COL_WEIGHT, LayersPanel
    panel = win.layers_dock.panel
    QSettings().remove(LayersPanel.HIDDEN_COLUMNS_KEY)
    panel._apply_column_choice()
    assert COL_WEIGHT in panel.hidden_columns()
    assert panel.tree.isColumnHidden(COL_WEIGHT)
    assert not panel.tree.isColumnHidden(COL_LINETYPE)
    menu = panel._columns_menu()
    act = next(a for a in menu.actions() if a.text() == "Line weight")
    assert not act.isChecked()
    act.setChecked(True)                             # one tick brings it back
    assert not panel.tree.isColumnHidden(COL_WEIGHT)


def test_the_columns_menu_reads_show_all_switches_then_defaults(win):
    panel = win.layers_dock.panel
    menu = panel._columns_menu()
    texts = [a.text() for a in menu.actions() if a.text()]
    assert texts[0] == "Show all columns"
    assert texts.index("Default columns") == texts.index(
        "Entities on the layer") + 1                # after the last switch
    assert texts[-1] == "Default columns"           # nothing after it


# ---- DXF: every layer with its colour ---------------------------------------


def test_an_edit_of_the_model_does_not_rebuild_the_rows(win):
    """The rows are rebuilt when what they show changes, not on every edit:
    a rebuild on every change of the scene was 9 ms of each edit on plaza
    (release_check 0.5.11). The counts still follow the edit."""
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("Muros")]
    win.layers_dock.show()
    win.layers_dock.raise_()
    _app.processEvents()
    panel.refresh()
    first = panel.tree.topLevelItem(0)
    m = Mesh()
    a, b = QVector3D(0, 0, 0), QVector3D(1, 0, 0)
    e = m.add_edge(a, b)
    e.layer = "Muros"
    g = Group(m, name="g")
    scene.groups.append(g)
    scene.version += 1
    win.viewport.notify_scene_changed()
    assert panel.tree.topLevelItem(0) is first          # not rebuilt
    panel.update_counts()                               # the timer's call
    assert panel._counts.get("Muros") == 1
    swatch = panel.item_for("Muros").icon(4).cacheKey()
    scene.layer("Muros").color = (1.0, 0.0, 0.0)
    win.viewport.notify_scene_changed()
    _app.processEvents()                        # the deferred turn
    # A layer changed: its row is repainted in place, not rebuilt.
    assert panel.tree.topLevelItem(0) is first
    assert panel.item_for("Muros").icon(4).cacheKey() != swatch


def test_behind_another_tab_the_panel_waits_until_shown(win):
    panel = win.layers_dock.panel
    scene = win.viewport.scene
    win.layers_dock.hide()
    _app.processEvents()
    assert not panel.isVisible()
    scene.layers += [Layer("Nueva")]
    win.viewport.notify_scene_changed()
    assert "Nueva" not in _shown(panel)                 # not rebuilt hidden
    win.layers_dock.show()
    _app.processEvents()
    assert "Nueva" in _shown(panel)
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


def test_a_new_layer_keeps_its_name_editor_open(win):
    """The row whose name is being typed must not be rebuilt under the
    editor: the panel announces the change first, then rebuilds — the
    deferred rebuild it armed is the one it just did."""
    from PySide6.QtWidgets import QAbstractItemView
    panel = win.layers_dock.panel
    win.layers_dock.raise_()
    _app.processEvents()
    name = panel.add_layer()
    _app.processEvents()
    assert panel.tree.state() == QAbstractItemView.EditingState
    assert panel.tree.currentItem().text(0) == name


def test_the_row_icons_follow_a_theme_switched_live(win):
    """Light to dark while the app runs: the eye, padlock and target of the
    rows — and the header's — are drawn again at once in the new ink, not
    left black until the list happens to rebuild."""
    from PySide6.QtGui import QColor
    from views.layers_panel import COL_EYE, COL_LOCK
    from views.theme import dark_palette, light_palette

    def darkest(icon):
        img = icon.pixmap(18, 18).toImage()
        tones = [QColor(img.pixel(x, y)) for x in range(18) for y in range(18)
                 if QColor.fromRgba(img.pixel(x, y)).alpha() > 200]
        return max(c.lightness() for c in tones) if tones else None

    panel = win.layers_dock.panel
    scene = win.viewport.scene
    scene.layers += [Layer("A")]
    _app.setPalette(light_palette())
    _app.processEvents()
    panel.refresh()
    item = panel.item_for("A")
    light = darkest(item.icon(COL_EYE))
    try:
        _app.setPalette(dark_palette())
        _app.processEvents()
        item = panel.item_for("A")
        assert darkest(item.icon(COL_EYE)) > light + 60      # light ink now
        assert darkest(item.icon(COL_LOCK)) > light + 60
        head = panel.tree.headerItem()
        assert darkest(head.icon(COL_EYE)) > light + 60
    finally:
        _app.setPalette(light_palette())
        _app.processEvents()


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


def test_first_construction_tag_keeps_existing_folder_rows(win):
    scene = win.viewport.scene
    folder = L.add_folder(scene, "Zone")
    scene.layers.extend([Layer("Walls", folder=folder), Layer("Floors", folder=folder)])
    panel = win.layers_dock.panel
    panel.refresh()
    old = {panel._key(item): item for item in panel._all_items()}
    guide = Layer("Guides")
    scene.layers.append(guide)
    panel._refresh_after_change()
    rows = {panel._key(item): item for item in panel._all_items()}
    assert all(rows[k] is item for k, item in old.items())
    assert any(k[1] == guide.key for k in rows)

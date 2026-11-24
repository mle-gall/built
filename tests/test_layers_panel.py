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
from tests.shell_isolation import restore_shell_patches  # noqa: F401
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


def test_active_pencil_uses_the_interface_accent(win):
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

    blue = theme.accent_color()
    assert has(item.icon(1), blue)                       # COL_ACTIVE
    assert has(panel.tree.headerItem().icon(1), blue)


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


def test_the_delete_key_deletes_the_current_row(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers += [Layer("A"), Layer("B")]
    action(native_host, "deletetag", "preview-tag:A")
    assert scene.layer("A") is None and scene.layer("B") is not None
    assert native_host.viewport.history.undo() and scene.layer("A") is not None
    action(native_host, "deletetag", "preview-tag:" + DEFAULT_LAYER)
    assert scene.layer(DEFAULT_LAYER) is not None and not native_host.viewport.history.undo_stack


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


# ---- Layers not printed ------------------------------------------------------


# ---- Order and where new rows go -------------------------------------------

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


# ---- DXF: every layer with its colour ---------------------------------------


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

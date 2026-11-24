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


def test_a_rows_target_isolates_and_a_second_click_brings_back_what_it_hid(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B"), Layer("C"), Layer("Off", visible=False)])
    face = _square(scene.mesh)
    assign_layer(face, "B")
    action(native_host, "tagisolate", "preview-tag:A")
    assert [ly.key for ly in scene.layers if ly.visible] == [DEFAULT_LAYER, "A"]
    assert not scene.entity_visible(face)
    action(native_host, "tagisolate", "preview-tag:B")
    assert scene.entity_visible(face) and not scene.layer("A").visible
    action(native_host, "tagisolate", "preview-tag:B")
    assert all(scene.layer(k).visible for k in ("A", "B", "C"))
    assert not scene.layer("Off").visible


def test_the_target_acts_on_every_selected_row_and_on_folders(native_host):
    import json
    from tests.native_tree import action
    scene = native_host.viewport.scene
    folder = L.add_folder(scene, "Arq")
    inner = L.add_folder(scene, "Inner", parent=folder)
    scene.layers.extend([Layer("A"), Layer("B"), Layer("C", folder=folder), Layer("D", folder=inner)])
    def isolate(*rows):
        action(native_host, "isolatetags", json.dumps(rows))
    isolate("preview-tag:A", "preview-tag:B")
    assert {ly.key for ly in scene.layers if ly.visible} == {DEFAULT_LAYER, "A", "B"}
    isolate("preview-folder:" + folder)
    assert {ly.key for ly in scene.layers if ly.visible} == {DEFAULT_LAYER, "C", "D"}
    assert native_host.viewport.history.undo()
    assert scene.layer("A").visible and not scene.layer("D").visible
    assert native_host.viewport.history.redo()
    isolate("preview-folder:" + folder)
    assert all(ly.visible for ly in scene.layers)


def test_the_target_header_ends_the_isolation(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B")])
    action(native_host, "tagisolate", "preview-tag:A")
    assert not scene.layer("B").visible
    action(native_host, "isolatetags", "[]")
    assert scene.layer("B").visible


def test_merge_tag_moves_carriers_in_one_step_and_rejects_a_replaced_target(native_host, monkeypatch):
    from tests.native_tree import action
    from views import chrome_forms
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B")])
    face = _square(scene.mesh)
    assign_layer(face, "A")
    callbacks = []
    monkeypatch.setattr(chrome_forms, "ask_choice", lambda parent, title, label, items, at, done: callbacks.append(done))
    action(native_host, "mergetag", "preview-tag:A")
    callbacks.pop()("B")
    assert scene.layer("A") is None and L.layer_of(face) == "B"
    assert native_host.viewport.history.undo()
    assert L.layer_of(face) == "A"
    action(native_host, "mergetag", "preview-tag:A")
    scene.layers.remove(scene.layer("B"))
    scene.layers.append(Layer("B"))
    callbacks.pop()("B")
    assert scene.layer("A") is not None and L.layer_of(face) == "A"


def test_expand_and_collapse_all_reach_nested_tag_folders():
    from tests.test_project_drawing_filters import Project
    project = Project()
    outer = L.add_folder(project.scene, "A")
    inner = L.add_folder(project.scene, "B", parent=outer)
    project.scene.layers.append(Layer("Walls", folder=inner))
    project.action("lens", "1")
    expanded = project.action("expandall", "")
    assert expanded["preview-folder:" + outer][1][8] == expanded["preview-folder:" + inner][1][8] == 1
    assert "preview-tag:Walls" in expanded
    folded = project.action("collapseall", "")
    assert not folded["preview-folder:" + outer][1][8] and "preview-tag:Walls" not in folded


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


def test_drawing_model_colours_belong_to_the_style(native_host):
    from core.style import Style
    from tests.native_tree import action
    scene = native_host.viewport.scene
    for on in (True, False):
        action(native_host, "tagcolours", str(int(on)))
        assert Style.from_dict(scene.display_style.to_dict()).color_by_layer == on
    scene.worksession_read_only = True
    action(native_host, "tagcolours", "1")
    assert not scene.display_style.color_by_layer


def test_tag_menu_changes_saved_scene_visibility_and_restores_it(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B")])
    first, second = SavedView("Plan", hidden_layers=["B"]), SavedView("Roof")
    scene.saved_views.extend([first, second])
    action(native_host, "scenetaghide", "preview-tag:A")
    assert list(first.hidden_layers) == ["B", "A"] and list(second.hidden_layers) == ["A"]
    action(native_host, "scenetagshow", "preview-tag:A")
    assert list(first.hidden_layers) == ["B"] and not second.hidden_layers
    assert native_host.viewport.history.undo() and list(second.hidden_layers) == ["A"]
    assert native_host.viewport.history.undo() and list(first.hidden_layers) == ["B"]


def test_template_export_and_import_in_drawing(native_host, tmp_path, monkeypatch):
    from tests.native_tree import action
    from views.filedialogs import file_dialogs
    scene = native_host.viewport.scene
    folder = L.add_folder(scene, "Oficina")
    tag = Layer("Ejes", folder=folder, linetype="dashdot")
    scene.layers.append(tag)
    path = tmp_path / "plantilla.json"
    for picker in ("getSaveFileName", "getOpenFileName"):
        monkeypatch.setattr(file_dialogs, picker, lambda *a: (str(path), ""))
    action(native_host, "exporttags", "")
    scene.layers[:] = [Layer(DEFAULT_LAYER)]
    action(native_host, "importtags", "")
    restored = scene.layer("Ejes")
    assert (restored.uid, restored.folder, restored.linetype) == (tag.uid, folder, "dashdot")
    restored.visible = False
    action(native_host, "importtags", "")
    assert not restored.visible and scene.layer("Ejes") is restored

    # Bad input cannot leave a newly imported folder behind.
    path.write_text('{"ingetrazo_layers":1,"folders":["Partial"],"layers":[],"folder_uids":[1]}')
    action(native_host, "importtags", "")
    assert "Partial" not in L.folders_of(scene)
    scene.worksession_read_only = True
    path.write_text('{"ingetrazo_layers":1,"layers":[{"name":"Forbidden"}]}')
    action(native_host, "importtags", "")
    assert scene.layer("Forbidden") is None


def test_right_click_hides_and_isolates_the_clicked_tag(native_host):
    from tests.test_layer_assignment import tag_form
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B")])
    face = _square(scene.mesh)
    assign_layer(face, "A")
    scene.select([face])
    for verb in ["Isolate selected tags", "Hide selected tags"]:
        form = tag_form(native_host)
        form.receive("item", str(form.field("item").keys().index(verb)))
        form.press("ok")
        assert not scene.layer("B").visible
    assert not scene.layer("A").visible and not scene.selection


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


def test_drawing_rows_show_the_document_line_type_and_weight():
    from tests.test_project_drawing_filters import Project
    p = Project(Layer("A", linetype="dashed", lineweight=.35), Layer("B"))
    p.action("lens", "1")
    text = {t[0]: t for t, _ in p.view.renderer.calls["ui_set_tree"][0]}
    assert text["preview-tag:A"][6:8] == ["dashed", "0.35"]
    assert text["preview-tag:B"][6] == "continuous"
    assert not p.scene.layer("B").lineweight


def test_selected_tags_take_the_same_line_and_projection_weight():
    from tests.test_project_drawing_filters import Project, drawing_edit
    p = Project(*(Layer(key) for key in "ABC"))
    drawing_edit(p, "drawingpatch", [["A","B"], {"linetype":"dashdot", "weight":"0.50"}])
    assert all((p.scene.layer(k).linetype, p.scene.layer(k).lineweight) == ("dashdot", .5) for k in "AB")
    assert p.scene.layer("C").linetype == "continuous"
    drawing_edit(p, "drawingpatch", [["A","B"], {"weight":"auto"}])
    assert all(p.scene.layer(k).lineweight is None for k in "AB")


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


def test_the_layer_and_folder_of_the_model_selection_stand_out():
    from tests.test_project_drawing_filters import Project
    project = Project(Layer("Walls", folder="Building"))
    group = Group(Mesh(), name="Wall")
    group.layer = "Walls"
    project.scene.groups.append(group)
    project.action("lens", "1")
    project.action("expandall", "")
    for selected in ([group], []):
        project.scene.select(selected)
        rows = project.rows()
        for key in ("preview-tag:Walls", "preview-folder:Building"):
            assert rows[key][1][9] == len(selected)


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

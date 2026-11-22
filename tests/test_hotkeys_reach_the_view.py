"""Hotkeys keep reaching the tool, wherever a click or a panel left the focus.

«Hotkeys sometimes stop working», on the Metal view. Letters are window
shortcuts and fired from most places; what died were the keys the view
reads itself (the arrows that lock an axis or an edge, a typed value and
Enter, Delete, Tab), and the line in progress with them. The causes, each
reproduced here in an offscreen main window with the Metal view, real key
and mouse events:

* Tab in the view: Qt's focus chain moved the keyboard to a tray button;
* ``viewport.setFocus()`` — Orbit / Pan, a scene picked in its list, the
  camera panel, a project switch — gave the focus to the GL viewport, hidden
  under Metal: a no-op, the keys stayed in the list or on the button;
* focus left on a button without a click: at start-up (the Tab chain gave it
  to the project tabs' button) or when a tray holding it was hidden;
* the Metal view cancelled the tool whenever it lost the focus, so a click
  in a tray mid-line reset the line (GL never did).

Checked and not a cause here: a tray floated out (its window still fires the
shortcuts), a popup menu or a window-modal progress dialog once closed (the
window gets its focus back on activation), the measurements box (a label,
never focused), a text field (it keeps its letters, as it must).
"""
import pytest
from PySide6.QtCore import QPointF, Qt, QTimer
from PySide6.QtGui import QVector3D as V
from PySide6.QtTest import QTest
from PySide6.QtWidgets import (QAbstractItemView, QApplication, QLineEdit,
                               QMenu, QProgressDialog, QPushButton)

from tools.base import ToolContext


@pytest.fixture
def win(monkeypatch, tmp_path, request):
    monkeypatch.setenv("INGETRAZO_WGPU", getattr(request, "param", "1"))
    monkeypatch.setenv("INGETRAZO_AI_PORT", "0")
    from core import autosave, renderer_settings
    monkeypatch.setattr(renderer_settings, "capability",
                        lambda: renderer_settings.Capability(True))
    monkeypatch.setattr(autosave, "autosave_dir", lambda: tmp_path)
    monkeypatch.setattr(autosave, "pending", lambda *a, **k: None)
    from views.wgpu_view import WgpuView
    # No Metal device offscreen: the view routes input without drawing.
    monkeypatch.setattr(WgpuView, "_upload", lambda self, force=False: None)
    from views.main_window import MainWindow
    w = MainWindow()
    w.resize(1200, 800)
    w.show()
    w.activateWindow()
    settle()
    yield w
    w._saved_version = w.viewport.scene.version
    w.close()
    settle()


def settle():
    for _ in range(3):
        QApplication.processEvents()


def focus():
    return QApplication.focusWidget()


def tool_name(win):
    tool = win.viewport.active_tool
    return type(tool).__name__ if tool is not None else None


def tree(dock):
    return next(v for v in dock.findChildren(QAbstractItemView) if v.isVisible())


def start_line(win):
    """The Line, its first point placed (the view's own click needs a
    renderer to pick; the tool's click is the same)."""
    QTest.keyClick(focus() or win, Qt.Key_L)
    line = win.viewport.active_tool
    assert type(line).__name__ == "LineTool"
    display = win._doc.stand_in
    display.tools.sync()
    line.on_click(ToolContext(display.tools, V(0, 0, 0), QPointF(), Qt.NoModifier, None))
    assert line.start_point is not None
    return line


def test_the_view_has_the_keyboard_at_start(win):
    assert focus() is win._doc.stand_in, focus()


def test_tab_stays_with_the_tool(win):
    display = win._doc.stand_in
    for modifiers in (Qt.NoModifier, Qt.ShiftModifier):
        display.setFocus()
        QTest.keyClick(display, Qt.Key_Tab, modifiers)
        settle()
        assert focus() is display, focus()


def test_viewport_set_focus_reaches_the_metal_view(win):
    display = win._doc.stand_in
    scenes = tree(win.scenes_dock)
    scenes.setFocus()
    settle()
    assert focus() is scenes
    win.viewport.setFocus()             # what a scene picked in its list does
    assert focus() is display
    scenes.setFocus()
    QTest.keyClick(scenes, Qt.Key_O)    # Orbit, from its key
    assert win.viewport.nav_mode == "orbit"
    assert focus() is display, focus()


def test_a_tray_hidden_with_the_focus_hands_it_back_to_the_view(win):
    layers = tree(win.layers_dock)
    layers.setFocus()
    settle()
    win.layers_dock.hide()
    settle()
    assert focus() is win._doc.stand_in, focus()


def test_the_line_survives_the_focus_going_to_a_tray_and_back(win):
    display = win._doc.stand_in
    display.setFocus()
    line = start_line(win)
    assert focus() is display
    layers = tree(win.layers_dock)
    layers.setFocus()
    settle()
    display.setFocus()
    settle()
    assert line.start_point is not None and line.start_point == V(0, 0, 0)


def test_orbit_from_its_key_mid_line_then_line_again_goes_on(win):
    """Item 2 through the window: O, then L, keep the first point."""
    line = start_line(win)
    QTest.keyClick(focus(), Qt.Key_O)
    assert win.viewport.nav_mode == "orbit"
    QTest.keyClick(focus(), Qt.Key_H)               # Pan
    QTest.keyClick(focus(), Qt.Key_L)
    assert win.viewport.active_tool is line
    assert line.start_point == V(0, 0, 0)
    QTest.keyClick(focus(), Qt.Key_R)               # another tool: reset
    assert line.start_point is None


def test_the_arrows_reach_the_tool_after_a_tray_was_hidden(win):
    start_line(win)
    layers = tree(win.layers_dock)
    layers.setFocus()
    settle()
    win.layers_dock.hide()
    settle()
    QTest.keyClick(focus(), Qt.Key_Right)
    assert win.viewport.axis_lock == "x"


def test_a_click_in_the_view_takes_the_keyboard(win):
    display = win._doc.stand_in
    layers = tree(win.layers_dock)
    layers.setFocus()
    settle()
    QTest.mousePress(display, Qt.LeftButton, Qt.NoModifier, display.rect().center())
    QTest.mouseRelease(display, Qt.LeftButton, Qt.NoModifier, display.rect().center())
    assert focus() is display


def test_a_text_field_keeps_its_letters(win):
    field = next(f for f in win.layers_dock.findChildren(QLineEdit) if f.isVisible())
    field.setFocus()
    settle()
    before = tool_name(win)
    QTest.keyClick(field, Qt.Key_L)
    settle()
    assert field.text() == "l" and tool_name(win) == before
    assert focus() is field


def test_a_clicked_button_still_clicks(win):
    button = QPushButton("Do", win.layers_dock.widget())
    button.setFocusPolicy(Qt.StrongFocus)
    button.show()
    clicks = []
    button.clicked.connect(lambda: clicks.append(1))
    QTest.mouseClick(button, Qt.LeftButton)
    settle()
    assert clicks == [1]


def test_after_a_menu_or_a_progress_dialog_the_view_has_the_keys(win):
    display = win._doc.stand_in
    display.setFocus()
    menu = QMenu(win)
    menu.addAction("x")
    QTimer.singleShot(30, menu.close)
    menu.exec(win.mapToGlobal(win.rect().center()))
    win.activateWindow()                # what the system does on close
    settle()
    assert focus() is display
    bar = QProgressDialog("…", None, 0, 0, win)
    bar.setWindowModality(Qt.WindowModal)
    bar.show()
    settle()
    bar.close()
    win.activateWindow()
    settle()
    assert focus() is display
    QTest.keyClick(focus(), Qt.Key_L)
    assert tool_name(win) == "LineTool"


def test_a_floating_tray_still_fires_the_tool_keys(win):
    dock = win.layers_dock
    dock.setFloating(True)
    dock.show()
    dock.activateWindow()
    layers = tree(dock)
    layers.setFocus()
    settle()
    QTest.keyClick(layers, Qt.Key_R)
    assert tool_name(win) == "RectangleTool"
    dock.setFloating(False)


@pytest.mark.parametrize("win", ["0", "1"], indirect=True)
def test_clean_screen_keeps_tool_shortcuts_and_measurements(win):
    display = win._doc.stand_in or win.viewport
    win._act_clean_screen.setChecked(True)
    settle()
    assert win.statusBar().isVisible()
    assert focus() is display
    QTest.keyClick(display, Qt.Key_R)
    assert tool_name(win) == "RectangleTool"
    QTest.keyClicks(display, "3;4")
    buffer = display.tools.value_buffer if display is not win.viewport else display._value_buffer
    assert buffer == "3;4"
    win._act_clean_screen.setChecked(False)
    QTest.keyClick(display, Qt.Key_L)
    assert tool_name(win) == "LineTool"


@pytest.mark.parametrize("shortcut,entry,count,x", [
    (Qt.Key_L, "3", 1, 3.0),
    (Qt.Key_R, "3;1", 4, 3.0),
    (Qt.Key_C, "3", 24, 3.0),
])
def test_metal_retype_replaces_placed_shape(win, shortcut, entry, count, x):
    display = win._doc.stand_in
    QTest.keyClick(display, shortcut)
    native = display.tools
    native.sync()
    for pt in (V(), V(2, 1 if shortcut == Qt.Key_R else 0, 0)):
        ctx = ToolContext(native, pt, QPointF(), Qt.NoModifier, None)
        native.tool.on_hover(ctx)
        native.tool.on_click(ctx)
    steps = len(native.history.undo_stack)
    QTest.keyClicks(display, entry)
    QTest.keyClick(display, Qt.Key_Return)
    assert len(native.history.undo_stack) == steps
    assert len(native.scene.mesh.edges) == count
    assert max(v.position.x() for v in native.scene.mesh.vertices) == pytest.approx(x)


@pytest.mark.parametrize("win", ["0", "1"], indirect=True)
def test_expression_beats_window_shortcuts_and_draws_its_result(win):
    from PySide6.QtGui import QAction, QKeySequence
    display = win._doc.stand_in or win.viewport
    QTest.keyClick(display, Qt.Key_L)
    target = display.tools if display is not win.viewport else display
    if display is not win.viewport:
        target.sync()
    line = win.viewport.active_tool
    line.on_click(ToolContext(target, V(), QPointF(), Qt.NoModifier, None))
    line.on_hover(ToolContext(target, V(2, 0, 0), QPointF(), Qt.NoModifier, None))

    fired = []
    for operator in "=+-*/()":
        action = QAction(operator, win)
        action.setShortcut(QKeySequence(operator))
        action.triggered.connect(lambda: fired.append(True))
        win.addAction(action)
    entry = "=(1.20m+35cm)*2/2-5cm"
    QTest.keyClicks(display, entry)
    assert not fired
    assert win._vcb_value.text() == entry
    QTest.keyClick(display, Qt.Key_Return)
    assert line.start_point.x() == pytest.approx(1.5)
    assert win.viewport.active_tool is line


@pytest.mark.parametrize("win", ["0", "1"], indirect=True)
def test_zoom_accepts_calculated_lens_and_field_of_view(win):
    display = win._doc.stand_in or win.viewport
    cam = win.viewport.camera
    cam.set_projection("perspective")
    win.viewport.set_nav_mode("zoom")
    QTest.keyClicks(display, "=20mm+1.5cm")
    QTest.keyClick(display, Qt.Key_Return)
    assert cam.focal_mm == pytest.approx(35)
    QTest.keyClicks(display, "=90/2")
    QTest.keyClick(display, Qt.Key_Return)
    assert cam.fov_deg == pytest.approx(45)

"""The Rust app shell's Python half (app_shell/), headless: no surface, so
no renderer is made; the document, tools and key routing run as in the
window. The windowed path is exercised by ``built --script`` (see
docs/rust-app-shell.md)."""
from pathlib import Path

import pytest
from PySide6.QtCore import Qt
from tests.shell_isolation import restore_shell_patches  # noqa: F401  (autouse)

FIXTURE = Path(__file__).parent / "fixtures" / "shell" / "slab-and-cube.igz"


@pytest.fixture
def host(tmp_path):
    from app_shell.host import Host
    h = Host(0, 1280, 800, 2.0, settings=str(tmp_path))
    yield h
    h.close()


def test_opens_a_document_headless(host):
    assert host.open(FIXTURE)
    state = host.state()
    assert (state["faces"], state["edges"]) == (6, 16)
    assert state["errors"] == 0
    assert host.view.renderer is None          # nothing presents headless


def test_borrowed_classes_are_no_widgets(host):
    from PySide6.QtWidgets import QWidget
    assert not isinstance(host.viewport, QWidget)
    assert not isinstance(host.view, QWidget)
    # Viewport's own state came through its borrowed __init__.
    assert host.viewport.history is not None and host.viewport.scene is not None
    assert host.view.source_viewport is host.viewport


def test_tool_shortcut_goes_through_shortcut_override(host):
    host.open(FIXTURE)
    host.window._activate_tool("select")
    host.key(True, Qt.Key_L.value, 0, "l")
    host.key(False, Qt.Key_L.value, 0, "l")
    assert type(host.viewport.active_tool).__name__ == "LineTool"
    assert host.window._tool_actions["line"].isChecked()
    assert host.errors == []


def test_tool_shortcut_restored_in_new_shell(host, tmp_path):
    from PySide6.QtGui import QKeySequence
    from app_shell.host import Host
    from views.shortcuts import save_shortcut

    action = host.window._tool_actions["line"]
    save_shortcut(action, [QKeySequence("F11")])
    host.close()
    reopened = Host(0, 1280, 800, 2.0, settings=str(tmp_path))
    try:
        action = reopened.window._tool_actions["line"]
        assert action.shortcut() == QKeySequence("F11")
        reopened.key(True, Qt.Key_F11.value, 0, "")
        assert type(reopened.viewport.active_tool).__name__ == "LineTool"
        assert not reopened.errors
    finally:
        reopened.close()


def test_undo_from_the_menu(host):
    host.open(FIXTURE)
    from ingetrazo_core import Vec3 as QVector3D
    from core.history import AddFaceCommand
    vp = host.viewport
    before = len(vp.scene.mesh.faces)
    loop = [QVector3D(5, 0, 0), QVector3D(6, 0, 0), QVector3D(6, 1, 0)]
    vp.history.execute(AddFaceCommand(loop))
    assert len(vp.scene.mesh.faces) == before + 1
    host.menu("undo")
    assert len(vp.scene.mesh.faces) == before
    assert host.errors == []


def test_events_answer_the_qt_calls_the_handlers_make():
    from app_shell.events import KeyEvent, MouseEvent, NativeGestureEvent, WheelEvent
    m = MouseEvent("press", 10.5, 20, Qt.LeftButton.value, Qt.LeftButton.value,
                   (Qt.ShiftModifier | Qt.ControlModifier).value)
    assert m.position().x() == 10.5 and m.button() == Qt.LeftButton
    assert m.modifiers() & Qt.ControlModifier and m.buttons() & Qt.LeftButton
    k = KeyEvent(True, Qt.Key_Escape.value)
    assert k.key() == Qt.Key_Escape and not k.isAutoRepeat()
    w = WheelEvent(1, 2, 0, 0, 0, 240)
    assert w.angleDelta().y() == 240 and w.pixelDelta().isNull()
    assert w.device().type().name == "Mouse"
    slide = WheelEvent(1, 2, 0, -30, 0, -60, phase="move", inverted=True)
    from ingetrazo_render import trackpad_nav
    assert trackpad_nav.is_slide(slide)
    g = NativeGestureEvent("zoom", 0.1, 3, 4)
    assert g.gestureType() == Qt.ZoomNativeGesture and g.value() == pytest.approx(0.1)


def test_qt_input_queries_answer_from_the_shell(host):
    """Push/Pull asks QApplication.queryKeyboardModifiers() for its keep-base
    Ctrl; offscreen Qt sees no keyboard, so the shell answers it."""
    from PySide6.QtWidgets import QApplication
    from PySide6.QtGui import QCursor
    host.pointer("move", 30, 40, 0, 0, Qt.ControlModifier.value)
    assert QApplication.queryKeyboardModifiers() & Qt.ControlModifier
    assert (QCursor.pos().x(), QCursor.pos().y()) == (30, 40)
    host.key(True, Qt.Key_Shift.value, Qt.ShiftModifier.value)
    assert QApplication.keyboardModifiers() == Qt.ShiftModifier


def test_rail_matches_classic_groups_and_dispatches(host):
    from unittest.mock import Mock
    from views import wgpu_library, wgpu_ui

    host.open(FIXTURE)
    win, view = host.window, host.view
    wgpu_library.install(win)
    view.renderer = Mock()
    try:
        wgpu_ui._send_tools(view, win)
        groups, = view.renderer.ui_set_tools.call_args.args
        from ingetrazo_render import rail_layout
        expected = [[key for key, _ in row] for row in rail_layout()]
        actions = {key for row in rail_layout() for key, binding in row if binding}
        assert [[entry[0] for entry in group] for group in groups] == expected
        assert set(view._ui_palette_runs) == set(actions)
    finally:
        view.renderer = None
    for group in expected:
        for key in group:
            if key not in actions:
                win._tool_actions[key].trigger()
                assert host.viewport.active_tool is win._tools[key]
    view._ui_palette_runs["eyedropper"]()
    assert win._act_eyedropper.isChecked()
    assert host.viewport.active_tool is win._tools["paint"]
    win.release_eyedropper()
    for key in ("bridge", "make_faces"):
        view._ui_palette_runs[key]()
    assert not host.errors


def test_text3d_place_edit_font_and_undo(host):
    import json
    from unittest.mock import Mock
    from ingetrazo_core import Vec3
    from views import wgpu_ui

    host.open(FIXTURE)
    win, view, vp = host.window, host.view, host.viewport
    view.renderer = Mock()
    view._ui_attached = True

    def apply(family):
        draft = json.loads(view.renderer.ui_font_picker.call_args.args[0])
        draft.update(text="Ab", font=family)
        wgpu_ui._run(win, ["font:" + json.dumps(draft)])

    try:
        win._act_3dtext.trigger()
        apply("Helvetica")
        vp.active_tool.place_at(vp, Vec3(1, 2, 0))
        group = vp.scene.groups[-1]
        assert group.text3d["font"] == "Helvetica"
        assert group.children
        win._on_edit_3d_text(group)
        apply("Georgia")
        assert group.text3d["font"] == "Georgia"
        host.menu("undo")
        assert group.text3d["font"] == "Helvetica"
        host.menu("undo")
        assert not vp.scene.groups
    finally:
        view._ui_attached = False
        view.renderer = None


@pytest.mark.parametrize("tool", ["text", "dimension"])
def test_annotation_font_options(host, tool):
    import json
    from unittest.mock import Mock
    from views import wgpu_ui
    host.open(FIXTURE)
    view, win = host.view, host.window
    win._activate_tool(tool)
    view.renderer = Mock()
    view._ui_attached = True
    try:
        run = next(row[-1] for row in wgpu_ui._options(view) if row[0] == "font")
        run()
        draft = json.loads(view.renderer.ui_font_picker.call_args.args[0])
        assert draft["unit"].strip() == "pt"
        draft.update(font="Georgia", height=12., italic=True)
        wgpu_ui._run(win, ["font:" + json.dumps(draft)])
        style = win.viewport.scene.dimension_style
        assert (style["font_family"], style["font_size"], style["font_italic"]) == ("Georgia", 12, True)
        run()
        wgpu_ui._run(win, ["font:"])
        assert style["font_family"] == "Georgia"
    finally:
        view._ui_attached = False
        view.renderer = None

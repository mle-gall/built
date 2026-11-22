"""The Rust app shell's Python half (app_shell/), headless: no surface, so
no renderer is made; the document, tools and key routing run as in the
window. The windowed path is exercised by ``built --script`` (see
docs/rust-app-shell.md)."""
from pathlib import Path

import pytest
from PySide6.QtCore import Qt

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


def test_undo_from_the_menu(host):
    host.open(FIXTURE)
    from PySide6.QtGui import QVector3D
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
    from views import trackpad_nav
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

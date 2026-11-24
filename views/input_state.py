"""Live input for a tool's own window, including during timer callbacks."""
from PySide6.QtCore import QPoint, Qt
from PySide6.QtGui import QCursor, QGuiApplication
from PySide6.QtWidgets import QApplication


def _window_input(owner):
    host = getattr(owner, "__dict__", {}).get("_host")
    return getattr(host, "input_state", None)


def modifiers(owner, *, query=False):
    state = _window_input(owner)
    if state is None:
        return QApplication.queryKeyboardModifiers() if query else QGuiApplication.keyboardModifiers()
    return Qt.KeyboardModifier(state.modifiers)


def mouse_buttons(owner):
    state = _window_input(owner)
    if state is None:
        return QApplication.mouseButtons()
    return Qt.MouseButton(state.buttons)


def cursor_position(owner):
    state = _window_input(owner)
    if state is None:
        return QCursor.pos()
    x, y = state.position
    return QPoint(int(x), int(y))

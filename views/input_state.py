"""Live input for a tool's own window, including during timer callbacks."""
from core.input_types import QPoint, Qt


def _window_input(owner):
    host = getattr(owner, "__dict__", {}).get("_host")
    if not hasattr(host, "input_state"):
        host = getattr(getattr(owner, "view", None), "__dict__", {}).get("_host")
    return getattr(host, "input_state", None)


def modifiers(owner, *, query=False):
    state = _window_input(owner)
    if state is None:
        from PySide6.QtGui import QGuiApplication
        from PySide6.QtWidgets import QApplication
        return QApplication.queryKeyboardModifiers() if query else QGuiApplication.keyboardModifiers()
    return Qt.KeyboardModifier(state.modifiers)


def mouse_buttons(owner):
    state = _window_input(owner)
    if state is None:
        from PySide6.QtWidgets import QApplication
        return QApplication.mouseButtons()
    return Qt.MouseButton(state.buttons)


def cursor_position(owner):
    state = _window_input(owner)
    if state is None:
        from PySide6.QtGui import QCursor
        return QCursor.pos()
    x, y = state.position
    return QPoint(int(x), int(y))

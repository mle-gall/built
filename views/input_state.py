"""Live input for a tool's own window, including during timer callbacks."""
from core.input_types import Input, PixelPoint


def _window_input(owner):
    host = getattr(owner, "__dict__", {}).get("_host")
    if not hasattr(host, "input_state"):
        host = getattr(getattr(owner, "view", None), "__dict__", {}).get("_host")
    return getattr(host, "input_state", None)


def modifiers(owner, *, query=False):
    state = _window_input(owner)
    return Input.KeyboardModifier(state.modifiers) if state is not None else Input.NoModifier


def mouse_buttons(owner):
    state = _window_input(owner)
    return Input.MouseButton(state.buttons) if state is not None else Input.NoButton


def cursor_position(owner):
    state = _window_input(owner)
    x, y = state.position if state is not None else (0, 0)
    return PixelPoint(int(x), int(y))

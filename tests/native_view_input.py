"""Deliver shell events directly to a view without moving the system pointer."""
from ingetrazo_render import KeyEvent, MouseEvent


def press(view, button, modifiers, point):
    view.mousePressEvent(MouseEvent("press", point.x(), point.y(), button, button, modifiers))


def release(view, button, modifiers, point):
    view.mouseReleaseEvent(MouseEvent("release", point.x(), point.y(), button, 0, modifiers))


def click(view, button, modifiers, point):
    press(view, button, modifiers, point)
    release(view, button, modifiers, point)


def double_click(view, button, modifiers, point):
    view.mouseDoubleClickEvent(MouseEvent("double", point.x(), point.y(), button, button, modifiers))
    release(view, button, modifiers, point)


def key(view, code, text=""):
    view.keyPressEvent(KeyEvent(True, code, 0, text))
    view.keyReleaseEvent(KeyEvent(False, code, 0, text))


def type_text(view, text):
    for char in text:
        key(view, ord(char.upper()), char)

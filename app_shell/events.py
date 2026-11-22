"""Input events as the Qt-era handlers read them, built from winit's.

The handlers in views/ and tools/ call ``event.position()``, ``button()``,
``buttons()``, ``modifiers()``, ``key()``, ``text()``, ``angleDelta()``,
``pixelDelta()``, ``phase()``... These are plain value objects answering
those calls with Qt's value types and enums; no QEvent is constructed (a
QMouseEvent needs a QPointingDevice, which needs a real platform).

Codes cross from Rust already in Qt's numbering (rust/built_app/src/keys.rs):
modifiers as a ``Qt.KeyboardModifier`` mask with ⌘ as Control (Qt's macOS
convention, which the tools rely on), keys as ``Qt.Key`` values.
"""
from __future__ import annotations

from PySide6.QtCore import QEvent, QPoint, QPointF, Qt


def mods(mask: int):
    return Qt.KeyboardModifier(int(mask))


def buttons(mask: int):
    return Qt.MouseButton(int(mask))


class _Event:
    _type = QEvent.None_

    def __init__(self):
        self._accepted = True

    def type(self):
        return self._type

    def accept(self):
        self._accepted = True

    def ignore(self):
        self._accepted = False

    def isAccepted(self):
        return self._accepted

    def setAccepted(self, on):
        self._accepted = bool(on)

    def spontaneous(self):
        return True


class MouseEvent(_Event):
    _TYPES = {"press": QEvent.MouseButtonPress, "release": QEvent.MouseButtonRelease,
              "move": QEvent.MouseMove, "double": QEvent.MouseButtonDblClick}

    def __init__(self, kind, x, y, button=0, held=0, modifiers=0, gx=None, gy=None):
        super().__init__()
        self._type = self._TYPES[kind]
        self._pos = QPointF(float(x), float(y))
        self._global = QPointF(float(x if gx is None else gx), float(y if gy is None else gy))
        self._button = buttons(button)
        self._buttons = buttons(held)
        self._mods = mods(modifiers)

    def position(self):
        return QPointF(self._pos)

    def pos(self):
        return self._pos.toPoint()

    def x(self):
        return int(self._pos.x())

    def y(self):
        return int(self._pos.y())

    def scenePosition(self):
        return QPointF(self._pos)

    def globalPosition(self):
        return QPointF(self._global)

    def globalPos(self):
        return self._global.toPoint()

    def button(self):
        return self._button

    def buttons(self):
        return self._buttons

    def modifiers(self):
        return self._mods

    def source(self):
        return Qt.MouseEventNotSynthesized

    def device(self):
        return None


class ContextMenuEvent(MouseEvent):
    _type = QEvent.ContextMenu

    def __init__(self, x, y, modifiers=0):
        super().__init__("press", x, y, Qt.RightButton.value, Qt.RightButton.value, modifiers)
        self._type = QEvent.ContextMenu

    def reason(self):
        return 1  # QContextMenuEvent.Mouse


class KeyEvent(_Event):
    def __init__(self, pressed, key, modifiers=0, text="", repeat=False):
        super().__init__()
        self._type = QEvent.KeyPress if pressed else QEvent.KeyRelease
        self._key = int(key)
        self._mods = mods(modifiers)
        self._text = text or ""
        self._repeat = bool(repeat)

    def key(self):
        return self._key

    def modifiers(self):
        return self._mods

    def text(self):
        return self._text

    def isAutoRepeat(self):
        return self._repeat

    def count(self):
        return 1

    def nativeModifiers(self):
        return 0

    def nativeScanCode(self):
        return 0

    def keyCombination(self):
        return self._mods | Qt.Key(self._key) if self._key in Qt.Key._value2member_map_ else self._mods

    def matches(self, _seq):
        return False


class ShortcutOverride(KeyEvent):
    def __init__(self, key, modifiers=0, text="", repeat=False):
        super().__init__(True, key, modifiers, text, repeat)
        self._type = QEvent.ShortcutOverride
        self._accepted = False


#: winit's TouchPhase → Qt's ScrollPhase (momentum is not reported by winit).
_PHASES = {"start": Qt.ScrollBegin, "move": Qt.ScrollUpdate, "end": Qt.ScrollEnd,
           "none": Qt.NoScrollPhase, "momentum": Qt.ScrollMomentum}


class WheelEvent(_Event):
    _type = QEvent.Wheel

    def __init__(self, x, y, pixel_dx, pixel_dy, angle_dx, angle_dy, phase="none",
                 modifiers=0, held=0, inverted=False):
        super().__init__()
        self._pos = QPointF(float(x), float(y))
        self._pixel = QPoint(int(round(pixel_dx)), int(round(pixel_dy)))
        self._angle = QPoint(int(round(angle_dx)), int(round(angle_dy)))
        self._phase = _PHASES.get(phase, Qt.NoScrollPhase)
        self._mods = mods(modifiers)
        self._buttons = buttons(held)
        self._inverted = bool(inverted)
        # A notch (no pixel delta) is a mouse wheel; a pixel delta a trackpad.
        self._device = _Device(mouse=self._pixel.isNull())

    def position(self):
        return QPointF(self._pos)

    def globalPosition(self):
        return QPointF(self._pos)

    def pixelDelta(self):
        return QPoint(self._pixel)

    def angleDelta(self):
        return QPoint(self._angle)

    def phase(self):
        return self._phase

    def modifiers(self):
        return self._mods

    def buttons(self):
        return self._buttons

    def inverted(self):
        return self._inverted

    def source(self):
        return Qt.MouseEventNotSynthesized

    def device(self):
        return self._device

    def pointingDevice(self):
        return self._device


_GESTURES = {"zoom": Qt.ZoomNativeGesture, "rotate": Qt.RotateNativeGesture,
             "smart_zoom": Qt.SmartZoomNativeGesture, "pan": Qt.PanNativeGesture,
             "begin": Qt.BeginNativeGesture, "end": Qt.EndNativeGesture,
             "swipe": Qt.SwipeNativeGesture}


class _Device:
    """``event.device().type()``: trackpad (TouchPad) or touch screen."""

    def __init__(self, touch_screen=False, mouse=False):
        from PySide6.QtGui import QInputDevice
        self._type = (QInputDevice.DeviceType.Mouse if mouse
                      else QInputDevice.DeviceType.TouchScreen if touch_screen
                      else QInputDevice.DeviceType.TouchPad)

    def type(self):
        return self._type

    def systemId(self):
        return 0

    def name(self):
        return "mouse" if self._type.name == "Mouse" else "trackpad"


class NativeGestureEvent(_Event):
    _type = QEvent.NativeGesture

    def __init__(self, kind, value, x, y, delta_x=0.0, delta_y=0.0, touch_screen=False, modifiers=0):
        super().__init__()
        self._kind = _GESTURES[kind]
        self._value = float(value)
        self._pos = QPointF(float(x), float(y))
        self._delta = QPointF(float(delta_x), float(delta_y))
        self._device = _Device(touch_screen)
        self._mods = mods(modifiers)

    def gestureType(self):
        return self._kind

    def value(self):
        return self._value

    def position(self):
        return QPointF(self._pos)

    def globalPosition(self):
        return QPointF(self._pos)

    def delta(self):
        return QPointF(self._delta)

    def device(self):
        return self._device

    def pointingDevice(self):
        return self._device

    def fingerCount(self):
        return 2

    def modifiers(self):
        return self._mods

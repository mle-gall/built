# SPDX-License-Identifier: GPL-3.0-or-later
"""GPU tests without a window or a toolkit (render-glue lane).

``renderer(w, h)`` is the release renderer drawing into a bare Core
Animation layer (``ingetrazo_render.headless_layer``, ``platform="layer"``),
as it draws into the shell's window. ``HeadlessView`` is the shell's
resident view (``MetalView`` over ``WidgetShim``, as
``ingetrazo_render.shell_views.ShellView``) with such a layer for its surface and timers
of its own; it replaces the hidden Qt ``WgpuView``. Drive its timers with
``view.run_timers()``; send it input with ``mouse``, ``key`` and ``wheel``."""
from __future__ import annotations

import importlib.util

import pytest

HAS_RENDERER = importlib.util.find_spec("ingetrazo_render") is not None


def renderer(width: int = 64, height: int = 64, scale: float = 1.0):
    """A ``Renderer`` on a headless layer of ``width`` × ``height`` points."""
    from ingetrazo_render import Renderer, headless_layer
    return Renderer(headless_layer(width, height), width, height, scale, platform="layer")


def _view_class():
    from views.view_host import WidgetShim
    from views.wgpu_view import MetalView

    class HeadlessView(MetalView, WidgetShim):
        """The resident view on a headless layer: what the shell's view is,
        less its window. Without ``present=True`` it makes no renderer, as
        the hidden Qt view did offscreen (the pack, picking and tool
        facades still run)."""

        _shim_visible = True

        def __init__(self, width=640, height=480, scale=1.0, *, present=False):
            from types import SimpleNamespace
            from ingetrazo_core import WindowInput
            from ingetrazo_render import WindowTimers
            self._present = present
            timers = WindowTimers()
            # The services tests/controller_surface.py gives a document, so
            # the bases find their host from the start.
            host = SimpleNamespace(width=width, height=height, scale=scale, window=None,
                                   request_frame=lambda: None, cursor_changed=lambda cursor: None,
                                   timers=timers, input_state=WindowInput())
            self.__dict__["_host"] = host
            MetalView.__init__(self, timers=timers)
            self._host = host           # WidgetShim's own start cleared it

        def _window_handles(self):
            from ingetrazo_render import headless_layer
            return headless_layer(self.width(), self.height()), {"platform": "layer"}

        def _can_create_surface(self):
            return self._present

        def isVisible(self):
            return self._present

        def resize(self, width, height=None):
            if height is None:
                width, height = width.width(), width.height()
            self._host.width, self._host.height = int(width), int(height)
            self._finish_resize()

        def run_timers(self):
            """Run the view's due timers (the shell's loop does this)."""
            self._timer_queue.run_due()

        def close(self):
            self.closeEvent(None)
            self._host.timers.cancel_context(self)
            return True

    return HeadlessView


def __getattr__(name):
    if name == "HeadlessView":
        return _view_class()
    raise AttributeError(name)


def pixel(image, x, y):
    """``(r, g, b, a)`` at ``(x, y)`` of a renderer image (``SurveyRaster``:
    a capture, ``render_image``, ``rasterize``)."""
    import numpy as np
    w, h = image.width, image.height
    return tuple(int(v) for v in np.frombuffer(image.rgba(), np.uint8).reshape(h, w, 4)[int(y), int(x)])


def _code(value) -> int:
    """A Qt enum or flag, or the native code, as the int the shell sends."""
    return int(getattr(value, "value", value) or 0)


_MOUSE = {"press": "mousePressEvent", "release": "mouseReleaseEvent",
          "move": "mouseMoveEvent", "double": "mouseDoubleClickEvent"}


def mouse(view, kind, x, y, button=0, held=0, modifiers=0, timestamp=0):
    """A pointer event as the shell delivers it (ingetrazo_render.Host.pointer):
    ``kind`` press, release, move or double, in logical pixels, at
    ``timestamp`` ms. Returns the handler's answer."""
    from ingetrazo_render import MouseEvent
    ev = MouseEvent(kind, float(x), float(y), _code(button), _code(held), _code(modifiers),
                    timestamp=int(timestamp))
    return getattr(view, _MOUSE[kind])(ev)


def click(view, x, y, button=1, modifiers=0):
    mouse(view, "press", x, y, button, button, modifiers)
    return mouse(view, "release", x, y, button, 0, modifiers)


def key(view, pressed, code, modifiers=0, text=None):
    """A key press or release as the shell delivers it: the shortcut check
    through ``event`` first, then the handler, as the window does."""
    from ingetrazo_render import KeyEvent, ShortcutOverride
    if pressed:
        view.event(ShortcutOverride(_code(code), _code(modifiers), text))
    ev = KeyEvent(bool(pressed), _code(code), _code(modifiers), text)
    return (view.keyPressEvent if pressed else view.keyReleaseEvent)(ev)


def tap(view, code, modifiers=0, text=None):
    key(view, True, code, modifiers, text)
    return key(view, False, code, modifiers, text)


def wheel(view, x, y, angle_dy, modifiers=0, pixel_dy=0.0, phase="none"):
    from ingetrazo_render import WheelEvent
    ev = WheelEvent(float(x), float(y), 0.0, float(pixel_dy), 0.0, float(angle_dy), phase,
                    _code(modifiers))
    return view.wheelEvent(ev)


@pytest.fixture
def headless_renderer():
    if not HAS_RENDERER:
        pytest.skip("release ingetrazo_render required")
    r = renderer()
    yield r
    del r

# SPDX-License-Identifier: GPL-3.0-or-later
"""GPU tests without a window or a toolkit (render-glue lane).

``renderer(w, h)`` is the release renderer drawing into a bare Core
Animation layer (``ingetrazo_render.headless_layer``), as it draws into the
shell's window. ``HeadlessView`` is the shell's resident view (``MetalView``
over ``WidgetShim``, as ``app_shell.view.ShellView``) with such a layer for
its surface and timers of its own; it replaces the hidden Qt ``WgpuView``.
Drive its timers with ``view.run_timers()``."""
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
            from ingetrazo_render import WindowTimers
            self._present = present
            MetalView.__init__(self, timers=WindowTimers())
            # As ShellView: the host after the bases, which start without one.
            self._host = SimpleNamespace(width=width, height=height, scale=scale,
                                         request_frame=lambda: None)

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

    return HeadlessView


def __getattr__(name):
    if name == "HeadlessView":
        return _view_class()
    raise AttributeError(name)


@pytest.fixture
def headless_renderer():
    if not HAS_RENDERER:
        pytest.skip("release ingetrazo_render required")
    r = renderer()
    yield r
    del r

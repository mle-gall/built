"""The document controller: ``views.viewport.Viewport``'s Python, no widget.

In the Qt app a hidden, disabled GL ``Viewport`` holds the scene, the
history, the camera, the active tool, the inference engine and the value
box; the Metal view routes input to the tools through it. Its GL context is
never made there, so the methods that path uses do not need one. Here the
same methods run on a QObject (:class:`ShellViewport`), with the class's
signals re-declared and the widget calls answered by
:class:`app_shell.borrow.WidgetShim`.
"""
from __future__ import annotations

from app_shell.borrow import WidgetShim, _rebind, borrow, signals_of


def _make_class():
    from views.viewport import Viewport
    from views.document_tools import DocumentTools
    from views.view_navigation import ViewNavigation
    cls = type("ShellViewport", (DocumentTools, ViewNavigation, WidgetShim), {
        **signals_of(Viewport),
        "__module__": __name__,
        "__doc__": "Viewport's document-controller methods on a QObject.",
    })
    init = _rebind(Viewport.__init__, cls)

    def __init__(self, host=None):
        init(self, None)
        self._host = host
        # As MainWindow._install_wgpu_display leaves the GL viewport.
        self._enabled = False

    cls.__init__ = __init__
    borrow(Viewport, cls)
    return cls


_CLASS = None


def ShellViewport(host=None):
    global _CLASS
    if _CLASS is None:
        _CLASS = _make_class()
    return _CLASS(host)

"""The Metal view: ``views.wgpu_view.WgpuView``'s Python on a QObject.

The renderer surface is the winit window's NSView, handed over by Rust; the
renderer itself is the statically linked ``ingetrazo_render`` module, so
Rust and Python share one ``Renderer``. Everything WgpuView does per frame
(packing, resident updates, inference index, selection, overlays, the egui
chrome through views/wgpu_ui.py) runs unchanged.
"""
from __future__ import annotations

from app_shell.borrow import WidgetShim, _rebind, borrow, signals_of


def _make_class():
    from views.wgpu_view import WgpuView
    cls = type("ShellView", (WidgetShim,), {
        **signals_of(WgpuView),
        "__module__": __name__,
        "_shim_visible": True,
    })
    init = _rebind(WgpuView.__init__, cls)

    def __init__(self, host):
        self._host = host
        init(self, None)
        self._host = host

    def winId(self):
        return self._host.ns_view

    def isVisible(self):
        # Headless (no surface yet, or the smoke test): nothing presents,
        # as a hidden WgpuView, so nothing creates a renderer.
        return bool(self._host.ns_view)

    def showEvent(self, event=None):
        if self.renderer is None or self._scene_dirty:
            self._upload()

    def resizeEvent(self, event=None):
        # The surface follows the window at once: winit already coalesces
        # live-resize steps into one Resized per frame.
        self._finish_resize()

    def hideEvent(self, event=None):
        self.tools.cancel()

    def leaveEvent(self, event=None):
        from views import wgpu_ui
        wgpu_ui.pointer(self, "leave")

    def closeEvent(self, event=None):
        self._closed = True
        tracker = getattr(self, "_instance_updates", None)
        if tracker is not None and tracker.unsubscribe is not None:
            tracker.unsubscribe()
        self.renderer = None
        if hasattr(self, "_inference_worker"):
            self._inference_worker.shutdown(wait=False, cancel_futures=True)

    for fn in (__init__, winId, isVisible, showEvent, resizeEvent, hideEvent, leaveEvent, closeEvent):
        setattr(cls, fn.__name__, fn)
    borrow(WgpuView, cls, skip={"paintEngine", "paintEvent"})
    return cls


_CLASS = None


def ShellView(host):
    global _CLASS
    if _CLASS is None:
        _CLASS = _make_class()
    return _CLASS(host)

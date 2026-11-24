"""The Metal view: ``views.wgpu_view.WgpuView``'s Python on a QObject.

The renderer surface is the winit window's NSView, handed over by Rust; the
renderer itself is the statically linked ``ingetrazo_render`` module, so
Rust and Python share one ``Renderer``. Everything WgpuView does per frame
(packing, resident updates, inference index, selection, overlays, the egui
chrome through views/wgpu_ui.py) runs unchanged.
"""
from __future__ import annotations

from views.view_host import WidgetShim
from views.wgpu_view import MetalView


class ShellView(MetalView, WidgetShim):
    _shim_visible = True

    def __init__(self, host):
        super().__init__(timers=host.timers)
        self._host = host

    def _presentation_timers(self):
        return (self._resize_timer, self._settle_timer, self.__dict__.get("_ui_timer"))

    def move_timers(self, host):
        self._timer_queue = host.timers
        for timer in self._presentation_timers():
            if timer is not None:
                host.timers.adopt(timer)

    def _window_handles(self):
        return self._host.ns_view, self._host.surface_handles

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
        self._host.timers.cancel_context(self)
        for timer in self._presentation_timers():
            if timer is not None:
                timer.stop()
        tracker = getattr(self, "_instance_updates", None)
        if tracker is not None and tracker.unsubscribe is not None:
            tracker.unsubscribe()
        self.renderer = None
        if hasattr(self, "_inference_worker"):
            self._inference_worker.shutdown(wait=False, cancel_futures=True)

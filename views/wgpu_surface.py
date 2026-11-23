"""The native handles for a Qt widget's wgpu presentation surface."""
from PySide6.QtGui import QGuiApplication


def window_handles(widget):
    # Use the QPA that owns the WId when constructing an unsafe native handle.
    platform = QGuiApplication.instance().property('platformName')
    window = int(widget.winId())
    if platform in ('cocoa', 'windows'):
        return window, {}
    if platform == 'xcb':
        app = QGuiApplication.instance().nativeInterface()
        return window, dict(display=int(app.connection()), platform='xcb')
    if platform.startswith('wayland'):
        # Qt's WId on Wayland is an integer token, not a wl_surface pointer.
        # A binding exposing QWaylandWindow supplies the actual surface.
        native_interface = getattr(widget.windowHandle(), 'nativeInterface', None)
        interface = native_interface() if native_interface else None
        surface = getattr(interface, 'surface', None)
        if callable(surface):
            display = QGuiApplication.instance().nativeInterface().display()
            return int(surface()), dict(display=int(display), platform='wayland')
        raise RuntimeError('This Qt binding requires X11 or XWayland (QT_QPA_PLATFORM=xcb) for GPU presentation')
    raise RuntimeError(f'No supported GPU surface for the Qt {platform} platform')

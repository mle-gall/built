"""The orbit never sticks to the mouse (Andrés Rodríguez, Windows 11,
0.5.0: «el orbital se mantiene activo aunque cambie a cualquier otra
herramienta… no se desactiva con nada»). A camera drag is remembered from
the press until its release; when that release was lost, every mouse move
orbited the view, whatever tool was picked."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from PySide6.QtCore import QPoint


def _window(native_host):
    win = native_host.window
    native_host.resize(800, 600, 1.0)
    return win


def test_picking_a_tool_ends_a_camera_drag(native_host):
    win = _window(native_host)
    vp = win.viewport
    vp.set_nav_mode("orbit")
    vp._last_pos = QPoint(100, 100)
    win._activate_tool("paint")
    assert vp.nav_mode is None and vp._last_pos is None
    # Leave no closed window holding Paint: until it is collected it sits
    # in the Alt-tap watcher and can take a later test's tap.
    win._activate_tool("select")



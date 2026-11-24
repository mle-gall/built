# SPDX-License-Identifier: GPL-3.0-or-later
"""T over a line starts a guide without a separate first mouse click."""
from tests.qt_window_lifetime import close_window
import pytest
from PySide6.QtCore import QPoint
from ingetrazo_core import Vec3 as V


@pytest.fixture
def setup(monkeypatch):
    from core.mesh import Mesh
    from views.main_window import MainWindow
    win = MainWindow(defer_extensions=True)
    vp = win.viewport
    vp.resize(800, 600)
    vp.scene.mesh = Mesh()
    vp.scene.groups = []
    vp.scene.mesh.add_face([V(0,0,0), V(4,0,0), V(4,3,0), V(0,3,0)])
    vp.scene.version += 1
    vp.camera.set_aspect(800, 600)
    vp.camera.set_view('top')
    vp.camera.perspective = False
    vp.camera.fit_to(V(-2,-2,-.1), V(6,5,.1))
    pointer = [vp._world_to_pixel(V(2,0,0))]
    # Emulate the position read by the shortcut without moving the real mouse.
    monkeypatch.setattr(vp, 'mapFromGlobal',
                        lambda _global: QPoint(round(pointer[0][0]), round(pointer[0][1])))
    yield win, vp, pointer
    win._saved_version = vp.scene.version
    close_window(win)


@pytest.mark.parametrize('where', ['empty', 'endpoint', 'outside'])
def test_T_elsewhere_keeps_the_normal_first_click_workflow(setup, where):
    win, vp, pointer = setup
    point = V(5,4,0) if where == 'empty' else V(0,0,0)
    pointer[0] = (-20,-20) if where == 'outside' else vp._world_to_pixel(point)
    win._activate_tool('tape')
    assert vp.active_tool.start_point is None
    assert not vp.scene.guides

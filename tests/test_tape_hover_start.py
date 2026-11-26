# SPDX-License-Identifier: GPL-3.0-or-later
"""Tape waits for its first click, including when selected over an edge."""
from tests.shell_isolation import restore_shell_patches  # noqa: F401
import pytest
from built_core import Vec3 as V


@pytest.fixture
def setup(native_host):
    from built_core.mesh import Mesh
    native_host.resize(800, 600, 1.)
    vp = native_host.viewport
    vp.scene.mesh = Mesh()
    vp.scene.groups = []
    vp.scene.mesh.add_face([V(0,0,0), V(4,0,0), V(4,3,0), V(0,3,0)])
    vp.scene.version += 1
    vp.camera.set_aspect(800, 600)
    vp.camera.set_view('top')
    vp.camera.perspective = False
    vp.camera.fit_to(V(-2,-2,-.1), V(6,5,.1))
    return native_host, vp


@pytest.mark.parametrize('where', ['empty', 'endpoint', 'outside'])
def test_T_elsewhere_keeps_the_normal_first_click_workflow(setup, where):
    host, vp = setup
    point = V(5,4,0) if where == 'empty' else V(0,0,0)
    pos = (-20,-20) if where == 'outside' else vp._world_to_pixel(point)
    host.pointer('move', *pos)
    host.window._activate_tool('tape')
    assert vp.active_tool.start_point is None
    assert not vp.scene.guides

def test_line_escape_then_tape_over_edge_starts_idle(setup):
    from built_core import Input as Qt
    host, vp = setup
    pos = vp._world_to_pixel(V(2, 0, 0))
    host.window._activate_tool("line")
    host.pointer("move", *pos)
    host.key(True, Qt.Key_Escape)
    host.window._activate_tool("tape")
    host.pointer("move", *pos)
    assert vp.active_tool.start_point is None
    assert not vp.active_tool.rubber_band_lines()
    assert "Click a point or an edge" in host.window.statusBar().currentMessage()
    host.pointer("press", *pos, button=1, held=1)
    host.pointer("release", *pos, button=1)
    assert vp.active_tool.start_point is not None
    assert not vp.scene.guides

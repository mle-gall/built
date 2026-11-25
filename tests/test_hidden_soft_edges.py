"""Hidden Geometry reveals smooth seams without changing normals or solids."""
import pytest
import numpy as np
from ingetrazo_core import Vec3 as V, Matrix4 as QMatrix4x4
from core.mesh import Mesh
from core.group import Group
from tests.controller_surface import live_image
from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401
from tests.test_rendering_surface_regressions import viewport
from tests.raster_samples import pixels
from tests.test_viewport_smooth_shading import _cylinder


@pytest.mark.parametrize('placement', ['loose', 'group', 'instance', 'nested'])
def test_hidden_geometry_reveals_soft_seams(viewport, placement):
    vp = viewport
    mesh = _cylinder(vp.scene.mesh if placement == 'loose' else Mesh())
    if placement != 'loose':
        g = Group(mesh, 'smooth tube')
        if placement in ('instance', 'nested'):
            g.xform = QMatrix4x4()
            g.xform.translate(.2, 0, 0)
        if placement == "nested":
            parent_mesh = Mesh()
            child = g
            g = Group(parent_mesh, "container")
            g.adopt([child])
            g.xform = QMatrix4x4()
            g.xform.rotate(25, 0, 0, 1)
        vp.scene.groups.append(g)
    vp.scene.show_axes = False
    vp.scene.display_style.profiles = False
    vp.scene.version += 1
    vp.camera.target = V(0, 0, 1)
    vp.camera.distance = 5
    vp.camera.pitch = .2
    vp.camera.yaw = .7
    before_flags = [e.soft for e in mesh.edges]
    off = pixels(live_image(vp)).astype(int)
    content = vp.scene.content_version
    vp.window()._set_hidden_view('show_hidden_geometry', True)
    assert vp.scene.content_version == content
    on = pixels(live_image(vp))
    # Thousands of smooth-wall pixels remain identical, with dashed seams
    # added in front. The source flags (and hence smoothing) stay unchanged.
    darker = np.any(on < off - 20, axis=2)
    assert np.count_nonzero(darker) > 50
    assert np.count_nonzero(darker) < darker.size * .07
    assert [e.soft for e in mesh.edges] == before_flags
    display = vp.keys_stand_in
    hidden_count = int(display.packed.hidden_edge_commands["count"].sum())
    assert hidden_count == 2 * sum(e.soft or e.hidden for e in mesh.edges)
    vp.window()._set_hidden_view("show_hidden_geometry", False)
    assert np.array_equal(off, pixels(live_image(vp)))
    vp.window()._set_hidden_view("show_hidden_geometry", True)
    assert np.array_equal(on, pixels(live_image(vp)))
    assert int(display.packed.hidden_edge_commands["count"].sum()) == hidden_count

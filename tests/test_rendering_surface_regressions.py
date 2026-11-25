"""Tube triangulation, transformed smooth instances and thin back materials."""
from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401
import math

import numpy as np
import pytest
from built_core import Vec3 as V, Matrix4 as QMatrix4x4
from core.mesh import Mesh
from core.group import Group
from built_core import group_transformed_mesh as transformed_mesh
from formats.meshexport import SmoothNormals
from tests.controller_surface import ControllerSurface


def _tube(split_panel=False):
    mesh = Mesh()
    count = 12
    bottom = [V(math.cos(i * math.tau / count), math.sin(i * math.tau / count), 0)
              for i in range(count)]
    top = [p + V(0, 0, 2) for p in bottom]
    sides = []
    for i in range(count):
        j = (i + 1) % count
        if split_panel and i == 0:
            sides.append(mesh.add_face([bottom[i], bottom[j], top[j]]))
            sides.append(mesh.add_face([bottom[i], top[j], top[i]]))
        else:
            sides.append(mesh.add_face([bottom[i], bottom[j], top[j], top[i]]))
    if not split_panel:
        mesh.add_face(list(reversed(bottom)))
        mesh.add_face(top)
    for edge in mesh.edges:
        edge.soft = len(edge.faces) == 2 and all(face in sides for face in edge.faces)
    return mesh


def test_splitting_one_tube_panel_does_not_move_its_corner_normals():
    m = _tube(split_panel=True)
    smooth = SmoothNormals()
    for f in m.faces:
        for pos, normal in smooth.corners(f).items():
            assert np.allclose(normal.toTuple(), (pos[0], pos[1], 0), atol=1e-5)


def test_rotating_smooth_cpu_chunk_matches_baking_world_geometry():
    vp = ControllerSurface()
    vp._LIGHT = V(1., .3, 0.).normalized()
    vp._chunk_cache_load = vp._chunk_cache_store = None
    mesh = _tube()
    xf = QMatrix4x4()
    xf.rotate(43, 1, 0, 0)
    instance = Group(mesh, 'instance')
    instance.xform = xf
    placed = Group(transformed_mesh(mesh, xf), 'placed')
    got = np.frombuffer(vp._instance_chunk(instance)['vcol'], np.float32).reshape(-1, 6)
    want = np.frombuffer(vp._group_chunk(placed)['vcol'], np.float32).reshape(-1, 6)
    # Side triangles are smooth; caps retain their quantised flat shade.
    assert np.allclose(got[:72], want[:72], atol=1e-5)


def pixels(image):
    width, height, rgba = image
    return np.frombuffer(rgba, np.uint8).reshape(height, width, 4)[..., :3].astype(int)


@pytest.fixture
def viewport(native_gpu_host):
    win = native_gpu_host.window
    vp = win.viewport
    yield vp


def test_thin_plate_reverse_material_does_not_show_through(viewport):
    vp = viewport
    m = vp.scene.mesh
    front = [V(-10, -10, 0), V(10, -10, 0), V(10, 10, 0), V(-10, 10, 0)]
    m.add_face(front)
    back = m.add_face([p - V(0, 0, .001) for p in reversed(front)])
    back.attrs['back'] = {'color': (1., 0., 0.)}
    cam = vp.camera
    cam.target, cam.distance = V(0, 0, 0), 20.
    cam.yaw, cam.pitch = math.radians(20), math.radians(5)
    vp.scene.version += 1
    red = pixels(vp.keys_stand_in.render_pixels(400, 300, overlays=False))
    back.attrs['back']['color'] = (0., 1., 0.)
    vp.scene.version += 1
    green = pixels(vp.keys_stand_in.render_pixels(400, 300, overlays=False))
    # A hidden reverse material must not affect the middle of the front.
    assert np.max(np.abs(red[130:170, 160:240] - green[130:170, 160:240])) <= 2


def test_smooth_gpu_cylinder_is_invariant_under_one_panel_rotation(viewport):
    vp = viewport
    mesh = _tube()
    group = Group(mesh, 'tube')
    group.xform = QMatrix4x4()
    vp.scene.groups.append(group)
    vp.camera.target, vp.camera.distance = V(0, 0, 1), 7.
    vp.camera.yaw, vp.camera.pitch = math.radians(25), math.radians(10)
    vp.scene.version += 1
    assert vp._instanced_eligible(group)
    a = pixels(vp.keys_stand_in.render_pixels(400, 300, overlays=False))
    group.xform.rotate(30, 0, 0, 1)
    vp.scene.version += 1
    b = pixels(vp.keys_stand_in.render_pixels(400, 300, overlays=False))
    assert np.percentile(np.abs(a-b), 99) <= 2


def test_document_capture_owns_rgba_and_preserves_display_scale():
    from types import SimpleNamespace
    from tests.controller_surface import live_image

    rgba = bytes((1, 2, 3, 4, 5, 6, 7, 8))
    events = []
    renderer = SimpleNamespace(request_capture=lambda: events.append("request"),
                               take_capture=lambda: rgba)
    display = SimpleNamespace(renderer=renderer, set_scene=events.append,
                              _prepare_pick_frame=lambda: events.append("prepare"),
                              draw_frame=lambda: events.append("draw"),
                              width=lambda: 1, devicePixelRatioF=lambda: 2)
    frame = live_image(SimpleNamespace(keys_stand_in=display, scene="scene"))
    assert events == ["scene", "prepare", "request", "draw"]
    assert (frame.width, frame.height) == (2, 1)
    assert frame.rgba() == rgba

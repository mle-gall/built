"""Several views of one resident Metal scene (Renderer.add_target).

Each view has its own surface, size, camera, style and pick IDs; the scene
is sent once. Default pytest runs Qt offscreen and skips these: run with
QT_QPA_PLATFORM=cocoa on macOS (no widget is shown).
"""
import importlib.util
import os
import sys

import numpy as np
import pytest

from tests.test_metal_parity_raster import synthetic_pack

HAS_RENDERER = importlib.util.find_spec("ingetrazo_render") is not None


def _camera(shift_x=0.):
    from ingetrazo_core import Matrix4 as QMatrix4x4
    view = QMatrix4x4()
    view.translate(shift_x, 0, -3)
    proj = QMatrix4x4()
    proj.ortho(-1, 1, -1, 1, .1, 10)
    return list(view.data()), list(proj.data())


@pytest.fixture
def two():
    if (not HAS_RENDERER or sys.platform != 'darwin'
            or os.environ.get('QT_QPA_PLATFORM') != 'cocoa'):
        pytest.skip('Metal checks need the release crate and an unshown Cocoa NSView')
    from PySide6.QtCore import Qt
    from PySide6.QtWidgets import QApplication, QWidget
    from ingetrazo_render import Renderer
    QApplication.instance() or QApplication([])
    hosts = []
    for w, h in ((64, 64), (48, 32)):
        host = QWidget()
        host.setAttribute(Qt.WA_DontShowOnScreen)
        host.setAttribute(Qt.WA_NativeWindow)
        host.resize(w, h)
        hosts.append(host)
    renderer = Renderer(int(hosts[0].winId()), 64, 64, 1.)
    renderer.set_antialias(False)
    renderer.set_edges(False)
    renderer.set_camera(*_camera())
    v, i, b, c, r = synthetic_pack(255)
    renderer.set_scene(tuple(x.view(np.uint8).ravel() for x in (v, i, b, c)),
                       r.view(np.uint8).ravel())
    yield renderer, hosts
    del renderer
    for host in hosts:
        host.close()


def _capture(renderer, w, h):
    renderer.request_capture()
    renderer.render(False)
    return np.frombuffer(bytes(renderer.take_capture()), np.uint8).reshape(h, w, 4)


def _red(px):
    return px[0] > 180 and px[1] < 30 and px[2] < 30


def test_a_second_view_shares_the_scene_and_keeps_its_own_camera(two):
    renderer, hosts = two
    scene_bytes = renderer.gpu_memory
    one_view = renderer.target_memory
    second = renderer.add_target(int(hosts[1].winId()), 48, 32, 1.)
    assert second == 1 and renderer.target_count == 2
    # Nothing of the scene is sent again; only the view's own attachments.
    assert renderer.gpu_memory == scene_bytes
    assert renderer.target_memory == one_view + 48 * 32 * (one_view // (64 * 64))
    renderer.bind_target(second)
    assert renderer.active_target == 1
    renderer.set_antialias(False)
    renderer.set_camera(*_camera(shift_x=5.))       # the quad is off to the side
    image = _capture(renderer, 48, 32)
    assert image.shape == (32, 48, 4)
    assert not _red(image[16, 24])
    renderer.bind_target(0)
    assert _red(_capture(renderer, 64, 64)[32, 32]), "the first view's camera came back"
    renderer.bind_target(second)
    assert not _red(_capture(renderer, 48, 32)[16, 24])


def test_each_view_has_its_own_style(two):
    renderer, hosts = two
    second = renderer.add_target(int(hosts[1].winId()), 48, 32, 1.)
    renderer.bind_target(second)
    renderer.set_camera(*_camera())
    renderer.set_style(dict(face_mode='wireframe', sky=False, edges=False,
                            profiles=False))
    assert not _red(_capture(renderer, 48, 32)[16, 24]), "wireframe: no faces"
    renderer.bind_target(0)
    renderer.set_style(dict(face_mode='shaded', sky=False, edges=False,
                            profiles=False))
    assert _red(_capture(renderer, 64, 64)[32, 32])
    renderer.bind_target(second)
    assert not _red(_capture(renderer, 48, 32)[16, 24]), "its style stayed"


def test_picking_reads_the_view_asked(two):
    renderer, hosts = two
    second = renderer.add_target(int(hosts[1].winId()), 48, 32, 1.)
    renderer.bind_target(second)
    renderer.set_camera(*_camera(shift_x=5.))
    renderer.render(False)
    assert renderer.pick(24, 16) == -1
    renderer.bind_target(0)
    renderer.render(False)
    assert renderer.pick(32, 32) == 0


def test_a_scene_change_reaches_every_view(two):
    renderer, hosts = two
    second = renderer.add_target(int(hosts[1].winId()), 48, 32, 1.)
    renderer.bind_target(second)
    renderer.set_camera(*_camera())
    renderer.render(False)
    assert renderer.pick(24, 16) == 0
    renderer.bind_target(0)
    renderer.remove_instances([0])            # while the other view is away
    renderer.bind_target(second)
    assert not renderer.pick_cache_valid
    assert renderer.pick(24, 16) == -1
    assert not _red(_capture(renderer, 48, 32)[16, 24])


def test_a_view_goes_and_the_first_stays(two):
    renderer, hosts = two
    second = renderer.add_target(int(hosts[1].winId()), 48, 32, 1.)
    renderer.bind_target(second)
    renderer.remove_target(second)
    assert renderer.active_target == 0 and renderer.target_count == 1
    with pytest.raises(ValueError):
        renderer.bind_target(second)
    with pytest.raises(ValueError):
        renderer.remove_target(0)
    assert _red(_capture(renderer, 64, 64)[32, 32])


def test_settled_pick_tile_keeps_pixel_rows_and_invalidates_on_camera_change(two):
    renderer, _ = two
    renderer.resize(149, 141, 1.)
    renderer.set_camera(*_camera())
    renderer.render(False)
    renderer.render(False)
    assert renderer.pick_cache_valid
    assert renderer.pick(74, 70) == 0
    assert renderer.pick(1, 1) == -1           # same tile as the face
    assert renderer.pick(148, 140) == -1       # clipped tile at both edges
    renderer.set_camera(*_camera(shift_x=5.))
    assert renderer.pick(74, 70) == -1

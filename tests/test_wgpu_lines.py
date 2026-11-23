"""What the Metal view sends to the native depth-tested line pass.

Device-free: a stub renderer records ``set_lines``. The raster behaviour
(depth test, dashes) is in ``test_metal_parity_raster.py``.
"""
from types import SimpleNamespace

import numpy as np
import pytest
from PySide6.QtGui import QPainter, QImage
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D
from PySide6.QtWidgets import QApplication

from core.group import Group
from core.guide import Guide
from core.mesh import Mesh
from core.scene import Scene
from views.document_interaction import _box_edge_vertices
from tests.controller_surface import ControllerSurface as Viewport
from views.wgpu_lines import SEGMENT, sync
from views.wgpu_scene import pack_scene
from views.wgpu_view import WgpuView

app = QApplication.instance() or QApplication([])
V = QVector3D


def cube_mesh():
    mesh = Mesh()
    mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 0, 1), V(0, 0, 1)])
    return mesh


@pytest.fixture
def view():
    inner = Group(cube_mesh(), name='inner')
    outer = Group(Mesh(), name='outer')
    outer.adopt([inner])
    scene = Scene(groups=[outer])
    source = Viewport()
    source.scene = scene
    source.resize(800, 600)
    v = WgpuView()
    v.resize(800, 600)
    v.scene = scene
    v.source_viewport = source
    v.camera = source.camera
    v.packed = pack_scene(scene)
    v.uid_rows = {g.uid: [i] for i, g in enumerate(v.packed.groups) if g is not None}
    sent = []
    layers = {}

    def set_lines(slot, data, above_edges=False):
        layers[slot] = above_edges
        sent.append((slot, np.frombuffer(bytes(data), SEGMENT).copy()))
    v.renderer = SimpleNamespace(set_lines=set_lines)
    v.line_layers = layers
    yield v, source, outer, inner, sent
    v.renderer = None
    v.close()
    source.close()


def last(sent, slot):
    return [data for name, data in sent if name == slot][-1]


def test_edit_box_is_twelve_dashed_segments_sent_once(view):
    v, source, outer, inner, sent = view
    assert sync(v) == []                      # nothing open, nothing to send
    v.scene.begin_group_edit(outer)
    assert sync(v) == ['edit_box']
    box = last(sent, 'edit_box')
    assert len(box) == 12
    expected = np.asarray(_box_edge_vertices(v._edit_group_box_corners()),
                          np.float32).reshape(-1, 2, 3)
    np.testing.assert_allclose(box['a'], expected[:, 0])
    np.testing.assert_allclose(box['b'], expected[:, 1])
    assert (box['color'] == (90, 110, 140, 255)).all()
    assert (box['width'] == 1).all()
    assert v.line_layers == {'edit_box': False}, 'under the scene edges, as in GL'
    # GL's _draw_scaffold dash: 0.003·h/tan(fov/2) window pixels, 3..20.
    assert 3 <= box['dash'][0] <= 20 and (box['gap'] == box['dash']).all()
    # A repaint with nothing changed sends nothing.
    assert sync(v) == []
    v.scene.end_one_group_edit()
    assert sync(v) == ['edit_box']
    assert len(last(sent, 'edit_box')) == 0


def test_edit_box_not_on_a_sheet(view):
    v, source, outer, _, sent = view
    v.scene.begin_group_edit(outer)
    source.plano_style = object()
    sync(v)
    assert not any(name == 'edit_box' and len(data) for name, data in sent)
    source.plano_style = None


def test_guides_are_gl_world_dashes_resent_on_camera_and_selection(view):
    from views.wgpu_annotations import guide_segments
    v, _, _, _, sent = view
    guide = Guide(V(0, 0, 1), V(1, 0, 0))
    v.scene.guides = [guide]
    assert sync(v) == ['guides']
    guides = last(sent, 'guides')
    pairs, _ = guide_segments(v.scene, v.camera)
    assert len(guides) == len(pairs) > 10
    np.testing.assert_allclose(guides['a'], pairs[:, 0])
    assert (guides['dash'] == 0).all()        # already dashed in world space
    assert (guides['color'] == (69, 89, 120, 255)).all()
    assert sync(v) == []
    v.scene.select([guide])
    assert sync(v) == ['guides']
    assert (last(sent, 'guides')['color'] == (242, 115, 41, 255)).all()
    v.camera.distance *= 2
    assert sync(v) == ['guides']
    v.scene.guides = []
    assert sync(v) == ['guides'] and len(last(sent, 'guides')) == 0


def test_selected_hidden_edges_dashed_only_with_hidden_geometry(view):
    v, _, outer, inner, sent = view
    soft = inner.mesh.edges[0]
    soft.soft = True
    matrix = QMatrix4x4()
    matrix.translate(5, 0, 0)
    inner.xform = matrix
    v.packed = pack_scene(v.scene)
    v.uid_rows = {g.uid: [i] for i, g in enumerate(v.packed.groups) if g is not None}
    v.scene.select([outer])
    assert 'selected_hidden' not in sync(v)
    v.scene.show_hidden_geometry = True
    assert sync(v) == ['selected_hidden']
    hidden = last(sent, 'selected_hidden')
    assert len(hidden) == 1
    ends = sorted([tuple(hidden['a'][0]), tuple(hidden['b'][0])])
    assert ends == sorted([tuple(np.add(soft.a.toTuple(), (5, 0, 0))),
                           tuple(np.add(soft.b.toTuple(), (5, 0, 0)))])
    assert (hidden['color'] == (242, 115, 41, 255)).all()
    assert hidden['dash'][0] == 3 * max(1., v.devicePixelRatioF())
    assert v.line_layers == {'selected_hidden': True}, 'after the scene edges, as in GL'
    assert sync(v) == []
    v.scene.select([])
    assert sync(v) == ['selected_hidden'] and len(last(sent, 'selected_hidden')) == 0
    v.scene.show_hidden_geometry = False


def test_live_annotations_leave_guide_lines_to_the_native_pass(monkeypatch):
    import views.wgpu_annotations as annotations
    calls = []
    monkeypatch.setattr(annotations, 'draw_guide_lines', lambda *a: calls.append(a))
    scene = Scene()
    scene.guides = [Guide(V(0, 0, 1), V(1, 0, 0))]
    from core.camera import OrbitCamera as Camera
    image = QImage(64, 64, QImage.Format_ARGB32_Premultiplied)
    for guide_lines, expected in ((False, 0), (True, 1)):
        painter = QPainter(image)
        annotations.draw_annotations(painter, scene, Camera(), (64, 64), guide_lines=guide_lines)
        painter.end()
        assert len(calls) == expected

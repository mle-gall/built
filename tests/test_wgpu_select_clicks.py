"""Select clicks in the Metal view, inside an open group.

The classic rhythm on a face: one click takes the face, a double click the
face and its edges, a triple click everything connected to it at the same
level. The view had no triple click: its third press was an ordinary click
and left a single face selected.

A face the ID pass shows covered by the faded rest of the model (a plate
against a profile) is still the open group's to click, as in GL. While the
snap index was being rebuilt on its worker, such a face took no click."""
import math

import pytest
from built_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from core.group import Group
from core.mesh import Edge, Face, Mesh
from core.scene import Scene
from built_core import resident_prepare as prepare
from built_core import pack_scene
from tests.headless_gpu import HeadlessView, mouse


class _Renderer:
    def __init__(self):
        self.row = 0

    def pick(self, *a):
        return self.row

    def pick_rect(self, *a):
        return [self.row]

    def set_highlight(self, *a):
        pass

    def set_rest(self, *a):
        pass


@pytest.fixture
def view():
    from tests.controller_surface import ControllerSurface as Viewport
    from tools.rust_tools import SelectTool
    scene = Scene()
    mesh = Mesh()
    # Two quads sharing the edge x=2 (7 edges), and a separate quad.
    v = QVector3D
    mesh.add_face([v(0, 0, 0), v(2, 0, 0), v(2, 2, 0), v(0, 2, 0)])
    mesh.add_face([v(2, 0, 0), v(4, 0, 0), v(4, 2, 0), v(2, 2, 0)])
    mesh.add_face([v(6, 0, 0), v(7, 0, 0), v(7, 1, 0), v(6, 1, 0)])
    group = Group(mesh, name='plates')
    group.xform = QMatrix4x4()
    group.xform.translate(4, 3, 0)
    scene.groups.append(group)
    # Outside the open group, half a unit above the plates: in the ID pass
    # it covers them.
    cover_mesh = Mesh()
    cover_mesh.add_face([v(3, 2, .5), v(12, 2, .5), v(12, 6, .5), v(3, 6, .5)])
    scene.groups.append(Group(cover_mesh, name='cover'))
    gl = Viewport()
    gl.resize(800, 600)
    gl.scene = scene
    gl.active_tool = SelectTool()
    view = HeadlessView()
    view.resize(800, 600)
    view.scene = scene
    view.source_viewport = gl
    gl.flash_status = lambda *a, **k: None
    scene.begin_group_edit(group)
    assert scene.edit_group is group
    view.packed = pack_scene(scene)
    view.uid_rows = {uid: [i] for i, uid in enumerate(view.packed.uids)}
    view.camera.target = QVector3D(7, 4, 0)
    view.camera.pitch = math.pi / 2 - .001
    view.camera.distance = 12
    view.camera.aspect = 800 / 600
    view.inference_index = prepare(view.packed)
    view.renderer = _Renderer()
    view.renderer.row = _row_of(view, group)
    yield view
    view.renderer = None
    view.source_viewport = None
    view.close()
    gl.close()


def _row_of(view, group):
    return next(i for i, g in enumerate(view.packed.groups) if g is group)


_T = [10_000]


def _send(view, kind, pos):
    _T[0] += 50
    mouse(view, kind, *pos, button=1, held=0 if kind == 'release' else 1, timestamp=_T[0])


def _clicks(view, pos, n):
    _send(view, 'press', pos)
    _send(view, 'release', pos)
    if n >= 2:
        _send(view, 'double', pos)
        _send(view, 'release', pos)
    if n >= 3:
        _send(view, 'press', pos)
        _send(view, 'release', pos)


def _counts(scene):
    sel = scene.selection
    return (sum(isinstance(e, Face) for e in sel),
            sum(isinstance(e, Edge) for e in sel))


def _first_face_pixel(view):
    # The first quad's centre, in world coordinates (x 4..6, y 3..5).
    x, y = view._world_to_pixel(QVector3D(5, 4, 0))
    return x, y


def test_one_click_takes_the_face(view):
    _clicks(view, _first_face_pixel(view), 1)
    assert _counts(view.scene) == (1, 0)


def test_double_click_takes_the_face_and_its_edges(view):
    _clicks(view, _first_face_pixel(view), 2)
    assert _counts(view.scene) == (1, 4)


def test_triple_click_takes_everything_connected(view):
    _clicks(view, _first_face_pixel(view), 3)
    # Both joined quads and their 7 edges; the separate quad stays out.
    assert _counts(view.scene) == (2, 7)


def test_a_late_third_press_is_a_new_click(view):
    pos = _first_face_pixel(view)
    _clicks(view, pos, 2)
    _T[0] += 5_000          # well past the double-click interval
    _clicks(view, pos, 1)
    assert _counts(view.scene) == (1, 0)


def _covered(view):
    cover = next(g for g in view.scene.groups if g.name == 'cover')
    view.renderer.row = _row_of(view, cover)


def test_a_face_under_the_faded_rest_takes_the_click(view):
    _covered(view)
    _clicks(view, _first_face_pixel(view), 1)
    assert _counts(view.scene) == (1, 0)


def test_a_covered_face_takes_the_click_while_the_index_builds(view):
    # The snap index is rebuilt on a worker after a repack (a second and
    # more on a big model); meanwhile the click must still reach the open
    # group's face rather than nothing.
    _covered(view)
    view.inference_index = None
    _clicks(view, _first_face_pixel(view), 1)
    assert _counts(view.scene) == (1, 0)
    view.scene.clear_selection()
    _clicks(view, _first_face_pixel(view), 3)
    assert _counts(view.scene) == (2, 7)


def test_a_click_past_the_open_group_still_takes_nothing_while_the_index_builds(view):
    _covered(view)
    view.inference_index = None
    x, y = view._world_to_pixel(QVector3D(14, 4, .5))
    view.scene.select([view.scene.mesh.faces[0]])
    _clicks(view, (x, y), 1)
    assert not [e for e in view.scene.selection if isinstance(e, (Face, Edge))]

"""Native pointer and keyboard events, model interaction and capture state restoration."""
from built_render import ContextMenuEvent, MouseEvent
import math
from tests.native_view_input import click, press, release, double_click, key, type_text
from types import SimpleNamespace

import numpy as np
import pytest
from built_core import PixelPoint as QPoint, ScreenPoint as QPointF
from built_core import Input as Qt
from built_core import Vec3 as QVector3D, Matrix4 as QMatrix4x4
from built_core.scene import Scene
from built_core.mesh import Mesh
from built_core.group import Group
from built_core.history import History
from tools.rust_tools import SelectTool
from tests.controller_surface import ControllerSurface as Viewport
from built_core import resident_prepare as prepare


@pytest.fixture
def view(native_host):
    source = native_host.viewport
    source.scene = Scene()
    source.history = History(source.scene)
    source.flash_status = lambda *a: None
    source.notify_scene_changed = lambda: None
    source.active_tool = SelectTool()
    native_host.resize(800, 600, 1.)
    v = native_host.view
    v.source_viewport = source
    v.camera = source.camera
    v.camera.target = QVector3D(1, 1, 0)
    v.camera.pitch = math.pi / 2 - .001
    v.camera.distance = 8
    v.camera.aspect = 800 / 600
    v.set_scene(source.scene)
    v._upload = lambda **kwargs: None
    yield v
    v.renderer = None


def face(view):
    f = view.scene.mesh.add_face([QVector3D(0, 0, 0), QVector3D(2, 0, 0),
                                  QVector3D(2, 2, 0), QVector3D(0, 2, 0)])
    view.scene.version += 1
    view.set_scene(view.scene)
    view.inference_index = prepare(view.packed)
    view.pick_row = lambda *a: 0
    return f


def pixel(view, point):
    x, y = view._world_to_pixel(point)
    return QPoint(round(x), round(y))


def mouse_move(view, p):
    event = MouseEvent('move', p.x(), p.y(), Qt.NoButton, Qt.NoButton, Qt.NoModifier)
    view.mouseMoveEvent(event)


def test_click_hover_modifiers(view):
    f = face(view)
    p = pixel(view, QVector3D(1, 1, 0))
    click(view, Qt.LeftButton, Qt.NoModifier, p)
    assert view.scene.selection == {f}
    click(view, Qt.LeftButton, Qt.ControlModifier, p)
    assert view.scene.selection == {f}
    click(view, Qt.LeftButton, Qt.ShiftModifier, p)
    assert not view.scene.selection
    click(view, Qt.LeftButton, Qt.ShiftModifier, p)
    assert view.scene.selection == {f}
    click(view, Qt.LeftButton, Qt.ControlModifier | Qt.ShiftModifier, p)
    assert not view.scene.selection
    mouse_move(view, p)
    assert view.tools._mesh_hover is None  # GL suppresses face hover
    edge = view.scene.edges[0]
    mouse_move(view, pixel(view, (edge.a + edge.b) * .5))
    assert view.tools._mesh_hover == edge


def test_box_occluded_and_loose(view):
    f = face(view)
    mesh = Mesh()
    mesh.add_face([QVector3D(0, 0, -1), QVector3D(2, 0, -1), QVector3D(0, 2, -1)])
    group = Group(mesh)
    view.scene.groups.append(group)
    view.scene.version += 1
    view.set_scene(view.scene)
    # Row picking sees only the front face; box selection must see the back group.
    view.pick_row = lambda *a: 0
    pts = [pixel(view, p) for p in f.vertices]
    lo = QPoint(min(p.x() for p in pts)-15, min(p.y() for p in pts)-15)
    hi = QPoint(max(p.x() for p in pts)+15, max(p.y() for p in pts)+15)
    press(view, Qt.LeftButton, Qt.NoModifier, lo)
    release(view, Qt.LeftButton, Qt.NoModifier, hi)
    assert f in view.scene.selection and group in view.scene.selection
    assert all(e in view.scene.selection for e in view.scene.edges)
    mid = QPoint((lo.x()+hi.x())//2, hi.y())
    press(view, Qt.LeftButton, Qt.NoModifier, lo)
    release(view, Qt.LeftButton, Qt.NoModifier, mid)
    assert f not in view.scene.selection
    press(view, Qt.LeftButton, Qt.NoModifier, mid)
    release(view, Qt.LeftButton, Qt.NoModifier, lo)
    assert f in view.scene.selection


def fixed_world(view, point):
    # Isolate command events from inference, tested independently above.
    view.hover = lambda *a: setattr(view, 'last_snap', SimpleNamespace(
        point=QVector3D(*point), kind=None, context=None))


def test_section_place_move_toggle_undo(view):
    from built_core.settings import settings
    from tools.section import SectionPlaneTool
    from tools.move import MoveTool
    settings().setValue("section/ask_name", "0")
    view.source_viewport.active_tool = SectionPlaneTool()
    fixed_world(view, (0, 0, 0))
    click(view, Qt.LeftButton, Qt.NoModifier, QPoint(400, 300))
    plane, = view.scene.section_planes
    assert plane.active
    view.source_viewport.active_tool = SelectTool()
    view.tools.sync()
    frame = view.tools.section_plane_frame(plane)
    p = pixel(view, (frame[0] + frame[1]) * .5)
    assert view.tools.pick_section_plane(p.x(), p.y()) is plane
    double_click(view, Qt.LeftButton, Qt.NoModifier, p)
    assert not plane.active
    view.source_viewport.history.undo()
    assert plane.active
    view.scene.select([plane])
    view.source_viewport.active_tool = MoveTool()
    click(view, Qt.LeftButton, Qt.NoModifier, QPoint(400, 300))
    fixed_world(view, (0, 0, 2))
    click(view, Qt.LeftButton, Qt.NoModifier, QPoint(400, 300))
    assert plane.point.z() == 2
    view.source_viewport.history.undo()
    assert plane.point.z() == 0
    view.source_viewport.history.undo()
    assert not view.scene.section_planes


def test_copy_move_array_and_undo(view):
    from tools.move import MoveTool
    mesh = Mesh()
    mesh.add_edge(QVector3D(), QVector3D(1, 0, 0))
    group = Group(mesh)
    group.xform = QMatrix4x4()
    view.scene.groups.append(group)
    view.scene.select([group])
    view.scene.version += 1
    view.set_scene(view.scene)
    view.source_viewport.active_tool = MoveTool()
    fixed_world(view, (0, 0, 0))
    click(view, Qt.LeftButton, Qt.ControlModifier, QPoint(400, 300))
    fixed_world(view, (2, 0, 0))
    click(view, Qt.LeftButton, Qt.ControlModifier, QPoint(420, 300))
    assert len(view.scene.groups) == 2
    assert group.xform.column(3).x() == 0
    type_text(view, '3x')
    key(view, Qt.Key_Return)
    assert group in view.scene.groups
    assert sorted(g.xform.column(3).x() for g in view.scene.groups[-1].children) == [2, 4, 6]
    type_text(view, '/2')
    key(view, Qt.Key_Return)
    assert sorted(g.xform.column(3).x() for g in view.scene.groups[-1].children) == [1, 2]
    view.source_viewport.history.undo()
    assert view.scene.groups == [group]


def test_context_group_and_locked_target(view, monkeypatch):
    mesh = Mesh()
    mesh.add_face([QVector3D(), QVector3D(2, 0, 0), QVector3D(0, 2, 0)])
    group = Group(mesh)
    view.scene.groups.append(group)
    view.scene.version += 1
    view.set_scene(view.scene)
    view.pick_row = lambda *a: 0
    # The window the menu's facts read (viewport_menu.rs): every check "no".
    win = SimpleNamespace(viewport=view.source_viewport, _hideable=lambda sel: False,
                          _makes_faces=lambda sel: False, _bridgeable=lambda sel: False,
                          _single_textured_face=lambda: None, _replacement_choices=lambda sel: [],
                          _clipboard_available=lambda: False, view=view)
    monkeypatch.setattr(view.source_viewport, "window", lambda: win)
    view._ui_attached = True
    view.renderer = SimpleNamespace(ui_over_chrome=lambda x, y: False,
                                    ui_set_context_menu=lambda *args: None)
    event = ContextMenuEvent(400, 300)
    view.contextMenuEvent(event)
    assert view.scene.selection == {group}
    assert view._ctx_menu.kind == "group" and view._ctx_menu.locked_group is None
    group.locked = True
    view.scene.clear_selection()
    view.contextMenuEvent(event)
    assert not view.scene.selection
    assert view._ctx_menu.locked_group is group


def test_selection_marks_and_preview_bridge(view):
    f = face(view)
    view.scene.select([f])
    calls = []
    view.renderer = SimpleNamespace(set_highlight=lambda *a: None,
        set_face_marks=lambda a: calls.append(('marks', a.copy())),
        set_suppressed_faces=lambda a: calls.append(('suppression', a)))
    view.sync_selection()
    assert calls[-1][0] == 'marks' and len(calls[-1][1]) == 6*28
    original = view.packed
    view.tools.set_suppressed_faces([f])
    assert calls[-1] == ('suppression', [(0, 0, 2)])
    assert view.packed is original and not f.attrs.get("hidden")
    view.tools.set_suppressed_faces([])
    assert calls[-1] == ('suppression', [])


@pytest.mark.parametrize('fail', [False, True])
def test_billboard_export_restores_world_and_schedules_overlays(view, fail, monkeypatch):
    from built_core.group import make_billboard_group
    from built_core import INSTANCE
    from built_render import SurveyRaster
    from pathlib import Path
    texture = Path('resources/components/person_billboard.png').resolve()
    im = SurveyRaster.open(texture)
    view.scene.groups.append(make_billboard_group(str(texture), 1.75, 'Figure', im.width/im.height))
    view.scene.version += 1
    view.set_scene(view.scene)
    view._scene_dirty = False
    before = view.packed.instances['world'].copy()
    scheduled = []
    def composite(image, *args, **kwargs):
        scheduled.append(True)
        return image
    monkeypatch.setattr("views.wgpu_annotations.composite_pixels", composite)

    class Capture:
        def __init__(self):
            self.world = before.copy()
        def set_instance_world(self, ids, values):
            self.world[ids] = values.reshape(-1, 4, 4)
        def set_axes(self, *args):
            pass
        def set_section_geometry(self, *args):
            pass
        def render_image(self, width, height, vm, pm, style, data):
            rows = np.frombuffer(data, INSTANCE)
            np.testing.assert_allclose(rows['world'], self.world)
            assert not np.array_equal(self.world, before)
            if fail:
                raise RuntimeError('capture failed')
            return bytes(width * height * 4)

    view.renderer = Capture()
    if fail:
        with pytest.raises(RuntimeError, match='capture failed'):
            view.render_pixels(320, 240, overlays=True)
        assert not scheduled
    else:
        assert view.render_pixels(320, 240, overlays=True)[:2] == (320, 240)
        assert scheduled == [True]
    np.testing.assert_array_equal(view.renderer.world, before)
    np.testing.assert_array_equal(view.packed.instances['world'], before)


def test_loose_edge_without_resident_pixel(view):
    edge = view.scene.mesh.add_edge(QVector3D(), QVector3D(2, 0, 0))
    view.scene.version += 1
    view.set_scene(view.scene)
    # A thin edge need not cover the ID-buffer pixel within the pick tolerance.
    view.pick_row = lambda *args: None
    p = pixel(view, QVector3D(1, 0, 0)) + QPoint(0, 3)
    mouse_move(view, p)
    assert view.tools._mesh_hover == edge
    click(view, Qt.LeftButton, Qt.NoModifier, p)
    assert view.scene.selection == {edge}


def test_hidden_reveal_invalidates_pack(view):
    from views.wgpu_view import _paint_key
    before = _paint_key(view.scene)
    view.scene.show_hidden_geometry = True
    assert _paint_key(view.scene) != before
    geometry = _paint_key(view.scene)
    view.scene.show_hidden_objects = True
    assert _paint_key(view.scene) != geometry


def test_texture_preview_owns_its_materials(view):
    from tools.texture_position import TexturePositionTool
    f = face(view)
    view.source_viewport.active_tool = TexturePositionTool()
    view.tools.sync()
    view.tools.set_suppressed_faces([f])
    assert not getattr(view, 'suppressed_faces', ())
    assert len(view.packed.vertices)


def test_guides_and_tool_overlay_without_labels(view):
    from built_core.guide import Guide
    from views.wgpu_overlay import rasterize, pixels
    from tools.walkthrough import WalkTool
    view.scene.guides.append(Guide(QVector3D(1, 1, 0)))
    view.source_viewport.active_tool = WalkTool()
    view.tools.sync()
    assert not view.scene.dimensions and not view.scene.text_labels
    calls = []
    view.tools.tool.draw_overlay = lambda vp, p: calls.append((vp.width(), vp.height()))
    image = rasterize([], 800, 600, 1, view.tools.draw_annotations)
    assert calls == [(800, 600)]
    assert np.any(pixels(image)[3::4])


def test_move_facade_reads_document_from_native_view(view):
    from tools.move import MoveTool
    from built_render import NativeTools
    mesh = Mesh()
    mesh.add_edge(QVector3D(), QVector3D(1, 0, 0))
    group = Group(mesh)
    group.xform = QMatrix4x4()
    view.scene.groups.append(group)
    view.scene.select([group])
    view.scene.version += 1
    view.set_scene(view.scene)
    old = view.source_viewport
    source = SimpleNamespace(active_tool=MoveTool(), history=old.history,
                             measurementChanged=SimpleNamespace(emit=lambda *a: None),
                             flash_status=lambda *a: None, notify_scene_changed=lambda: None)
    view.source_viewport = source
    native = NativeTools(view)
    event = MouseEvent("press", 400, 300, Qt.LeftButton, Qt.LeftButton, Qt.NoModifier)
    try:
        for point in ((0, 0, 0), (2, 0, 0)):
            view.last_snap = SimpleNamespace(point=QVector3D(*point), kind=None)
            native.click(event)
        assert group.xform.column(3).x() == 2
        source.history.undo()
        assert group.xform.column(3).x() == 0
    finally:
        view.source_viewport = old


def test_context_click_uses_current_frame_and_ignores_faded_occluder(view):
    def triangle(z):
        mesh = Mesh()
        mesh.add_face([QVector3D(0, 0, z), QVector3D(3, 0, z), QVector3D(0, 3, z)])
        return Group(mesh)

    child, outside = triangle(0), triangle(1)
    root = Group()
    root.adopt([child])
    view.scene.groups.extend([root, outside])
    view.scene.version += 1
    view.set_scene(view.scene)
    resident = view.packed
    view.inference_index = prepare(resident)
    shown = []
    def prepare_ids(*, pick_only):
        assert pick_only
        shown.append(view.scene.edit_group)
        view._pick_frame_dirty = False
    view._present_frame = prepare_ids
    view.renderer = SimpleNamespace(
        render=lambda: pytest.fail("Picking must not present a colour frame"), set_rest=lambda *_: None, set_highlight=lambda *_: None,
        pick=lambda *_: resident.groups.index(outside if shown[-1] else child))
    p = pixel(view, QVector3D(1, 1, 0))
    double_click(view, Qt.LeftButton, Qt.NoModifier, p)
    assert view.scene.edit_group is root
    view.set_scene(view.scene)
    click(view, Qt.LeftButton, Qt.NoModifier, p)
    assert shown == [None, root]
    assert view.scene.selection == {child}
    mouse_move(view, p)
    assert not view.hover_rows
    view.source_viewport.end_one_group_edit()
    view.set_scene(view.scene)
    assert view.scene.edit_group is None
    assert view.packed is resident


def test_transformed_component_retains_resident_row_and_picks_world_copy(view):
    mesh = Mesh()
    mesh.add_face([QVector3D(0, 0, 0), QVector3D(2, 0, 0), QVector3D(0, 2, 0)])
    group = Group(mesh)
    group.xform = QMatrix4x4()
    group.xform.translate(1, 0, 0)
    view.scene.groups.append(group)
    view.scene.version += 1
    view.set_scene(view.scene)
    resident = view.packed
    view.pick_row = lambda *_: 0
    view.scene.begin_group_edit(group)
    view.set_scene(view.scene)
    assert view.packed is resident
    p = pixel(view, QVector3D(1.25, .25, 0))
    assert view.tools.pick_face(p.x(), p.y()) == group.mesh.faces[0]
    view.scene.end_one_group_edit()
    view.set_scene(view.scene)
    assert view.packed is resident
    assert group.mesh is mesh


def test_geometry_repack_keeps_previous_frame_until_upload(view, monkeypatch):
    from views import wgpu_view
    scene = view.scene
    mesh = Mesh()
    mesh.add_edge(QVector3D(), QVector3D(1, 0, 0))
    root = Group()
    root.adopt([Group(mesh)])
    scene.groups.append(root)
    scene.version += 1
    view.set_scene(scene)
    old = view.packed
    events = []
    pack = wgpu_view.pack_scene

    def replacement(*args, **kw):
        assert view.packed is old
        events.append("pack")
        return pack(*args, **kw)

    def upload():
        assert view.packed is not old
        assert len(view.packed.instances) == 1
        events.append("upload")

    monkeypatch.setattr(wgpu_view, 'pack_scene', replacement)
    monkeypatch.setattr(view, '_upload', upload)
    monkeypatch.setattr(view, 'blank', lambda: pytest.fail("cleared resident frame"))
    mesh.add_edge(QVector3D(1, 0, 0), QVector3D(2, 0, 0))
    # A geometry edit alone rebakes its prototype in place now; showing
    # hidden geometry changes the baked paint and still packs it all.
    scene.show_hidden_geometry = True
    scene.version += 1
    view.set_scene(scene)
    assert events == ["pack", "upload"]

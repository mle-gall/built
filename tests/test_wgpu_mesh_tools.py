"""Native routing invokes the document tools, including History and cancellation."""
from types import SimpleNamespace
import pytest
from built_core import Input as Qt
from built_core import ScreenPoint as QPointF
from built_core import Vec3 as QVector3D
from built_core.scene import Scene
from built_core.mesh import Mesh
from built_core.group import Group
from built_core.history import History
from tools.rust_tools import LineTool
from tools.rust_tools import RectangleTool
from tools.base import ToolContext
from built_render import NativeTools
from built_core import pack_scene
from built_core import InstanceUpdates


def harness(tool, inside=False):
    scene = Scene()
    if inside:
        group = Group(Mesh())
        scene.groups.append(group)
        scene.begin_group_edit(group)
    history = History(scene)
    source = SimpleNamespace(scene=scene, history=history, active_tool=tool,
        flash_status=lambda *a: None, measurementChanged=SimpleNamespace(emit=lambda *a: None),
        _measurement_text=lambda: '')
    updates = InstanceUpdates(scene, pack_scene(scene))
    updates.observe(history)
    def refresh(scene):
        updates.apply(*updates.diff(scene))
    view = SimpleNamespace(source_viewport=source, scene=scene, overlay_primitives=[],
        update=lambda: None, set_scene=refresh, _world_to_pixel=lambda p: (p.x(), p.y()))
    native = NativeTools(view)
    native.sync()
    return scene, history, native, updates


def event():
    return SimpleNamespace(position=lambda: QPointF(), modifiers=lambda: Qt.NoModifier)


def click(native, point):
    native.view.last_snap = SimpleNamespace(point=QVector3D(*point), kind='endpoint', context=None)
    native.click(event())


def geometry(mesh):
    return sorted(tuple(v.position.toTuple()) for v in mesh.vertices), len(mesh.edges), len(mesh.faces)


@pytest.mark.parametrize('tool_type', [LineTool, RectangleTool])
@pytest.mark.parametrize('inside', [False, True])
def test_draw_commands_match_direct_tool_and_restore(tool_type, inside):
    scene, history, native, updates = harness(tool_type(), inside)
    gl_scene, gl_history, direct, _ = harness(tool_type(), inside)
    before = geometry(scene.mesh)
    for p in [(0, 0, 0), (2, 3, 0)]:
        click(native, p)
        ctx = ToolContext(direct, QVector3D(*p), QPointF(), Qt.NoModifier,
                          SimpleNamespace(kind='endpoint', context=None))
        direct.tool.on_hover(ctx)
        direct.tool.on_click(ctx)
    assert type(history.undo_stack[-1]) is type(gl_history.undo_stack[-1])
    assert geometry(scene.mesh) == geometry(gl_scene.mesh)
    history.undo()
    delta = updates.diff(scene)
    assert len(updates.changed) == 1
    gl_history.undo()
    assert geometry(scene.mesh) == geometry(gl_scene.mesh)
    assert geometry(scene.mesh)[1:] == before[1:]
    updates.apply(*delta)
    history.redo()
    updates.diff(scene)
    assert len(updates.changed) == 1
    native.cancel()
    assert native.tool.start_point is None
    assert not native.view.overlay_primitives

from tools.pushpull import PushPullTool


@pytest.mark.parametrize('inside', [False, True])
def test_pushpull_numeric_commit_and_cancel(inside):
    scene, history, native, updates = harness(PushPullTool(), inside)
    face = scene.mesh.add_face([QVector3D(0, 0, 0), QVector3D(2, 0, 0),
                                QVector3D(2, 2, 0), QVector3D(0, 2, 0)])
    native.pick_face_any = lambda *a: (face, None)
    native._ray_hit_plane = lambda *a: QVector3D(1, 1, 0)
    native.update()
    before = geometry(scene.mesh)
    click(native, (1, 1, 0))
    assert native.tool.dragging
    assert native.tool.on_value(native, 1.)
    assert len(history.undo_stack) == 1
    assert len(scene.mesh.faces) > before[2]
    assert len(updates.changed) == 1
    history.undo()
    native.update()
    assert geometry(scene.mesh) == before
    history.redo()
    native.update()
    assert len(updates.changed) == 1
    native.cancel()
    assert not native.tool.dragging
    assert not native.view.overlay_primitives

from tools.eraser import EraserTool


@pytest.mark.parametrize('inside', [False, True])
def test_erase_stroke_commits_on_release(inside):
    scene, history, native, updates = harness(EraserTool(), inside)
    edge = scene.mesh.add_edge(QVector3D(), QVector3D(1, 0, 0))
    native.pick_visible_edge = lambda *a: edge
    native.update()
    click(native, (0, 0, 0))
    assert not history.undo_stack
    native.tool.on_release(native)
    assert len(history.undo_stack) == 1
    assert not scene.mesh.edges
    # Inside a group that started empty the placement never had a GPU row;
    # emptied again it retires instead of rebaking (3689a2ae).
    assert len(updates.changed) == (0 if inside else 1)
    history.undo()
    native.update()
    assert edge in scene.mesh.edges
    assert len(updates.changed) == 1
    click(native, (0, 0, 0))
    native.cancel()
    native.tool.on_release(native)
    assert edge in scene.mesh.edges
    assert not native.view.overlay_primitives

from tools.move import MoveTool


@pytest.mark.parametrize('inside', [False, True])
@pytest.mark.parametrize('target', ['face', 'edge', 'vertex'])
def test_move_loose_preview_cancel_commit(inside, target):
    scene, history, native, updates = harness(MoveTool(), inside)
    face = scene.mesh.add_face([QVector3D(), QVector3D(2, 0, 0), QVector3D(0, 2, 0)])
    if target != 'vertex':
        scene.select([face if target == 'face' else scene.mesh.edges[0]])
    else:
        native.pick_group = lambda *a: None
        native.pick_edge = lambda *a: None
        native.pick_face = lambda *a: None
    native.update()
    before = geometry(scene.mesh)
    click(native, (0, 0, 0))
    native.view.last_snap = SimpleNamespace(point=QVector3D(0, 0, 1), kind='endpoint', context=None)
    native.hover(event())
    assert geometry(scene.mesh) != before
    native.cancel()
    assert geometry(scene.mesh) == before
    assert not native.view.overlay_primitives
    click(native, (0, 0, 0))
    click(native, (0, 0, 1))
    assert history.undo_stack
    assert len(updates.changed) <= 1
    history.undo()
    native.update()
    assert geometry(scene.mesh) == before


def test_rectangle_vcb_dimensions_and_units():
    from built_render import KeyEvent
    scene, history, native, _ = harness(RectangleTool())
    click(native, (0, 0, 0))
    native.view.last_snap = SimpleNamespace(point=QVector3D(1, 1, 0), kind='endpoint', context=None)
    native.hover(event())
    for char in '200cm;300cm':
        assert native.key(KeyEvent(True, 0, 0, char))
    assert native.key(KeyEvent(True, Qt.Key_Return.value, 0))
    assert len(scene.mesh.faces) == 1
    points = [v.position for v in scene.mesh.vertices]
    assert max(p.x() for p in points) == pytest.approx(2.)
    assert max(p.y() for p in points) == pytest.approx(3.)
    assert not native.value_buffer
    assert history.undo_stack


def test_typed_rectangle_uses_moved_document_axes():
    from built_core import Matrix4
    from built_render import KeyEvent

    scene, _, native, _ = harness(RectangleTool())
    frame = Matrix4()
    frame.translate(4, 3, 1)
    frame.rotate(30, 0, 0, 1)
    scene.world_drawing_frame = frame
    origin = frame.map(QVector3D())
    click(native, origin.toTuple())
    native.view.last_snap = SimpleNamespace(
        point=frame.map(QVector3D(1, 1, 0)), kind='endpoint', context=None)
    native.hover(event())
    for char in '2;3':
        assert native.key(KeyEvent(True, 0, 0, char))
    assert native.key(KeyEvent(True, Qt.Key_Return.value, 0))
    assert len(scene.mesh.faces) == 1
    points = [v.position for v in scene.mesh.vertices]
    for x, y in [(0, 0), (2, 0), (0, 3), (2, 3)]:
        corner = frame.map(QVector3D(x, y, 0))
        assert any((p - corner).length() < 1e-5 for p in points)


def test_axes_align_to_a_picked_occurrence_face():
    from built_core import Matrix4
    from built_core.group import frame_axes
    from tools.change_axes import ChangeAxesTool

    tool = ChangeAxesTool()
    scene, history, native, _ = harness(tool)
    mesh = Mesh()
    face = mesh.add_face([QVector3D(0, 0, 0), QVector3D(0, 1, 0),
                          QVector3D(0, 1, 4), QVector3D(0, 0, 4)])
    group = Group(mesh)
    group.xform = Matrix4()
    group.xform.translate(4, 8, 5)
    group.xform.rotate(90, 0, 0, 1)
    scene.groups.append(group)
    native.pick_face_placement = lambda *args: (face, group)
    tool.align_face = True
    click(native, (4, 8, 5))
    origin, red, _, blue = frame_axes(scene.drawing_frame)
    assert (origin - QVector3D(3, 8, 5)).length() < 1e-6
    assert (blue - QVector3D(0, 1, 0)).length() < 1e-6
    assert (red - QVector3D(0, 0, 1)).length() < 1e-6
    assert history.undo() and scene.drawing_frame is None


def test_protractor_follows_a_tilted_drawing_plane():
    from built_core import Matrix4
    from tools.protractor import ProtractorTool

    scene, _, native, _ = harness(ProtractorTool())
    frame = Matrix4()
    frame.rotate(38, 1, 0, 0)
    frame.rotate(25, 0, 0, 1)
    scene.world_drawing_frame = frame
    native.sync()
    u, v = native.tool._basis()
    for value, direction in [(QVector3D(*u), QVector3D(1, 0, 0)),
                             (QVector3D(*v), QVector3D(0, 1, 0)),
                             (native.tool._axis(), QVector3D(0, 0, 1))]:
        assert (value - frame.mapVector(direction)).length() < 1e-6

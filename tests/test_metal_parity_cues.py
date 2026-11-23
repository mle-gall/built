"""Interaction cues the Metal view paints with GL's own code (docs/metal-parity-audit.md).

Device-free: painters run on QImages, renderer calls are recorded.
"""
import os
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
import math
from types import SimpleNamespace
import numpy as np
import pytest
from PySide6.QtCore import QPointF, Qt
from PySide6.QtGui import QMouseEvent
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V
from PySide6.QtWidgets import QApplication
from core.camera import OrbitCamera as Camera
from core.group import Group
from core.mesh import Mesh
from core.scene import Scene
from core.snap import SnapResult
from views.viewport import Viewport
from views.wgpu_annotations import AnnotationLayout, draw_tool_feedback
from views.wgpu_overlay import draw_snap_indicator, pixels
from views.wgpu_scene import VERTEX, pack_scene
from tests.test_metal_parity_overlays import equal, painted

SIZE = (360, 200)


@pytest.fixture(scope="module", autouse=True)
def app():
    return QApplication.instance() or QApplication([])


def camera():
    cam = Camera()
    cam.distance = 12
    cam.aspect = SIZE[0] / SIZE[1]
    return cam


def tools_for(tool, snap=None, typed=""):
    """What draw_tool_feedback reads off NativeTools."""
    return SimpleNamespace(active_tool=tool, last_snap=snap, value_buffer=typed)


def gl_reference(tool, snap=None, typed=""):
    """The GL viewport's state for the same frame, on GL's own methods."""
    gl = AnnotationLayout(Scene(), camera(), SIZE)
    gl.active_tool, gl.last_snap, gl._value_buffer = tool, snap, typed
    gl._route_marker = None
    return gl


def metal(tool, snap=None, typed=""):
    layout = AnnotationLayout(Scene(), camera(), SIZE)
    return painted(lambda p: draw_tool_feedback(p, tools_for(tool, snap, typed), layout), SIZE)


def line_tool(**extra):
    return SimpleNamespace(rubber_band_lines=lambda: [(V(-3, 0, 0), V(3, 0, 1))],
                           wireframe_color=None, **extra)


@pytest.mark.parametrize("kind", [None, "axis", "axis_inference", "reference", "close"])
def test_rubber_band_has_gl_colour_and_width(kind):
    # Metal drew every band in one cyan 1.5 px pen: an axis-locked line
    # did not turn red/green/blue.
    snap = None if kind is None else SnapResult(kind=kind, point=V(3, 0, 1),
                                                color=(.86, .22, .27), axis="x")
    tool = line_tool()
    gl = gl_reference(tool, snap)

    def reference(p):
        segments, color, width = Viewport._rubber_band_style(gl)
        gl._overlay_rubber = (segments, color, width)
        Viewport._draw_rubber_band_overlay(gl, p)
        Viewport._draw_length_label(gl, p)
    expected = painted(reference, SIZE)
    actual = metal(tool, snap)
    equal(actual, expected)
    assert pixels(actual).any()
    if kind == "axis":
        rgba = pixels(actual).reshape(-1, 4)
        reddish = (rgba[:, 0] > 180) & (rgba[:, 1] < 90) & (rgba[:, 3] > 200)
        assert reddish.sum() > 20


def test_depth_tested_band_goes_to_the_native_preview_not_the_overlay():
    # Push/Pull's forming box is hidden by its faces in GL (depth-tested).
    from views.wgpu_mesh_tools import MeshToolAccess
    tool = line_tool(wireframe_depth_tested=True)
    tool.wireframe_color = (.13, .17, .23, 1.)
    uploads = []
    renderer = SimpleNamespace(set_preview_mesh=lambda *a: uploads.append(a))
    access = MeshToolAccess()
    access.tool = tool
    access.view = SimpleNamespace(renderer=renderer, last_snap=None)
    access.active_tool = tool
    access.last_snap = None
    access.measurementChanged = SimpleNamespace(emit=lambda *a: None)
    access.mesh_feedback()
    access.mesh_feedback()          # unchanged preview: not uploaded again
    assert len(uploads) == 1
    edges = np.frombuffer(bytes(uploads[0][1]), VERTEX)
    assert len(edges) == 2
    assert tuple(edges['color'][0]) == (33, 43, 59, 255)
    # Not painted in the overlay (the band only; the label still is).
    gl = gl_reference(tool)
    expected = painted(lambda p: Viewport._draw_length_label(gl, p), SIZE)
    equal(metal(tool), expected)


def test_guide_preview_is_dashed_like_gl():
    tool = SimpleNamespace(guide_preview_lines=lambda: [(V(-50, 0, 0), V(50, 0, 0))])
    gl = gl_reference(tool)
    expected = painted(lambda p: Viewport._draw_guide_preview(gl, p), SIZE)
    actual = metal(tool)
    equal(actual, expected)
    assert pixels(actual).any()


def test_typed_value_label_is_warm_like_gl():
    tool = line_tool(value_label=lambda: ("2.50 m", V(0, 0, 0)))
    gl = gl_reference(tool, typed="2.5")

    def reference(p):
        gl._overlay_rubber = Viewport._rubber_band_style(gl)
        Viewport._draw_rubber_band_overlay(gl, p)
        Viewport._draw_length_label(gl, p)
    equal(metal(tool, typed="2.5"), painted(reference, SIZE))


def test_scale_grips_and_box_match_gl():
    box = [V(x, y, 0) for x, y in ((-2, -1), (2, -1), (2, 1), (-2, 1))]
    state = {"segments": list(zip(box, box[1:] + box[:1])),
             "grips": [(box[0], "idle"), (box[1], "hover"), (box[2], "anchor")]}
    tool = SimpleNamespace(scale_box_state=lambda: state)
    gl = gl_reference(tool)
    expected = painted(lambda p: Viewport._draw_scale_box(gl, p), SIZE)
    actual = metal(tool)
    equal(actual, expected)
    assert pixels(actual).any()


def test_push_pull_level_with_marker_matches_gl():
    tool = SimpleNamespace(inference_marker=lambda: (V(1, 0, 0), "edge"),
                           inference_guide_lines=lambda: [(V(-2, 0, 0), V(1, 0, 0), "edge")])
    gl = gl_reference(tool)
    expected = painted(lambda p: Viewport._draw_inference_marker(gl, p), SIZE)
    actual = metal(tool)
    equal(actual, expected)
    assert pixels(actual).any()


def test_route_marker_and_extension_overlays_paint():
    tool = SimpleNamespace()
    calls = []
    layout = AnnotationLayout(Scene(), camera(), SIZE)
    native = tools_for(tool)
    native._route_marker = V(0, 0, 0)
    native._ext_overlays = [lambda vp, p: calls.append(vp)]
    image = painted(lambda p: draw_tool_feedback(p, native, layout), SIZE)
    assert calls == [native]
    gl = gl_reference(tool)
    gl._route_marker = V(0, 0, 0)
    equal(image, painted(lambda p: Viewport._draw_route_marker(gl, p), SIZE))


def test_snap_tooltip_is_painted():
    # GL labels the marker ("Endpoint in group"); Metal passed labels=False.
    snap = SnapResult(kind="endpoint", point=V(60, 50, 0), color=(.16, .62, .36))
    snap.context = "group"
    project = lambda p: (p.x(), p.y())
    clip = lambda a, b: (a, b)
    gl = SimpleNamespace(_world_to_pixel=project, _clip_pixel_line=clip,
                         _SNAP_LABELS=Viewport._SNAP_LABELS)
    gl._snap_context_suffix = lambda c: Viewport._snap_context_suffix(gl, c)
    expected = painted(lambda p: Viewport._draw_snap_indicator(gl, p, snap), SIZE)
    bare = painted(lambda p: draw_snap_indicator(p, snap, project, clip, labels=False), SIZE)
    assert np.abs(pixels(expected).astype(int) - pixels(bare).astype(int)).max() > 1
    from views.wgpu_mesh_tools import MeshToolAccess
    import inspect
    source = inspect.getsource(MeshToolAccess.draw_annotations)
    assert "labels=False" not in source


# ---- view-level cues --------------------------------------------------------

@pytest.fixture
def view():
    from views.wgpu_view import WgpuView
    from views.wgpu_inference import prepare
    scene = Scene()
    mesh = Mesh()
    mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    group = Group(mesh, name='placed')
    group.xform = QMatrix4x4()
    group.xform.translate(4, 3, 0)
    scene.groups.append(group)
    v = WgpuView()
    v.resize(800, 600)
    v.scene = scene
    v.packed = pack_scene(scene)
    v.uid_rows = {uid: [i] for i, uid in enumerate(v.packed.uids)}
    v.camera.target = V(5, 4, 0)
    v.camera.pitch = math.pi/2-.001
    v.camera.distance = 8
    v.camera.aspect = 800/600
    v.inference_index = prepare(v.packed)
    v.calls = {}
    record = lambda name: (lambda *a: v.calls.__setitem__(name, a))
    v.renderer = SimpleNamespace(set_highlight=record('highlight'), set_rest=lambda *a: None,
                                 set_face_marks=record('marks'),
                                 set_preview_mesh=record('preview'),
                                 pick=lambda *a: 0, pick_rect=lambda *a: [0])
    v.source_viewport = SimpleNamespace(plano_style=None, style_override=None,
                                        _preview_groups=(), _hover_entity=None)
    yield v
    v.renderer = None
    v.close()


def test_tool_hovered_face_wears_dots_through_its_placement(view):
    # GL dots the face under Push/Pull, Paint, Offset...; Metal outlined it.
    group = view.scene.groups[0]
    face = group.mesh.faces[0]
    view.tools.mode = 'mesh'
    view.tools.set_hover(face, SimpleNamespace(xform=group.xform))
    view.sync_selection()
    marks = np.frombuffer(bytes(view.calls['marks'][0]), VERTEX)
    assert len(marks) == 6
    xs, ys = marks['pos'][:, 0], marks['pos'][:, 1]
    assert (xs.min(), xs.max(), ys.min(), ys.max()) == (4, 6, 3, 5)
    assert tuple(marks['color'][0][:3]) == (242, 114, 40)
    # Select never hovers a face, and a selected face shows its own dots.
    view.tools.mode = 'select'
    view.sync_selection()
    assert len(view.calls['marks'][0]) == 0


def test_selected_billboard_is_outlined_not_tinted(view):
    from views.wgpu_edit_context import billboard_outline_primitives
    group = view.scene.groups[0]
    group.billboard = True
    quad = [V(4, 3, 0), V(6, 3, 0), V(6, 3, 2), V(4, 3, 2)]
    view.source_viewport._billboard_quad = lambda g: (quad, None)
    assert billboard_outline_primitives(view) == []
    view.scene.selection = {group}
    lines = billboard_outline_primitives(view)
    assert len(lines) == 4 and all(line.color == '#f2732a' for line in lines)
    assert lines[0].points == (view._world_to_pixel(quad[0]), view._world_to_pixel(quad[1]))
    view._rows_for_group = lambda g: [0]
    view.sync_selection()
    assert view.calls['highlight'][0] == []
    group.billboard = False
    view._selection_signature = None
    view.sync_selection()
    assert view.calls['highlight'][0] == [0]


def test_selection_rectangle_waits_for_the_drag_threshold(view):
    view.tools.sync = lambda: 'select'
    view.tools.mode = 'select'
    view.source_viewport.nav_mode = None
    a, b, c = QPointF(100, 100), QPointF(102, 101), QPointF(140, 130)
    QApplication.sendEvent(view, QMouseEvent(QMouseEvent.MouseButtonPress, a, a,
        Qt.LeftButton, Qt.LeftButton, Qt.NoModifier))
    QApplication.sendEvent(view, QMouseEvent(QMouseEvent.MouseMove, b, b,
        Qt.NoButton, Qt.LeftButton, Qt.NoModifier))
    assert view.overlay_primitives == []
    QApplication.sendEvent(view, QMouseEvent(QMouseEvent.MouseMove, c, c,
        Qt.NoButton, Qt.LeftButton, Qt.NoModifier))
    assert [p.kind for p in view.overlay_primitives] == ['rectangle']


def _preview_edges(view):
    return np.frombuffer(bytes(view.calls['preview'][1]), VERTEX)


def test_selection_box_is_a_depth_tested_preview_edge(view):
    # GL draws the box in the scene pass (its back edges hidden by the
    # group); an overlay line showed all twelve through the geometry.
    group = view.scene.groups[0]
    view.scene.selection = {group}
    view.sync_selection()
    edges = _preview_edges(view)
    assert len(edges) >= 16 and len(edges) % 2 == 0
    assert {tuple(c) for c in edges['color']} == {(242, 114, 40, 255)}
    assert edges['pos'][:, 0].min() == pytest.approx(4) and edges['pos'][:, 0].max() == pytest.approx(6)
    assert edges['pos'][:, 1].min() == pytest.approx(3) and edges['pos'][:, 1].max() == pytest.approx(5)
    # A move commit measures again.
    group.xform.translate(1, 0, 0)
    view.scene.version += 1
    view.sync_selection()
    assert _preview_edges(view)['pos'][:, 0].max() == pytest.approx(7)


def test_mesh_tool_preview_carries_the_box_except_while_moving_it(view):
    group = view.scene.groups[0]
    view.scene.selection = {group}
    tool = line_tool()
    view.tools.tool = tool
    view.tools.mode = 'mesh'
    view.tools.mesh_feedback()
    assert len(_preview_edges(view)) >= 16
    view.calls.pop('preview')
    view.tools.mesh_feedback()                  # unchanged: no upload
    assert 'preview' not in view.calls
    view.tools._moving_groups = (group,)        # Move/Rotate drag preview
    view.tools.mesh_feedback()
    assert len(_preview_edges(view)) == 0

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
from PySide6.QtGui import QMatrix4x4, QMouseEvent, QVector3D as V
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
    expected = painted(lambda p: Viewport._draw_snap_indicator(gl, p, snap), SIZE)
    bare = painted(lambda p: draw_snap_indicator(p, snap, project, clip, labels=False), SIZE)
    assert np.abs(pixels(expected).astype(int) - pixels(bare).astype(int)).max() > 1
    from views.wgpu_mesh_tools import MeshToolAccess
    import inspect
    source = inspect.getsource(MeshToolAccess.draw_annotations)
    assert "labels=False" not in source

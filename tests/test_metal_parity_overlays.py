"""QImage parity with actual GL painter methods; no mapped widgets or GPU needed."""
import os
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
from copy import copy
from types import SimpleNamespace
import numpy as np
import pytest
from PySide6.QtCore import QPointF, Qt
from PySide6.QtGui import QColor, QImage, QPainter
from ingetrazo_core import Vec3 as V
from PySide6.QtWidgets import QApplication
from core.camera import OrbitCamera as Camera
from core.scene import Scene
from core.dimension import Dimension
from core.textlabel import TextLabel
from core.guide import Guide
from core.section import SectionPlane
from core.snap import SnapResult
from views.document_interaction import _guide_vertices, shown_guides
from tests.controller_surface import ControllerSurface as Viewport
from views.wgpu_overlay import draw_snap_indicator, pixels, rasterize, selection_rectangle
from views.wgpu_annotations import (
    AnnotationLayout, composite_annotations, draw_annotations, draw_tool_overlays,
    guide_segments,
)


@pytest.fixture(scope="module", autouse=True)
def app():
    return QApplication.instance() or QApplication([])


def painted(draw, size=(360, 100), dpr=1):
    image = QImage(int(size[0] * dpr), int(size[1] * dpr), QImage.Format_RGBA8888)
    image.setDevicePixelRatio(dpr)
    image.fill(Qt.transparent)
    painter = QPainter(image)
    painter.setRenderHint(QPainter.Antialiasing)
    try:
        draw(painter)
    finally:
        painter.end()
    return image


def equal(a, b):
    assert a.size() == b.size()
    delta = np.abs(pixels(a).astype(int) - pixels(b).astype(int))
    assert delta.max(initial=0) <= 1


KINDS = ("endpoint origin component_origin on_edge on_axis intersection_edge "
         "intersection_face on_line extension from_point aligned tangent midpoint "
         "arc_midpoint on_face close reference through_point perp_face intersection "
         "center axis axis_inference none").split()


@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("dpr", [1, 2])
def test_snap_matches_gl(kind, dpr):
    snap = SnapResult(kind=kind, point=V(48, 40, 0), color=(.16, .62, .36),
                      guide=(V(80, 40, 0), V(325, 40, 0)),
                      guide_color=(.86, .22, .27))
    snap.guides = [(V(80, 65, 0), V(325, 65, 0), (.2, .4, .78))]
    snap.axis = "z"
    snap.context = "component"
    project = lambda p: (p.x(), p.y())
    clip = lambda a, b: (a, b)
    reference = SimpleNamespace(_world_to_pixel=project, _clip_pixel_line=clip,
                                _SNAP_LABELS=Viewport._SNAP_LABELS)
    reference._snap_context_suffix = (
        lambda c: Viewport._snap_context_suffix(reference, c))
    expected = painted(lambda p: Viewport._draw_snap_indicator(reference, p, snap), dpr=dpr)
    actual = painted(lambda p: draw_snap_indicator(p, snap, project, clip), dpr=dpr)
    equal(actual, expected)
    if kind in ("axis", "axis_inference"):
        assert not pixels(actual).any()


@pytest.mark.parametrize("style", ["window", "crossing", "zoom"])
@pytest.mark.parametrize("dpr", [1, 2])
def test_rectangle_matches_gl(style, dpr):
    a, b = (30, 20), (280, 80)
    if style == "crossing":
        a, b = b, a
    box = selection_rectangle(a, b, zoom=style == "zoom")
    actual = rasterize([box], 360, 100, dpr)
    color = {"window": "#3366c7", "crossing": "#289e5c", "zoom": "#f37329"}[style]
    assert box.color == color
    assert box.pen_style == (Qt.SolidLine if style == "window" else Qt.DashLine)
    assert actual.pixelColor(40 * dpr, 40 * dpr).alpha() == (26 if style == "zoom" else 28)
    assert actual.pixelColor(10 * dpr, 40 * dpr).alpha() == 0


def fixture():
    scene = Scene()
    scene.dimensions = [Dimension(V(-2, 0, 0), V(2, 0, 0), V(0, -1, 0))]
    scene.text_labels = [TextLabel(V(0, 0, 0), V(-1, 0, 2), "Leader\nsecond line")]
    scene.guides = [Guide(V(0, 0, 1), V(1, 0, 0)),
                    Guide(V(-2, 0, 1), origin=V(-2, 0, 0))]
    scene.section_planes = [SectionPlane(V(0, 0, 0), V(0, 0, 1), active=False)]
    camera = Camera()
    camera.distance = 12
    return scene, camera


@pytest.mark.parametrize("size,dpr", [((640, 400), 1), ((1280, 800), 2),
                                     ((500, 700), 1), ((1000, 1400), 2)])
@pytest.mark.parametrize("only", [True, False])
def test_export_layout_matches_gl_at_output_aspect(size, dpr, only):
    scene, camera = fixture()
    old_aspect = camera.aspect
    # Reference uses a real (never shown) GL viewport's painter code at the
    # virtual sheet size. No GL context, render, or native window is requested.
    gl = Viewport()
    gl.scene = scene
    gl.camera = copy(camera)
    logical_height = size[1] * 640 / size[0]
    gl.resize(640, round(logical_height))
    gl.width = lambda: 640
    gl.height = lambda: logical_height
    gl.camera.aspect = 640 / logical_height
    gl._hover_entity = None
    gl._occluded_on_screen = lambda p: False
    gl._depth_snap = None
    gl._preview_groups = None
    def reference(p):
        p.scale(size[0] / 640 / dpr, size[0] / 640 / dpr)
        if not only:
            # GL guides are native world segments; paint their projection to
            # compare dash placement without asserting GPU AA/depth parity.
            from views.wgpu_annotations import draw_guide_lines
            draw_guide_lines(p, gl)
            for name in ("_draw_guides", "_draw_section_planes", "_draw_geo_surfaces",
                         "_draw_geo_paths", "_draw_geo_points"):
                getattr(gl, name)(p)
        gl._draw_dimensions(p)
        gl._draw_text_labels(p)
    expected = painted(reference, (size[0] / dpr, size[1] / dpr), dpr)
    blank = painted(lambda p: None, (size[0] / dpr, size[1] / dpr), dpr)
    actual = composite_annotations(blank, scene, camera, logical_size=(640, 400),
                                   annotations_only=only)
    # Keep the byte-level check of shared projection, selection and pen layout.
    def layout_reference(painter):
        painter.scale(size[0] / 640 / dpr, size[0] / 640 / dpr)
        draw_annotations(painter, scene, camera, (640, logical_height), annotations_only=only)
    equal(painted(layout_reference, (size[0] / dpr, size[1] / dpr), dpr), expected)

    # tiny-skia and the platform painter sample strokes and font outlines
    # differently. Compare coverage and location in both directions.
    def alpha(image):
        return pixels(image).reshape(size[1], size[0], 4)[:, :, 3]
    a, b = alpha(actual), alpha(expected)
    assert 0.80 < a.sum() / b.sum() < 1.20
    def within_stroke(mask):
        near = mask.copy()
        for _ in range(3 * dpr):
            prev = near.copy()
            near[1:] |= prev[:-1]
            near[:-1] |= prev[1:]
            prev = near.copy()
            near[:, 1:] |= prev[:, :-1]
            near[:, :-1] |= prev[:, 1:]
        return near
    for observed, target in ((a, b), (b, a)):
        assert within_stroke(target > 24)[observed > 24].mean() > .95
    assert pixels(actual).sum() > 0
    assert not pixels(blank).any()
    assert camera.aspect == old_aspect
    equal(composite_annotations(actual, scene, camera, logical_size=(640, 400),
                                overlays=False), actual)
    gl.close()


def test_guide_world_dash_geometry_and_selection():
    scene, camera = fixture()
    scene.select([scene.guides[0]])
    segments, flags = guide_segments(scene, camera)
    coords, spans = _guide_vertices(shown_guides(scene), camera.distance * .03,
                                   scene.selection, centre=camera.target,
                                   reach=camera.distance * 15, eye=camera.eye())
    np.testing.assert_array_equal(segments.reshape(-1), np.asarray(coords))
    assert flags[:spans[0][1] // 2].all()
    assert not flags[spans[0][1] // 2:].any()
    # The world-space dash occupies half each period.
    assert np.isclose(np.linalg.norm(segments[0, 1] - segments[0, 0]) /
                      np.linalg.norm(segments[1, 0] - segments[0, 0]), .5, atol=3e-5)


@pytest.mark.parametrize("tool_name", ["walk", "flip"])
def test_tool_overlay_on_empty_annotation_document(tool_name):
    scene, camera = fixture()
    scene.dimensions = []
    scene.text_labels = []
    layout = AnnotationLayout(scene, camera, (360, 100))
    if tool_name == "walk":
        from tools.walkthrough import WalkTool
        tool = WalkTool()
        tool._anchor_px = QPointF(180, 50)
    else:
        from tools.flip import FlipTool
        from core.mesh import Mesh
        from core.group import Group
        mesh = Mesh()
        mesh.add_face([V(-1, -1, 0), V(1, -1, 0), V(0, 1, 1)])
        group = Group(mesh)
        scene.groups.append(group)
        scene.select([group])
        tool = FlipTool()
    layout.tool = tool
    expected = painted(lambda p: tool.draw_overlay(layout, p))
    actual = painted(lambda p: draw_tool_overlays(p, layout, labels=False))
    equal(actual, expected)
    assert pixels(actual).sum() > 0


@pytest.mark.parametrize("label", ["axis", "reference", "plane", "inference", "linear"])
def test_lock_labels_match_gl(label):
    ns = SimpleNamespace(reference_mode=None, reference_edge=None, axis_lock=None,
                         last_snap=None, linear_inference_mode="all", height=lambda: 100)
    ns.tool = ns.active_tool = SimpleNamespace(plane_lock=None)
    method = None
    if label == "axis":
        ns.axis_lock = "x"
        method = Viewport._draw_axis_lock_label
    elif label == "reference":
        ns.reference_mode, ns.reference_edge = "parallel", object()
        method = Viewport._draw_reference_label
    elif label == "plane":
        ns.tool.plane_lock = "y"
        method = Viewport._draw_plane_lock_label
    elif label == "inference":
        ns.last_snap = SnapResult(kind="axis", point=V(), color=(.2, .4, .78), axis="z")
        method = Viewport._draw_inference_label
    else:
        ns.linear_inference_mode = "off"
        method = Viewport._draw_linear_mode_label
    equal(painted(lambda p: draw_tool_overlays(p, ns)),
          painted(lambda p: method(ns, p)))


def test_capture_occlusion_callback_and_painter_state():
    scene, camera = fixture()
    layout = AnnotationLayout(scene, camera, (360, 100), occluded=lambda p: p.x() > 0)
    assert layout._project_samples([V(-1, 0, 0), V(1, 0, 0)])[1][1] is False
    def paint(p):
        before = (p.pen(), p.brush(), p.font(), p.transform())
        draw_annotations(p, scene, camera, (360, 100), annotations_only=True)
        assert before == (p.pen(), p.brush(), p.font(), p.transform())
    painted(paint)


def test_clear_dimension_stroke_covers_arrows_and_label_for_this_frame():
    scene, camera = fixture()
    a, b = V(-1, 0, 0), V(1, 0, 0)
    reads = []
    layout = AnnotationLayout(scene, camera, (360, 100),
                              clear_segment=lambda a, b: True,
                              occluded=lambda p: reads.append(p) or True)
    painted(lambda painter: layout._draw_occluded_segment(painter, a, b))
    assert not any(layout._occluded_on_screen(p) for p in [a, b, (a + b) * .5])
    assert not reads
    assert layout._occluded_on_screen(V(0, 1, 0))


def test_capture_marks_composite_over_straight_alpha_and_escape_label_text():
    from ingetrazo_render import AnnotationCanvas
    from PySide6.QtCore import QPointF
    from PySide6.QtGui import QColor, QPen
    target = AnnotationCanvas(96, 36, 96)
    base = bytes([0, 0, 255, 128]) * (96 * 36)
    assert target.composite(base) is base
    target.fillRect(0, 0, 8, 8, QColor(255, 0, 0, 128))
    target.setPen(QPen(QColor(0, 255, 0)))
    target.drawText(QPointF(10, 24), "A < B & C")
    pixels = np.frombuffer(target.composite(base), np.uint8).reshape(36, 96, 4)
    np.testing.assert_allclose(pixels[2, 2], [170, 0, 85, 192], atol=1)
    assert pixels[:, 10:, 1].max() > 160
    assert pixels[35, 95].tolist() == list(base[:4])

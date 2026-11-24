"""Compare native annotation layout, tool overlays and offscreen RGBA captures."""

from copy import copy
from types import SimpleNamespace
import numpy as np
import pytest
from core.input_types import QPointF, Qt
from ingetrazo_render import AnnotationCanvas, PaintColor, PaintPen, SurveyRaster
from tests.raster_samples import pixels
from ingetrazo_core import Vec3 as V
from core.camera import OrbitCamera as Camera
from core.scene import Scene
from core.dimension import Dimension
from core.textlabel import TextLabel
from core.guide import Guide
from core.section import SectionPlane
from core.snap import SnapResult
from views.document_interaction import _guide_vertices, shown_guides
from tests.controller_surface import ControllerSurface as Viewport
from views.wgpu_overlay import draw_snap_indicator, paint_layer, selection_rectangle
from views.wgpu_annotations import (
    AnnotationLayout, composite_pixels, draw_annotations, draw_tool_overlays,
    guide_segments,
)


def painted(draw, size=(360, 100), dpr=1, *, logical_width=None):
    width, height = (int(n * dpr) for n in size)
    canvas = AnnotationCanvas(width, height, float(size[0] if logical_width is None else logical_width))
    draw(canvas)
    return SurveyRaster(width, height, canvas.composite(bytes(width * height * 4)))


def equal(a, b):
    assert (a.width, a.height) == (b.width, b.height)
    delta = np.abs(pixels(a).astype(int) - pixels(b).astype(int))
    assert delta.max(initial=0) <= 1


KINDS = ("endpoint origin component_origin on_edge on_axis intersection_edge "
         "intersection_face on_line extension from_point aligned tangent midpoint "
         "arc_midpoint on_face close reference through_point perp_face intersection "
         "center axis axis_inference none").split()


def canvas_rgba(draw, size=(360, 100), dpr=1):
    from ingetrazo_render import AnnotationCanvas
    w, h = int(size[0] * dpr), int(size[1] * dpr)
    canvas = AnnotationCanvas(w, h, float(size[0]))
    draw(canvas)
    return canvas.composite(bytes(w * h * 4))


@pytest.mark.parametrize("kind", KINDS)
@pytest.mark.parametrize("dpr", [1, 2])
def test_snap_marker_is_pinned(kind, dpr):
    """The marker moved from Python painting to AnnotationCanvas.draw_snap.
    The digests are of the Python marker on the same canvas with the
    native paint types, before the port (docs/render-glue-port.md); the
    Python marker in its turn matched GL's within one per channel. Labels
    are laid out once and placed by a transform since, which moves a few
    edge pixels; the pin is of the marker and its guides."""
    import hashlib
    import json
    from pathlib import Path
    snap = SnapResult(kind=kind, point=V(48, 40, 0), color=(.16, .62, .36),
                      guide=(V(80, 40, 0), V(325, 40, 0)),
                      guide_color=(.86, .22, .27))
    snap.guides = [(V(80, 65, 0), V(325, 65, 0), (.2, .4, .78))]
    project = lambda p: (p.x(), p.y())
    clip = lambda a, b: (a, b)
    rgba = canvas_rgba(lambda c: draw_snap_indicator(c, snap, project, clip, labels=False), dpr=dpr)
    pins = json.loads((Path(__file__).parent / "data" / "snap_marker_digests.json").read_text())
    assert hashlib.sha256(rgba).hexdigest()[:24] == pins[f"{dpr}|{kind}"]
    assert any(rgba) == (kind not in ("axis", "axis_inference"))


@pytest.mark.parametrize("style", ["window", "crossing", "zoom"])
@pytest.mark.parametrize("dpr", [1, 2])
def test_rectangle_matches_gl(style, dpr):
    a, b = (30, 20), (280, 80)
    if style == "crossing":
        a, b = b, a
    box = selection_rectangle(a, b, zoom=style == "zoom")
    actual = pixels(painted(lambda canvas: paint_layer(canvas, [box]), dpr=dpr))
    color = {"window": "#3366c7", "crossing": "#289e5c", "zoom": "#f37329"}[style]
    assert box.color == color
    assert box.pen_style == (Qt.SolidLine if style == "window" else Qt.DashLine)
    assert actual[40 * dpr, 40 * dpr, 3] == (26 if style == "zoom" else 28)
    assert actual[40 * dpr, 10 * dpr, 3] == 0


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


@pytest.mark.parametrize("size", [(640, 400), (1280, 800), (500, 700), (1000, 1400)])
@pytest.mark.parametrize("only", [True, False])
def test_export_layout_matches_document_at_output_aspect(size, only):
    scene, camera = fixture()
    old_aspect = camera.aspect
    # Compare the document projection against the exported capture at the same logical width.
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
    expected = painted(reference, size, logical_width=640)
    background = bytes(size[0] * size[1] * 4)
    rgba = composite_pixels(background, *size, scene, camera, logical_size=(640, 400),
                            annotations_only=only)
    actual = SurveyRaster(*size, rgba)
    equal(actual, expected)
    assert pixels(actual)[:, :, 3].any()
    assert camera.aspect == old_aspect
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


def test_unclassified_navigation_point_leaves_the_overlay_transparent():
    tools = SimpleNamespace(last_snap=SnapResult(kind=None, point=V()), tool=None)
    image = painted(lambda painter: draw_tool_overlays(painter, tools))
    assert not pixels(image).any()


def test_capture_occlusion_callback_and_painter_state():
    scene, camera = fixture()
    layout = AnnotationLayout(scene, camera, (360, 100), occluded=lambda p: p.x() > 0)
    assert layout._project_samples([V(-1, 0, 0), V(1, 0, 0)])[1][1] is False
    # A caller's stroke and transform must survive an annotation pass.
    def marker(canvas, annotate):
        canvas.setPen(PaintPen(PaintColor(0, 255, 0), 2))
        if annotate:
            draw_annotations(canvas, scene, camera, (360, 100), annotations_only=True)
        canvas.drawLine(QPointF(1, 2), QPointF(12, 2))
    reference = pixels(painted(lambda canvas: marker(canvas, False)))
    actual = pixels(painted(lambda canvas: marker(canvas, True)))
    assert reference[:5, :14, 1].max() == 255
    np.testing.assert_array_equal(reference[:5, :14], actual[:5, :14])


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
    from core.input_types import QPointF
    target = AnnotationCanvas(96, 36, 96)
    base = bytes([0, 0, 255, 128]) * (96 * 36)
    assert target.composite(base) is base
    target.fillRect(0, 0, 8, 8, PaintColor(255, 0, 0, 128))
    target.setPen(PaintPen(PaintColor(0, 255, 0)))
    target.drawText(QPointF(10, 24), "A < B & C")
    pixels = np.frombuffer(target.composite(base), np.uint8).reshape(36, 96, 4)
    np.testing.assert_allclose(pixels[2, 2], [170, 0, 85, 192], atol=1)
    assert pixels[:, 10:, 1].max() > 160
    assert pixels[35, 95].tolist() == list(base[:4])


def test_capture_rejects_impossible_pixel_dimensions_without_allocating():
    from ingetrazo_render import AnnotationCanvas

    with pytest.raises(ValueError):
        AnnotationCanvas(1 << 31, 1 << 31, 800).composite(b"")

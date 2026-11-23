"""Painter-only Metal annotations and the P1 integration contract.

Live: call draw_annotations(painter, scene, camera, (width, height),
occluded=tools._is_occluded) from rasterize's annotations callback. Schedule
this for guides/section widgets as well as dimensions/text, even in an otherwise
empty document. Then call draw_tool_overlays(painter, tools), independently of
document annotation presence. Draw snap feedback with wgpu_overlay.
draw_snap_indicator(painter, snap, layout._world_to_pixel,
layout._clip_pixel_line), where layout is the AnnotationLayout returned by
draw_annotations. Append selection_rectangle(start, end, zoom=False) after
the input drag threshold; retain unsorted start/end coordinates.

Export: after native capture, call composite_annotations(image, scene, camera,
logical_size=(live_width, live_height), annotations_only=False). It returns a
new image. Set overlays=False to leave it untouched. Camera is copied and its
aspect is set to the OUTPUT aspect, so sheet frames do not project through the
live widget. Uniform output_width/logical_width scaling keeps fonts/arrows
undistorted on differently shaped sheets; logical height follows output aspect.
Pass an occluded(world_point)->bool callback ONLY if valid for that capture
camera (never a stale live pick buffer). Default is visible, explicitly.

Document guide dashes reuse GL's world-space spacing/reach and colors; guide
points use GL's [14,10] pen. The overlay has NO depth attachment, so the live
view sends guide lines to the native depth-tested line pass instead
(``views/wgpu_lines.py``, ``guide_lines=False``); exports still paint them
here. Dimensions/leaders support the optional CPU occlusion callback.
Live tools keep their own draw_overlay hook (Walk crosshair, Flip planes —
the reference Flip tool draws planes, not arrows). Export excludes these and
lock/snap/selection feedback. annotations_only excludes guides/section/geodata.

Only painter-safe GL methods are bound below, never a QWidget or GL context.
This shares annotation layout/localization rather than duplicating those rules.
"""
from copy import copy

import numpy as np
from PySide6.QtCore import QPointF, Qt
from PySide6.QtGui import QColor, QPainter, QPen
from ingetrazo_core import Vec3 as QVector3D

from views.viewport import Viewport, _guide_vertices, shown_guides


class AnnotationLayout:
    """Isolated scene/camera projection for a logical annotation canvas."""

    # Explicit allowlist: no accidental delegation into live widget geometry.
    _clip_pixel_line = Viewport._clip_pixel_line
    _clip_segment_front = Viewport._clip_segment_front
    _draw_dimensions = Viewport._draw_dimensions
    _draw_text_labels = Viewport._draw_text_labels
    _draw_guides = Viewport._draw_guides
    _draw_section_planes = Viewport._draw_section_planes
    section_plane_frame = Viewport.section_plane_frame
    _draw_geo_surfaces = Viewport._draw_geo_surfaces
    _draw_geo_paths = Viewport._draw_geo_paths
    _draw_geo_points = Viewport._draw_geo_points
    _draw_geo_path_label = Viewport._draw_geo_path_label
    ground_height = Viewport.ground_height
    drape = Viewport.drape
    _text_block_x = staticmethod(Viewport._text_block_x)
    _format_dim_value = staticmethod(Viewport._format_dim_value)
    # Live tool feedback (draw_tool_feedback): GL's own painters.
    _segment_to_pixels = Viewport._segment_to_pixels
    _clip_w = Viewport._clip_w
    _CLIP_W = Viewport._CLIP_W
    _SNAP_LABELS = Viewport._SNAP_LABELS
    _rubber_band_style = Viewport._rubber_band_style
    _draw_rubber_band_overlay = Viewport._draw_rubber_band_overlay
    _draw_guide_preview = Viewport._draw_guide_preview
    _draw_inference_marker = Viewport._draw_inference_marker
    _draw_scale_box = Viewport._draw_scale_box
    _draw_length_label = Viewport._draw_length_label
    _draw_route_marker = Viewport._draw_route_marker
    _OCCLUSION_STEP_PX = Viewport._OCCLUSION_STEP_PX
    _OCCLUSION_MAX_SAMPLES = Viewport._OCCLUSION_MAX_SAMPLES
    _OCCLUSION_SPLIT = Viewport._OCCLUSION_SPLIT
    _OCCLUSION_ROUNDS = Viewport._OCCLUSION_ROUNDS

    def __init__(self, scene, camera, logical_size, *, occluded=None, hover=None, clear_segment=None):
        self.scene = scene
        self.camera = copy(camera)
        self._width, self._height = map(float, logical_size)
        if self._width <= 0 or self._height <= 0:
            raise ValueError("Annotation canvas dimensions must be positive")
        self.camera.aspect = self._width / self._height
        self._mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        self._clear_points = set()
        self._clear_segment = clear_segment
        self._occluded = occluded
        self._hover_entity = hover

    def width(self):
        return self._width

    def height(self):
        return self._height

    def _world_to_pixel(self, point, mvp=None):
        return Viewport._world_to_pixel(self, point, self._mvp if mvp is None else mvp)

    def _occluded_on_screen(self, point):
        if not self._occluded or point.toTuple() in self._clear_points:
            return False
        return bool(self._occluded(point))

    def _draw_occluded_segment(self, painter, a, b):
        # Keep the same projected strokes and pen joins when the resident
        # boxes prove this entire line is in front of any possible occluder.
        occluded = self._occluded
        if self._clear_segment and self._clear_segment(a, b):
            # The dimension's arrowheads and label query these same points
            # immediately after the stroke. Its clear segment covers all three.
            self._clear_points.update((a.toTuple(), b.toTuple(), ((a + b) * .5).toTuple()))
            self._occluded = None
        try:
            Viewport._draw_occluded_segment(self, painter, a, b)
        finally:
            self._occluded = occluded

    def _project_samples(self, points):
        return [(self._world_to_pixel(p), not self._occluded_on_screen(p))
                for p in points]


def guide_segments(scene, camera):
    """Return GL world dash vertices (N,2,3) and selected flags (N,).

    P1/P2 may upload these to a depth-tested line pass instead of painting them.
    Includes finite guide-point origin segments and layer visibility filtering.
    """
    distance = max(float(camera.distance), 1e-3)
    coords, spans = _guide_vertices(
        shown_guides(scene), distance * .03, scene.selection,
        centre=camera.target, reach=distance * 15,
        eye=camera.eye() if camera.perspective else None)
    segments = np.asarray(coords, dtype=np.float32).reshape(-1, 2, 3)
    selected = np.zeros(len(segments), dtype=bool)
    for start, count, flag in spans:
        selected[start // 2:(start + count) // 2] = flag
    return segments, selected


def draw_guide_lines(painter, layout):
    """Project GL world dashes; no depth-buffer occlusion is possible here."""
    segments, selected = guide_segments(layout.scene, layout.camera)
    pens = [QPen(QColor.fromRgbF(*rgb), 1.0) for rgb in
            ((.27, .35, .47), (.95, .45, .16))]
    for pen in pens:
        pen.setCapStyle(Qt.FlatCap)
    painter.save()
    try:
        for (a, b), flag in zip(segments, selected):
            clipped = layout._clip_segment_front(QVector3D(*a), QVector3D(*b))
            if clipped is None:
                continue
            pa, pb = (layout._world_to_pixel(p) for p in clipped)
            if pa is None or pb is None:
                continue
            visible = layout._clip_pixel_line(pa, pb)
            if visible is not None:
                painter.setPen(pens[int(flag)])
                painter.drawLine(QPointF(*visible[0]), QPointF(*visible[1]))
    finally:
        painter.restore()


def draw_annotations(painter, scene, camera, logical_size, *, annotations_only=False,
                     occluded=None, hover=None, guide_lines=True, clear_segment=None):
    """Paint document annotations; return the isolated AnnotationLayout for snaps.

    ``guide_lines=False`` leaves the guide lines to the native depth-tested
    line pass (``views/wgpu_lines.py``); their points stay painted here."""
    layout = AnnotationLayout(scene, camera, logical_size, occluded=occluded, hover=hover,
                              clear_segment=clear_segment)
    painter.save()
    try:
        painter.setRenderHint(QPainter.Antialiasing)
        painter.setRenderHint(QPainter.TextAntialiasing)
        if not annotations_only:
            if guide_lines:
                draw_guide_lines(painter, layout)
            layout._draw_guides(painter)
            layout._draw_section_planes(painter)
            layout._draw_geo_surfaces(painter)
            layout._draw_geo_paths(painter)
            layout._draw_geo_points(painter)
        layout._draw_dimensions(painter)
        layout._draw_text_labels(painter)
    finally:
        painter.restore()
    return layout


def rubber_band(tools):
    """GL's rubber band for the active tool: ``(segments, rgba, width,
    depth_tested)`` or None. A depth-tested band (Push/Pull's forming box,
    Offset, Paste) goes to the native preview's edges so faces hide its
    back side; the rest floats in the overlay, as in GL."""
    style = Viewport._rubber_band_style(tools)
    if style is None:
        return None
    tool = getattr(tools, "active_tool", None)
    return (*style, bool(getattr(tool, "wireframe_depth_tested", False)))


def draw_tool_feedback(painter, tools, layout):
    """Paint the live tool cues GL's ``_draw_overlay`` paints, with GL's
    painters: the floating rubber band in its axis/lock colour, the dashed
    guide preview of Tape and Protractor, Scale's grip box, Push/Pull's
    «level with» marker, the route marker, extension overlays and the
    bold length label (warm while a value is typed).

    The Metal view had its own stand-ins (one cyan pen for every line,
    plain text, flat grips) that dropped the colours telling you which
    axis a line follows. ``layout`` is the frame's AnnotationLayout (live
    camera at the overlay's logical size); ``tools`` is NativeTools."""
    tool = getattr(tools, "active_tool", None)
    if tool is None:
        return
    layout.active_tool = tool
    layout.last_snap = getattr(tools, "last_snap", None)
    layout._value_buffer = getattr(tools, "value_buffer", "") or ""
    layout._route_marker = getattr(tools, "_route_marker", None)
    painter.save()
    try:
        band = rubber_band(tools) if callable(getattr(tool, "rubber_band_lines", None)) else None
        layout._overlay_rubber = band[:3] if band is not None and not band[3] else None
        layout._draw_rubber_band_overlay(painter)
        if hasattr(tool, "guide_preview_lines"):
            layout._draw_guide_preview(painter)
        layout._draw_inference_marker(painter)
        layout._draw_scale_box(painter)
        layout._draw_route_marker(painter)
        draw = getattr(Viewport, "_draw_extension_overlays", None)
        if draw is not None and getattr(tools, "_ext_overlays", None):
            draw(tools, painter)
        if (callable(getattr(tool, "value_label", None))
                or callable(getattr(tool, "rubber_band_lines", None))):
            layout._draw_length_label(painter)
    finally:
        painter.restore()


def composite_annotations(image, scene, camera, *, logical_size,
                          overlays=True, annotations_only=False, occluded=None):
    """Return a capture copy with annotations placed for its scale and aspect.

    image dimensions are physical pixels; logical_size is the live logical size.
    DPR metadata is respected exactly once, including a 2x QImage.
    """
    result = image.copy()
    if not overlays:
        return result
    if result.isNull():
        raise ValueError("Cannot annotate a null image")
    width, height = map(float, logical_size)
    if width <= 0 or height <= 0:
        raise ValueError("Annotation canvas dimensions must be positive")
    scale = result.width() / width
    canvas = (width, result.height() / scale)
    painter = QPainter(result)
    try:
        factor = scale / result.devicePixelRatio()
        painter.scale(factor, factor)
        draw_annotations(painter, scene, camera, canvas,
                         annotations_only=annotations_only, occluded=occluded)
    finally:
        painter.end()
    return result


def draw_tool_overlays(painter, tools, *, labels=True):
    """Call active tool's GL painter hook, then prioritized lock/reference labels.

    P1 passes NativeTools (or a GL-compatible facade), every live frame regardless
    of document annotations. No export call. Returns without altering painter state.
    """
    tool = getattr(tools, "tool", getattr(tools, "active_tool", None))
    painter.save()
    try:
        painter.setBrush(Qt.NoBrush)
        hook = getattr(tool, "draw_overlay", None)
        if callable(hook):
            hook(tools, painter)
        if not labels:
            return
        if getattr(tools, "reference_mode", None) is not None:
            Viewport._draw_reference_label(tools, painter)
        elif getattr(tools, "axis_lock", None) is not None:
            Viewport._draw_axis_lock_label(tools, painter)
        elif getattr(tool, "plane_lock", None) is not None:
            # NativeTools may still expose source.active_tool; use the actual tool.
            from types import SimpleNamespace
            Viewport._draw_plane_lock_label(SimpleNamespace(active_tool=tool), painter)
        else:
            Viewport._draw_inference_label(tools, painter)
        if getattr(tools, "linear_inference_mode", "all") != "all":
            Viewport._draw_linear_mode_label(tools, painter)
    finally:
        painter.restore()

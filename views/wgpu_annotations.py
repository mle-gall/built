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
from core.input_types import QPointF, Qt
from views.paint_types import QColor, QPen
from ingetrazo_core import Vec3 as QVector3D

from views.document_interaction import DocumentInteraction, _guide_vertices, shown_guides


class AnnotationLayout:
    """Isolated scene/camera projection for a logical annotation canvas."""

    # Explicit allowlist: no accidental delegation into live widget geometry.
    _clip_pixel_line = DocumentInteraction._clip_pixel_line
    _clip_segment_front = DocumentInteraction._clip_segment_front
    _draw_dimensions = DocumentInteraction._draw_dimensions
    _draw_text_labels = DocumentInteraction._draw_text_labels
    _draw_guides = DocumentInteraction._draw_guides
    _draw_section_planes = DocumentInteraction._draw_section_planes
    section_plane_frame = DocumentInteraction.section_plane_frame
    _draw_geo_surfaces = DocumentInteraction._draw_geo_surfaces
    _draw_geo_paths = DocumentInteraction._draw_geo_paths
    _draw_geo_points = DocumentInteraction._draw_geo_points
    _draw_geo_path_label = DocumentInteraction._draw_geo_path_label
    ground_height = DocumentInteraction.ground_height
    drape = DocumentInteraction.drape
    _text_block_x = staticmethod(DocumentInteraction._text_block_x)
    _format_dim_value = staticmethod(DocumentInteraction._format_dim_value)
    # Live tool feedback (draw_tool_feedback): GL's own painters.
    _segment_to_pixels = DocumentInteraction._segment_to_pixels
    _clip_w = DocumentInteraction._clip_w
    _CLIP_W = DocumentInteraction._CLIP_W
    _SNAP_LABELS = DocumentInteraction._SNAP_LABELS
    _rubber_band_style = DocumentInteraction._rubber_band_style
    _draw_rubber_band_overlay = DocumentInteraction._draw_rubber_band_overlay
    _draw_guide_preview = DocumentInteraction._draw_guide_preview
    _draw_inference_marker = DocumentInteraction._draw_inference_marker
    _draw_scale_box = DocumentInteraction._draw_scale_box
    _draw_length_label = DocumentInteraction._draw_length_label
    _length_label_at = DocumentInteraction._length_label_at
    _draw_route_marker = DocumentInteraction._draw_route_marker
    _OCCLUSION_STEP_PX = DocumentInteraction._OCCLUSION_STEP_PX
    _OCCLUSION_MAX_SAMPLES = DocumentInteraction._OCCLUSION_MAX_SAMPLES
    _OCCLUSION_SPLIT = DocumentInteraction._OCCLUSION_SPLIT
    _OCCLUSION_ROUNDS = DocumentInteraction._OCCLUSION_ROUNDS

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
        return DocumentInteraction._world_to_pixel(self, point, self._mvp if mvp is None else mvp)

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
            DocumentInteraction._draw_occluded_segment(self, painter, a, b)
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
        if not annotations_only:
            if guide_lines:
                draw_guide_lines(painter, layout)
            layout._draw_guides(painter)
            layout._draw_section_planes(painter)
            layout._draw_geo_surfaces(painter)
            layout._draw_geo_paths(painter)
            layout._draw_geo_points(painter)
            DocumentInteraction._draw_image_outlines(layout, painter)
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
    style = DocumentInteraction._rubber_band_style(tools)
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
    view = getattr(tools, "view", None)
    native_wire = getattr(getattr(view, "renderer", None), "set_tool_wire", None)
    if tool is None:
        if native_wire is not None:
            native_wire(None, (), 1.)
        return
    layout.active_tool = tool
    layout.last_snap = getattr(tools, "last_snap", None)
    layout._value_buffer = getattr(tools, "value_buffer", "") or ""
    # With the chrome on, the value field on the geometry is the chrome's
    # (ui/value_field.rs): the bold label is not painted under it.
    layout._chrome_owns_labels = bool(getattr(getattr(tools, "view", None), "_ui_attached", False))
    layout._route_marker = getattr(tools, "_route_marker", None)
    painter.save()
    try:
        floating = (not getattr(tool, "wireframe_depth_tested", False)
                    and callable(getattr(tool, "rubber_band_lines", None)))
        band = rubber_band(tools) if floating else None
        if native_wire is not None:
            visible = getattr(getattr(view, "source_viewport", None), "tool_preview", True)
            guides = getattr(tool, "guide_preview_lines", lambda: ())() if visible else ()
            native_wire(band if visible else None, guides, painter.device().devicePixelRatioF())
        else:
            layout._overlay_rubber = band[:3] if band is not None else None
            layout._draw_rubber_band_overlay(painter)
            if hasattr(tool, "guide_preview_lines"):
                layout._draw_guide_preview(painter)
        layout._draw_inference_marker(painter)
        layout._draw_scale_box(painter)
        layout._draw_route_marker(painter)
        draw = getattr(DocumentInteraction, "_draw_extension_overlays", None)
        if draw is not None and getattr(tools, "_ext_overlays", None):
            plugin_paint(painter, lambda p: draw(tools, p))
        if (callable(getattr(tool, "value_label", None))
                or callable(getattr(tool, "rubber_band_lines", None))):
            layout._draw_length_label(painter)
    finally:
        painter.restore()


def composite_pixels(rgba, width, height, scene, camera, *, logical_size,
                     annotations_only=False, occluded=None):
    """Project shared document marks and composite them into a capture in Rust."""
    from ingetrazo_render import AnnotationCanvas
    if min(logical_size) <= 0:
        raise ValueError("Annotation canvas dimensions must be positive")
    target = AnnotationCanvas(width, height, logical_size[0])
    draw_annotations(target, scene, camera, target.logical_size,
                     annotations_only=annotations_only, occluded=occluded)
    return target.composite(rgba)


def composite_annotations(image, scene, camera, *, logical_size,
                          overlays=True, annotations_only=False, occluded=None):
    """``composite_pixels`` for a caller holding an ``RgbaImage``: a new one
    with the marks; its device pixel ratio is kept."""
    from ingetrazo_render import RgbaImage
    if not overlays:
        return image.copy()
    if image.isNull():
        raise ValueError("Cannot annotate a null image")
    w, h = image.width(), image.height()
    rgba = composite_pixels(image.rgba(), w, h, scene, camera, logical_size=logical_size,
                            annotations_only=annotations_only, occluded=occluded)
    result = RgbaImage(w, h, rgba)
    result.setDevicePixelRatio(image.devicePixelRatio())
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
            if getattr(hook, "__module__", "").startswith("tools."):
                hook(tools, painter)
            else:
                plugin_paint(painter, lambda p: hook(tools, p))
        if not labels:
            return
        if getattr(tools, "reference_mode", None) is not None:
            DocumentInteraction._draw_reference_label(tools, painter)
        elif getattr(tools, "axis_lock", None) is not None:
            DocumentInteraction._draw_axis_lock_label(tools, painter)
        elif getattr(tool, "plane_lock", None) is not None:
            # NativeTools may still expose source.active_tool; use the actual tool.
            from types import SimpleNamespace
            DocumentInteraction._draw_plane_lock_label(SimpleNamespace(active_tool=tool), painter)
        else:
            DocumentInteraction._draw_inference_label(tools, painter)
        if getattr(tools, "linear_inference_mode", "all") != "all":
            DocumentInteraction._draw_linear_mode_label(tools, painter)
    finally:
        painter.restore()


def plugin_paint(target, draw):
    """An extension's overlay hook paints on the annotation canvas itself,
    which answers the painter calls it makes (pens, brushes, lines,
    polygons, rectangles, ellipses, text). There is no Qt painter."""
    return draw(target)

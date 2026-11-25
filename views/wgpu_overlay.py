"""Logical-pixel tool primitives and the snap marker on the annotation
canvas (``ingetrazo_render.AnnotationCanvas``), which the renderer
composites over the scene."""
from dataclasses import dataclass
import math

import numpy as np
from core.input_codes import Input as Qt


@dataclass(frozen=True)
class Primitive:
    kind: str
    points: tuple
    color: str = "#00dfff"
    width: float = 1.5
    text: str = ""
    fill: str | None = None
    dash: tuple = ()
    pen_style: Qt.PenStyle = Qt.SolidLine
    snap_kind: str = ""


def rasterize(primitives, width, height, scale, annotations=None):
    """The primitives on a fresh annotation canvas, as a straight-alpha
    ``SurveyRaster`` at physical resolution (scripts and tests read its pixels)."""
    from ingetrazo_render import AnnotationCanvas, SurveyRaster
    w, h = max(1, int(width * scale)), max(1, int(height * scale))
    canvas = AnnotationCanvas(w, h, float(width))
    paint_layer(canvas, primitives, annotations)
    return SurveyRaster(w, h, canvas.composite(bytes(w * h * 4)))


def paint_layer(painter, primitives, annotations=None):
    """The document's annotations (``annotations(painter)``), then the tool
    primitives, on the annotation canvas (``AnnotationCanvas.draw_primitives``)."""
    if annotations is not None:
        annotations(painter)
    if primitives:
        painter.draw_primitives([
            (p.kind, [(float(x), float(y)) for x, y in p.points], _colour(p.color),
             float(p.width), str(p.text), None if not p.fill else _colour(p.fill),
             [float(v) for v in p.dash],
             int(getattr(p.pen_style, "value", p.pen_style)), p.snap_kind)
            for p in primitives])


def _colour(c):
    """A colour string as Qt names one, from a string or a QColor."""
    if isinstance(c, str):
        return c
    r, g, b, a = c.getRgb()
    return f"#{a:02x}{r:02x}{g:02x}{b:02x}"


def pixels(image):
    """The image's straight RGBA bytes as a flat array."""
    return np.frombuffer(image.rgba(), dtype=np.uint8)


def snap_glyph(point, kind, color="#00dfff"):
    """Return one haloed marker; color accepts a Qt color string or QColor.

    P1 should prefer draw_snap_indicator for the full SnapResult, including
    guide colors, contextual/translated labels, and axis marker suppression.
    The legacy vertex alias still means endpoint.
    """
    return Primitive("snap", (point,), color, snap_kind=(
        "endpoint" if kind == "vertex" else kind))


def selection_rectangle(start, end, *, zoom=False):
    """Return GL's direction-sensitive box in logical pixels (no drag threshold).

    P1 calls this after its drag threshold, with unsorted start/end tuples.
    Set zoom=True for Zoom Window; append the returned Primitive.
    """
    crossing = end[0] < start[0]
    color = "#f37329" if zoom else ("#289e5c" if crossing else "#3366c7")
    alpha = "1a" if zoom else "1c"
    return Primitive("rectangle", (start, end), color, 1.5,
                     fill="#" + alpha + color[1:],
                     pen_style=Qt.DashLine if zoom or crossing else Qt.SolidLine)


def axes(origin, directions, length=32):
    """Projected camera axes; callers supply three normalized screen vectors."""
    result = []
    for label, color, direction in zip("XYZ", ("#ff5555", "#55dd55", "#5599ff"), directions):
        x, y = origin
        dx, dy = direction
        norm = max(math.hypot(dx, dy), 1e-12)
        end = (x + length*dx/norm, y + length*dy/norm)
        result.extend((Primitive("polyline", (origin, end), color),
                       Primitive("text", (end,), color, text=label)))
    return result


def draw_snap_indicator(painter, snap, project, clip_line, *, labels=True,
                        scene=None):
    """Paint a SnapResult on the annotation canvas: its guides, glyph and
    label (``AnnotationCanvas.draw_snap``, annotation_canvas.rs).

    ``project`` maps world to logical pixels and ``clip_line`` clips a
    screen segment, (a, b) -> (a, b) or None. Colours come from the
    SnapResult, guides included. Axis snaps draw nothing."""
    if snap.kind in ("axis_inference", "axis"):
        return
    pixel = project(snap.point)
    if pixel is None:
        return
    r, g, b = snap.color
    guides = []
    if snap.guide is not None:
        # The 'from point' guide is axis-coloured while the point is green.
        gc = snap.guide_color if snap.guide_color is not None else (r, g, b)
        guides.append((*snap.guide, gc))
    guides.extend(snap.guides or ())
    segments = []
    for ga, gb, gc in guides:
        gp0, gp1 = project(ga), project(gb)
        if gp0 is not None and gp1 is not None:
            vis = clip_line(gp0, gp1)
            if vis is not None:
                segments.append((tuple(vis[0]), tuple(vis[1]), tuple(gc[:3])))
    painter.draw_snap(snap.kind, tuple(pixel), (r, g, b), segments,
                      _snap_label(snap, scene) if labels else None)


def _snap_label(snap, scene):
    """The words beside the marker ("On Edge", "in group"), translated."""
    from views.document_interaction import DocumentInteraction
    from core.i18n import tr
    label = DocumentInteraction._SNAP_LABELS.get(snap.kind)
    if snap.kind == "aligned" and snap.axis == "z":
        label = "Level with point"
    if label:
        label = tr(label)
    if getattr(snap, "label", None):
        label = snap.label               # an extension's own words
    if not label:
        return None
    ctx_ = getattr(snap, "context", None)
    if ctx_ == "component":
        label += " " + tr("in component")
    elif ctx_ == "group":
        label += " " + tr("in group")
    elif ctx_ == "outside" and scene is not None:
        # As DocumentInteraction._snap_context_suffix (#1327): geometry
        # outside the group or component open for editing.
        is_comp = (scene.edit_definition() is not None
                   if scene.edit_group is not None else None)
        label += " " + tr("outside the component" if is_comp
                          else "outside the group")
    return label

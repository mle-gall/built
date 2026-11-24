# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Document editing, picking and painter annotations shared by the presentation hosts."""
from __future__ import annotations
from core.layers import effective_visible as _effective_layer_visible
from core.mesh import element_identity
from core.element_set import ElementSet

import copy
import importlib
import itertools
import math
import os
import sys
import time as _time_mod
import types
from array import array
from pathlib import Path
from typing import Optional
from core.units import fmt_len as _fmt_len
from core.units import typed_value_text as _typed_value_text

# Perf telemetry (INGETRAZO_PERF=1): every operation slower than 50 ms and a
# once-per-second frame summary land in ~/ingetrazo-perf.log — the tool for
# "it feels slow" reports from real sessions, where synthetic benchmarks lie.
_PERF = bool(os.environ.get("INGETRAZO_PERF"))
# Kill-switch for the instanced component pass (P2): set to 1 to draw
# every instance through the consolidated VBOs like before.
_NO_INSTANCING = os.environ.get("INGETRAZO_NO_INSTANCING", "") == "1"
# Kill-switch for the default-back tint pass (faces have sides, 2026-09-11):
# set to 1 to draw every paint on both sides like before — a diagnostic,
# and the escape hatch should a driver misbehave with front culling.
_NO_BACK_TINT = os.environ.get("INGETRAZO_NO_BACK_TINT", "") == "1"
# Kill-switch for the snap engine's shortcuts (edge grid, occlusion in
# batches, centre lookup through the registry): set to 1 to run every hover
# the way it ran before — one pass over all group edges, one ray per
# candidate, the rounded-position table. A diagnostic, and the escape hatch
# should a snap ever land somewhere it did not before.
_NO_SNAP_FAST = os.environ.get("INGETRAZO_NO_SNAP_FAST", "") == "1"
# Kill-switch for the pick index's span grid: set to 1 to keep one span per
# chunk however big, as before (a ray that meets the box of a 107 k-triangle
# group then tests all of its triangles).
_NO_SPAN_GRID = os.environ.get("INGETRAZO_NO_SPAN_GRID", "") == "1"
#: A chunk with more triangles than this is split into grid cells for the
#: ray prefilter (_split_chunk_spans); ~4 096 triangles a cell.
_SPAN_GRID_MIN = 8192
_SPAN_CELL_TRIS = 4096
_perf_file = None


def _plog(tag: str, ms: float, extra: str = "", floor: float = 50.0) -> None:
    """Frame telemetry (P0). Lines carry the writer's pid and honour
    ``$INGETRAZO_PERF_LOG`` — see ``core.history._plog`` for why."""
    global _perf_file
    if not _PERF or ms < floor:
        return
    if _perf_file is None:
        path = os.environ.get("INGETRAZO_PERF_LOG")
        _perf_file = open(Path(path) if path
                          else Path.home() / "ingetrazo-perf.log",
                          "a", buffering=1)
    _perf_file.write(f"{_time_mod.strftime('%H:%M:%S')} [{os.getpid()}] "
                     f"{tag} {ms:.0f}ms"
                     f"{' ' + extra if extra else ''}\n")

from core.input_types import NATIVE, QEvent, Qt, QPointF, QRectF
from core.host_callbacks import Signal
from core.timers import later, timer as window_timer
if not NATIVE:
    from PySide6.QtCore import QObject, QTimer
from views.paint_types import QColor, QFont, QPen, QPolygonF
if not NATIVE:
    from PySide6.QtGui import QFontMetrics, QImage, QPainter
import numpy as np
import ingetrazo_core
import ingetrazo_render
from ingetrazo_render import document_value_key as _value_key
from ingetrazo_core import ray_aabb_span as _ray_aabb_span
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D, Vec4 as QVector4D

from core.camera import OrbitCamera
from core.i18n import tr
from core.group import (Group, PlacementProxy, copy_group, effective_material,
                        world_mesh)
from core import layers as _layers
from core.mesh import Edge, Face, smooths
from core.history import EraseSelectionCommand, History
from core.scene import Scene
from core.style import (DEFAULT_BACK_COLOR, DEFAULT_FACE_COLOR,
                        MONOCHROME_GLASS_RGB,
                        effective_back_color)
from core.materials import effective_attrs, material_sig as _material_sig
from core.snap import SnapResult, _AXIS_VECTORS
from core.texture import face_uv_axes
from core.layers import is_link_untagged
from core.triangulate import plane_axes
from tools.base import Tool, ToolContext
from core.inference import InferenceEngine


class _HoverEvent:
    """Minimal stand-in for a QMouseEvent inside the coalesced hover path —
    just what ``_build_ctx`` and the hover handlers read."""

    __slots__ = ("_pos", "_mods")

    def __init__(self, pos, mods) -> None:
        self._pos = pos
        self._mods = mods

    def position(self):
        return self._pos

    def modifiers(self):
        return self._mods


_active_cut = ingetrazo_core.active_cut


_SnapEdge = ingetrazo_render.SnapEdge


_copy_snap_edge = ingetrazo_render.copy_snap_edge


def _snap_context_of(vp, place):
    return ingetrazo_core.snap_context_of(vp, place)


_group_pseudo_edge = ingetrazo_core.group_pseudo_edge


#: The depth slack of a line the Select tool may take: a fraction of the
#: eye's distance to it, and never less than a millimetre. A line drawn on
#: a face, or a few millimetres under or over it, z-fights that face: the
#: renderers draw it at one angle and lose it at the next (Metal, 2 mm
#: under a face 9 m away: drawn from 45°, gone from above), and the strict
#: occlusion test made it unclickable — the object behind took the click.
LINEWORK_SLACK = 5e-4
LINEWORK_SLACK_MIN = 1e-3


def linework_slack_point(camera, world):
    return ingetrazo_render.linework_slack_point(camera, world)


def _boxes_around(keys, x0, y0, x1, y1, px, py, owner_of, scene):
    return ingetrazo_core.boxes_around(keys, x0, y0, x1, y1, px, py, owner_of, scene)




# ---- Geometry helpers ------------------------------------------------------

#: Beyond this many loose edges the snap engine gets a near-cursor subset
#: instead of the whole mesh: compute_snap walks its edge list in Python
#: ~a dozen times per hover, and an exploded medium import (9k faces =
#: ~20k edges) froze every mouse move (user report, piscina.igz).
_LOOSE_SNAP_CAP = 3000


def _inst_proto_of(mod, base):
    return ingetrazo_render.inst_proto_of(mod, base)


_PickWarm = ingetrazo_render.PickWarm
_pick_warm_take = ingetrazo_render.pick_warm_take


def _entity_owner(entities, i):
    """The owner half of pick-index entity ``i``: a lazy piece of the index
    answers it without building its faces (see views.array_bake)."""
    at = getattr(entities, "owner_at", None)
    return at(i) if at is not None else entities[i][1]


def _split_chunk_spans(chunk, depth=0, force=False):
    return ingetrazo_render.split_chunk_spans(chunk, depth, force)


#: Every chunk entry built (or loaded from disk) takes the next number.
#: Keys that told entries apart by ``id(entry)`` could not: the dict a
#: rebuild replaces is freed at once, and the new one may land at the same
#: address — same id, same rev 0 — so the instance arrays and the GL draw
#: entry derived from the OLD chunk were served as if nothing had changed
#: (a face painted inside a component came back unpainted one level up,
#: Marco, 2026-09-11).
_chunk_uid = itertools.count(1)


class _LazyChunk(dict):
    """A chunk whose draw-only fields are computed on first use: a component
    placement baked for the pick index (#158's lazy placements, hundreds per
    pointer move) needs its triangles, edges and box, never its colours,
    texture runs and re-shading — the dearer two thirds of the bake. Each
    field comes from the same function on the same inputs whenever it is
    asked for, so what is drawn is what the eager bake gave."""
    __slots__ = ("_later",)

    def __init__(self, data, later):
        super().__init__(data)
        self._later = later              # field -> callable making it

    def __missing__(self, key):
        return ingetrazo_render.lazy_chunk_value(self, key)

    def __setitem__(self, key, value):
        self._later.pop(key, None)
        dict.__setitem__(self, key, value)

    def __contains__(self, key):
        return dict.__contains__(self, key) or key in self._later

    def get(self, key, default=None):
        return self[key] if key in self else default

    def _made(self):
        return ingetrazo_render.lazy_chunk_fill(self)

    def keys(self):
        return dict.keys(self._made())

    def items(self):
        return dict.items(self._made())

    def values(self):
        return dict.values(self._made())

    def __iter__(self):
        return dict.__iter__(self._made())

    def __len__(self):
        return dict.__len__(self._made())

    def copy(self):
        return dict(self.items())


#: The snap engine's occlusion rays go through core._native's kernel when
#: the experimental engine is on (tests compare both).
_OCCL_NATIVE = True


_occlusion_native = ingetrazo_render.occlusion_native


#: Lazy placements are filtered by core._native's spatial index before
#: being baked for a pick (the experimental engine; tests compare both).
_INSTANCE_FILTER = True
#: The engine answers the corner and occlusion queries for the lazy
#: placements without baking them (tests compare both ways).
_LAZY_NATIVE = True
#: A big document's pick index starts behind it as soon as it opens (see
#: DocumentInteraction.start_pick_warmup; the switch is for measuring and for tests).
_PICK_WARM = os.environ.get("INGETRAZO_NO_PICK_WARM", "") != "1"
#: A pointer move over a still view reuses the model as the last frame
#: drew it (Viewport._frame_cache_key); INGETRAZO_FRAME_CACHE=0 turns it
#: off, and every frame draws everything as before.
_FRAME_CACHE = os.environ.get("INGETRAZO_FRAME_CACHE", "1") != "0"
#: Occlusion answers kept a view (DocumentInteraction._occluded_many);
#: INGETRAZO_OCCLUSION_MEMO=0 asks every point again.
_OCCLUSION_MEMO = os.environ.get("INGETRAZO_OCCLUSION_MEMO",
                                 "1") != "0"
#: The first corner in sight asked of the native engine in one call
#: (DocumentInteraction._first_clear); INGETRAZO_FIRST_CLEAR=0 asks in batches.
_FIRST_CLEAR = os.environ.get("INGETRAZO_FIRST_CLEAR", "1") != "0"
#: Span lists this long are culled with NumPy (_visible_spans_np).
_SPAN_VECTOR_MIN = 64
#: The CPU side of a document's bakes — the render/pick chunks of its
#: groups, the instances' derived chunks, the fingerprints and the
#: prototypes' stand-ins — one store per Scene, shared by every viewport
#: that shows it. Four views of the plaza (1 035 objects) baked the model
#: four times over, 16.7 s on the first split; the arrays do not depend on
#: the view, only their upload to each view's own GL context does.
import weakref
#: ``id(scene)`` → ``(weakref to the scene, its store)``. By id, not by the
#: scene itself: a Scene compares by value and has no hash. The weak
#: reference tells a dead scene's id from a new scene born at the same
#: address, and a finalizer drops the store when the scene goes.
_BAKE_STORES: dict = {}
_SHARED_BAKES = ("_group_chunks", "_inst_chunks", "_fp_memo",
                 "_proto_wrappers")


_bake_store_of = ingetrazo_render.bake_store_of


def _drop_bake_store(key, ref) -> None:
    return ingetrazo_render.drop_bake_store(key, ref)


def _bind_bake_store(vp) -> None:
    return ingetrazo_render.bind_bake_store(vp)


#: Every viewport made, weakly: the decoded textures are kept only while
#: more than one of them is on screen (:func:`_several_views_shown`).
_LIVE_VIEWS: "weakref.WeakSet" = weakref.WeakSet()


_several_views_shown = ingetrazo_render.several_views_shown


#: Texture images decoded once per file, for every GL context that
#: uploads them: ``path`` → ``(mtime, size, mirrored QImage, cutout)``,
#: the newest last, held to a budget of bytes. Three views joining the
#: plaza's decoded its textures again each (1.6 s of the 3.8 s); the GL
#: texture itself belongs to each context and is made from this.
_DECODED_TEXTURES: dict = {}
_DECODED_BUDGET = 256 * 1024 * 1024


_decoded_texture = ingetrazo_render.decoded_texture


def _array_bake():
    """views.array_bake, which imports back here (its import statement ran
    per placement, per frame pass)."""
    return sys.modules.get("views.array_bake") or importlib.import_module(
        "views.array_bake")


def _proto_wrapper(vp, mesh, paint=None):
    return ingetrazo_render.proto_wrapper(vp, mesh, paint)


_fp_winding = ingetrazo_render.fp_winding


_flags_read = ingetrazo_render.flags_read


#: flat look.
_BY_LAYER_MODES = ("textures", "shaded", "xray")


def _layer_look_sig(scene, style=None) -> tuple:
    return ingetrazo_render.layer_look_sig(scene, style)


#: The face modes that draw the per-vertex colour, where the style's Front
#: color reaches the default faces through their bake. Hidden line and
#: Monochrome paint every face flat in it with a uniform instead.
_VCOLOR_MODES = ("textures", "shaded", "xray")


_front_tint = ingetrazo_render.front_tint


def _with_front_tint(paint, style):
    return ingetrazo_render.with_front_tint(paint, style)


def _paint_of(vp, group):
    return ingetrazo_render.paint_of(vp, group)


_ghost_edge_points = ingetrazo_render.ghost_edge_points


def _cache_ver(vp):
    return ingetrazo_render.cache_ver(vp)


def _content_ver(vp):
    return ingetrazo_render.content_ver(vp)


_plan_curved_edges = ingetrazo_core.plan_curved_edges


def _lazy_edge_may_be_curve(mesh, a, b, tol=1e-3) -> bool:
    return ingetrazo_core.lazy_edge_may_be_curve(mesh, a, b, tol)


def _snap_dropped_loose(vp):
    return ingetrazo_core.snap_dropped_loose(vp)


def _px_near(x, y, ok, px, py, thr, pts=None, seen=None):
    return ingetrazo_core.px_near(x, y, ok, px, py, thr, pts, seen)


_nearest_within = ingetrazo_core.document_nearest_within


#: The cursor's edge and the snap's read off one search in an event
#: (INGETRAZO_EDGES_SHARED=0: two searches, as before).
_EDGES_SHARED = os.environ.get("INGETRAZO_EDGES_SHARED", "1") != "0"


#: The front face at a pixel without gathering every face the ray meets
#: (INGETRAZO_FRONT_LEAN=0: through _face_hits, as before).
_FRONT_LEAN = os.environ.get("INGETRAZO_FRONT_LEAN", "1") != "0"


#: The nearest group edges picked in the native engine
#: (INGETRAZO_NEAREST=0: in NumPy, as before).
_NEAREST_NATIVE = os.environ.get("INGETRAZO_NEAREST", "1") != "0"


#: The depth test of the corners near the cursor in the native engine
#: (INGETRAZO_NEAR_SEEN=0: in NumPy, as before).
_NEAR_SEEN_NATIVE = os.environ.get("INGETRAZO_NEAR_SEEN", "1") != "0"


# Existing native callers still accept the legacy rounding-mode argument.
from core._native import qt_hypot
_QV_HYPOT3 = qt_hypot()


def _hypot3_f32(a, kind="libstdc++"):
    return ingetrazo_core.hypot3_f32(a, kind)


def _qv_hypot3():
    return _QV_HYPOT3


_unit_rays = ingetrazo_core.document_unit_rays


# Barycentric roundoff on a shared triangle edge can reject both faces.
_BARY_EPS = 1e-14


_occlusion_block = ingetrazo_core.occlusion_block


_placed_maps = ingetrazo_render.placed_maps


def _batch_instances(base, xfs, tri_parts, hard_parts, soft_parts,
                     n_parts) -> None:
    return ingetrazo_render.batch_hlr_instances(base, xfs, tri_parts, hard_parts, soft_parts, n_parts)


_cat_kept = ingetrazo_render.cat_kept


_offset_kept = ingetrazo_render.offset_kept


_moller_trumbore = ingetrazo_core.ray_triangles


#: How the rest of the model reads while a group is open for editing.
#: ``normal`` draws it as always, ``fade`` washes it out behind the group
#: being edited (the default), ``hide`` leaves it out of the frame.
EDIT_REST_MODES = ("normal", "fade", "hide")
#: Default for how far the context is mixed TOWARD grey (0 = untouched, 1 =
#: fully washed). It is a wash, not transparency: the context keeps writing
#: depth, so it still occludes itself and never turns the model into glass.
#: The user-adjustable value lives on :class:`Viewport`; this constant remains
#: the default and the fallback for the small viewport stubs used by tools and
#: tests.
EDIT_REST_FADE = 0.75

edit_fade_color = ingetrazo_render.edit_fade_color

#: X-ray: how opaque each translucent face layer is. Layers stack, so a face
#: seen through another reads darker and an opening (no face) stays clear.
XRAY_FACE_OPACITY = 0.6
#: X-ray: edges hidden behind a face are washed this far toward the
#: background (0 = like a visible edge, 1 = gone), so the form still reads
#: and you can tell which edges have a face in front of them.
XRAY_HIDDEN_EDGE_FADE = 0.6

#: Dot colour of a SELECTED face (the usual selection orange) and the one it
#: switches to on a surface that is itself orange/red, where orange dots
#: would vanish into the paint.
SELECTION_DOT_COLOR = (0.95, 0.45, 0.16)
SELECTION_DOT_ALT_COLOR = (0.10, 0.25, 0.85)


selection_dot_color = ingetrazo_render.selection_dot_color


_box_edges = ingetrazo_render.box_edges


_load_edit_rest_mode = ingetrazo_render.load_edit_rest_mode


_load_edit_rest_fade = ingetrazo_render.load_edit_rest_fade


_load_hide_similar = ingetrazo_render.load_hide_similar


#: Soft sun shadows (#1281), Preferences ▸ General: off (0) or a level, and
#: how fast the penumbra widens with the distance from what casts it, in
#: world width per world distance. The real sun is about 0.009; the levels
#: are broader, as a picture of soft daylight asks.
SOFT_SHADOW_SPREAD = {1: 0.012, 2: 0.03, 3: 0.06}


_load_soft_shadows = ingetrazo_render.load_soft_shadows


_AXIS_DIRS = {"x": (1.0, 0.0, 0.0), "y": (0.0, 1.0, 0.0), "z": (0.0, 0.0, 1.0)}


def _axes_vertices(spacing: float, pos_len: float = 1.0e5):
    return ingetrazo_render.axes_vertices(spacing, pos_len)


def _box_edge_vertices(corners):
    from ingetrazo_core import box_edge_vertices
    coords = array("f")
    coords.frombytes(memoryview(box_edge_vertices(corners)).cast("B"))
    return coords


def shown_guides(scene) -> list:
    return ingetrazo_render.shown_guides(scene)


from ingetrazo_core import point_in_screen_hull as _point_in_screen_hull
from ingetrazo_core import MIN_CIRCLE_DASHES as _MIN_CIRCLE_DASHES


def _guide_vertices(guides, spacing: float, selection=(), centre=None,
                    reach: float = 0.0, eye=None):
    from core import axes
    from ingetrazo_core import guide_vertices
    coords, spans = guide_vertices(guides or (), spacing, selection or (),
                                   centre, reach, eye,
                                   None if axes.is_world() else axes.frame())
    data = array("f")
    data.frombytes(memoryview(coords).cast("B"))
    return data, spans


def _ray_triangle(
    origin: QVector3D,
    direction: QVector3D,
    v0: QVector3D,
    v1: QVector3D,
    v2: QVector3D,
) -> Optional[float]:
    return ingetrazo_render.scalar_ray_triangle(origin, direction, v0, v1, v2)


def _point_to_segment_distance_2d(p, a, b) -> float:
    return ingetrazo_render.point_segment_distance(p, a, b)


# ---- Viewport --------------------------------------------------------------

def _parse_number(tok: str):
    """``2``, ``2.5``, ``.5``, ``3/4`` (a fraction), ``1-1/2`` (a mixed
    number, the hyphen form) → float, else None — ``core.units`` reads it,
    refusing a number too long for a float or too big for a model (#185)."""
    from core.units import _number
    return _number(tok)


def _parse_length_field(field: str):
    """One typed length in metres: metric or imperial, the usual forms.

    ``2`` (the document's unit, issue #33) · ``30cm`` · ``1500mm`` ·
    ``1m20`` · ``2"`` / ``2in`` · ``1'`` / ``1ft`` · ``1'6"`` · ``3/4"`` ·
    ``1'3/4"``. A leading minus is kept. Returns None when the field is not
    a length. The reading itself lives in ``core.units.parse``, the one
    every length field shares."""
    from core.units import parse_length_field
    return parse_length_field(field)


class _AltWatch(object if NATIVE else QObject):
    """Hears Alt at APPLICATION level, without taking it, for the Paint
    tool's eyedropper pointer (@pacaeiro, #47: «press Alt and release it —
    icon changed but we're still on Paint mode»). The viewport's own key
    events miss every other tap: the first release hands keyboard focus to
    the menu bar, so the next press lands there. The function followed (it
    reads the mouse event's modifiers); only the pointer lagged.

    ONE filter for the whole application, telling every live viewport: an
    application filter runs on every event, and one per viewport made the
    test suite crawl (hundreds of viewports alive)."""

    _instance = None

    def __init__(self, app) -> None:
        super().__init__(app)
        import weakref
        self._viewports = weakref.WeakSet()
        self._clean = False       # Alt down, nothing else pressed since

    @classmethod
    def watch(cls, viewport) -> None:
        from PySide6.QtWidgets import QApplication
        app = QApplication.instance()
        if app is None:
            return
        if cls._instance is None:
            cls._instance = cls(app)
            app.installEventFilter(cls._instance)
            # Off before Python tears itself down: Qt keeps delivering
            # events (deferred deletes) through an application filter while
            # the interpreter is half gone — the suite segfaulted on exit.
            import atexit
            atexit.register(cls._unwatch)
        cls._instance._viewports.add(viewport)

    @classmethod
    def _unwatch(cls) -> None:
        inst, cls._instance = cls._instance, None
        if inst is None:
            return
        from PySide6.QtWidgets import QApplication
        app = QApplication.instance()
        try:
            if app is not None:
                app.removeEventFilter(inst)
        except RuntimeError:
            pass

    def eventFilter(self, obj, ev) -> bool:
        return ingetrazo_render.alt_filter(self, ev)


def _face_geometry(f, memo, triangulate):
    return ingetrazo_render.face_geometry(f, memo, triangulate)


_finite_point = ingetrazo_render.finite_point


_curved_faces = ingetrazo_render.curved_faces


_smooth_vcol = ingetrazo_render.smooth_vcol


from views.document_tools import DocumentTools
from views.view_navigation import ViewNavigation


class DocumentInteraction(DocumentTools, ViewNavigation):
    """Editing and input shared by the document presentation hosts."""

    def _configure_surface(self):
        pass

    valueBufferChanged = Signal(str)
    sceneVersionChanged = Signal(int)
    # Live measurement for the VCB box (e.g. "5.00 m", "3.00 × 2.00 m").
    measurementChanged = Signal(str)
    # A base-map tile finished downloading (Track G) — lets the 3D terrain
    # rebuild its mosaic as imagery arrives.
    tilesChanged = Signal()
    # UTM coordinate under the cursor, for the status bar readout (Track G).
    coordinateChanged = Signal(str)
    # The camera's projection changed ("parallel" / "perspective" /
    # "two_point"), whoever changed it — the View toolbar's quick buttons
    # follow it.
    projectionChanged = Signal(str)
    # A trackpad's smart zoom (two-finger double tap on macOS): the window
    # frames the model, and a second one goes back (#248).
    smartZoomRequested = Signal()

    # Soft warm white painted on faces with no material colour — like the matte
    # cardstock of an architecture model (a near-white default).
    DEFAULT_FACE_COLOR = DEFAULT_FACE_COLOR
    # Fixed world light (from above, slightly front-right) for the subtle diffuse
    # face shading. World-fixed so shading is stable while orbiting.
    _LIGHT = QVector3D(0.35, 0.25, 1.0).normalized()

    # Tooltip text shown next to the snap marker. English source
    # strings; translated at draw time via ``tr`` (see i18n/es.json).
    #: How close (px) the cursor must come to an acquired circle centre
    #: for its green dot to show — beyond that the reference stays silent.
    #: The real radius is a share of the circle's size on screen, capped
    #: here (see ``_center_hint_px``).
    CENTER_HINT_PX = 40.0
    #: A circle smaller than this on screen offers no centre at all.
    CENTER_MIN_RADIUS_PX = 12.0

    #: How long the cursor must rest on a point to encourage it (ms).
    ENCOURAGE_MS = 250
    #: Snap kinds whose point can be encouraged (a definite point, not a
    #: place along an edge or on a face).
    _ENCOURAGING_KINDS = frozenset((
        "endpoint", "midpoint", "arc_midpoint", "center", "component_origin",
        "origin", "intersection", "intersection_edge", "intersection_face",
        "close", "face_center"))

    _SNAP_LABELS = {
        "endpoint": "Endpoint",
        "midpoint": "Midpoint",
        "arc_midpoint": "Arc midpoint",
        "quadrant": "Quadrant",
        "component_origin": "Component origin",
        "on_line": "On line",
        "on_circle": "On circle",
        "on_axis": "On axis",
        "tangent": "Tangent",
        "on_edge": "On edge",
        "on_face": "On face",
        "face_center": "Face center",
        "origin": "Origin",
        "extension": "Extension",
        "intersection": "Intersection",
        "intersection_edge": "Intersect with Edge",
        "intersection_face": "Intersect with Face",
        "from_point": "From point",
        # The axis PLANE through an encouraged point cut by the face under
        # the cursor (rule 8e): "Level with point" when it is the horizontal
        # one, see ``_draw_snap_indicator``.
        "aligned": "In line with point",
        "through_point": "Through point",
        "perp_face": "Perpendicular to face",
        "center": "Center",
        "guide_point": "Guide point",
    }

    _bind_bake_store = _bind_bake_store

    def __init__(self, parent=None, **surface) -> None:
        super().__init__(parent, **surface)
        self.view_filter = None
        self.view_tint = None
        self.active_view_key = None
        # The model and its camera are the same in every language: a right-
        # to-left interface (#1067) does not mirror what is drawn here.
        if not NATIVE:
            from views.rtl import keep_ltr
            keep_ltr(self)
        self._configure_surface()
        self.setMinimumSize(640, 480)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setMouseTracking(True)

        self._init_document()

        ingetrazo_render.init_document_interaction(self)
        if not NATIVE:
            _AltWatch.watch(self)
        ingetrazo_render.init_navigation(self)

    def _sync_axes(self) -> None:
        """Make the drawing axes the current context's (issue #44) — the
        world's at the top level, the open group's own inside it."""
        from core import axes as _axes
        _axes.sync(getattr(self.scene, "drawing_frame", None))

    #: This picture's background is alpha 0 (``render_image``'s ``transparent``).
    _export_transparent = False

    #: Off in a view of several (ingetrazo_render.DocumentViews) other than the one being
    #: worked in: the model shows there, without the tool in hand's preview.
    tool_preview = True

    #: How many viewports are inside ``paintGL`` right now (a view of
    #: several paints the others from its own). What a paint announces must
    #: not show or hide a widget beside the view: that resizes it in the
    #: middle of its frame, and macOS crashed on the flush after (#1316).
    painting = 0

    # ---- Instanced components (P2) ------------------------------------------
    # Component instances draw from ONE static proto VBO (local coords) plus
    # a per-instance matrix buffer (divisor 1): N hedges cost one upload and
    # one draw per proto run. Instances keep their baked chunks for picks,
    # silhouettes, selection and the drag preview; only the CONSOLIDATED
    # draw excludes them. INGETRAZO_NO_INSTANCING=1 reverts to the old path.

    def _proto_base_chunk(self, mesh, paint=None):
        """The prototype's chunk in LOCAL coordinates (the same wrapper
        ``_instance_chunk`` derives from). ``paint`` is the container
        material the variant is baked in (issue #47): a painted instance
        has its own bake of the prototype, unpainted siblings share the
        plain one."""
        return self._group_chunk(_proto_wrapper(self, mesh, paint))

    #: How many past epochs keep their parts for ``_epoch_only_matrices``.
    _EPOCH_HISTORY = 4

    def prepare_drawing(self, progress=None) -> None:
        return ingetrazo_render.prepare_drawing(self, progress)

    def _placement_bbox(self, g):
        """World AABB of an instanced placement from its PROTOTYPE's local
        box and the placement matrix — eight corners, no bake. Reading it
        off ``_group_chunk(g)`` baked the whole placement to world
        coordinates only to take its box: on the model of issue #158 that
        was 21 000 baked copies of geometry the instanced pass never uses."""
        return self._placement_frame(g)[0]

    #: How far past the view the instanced batches reach, as a fraction of
    #: its width and height, so an orbit keeps drawing the same ones
    #: (_gather_instanced).
    _INST_VIEW_MARGIN = 0.10
    #: ... and while they hold at most this many times what is in view.
    _INST_SUPERSET_MAX = 2

    #: An instance whose bounding sphere spans less than this many
    #: framebuffer pixels is not drawn (the camera's frame only).
    _INST_MIN_PX = 1.0

    def image_plane_at(self, screen_x: float, screen_y: float,
                       selectable_only: bool = False):
        return ingetrazo_render.image_plane_at(self, screen_x, screen_y, selectable_only)

    def pick_image_plane(self, screen_x: float, screen_y: float):
        """Selection entry point: like :meth:`image_plane_at` but skipping
        locked images, so a click on an aligned scan falls through to the
        geometry being drawn on it."""
        return self.image_plane_at(screen_x, screen_y, selectable_only=True)

    def image_in_front_of(self, face, screen_x: float, screen_y: float):
        return ingetrazo_render.image_in_front_of(self, face, screen_x, screen_y)

    def pick_locked_image_plane(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_locked_image_plane(self, screen_x, screen_y)

    def pick_locked_group(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_locked_group(self, screen_x, screen_y)

    def finish_image_placement(self) -> None:
        """Called by the Image tool once a picture is placed: drop back to
        Select, the classic behaviour after a one-shot placement."""
        win = self.window()
        if hasattr(win, "activate_select_tool"):
            win.activate_select_tool()

    # ---- Line weight for exports --------------------------------------------
    # ---- Base-map tiles (Track G) -------------------------------------------
    #: Per-document caches keyed by ``id()`` of the document's groups,
    #: meshes and placements — the render/pick chunks and what hangs off
    #: them. Nothing but the document boundary makes them all stale at once.
    _DOCUMENT_CACHES = ("_group_chunks", "_inst_chunks", "_fp_memo",
                        "_proto_wrappers", "_proto_draw", "_hidden_edge_draws", "_faceme_cache",
                        "_proto_pts_store", "_container_obb", "_placement_frames",
                        "_pick_live", "_pick_foot_keep",
                        # Also keyed by id(): a face-me's placed sprite, the
                        # nested-placement proxies and the arc midpoints per
                        # mesh. A group of the next document born at a dead
                        # one's address met its entry — the scale figure
                        # «transported» into an opened file (issue #75).
                        "_billboard_world", "_placement_proxies",
                        "_placement_tops", "_arc_mid_by_mesh",
                        "_billboard_extents", "_figure_tokens")

    def refresh_after_edit(self) -> None:
        """Refresh an externally executed command without discarding geometry.

        Commands must bump the scene version and use the mesh mutation APIs,
        just as interactive tools do. The renderer compares mesh, material
        and placement signatures and rebuilds only changed payloads. Keep
        reset_document_caches() for New/Open, not individual bridge edits.
        Call once after a batch, outside an active transform preview.
        This does not create an undo step or mark a clean document dirty.
        """
        self.scene.bump_view()
        self.notify_scene_changed()


    def reset_document_caches(self, keep_bakes: bool = False) -> None:
        return ingetrazo_render.reset_document_caches(self, keep_bakes)






    def reset_tiles(self) -> None:
        return ingetrazo_render.reset_tiles(self)

    # Max NEW GL textures created per frame. Uploading hundreds at once (a big
    # capture) overwhelms Mesa and reads back as garbage (black/green tears at
    # the far edge); creating a few per frame spreads it — the map fills in over
    # a second and repaints itself until done.
    # P4: 6 uploads (each with mipmap generation, ~5-15 ms on the iGPU)
    # stacked into one frame was the ~100 ms hitch while zooming over the
    # base map. 2 per frame keeps every frame under the 33 ms gate; the
    # deferred-repaint loop below spreads the rest across frames.
    _TEX_PER_FRAME = 2


    def prefetch_tiles(self, source, tile_list, zoom) -> None:
        return ingetrazo_render.prefetch_tiles(self, source, tile_list, zoom)



    # ---- Photogrammetric survey (Track G, G6) ------------------------------








    # Sky (top) and ground (bottom) backdrop colours — subtle two-tone.
    _SKY_RGB = (0.925, 0.935, 0.945)
    _GROUND_RGB = (0.815, 0.820, 0.815)

    #: NDC span over which each half of the backdrop completes its ramp.
    _SKY_GRAD_SPAN = 1.4
    _GROUND_GRAD_SPAN = 0.9

    # ---- Sun shadows (core/sun.py) ------------------------------------------
    _SHADOW_MAP_SIZE = 2048
    #: The map an exported picture draws with (``render_image``'s
    #: ``sharp_shadows``): drawn once per picture, so its cost does not
    #: matter the way the view's does. Capped by the driver's texture size.
    _SHADOW_EXPORT_SIZE = 8192
    _shadow_size = _SHADOW_MAP_SIZE

    #: The .skp translucency rule: materials under 70 % opacity cast no
    #: shadow — the sun passes through glass.
    _SHADOW_OPACITY_MIN = 0.7

    #: How long the view must stay put before the fitted map is drawn (#958).
    SHADOW_SETTLE_MS = 200

    # ---- Dynamic uploads ----------------------------------------------------
    def prune_stale_selection(self):
        return ingetrazo_render.prune_stale_selection(self)

    def notify_scene_changed(self) -> None:
        return ingetrazo_render.document_scene_changed(self)

    def begin_groups_preview(self, groups=(), external: bool = False) -> None:
        return ingetrazo_render.begin_groups_preview(self, groups, external)

    def end_groups_preview(self) -> None:
        return ingetrazo_render.end_groups_preview(self)

    def set_groups_preview_offset(self, delta: QVector3D) -> None:
        self._preview_offset = QVector3D(delta)
        self._preview_matrix = None
        self.update()

    def set_groups_preview_matrix(self, m) -> None:
        """Full-matrix variant (Rotate/Scale previews): the scratch copies
        draw through ``m`` — same zero-churn frame as the translation."""
        self._preview_matrix = QMatrix4x4(m)
        self.update()

    #: A face this big (corners, holes included) is worth handing to the
    #: native earcut ahead of ``_tris_of``.
    NATIVE_TRIS_MIN_CORNERS = 64

    def _faceme_dir(self, anchor: QVector3D) -> QVector3D:
        return ingetrazo_render.faceme_dir(self.camera, anchor)

    def set_hover(self, entity, placement=None) -> None:
        return ingetrazo_render.set_hover(self, entity, placement)

    def flash_status(self, text: str, msec: int = 2500) -> None:
        return ingetrazo_render.document_status(self, text, msec)

    def set_suppressed_faces(self, faces) -> None:
        return ingetrazo_render.set_suppressed_faces(self, faces)

    def _draw_snap_indicator(self, painter: QPainter, snap: SnapResult) -> None:
        # Axis-lock and inference state is conveyed by the coloured rubber
        # band; no badge follows the cursor along the lock line. Only the
        # discrete point snaps get a marker.
        return ingetrazo_render.draw_snap_indicator(self, painter, snap)

    def edit_group_box_contains_pixel(self, x: float, y: float) -> bool:
        from ingetrazo_core import edit_box_contains_pixel
        return edit_box_contains_pixel(self, x, y)

    # ---- Section planes ------------------------------------------------------

    def section_plane_frame(self, sp) -> list:
        from ingetrazo_core import section_frame
        return section_frame(sp.point, plane_axes(sp.normal), self.scene.bounds())

    def pick_section_plane(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_section_plane(self, screen_x, screen_y)

    #: The sheet's cut pen (mm) a layer's own cut weight is measured
    #: against in the model: a layer at 0.50 draws the usual ~2.5 px line.
    CUT_PEN_MM = 0.5

    def _clip_segment_front(
        self, a: QVector3D, b: QVector3D
    ) -> Optional[tuple[QVector3D, QVector3D]]:
        from ingetrazo_core import clip_annotation_front
        return clip_annotation_front(self.camera.projection_matrix() * self.camera.view_matrix(), a, b)

    def _guide_snap_segment(
        self, g, margin: float = 1.5
    ) -> Optional[tuple[QVector3D, QVector3D]]:
        from core.guide import GUIDE_HALF_LEN
        from ingetrazo_core import clip_snap_guide
        return clip_snap_guide(self.camera.projection_matrix() * self.camera.view_matrix(),
                               g.point, g.direction, GUIDE_HALF_LEN, margin)

    def _clip_pixel_line(
        self, p0: tuple[float, float], p1: tuple[float, float],
        margin: float = 64.0,
    ) -> Optional[tuple[tuple[float, float], tuple[float, float]]]:
        from ingetrazo_core import clip_annotation_line
        return clip_annotation_line(p0, p1, self.width(), self.height(), margin)

    def ground_height(self, x: float, y: float) -> float | None:
        return ingetrazo_render.ground_height(self, x, y)

    def ground_elevation(self, x: float, y: float) -> float | None:
        return ingetrazo_render.ground_elevation(self, x, y)

    def vertical_reference(self) -> str:
        return ingetrazo_render.vertical_reference(self)

    def has_ground_surface(self) -> bool:
        return ingetrazo_render.has_ground_surface(self)

    def drape(self, v: QVector3D) -> QVector3D:
        return ingetrazo_render.drape(self, v)

    @staticmethod
    def _format_dim_value(metres: float, style: dict) -> str:
        """Format a length (metres) per the dimension style: unit + precision
        (see :mod:`core.units` — metric, decimal or fractional imperial),
        with or without the unit symbol (#374)."""
        from core.units import format_length
        return format_length(metres, style.get("units", "m"),
                             int(style.get("decimals", 2)),
                             bool(style.get("show_unit", True)))

    #: Screen pixels between the occlusion samples of a long annotation
    #: line (``_draw_occluded_segment``), and the most samples it takes.
    _OCCLUSION_STEP_PX = 12.0
    _OCCLUSION_MAX_SAMPLES = 128
    #: Where a line goes behind something, its sample step is cut in this
    #: many parts this many times: 1/64 of a step, under a pixel.
    _OCCLUSION_SPLIT = 8
    _OCCLUSION_ROUNDS = 2

    def _project_samples(self, pts: list[QVector3D]):
        from ingetrazo_core import annotation_samples
        return annotation_samples(self, pts, self.camera.projection_matrix() * self.camera.view_matrix())

    # ---- Pixel ↔ world ------------------------------------------------------

    #: A ray meeting the work plane below this angle gives a point nobody
    #: asked for. Measured: at 25 deg of camera pitch — an ordinary
    #: architectural view — nothing on screen comes anywhere near it, so this
    #: only fires where the plane really is unreadable.
    GRAZING_PLANE_DEG = 6.0

    def _work_plane_normal(self) -> Optional[QVector3D]:
        """The normal of the plane the last pixel→world used, or ``None``."""
        plane = getattr(self, "_last_work_plane", None)
        return plane[1] if plane is not None else None

    #: XY stays the ground in shallow views and where its cursor-ray hit is
    #: reasonably close compared with the two vertical global planes.
    GROUND_PREFERENCE_ANGLE_DEG = 5.0
    GROUND_DEPTH_PREFERENCE_RATIO = 1.25
    PERSPECTIVE_GROUND_DEPTH_PREFERENCE_RATIO = 10.0
    PLANE_ALIGNMENT_TIE_MARGIN = 0.05

    # When the camera is at least this tilted off the horizon, the work plane
    # stays horizontal (XY) — that covers top, iso, and most architectural
    # angles. Only at near-horizon views does it switch to a vertical plane,
    # which is the only orientation where cursor-to-ground is ambiguous.
    HORIZON_PITCH_THRESHOLD_DEG = 15.0

    #: How far from the cursor (px) a face crossing snaps, and how far
    #: either side of the crossing the search checks that each face shows.
    _CROSSING_RING_PX = 8.0
    _CROSSING_SIDE_PX = 3.0

    #: More selected faces than this and their centres are neither drawn
    #: nor offered: a «select all» would bury the model in dots.
    FACE_CENTER_SELECTION_MAX = 500

    #: Eye depth (m) a segment is cut at when it runs behind the camera.
    _CLIP_W = 1e-3

    def _segment_to_pixels(self, p0: QVector3D, p1: QVector3D):
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        return ingetrazo_render.segment_to_pixels(mvp, p0, p1, self._clip_w(),
                                                 self.width(), self.height())

    def _draw_extension_overlays(self, painter: QPainter) -> None:
        return ingetrazo_render.document_paint_extensions(self, painter)

    def world_to_pixel(self, world: QVector3D) -> Optional[tuple[float, float]]:
        """Public API (extensions): world point (metres) → widget pixel
        ``(x, y)`` in logical pixels, or ``None`` when the point is behind
        the camera. The same projection the host's own overlays use."""
        return self._world_to_pixel(world)

    def world_to_pixels(self, pts):
        """Public API (extensions): an ``(N, 3)`` float array of world points
        (metres) → ``(px, py, in_front)`` NumPy arrays, vectorised — for
        overlays with thousands of points, where a Python loop over
        :meth:`world_to_pixel` would cost frames. Points with
        ``in_front == False`` are behind the camera; skip them."""
        import numpy as np
        return self._project_px(np.asarray(pts, dtype=float).reshape(-1, 3))

    # ---- On-disk chunk cache (P4: fast cold start) --------------------------
    # Chunk arrays are a pure function of the mesh content; persisting them
    # turns the multi-second cold build of a big group into a disk read.
    # Lives in the app's own cache dir (like extracted textures) and is
    # keyed by a STABLE sha1 content digest — a stale entry can only miss.

    _CHUNK_CACHE_MIN_FACES = 5000       # small groups rebuild faster than IO
    _CHUNK_CACHE_KEEP = 120             # files kept; oldest pruned at save

    @staticmethod
    def _chunk_cache_dir():
        from core.texture import texture_cache_root
        d = texture_cache_root().parent / "chunks"
        d.mkdir(parents=True, exist_ok=True)
        return d

    _CHUNK_CACHE_FIELDS = (
        "edges", "vcol", "smooth_vcol", "by_texture", "tcol", "ttex", "back_vcol",
        "back_tex", "back_tcol", "back_ttex", "fvcol", "dback", "areas",
        "v0", "e1", "e2", "tri_ent", "soft_pts", "soft_n0", "soft_c0",
        "soft_n1", "soft_c1", "soft_single", "edge_layer", "soft_layer",
        "bbox", "coordsum", "nv", "ne", "nf", "samples")

    def _visible_spans(self, spans, planes, split=None):
        from ingetrazo_core import visible_draw_spans
        return visible_draw_spans(self, spans, planes, split,
                                  len(spans) >= _SPAN_VECTOR_MIN and bool(planes))

    def _visible_spans_np(self, spans, planes, split=None):
        from ingetrazo_core import visible_draw_spans
        return visible_draw_spans(self, spans, planes, split, True)

    #: Component placements holding more faces than this, all together,
    #: switch the pick index to LAZY placements (issue #158): below it the
    #: index is exactly what it always was.
    _PICK_LAZY_MIN_FACES = 1_000_000
    #: How many faces of lazy placements the index keeps baked at once.
    _PICK_LIVE_MAX_FACES = 600_000
    #: Lazy placements baked by one query at most (the nearest first) —
    #: by all the footprint queries of one input event together.
    _PICK_NEW_PER_QUERY = 300
    #: A snap's footprint skips placements under this many framebuffer
    #: pixels across — the ones the frame does not draw.
    _PICK_MIN_PX = 1.0
    #: Screen margin, in pixels, around the cursor for «near the cursor».
    _PICK_NEAR_PX = 32.0

    def start_pick_warmup(self) -> None:
        return ingetrazo_core.start_pick_warmup(self)

    def ray_distance(self, origin: QVector3D, direction: QVector3D,
                     reach: float | None = None):
        return ingetrazo_core.index_ray_distance(self, origin, direction, reach)

    def _ray_near(self, x: float, y: float):
        """What a ray pick at widget pixel ``(x, y)`` asks the pick index
        to bake: nothing with the engine, which answers for the lazy
        placements itself (see _face_hits), the placements the ray meets
        otherwise."""
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        return "skip" if nat is not None else ("px", x, y)

    def _hover_hits(self, idx, origin, direction):
        return ingetrazo_core.cached_face_hits(self, idx, origin, direction, False)

    def _infer_hits(self, idx, origin, direction):
        return ingetrazo_core.cached_face_hits(self, idx, origin, direction, True)

    def edge_point_under_cursor(self, edge, screen_x: float, screen_y: float):
        return ingetrazo_core.edge_cursor_point(self, edge, screen_x, screen_y)

    def pick_edge(self, screen_x: float, screen_y: float):
        """Return the edge closest to ``(screen_x, screen_y)`` within threshold."""
        return self._pick_edge(screen_x, screen_y, visible_only=False)

    def pick_visible_edge(self, screen_x: float, screen_y: float):
        """The edge a CLICK means: like :meth:`pick_edge`, but an edge the
        user cannot see never wins — a soft edge (the seams of a smooth
        surface, which are not drawn) unless hidden geometry is shown, and
        an edge behind the model. The usual rule, and what made a
        cylinder's side unclickable (@pacaeiro, issue #71: the invisible
        seams, front and back, took up to 90 % of the side at normal zoom).
        Tools that want the tube's seam as a reference keep pick_edge."""
        return self._pick_edge(screen_x, screen_y, visible_only=True)

    def linework_hidden(self, world: QVector3D) -> bool:
        """Whether a line through ``world`` is hidden for the Select tool:
        :meth:`_is_occluded` with a little depth slack, so a line lying ON
        a face — or a hair above or below it, where the drawn line
        z-fights the face (shown at one angle, gone at the next) — is
        still the line the cursor is on. See :func:`linework_slack_point`."""
        return self._is_occluded(linework_slack_point(self.camera, world))

    def pick_linework_edge(self, screen_x: float, screen_y: float):
        """``(edge, pixels)``: the loose edge the Select tool takes before a
        face under the cursor — the nearest one within the pick threshold
        that :meth:`linework_hidden` does not hide — or ``(None, inf)``."""
        return self._pick_edge(screen_x, screen_y, visible_only=True,
                               hidden=self.linework_hidden,
                               with_distance=True)

    def pick_linework_groups(self, screen_x: float, screen_y: float):
        return ingetrazo_core.linework_groups(self, screen_x, screen_y, False)

    def linework_groups_around(self, screen_x: float, screen_y: float):
        return ingetrazo_core.linework_groups(self, screen_x, screen_y, True)

    def pick_dimension(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_dimension(self, screen_x, screen_y)

    def pick_text_label(self, screen_x: float, screen_y: float,
                        rect_only: bool = False):
        return ingetrazo_render.pick_text_label(self, screen_x, screen_y, rect_only)

    def pick_geopath(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_geopath(self, screen_x, screen_y)

    #: The group-edge distances' contract with their two readers (the
    #: hovered-edge pick, within ``pick_threshold_px``; the snap prefilter,
    #: the ``GEDGE_ENOUGH`` nearest within 48 px): exact within
    #: ``GEDGE_NEAR_PX``, and within ``GEDGE_REACH_PX`` too unless at least
    #: ``GEDGE_ENOUGH`` edges already lie inside the near radius — then the
    #: nearest ones are all there. Past that a big model says ``inf``.
    GEDGE_NEAR_PX = 8.0
    GEDGE_REACH_PX = 64.0
    GEDGE_ENOUGH = 48
    #: The radius of the snap's own search of group edges near the cursor
    #: (``_nearby_group_edges``'s default), which narrower ones in the
    #: same event are read off (see _nearby_edges_unbaked).
    GEDGE_SHARED_PX = 48.0
    #: From this many group edges up, the distances come through a screen
    #: grid (core.edge_grid) instead of one pass over all of them.
    GEDGE_GRID_MIN = 20000

    def pick_edge_any(self, screen_x: float, screen_y: float):
        return ingetrazo_core.pick_edge_any(self, screen_x, screen_y)

    #: How close (px) the cursor must come to a component's origin or an
    #: arc's midpoint for it to enter the snap scene at all.
    POINT_INFERENCE_PX = 40.0

    def pick_axis(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_axis(self, screen_x, screen_y)

    def _shown_guides(self) -> list:
        return shown_guides(self.scene)

    def pick_guide(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_guide(self, screen_x, screen_y)

    def pick_vertex(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_vertex(self, screen_x, screen_y)

    def hlr_geometry(self, tags: bool = False):
        return ingetrazo_render.hlr_geometry(self, tags)

    def hlr_geometry_now(self):
        return ingetrazo_render.hlr_geometry_now(self)

    def _view_style(self):
        """The style this view shows outside the composer: its own when it
        has one (one of several views, restyled there), else the scene's."""
        return (getattr(self, "view_style", None)
                or getattr(self.scene, "display_style", None))

    # ---- Screen-depth occlusion for the overlay's annotations ---------------



    #: Cap on rays × triangles in one NumPy block of ``_occluded_many``
    #: (×3 float64 per array: ~50 MB at most).
    _OCCL_BLOCK = 2_000_000

    def pick_face(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_face(self, screen_x, screen_y)

    def pick_face_any(self, screen_x: float, screen_y: float):
        """Front-most face under the cursor across the loose mesh **and** every
        group: returns ``(face, group_or_None)``. Same coplanar tiebreak as
        :meth:`pick_face` (the smallest of the overlapping faces wins). The
        group half is what lets a tool tell "this face is inside a container"
        — drawing takes its plane, Push/Pull refuses it until the group is
        opened. While a group IS open its own faces are the loose part and
        come back with ``None``; the model around it is in the index for
        snapping and occlusion only, never as an answer here (the snap
        engine's own reading is :meth:`pick_face_inference`).

        Memoised per cursor position and view: a hover asks up to three
        times (work plane, acquisition, on-face flag) for the same answer."""
        face, owner, _place, _mesh = DocumentInteraction._face_any(
            self, screen_x, screen_y, "sel")
        return face, owner

    def pick_face_inference(self, screen_x: float, screen_y: float):
        """The face the SNAP ENGINE may read under the cursor as
        ``(face, placement_or_None)``: selectable faces as
        :meth:`pick_face_placement`, plus the rest of the model drawn
        around an open group / component — faded, never pickable, but a
        reference the cursor may infer «On face» from, and the plane a
        planar tool or a moved part lands on.

        Picking never uses this: a click outside the open context must not
        select or edit what it cannot touch (see :meth:`pick_face_any`)."""
        face, _owner, place, _mesh = DocumentInteraction._face_any(
            self, screen_x, screen_y, "infer")
        return face, place

    def pick_face_placement(self, screen_x: float, screen_y: float):
        """Like :meth:`pick_face_any`, but the group half is the PLACEMENT
        the face was hit through — the proxy whose composed matrix puts it
        in the world — not the top-level owner a click would select. What
        every face-PLANE reading needs: the owner of a nested component
        carries the container's matrix, so a rectangle on the pergola's
        post got the post's plane in the wrong place (Marco, 2026-09-14)."""
        face, _owner, place, _mesh = DocumentInteraction._face_any(
            self, screen_x, screen_y, "sel")
        if face is None:
            return None, None
        return face, place

    def pick_group(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_group(self, screen_x, screen_y)

    # ---- Copy / paste -------------------------------------------------------
    def copy_selection(self, *, publish=True) -> bool:
        return ingetrazo_render.copy_selection(self, publish)

    def cut_selection(self, *, publish=True) -> bool:
        return ingetrazo_render.cut_selection(self, publish)

    # ---- Group-edit context (Groups v2) --------------------------------------
    def _offer_link_source(self, group) -> None:
        """«“Site.igz” is linked: open it to edit it?» — and open it in a
        tab of its own (or bring that tab forward) on yes."""
        from pathlib import Path
        from core import links
        from views import chrome_dialogs
        path = Path(links.link_of(group)["path"])
        if not path.is_file():
            chrome_dialogs.warning(
                self, tr("Linked document"),
                tr("“{name}” is a linked document, and its file was not "
                   "found:\n{path}", name=path.name, path=str(path)))
            return
        # The Built UI asks on its sheet; Qt when the chrome is not on screen.
        answer = chrome_dialogs.ask(
            self, tr("Open the source file “{name}”?", name=path.name),
            tr("It is edited in its own document; every change saved there "
               "shows here."),
            [("cancel", tr("Cancel")), ("open", tr("Open {name}", name=path.name))],
            default="open", cancel="cancel")
        if answer != "open":
            return
        window = getattr(self, "window", None)
        window = window() if callable(window) else None
        opener_fn = getattr(window, "open_path", None)
        if callable(opener_fn):
            from views.wgpu_tabs import open_link_source
            open_link_source(window, path, group)

    # ---- Rest-of-model context while editing a group -------------------------
    @property
    def edit_rest_mode(self) -> str:
        """``normal`` / ``fade`` / ``hide`` — how the model outside the group
        being edited reads. See :data:`EDIT_REST_MODES`."""
        return self._edit_rest_mode

    def set_edit_rest_mode(self, mode: str) -> None:
        return ingetrazo_render.set_edit_rest_mode(self, mode)

    @property
    def edit_rest_fade(self) -> float:
        """0..1 strength of the grey wash applied outside the edit context."""
        return float(getattr(self, "_edit_rest_fade_strength",
                             EDIT_REST_FADE))

    def set_edit_rest_fade(self, strength: float) -> None:
        return ingetrazo_render.set_edit_rest_fade(self, strength)

    def set_hide_similar_components(self, on: bool) -> None:
        return ingetrazo_render.set_hide_similar_components(self, on)

    def _rest_is_hidden(self) -> bool:
        """Whether the model outside the edited group is out of this frame."""
        return (self.scene.edit_group is not None
                and self._edit_rest_mode == "hide")

    def _owner_of(self, group):
        """The object a click on ``group`` must select: a nested placement
        proxy stands for the top-level group that owns it."""
        return getattr(group, "owner", None) or group

    def set_nav_mode(self, mode: Optional[str]) -> None:
        return ingetrazo_render.set_nav_mode(self, mode)

    # ---- Input --------------------------------------------------------------
    # ---- Mouse look (a tool with ``on_look``: First Person) -----------------


    # -- Snapping inference, delegated to self.inference -----------------
    # The reference state (the hover / acquired edges and points, the centre
    # reference, the axis and reference locks, the last snap) lives in
    # core.inference.InferenceEngine, and so do the rules that used to be the
    # two post-processors of the search (the axis-source cue, the extension
    # hooks). These properties and thin wrappers keep the surface the tools,
    # the extensions, the probe and the tests read and write, so the
    # extraction stays behaviour-neutral.
    _acquired_point = ingetrazo_render.InferenceField("_acquired_point")
    _encouraged = ingetrazo_render.InferenceField("_encouraged")
    _dwell_point = ingetrazo_render.InferenceField("_dwell_point")
    _dwell_timer = ingetrazo_render.InferenceField("_dwell_timer")
    axis_lock = ingetrazo_render.InferenceField("axis_lock")
    last_snap = ingetrazo_render.InferenceField("last_snap")
    reference_edge = ingetrazo_render.InferenceField("reference_edge")
    reference_mode = ingetrazo_render.InferenceField("reference_mode")
    _shift_lock = ingetrazo_render.InferenceField("_shift_lock")
    _shift_lock_ref = ingetrazo_render.InferenceField("_shift_lock_ref")
    _hover_edge = ingetrazo_render.InferenceField("_hover_edge")
    _hover_center = ingetrazo_render.InferenceField("_hover_center")
    _center_ref = ingetrazo_render.InferenceField("_center_ref")
    _acquired_edge = ingetrazo_render.InferenceField("_acquired_edge")
    _acquired_face_normal = ingetrazo_render.InferenceField("_acquired_face_normal")

    def _dwell_on(self, point: Optional[QVector3D]) -> None:
        self.inference._dwell_on(point)

    def _encourage_dwelt(self) -> None:
        self.inference._encourage_dwelt()

    def encourage_point(self, point: QVector3D) -> None:
        self.inference.encourage_point(point)

    # The two post-processors of the search (the axis-source cue for the
    # tools that read the model axes as a source, the extension hooks) moved
    # behind ``self.inference.snap()`` too. The names stay on the viewport
    # because the extension tests and the probe call them from here.
    def _axis_source_cue(self, snap, px_x: float, px_y: float):
        return self.inference._axis_source_cue(snap, px_x, px_y)

    def _extension_snap(self, snap, px_x: float, px_y: float):
        return self.inference._extension_snap(snap, px_x, px_y)

    # Below this many pixels of drag, a left press/release is a click, not a box.
    BOX_DRAG_THRESHOLD_PX = 4.0

    def wheelEvent(self, ev) -> None:
        # While a 3D mouse moves the view, wheel notches are its driver's
        # imitation of the same push (3Dconnexion does that for programs it
        # does not know): zooming on them too made the view jump a step
        # about eight times a second over the smooth move.
        return ingetrazo_render.document_wheel(self, ev)


    #: The least a notch of the wheel backs out, from a close-up, once the
    #: camera stands 2 cm or more from what it looks at — and always in an
    #: empty scene, which has no size to go by.
    _ZOOM_OUT_STEP = 0.05


    _MODIFIER_KEYS = frozenset({Qt.Key_Control, Qt.Key_Shift, Qt.Key_Alt,
                                Qt.Key_Meta, Qt.Key_AltGr})

    def ctrl_tapped(self) -> bool:
        """Whether the Ctrl being released was pressed ALONE — no other key
        in between, not even one a menu shortcut took (Ctrl+Z reaches us
        only as a ShortcutOverride). Tools that toggle on a Ctrl tap ask
        this on the release (#183)."""
        return getattr(self, "_ctrl_alone", False)

    def event(self, ev) -> bool:
        if ingetrazo_render.document_event(self, ev):
            return True
        return super().event(ev)

    def keyPressEvent(self, ev) -> None:
        if ingetrazo_render.document_key_press(self, ev):
            return
        return super().keyPressEvent(ev)

    def escape(self) -> None:
        return ingetrazo_render.document_escape(self)

    # ---- Items an extension lets the user select (issue #205) ---------------
    #: The one extension item selected — ``(entry, item_id, scene version)``
    #: — or None. Kept APART from ``scene.selection``, which only ever holds
    #: the model's own entities, so nothing that walks the selection (Move,
    #: layers, zoom, copy) meets a thing it does not know.
    extension_pick = None

    def pick_extension_item(self, px: float, py: float):
        return ingetrazo_render.pick_extension_item(self, px, py)

    def set_extension_pick(self, hit) -> None:
        return ingetrazo_render.set_extension_pick(self, hit)

    def clear_extension_pick(self, notify: bool = True) -> None:
        return ingetrazo_render.clear_extension_pick(self, notify)

    def delete_extension_pick(self) -> bool:
        return ingetrazo_render.delete_extension_pick(self)

    def release_constraints(self) -> bool:
        return ingetrazo_render.release_constraints(self)

    def refresh_status_hint(self) -> None:
        return ingetrazo_render.refresh_status_hint(self)

    def keyReleaseEvent(self, ev) -> None:
        if ingetrazo_render.document_key_release(self, ev):
            return
        return super().keyReleaseEvent(ev)

    #: The view shown in this one's place (the Metal view), which takes the
    #: keyboard whenever this hidden one is given it.
    keys_stand_in = None

    def setFocus(self, *args) -> None:  # noqa: N802 — Qt's name
        if ingetrazo_render.document_focus(self, args):
            return
        return super().setFocus(*args)

    def focusNextPrevChild(self, next) -> bool:  # noqa: A002 — Qt's name
        """Tab is a tool key (``views.shortcuts.reserved_reason``): Qt's
        focus chain took it and left the keyboard on a tray button, where
        the arrows, typed values and Enter did nothing."""
        return False

    def focusOutEvent(self, ev) -> None:
        ingetrazo_render.document_focus_out(self)
        super().focusOutEvent(ev)

    # Inferences whose direction can be captured by a Shift lock.
    _SHIFT_LOCKABLE = frozenset({
        "axis", "axis_inference", "reference", "through_point", "perp_face",
        "extension", "from_point",
    })

    # ---- Zoom: the lens typed in the value box ------------------------------
    def apply_zoom_value(self, typed: str) -> bool:
        return ingetrazo_render.apply_zoom_value(self, typed)

    # ---- Numeric value buffer (VCB-style) ----------------------------------
    def _handle_value_key(self, ev) -> bool:
        """Route a value-box key to its document tool or the camera lens."""
        return _value_key(self, ev)

    @staticmethod
    def _parse_value_buffer(buffer: str, comma_lists: bool = False):
        """The value box's text → a float, a 2-tuple ``(w, h)``, a 3-tuple
        ``(dx, dy, dz)``, an array, radius, profile, segments or slope
        tuple, or None (``core.units.parse_value_box``: the grammar is
        Rust's, ``rust/ingetrazo_core/src/units.rs``). Each tool's
        ``on_value`` takes the arity it understands. ``comma_lists``: a
        comma separates fields when the entry has no ``;`` and no space
        (#152)."""
        from core.units import parse_value_box
        return parse_value_box(buffer, comma_lists)

    def toggle_projection(self) -> None:
        self.camera.toggle_projection()
        self._sync_projection()
        self.update()

    def toggle_two_point(self) -> None:
        self.camera.toggle_two_point()
        self._sync_projection()
        self.update()

    def set_projection(self, mode: str) -> None:
        """Parallel, perspective or two-point (the View toolbar's buttons)."""
        self.camera.set_projection(mode)
        self._sync_projection()
        self.update()

    # ---- Helpers ------------------------------------------------------------


ingetrazo_render.register_document_host(DocumentInteraction)

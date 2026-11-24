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


def _active_cut(scene):
    """The section plane actually CUTTING (cuts shown + one active), or
    None. A module function so stub viewports in tests degrade gracefully;
    picks and snaps filter by it — what the cut hides is not clickable nor
    snappable."""
    if not getattr(scene, "show_section_cuts", True):
        return None
    active = getattr(scene, "active_section", None)
    return active() if callable(active) else None


class _SnapEdge:
    """Lightweight edge stand-in fed to the snap engine for group geometry —
    ``compute_snap`` only reads ``.a``/``.b`` (world endpoints). ``center``
    marks the degenerate one that carries a circle's centre."""

    __slots__ = ("a", "b", "center", "component_origin", "arc_midpoint",
                 "guide", "in_group", "context", "group", "figure")

    def __init__(self, a, b, center: bool = False, component_origin: bool = False,
                 arc_midpoint: bool = False, guide: bool = False,
                 in_group: bool = False, context=None, group=None,
                 figure: bool = False) -> None:
        self.a = a
        self.b = b
        self.figure = figure                       # a face-me figure's feet
        self.center = center
        self.component_origin = component_origin   # a group's insertion point
        self.arc_midpoint = arc_midpoint           # the middle of an arc's sweep
        self.guide = guide                         # a construction guide: 'on line'
        self.in_group = in_group                   # magenta point inferences
        self.context = context                     # "group" / "component" for the tip
        self.group = group                         # the top-level owner, when known


def _copy_snap_edge(e: _SnapEdge) -> _SnapEdge:
    """A fresh ``_SnapEdge`` like ``e``: one search's edges handed to two
    callers (see DocumentInteraction._nearby_edges_unbaked) are not shared."""
    c = _SnapEdge.__new__(_SnapEdge)
    for k in _SnapEdge.__slots__:
        v = getattr(e, k)
        setattr(c, k, QVector3D(v) if isinstance(v, QVector3D) else v)
    return c


def _snap_context_of(vp, place):
    """What the ScreenTip says the snapped geometry belongs to.

    ``None`` for the open context's own loose geometry (and at the root);
    ``"outside"`` for everything drawn around an open group / component —
    its loose mesh, another component, the rest of the model — so the tip
    can say the point is not in the thing being edited; otherwise
    ``"group"`` / ``"component"`` for the placement it was reached through.
    """
    if place is None:
        return None
    scene = getattr(vp, "scene", None)
    ctx = getattr(scene, "edit_group", None)
    if ctx is not None and place is not ctx \
            and getattr(place, "context", None) is not ctx:
        return "outside"
    owner = getattr(place, "owner", None) or place
    is_comp = getattr(owner, "is_instance", None)
    return "component" if (is_comp is not None and is_comp()) else "group"


def _group_pseudo_edge(idx, i: int) -> _SnapEdge:
    """The i-th group hard edge of the pick index as a world pseudo-edge
    that knows its owner and whether that is a group or a component — or
    the rest of the model around an open context ("outside")."""
    gi = getattr(idx, "gedge_gi", None)
    groups = getattr(idx, "gedge_groups", None)
    g = int(gi[i]) if gi is not None and groups else 0
    owner = groups[g] if gi is not None and groups else None
    ctxs = getattr(idx, "gedge_ctx", None)
    if ctxs is not None and g < len(ctxs):
        context = ctxs[g]
    else:
        is_comp = getattr(owner, "is_instance", None)
        context = "component" if (is_comp is not None and is_comp()) else "group"
    return _SnapEdge(QVector3D(*idx.gedge_a[i]), QVector3D(*idx.gedge_b[i]),
                     in_group=True, context=context, group=owner)


#: The depth slack of a line the Select tool may take: a fraction of the
#: eye's distance to it, and never less than a millimetre. A line drawn on
#: a face, or a few millimetres under or over it, z-fights that face: the
#: renderers draw it at one angle and lose it at the next (Metal, 2 mm
#: under a face 9 m away: drawn from 45°, gone from above), and the strict
#: occlusion test made it unclickable — the object behind took the click.
LINEWORK_SLACK = 5e-4
LINEWORK_SLACK_MIN = 1e-3


def linework_slack_point(camera, world):
    """``world`` moved towards the eye, along its own sight line, by the
    linework slack: whatever hides THAT point hides the line for good."""
    if getattr(camera, "perspective", True):
        towards = camera.eye() - world
    else:
        towards = -camera.forward()
    length = towards.length()
    if length < 1e-12:
        return QVector3D(world)
    depth = abs(QVector3D.dotProduct(world - camera.eye(), camera.forward()))
    slack = max(LINEWORK_SLACK_MIN, LINEWORK_SLACK * depth)
    return world + towards * (min(slack, 0.5 * length) / length)


def _boxes_around(keys, x0, y0, x1, y1, px, py, owner_of, scene):
    """The owners (``owner_of(key)``) of the screen boxes holding
    ``(px, py)``, smallest box first, each once: ``keys`` and the four
    bound arrays per segment, a box being the bounds of a key's segments."""
    import numpy as np
    if not len(keys):
        return []
    uniq, inv = np.unique(keys, return_inverse=True)
    lo_x = np.full(len(uniq), np.inf)
    lo_y = np.full(len(uniq), np.inf)
    hi_x = np.full(len(uniq), -np.inf)
    hi_y = np.full(len(uniq), -np.inf)
    np.minimum.at(lo_x, inv, x0)
    np.minimum.at(lo_y, inv, y0)
    np.maximum.at(hi_x, inv, x1)
    np.maximum.at(hi_y, inv, y1)
    inside = np.flatnonzero((lo_x <= px) & (px <= hi_x)
                            & (lo_y <= py) & (py <= hi_y))
    area = (hi_x - lo_x) * (hi_y - lo_y)
    out, seen = [], set()
    for k in inside[np.argsort(area[inside])]:
        owner = owner_of(int(uniq[k]))
        if owner is None or id(owner) in seen \
                or not scene.entity_selectable(owner):
            continue
        seen.add(id(owner))
        out.append(owner)
    return out




# ---- Geometry helpers ------------------------------------------------------

#: Beyond this many loose edges the snap engine gets a near-cursor subset
#: instead of the whole mesh: compute_snap walks its edge list in Python
#: ~a dozen times per hover, and an exploded medium import (9k faces =
#: ~20k edges) froze every mouse move (user report, piscina.igz).
_LOOSE_SNAP_CAP = 3000


def _inst_proto_of(mod, base):
    """core._native's InstanceProto of a prototype's local bake."""
    import numpy as np
    z = np.zeros((0, 3))
    v0 = base.get("v0")
    edges = (np.frombuffer(base["edges"], dtype=np.float32)
             .reshape(-1, 6).astype(np.float64)
             if base.get("edges") else np.zeros((0, 6)))
    return mod.InstanceProto(
        np.ascontiguousarray(v0) if v0 is not None else z,
        np.ascontiguousarray(base["e1"]) if v0 is not None else z,
        np.ascontiguousarray(base["e2"]) if v0 is not None else z, edges)


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
        make = self._later.pop(key, None)
        if make is None:
            raise KeyError(key)
        value = make()
        dict.__setitem__(self, key, value)
        return value

    def __setitem__(self, key, value):
        self._later.pop(key, None)
        dict.__setitem__(self, key, value)

    def __contains__(self, key):
        return dict.__contains__(self, key) or key in self._later

    def get(self, key, default=None):
        return self[key] if key in self else default

    def _made(self):
        for key in list(self._later):
            self[key]                    # noqa: B018 — makes it
        return self

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


def _occlusion_native():
    """core._native when it can take ``_occluded_many``'s rays, else None."""
    if not _OCCL_NATIVE:
        return None
    from core import _native
    mod = _native.native
    return mod if mod is not None and hasattr(mod, "occlusion_span") else None


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


def _bake_store_of(scene) -> dict:
    return ingetrazo_render.bake_store_of(scene)


def _drop_bake_store(key, ref) -> None:
    return ingetrazo_render.drop_bake_store(key, ref)


def _bind_bake_store(vp) -> None:
    return ingetrazo_render.bind_bake_store(vp)


#: Every viewport made, weakly: the decoded textures are kept only while
#: more than one of them is on screen (:func:`_several_views_shown`).
_LIVE_VIEWS: "weakref.WeakSet" = weakref.WeakSet()


def _several_views_shown() -> bool:
    return ingetrazo_render.several_views_shown()


#: Texture images decoded once per file, for every GL context that
#: uploads them: ``path`` → ``(mtime, size, mirrored QImage, cutout)``,
#: the newest last, held to a budget of bytes. Three views joining the
#: plaza's decoded its textures again each (1.6 s of the 3.8 s); the GL
#: texture itself belongs to each context and is made from this.
_DECODED_TEXTURES: dict = {}
_DECODED_BUDGET = 256 * 1024 * 1024


def _decoded_texture(path: str):
    return ingetrazo_render.decoded_texture(path)


def _array_bake():
    """views.array_bake, which imports back here (its import statement ran
    per placement, per frame pass)."""
    return sys.modules.get("views.array_bake") or importlib.import_module(
        "views.array_bake")


def _proto_wrapper(vp, mesh, paint=None):
    return ingetrazo_render.proto_wrapper(vp, mesh, paint)


def _fp_winding(fp) -> int:
    """The flip count a chunk fingerprint was taken at, 0 when it has no such
    term (see ``_mesh_fingerprint``). Module-level for the same reason as
    :func:`_proto_wrapper`."""
    return fp[7][1] if len(fp) > 7 and fp[7][0] == "winding" else 0


def _flags_read(mesh, entry: dict) -> None:
    return ingetrazo_render.flags_read(mesh, entry)


#: flat look.
_BY_LAYER_MODES = ("textures", "shaded", "xray")


def _layer_look_sig(scene, style=None) -> tuple:
    return ingetrazo_render.layer_look_sig(scene, style)


#: The face modes that draw the per-vertex colour, where the style's Front
#: color reaches the default faces through their bake. Hidden line and
#: Monochrome paint every face flat in it with a uniform instead.
_VCOLOR_MODES = ("textures", "shaded", "xray")


def _front_tint(style):
    return ingetrazo_render.front_tint(style)


def _with_front_tint(paint, style):
    return ingetrazo_render.with_front_tint(paint, style)


def _paint_of(vp, group):
    return ingetrazo_render.paint_of(vp, group)


def _ghost_edge_points(scene) -> array:
    """The ends of the edges of the mesh being edited that View ▸ Hidden
    Geometry draws dotted, on layers that are shown: the hidden ones, and
    the soft ones a curved surface or a bevel reads smooth with (#1361).
    The pick already takes soft edges while the view is on; drawn, they
    are no longer clicked blind.

    Module-level, like :func:`_cache_ver`, so tests reach it without a GL
    context."""
    points = array("f")
    for e in scene.mesh.edges:
        if ((getattr(e, "hidden", False) or getattr(e, "soft", False))
                and scene._layer_state(e)[0]):
            points.extend([e.a.x(), e.a.y(), e.a.z(),
                           e.b.x(), e.b.y(), e.b.z()])
    return points


def _cache_ver(vp):
    """The scene version the per-version caches key on — frozen during a
    groups-only transform preview (see DocumentInteraction.begin_groups_preview).
    Module-level so the stub viewports in tests hit it too."""
    frozen = getattr(vp, "_frozen_cache_version", None)
    return frozen if frozen is not None else vp.scene.version


def _content_ver(vp):
    """``_cache_ver`` without the view-only bumps (a selection, a group
    opened: ``Scene.bump_view``) — for caches of what the model IS, which a
    click must not throw away: on the plant of #158 every click re-walked
    21 406 placements and rebuilt every edge and outline (~1 s)."""
    frozen = getattr(vp, "_frozen_cache_version", None)
    if frozen is not None:
        return ("frozen", frozen)
    sc = vp.scene
    # Nor the section planes' own bumps: a plane moved changes no face.
    return getattr(sc, "geometry_version",
                   getattr(sc, "content_version", sc.version))


def _plan_curved_edges(plan):
    """The indices of a lazy plan's curve edges, in edge order (memoised
    on the plan, which cannot change while the mesh stays lazy)."""
    curved = plan.extra.get("curved_edges")
    if curved is None:
        import numpy as np
        ids = plan.curve
        # Most meshes have no curve at all: ``count`` tells in C.
        curved = plan.extra["curved_edges"] = (
            np.empty(0, dtype=np.int64) if ids.count(None) == len(ids)
            else np.array([i for i, c in enumerate(ids) if c is not None],
                          dtype=np.int64))
    return curved


def _lazy_edge_may_be_curve(mesh, a, b, tol=1e-3) -> bool:
    """Whether the edge of ``mesh`` from ``a`` to ``b`` (its own space) may
    be a curve's, asked of a still-lazy mesh's plan without building it.
    False only when no edge with both ends within ``tol`` of the two points
    — far wider than the weld lookups that find the real edge — carries a
    curve; a built (or never lazy) mesh always answers True."""
    from core.lazy_mesh import plan_of
    plan = plan_of(mesh)
    if plan is None:
        return True
    if not len(plan.edges):
        return False
    curved = _plan_curved_edges(plan)
    if not len(curved):
        return False
    import numpy as np
    ends = plan.edges[curved]
    p0, p1 = plan.pos[ends[:, 0]], plan.pos[ends[:, 1]]
    pa = np.array((a.x(), a.y(), a.z()))
    pb = np.array((b.x(), b.y(), b.z()))

    def near(p, q):
        return (np.abs(p - q) <= tol).all(axis=1)
    return bool(((near(p0, pa) & near(p1, pb))
                 | (near(p0, pb) & near(p1, pa))).any())


def _snap_dropped_loose(vp):
    """Ids of the loose edges the inference must not land on, cached per scene
    version. An EMPTY set means ``DocumentInteraction._snap_scene`` may hand the raw
    scene over, with no list to build on a hover.

    The engine walked the loose mesh as it is, so an edge hidden on its own —
    Hide with the eraser, the seam of a smoothed surface, an edge on a hidden
    layer — still offered its endpoints and its line. The cursor lit up on
    geometry that draws nothing, and a line drawn to that spot cut the face
    underneath at a point the user could not see: the "phantom corner" that
    segmented a fresh face out of nowhere. Render, pick and ``bounds()``
    already skip those edges; this asks the same rule
    (:meth:`Scene.edge_snappable`) once per change instead of once per edge
    per hover.

    Module-level, like :func:`_cache_ver`, so the stub viewports in tests hit
    it too."""
    scene = vp.scene
    key = (_cache_ver(vp), id(scene.mesh), len(scene.mesh.edges),
           bool(getattr(scene, "show_hidden_geometry", False)))
    memo = getattr(vp, "_loose_snap_drops_cache", None)
    if memo is not None and memo[0] == key:
        return memo[1]
    drops = {element_identity(e) for e in scene.edges if not scene.edge_snappable(e)}
    vp._loose_snap_drops_cache = (key, drops)
    return drops


def _px_near(x, y, ok, px, py, thr, pts=None, seen=None):
    """The projected points ``(x, y)`` in front (``ok``) within ``thr`` px of
    ``(px, py)``: their indices in order and distances, as one ``np.hypot``
    pass over all of them gives them. The native engine finds the
    candidates (the 650 000 corners of the plant of #158 took 12 ms a
    pointer move in NumPy) a hair wider than ``thr``, and those are measured
    here the same way. With the points ``pts`` and the frame's limits
    ``seen`` (see DocumentInteraction._depth_limits), those the frame hides are left
    out as well, as DocumentInteraction._depth_hidden leaves them out."""
    import numpy as np
    from core import _native
    mod = _native.native
    if mod is not None and hasattr(mod, "points_near_px") and len(x) > 4096:
        args = (np.ascontiguousarray(x, dtype=np.float64),
                np.ascontiguousarray(y, dtype=np.float64),
                np.ascontiguousarray(ok, dtype=bool),
                float(px), float(py), float(thr) * (1.0 + 1e-6) + 1e-6)
        hidden = None
        if seen is not None and _NEAR_SEEN_NATIVE:
            try:
                k = mod.points_near_px(
                    *args, np.ascontiguousarray(pts, dtype=np.float64), seen)
            except TypeError:               # an older module
                k = mod.points_near_px(*args)
                hidden = True
        else:
            k = mod.points_near_px(*args)
            hidden = seen is not None or None
        d = np.hypot(x[k] - px, y[k] - py)
        near = d < thr
        k, d = k[near], d[near]
    else:
        d = np.where(ok, np.hypot(x - px, y - py), np.inf)
        k = np.flatnonzero(d < thr)
        d = d[k]
        hidden = seen is not None or None
    if hidden and len(k):
        keep = ~DocumentInteraction._depth_hidden(seen, pts[k], x[k], y[k])
        k, d = k[keep], d[keep]
    return k, d


def _nearest_within(d, radius, cap):
    """The indices of ``d`` under ``radius``: all of them in order, or past
    ``cap`` the nearest ``cap`` by distance and then index. The native
    engine picks those in one pass (475 000 fixed group edges lay within
    the radius over the plant of #158: ~4 ms of NumPy, twice a move); the
    same ones, by distance and index, whose order a stable sort by
    distance keeps."""
    import numpy as np
    from core import _native
    mod = _native.native
    if (_NEAREST_NATIVE and cap > 0 and len(d) > 4096 and mod is not None
            and hasattr(mod, "nearest_within")):
        return np.asarray(mod.nearest_within(
            np.ascontiguousarray(d, dtype=np.float64), float(radius),
            int(cap)), dtype=np.int64)
    cand = np.flatnonzero(d < radius)
    if len(cand) > cap:
        dc = d[cand]
        if len(cand) > 4 * cap:
            part = np.argpartition(dc, cap - 1)[:cap]
            cand, dc = cand[part], dc[part]
        cand = cand[np.lexsort((cand, dc))[:cap]]
    return cand


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
    """The three-argument ``std::hypot`` in float of the C++ library
    ``kind``, row by row of ``a`` (N, 3) float32, as Qt's
    ``QVector3D::length`` computes it: libstdc++ scales the largest
    magnitude out; MSVC's STL (Qt on Windows) swaps it to the front, keeps
    it when the other two are below its epsilon and adds the other two
    ratios' squares to 1 in the order the swaps left them."""
    import numpy as np
    a = np.abs(a)
    if kind == "msvc":
        x, y, z = a[:, 0], a[:, 1], a[:, 2]
        s = y > x
        x, y = np.where(s, y, x), np.where(s, x, y)
        s = z > x
        x, z = np.where(s, z, x), np.where(s, x, z)
        e = x * np.float32(np.finfo(np.float32).eps)
        with np.errstate(divide="ignore", invalid="ignore", over="ignore"):
            p, q = y / x, z / x
            r = x * np.sqrt((np.float32(1) + p * p) + q * q)
        r = np.where((e >= y) & (e >= z), x, r)
        return np.where(np.isinf(a).any(axis=1), np.float32(np.inf),
                        r).astype(np.float32)
    m = a.max(axis=1)
    with np.errstate(divide="ignore", invalid="ignore"):
        q = a / m[:, None]
        s = (q[:, 0] * q[:, 0] + q[:, 1] * q[:, 1]) + q[:, 2] * q[:, 2]
        return np.where(m > 0, m * np.sqrt(s), np.float32(0)).astype(np.float32)


def _qv_hypot3():
    return _QV_HYPOT3


def _unit_rays(eye, pts):
    return ingetrazo_core.document_unit_rays(eye, pts)


# Barycentric roundoff on a shared triangle edge can reject both faces.
_BARY_EPS = 1e-14


def _occlusion_block(o, d, v0, e1, e2):
    return ingetrazo_core.occlusion_block(o, d, v0, e1, e2)


def _placed_maps(xf):
    """``(L, tv, tp)`` of a placement matrix (QMatrix4x4): its linear part
    and the maps a component bake puts local directions and positions
    through. Term by term, not ``a @ L.T``: a BLAS product sums in an order
    of its own (fused, blocked), and the native engine must find the very
    same world triangles and corners without baking them (#158) —
    ((x·L0 + y·L1) + z·L2) + t, as core/instances.rs writes it."""
    import numpy as np
    m = np.array(xf.data(), dtype=np.float64).reshape(4, 4, order="F")
    L, t = m[:3, :3], m[:3, 3]

    def tv(a):                           # direction vectors (no translation)
        return (a[:, 0:1] * L[:, 0] + a[:, 1:2] * L[:, 1]
                + a[:, 2:3] * L[:, 2])

    def tp(a):                           # positions
        return tv(a) + t
    return L, tv, tp


def _batch_instances(base, xfs, tri_parts, hard_parts, soft_parts,
                     n_parts) -> None:
    return ingetrazo_render.batch_hlr_instances(base, xfs, tri_parts, hard_parts, soft_parts, n_parts)


def _cat_kept(vp, slot, parts):
    return ingetrazo_render.cat_kept(vp, slot, parts)


def _offset_kept(vp, te, offset):
    return ingetrazo_render.offset_kept(vp, te, offset)


def _moller_trumbore(o, d, v0, e1, e2):
    """Möller–Trumbore of the ray ``o`` + t·``d`` against triangles (v0, e1,
    e2) (N, 3): ``(t, hit)``, with the thresholds of :func:`_ray_triangle`
    (``ingetrazo_core.ray_triangles``, rust/ingetrazo_core/src/pick.rs)."""
    import numpy as np
    from ingetrazo_core import ray_triangles
    as_rows = lambda a: np.ascontiguousarray(a, dtype=np.float64).reshape(-1, 3)
    o, d = np.asarray(o, dtype=np.float64), np.asarray(d, dtype=np.float64)
    return ray_triangles(tuple(map(float, o)), tuple(map(float, d)),
                         as_rows(v0), as_rows(e1), as_rows(e2))


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

def edit_fade_color(background) -> tuple:
    """What the rest of the model fades TOWARDS while a group is edited: a
    grey half way between the background and a neutral mid grey. Fading
    to the background itself left white faces on a light background
    looking the same as before — nothing said you were inside a group
    («quiero que lo demás quede gris», 14-10). Towards this grey the rest
    reads grey on a light background and dim on a dark one, and what is
    being edited keeps its colours."""
    return tuple(0.5 * float(c) + 0.5 * 0.55 for c in list(background)[:3])

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


def selection_dot_color(surface) -> tuple:
    """The dot colour that reads on a face side painted ``surface`` (an RGB
    triple, or ``None`` when unknown, e.g. a texture): the selection orange,
    unless the surface is orange-ish or red-ish itself (hue within ~50° of
    red, with enough saturation and value to look coloured), where the dots
    turn blue so a selected face still tells itself apart."""
    if surface is None:
        return SELECTION_DOT_COLOR
    import colorsys
    h, s, v = colorsys.rgb_to_hsv(*(float(c) for c in surface[:3]))
    if s >= 0.35 and v >= 0.35 and (h <= 0.14 or h >= 0.96):
        return SELECTION_DOT_ALT_COLOR
    return SELECTION_DOT_COLOR


def _box_edges(frame, lo, hi) -> bytes:
    """The twelve segments of a box given in ``frame``'s axes, as an
    interleaved float32 line buffer — the selection cue for a whole group."""
    from core.group import oriented_box_corners
    corners = oriented_box_corners(frame, lo, hi)
    buf = array("f")
    for i in range(8):
        for bit in (1, 2, 4):      # neighbours differ in exactly one axis
            j = i | bit
            if j != i:
                for p in (corners[i], corners[j]):
                    buf.extend((p.x(), p.y(), p.z()))
    return buf.tobytes()


def _load_edit_rest_mode() -> str:
    from core.settings import settings as QSettings
    mode = str(QSettings().value("display/edit_rest_mode", "fade"))
    return mode if mode in EDIT_REST_MODES else "fade"


def _load_edit_rest_fade() -> float:
    """Saved strength of the grey edit context, safely clamped to 0..1."""
    from core.settings import settings as QSettings
    try:
        value = float(QSettings().value("display/edit_rest_fade",
                                        EDIT_REST_FADE))
    except (TypeError, ValueError):
        value = EDIT_REST_FADE
    return max(0.0, min(1.0, value))


def _load_hide_similar() -> bool:
    from core.settings import settings as QSettings
    return str(QSettings().value("display/hide_similar_components",
                                 "0")) == "1"


def _load_invert_wheel() -> bool:
    from core.settings import settings as QSettings
    return str(QSettings().value("nav/invert_wheel", "0")) != "0"


def _load_invert_orbit_y() -> bool:
    """Preferences ▸ orbit with the vertical axis reversed.

    Off by default: `OrbitCamera.orbit` grabs the model in both axes, the
    classic way. This is for the hands that learned the other feel —
    including the ones that learned it from IngeTrazo before 0.3.16, when
    the vertical axis was inverted by mistake.
    """
    from core.settings import settings as QSettings
    return str(QSettings().value("nav/invert_orbit_y", "0")) != "0"


#: Soft sun shadows (#1281), Preferences ▸ General: off (0) or a level, and
#: how fast the penumbra widens with the distance from what casts it, in
#: world width per world distance. The real sun is about 0.009; the levels
#: are broader, as a picture of soft daylight asks.
SOFT_SHADOW_SPREAD = {1: 0.012, 2: 0.03, 3: 0.06}


def _load_soft_shadows() -> int:
    """The soft shadows level (Preferences ▸ General): 0 = off, the
    default, since it costs the graphics card more while the sun is on."""
    from core.settings import settings as QSettings
    try:
        n = int(QSettings().value("display/soft_shadows", 0))
    except (TypeError, ValueError):
        n = 0
    return n if n in SOFT_SHADOW_SPREAD else 0


_AXIS_DIRS = {"x": (1.0, 0.0, 0.0), "y": (0.0, 1.0, 0.0), "z": (0.0, 0.0, 1.0)}


def _axes_vertices(spacing: float, pos_len: float = 1.0e5):
    """Drawing axes: a long solid line in the positive direction and an
    **evenly-spaced** dashed line in the negative (constant world ``spacing``, so
    the dashes converge toward the horizon by perspective — the classic look, not
    spreading apart). ``spacing`` scales with the camera distance so the on-screen
    density stays stable across zoom. Returns ``(coords, spans)`` where ``spans``
    maps ``'x'|'y'|'z'`` → ``(first_vertex, vertex_count)`` for a per-axis draw."""
    coords = array("f")
    spans: dict[str, tuple[int, int]] = {}
    spacing = max(spacing, 1e-4)
    # Short dots, not half-duty dashes: GL lines are stuck at 1px (Mesa
    # clamps glLineWidth), so the dash LENGTH is what sets the perceived
    # weight — the negative axes should read as fine dotted lines.
    dash = spacing * 0.18
    n = 520                          # dashes → reach = spacing*n past the model
    # The DRAWING axes (core.axes): the world's at the top level, the open
    # group's own — at its origin — inside it (issue #44).
    from core import axes as _axes
    o = _axes.ORIGIN
    ox, oy, oz = o.x(), o.y(), o.z()
    for name in ("x", "y", "z"):
        if _axes.is_world():
            dx, dy, dz = _AXIS_DIRS[name]
        else:
            dx, dy, dz = _axes.as_tuple(name)
        start = len(coords) // 3
        coords.extend([ox, oy, oz, ox + dx * pos_len, oy + dy * pos_len,
                       oz + dz * pos_len])
        for k in range(n):
            t0 = k * spacing
            t1 = t0 + dash
            coords.extend([ox - dx * t0, oy - dy * t0, oz - dz * t0,
                           ox - dx * t1, oy - dy * t1, oz - dz * t1])
        spans[name] = (start, len(coords) // 3 - start)
    return coords, spans


def _box_edge_vertices(corners):
    from ingetrazo_core import box_edge_vertices
    coords = array("f")
    coords.frombytes(memoryview(box_edge_vertices(corners)).cast("B"))
    return coords


def shown_guides(scene) -> list:
    """The guides on screen: the ones whose layer is visible (a guide is
    scaffolding on a layer like any dimension, so switching the layer off
    takes its guides off the screen and out of the snap). A function of the
    scene, so the snap of a stand-in viewport (tests) asks it too."""
    guides = getattr(scene, "guides", None) or ()
    visible = getattr(scene, "entity_visible", None)
    if visible is None:
        return list(guides)
    return [g for g in guides if visible(g)]


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
    """Möller–Trumbore ray / triangle intersection. Returns distance ``t``
    along the ray, or ``None`` for a miss / behind-camera hit. The triangle
    is intersected from both sides — front/back orientation does not matter
    because IngeTrazo doesn't (yet) cull back faces."""
    eps = 1e-6
    e1 = v1 - v0
    e2 = v2 - v0
    h = QVector3D.crossProduct(direction, e2)
    a = QVector3D.dotProduct(e1, h)
    # The same reading as _moller_trumbore: a small det of a small triangle
    # is measured against its own size, not against 1e-6 (#1294).
    if abs(a) < eps and (a == 0.0 or abs(a) <= eps * QVector3D.crossProduct(
            e1, e2).length() * direction.length()):
        return None
    f = 1.0 / a
    s = origin - v0
    u = f * QVector3D.dotProduct(s, h)
    if u < -_BARY_EPS or u > 1.0 + _BARY_EPS:
        return None
    q = QVector3D.crossProduct(s, e1)
    v = f * QVector3D.dotProduct(direction, q)
    if v < -_BARY_EPS or u + v > 1.0 + _BARY_EPS:
        return None
    t = f * QVector3D.dotProduct(e2, q)
    if t < eps:
        return None
    return t


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
        t = ev.type()
        if t == QEvent.MouseButtonPress:
            # Alt+click is a sample of its own, not a tap.
            self._clean = False
            return False
        if t != QEvent.KeyPress and t != QEvent.KeyRelease:
            return False
        if ev.isAutoRepeat():
            return False
        if ev.key() != Qt.Key_Alt:
            if t == QEvent.KeyPress:
                self._clean = False           # Alt+Tab, Alt+F… is no tap
            return False
        down = t == QEvent.KeyPress
        tap = not down and self._clean
        self._clean = down
        for vp in list(self._viewports):
            try:
                if down != vp._alt_down:
                    vp._apply_alt_cursor(down)
                if tap and vp._alt_tapped():
                    tap = False       # the eyedropper state is shared: once
            except RuntimeError:          # the C++ side already went
                pass
        return False


def _face_geometry(f, memo, triangulate):
    return ingetrazo_render.face_geometry(f, memo, triangulate)


def _finite_point(p) -> bool:
    """Whether ``p`` (a QVector3D or None) is a real point."""
    import math
    return p is not None and all(
        math.isfinite(c) for c in (p.x(), p.y(), p.z()))


def _curved_faces(mesh) -> set:
    return ingetrazo_render.curved_faces(mesh)


def _smooth_vcol(face, base, flat_rgb, smoother, tris, light) -> list:
    return ingetrazo_render.smooth_vcol(face, base, flat_rgb, smoother, tris, light)


from views.document_tools import DocumentTools
from views.view_navigation import ViewNavigation


def _publish_scene_version(viewport, version):
    watch = viewport.__dict__.get("_document_watch")
    if watch is None:
        viewport.sceneVersionChanged.emit(version)
    else:
        watch.observe()


class DocumentInteraction(DocumentTools, ViewNavigation):
    """Editing and input shared by the document presentation hosts."""

    def _render_framed_image(self, width_px, overlays, annotations_only,
                             sharp_shadows, transparent, *, raster=False):
        render = self.render_raster if raster else self.render_image
        lw, lh = max(self.width(), 1), max(self.height(), 1)
        from core.camera import frame_rect
        rect = frame_rect(lw, lh, getattr(self.camera, "frame_aspect",
                                           None))
        if rect is not None and (rect[2] < lw - 0.5
                                 or rect[3] < lh - 0.5):
            fx, fy, fw, fh = rect
            aspect = self.camera.frame_aspect
            out_w = max(int(width_px), 64)
            out_h = max(int(round(out_w / aspect)), 1)
            full = render(
                max(int(round(out_w * lw / fw)), 64), None, overlays,
                annotations_only, sharp_shadows, transparent, False)
            if full is None:
                return None
            if raster:
                return full.frame_region((fx / lw, fy / lh, fw / lw, fh / lh), out_w, out_h)
            sx = full.width() / lw
            sy = full.height() / lh
            x = int(round(fx * sx))
            y = int(round(fy * sy))
            w = min(int(round(fw * sx)), full.width() - x)
            h = min(int(round(fh * sy)), full.height() - y)
            image = full.copy(x, y, w, h)
            if (image.width(), image.height()) != (out_w, out_h):
                image = image.scaled(out_w, out_h, Qt.IgnoreAspectRatio,
                                     Qt.SmoothTransformation)
            return image
        return render(width_px, None, overlays, annotations_only,
                                 sharp_shadows, transparent, False)

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

        # Linear-inference toggle (Alt): "all" | "off" | "parallel_perp".
        self.linear_inference_mode = "all"
        #: A bare Alt is down and nothing else has happened since — the tap
        #: that cycles the mode on release (issue #26).
        self._alt_tap = False
        self._alt_down = False
        if not NATIVE:
            _AltWatch.watch(self)
        self._last_mouse_pos: Optional[QPointF] = None

        # Pixel radius for point snaps (endpoint, origin, close). 12 px felt
        # mushy when the cursor was running along an existing edge: as long
        # as the cursor was within 12 px of either end of a short edge,
        # endpoint snap kept firing. It must be tighter — the green dot
        # only lights up right at the vertex.
        self.snap_threshold_px = 9.0
        # On-edge snap gets a bigger radius than point snaps: an edge is a large
        # linear target, so resting a corner on it (e.g. a door on the floor
        # line) should be forgiving and not slip just outside the face.
        self.edge_snap_threshold_px = 14.0
        self.pick_threshold_px = 8.0
        self.inference_angle_deg = 3.0

        # How the rest of the model reads while you are INSIDE a group
        # (Model Info ▸ Components). "fade" keeps the context
        # visible but out of the way, "hide" drops it from the frame
        # entirely — on a heavy import that is also the fastest, since a
        # hidden group never reaches the VBOs. See `edit_rest_mode`.
        self._edit_rest_mode = _load_edit_rest_mode()
        # Keep the user setting distinct from ``_edit_rest_fade(group)``,
        # which computes the effective fade for image and billboard passes.
        self._edit_rest_fade_strength = _load_edit_rest_fade()
        self.scene.hide_similar_components = _load_hide_similar()
        self._invert_wheel = _load_invert_wheel()
        self._invert_orbit_y = _load_invert_orbit_y()
        from ingetrazo_render import trackpad_nav
        self._trackpad_slide = trackpad_nav.load_slide_mode()
        self._msaa = 4
        self._soft_shadows = _load_soft_shadows()

        # Hover highlight (Select tool). Not version-tracked — it changes with
        # the cursor, not with scene mutations — so it's uploaded per paint.
        self._hover_entity = None  # None | Edge | Face under the cursor
        #: The placement a hovered face is reached THROUGH (a part inside a
        #: component): its matrix puts the face's own coordinates in the
        #: world. ``None`` for a loose face, which is already there.
        self._hover_placement = None
        self._last_double = None   # (timestamp, pos) of the last double-click
        # Faces hidden from the normal pass while a tool previews — Push/Pull
        # hides the flat inner face it's pushing in (a window/door) so the recess
        # forming behind it is visible instead of covered. Keyed by identity.
        self._suppressed_faces: ElementSet = ElementSet()

        # When set, paintGL renders at this framebuffer size into the scene
        # FBO and stops there (no blit, no widget overlay) — the hi-res image
        # export path (render_image) reads the FBO back instead.
        self._export_size: Optional[tuple[int, int]] = None
        #: Pen widths in PIXELS for the next export render (edges and
        #: profiles); 1 = the plain GL hairline. See ``_line_jitter``.
        self._export_edge_px = 1
        self._export_profile_px = 1
        # Sheet-composer technical style: None (shaded), "tecnico" (white
        # faces + dark edges on white, no axes/sky) or "lineas" (edges only).
        # Set around a composer render; never persists across frames.
        self.plano_style: Optional[str] = None
        # Full Style override for a composer frame ("style:<name>"): wins
        # over the scene's display style for that render; axes are skipped
        # (a frame is a document, not the workspace). Never persists.
        self.style_override = None
        # This view's own Style when the window shows several views and the
        # style was changed in this one (ingetrazo_render.DocumentViews); None: the
        # scene's, which the window's own view always draws with.
        self.view_style = None
        self._fbo_size = (0, 0)

        # Camera navigation state (middle button)
        self._last_pos = None
        self._pan_mode = False
        self._middle_drag_active = False
        #: where the current orbit gesture turns (#164); None = the target
        self._orbit_pivot = None
        # A mouse-look drag for a tool with ``on_look`` (First Person):
        # (button, last local point) while a button is held, else None.
        self._look_drag = None
        # Navigation mode for trackpad users with no middle
        # mouse button: when set ("orbit" / "pan"), a left-drag drives the
        # camera instead of the active tool. None means a drawing tool is in
        # charge of the left button.
        self.nav_mode: Optional[str] = None

        # Preview line for the "always on top" tools, stashed during GL render
        # and drawn in the QPainter overlay (thick, reliable pen).
        self._overlay_rubber: Optional[tuple] = None

        # Rubber-band box selection (left-drag with a box_select tool).
        self._box_active = False
        self._box_start: Optional[QPointF] = None
        self._box_cur: Optional[QPointF] = None

        # Zoom Window rubber-band (nav_mode == "zoom_window").
        self._zoom_box_active = False
        self._zoom_box_start: Optional[QPointF] = None
        self._zoom_box_cur: Optional[QPointF] = None

        # Numeric value buffer (VCB-style typed length).
        self._value_buffer = ""

        # Base-map tiles (Track G, G1). The fetcher is created lazily (needs a
        # running app); GL textures are cached per tile, keyed by (source, x,y,z).
        self._tile_fetcher = None
        # A paint must only draw.  In particular, creating Qt's native HTTPS
        # backend while an OpenGL context is current crashed in ntdll on an
        # AMD Windows driver (#327).  Missing tiles schedule one event-loop
        # callback; that callback starts the network work after paintGL has
        # returned and the context is no longer inside a driver callback.
        self._tile_requests_scheduled = False
        self._last_coordinate = ""      # so the label only repaints on change
        # Georef path node being hovered ``(path, index)`` — for the drag handle
        # highlight (Track G, GeoPath node editing).
        self._hover_geo_node = None
        # World point on the profiled route to mark (profile→plan link), or None.
        self._route_marker = None

        # 3D draped terrain (Track G, G2 full).

    def _fb_size(self) -> tuple[int, int]:
        """Framebuffer pixel size (logical size × device pixel ratio)."""
        dpr = self.devicePixelRatioF()
        return max(int(round(self.width() * dpr)), 1), max(int(round(self.height() * dpr)), 1)

    def _sync_axes(self) -> None:
        """Make the drawing axes the current context's (issue #44) — the
        world's at the top level, the open group's own inside it."""
        from core import axes as _axes
        _axes.sync(getattr(self.scene, "drawing_frame", None))

    def _sync_projection(self) -> None:
        """Tell the quick buttons when the projection changed. Scenes,
        saved views, the geo path and the composer all set the camera's
        fields directly, so the check runs on every paint: whatever set
        it, the next frame shows it and the buttons follow."""
        mode = self.camera.projection_mode()
        if mode != getattr(self, "_shown_projection", None):
            self._shown_projection = mode
            self.projectionChanged.emit(mode)

    def _sync_depth_reach(self) -> None:
        """Tell the camera what the view holds, for the depth range of a
        parallel view (``OrbitCamera.parallel_depth``, issue #874): the
        model's box, the reference images and figures, and the drawing
        axes' origin. ``None`` — the old ±far range — while a base map or
        a survey shows, whose extent the model's box does not know."""
        cam = self.camera
        if cam.perspective:
            return                       # perspective does not read it
        sc = self.scene
        layer = getattr(sc, "tile_layer", None)
        survey = getattr(sc, "photo_mesh", None)
        if (layer is not None and getattr(layer, "visible", False)) \
                or (survey is not None and getattr(survey, "visible", False)):
            cam.depth_reach = None
            return
        from core import axes as _axes
        lo, hi = sc.bounds()
        pts = [_axes.ORIGIN]
        if lo is not None:
            pts += [lo, hi]
        for im in getattr(sc, "image_planes", None) or ():
            if sc.entity_visible(im):
                pts.extend(im.corners())
        for g in sc.groups:
            if getattr(g, "billboard", False) and sc.entity_visible(g):
                xf = g.xform
                pts.append(QVector3D(xf.column(3).x(), xf.column(3).y(),
                                     xf.column(3).z())
                           if xf is not None else QVector3D())
                pts.append(pts[-1] + QVector3D(0.0, 0.0, 3.0))
        xs = [p.x() for p in pts]
        ys = [p.y() for p in pts]
        zs = [p.z() for p in pts]
        a = QVector3D(min(xs), min(ys), min(zs))
        b = QVector3D(max(xs), max(ys), max(zs))
        cam.depth_reach = ((a + b) * 0.5, (b - a).length() * 0.5)

    def _navigation_detail(self, mvp, mode):
        """Defer edge linework during camera movement in dense assemblies.

        Keep faces/materials intact and repaint at rest even if no mouse event
        follows. Exports, line styles and editing contexts retain full detail.
        """
        if self._export_size is not None:
            return False
        cam = tuple(mvp.data())
        previous = getattr(self, "_nav_detail_camera", None)
        self._nav_detail_camera = cam
        pool = getattr(self, "_inst_pool", None)
        eligible = (pool is not None and len(pool[1]) >= 2000
                    and mode in ("shaded", "textures", "monochrome")
                    and self.scene.edit_group is None
                    and not self.scene.selection)
        if not eligible:
            self._nav_detail_until = 0.0
            return False
        now = _time_mod.monotonic()
        if previous is not None and previous != cam:
            self._nav_detail_until = now + 0.35
            timer = getattr(self, "_nav_detail_timer", None)
            if timer is None:
                timer = self._nav_detail_timer = window_timer(self, self.update)
            timer.start(370)
        if now < getattr(self, "_nav_detail_until", 0.0):
            return True
        return False


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

    def _depth_fit_refused(self) -> bool:
        """Whether this frame keeps the automatic near/far: something
        drawn may lie outside the model's boxes, or the boxes are not
        known (shared by the Metal view, whose camera this is)."""
        sc = self.scene
        if (not self.camera.perspective or sc.edit_group is not None
                or getattr(self, "_preview_groups", None)
                or getattr(self, "_frozen_cache_version", None) is not None
                or self._base_map_showing() or self._terrain_showing()
                or self._photo_showing()
                or getattr(sc, "guides", None)
                or getattr(getattr(sc, "shadows", None), "enabled", False)
                or getattr(sc, "show_hidden_objects", False)
                or getattr(sc, "show_hidden_geometry", False)):
            return True
        # A tool's live preview (Push/Pull's box, a pasted copy, Follow
        # Me) can reach toward the eye past the model's nearest box.
        tool = getattr(self, "active_tool", None)
        provider = getattr(tool, "preview_faces", None) if tool else None
        try:
            if callable(provider) and provider():
                return True
        except Exception:
            return True
        return bool(getattr(tool, "wireframe_depth_tested", False))

    def _fit_scene_depth(self):
        """Fit the perspective near/far to the visible placements' boxes
        (``OrbitCamera.fit_depth_range``) for this frame. The boxes are
        walked once per content and placement epoch; a refusal (a box not
        known) is cached too, so the walk is not repeated per frame."""
        import numpy as np
        self.camera._depth_range = None
        if self._depth_fit_refused():
            return
        key = (self._placements_epoch(), _content_ver(self), id(self.scene.mesh))
        memo = getattr(self, "_depth_boxes", None)
        if memo is None or memo[0] != key:
            memo = self._depth_boxes = (key, *self._depth_fit_boxes())
        if memo[1] is not None:
            self.camera.fit_depth_range(memo[1], memo[2])

    def _depth_fit_boxes(self):
        """``(lo, hi)`` arrays of world boxes covering everything drawn,
        or ``(None, None)`` when one placement's box is not known."""
        import numpy as np
        from core.lazy_mesh import entity_count
        boxes = []
        for g in self._placements():
            if not self.scene.entity_visible(g):
                continue
            if getattr(g, "billboard", False):
                verts = self._billboard_mesh(g).vertices
                if verts:
                    a = np.asarray([v.position.toTuple() for v in verts])
                    lo, hi = a.min(axis=0), a.max(axis=0)
                    # Face-me figures turn about their vertical axis.
                    ctr = (lo + hi) * .5
                    radius = np.linalg.norm(hi[:2] - lo[:2]) * .5
                    boxes.append(((ctr[0] - radius, ctr[1] - radius, lo[2]),
                                  (ctr[0] + radius, ctr[1] + radius, hi[2])))
                continue
            box = (self._placement_bbox(g) if g.xform is not None
                   else self._group_chunk(g).get("bbox"))
            if box is None:
                if entity_count(g.mesh) == 0:
                    continue            # an empty container draws nothing
                return None, None
            boxes.append(box)
        # Loose geometry has no placement box. A still-deferred mesh is
        # read from its plan, not built.
        from core.lazy_mesh import plan_of
        planned = plan_of(self.scene.mesh)
        a = (planned.pos if planned is not None else
             np.asarray([v.position.toTuple() for v in self.scene.mesh.vertices]))
        if len(a):
            a = np.asarray(a, float).reshape(-1, 3)
            boxes.append((a.min(axis=0), a.max(axis=0)))
        for im in getattr(self.scene, "image_planes", ()) or ():
            if self.scene.entity_visible(im):
                a = np.asarray([p.toTuple() for p in im.corners()])
                boxes.append((a.min(axis=0), a.max(axis=0)))
        if not boxes:
            return None, None
        a = np.asarray(boxes, dtype=float).reshape(-1, 2, 3)
        return a[:, 0], a[:, 1]





    # ---- Instanced components (P2) ------------------------------------------
    # Component instances draw from ONE static proto VBO (local coords) plus
    # a per-instance matrix buffer (divisor 1): N hedges cost one upload and
    # one draw per proto run. Instances keep their baked chunks for picks,
    # silhouettes, selection and the drag preview; only the CONSOLIDATED
    # draw excludes them. INGETRAZO_NO_INSTANCING=1 reverts to the old path.

    def _instanced_eligible(self, g) -> bool:
        return ingetrazo_render.instanced_eligible(self, g)

    def _proto_base_chunk(self, mesh, paint=None):
        """The prototype's chunk in LOCAL coordinates (the same wrapper
        ``_instance_chunk`` derives from). ``paint`` is the container
        material the variant is baked in (issue #47): a painted instance
        has its own bake of the prototype, unpainted siblings share the
        plain one."""
        return self._group_chunk(_proto_wrapper(self, mesh, paint))

    def _placements_epoch(self):
        return ingetrazo_render.placements_epoch(self)

    #: How many past epochs keep their parts for ``_epoch_only_matrices``.
    _EPOCH_HISTORY = 4

    def _epoch_remember(self, epoch, head, parts, tops=()) -> None:
        return ingetrazo_render.epoch_remember(self, epoch, head, parts, tops)

    def _epoch_only_matrices(self, old, new=None) -> bool:
        return ingetrazo_render.epoch_only_matrices(self, old, new)

    def _epoch_kept_tops(self, old, new=None, ctx=False):
        return ingetrazo_render.epoch_kept_tops(self, old, new, ctx)

    def _refilter(self, old_epoch, old_out, source, test, old_source=None,
                  ctx=False):
        return ingetrazo_render.refilter(self, old_epoch, old_out, source, test, old_source, ctx)

    def _placement_top(self, g):
        return ingetrazo_render.placement_top(self, g)

    def _placements(self):
        return ingetrazo_render.placements(self)

    def _expand_placements(self, group, out=None, seen=None, above=None):
        return ingetrazo_render.expand_placements(self, group, out, seen, above)

    def _bind_placement_rows(self, handles, parents, out, seen=None, above=None):
        return ingetrazo_render.bind_placement_rows(self, handles, parents, out, seen, above)

    def prepare_drawing(self, progress=None) -> None:
        return ingetrazo_render.prepare_drawing(self, progress)

    def _placement_bbox(self, g):
        """World AABB of an instanced placement from its PROTOTYPE's local
        box and the placement matrix — eight corners, no bake. Reading it
        off ``_group_chunk(g)`` baked the whole placement to world
        coordinates only to take its box: on the model of issue #158 that
        was 21 000 baked copies of geometry the instanced pass never uses."""
        return self._placement_frame(g)[0]

    def _placement_frame(self, g):
        return ingetrazo_render.placement_frame(self, g)

    #: How far past the view the instanced batches reach, as a fraction of
    #: its width and height, so an orbit keeps drawing the same ones
    #: (_gather_instanced).
    _INST_VIEW_MARGIN = 0.10
    #: ... and while they hold at most this many times what is in view.
    _INST_SUPERSET_MAX = 2

    @staticmethod
    def _widened_planes(planes, frac):
        from ingetrazo_core import widened_planes
        return widened_planes(planes, frac)

    @staticmethod
    def _instances_in_frustum(lo, hi, planes, margin=0.0):
        from ingetrazo_core import instances_in_frustum
        return instances_in_frustum(lo, hi, planes, margin)

    #: An instance whose bounding sphere spans less than this many
    #: framebuffer pixels is not drawn (the camera's frame only).
    _INST_MIN_PX = 1.0

    def _instances_below_a_pixel(self, pool, scale=1.0):
        """Mask of the pool's placements whose bounding sphere comes out
        under ``_INST_MIN_PX`` (× ``scale``) framebuffer pixels across on
        this frame.

        From the whole plant of #158, 70 % of the instanced faces belonged
        to placements under a pixel — bolts and fittings a hundred metres
        off — and the GPU spent two thirds of every orbit frame on them.
        The sphere is the box's (its half diagonal), so the test only ever
        errs toward drawing; a box that reaches the eye plane always draws."""
        import numpy as np
        memo = getattr(self, "_inst_sphere_memo", None)
        if memo is None or memo[0] is not pool:
            memo = self._inst_sphere_memo = (
                pool, *self._bounding_spheres(pool[2], pool[3]))
        return self._spheres_below_a_pixel(memo[1], memo[2],
                                           self._INST_MIN_PX * scale)

    @staticmethod
    def _bounding_spheres(lo, hi):
        from ingetrazo_core import bounding_spheres
        return bounding_spheres(lo, hi)

    def _spheres_below_a_pixel(self, ctr, rad, floor=None):
        from ingetrazo_core import small_spheres
        height = (self._export_size or self._fb_size())[1]
        scale = abs(self.camera.projection_matrix().data()[5]) * height
        return small_spheres(ctr, rad, self._np_mvp()[3], scale, self.camera.perspective,
                             self._INST_MIN_PX if floor is None else floor)

    def _edit_rest_fade(self, group):
        """How a reference image (``group=None``) or a face-me figure is
        drawn while a group is open: the fade to apply, or ``None`` to skip
        it. These passes bypass the buffers that carry the edit split, so
        the people and images kept their colours on a grey model."""
        if self.scene.edit_group is None:
            return 0.0
        if group is not None and not self._draws_in_edit_context(group):
            return 0.0
        if self._edit_rest_mode == "hide":
            return None
        return self.edit_rest_fade if self._edit_rest_mode == "fade" else 0.0

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

    def _draw_image_outlines(self, painter: QPainter) -> None:
        """Border of each reference image: faint normally, selection orange
        when picked, so an image lying flat on the ground is still findable.
        A LOCKED image shows it only while selected: it is a backdrop that
        refuses clicks, and a scan's rectangle runs past the photo over its
        transparent corners, so the faint dashes cut across the sky (Marco,
        2026-10-27, the orthomosaic of playa Yanque)."""
        images = getattr(self.scene, "image_planes", None)
        if not images:
            return
        pen = QPen(QColor(120, 140, 165), 1, Qt.DashLine)
        sel_pen = QPen(QColor(243, 115, 41), 2)      # selection orange
        selection = self.scene.selection
        for im in images:
            if not self.scene.entity_visible(im):
                continue
            picked = im in selection
            if getattr(im, "locked", False) and not picked:
                continue
            painter.setPen(sel_pen if picked else pen)
            for a, b in im.border_edges():
                seg = self._clip_segment_front(a, b)
                if seg is None:
                    continue
                pa = self._world_to_pixel(seg[0])
                pb = self._world_to_pixel(seg[1])
                if pa is not None and pb is not None:
                    painter.drawLine(QPointF(*pa), QPointF(*pb))

    def finish_image_placement(self) -> None:
        """Called by the Image tool once a picture is placed: drop back to
        Select, the classic behaviour after a one-shot placement."""
        win = self.window()
        if hasattr(win, "activate_select_tool"):
            win.activate_select_tool()

    def _selection_dot_colors(self, face) -> tuple:
        """``(front_dot, back_dot)``: the selection dot colour for each side
        of ``face``, each picked against the paint that side shows — its
        material colour (or the open group's paint), the two-sided back
        paint, or the style's back-face tint."""
        from core.materials import effective_attrs
        attrs = effective_attrs(face.attrs,
                                _paint_of(self, self.scene.edit_group))
        tex = attrs.get("texture")
        if tex is not None and tex.get("path"):
            front = None                      # a texture: colour unknown
        else:
            front = tuple(attrs.get("color") or self.DEFAULT_FACE_COLOR)
        back_attr = attrs.get("back")
        if back_attr is True:
            back = front                      # both sides wear the front
        elif isinstance(back_attr, dict):
            col = back_attr.get("color")
            back = tuple(col) if col is not None else None
        else:
            back = effective_back_color(
                self._view_style(), self.scene)
        return selection_dot_color(front), selection_dot_color(back)


    def _shade_factor(self, normal) -> float:
        return ingetrazo_render.shade_factor(self, normal)

    def _shaded_color(self, base, normal):
        return ingetrazo_render.shaded_color(self, base, normal)

    # ---- Line weight for exports --------------------------------------------
    def _line_jitter(self, px: int, w: int, h: int) -> list:
        """NDC offsets that thicken a 1-px GL line to about ``px`` pixels by
        redrawing it shifted — a disc of sub-pixel samples (2 px = the four
        half-pixel corners, 3 px = a cross, …). Only export renders ask for
        more than one; on screen the list is the single origin."""
        if px <= 1 or self._export_size is None:
            return [(0.0, 0.0)]
        px = min(int(px), 8)
        r = (px - 1) / 2.0
        offs = [-r + i for i in range(px)]
        return [(dx * 2.0 / max(w, 1), dy * 2.0 / max(h, 1))
                for dx in offs for dy in offs
                if dx * dx + dy * dy <= r * r + 0.51]

    # ---- Base-map tiles (Track G) -------------------------------------------
    def _base_map_showing(self) -> bool:
        return ingetrazo_render.base_map_showing(self)

    def _ensure_tile_fetcher(self):
        return ingetrazo_render.ensure_tile_fetcher(self)

    def _on_tile_ready(self, source_id, x, y, z, image) -> None:
        return ingetrazo_render.on_tile_ready(self, source_id, x, y, z, image)

    def _disk_cached_tile(self, source, x, y, z):
        return ingetrazo_render.disk_cached_tile(self, source, x, y, z)

    def _schedule_tile_requests(self) -> None:
        return ingetrazo_render.schedule_tile_requests(self)

    def _request_missing_tiles(self) -> None:
        return ingetrazo_render.request_missing_tiles(self)

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


    def _terrain_showing(self) -> bool:
        return ingetrazo_render.terrain_showing(self)

    def prefetch_tiles(self, source, tile_list, zoom) -> None:
        return ingetrazo_render.prefetch_tiles(self, source, tile_list, zoom)



    # ---- Photogrammetric survey (Track G, G6) ------------------------------








    # Sky (top) and ground (bottom) backdrop colours — subtle two-tone.
    _SKY_RGB = (0.925, 0.935, 0.945)
    _GROUND_RGB = (0.815, 0.820, 0.815)

    def _horizon_ndc_y(self, mvp, camera=None) -> float:
        """Screen-space NDC y of the horizon (the ground plane at infinity),
        from the camera orientation. Returns a value that may exceed ±1 when the
        horizon is off-screen (looking straight down = all ground)."""
        camera = camera or self.camera
        eye = camera.eye()
        fwd = camera.target - eye
        # Horizontal component of the view direction → its vanishing point.
        dh = QVector3D(fwd.x(), fwd.y(), 0.0)
        if dh.length() < 1e-5:
            # Looking straight down/up: no horizon on screen.
            return 2.0 if fwd.z() < 0 else -2.0
        dh = dh.normalized()
        # A point very far along the horizontal heading, at eye height: as the
        # distance → ∞ it converges to the horizon, so it stays put on zoom.
        far = eye + dh * 1.0e6
        clip = mvp.map(QVector4D(far.x(), far.y(), far.z(), 1.0))
        if abs(clip.w()) < 1e-9:
            return 2.0 if fwd.z() < 0 else -2.0
        return clip.y() / clip.w()

    #: NDC span over which each half of the backdrop completes its ramp.
    _SKY_GRAD_SPAN = 1.4
    _GROUND_GRAD_SPAN = 0.9

    @staticmethod
    def _sky_gradient(sky_rgb, ground_rgb, hy):
        return ingetrazo_render.sky_gradient(sky_rgb, ground_rgb, hy)

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

    def _sun_dir_now(self):
        return ingetrazo_render.sun_dir_now(self)


    @staticmethod
    def _shadow_footprint(light_vp) -> tuple[float, float]:
        return ingetrazo_render.shadow_footprint(light_vp)

    @staticmethod
    def _light_vp(d, lo, hi, fit=None):
        return ingetrazo_render.light_vp(d, lo, hi, fit)

    @staticmethod
    def _light_view(d, lo, hi):
        return ingetrazo_render.light_view(d, lo, hi)

    #: How long the view must stay put before the fitted map is drawn (#958).
    SHADOW_SETTLE_MS = 200

    # ---- Dynamic uploads ----------------------------------------------------
    def prune_stale_selection(self):
        return ingetrazo_render.prune_stale_selection(self)

    def notify_scene_changed(self) -> None:
        """Force a redraw and emit the version-changed signal.

        Use this when an outside system (load, undo, redo) has mutated the
        scene and wants subscribers (title-bar dirty flag, etc.) to react
        without waiting for the next paint.
        """
        self.prune_stale_selection()
        _publish_scene_version(self, self.scene.version)
        self.update()

    def begin_groups_preview(self, groups=(), external: bool = False) -> None:
        return ingetrazo_render.begin_groups_preview(self, groups, external)

    def _preview_chunk(self, g):
        return ingetrazo_render.preview_chunk(self, g)

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

    def _groups_preview_transform(self) -> QMatrix4x4:
        return ingetrazo_render.groups_preview_transform(self)


    def _newell_of(self, face):
        return ingetrazo_render.newell_of(self, face)

    def _area_of(self, face) -> float:
        return ingetrazo_render.area_of(self, face)

    def _tris_of(self, face):
        return ingetrazo_render.tris_of(self, face)

    #: A face this big (corners, holes included) is worth handing to the
    #: native earcut ahead of ``_tris_of``.
    NATIVE_TRIS_MIN_CORNERS = 64

    def _seed_tris_native(self, faces) -> None:
        return ingetrazo_render.seed_tris_native(self, faces)

    def _normal_of(self, face):
        return ingetrazo_render.normal_of(self, face)

    def _static_draw_groups(self, groups):
        return ingetrazo_render.static_draw_groups(self, groups)

    def _bucket_back_face(self, face, back):
        return ingetrazo_render.bucket_back_face(self, face, back)

    def _append_textured_face(self, by_texture: dict, face, tex: dict) -> None:
        return ingetrazo_render.append_textured_face(self, by_texture, face, tex)

    def _faceme_dir(self, anchor: QVector3D) -> QVector3D:
        """Direction a face-me sprite at ``anchor`` turns toward (not yet
        flattened or normalised). Perspective: toward the eye, so figures
        left and right of centre turn slightly. Parallel: the
        VIEW direction for every sprite — a parallel camera has no real eye,
        and using the fictitious one made a figure far from the orbit target
        turn 45° away when zoomed in (Marco: "Sumari no se ve bien en vista
        frontal", 2026-09-02)."""
        cam = self.camera
        if cam.perspective:
            return cam.eye() - anchor
        return cam.eye() - cam.target

    def _figure_token(self, group):
        return ingetrazo_render.figure_token(self, group)

    def _billboard_mesh(self, group):
        return ingetrazo_render.billboard_mesh(self, group)

    def _billboard_extent(self, group):
        return ingetrazo_render.billboard_extent(self, group)

    def _billboard_quad(self, group, face_dir=None):
        return ingetrazo_render.billboard_quad(self, group, face_dir)

    def _faceme_base(self, g):
        return ingetrazo_render.faceme_base(self, g)

    def set_hover(self, entity, placement=None) -> None:
        """Set the entity (edge/face) highlighted under the cursor and repaint
        if it changed. ``None`` clears the highlight. ``placement`` is the
        component placement a face inside a component was picked through
        (see :meth:`pick_face_placement`): it is what puts the marking on the
        part."""
        if entity == self._hover_entity and placement is self._hover_placement:
            return
        self._hover_entity = entity
        self._hover_placement = placement
        self.update()

    def flash_status(self, text: str, msec: int = 2500) -> None:
        """Briefly show ``text`` in the main window's status bar (e.g. Push/Pull
        saying how far the solid lets it go). No-op if there is no status bar
        yet."""
        window = self.window()
        bar = window.statusBar() if window is not None else None
        if bar is not None:
            bar.showMessage(text, msec)

    def set_suppressed_faces(self, faces) -> None:
        """Hide a set of scene faces from the normal pass (e.g. the flat inner
        face a Push/Pull is recessing). Identity-keyed; empty set restores.
        No-op when unchanged so the drag doesn't rebuild every frame."""
        faces = ElementSet(faces)
        if faces == self._suppressed_faces:
            return
        self._suppressed_faces = faces
        self._edges_version = -1  # the faces VBO is rebuilt by _sync_edges
        self.update()

    def _rubber_band_style(self):
        """The active tool's rubber band as GL draws it: ``(segments,
        rgba, pen width)`` or None. Shared with the Metal view, whose
        overlay used one cyan pen for every band and so lost the axis and
        lock colours that tell you which way the line is going."""
        tool = self.active_tool
        if tool is None:
            return None
        segments = tool.rubber_band_lines()
        if not segments:
            return None

        # A tool can force its preview-line colour (Push/Pull uses the normal
        # edge colour so its forming box reads like real geometry, not a loose
        # orange rubber band).
        forced = getattr(tool, "wireframe_color", None)
        snap = self.last_snap
        # Axis / reference / extension cues read as "projection lines": draw them
        # a touch thicker so the alignment is easy to spot while drawing.
        inference = (
            forced is None
            and snap is not None
            and snap.kind in ("axis", "axis_inference", "reference", "extension",
                              "through_point", "perp_face")
        )
        lock = getattr(snap, "lock_color", None) if snap is not None else None
        if forced is not None:
            color = forced
        elif lock is not None:
            # A point found ON a lock line keeps the lock's colour (#711).
            color = (lock[0], lock[1], lock[2], 1.0)
        elif snap is not None and snap.kind == "axis":
            r, g, b = snap.color
            color = (r, g, b, 1.0)
        elif snap is not None and snap.kind == "axis_inference":
            r, g, b = snap.color
            color = (r, g, b, 0.50)
        elif snap is not None and snap.kind in ("reference", "through_point", "perp_face"):
            r, g, b = snap.color
            color = (r, g, b, 1.0)
        elif snap is not None and snap.kind == "close":
            color = (0.20, 0.40, 0.78, 0.95)
        else:
            color = (0.95, 0.45, 0.16, 0.85)
        return segments, color, 2.5 if inference else 2.0


    def _snap_context_suffix(self, context) -> str:
        return ingetrazo_render.snap_context_suffix(self, context)

    def _draw_snap_indicator(self, painter: QPainter, snap: SnapResult) -> None:
        # Axis-lock and inference state is conveyed by the coloured rubber
        # band; no badge follows the cursor along the lock line. Only the
        # discrete point snaps get a marker.
        return ingetrazo_render.draw_snap_indicator(self, painter, snap)

    def _edit_group_box_corners(self):
        from ingetrazo_core import edit_group_box
        return edit_group_box(self)

    def edit_group_box_contains_pixel(self, x: float, y: float) -> bool:
        from ingetrazo_core import edit_box_contains_pixel
        return edit_box_contains_pixel(self, x, y)

    # ---- Section planes ------------------------------------------------------

    def section_plane_frame(self, sp) -> list:
        from ingetrazo_core import section_frame
        return section_frame(sp.point, plane_axes(sp.normal), self.scene.bounds())

    def _draw_section_planes(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_section_planes(self, painter)

    def pick_section_plane(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_section_plane(self, screen_x, screen_y)

    def _section_cut_segments(self):
        from ingetrazo_render import section_cut
        return section_cut(self)

    @staticmethod
    def _cut_layer_tags(idx, ents):
        from ingetrazo_render import cut_layer_tags
        return cut_layer_tags(idx, ents)

    def _section_fill_tris(self):
        from ingetrazo_render import section_fill
        return section_fill(self)

    def _model_linetype_edges(self) -> list:
        from ingetrazo_render import model_layer_lines
        return model_layer_lines(self)

    @staticmethod
    def _model_line_words(segs, spec):
        from ingetrazo_render import model_line_words
        return model_line_words(segs, spec)



    def _section_layer_fills(self, color):
        from ingetrazo_render import section_layer_fills
        return section_layer_fills(self, color)



    def _section_hatch(self, spacing: float, cross: bool):
        from ingetrazo_render import section_hatch
        return section_hatch(self, spacing, cross)

    #: The sheet's cut pen (mm) a layer's own cut weight is measured
    #: against in the model: a layer at 0.50 draws the usual ~2.5 px line.
    CUT_PEN_MM = 0.5

    def _cut_edge_weights(self, plane, keep):
        from ingetrazo_render import cut_weights
        return cut_weights(self, plane, keep)

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

    def _draw_scale_box(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_scale_box(self, painter)

    def _clip_pixel_line(
        self, p0: tuple[float, float], p1: tuple[float, float],
        margin: float = 64.0,
    ) -> Optional[tuple[tuple[float, float], tuple[float, float]]]:
        from ingetrazo_core import clip_annotation_line
        return clip_annotation_line(p0, p1, self.width(), self.height(), margin)

    def _draw_guides(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_guides(self, painter)

    def _draw_geo_surfaces(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_geo_surfaces(self, painter)

    def ground_height(self, x: float, y: float) -> float | None:
        return ingetrazo_render.ground_height(self, x, y)

    def _emit_coordinate(self, ev) -> None:
        return ingetrazo_render.emit_coordinate(self, ev)

    def ground_elevation(self, x: float, y: float) -> float | None:
        return ingetrazo_render.ground_elevation(self, x, y)

    def vertical_reference(self) -> str:
        return ingetrazo_render.vertical_reference(self)

    def has_ground_surface(self) -> bool:
        return ingetrazo_render.has_ground_surface(self)

    def drape(self, v: QVector3D) -> QVector3D:
        return ingetrazo_render.drape(self, v)

    def _draw_geo_points(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_geo_points(self, painter)

    def _draw_rubber_band_overlay(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_rubber_band_overlay(self, painter)

    def _draw_guide_preview(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_guide_preview(self, painter)

    def _draw_route_marker(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_route_marker(self, painter)

    def _draw_linear_mode_label(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_linear_mode_label(self, painter)

    def _draw_geo_paths(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_geo_paths(self, painter)

    def _draw_geo_path_label(self, painter: QPainter, path) -> None:
        return ingetrazo_render.draw_geo_path_label(self, painter, path)

    @staticmethod
    def _text_block_x(p_pos, p_anchor, block_w: float) -> float:
        return ingetrazo_render.text_block_x(p_pos, p_anchor, block_w)

    def _draw_text_labels(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_text_labels(self, painter)

    def _draw_dimensions(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_dimensions(self, painter)

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

    def _draw_occluded_segment(self, painter: QPainter, p3a: QVector3D,
                               p3b: QVector3D, samples: int = 16) -> None:
        from ingetrazo_core import annotation_strokes
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        for a, b in annotation_strokes(self, p3a, p3b, mvp, samples):
            painter.drawLine(QPointF(*a), QPointF(*b))

    def _project_samples(self, pts: list[QVector3D]):
        from ingetrazo_core import annotation_samples
        return annotation_samples(self, pts, self.camera.projection_matrix() * self.camera.view_matrix())

    def _draw_inference_marker(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_inference_marker(self, painter)

    def _length_label_at(self):
        return ingetrazo_render.length_label_at(self)

    def _draw_length_label(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_length_label(self, painter)

    def _measurement_text(self) -> str:
        return ingetrazo_render.measurement_text(self)

    def _draw_axis_lock_label(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_axis_lock_label(self, painter)

    def _draw_plane_lock_label(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_plane_lock_label(self, painter)

    def _draw_inference_label(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_inference_label(self, painter)

    def _draw_reference_label(self, painter: QPainter) -> None:
        return ingetrazo_render.draw_reference_label(self, painter)

    # ---- Pixel ↔ world ------------------------------------------------------

    def _coplanar_eps(self, origin: QVector3D, direction: QVector3D,
                      best_t: float) -> float:
        """How much deeper than the nearest hit a face may sit and still
        count as "the same depth" (coplanar faces, a hole's rim on its
        slab): a ten-thousandth of the distance from the EYE, never less
        than 0.1 mm. It used to be a ten-thousandth of the ray parameter —
        fine in perspective, where the ray starts at the eye, but the
        parallel camera's ray starts a far plane behind it, 10 km back, so
        the tolerance came out a full metre and a smaller face 30 cm
        BEHIND the one under the cursor won the coplanar tiebreak
        (@pacaeiro, issue #37: «face detection is identifying the face
        that is behind the one in front… in CAMERA orthogonal mode»)."""
        cam = getattr(self, "camera", None)
        if cam is None:                     # a bare stand-in (tests)
            return max(1e-4, best_t * 1e-4)
        t_eye = QVector3D.dotProduct(cam.eye() - origin, direction)
        return max(1e-4, abs(best_t - t_eye) * 1e-4)

    def _world_from_pixel(self, x: int, y: int) -> Optional[QVector3D]:
        return ingetrazo_render.world_from_pixel(self, x, y)

    #: A ray meeting the work plane below this angle gives a point nobody
    #: asked for. Measured: at 25 deg of camera pitch — an ordinary
    #: architectural view — nothing on screen comes anywhere near it, so this
    #: only fires where the plane really is unreadable.
    GRAZING_PLANE_DEG = 6.0

    def _work_plane_normal(self) -> Optional[QVector3D]:
        """The normal of the plane the last pixel→world used, or ``None``."""
        plane = getattr(self, "_last_work_plane", None)
        return plane[1] if plane is not None else None

    @staticmethod
    def _same_plane(a, b, tol: float = 1e-6) -> bool:
        return ingetrazo_render.same_plane(a, b, tol)

    def _planar_face_plane(self, tool, cursor, captured=None, face_plane=None):
        return ingetrazo_render.planar_face_plane(self, tool, cursor, captured, face_plane)

    def _ray_plane(self, origin: QVector3D, direction: QVector3D,
                   plane_point: QVector3D, plane_normal: QVector3D,
                   *, allow_grazing: bool = False,
                   ) -> Optional[QVector3D]:
        return ingetrazo_render.ray_plane(self, origin, direction, plane_point, plane_normal, allow_grazing)

    #: XY stays the ground in shallow views and where its cursor-ray hit is
    #: reasonably close compared with the two vertical global planes.
    GROUND_PREFERENCE_ANGLE_DEG = 5.0
    GROUND_DEPTH_PREFERENCE_RATIO = 1.25
    PERSPECTIVE_GROUND_DEPTH_PREFERENCE_RATIO = 10.0
    PLANE_ALIGNMENT_TIE_MARGIN = 0.05

    def _camera_planar_work_plane(
        self, cursor: Optional[tuple[float, float]] = None,
    ) -> Optional[tuple[QVector3D, QVector3D]]:
        """Choose XY as floor/ceiling when preferred, otherwise XZ or YZ.

        The vertical planes compete by their alignment with the local cursor
        ray. XY remains the ground (or ceiling when viewed from below), with
        a depth preference that avoids mapping a nearby cursor to an
        implausibly distant point on the floor.
        """
        from core import axes
        origin = axes.origin()
        view = self.camera.target - self.camera.eye()
        if view.lengthSquared() < 1e-18:
            return None
        view.normalize()
        ray_origin = self.camera.eye()
        ray_direction = view
        cursor_origin: Optional[QVector3D] = None
        cursor_direction: Optional[QVector3D] = None
        if cursor is not None:
            cursor_origin, cursor_direction = self._pixel_to_ray(*cursor)
            if cursor_origin is not None and cursor_direction is not None:
                ray_origin = cursor_origin
                ray_direction = cursor_direction.normalized()

        normals = {name: axes.axis(name) for name in ("x", "y", "z")}
        hits: dict[str, tuple[QVector3D, float]] = {}
        if ray_origin is not None:
            for name, normal in normals.items():
                hit = self._ray_plane(
                    ray_origin, ray_direction, origin, normal,
                    allow_grazing=True)
                if hit is None:
                    continue
                depth = (
                    (hit - self.camera.eye()).length()
                    if self.camera.perspective
                    else QVector3D.dotProduct(hit - self.camera.eye(), view)
                )
                if depth >= 0:
                    hits[name] = hit, depth

        horizontal_angle = math.degrees(math.asin(min(
            1.0, abs(QVector3D.dotProduct(view, normals["z"])))))
        if "z" in hits:
            vertical_depths = [
                hits[name][1] for name in ("x", "y") if name in hits]
            depth_ratio = (
                self.GROUND_DEPTH_PREFERENCE_RATIO
                if horizontal_angle <= self.GROUND_PREFERENCE_ANGLE_DEG
                else self.PERSPECTIVE_GROUND_DEPTH_PREFERENCE_RATIO
            )
            if (not vertical_depths
                    or hits["z"][1] <= min(vertical_depths) * depth_ratio):
                return origin, normals["z"]

        alignment = {
            name: abs(QVector3D.dotProduct(ray_direction, normals[name]))
            for name in ("x", "y")
        }
        candidates = {"x", "y"}
        if cursor_origin is not None and cursor_direction is not None:
            candidates = {
                name for name in candidates
                if self._ray_plane(
                    cursor_origin, cursor_direction, origin, normals[name],
                    allow_grazing=True) is not None
            }
        if not candidates:
            candidates = {"x", "y"}
        normal_name = max(candidates, key=alignment.__getitem__)
        if (candidates == {"x", "y"}
                and abs(alignment["x"] - alignment["y"])
                <= self.PLANE_ALIGNMENT_TIE_MARGIN):
            normal_name = "y"
        return origin, normals[normal_name]

    # When the camera is at least this tilted off the horizon, the work plane
    # stays horizontal (XY) — that covers top, iso, and most architectural
    # angles. Only at near-horizon views does it switch to a vertical plane,
    # which is the only orientation where cursor-to-ground is ambiguous.
    HORIZON_PITCH_THRESHOLD_DEG = 15.0

    def _current_work_plane(
        self, cursor: Optional[tuple[float, float]] = None
    ) -> tuple[QVector3D, QVector3D]:
        return ingetrazo_render.current_work_plane(self, cursor)

    def _front_face_at(self, x: float, y: float, which: str = "sel"):
        return ingetrazo_render.front_face_at(self, x, y, which)

    def _front_face_lean(self, idx, nat, o, d, which: str = "sel"):
        return ingetrazo_render.front_face_lean(self, idx, nat, o, d, which)

    #: How far from the cursor (px) a face crossing snaps, and how far
    #: either side of the crossing the search checks that each face shows.
    _CROSSING_RING_PX = 8.0
    _CROSSING_SIDE_PX = 3.0

    def _face_crossing_point(self, px: float, py: float):
        return ingetrazo_render.face_crossing_point(self, px, py)

    def _crossing_sides(self, px, py, o, d, f, g):
        return ingetrazo_render.crossing_sides(self, px, py, o, d, f, g)

    def _crossing_on_screen(self, f, g, found):
        return ingetrazo_render.crossing_on_screen(self, f, g, found)

    def _face_outline_world(self, px: float, py: float):
        return ingetrazo_render.face_outline_world(self, px, py)

    def _still_face_under(self, px: float, py: float) -> bool:
        return ingetrazo_render.still_face_under(self, px, py)

    def _face_center_under(self, px: float, py: float):
        return ingetrazo_render.face_center_under(self, px, py)

    #: More selected faces than this and their centres are neither drawn
    #: nor offered: a «select all» would bury the model in dots.
    FACE_CENTER_SELECTION_MAX = 500

    def _selected_face_centers(self) -> list:
        return ingetrazo_render.selected_face_centers(self)

    def _face_centers_near(self, px: float, py: float) -> list:
        return ingetrazo_render.face_centers_near(self, px, py)

    def _draw_selected_face_centers(self, painter: QPainter) -> None:
        """A small dot at the centre of each selected face (the ones the
        view can see), so the point is in sight before the cursor gets
        there — on a ring it sits in the hole, where hovering finds no face."""
        pts = self._selected_face_centers()
        if not pts:
            return
        cam = self.camera
        key = (self.scene.version, tuple((p.x(), p.y(), p.z()) for p in pts), cam.yaw,
               cam.pitch, cam.distance, cam.perspective, cam.target.x(),
               cam.target.y(), cam.target.z(), self.width(), self.height())
        cached = getattr(self, "_sel_face_center_dots", None)
        if cached is None or cached[0] != key:
            hidden = self._occluded_many(pts)
            dots = []
            for p, h in zip(pts, hidden):
                pc = None if h else self._world_to_pixel(p)
                if pc is not None:
                    dots.append(pc)
            cached = self._sel_face_center_dots = (key, dots)
        if not cached[1]:
            return
        # As big as a snap marker (14 px, like the endpoint's square), so
        # the dot reads at a glance; solid and without the cross, so the
        # one the cursor has snapped to still stands out.
        from core.snap import COLOR_ON_FACE
        painter.setPen(QPen(QColor(255, 255, 255, 230), 2.5))
        painter.setBrush(QColor.fromRgbF(*COLOR_ON_FACE, 0.85))
        for x, y in cached[1]:
            painter.drawEllipse(QPointF(x, y), 7.0, 7.0)
        painter.setBrush(Qt.NoBrush)

    def _face_context_under(self, px: float, py: float):
        return ingetrazo_render.face_context_under(self, px, py)

    def _hover_face_plane(self, cursor):
        return ingetrazo_render.hover_face_plane(self, cursor)

    def _locked_plane_through_reference(self, tool, cursor):
        return ingetrazo_render.locked_plane_through_reference(self, tool, cursor)

    def _ray_hit_plane(self, cursor, plane):
        return ingetrazo_render.ray_hit_plane(self, cursor, plane)

    def _start_point_plane(self, tool, start):
        """The camera-aware plane through the active start point."""
        forward = (self.camera.target - self.camera.eye())
        if forward.length() < 1e-9:
            return start, QVector3D(0.0, 0.0, 1.0)
        forward = forward.normalized()
        # Tools that drag geometry up and down (Move) use a camera-facing
        # vertical plane through the grab point, so pulling the mouse up raises
        # the geometry rigidly instead of sliding it across the ground (which
        # shears connected faces and looks disordered). The plane contains the
        # world Z axis; its normal is the camera's horizontal heading. Only when
        # looking nearly straight down — where height is unreadable anyway — does
        # it fall back to the horizontal plane.
        if getattr(tool, "prefers_vertical_drag", False):
            horiz = QVector3D(forward.x(), forward.y(), 0.0)
            if horiz.length() >= math.sin(math.radians(self.HORIZON_PITCH_THRESHOLD_DEG)):
                return start, horiz.normalized()
            return start, QVector3D(0.0, 0.0, 1.0)
        # |forward.z| ≈ sin(pitch). Anything tilted more than the threshold
        # keeps the horizontal plane.
        vertical = self._near_horizon_vertical(start)
        if vertical is not None:
            return vertical
        return start, QVector3D(0.0, 0.0, 1.0)

    def _near_horizon_vertical(
        self, start: QVector3D
    ) -> Optional[tuple[QVector3D, QVector3D]]:
        """The vertical work plane through ``start`` when the camera sits near
        the horizon — the only orientation where cursor→ground is ambiguous
        and dragging up must move in Z. ``None`` at steeper tilts."""
        forward = (self.camera.target - self.camera.eye())
        if forward.length() < 1e-9:
            return None
        forward = forward.normalized()
        if abs(forward.z()) >= math.sin(
                math.radians(self.HORIZON_PITCH_THRESHOLD_DEG)):
            return None
        # Pick the vertical plane whose normal is more end-on to the camera
        # so cursor motion maps cleanly to it.
        if abs(forward.x()) >= abs(forward.y()):
            return start, QVector3D(1.0, 0.0, 0.0)
        return start, QVector3D(0.0, 1.0, 0.0)


    #: Eye depth (m) a segment is cut at when it runs behind the camera.
    _CLIP_W = 1e-3

    def _clip_w(self) -> float:
        """Where a segment is cut in front of the eye: 1 mm, or the near
        plane when the camera has come closer than that would allow — at a
        few millimetres from a small part, 1 mm cut away the very line
        being drawn (#653, #692)."""
        cam = self.camera
        if not getattr(cam, "perspective", True):
            return self._CLIP_W
        near = getattr(cam, "near_plane", None)
        return min(self._CLIP_W, near()) if callable(near) else self._CLIP_W

    def _segment_to_pixels(self, p0: QVector3D, p1: QVector3D):
        """A world segment → its two screen pixels, cut where it passes
        behind the eye instead of dropped whole. A guide is kilometres
        long: in perspective one end is nearly always behind the camera,
        and the preview never showed (#89)."""
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        c0 = mvp.map(QVector4D(p0.x(), p0.y(), p0.z(), 1.0))
        c1 = mvp.map(QVector4D(p1.x(), p1.y(), p1.z(), 1.0))
        w0, w1, eps = c0.w(), c1.w(), self._clip_w()
        if w0 < eps and w1 < eps:
            return None
        if w0 < eps:
            c0 = c0 + (c1 - c0) * ((eps - w0) / (w1 - w0))
            c0.setW(eps)
        elif w1 < eps:
            c1 = c1 + (c0 - c1) * ((eps - w1) / (w0 - w1))
            c1.setW(eps)
        (x0, y0), (x1, y1) = [
            ((c.x() / c.w() * 0.5 + 0.5) * self.width(),
             (1.0 - (c.y() / c.w() * 0.5 + 0.5)) * self.height())
            for c in (c0, c1)]
        # Cut to the window (a margin past it): a point a hair in front of
        # the eye lands millions of pixels out, past what the painter's
        # fixed-point rasteriser draws right.
        m = 64.0
        lo_x, hi_x = -m, self.width() + m
        lo_y, hi_y = -m, self.height() + m
        dx, dy = x1 - x0, y1 - y0
        t0, t1 = 0.0, 1.0
        for p, q in ((-dx, x0 - lo_x), (dx, hi_x - x0),
                     (-dy, y0 - lo_y), (dy, hi_y - y0)):
            if p == 0.0:
                if q < 0.0:
                    return None
                continue
            r = q / p
            if p < 0.0:
                t0 = max(t0, r)
            else:
                t1 = min(t1, r)
        if t0 > t1:
            return None
        return [(x0 + dx * t0, y0 + dy * t0), (x0 + dx * t1, y0 + dy * t1)]

    def _draw_extension_overlays(self, painter: QPainter) -> None:
        """Extensions' overlays (``ExtensionApp.add_overlay``): whatever the
        active tool, and never able to break the frame. Each call is fenced
        by save/restore, and one that raises is logged ONCE and dropped: a
        paint handler failing on every frame would flood the log and leave
        the view half drawn for good."""
        for fn in list(getattr(self, "_ext_overlays", ())):
            painter.save()
            try:
                fn(self, painter)
            except Exception:  # noqa: BLE001 — a plugin never breaks paint
                import logging
                logging.getLogger("ingetrazo.plugins").exception(
                    "extension overlay %r failed; removed", fn)
                try:
                    self._ext_overlays.remove(fn)
                except ValueError:
                    pass
            finally:
                painter.restore()

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

    def _pixel_projector(self):
        """:meth:`_world_to_pixel` with the camera as it is NOW: the matrix
        is made once and the same Qt arithmetic maps every point, so a
        snap search asking for thousands of points per pointer move (20 000
        on the plant of #158) gets the very pixels it got before without
        rebuilding the view and projection for each one. Only for work the
        camera cannot move under (one snap search)."""
        if "_world_to_pixel" in vars(self):
            return self._world_to_pixel     # a test's stand-in wins
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        w, h = self.width(), self.height()
        vec4 = QVector4D

        def world_to_pixel(world):
            clip = mvp.map(vec4(world.x(), world.y(), world.z(), 1.0))
            if clip.w() <= 0:
                return None
            ndc_x = clip.x() / clip.w()
            ndc_y = clip.y() / clip.w()
            return ((ndc_x * 0.5 + 0.5) * w, (1.0 - (ndc_y * 0.5 + 0.5)) * h)
        world_to_pixel.snap_projection = (mvp.data(), (w, h))
        return world_to_pixel


    @staticmethod
    def _mesh_fingerprint(mesh):
        return ingetrazo_render.mesh_fingerprint(mesh)

    @staticmethod
    def _faces_paint_fp(mesh) -> int:
        return ingetrazo_render.faces_paint_fp(mesh)

    def _group_fp(self, group):
        return ingetrazo_render.group_fp(self, group)

    @staticmethod
    def _translation_probe(entry, mesh):
        return ingetrazo_render.translation_probe(entry, mesh)

    @staticmethod
    def _samples_match(entry, mesh) -> bool:
        return ingetrazo_render.samples_match(entry, mesh)

    @staticmethod
    def _shift_obb(entry, d) -> None:
        return ingetrazo_render.shift_obb(entry, d)

    def _shift_chunk(self, entry, d, mesh) -> None:
        return ingetrazo_render.shift_chunk(self, entry, d, mesh)


    def _instance_chunk(self, group):
        return ingetrazo_render.instance_chunk(self, group)

    def _shift_instance_entry(self, entry, d: QVector3D) -> None:
        return ingetrazo_render.shift_instance_entry(self, entry, d)

    def _group_chunk(self, group):
        return ingetrazo_render.group_chunk(self, group)

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

    @staticmethod
    def _chunk_cache_key(mesh, fp) -> str:
        return ingetrazo_render.chunk_cache_key(mesh, fp)

    _CHUNK_CACHE_FIELDS = (
        "edges", "vcol", "smooth_vcol", "by_texture", "tcol", "ttex", "back_vcol",
        "back_tex", "back_tcol", "back_ttex", "fvcol", "dback", "areas",
        "v0", "e1", "e2", "tri_ent", "soft_pts", "soft_n0", "soft_c0",
        "soft_n1", "soft_c1", "soft_single", "edge_layer", "soft_layer",
        "bbox", "coordsum", "nv", "ne", "nf", "samples")

    def _chunk_cache_load(self, group, fp, vkey):
        return ingetrazo_render.chunk_cache_load(self, group, fp, vkey)

    def _chunk_cache_store(self, mesh, entry) -> None:
        return ingetrazo_render.chunk_cache_store(self, mesh, entry)

    def _np_mvp(self):
        """Current MVP as a (4, 4) float64 NumPy matrix (row-major indexing).
        ``QMatrix4x4.data()`` is column-major, hence the Fortran reshape."""
        m = self.camera.projection_matrix() * self.camera.view_matrix()
        return np.asarray(m.data(), dtype=np.float64).reshape(4, 4, order="F")

    def _camera_px(self):
        """``(mvp, width, height)``: what the native pick kernels project
        with (``mvp`` column-major, as ``Matrix4.data()``)."""
        m = self.camera.projection_matrix() * self.camera.view_matrix()
        return list(m.data()), float(self.width()), float(self.height())

    def _project_px(self, pts):
        """Batch world points (N, 3) → ``(px, py, in_front)`` arrays — the
        exact math of :meth:`_world_to_pixel` (``ingetrazo_core.project_px``)."""
        points = np.ascontiguousarray(pts, dtype=np.float64).reshape(-1, 3)
        matrix = self._np_mvp().ravel(order="F").tolist()
        return ingetrazo_core.project_px(points, matrix, float(self.width()), float(self.height()))

    @staticmethod
    def _frustum_planes(mvp):
        from ingetrazo_core import frustum_planes
        return frustum_planes(mvp)

    @staticmethod
    def _aabb_visible(planes, lo, hi) -> bool:
        from ingetrazo_core import aabb_visible
        return aabb_visible(planes, lo, hi)

    def _visible_spans(self, spans, planes, split=None):
        from ingetrazo_core import visible_draw_spans
        return visible_draw_spans(self, spans, planes, split,
                                  len(spans) >= _SPAN_VECTOR_MIN and bool(planes))

    def _visible_spans_np(self, spans, planes, split=None):
        from ingetrazo_core import visible_draw_spans
        return visible_draw_spans(self, spans, planes, split, True)

    def _tex_run_spans(self, parts, planes, fading: bool):
        """Frustum-culled draw spans of one texture run, split into
        ``(context, subject)`` when the edited group's surroundings are
        fading. Without a fade everything is context and ``subject`` is
        empty, so callers draw one list either way."""
        if not fading:
            return self._visible_spans(
                [(bb, s, n) for bb, s, n, _ in parts], planes)[0], []
        ctx = self._visible_spans(
            [(bb, s, n) for bb, s, n, sub in parts if not sub], planes)[0]
        subj = self._visible_spans(
            [(bb, s, n) for bb, s, n, sub in parts if sub], planes)[0]
        return ctx, subj

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

    def _pick_lazy_table(self):
        return ingetrazo_core.pick_lazy_table(self)

    def _pick_instance_set(self, table):
        return ingetrazo_core.pick_instance_set(self, table)

    def start_pick_warmup(self) -> None:
        return ingetrazo_core.start_pick_warmup(self)

    def _static_occluders(self, idx):
        return ingetrazo_core.static_occluders(self, idx)

    def _static_ray_rows(self, idx, ent_mask, o3, d3):
        return ingetrazo_core.static_ray_rows(self, idx, ent_mask, o3, d3)

    def _lazy_native(self):
        return ingetrazo_core.pick_lazy_native(self)

    def _pick_materialize(self, near):
        return ingetrazo_core.pick_materialize(self, near)

    def _depth_limits(self, x: float, y: float, r: float):
        return ingetrazo_core.pick_depth_limits(self, x, y, r)

    @staticmethod
    def _depth_hidden(seen, pts, sx, sy):
        return ingetrazo_core.depth_hidden(seen, pts, sx, sy)

    def _footprint_need(self, table, x: float, y: float, reach=None):
        return ingetrazo_core.footprint_need(self, table, x, y, reach)

    def _footprint_rows(self, table):
        return ingetrazo_core.footprint_rows(self, table)

    def _visible_depth_limit(self, x: float, y: float):
        return ingetrazo_core.visible_depth_limit(self, x, y)

    def _placements_under_px(self, lo, hi, x, y, r=None):
        return ingetrazo_core.placements_under_px(self, lo, hi, x, y, r)

    def _outer_loose_meshes(self):
        return ingetrazo_core.outer_loose_meshes(self)

    def _loose_mesh_wrapper(self, mesh):
        return ingetrazo_core.loose_mesh_wrapper(self, mesh)

    def _pick_index(self, near=None):
        return ingetrazo_core.document_pick_index(self, near)

    @staticmethod
    def _pick_flat(idx):
        return ingetrazo_core.flat_pick_index(idx)

    def ray_distance(self, origin: QVector3D, direction: QVector3D,
                     reach: float | None = None):
        return ingetrazo_core.index_ray_distance(self, origin, direction, reach)

    def _ray_hits(self, idx, origin, direction, ent_mask,
                  reduce_global: bool = False, reach: float | None = None,
                  own: bool = True):
        return ingetrazo_core.index_ray_hits(self, idx, origin, direction, ent_mask,
                              reduce_global, reach, own)

    def _ray_near(self, x: float, y: float):
        """What a ray pick at widget pixel ``(x, y)`` asks the pick index
        to bake: nothing with the engine, which answers for the lazy
        placements itself (see _face_hits), the placements the ray meets
        otherwise."""
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        return "skip" if nat is not None else ("px", x, y)

    def _face_hits(self, idx, origin, direction, which: str = "sel"):
        return ingetrazo_core.face_hits(self, idx, origin, direction, which)

    def _lazy_sel_rows(self, nat):
        return ingetrazo_core.pick_lazy_selectable(self, nat)

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

    def _linework_edge_rows(self, screen_x: float, screen_y: float):
        return ingetrazo_core.linework_edge_rows(self, screen_x, screen_y)

    def pick_linework_groups(self, screen_x: float, screen_y: float):
        return ingetrazo_core.linework_groups(self, screen_x, screen_y, False)

    def linework_groups_around(self, screen_x: float, screen_y: float):
        return ingetrazo_core.linework_groups(self, screen_x, screen_y, True)

    def _pick_edge(self, screen_x: float, screen_y: float,
                   visible_only: bool, which: str = "sel", hidden=None,
                   with_distance: bool = False):
        """``which``: "sel" for the edges a click means, "snap" for the ones a
        reference may hold — the rest of the model's loose edges, drawn
        around an open group / component, are snappable and not selectable.
        ``hidden``: the visibility test of ``visible_only`` (default
        :meth:`_is_occluded`). ``with_distance``: ``(edge, pixels)``,
        ``(None, inf)`` for no edge."""
        found = self._pick_edge_d(screen_x, screen_y, visible_only, which,
                                  hidden)
        if with_distance:
            return found if found is not None else (None, float("inf"))
        return found[0] if found is not None else None

    def _pick_edge_d(self, screen_x, screen_y, visible_only, which, hidden):
        return ingetrazo_core.pick_edge_distance(self, screen_x, screen_y, visible_only, which, hidden)

    def pick_dimension(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_dimension(self, screen_x, screen_y)

    def pick_text_label(self, screen_x: float, screen_y: float,
                        rect_only: bool = False):
        return ingetrazo_render.pick_text_label(self, screen_x, screen_y, rect_only)

    def pick_geopath(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_geopath(self, screen_x, screen_y)

    @classmethod
    def _tool_busy_for_delete(cls, tool) -> bool:
        """``_tool_busy`` plus the per-tool state that means "an operation
        holds geometry": Offset's / Push-Pull's base face, Follow Me's
        profile, Fillet's picked edges or live sizing. Delete waits then."""
        if cls._tool_busy(tool):
            return True
        return any(getattr(tool, attr, None)
                   for attr in ("base_face", "_profile", "sizing", "edges"))

    @staticmethod
    def _tool_busy(tool) -> bool:
        """Whether the active tool has an operation in progress that Esc should
        cancel before falling through to clearing the selection: an unfinished
        chain (start_point / nodes), a drag (dragging / grab / node edit), or
        an eraser stroke.

        The POINTS are tested against None, not for truth: ``QVector3D(0, 0,
        0)`` is falsy in PySide6 (the house gotcha), so a line or a move
        started exactly ON THE ORIGIN read as "nothing in progress" — Esc
        would skip straight to clearing the selection instead of cancelling
        it. Found on 2026-09-17 writing the Alt gate, which asks this same
        question."""
        for attr in ("start_point", "grab"):
            if getattr(tool, attr, None) is not None:
                return True
        for attr in ("dragging", "_drag", "_stroke"):
            if getattr(tool, attr, None):
                return True
        if getattr(tool, "nodes", None):
            return True
        return False

    def _gedge_screen(self):
        return ingetrazo_core.gedge_screen(self)

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

    def _gedge_dist(self, px: float, py: float):
        return ingetrazo_core.gedge_dist(self, px, py)

    def _ledge_screen(self):
        return ingetrazo_core.ledge_screen(self)

    def _nearby_loose_edges(self, px: float, py: float,
                            radius_px: float = 64.0, cap: int = 400) -> list:
        return ingetrazo_core.nearby_loose_edges(self, px, py, radius_px, cap)

    def _nearby_group_edges(self, px: float, py: float,
                            radius_px: float = 48.0, cap: int = 48) -> list:
        return ingetrazo_core.nearby_group_edges(self, px, py, radius_px, cap)

    def _nearby_edges_unbaked(self, px, py, radius, cap, nat,
                              with_d=False) -> list:
        return ingetrazo_core.nearby_edges_unbaked(self, px, py, radius, cap, nat, with_d)

    def _nearby_edges_found(self, px, py, radius, cap, nat) -> list:
        return ingetrazo_core.nearby_edges_found(self, px, py, radius, cap, nat)

    def _pick_group_edge_unbaked(self, px, py, nat):
        return ingetrazo_core.pick_group_edge_unbaked(self, px, py, nat)

    def pick_edge_any(self, screen_x: float, screen_y: float):
        return ingetrazo_core.pick_edge_any(self, screen_x, screen_y)

    #: How close (px) the cursor must come to a component's origin or an
    #: arc's midpoint for it to enter the snap scene at all.
    POINT_INFERENCE_PX = 40.0

    def _component_origin_points(self, px: float, py: float) -> list:
        return ingetrazo_core.component_origin_points(self, px, py)

    def _arc_midpoints(self, px: float, py: float) -> list:
        return ingetrazo_core.arc_midpoints(self, px, py)

    @staticmethod
    def _mesh_arc_midpoints(mesh) -> list:
        return ingetrazo_core.mesh_arc_midpoints(mesh)

    def _billboard_placements(self) -> list:
        return ingetrazo_core.billboard_placements(self)

    def _billboard_snap_edges(self) -> list:
        return ingetrazo_core.billboard_snap_edges(self)

    def _snap_scene(self, px: Optional[float] = None,
                    py: Optional[float] = None):
        return ingetrazo_core.document_snap_scene(self, px, py)

    def _group_obb(self, group):
        return ingetrazo_core.group_obb(self, group)

    def _proto_points_cache(self, group) -> dict:
        return ingetrazo_core.proto_points_cache(self, group)

    def _editing_meshes(self) -> set:
        return ingetrazo_core.editing_meshes(self)

    def _proto_points_store(self, group, protos: dict) -> None:
        return ingetrazo_core.proto_points_store(self, group, protos)

    @staticmethod
    def _compute_obb(group, protos=None):
        return ingetrazo_core.compute_obb(group, protos)

    def _update_center_ref(self, x: float, y: float) -> None:
        return ingetrazo_core.update_center_ref(self, x, y)

    def _center_of_group_edge(self, pseudo):
        return ingetrazo_core.center_of_group_edge(self, pseudo)

    def _center_of_edge(self, edge, mesh):
        return ingetrazo_core.center_of_edge(self, edge, mesh)

    def _center_of_face(self, face, group, mesh, x, y):
        return ingetrazo_core.center_of_face(self, face, group, mesh, x, y)

    def _center_hint_px(self, ref, pc) -> float:
        return ingetrazo_core.center_hint_px(self, ref, pc)

    def _valid_center_ref(self):
        return ingetrazo_core.valid_center_ref(self)

    def _selection_box_points(self, skip=()) -> list:
        return ingetrazo_core.selection_box_points(self, skip)

    def pick_axis(self, screen_x: float, screen_y: float):
        """The model axis line under the cursor — ``"x"`` / ``"y"`` /
        ``"z"`` — or ``None``. The Tape reads an axis as a guide source
        (Rafael, Revisión 3: «pinchas el eje y sacas una guía
        paralela a 20 m del origen» before anything is drawn)."""
        from core.guide import GUIDE_HALF_LEN
        from core import axes as _axes
        best, best_d = None, self.pick_threshold_px
        o = _axes.origin()
        for name in ("x", "y", "z"):
            d = _axes.axis(name) * GUIDE_HALF_LEN
            seg = self._clip_segment_front(o - d, o + d)
            if seg is None:
                continue
            pa = self._world_to_pixel(seg[0])
            pb = self._world_to_pixel(seg[1])
            if pa is None or pb is None:
                continue
            dist = _point_to_segment_distance_2d((screen_x, screen_y), pa, pb)
            if dist < best_d:
                best_d = dist
                best = name
        return best

    def _axis_source_visible(self, name: str, point: QVector3D) -> bool:
        """Whether an axis has a visible screen direction at ``point``.

        An axis viewed end-on has no usable screen direction; near its
        vanishing point a small world step must still move visibly on screen.
        """
        from core import axes as _axes
        axis = _axes.axis(name)
        half_step = 0.25
        before = self._world_to_pixel(point - axis * half_step)
        after = self._world_to_pixel(point + axis * half_step)
        if before is None or after is None:
            return False
        return math.hypot(after[0] - before[0],
                          after[1] - before[1]) >= 0.5

    def _shown_guides(self) -> list:
        return shown_guides(self.scene)

    def _circle_pick_distance(self, g, screen_x: float, screen_y: float):
        return ingetrazo_render.circle_pick_distance(self, g, screen_x, screen_y)

    def pick_guide(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_guide(self, screen_x, screen_y)

    def pick_vertex(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_vertex(self, screen_x, screen_y)

    def _pick_vertex_unbaked(self, screen_x, screen_y, nat):
        return ingetrazo_render.pick_vertex_unbaked(self, screen_x, screen_y, nat)

    def hlr_geometry(self, tags: bool = False):
        return ingetrazo_render.hlr_geometry(self, tags)

    def hlr_geometry_now(self):
        return ingetrazo_render.hlr_geometry_now(self)

    def _effective_style(self):
        """The display style this frame draws with (Styles): the
        composer's ``plano_style`` override maps onto the same face modes,
        then a ``style_override``, else the scene's active style. Shared by
        the paint pass and the snap engine so what the eye sees and what
        the cursor may grab never disagree."""
        from core.style import Style
        if self.plano_style == "tecnico":
            return Style(name="tecnico", face_mode="hidden_line", sky=False,
                         background=(1.0, 1.0, 1.0))
        if self.plano_style == "lineas":
            return Style(name="lineas", face_mode="wireframe", sky=False,
                         background=(1.0, 1.0, 1.0))
        if self.style_override is not None:
            return self.style_override
        return self._view_style() or Style()

    def _view_style(self):
        """The style this view shows outside the composer: its own when it
        has one (one of several views, restyled there), else the scene's."""
        return (getattr(self, "view_style", None)
                or getattr(self.scene, "display_style", None))

    # ---- Screen-depth occlusion for the overlay's annotations ---------------



    def _occluded_on_screen(self, world: QVector3D) -> bool:
        return ingetrazo_core.occluded_on_screen(self, world)

    def _is_occluded(self, world: QVector3D) -> bool:
        return ingetrazo_core.is_occluded(self, world)

    #: Cap on rays × triangles in one NumPy block of ``_occluded_many``
    #: (×3 float64 per array: ~50 MB at most).
    _OCCL_BLOCK = 2_000_000

    def _occluded_many(self, worlds) -> list:
        return ingetrazo_core.occluded_many(self, worlds)

    def _first_clear(self, pts):
        return ingetrazo_core.first_clear(self, pts)

    def _occluded_memo_key(self):
        return ingetrazo_core.occluded_memo_key(self)

    def _occluded_many_now(self, worlds) -> list:
        return ingetrazo_core.occluded_many_now(self, worlds)

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

    def _face_any(self, screen_x: float, screen_y: float, which: str):
        return ingetrazo_render.face_any(self, screen_x, screen_y, which)

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

    def _smallest_at_front(self, hits, origin, direction):
        return ingetrazo_render.smallest_at_front(self, hits, origin, direction)

    def pick_group(self, screen_x: float, screen_y: float):
        return ingetrazo_render.pick_group(self, screen_x, screen_y)

    def _pointer_for_tool(self, key, plus=False):
        factory = getattr(getattr(self, "_host", None), "tool_cursor", None)
        if factory is None:
            from views.icons import tool_cursor as factory
        return factory(key, plus)

    def _apply_tool_cursor(self) -> None:
        """The pointer becomes the active tool's icon; Select and
        unknown tools keep the standard arrow.

        A tool that does something else under a modifier says so with the
        pointer: Paint holds Alt to SAMPLE a face's material instead of
        painting it, and the cursor swaps the bucket for an eyedropper while
        it is down."""
        icon = getattr(self.active_tool, "icon", None)
        if icon == "paint":
            from tools.paint import PaintTool
            # ``keyboardModifiers()`` is the state as of the LAST event
            # delivered: on the Alt press it did not include Alt yet and on
            # the release it still did, so the pointer flipped once per
            # tap instead of following the key (issue #47, @pacaeiro: «the
            # cursor is acting as a toggle button»). The key handlers say
            # explicitly whether Alt is down.
            if PaintTool.sample_armed or getattr(self, "_alt_down", False):
                icon = "eyedropper"
        # The guide-mode plus: the Tape and the Protractor say with
        # it whether this measurement will leave a guide behind (issue #29).
        plus = bool(getattr(self.active_tool, "cursor_plus", False))
        cur = (self._pointer_for_tool(icon, plus)
               if self.active_tool is not None else None)
        if cur is None and self.active_tool is not None:
            # A tool without a drawn icon can still ask for a stock Qt
            # cursor (Position Texture shows a hand).
            solid = getattr(self.active_tool, "solid_cursor_key", None)
            if solid and getattr(getattr(self, "_host", None), "tool_cursor", None):
                self.setCursor("tool:" + solid)
                return
            shape = getattr(self.active_tool, "qt_cursor", None)
            if shape is not None:
                if NATIVE:
                    cur = shape
                else:
                    from PySide6.QtGui import QCursor
                    cur = QCursor(shape)
        if cur is not None:
            self.setCursor(cur)
        else:
            self.unsetCursor()

    def _apply_nav_cursor(self) -> None:
        """The pointer for a camera nav mode: orbit / pan / the magnifier
        (each navigation tool shows its own icon)."""
        if self.nav_mode in ("orbit", "pan", "zoom", "zoom_window"):
            cur = self._pointer_for_tool(self.nav_mode)
            if cur is not None:
                self.setCursor(cur)
                return
            self.setCursor(Qt.CrossCursor
                           if self.nav_mode in ("zoom", "zoom_window")
                           else Qt.OpenHandCursor)

    # ---- Copy / paste -------------------------------------------------------
    def copy_selection(self, *, publish=True) -> bool:
        return ingetrazo_render.copy_selection(self, publish)

    def _seed_proto_chunk(self, mesh, src_group) -> None:
        return ingetrazo_render.seed_proto_chunk(self, mesh, src_group)

    def _drop_clip_protos(self, clip) -> None:
        return ingetrazo_render.drop_clip_protos(self, clip)

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

    def _pick_candidates(self, oculto, lazy):
        from ingetrazo_render import pick_candidates
        return pick_candidates(self, oculto, lazy)

    def _context_placements(self):
        from ingetrazo_render import context_placements
        return context_placements(self)

    def _owner_of(self, group):
        """The object a click on ``group`` must select: a nested placement
        proxy stands for the top-level group that owns it."""
        return getattr(group, "owner", None) or group

    def _draws_in_edit_context(self, group) -> bool:
        """Whether ``group`` is part of the surroundings of the group being
        edited (so it fades or hides), rather than the subject itself."""
        ctx = self.scene.edit_group
        if ctx is None:
            return False
        if group is ctx or getattr(group, "context", None) is ctx:
            return False          # the container and what lives inside it
        return self._owner_of(group) is not ctx


    def _model_point_at(self, x: float, y: float):
        return ingetrazo_render.model_point_at(self, x, y)


    def _orbit_pivot_at(self, x: float, y: float):
        return ingetrazo_render.orbit_pivot_at(self, x, y)

    def _set_middle_drag_pan_mode(self, pan: bool) -> None:
        return ingetrazo_render.set_middle_drag_pan_mode(self, pan)

    def set_nav_mode(self, mode: Optional[str]) -> None:
        return ingetrazo_render.set_nav_mode(self, mode)

    # ---- Input --------------------------------------------------------------
    def _context_pick(self, x: float, y: float):
        from ingetrazo_render import context_pick
        return context_pick(self, x, y)

    def _face_or_image_in_front(self, x: float, y: float):
        return ingetrazo_render.face_or_image_in_front(self, x, y)

    def _dispatch_tool_click(self, ev, double: bool = False) -> None:
        return ingetrazo_render.dispatch_tool_click(self, ev, double)

    def _reassert_tool_cursor(self) -> None:
        return ingetrazo_render.reassert_tool_cursor(self)

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

    def _process_hover(self, pos, modifiers) -> None:
        return ingetrazo_render.process_hover(self, pos, modifiers)

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


    def _model_diag(self) -> float:
        return ingetrazo_render.model_diag(self)

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

    def _measurement_claims_key(self, ev, buffer) -> bool:
        return ingetrazo_render.measurement_claims_key(self, ev, buffer)

    def _tool_claims_key(self, ev) -> bool:
        return ingetrazo_render.tool_claims_key(self, ev)

    def keyPressEvent(self, ev) -> None:
        if ingetrazo_render.document_key_press(self, ev):
            return
        return super().keyPressEvent(ev)

    def _release_linear_mode(self) -> None:
        return ingetrazo_render.release_linear_mode(self)

    def _release_axis_lock_after_operation(self, committed: bool = False) -> None:
        return ingetrazo_render.release_axis_lock_after_operation(self, committed)

    def _cycle_linear_inference_mode(self) -> None:
        return ingetrazo_render.cycle_linear_inference_mode(self)

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

    def _cycle_reference_mode(self, edge=None) -> None:
        return ingetrazo_render.cycle_reference_mode(self, edge)

    def _refresh_snap(self) -> None:
        return ingetrazo_render.refresh_snap(self)

    def refresh_status_hint(self) -> None:
        return ingetrazo_render.refresh_status_hint(self)

    def _alt_tapped(self) -> bool:
        return ingetrazo_render.alt_tapped(self)

    def _apply_alt_cursor(self, down: bool) -> None:
        return ingetrazo_render.apply_alt_cursor(self, down)

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

    def _capture_shift_lock(self) -> None:
        return ingetrazo_render.capture_shift_lock(self)

    def _mark_plane_for_shift(self) -> None:
        return ingetrazo_render.mark_plane_for_shift(self)

    def _release_shift_plane_mark(self) -> None:
        return ingetrazo_render.release_shift_plane_mark(self)

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

    def _array_pending(self) -> bool:
        return ingetrazo_render.array_pending(self)

    def _set_value_buffer(self, text: str) -> None:
        return ingetrazo_render.set_value_buffer(self, text)

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
    def _snap_at(
        self, px_x: float, px_y: float, modifiers
    ) -> Optional[SnapResult]:
        return ingetrazo_render.document_snap_at(self, px_x, px_y, modifiers)

    def _build_ctx(self, ev) -> Optional[ToolContext]:
        return ingetrazo_render.build_ctx(self, ev)

    def _note_snap(self, snap: SnapResult) -> None:
        return ingetrazo_render.note_snap(self, snap)

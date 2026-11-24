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


class _PickWarm:
    """The pick index's heavy pure parts, worked out behind the document just
    opened (DocumentInteraction.start_pick_warmup): threads that touch nothing but their
    own copies of the arrays' references. What they made reaches the viewport
    in _pick_warm_take, on the GUI thread, and only if it still applies."""

    def __init__(self, mod, protos, grids, bounds=()):
        import queue
        import threading
        from tools.select_host import _local_bounds as local_bounds
        self.protos: dict = {}
        self.grids: list = []
        self.bounds: list = []
        jobs: queue.SimpleQueue = queue.SimpleQueue()
        for ckey, base in protos:
            jobs.put(("proto", ckey, base))
        for chunk in grids:
            jobs.put(("grid", chunk, {
                "v0": chunk["v0"], "e1": chunk["e1"], "e2": chunk["e2"],
                "tri_ent": chunk["tri_ent"], "rev": chunk.get("rev")}))
        for chunk in bounds:
            jobs.put(("bounds", chunk, {
                "v0": chunk["v0"], "e1": chunk["e1"], "e2": chunk["e2"],
                "edges": chunk["edges"]}))

        def work():
            while True:
                try:
                    kind, a, b = jobs.get_nowait()
                except queue.Empty:
                    with lock:
                        left[0] -= 1
                        last = left[0] == 0
                    if last:
                        # The sorting and copying leave ~1.6 GB of freed heap
                        # on a 5 M face model, which glibc keeps (#1092).
                        from core import malloc_arenas
                        malloc_arenas.trim()
                    return
                try:
                    if kind == "proto":
                        self.protos[a] = _inst_proto_of(mod, b)
                    elif kind == "grid":
                        _split_chunk_spans(b)
                        self.grids.append((a, b))
                    else:
                        local_bounds(b)
                        self.bounds.append((a, b))
                except Exception:      # noqa: BLE001 — the hover will retry
                    pass
        # Daemons: closing the window does not wait for them.
        self.threads = [threading.Thread(target=work, daemon=True)
                        for _ in range(min(8, os.cpu_count() or 2))]
        lock = threading.Lock()
        left = [len(self.threads)]
        for t in self.threads:
            t.start()

    def join(self):
        for t in self.threads:
            t.join()


def _pick_warm_take(vp) -> None:
    """Wait for what the warm-up has left and take it: its prototypes for the
    next InstanceSet, its grids for the chunks that have not moved since."""
    warm = getattr(vp, "_pick_warm", None)
    if warm is None:
        return
    vp._pick_warm = None
    warm.join()
    cache = getattr(vp, "_inst_proto_cache", None)
    if cache is None:
        cache = vp._inst_proto_cache = {}
    cache.update(warm.protos)
    for chunk, snap in warm.grids:
        if ("_span_grid" not in chunk and chunk.get("rev") == snap["rev"]
                and chunk["v0"] is snap["v0"]):
            chunk["_span_grid"] = snap["_span_grid"]
    # (Keyed on the very arrays: a chunk that has moved on just misses it.)
    for chunk, snap in warm.bounds:
        if "_box_bounds" not in chunk:
            chunk["_box_bounds"] = snap["_box_bounds"]


def _entity_owner(entities, i):
    """The owner half of pick-index entity ``i``: a lazy piece of the index
    answers it without building its faces (see views.array_bake)."""
    at = getattr(entities, "owner_at", None)
    return at(i) if at is not None else entities[i][1]


def _split_chunk_spans(chunk, depth=0, force=False):
    """``(v0, e1, e2, tri_ent, cells)`` of a chunk for the pick index. For a
    chunk of more than ``_SPAN_GRID_MIN`` triangles: the triangles reordered
    by the cell of their centroid in a grid over the chunk (cells per axis
    in proportion to its extent there, ~``_SPAN_CELL_TRIS`` a cell), and
    ``cells = [(bbox, start, count)]``, one per non-empty cell, each box
    tight around its triangles; a cell that still holds over twice the
    target (triangles cluster) is binned again on its own, up to three
    levels. Otherwise ``cells = None`` and the arrays as they are. Cached on
    the chunk by its revision.

    One span per chunk made every ray that met the box of a big group run
    Möller–Trumbore over all of it: a 107 k-triangle sofa, 2.6 ms a ray,
    ~100 rays a hover with Line (snap occlusion), 300 ms a Move step on a
    306 k-face room. The order of the triangles inside the index is free
    (each carries its entity), so the answer does not change."""
    v0, e1, e2, te = chunk["v0"], chunk["e1"], chunk["e2"], chunk["tri_ent"]
    n = len(v0)
    if _NO_SPAN_GRID or (n <= _SPAN_GRID_MIN and not force):
        return v0, e1, e2, te, None
    memo = chunk.get("_span_grid") if not force else None
    if memo is not None and memo[0] == (chunk.get("rev"), n):
        return memo[1]
    import numpy as np
    cen = v0 + (e1 + e2) / 3.0                                  # centroids
    lo = cen.min(axis=0)
    span = np.maximum(cen.max(axis=0) - lo, 1e-9)
    # Cells per axis in proportion to the extent on it; the weights multiply
    # to 1, so their product stays near the target count.
    w = span / np.exp(np.log(span).mean())
    per_axis = np.maximum(
        1, np.round((n / _SPAN_CELL_TRIS) ** (1.0 / 3.0) * w)).astype(np.int64)
    cell = np.minimum(((cen - lo) * (per_axis / span)).astype(np.int64),
                      per_axis - 1)
    key = (cell[:, 0] * per_axis[1] + cell[:, 1]) * per_axis[2] + cell[:, 2]
    order = np.argsort(key, kind="stable")
    key = key[order]
    v0p = np.ascontiguousarray(v0[order])
    e1p = np.ascontiguousarray(e1[order])
    e2p = np.ascontiguousarray(e2[order])
    tep = np.ascontiguousarray(te[order])
    starts = np.flatnonzero(np.r_[True, key[1:] != key[:-1]])
    ends = np.r_[starts[1:], len(key)]
    cells = []
    for s0, s1 in zip(starts.tolist(), ends.tolist()):
        if s1 - s0 > 2 * _SPAN_CELL_TRIS and depth < 3:
            sv0, se1, se2, ste, sub = _split_chunk_spans(
                {"v0": v0p[s0:s1], "e1": e1p[s0:s1], "e2": e2p[s0:s1],
                 "tri_ent": tep[s0:s1]}, depth + 1, force=True)
            # The sub-grid is a permutation of this range: write it back.
            v0p[s0:s1], e1p[s0:s1], e2p[s0:s1], tep[s0:s1] = sv0, se1, se2, ste
            cells += [(bb, s0 + a, c) for bb, a, c in sub]
            continue
        pts = np.concatenate([v0p[s0:s1], v0p[s0:s1] + e1p[s0:s1],
                              v0p[s0:s1] + e2p[s0:s1]])
        cells.append(((tuple(pts.min(axis=0).tolist()),
                       tuple(pts.max(axis=0).tolist())), s0, s1 - s0))
    out = (v0p, e1p, e2p, tep, cells)
    if not force:
        chunk["_span_grid"] = ((chunk.get("rev"), n), out)
    return out


#: Every chunk entry built (or loaded from disk) takes the next number.
#: Keys that told entries apart by ``id(entry)`` could not: the dict a
#: rebuild replaces is freed at once, and the new one may land at the same
#: address — same id, same rev 0 — so the instance arrays and the GL draw
#: entry derived from the OLD chunk were served as if nothing had changed
#: (a face painted inside a component came back unpainted one level up,
#: Marco, 2026-09-11).
_chunk_uid = itertools.count(1)


def _pv_hole(raw):
    """Zeros the size of ``raw``: a moving group's place in the static
    buffers while its preview draws it elsewhere. All-zero triangles and
    lines have no area and no length, so they draw nothing in any pass."""
    return bytes(len(raw))


_ZEROS: dict = {}


def _zeros(n: int) -> bytes:
    """``n`` zero bytes, the same object for the same size: a gone
    object's place (``_static_slots``) is passed over by identity on the
    next upload, not compared byte by byte."""
    z = _ZEROS.get(n)
    if z is None:
        if len(_ZEROS) > 256:
            _ZEROS.clear()
        z = _ZEROS[n] = bytes(n)
    return z


class _Slot:
    """One object's place in the static buffers (``_static_slots``): the
    object, and the byte sizes it took the last time it was laid out, per
    buffer — what its zeros take once it is gone."""

    __slots__ = ("g", "sizes")

    def __init__(self, g) -> None:
        self.g = g
        self.sizes: dict = {}


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


#: The recorded, state-tracking instanced passes: ``None`` follows the
#: experimental engine (on with it, off without — they change how the GPU
#: is fed, and drivers differ); True/False force them (tests draw both).
_LEAN_INSTANCED = None


def _lean_instanced() -> bool:
    if _LEAN_INSTANCED is not None:
        return _LEAN_INSTANCED
    from core import _native
    return _native.native is not None
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
#: Recorded instanced passes replayed by the native engine when it is on.
class _InstanceBatch(list):
    """One frame's placements of an instanced pool (a plain list of them)
    with their indices in the pool and the pool's matrix rows, so the
    matrix buffer is filled by one NumPy take instead of sixteen floats
    read per placement."""

    __slots__ = ("idx", "mats", "draw_bucket")


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
    """The shared store of ``scene``, made on first use; ``None`` for a
    stand-in that cannot be weakly referenced."""
    key = id(scene)
    hit = _BAKE_STORES.get(key)
    if hit is not None and hit[0]() is scene:
        return hit[1]
    try:
        ref = weakref.ref(scene)
    except TypeError:
        return None
    store: dict = {}
    _BAKE_STORES[key] = (ref, store)
    weakref.finalize(scene, _drop_bake_store, key, ref)
    return store


def _drop_bake_store(key, ref) -> None:
    hit = _BAKE_STORES.get(key)
    if hit is not None and hit[0] is ref:
        del _BAKE_STORES[key]


def _bind_bake_store(vp) -> None:
    """Point ``vp``'s shared caches at its scene's store (made on first
    use); another scene, another store. A stub without a scene keeps
    dicts of its own."""
    store = _bake_store_of(getattr(vp, "scene", None))
    if store is None:
        store = {}
    for name in _SHARED_BAKES:
        setattr(vp, name, store.setdefault(name, {}))
    try:
        _LIVE_VIEWS.add(vp)
    except TypeError:
        pass


#: Every viewport made, weakly: the decoded textures are kept only while
#: more than one of them is on screen (:func:`_several_views_shown`).
_LIVE_VIEWS: "weakref.WeakSet" = weakref.WeakSet()


def _several_views_shown() -> bool:
    shown = 0
    for vp in list(_LIVE_VIEWS):
        try:
            if vp.isVisible():
                shown += 1
        except RuntimeError:              # its C++ side already gone
            continue
        if shown > 1:
            return True
    return False


#: Texture images decoded once per file, for every GL context that
#: uploads them: ``path`` → ``(mtime, size, mirrored QImage, cutout)``,
#: the newest last, held to a budget of bytes. Three views joining the
#: plaza's decoded its textures again each (1.6 s of the 3.8 s); the GL
#: texture itself belongs to each context and is made from this.
_DECODED_TEXTURES: dict = {}
_DECODED_BUDGET = 256 * 1024 * 1024


def _decoded_texture(path: str):
    """``(QImage mirrored for GL, cutout)`` of the image at ``path`` — the
    file's current version — or ``None`` when it cannot be loaded.
    ``cutout``: a foliage card of low alpha coverage (mip-averaged alpha
    plus atlas bleed erase or darken the leaves at distance, and the Bayer
    dither turns the rest into ghost speckle: the textured passes sample
    the top level only and cut hard at 0.5)."""
    import os
    try:
        st = os.stat(path)
        stamp = (st.st_mtime_ns, st.st_size)
    except OSError:
        stamp = None
    hit = _DECODED_TEXTURES.get(path)
    if hit is not None and hit[0] == stamp:
        _DECODED_TEXTURES[path] = _DECODED_TEXTURES.pop(path)   # newest last
        return hit[1]
    img = QImage(path)
    if img.isNull():
        _DECODED_TEXTURES.pop(path, None)
        return None
    cutout = False
    if img.hasAlphaChannel():
        small = img.scaled(64, 64).convertToFormat(QImage.Format_Alpha8)
        import numpy as _np
        al = _np.frombuffer(small.constBits(), _np.uint8,
                            count=small.height() * small.bytesPerLine())
        cutout = (float((al > 128).mean()) if len(al) else 1.0) < 0.6
    out = (img.mirrored(), cutout)          # OBJ/.skp V is bottom-up
    _DECODED_TEXTURES.pop(path, None)
    if not _several_views_shown():
        # One view uploads each texture once: keeping the decoded images
        # only held their memory — ~65 MB on the plaza, all the time
        # (release check, 14-11). A view that joins decodes again, once,
        # and from then on the views share the decodings.
        _DECODED_TEXTURES.clear()
        return out
    _DECODED_TEXTURES[path] = (stamp, out)
    total = sum(v[1][0].sizeInBytes() for v in _DECODED_TEXTURES.values())
    for old in list(_DECODED_TEXTURES):
        if total <= _DECODED_BUDGET or old == path:
            break
        total -= _DECODED_TEXTURES.pop(old)[1][0].sizeInBytes()
    return out


def _array_bake():
    """views.array_bake, which imports back here (its import statement ran
    per placement, per frame pass)."""
    return sys.modules.get("views.array_bake") or importlib.import_module(
        "views.array_bake")


def _proto_wrapper(vp, mesh, paint=None):
    """The stable stand-in a prototype mesh is baked through, one per
    (mesh, container paint): ``_group_chunk`` caches by the wrapper's id,
    so the same object must come back for the same mesh — and a painted
    instance (issue #47) gets a wrapper of its own. Module-level so the
    stub viewports in tests reach it too."""
    wrappers = getattr(vp, "_proto_wrappers", None)
    if wrappers is None:
        wrappers = vp._proto_wrappers = {}
    wkey = (id(mesh), _material_sig(paint))
    w = wrappers.get(wkey)
    if w is None:
        w = wrappers[wkey] = types.SimpleNamespace(mesh=mesh, xform=None,
                                                   material=paint)
    else:
        w.material = paint
    return w


def _fp_winding(fp) -> int:
    """The flip count a chunk fingerprint was taken at, 0 when it has no such
    term (see ``_mesh_fingerprint``). Module-level for the same reason as
    :func:`_proto_wrapper`."""
    return fp[7][1] if len(fp) > 7 and fp[7][0] == "winding" else 0


def _flags_read(mesh, entry: dict) -> None:
    """``entry`` has answered for what the mesh's dirty flags announced: they
    clear, and any other bake of this mesh is left behind (``flags_gen``)."""
    if getattr(mesh, "_chunk_dirty", True) or getattr(mesh, "_attrs_dirty", False):
        mesh._flags_gen = getattr(mesh, "_flags_gen", 0) + 1
    mesh._chunk_dirty = False
    mesh._attrs_dirty = False
    entry["flags_gen"] = getattr(mesh, "_flags_gen", 0)


#: flat look.
_BY_LAYER_MODES = ("textures", "shaded", "xray")


def _layer_look_sig(scene, style=None) -> tuple:
    """What of the layers changes the paint objects are drawn in: Colour by
    layer, and each layer's colour and transparency — and the style's Front
    color, which dresses the faces that wear no material. ``style``: the one
    the view draws with (a view of several has its own); else the scene's."""
    if style is None:
        style = getattr(scene, "display_style", None)
    by_layer = bool(getattr(style, "color_by_layer", False))
    return (by_layer, getattr(style, "face_mode", None) if by_layer else None,
            tuple((ly.key, ly.color, ly.opacity)
                  for ly in getattr(scene, "layers", ())),
            _front_tint(style))


#: The face modes that draw the per-vertex colour, where the style's Front
#: color reaches the default faces through their bake. Hidden line and
#: Monochrome paint every face flat in it with a uniform instead.
_VCOLOR_MODES = ("textures", "shaded", "xray")


def _front_tint(style):
    """The colour the faces wearing no material bake to under ``style``
    when its Front color is not the stock white, else ``None`` (the
    warm-white default, unchanged). The default is TINTED by the pick
    rather than replaced, so the stock white keeps the stock look and a
    picked colour shows as itself (issue #776: the Front color only
    showed in Hidden line and Monochrome)."""
    if style is None or getattr(style, "face_mode", None) not in _VCOLOR_MODES:
        return None
    front = tuple(float(c) for c in
                  (getattr(style, "front_color", None) or (1.0, 1.0, 1.0))[:3])
    if all(c >= 0.999 for c in front):
        return None
    from core.style import DEFAULT_FACE_COLOR
    return tuple(round(d * c, 4) for d, c in zip(DEFAULT_FACE_COLOR, front))


def _with_front_tint(paint, style):
    """``paint`` with the style's Front color standing in for the default
    material when there is no container paint: a front-only paint, so the
    back faces keep the back tint."""
    if paint:
        return paint
    tint = _front_tint(style)
    if tint is None:
        return paint
    return {"color": tint, "front_only": True}


def _paint_of(vp, group):
    """The container paint faces are baked with: the object's own paint
    (issue #47), or — with Colour by layer on — the layer colours
    (core.layers.layer_paint). ``group`` None is the loose model; a
    prototype stand-in (``_proto_wrapper``) already carries its paint.
    Module-level, like ``_proto_wrapper``, so stub viewports reach it.
    (Its names are the module's: asked 240 000 times to open the plant of
    #158, an import statement each time was a second of it.)"""
    if group is not None and not isinstance(group, Group):
        return effective_material(group)
    scene = getattr(vp, "scene", None)
    worn = effective_material(group)
    if group is not None and group is getattr(scene, "edit_group", None):
        # Open, its faces draw as the loose mesh and keep what they wore
        # closed: the nearest painted container's, not only its own (#1101).
        worn = scene.open_group_paint()
    if scene is None or getattr(vp, "plano_style", None) is not None:
        return worn                            # a sheet's technical look
    style = (getattr(vp, "style_override", None)
             or getattr(vp, "view_style", None)
             or getattr(scene, "display_style", None))
    tint = (getattr(vp, 'view_tint', None) or {}).get(getattr(group, 'uid', None))
    if tint is not None and getattr(style, 'face_mode', None) in _BY_LAYER_MODES:
        return {'by_layer': {}, 'color': tint}
    by_layer = bool(getattr(style, "color_by_layer", False)
                    and style.face_mode in _BY_LAYER_MODES)
    memo = getattr(vp, "_layer_paints", None)
    if memo is None or memo[0] != scene.version:
        memo = vp._layer_paints = (scene.version, {}, _layers.layer_fx(scene))
    fx = memo[2]
    if not by_layer and not fx:
        return _with_front_tint(worn, style)
    own = None if by_layer else _with_front_tint(worn, style)
    key = (by_layer, _layers.object_layer(group), _material_sig(own))
    paint = memo[1].get(key)
    if paint is None:
        base = _layers.layer_paint(scene, group) if by_layer else own
        paint = memo[1][key] = _layers.layer_look(scene, group, base, fx)
    return paint


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
    """Binary64 unit directions and distances, in model vector operation order."""
    import numpy as np
    if not _qv_hypot3():
        return None
    delta = np.asarray(pts, dtype=np.float64).reshape(-1, 3) - eye.toTuple()
    x, y, z = delta.T
    lengths = np.sqrt((x*x + y*y) + z*z)
    rows = np.flatnonzero(lengths >= 1e-9)
    return rows, delta[rows] / lengths[rows, None], lengths[rows]


# Barycentric roundoff on a shared triangle edge can reject both faces.
_BARY_EPS = 1e-14


def _occlusion_block(o, d, v0, e1, e2):
    """``DocumentInteraction._occluded_many``'s NumPy block: the nearest accepted hit
    of each ray ``d`` (r,3) from ``o`` on the triangles (inf: none)."""
    import numpy as np
    if not len(v0):
        return np.full(len(d), np.inf)
    s = o - v0
    q = np.cross(s, e1)
    uq = np.einsum("ij,ij->i", e2, q)
    p = np.cross(d[:, None, :], e2[None, :, :])
    det = np.einsum("nj,rnj->rn", e1, p)
    ok = np.abs(det) > 1e-6
    inv = np.where(ok, 1.0 / np.where(ok, det, 1.0), 0.0)
    u = np.einsum("nj,rnj->rn", s, p) * inv
    v = (d @ q.T) * inv
    t = uq[None, :] * inv
    hit = (ok & (u >= -_BARY_EPS) & (u <= 1.0 + _BARY_EPS) & (v >= -_BARY_EPS) & (u + v <= 1.0 + _BARY_EPS)
           & (t > 1e-6))
    return np.where(hit, t, np.inf).min(axis=1)


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
    """The world triangles, hard and soft edges (with normals) of every
    copy of prototype chunk ``base`` placed by the matrices ``xfs``, added
    to the lists :meth:`DocumentInteraction.hlr_geometry_now` joins. The maps of
    :func:`_placed_maps`, term by term, over all the copies at once."""
    import numpy as np
    m = np.stack([np.array(x.data(), dtype=np.float64).reshape(4, 4, order="F")
                  for x in xfs])
    L, t = m[:, :3, :3], m[:, :3, 3]            # (k, 3, 3), (k, 3)

    def tv(a):                                   # (n, 3) → (k, n, 3)
        return (a[None, :, 0:1] * L[:, None, :, 0]
                + a[None, :, 1:2] * L[:, None, :, 1]
                + a[None, :, 2:3] * L[:, None, :, 2])

    def tp(a):
        return tv(a) + t[:, None, :]

    v0 = base.get("v0")
    if v0 is not None and len(v0):
        w0 = tp(np.asarray(v0, dtype=np.float64))
        w1 = tv(np.asarray(base["e1"], dtype=np.float64))
        w2 = tv(np.asarray(base["e2"], dtype=np.float64))
        tri_parts.append(np.stack([w0, w0 + w1, w0 + w2],
                                  axis=2).reshape(-1, 3, 3))
    if base.get("edges"):
        a = np.frombuffer(base["edges"], np.float32).reshape(-1, 3)
        we = tp(a.astype(np.float64)).astype(np.float32)
        hard_parts.append(we.reshape(-1, 2, 3).astype(np.float64))
    sp = base.get("soft_pts")
    if sp is not None and len(sp):
        s6 = np.asarray(sp).reshape(-1, 6).astype(np.float64)
        a0 = tp(s6[:, 0:3]).astype(np.float32).astype(np.float64)
        a1 = tp(s6[:, 3:6]).astype(np.float32).astype(np.float64)
        soft_parts.append(np.stack([a0, a1], axis=2).reshape(-1, 2, 3))
        inv = np.empty_like(L)
        for i in range(len(L)):
            try:
                inv[i] = np.linalg.inv(L[i])
            except np.linalg.LinAlgError:
                inv[i] = np.eye(3)

        def tn(a):                                # unit normals
            a = np.asarray(a, dtype=np.float64)
            out = np.matmul(a[None], inv)         # a @ inv(L) = a @ n_mat.T
            ln = np.linalg.norm(out, axis=2, keepdims=True)
            return out / np.where(ln > 1e-12, ln, 1.0)
        n0 = tn(base["soft_n0"])
        n1 = np.where(np.asarray(base["soft_single"])[None, :, None],
                      np.nan, tn(base["soft_n1"]))
        n_parts.append(np.stack([n0, n1], axis=2).reshape(-1, 2, 3))


def _cat_kept(vp, slot, parts):
    """``np.concatenate(parts)``, the same array again while the parts are
    the same objects: the pick index is rebuilt on each change of its lazy
    set (#158), and joined its static triangles anew — 1.4 million rows,
    ~100 MB, a few times a pointer move over the plant of #158. Nothing
    writes into these arrays."""
    import numpy as np
    memo = getattr(vp, "_pick_cat_memo", None)
    if memo is None:
        memo = {}
        try:
            vp._pick_cat_memo = memo
        except AttributeError:
            pass
    m = memo.get(slot)
    if m is not None and len(m[0]) == len(parts) and all(
            a is b for a, b in zip(m[0], parts)):
        return m[1]
    out = parts[0] if len(parts) == 1 else np.concatenate(parts)
    memo[slot] = (list(parts), out)
    return out


def _offset_kept(vp, te, offset):
    """``te + offset``, the same array again for the same ``te`` and
    offset (see :func:`_cat_kept`)."""
    if not offset:
        return te
    m = getattr(vp, "_pick_te_memo", None)
    if m is not None and m[0] is te and m[1] == offset:
        return m[2]
    out = te + offset
    try:
        vp._pick_te_memo = (te, offset, out)
    except AttributeError:
        pass
    return out


def _slab_far(t):
    """``nanmax`` over the last (x, y, z) axis of a slab test, column by
    column: a reduction along an axis of three was ~4x slower, and the
    lazy pick index runs one per ray query (25 a mouse move with Line over
    the plant of #158)."""
    import numpy as np
    return np.fmax(np.fmax(t[..., 0], t[..., 1]), t[..., 2])


def _slab_near(t):
    """``nanmin`` twin of :func:`_slab_far`."""
    import numpy as np
    return np.fmin(np.fmin(t[..., 0], t[..., 1]), t[..., 2])


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
    ax, ay = a
    bx, by = b
    px, py = p
    dx = bx - ax
    dy = by - ay
    if dx == 0.0 and dy == 0.0:
        return math.hypot(px - ax, py - ay)
    t = ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)
    t = max(0.0, min(1.0, t))
    qx = ax + t * dx
    qy = ay + t * dy
    return math.hypot(px - qx, py - qy)


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
    """``(key, normal, area, triangles, centroid)`` of a mesh face, from
    ``memo`` when every corner (holes included) is exactly where it was
    (``memo`` None: computed, not remembered).
    Triangles are ``((x, y, z), …)`` float tuples — copies, never the live
    vertex positions, which edits move in place."""
    loop = [v.position for v in f.loop]
    holes = [[v.position for v in h] for h in f.hole_loops]
    if memo is None:
        key = None                  # a closed group: nothing to remember
    else:
        key = (tuple((p.x(), p.y(), p.z()) for p in loop),
               tuple(tuple((p.x(), p.y(), p.z()) for p in h) for h in holes))
        hit = memo.get(element_identity(f))
        if hit is not None and hit[0] == key:
            return hit
    if len(loop) < 3:
        geom = (key, QVector3D(0.0, 0.0, 1.0), 0.0, [], None, {}, (), None)
    else:
        n_raw = f._newell()
        ln = n_raw.length()
        normal = n_raw / ln if ln > 1e-9 else QVector3D(0.0, 0.0, 1.0)
        tris = [tuple((t.x(), t.y(), t.z()) for t in tri)
                for tri in triangulate(loop, holes, normal)]
        c = f.centroid()
        flat = tuple(x for tri in tris for corner in tri for x in corner)
        props = ((normal.x(), normal.y(), normal.z()), (c.x(), c.y(), c.z()))
        # 5: shaded colour per base colour · 6: triangles as 9 floats each
        # · 7: the soft-edge silhouette props (normal, centroid)
        geom = (key, normal, 0.5 * ln, tris, c, {}, flat, props)
    if memo is not None:
        memo[element_identity(f)] = geom
    return geom


def _finite_point(p) -> bool:
    """Whether ``p`` (a QVector3D or None) is a real point."""
    import math
    return p is not None and all(
        math.isfinite(c) for c in (p.x(), p.y(), p.z()))


def _curved_faces(mesh) -> set:
    """The faces of ``mesh`` on a curved surface: those a soft edge that
    smooths (not ``flat``) joins to another face. Only these can have a smooth corner, so
    every other face keeps its one flat colour and costs nothing."""
    out: set = set()
    for e in mesh.edges:
        if smooths(e) and len(e.faces) > 1:
            out.update(e.faces)
    return out


def _smooth_vcol(face, base, flat_rgb, smoother, tris, light) -> list:
    """``pos(3) + rgb(3)`` floats of ``face``'s triangles (corners as
    float tuples), each corner on a curved surface shaded against
    ``light`` by the SURFACE's normal there
    (:class:`formats.meshexport.SmoothNormals`, the one the exports
    write) instead of the face's own: the vertex colours blend across
    the triangles and a sphere reads round, not faceted (#131). Hard
    edges split the surface, so a cylinder keeps its crease against its
    caps.

    Unquantised, unlike ``DocumentInteraction._shaded_color``: the face pass is one
    draw call whatever its colours, and the 1/32 shade steps would show
    as rings across a sphere. A corner whose surface normal IS the
    face's (coplanar faces joined by a soft diagonal) keeps the flat
    colour, so a flat patch never shows a seam against its neighbour."""
    corners = smoother.corners(face)
    flat = smoother.flat(face)
    rgb_at: dict = {}
    out: list = []
    for tri in tris:
        for p in tri:
            rgb = rgb_at.get(p)
            if rgb is None:
                n = corners.get(p)
                if n is None or QVector3D.dotProduct(n, flat) > 0.99999:
                    rgb = flat_rgb
                else:
                    d = abs(QVector3D.dotProduct(n, light))
                    s = 0.62 + 0.38 * d
                    rgb = (min(1.0, base[0] * s), min(1.0, base[1] * s),
                           min(1.0, base[2] * s))
                rgb_at[p] = rgb
            out.extend((*p, *rgb))
    return out


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
        """The topmost reference image under the cursor, or ``None``.

        Casts the pixel ray at each image's plane and keeps the hit whose
        ``(u, v)`` falls inside the rectangle, nearest first. Later images win
        ties, matching the order they are painted in.

        ``selectable_only`` is what separates the two callers. Picking honours
        the image's own lock; the work-plane inference does NOT — a locked
        scan is exactly the one you have finished aligning and now want to
        draw on, so it must keep offering its plane while refusing clicks.
        """
        images = getattr(self.scene, "image_planes", None)
        if not images:
            return None
        origin, direction = self._pixel_to_ray(screen_x, screen_y)
        if origin is None or direction is None:
            return None
        best, best_t = None, float("inf")
        for im in images:
            if not self.scene.entity_visible(im):
                continue
            if selectable_only and (getattr(im, "locked", False)
                                    or not self.scene.entity_selectable(im)):
                continue
            n = im.normal()
            denom = QVector3D.dotProduct(n, direction)
            if abs(denom) < 1e-9:
                continue
            t = QVector3D.dotProduct(n, im.center() - origin) / denom
            if t < 0.0 or t > best_t:
                continue
            fu, fv = im.project(origin + direction * t)
            if im.contains_uv(fu, fv):
                best, best_t = im, t
        return best

    def pick_image_plane(self, screen_x: float, screen_y: float):
        """Selection entry point: like :meth:`image_plane_at` but skipping
        locked images, so a click on an aligned scan falls through to the
        geometry being drawn on it."""
        return self.image_plane_at(screen_x, screen_y, selectable_only=True)

    def image_in_front_of(self, face, screen_x: float, screen_y: float):
        """A selectable reference image under the cursor that lies strictly
        NEARER than ``face`` along the pixel ray, or ``None``. An image is
        an underlay — a face on its plane, or above it, still wins the
        click — but one behind it must not: with the top of a box hidden,
        its bottom took every click meant for the image lying on top
        (#426)."""
        image = self.pick_image_plane(screen_x, screen_y)
        if image is None or face is None:
            return None
        origin, direction = self._pixel_to_ray(screen_x, screen_y)
        if origin is None or direction is None:
            return None
        verts = getattr(face, "vertices", None) or []
        if not verts:
            return None
        n = face.normal()
        denom = QVector3D.dotProduct(n, direction)
        if abs(denom) < 1e-9:
            return None
        t_face = QVector3D.dotProduct(n, verts[0] - origin) / denom
        ni = image.normal()
        di = QVector3D.dotProduct(ni, direction)
        if abs(di) < 1e-9:
            return None
        t_img = QVector3D.dotProduct(ni, image.center() - origin) / di
        if 0.0 <= t_img < t_face - max(1e-4, 1e-5 * abs(t_face)):
            return image
        return None

    def pick_locked_image_plane(self, screen_x: float, screen_y: float):
        """The right-click's last resort: a LOCKED image under the cursor
        (its layer still visible and unlocked). A locked image refuses
        clicks by design, but the context menu is the only door back to
        Unlock / Delete — the usual place to offer Unlock.
        Without this a locked scan could never be selected again (Marco,
        2026-09-14, the orthomosaic that arrives locked)."""
        images = getattr(self.scene, "image_planes", None)
        if not images:
            return None
        origin, direction = self._pixel_to_ray(screen_x, screen_y)
        if origin is None or direction is None:
            return None
        best, best_t = None, float("inf")
        for im in images:
            if not getattr(im, "locked", False):
                continue
            if not self.scene.entity_visible(im) or not self.scene.entity_selectable(im):
                continue
            n = im.normal()
            denom = QVector3D.dotProduct(n, direction)
            if abs(denom) < 1e-9:
                continue
            t = QVector3D.dotProduct(n, im.center() - origin) / denom
            if t < 0.0 or t > best_t:
                continue
            fu, fv = im.project(origin + direction * t)
            if im.contains_uv(fu, fv):
                best, best_t = im, t
        return best

    def pick_locked_group(self, screen_x: float, screen_y: float):
        """The right-click's door back to a LOCKED group or component: the
        one whose face is the front-most thing under the cursor (drawn,
        its layer visible and unlocked), or ``None``. A locked group
        refuses every click by design, so the context menu is the only
        place left to offer Unlock (#782)."""
        origin, direction = self._pixel_to_ray(screen_x, screen_y)
        if origin is None or direction is None:
            return None
        import numpy as np
        idx = self._pick_index(near=DocumentInteraction._ray_near(self, screen_x, screen_y))
        if idx is None:
            return None
        hits = DocumentInteraction._face_hits(self, idx, origin, direction, "vis")
        ingetrazo_core.drop_past_cut(self, hits, origin, direction)
        if hits is None or not len(hits.t):
            return None
        front = DocumentInteraction._smallest_at_front(self, hits, origin, direction)
        if not len(front):
            return None
        owner = hits.owner(int(front[0]))
        for g in (owner, getattr(owner, "owner", None)):
            if (g is not None and getattr(g, "locked", False)
                    and hasattr(g, "children")):
                visible, layer_locked = self.scene._layer_state(g)
                if (visible and not layer_locked
                        and self.scene.entity_visible(g)):
                    return g
                return None
        return None

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

    def _texture_avg_color(self, path: str) -> tuple[float, float, float]:
        """Average colour of a texture image — the "Shaded" face style
        shows the material COLOUR instead of its texture. Cached per path."""
        cache = getattr(self, "_tex_avg_cache", None)
        if cache is None:
            cache = self._tex_avg_cache = {}
        c = cache.get(path)
        if c is None:
            from core.vista_caras import texture_average
            c = texture_average(path, cache) or (0.78, 0.78, 0.78)
            cache[path] = c
        return c

    #: Default back-face colour, the classic blue-grey: a visible back face
    #: means "you are looking at the inside" (or at a genuinely inverted
    #: face) — honest feedback the winding-proof shading used to hide. The
    #: active style's Back color overrides it (core/style.py).
    BACK_FACE_COLOR = DEFAULT_BACK_COLOR


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
        """Diffuse factor (0.62..1.0) of a face normal against the fixed
        world light — quantised to 1/32 so it can key texture draw runs
        without exploding the run count on curved surfaces."""
        if normal.length() < 1e-9:
            return 0.84375
        # abs(): shading depends on the face's plane, not its winding — a
        # flat plan whose faces happen to wind downward still reads bright,
        # while a solid keeps its top-bright / sides-toned maquette look.
        # The 0.62..1.0 range matches the classic default-style contrast
        # (measured on user models: its shaded walls sit near 0.65 of the
        # lit tone) — the previous 0.80..1.0 read as "unshaded"/wrong
        # colour next to a reference render of the same building.
        d = abs(QVector3D.dotProduct(normal.normalized(), self._LIGHT))
        return round((0.62 + 0.38 * d) * 32.0) / 32.0

    def _shaded_color(self, base, normal):
        """Multiply ``base`` RGB by a subtle diffuse term from the face normal vs
        the fixed world light — the matte-model shading. Returns a clamped RGB
        tuple used as the render key (identical normals/colours group together)."""
        shade = self._shade_factor(normal)
        # Quantise to 1/64 steps: the tuple is the DRAW-RUN key, and a model
        # with thousands of distinct normals (imported trees, curved detail)
        # otherwise explodes into one draw call per unique shade — 63k draw
        # calls per frame on a real 100k-face project. 1/64 banding is
        # invisible; the run count collapses to a few hundred.
        return (round(min(1.0, base[0] * shade) * 64.0) / 64.0,
                round(min(1.0, base[1] * shade) * 64.0) / 64.0,
                round(min(1.0, base[2] * shade) * 64.0) / 64.0)

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
        """True when a georeferenced base map is currently visible."""
        layer = getattr(self.scene, "tile_layer", None)
        return (layer is not None and getattr(layer, "visible", False)
                and getattr(self.scene, "georef", None) is not None)

    def _ensure_tile_fetcher(self):
        """Create the tile fetcher on first use (needs a running app)."""
        if self._tile_fetcher is None:
            from georef.tile_fetcher import TileFetcher
            self._tile_fetcher = TileFetcher(parent=self)
            self._tile_fetcher.tileReady.connect(self._on_tile_ready)
        return self._tile_fetcher

    def _on_tile_ready(self, source_id, x, y, z, image) -> None:
        """A downloaded tile arrived: stash its image and schedule a repaint."""
        layer = getattr(self.scene, "tile_layer", None)
        if (layer is not None and layer.source.id == source_id
                and z == layer.zoom):
            layer.images[(x, y, z)] = image
            self.tilesChanged.emit()
            self.update()

    def _disk_cached_tile(self, source, x, y, z):
        """Tile ``(x, y, z)`` of ``source`` from the on-disk cache, or None.
        The same cache the fetcher fills, read without the fetcher: safe
        inside paintGL, where the fetcher's network manager must not be
        created (#327)."""
        if z < 0 or z > source.max_zoom:
            return None
        if self._tile_fetcher is not None:
            cache = self._tile_fetcher.cache
        else:
            cache = getattr(self, "_tile_disk_cache", None)
            if cache is None:
                from georef.tiles import TileCache, default_cache_dir
                cache = self._tile_disk_cache = TileCache(default_cache_dir())
        data = cache.get(source.id, x, y, z)
        if data is None:
            return None
        from ingetrazo_render import SurveyRaster
        return SurveyRaster.decode(data)

    def _schedule_tile_requests(self) -> None:
        """Start missing tile requests after the current GL paint returns."""
        if self._tile_requests_scheduled:
            return
        self._tile_requests_scheduled = True
        if NATIVE:
            later(self, 0, self._request_missing_tiles)
        else:
            QTimer.singleShot(0, self._request_missing_tiles)

    def _request_missing_tiles(self) -> None:
        """Fetch the current layer's missing images outside ``paintGL``.

        Cache hits are copied into the layer immediately and repaint once;
        network misses are delivered later by :meth:`_on_tile_ready`.
        Reading the current scene here also makes a callback queued just before
        a source/location change harmless.
        """
        self._tile_requests_scheduled = False
        layer = getattr(self.scene, "tile_layer", None)
        datum = getattr(self.scene, "georef", None)
        if layer is None or datum is None or not getattr(layer, "visible", False):
            return
        try:
            tiles = layer.flat_tiles(datum)
        except Exception:
            return
        fetcher = self._ensure_tile_fetcher()
        cached = False
        for x, y in tiles:
            key = (x, y, layer.zoom)
            if key in layer.images:
                continue
            image = fetcher.request(layer.source, x, y, layer.zoom)
            if image is not None:
                layer.images[key] = image
                cached = True
        if cached:
            self.update()

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
        """Forget the previous document's chunks at the document boundary.

        ``keep_bakes``: only the GL side goes — a viewport moving to another
        parent gets a new context and must upload again, but its scene's
        shared bakes (:data:`_SHARED_BAKES`, CPU arrays) are as good as
        before, and the views that join it in the grid draw from them.

        The chunk caches are keyed by ``id()`` of groups and meshes and were
        never emptied: every New / Open kept the old document's chunks — and,
        through their pick arrays, its Face objects and meshes — alive. The
        release check measured it on the Plaza Yanque (25-09-2026): five
        reopenings of the same document, 617 → 1277 MB, 887 → 3219 chunks,
        71 000 → 135 000 live faces for a 71 000-face model. Opening a
        document also slowed down reopen after reopen (the garbage collector
        walking the dead ones). And a CPython id() reused by a new group
        could meet an old entry — the composer's lesson with frames."""
        self.view_filter = None
        self.view_tint = None
        self.active_view_key = None
        self._previous_views = []
        self._view_expression = None
        self._view_color = None
        self._view_isolate = True
        self._view_exact = False
        self.scene._navigator_allowed = None
        self.scene._navigator_view_key = None
        self._pick_warm = None               # its threads end on their own
        self._inst_obb = None
        self._pick_warm_due = False
        for name in self._DOCUMENT_CACHES:
            if keep_bakes and name in _SHARED_BAKES:
                continue
            cache = getattr(self, name, None)
            if isinstance(cache, dict):
                cache.clear()
        self._edges_version = -1          # rebuild the VBOs from nothing
        self._frozen_cache_version = None
        self._selected_placements_cache = None
        self._selected_profile_key = None
        self._sil_table = None            # holds the old document's bakes
        self._pick_cat_memo = None        # ...and its pick triangles
        self._pick_te_memo = None
        self._pick_loose_memo = None
        self._lazy_native_memo = None
        self._static_occl_memo = None
        self._sil_groups = None
        self._billboard_groups = None
        self._placements_memo = None
        self._epoch_same = None
        self._pick_lazy_memo = None
        self._pick_near_memo = self._edge_near_memo = None
        self._proto_draw_fast = None
        self._arc_mid_cache = None
        self._epoch_hist = None           # ids of the old document's objects
        self._kept_tops_memo = None
        self._slot_layout = None          # holds its gone objects






    def reset_tiles(self) -> None:
        """Discard tiles for the previous capture and cancel its pending requests."""
        if self._tile_fetcher is not None:
            self._tile_fetcher.cancel_all()
        layer = getattr(self.scene, "tile_layer", None)
        if layer is not None:
            layer.images.clear()
        self.update()

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
        t = getattr(self.scene, "terrain", None)
        return t is not None and getattr(t, "visible", False)

    def prefetch_tiles(self, source, tile_list, zoom) -> None:
        """Request the given tiles so their images populate ``tile_layer.images``
        (used to build the 3D terrain mosaic even when the flat map is hidden)."""
        layer = getattr(self.scene, "tile_layer", None)
        if layer is None:
            return
        fetcher = self._ensure_tile_fetcher()
        for (x, y) in tile_list:
            img = fetcher.request(source, x, y, zoom)
            if img is not None:
                layer.images[(x, y, zoom)] = img



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
        """Corner colours of the backdrop gradient: the style's sky tone at
        the zenith and ground tone underfoot, both hazed toward white at the
        horizon (atmosphere — the one-colour input stays the one the user
        picked). Computed against the UNCLAMPED horizon ``hy``, so a pitched
        camera sees the right slice of the ramp (all zenith when looking up,
        all ground when looking down). Returns ``(sky_bottom, sky_top,
        ground_top, ground_bottom, horizon_line)``; the ramps are linear in
        NDC y, so colouring only the quad corners is exact."""
        def mix(a, b, t):
            return (a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t)

        white = (1.0, 1.0, 1.0)
        sky_h = mix(sky_rgb, white, 0.55)
        gnd_h = mix(ground_rgb, white, 0.30)

        def sky_at(y):
            t = min(1.0, max(0.0, (y - hy) / DocumentInteraction._SKY_GRAD_SPAN))
            return mix(sky_h, sky_rgb, t)

        def gnd_at(y):
            t = min(1.0, max(0.0, (hy - y) / DocumentInteraction._GROUND_GRAD_SPAN))
            return mix(gnd_h, ground_rgb, t)

        y0 = max(-1.0, min(1.0, hy))
        line = mix(mix(sky_h, gnd_h, 0.5), (0.0, 0.0, 0.0), 0.12)
        return sky_at(y0), sky_at(1.0), gnd_at(y0), gnd_at(-1.0), line

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
        """Unit vector toward the sun for the scene's shadow settings —
        ``None`` when there is nothing to cast (shadows off, kill switch
        ``INGETRAZO_NO_SHADOWS``, a sheet plan style, night, or a date that
        does not exist in a hand-edited file)."""
        import os
        if os.environ.get("INGETRAZO_NO_SHADOWS"):
            return None
        if self.plano_style is not None:
            return None
        from core.sun import scene_sun_direction
        return scene_sun_direction(self.scene)


    @staticmethod
    def _shadow_footprint(light_vp) -> tuple[float, float]:
        """Metres across one side of the map and along its depth range.
        The light's view is a rigid motion, so each row of the orthographic
        projection keeps its scale: 2 / width and 2 / depth."""
        def span(row: int) -> float:
            n = light_vp.row(row).toVector3D().length()
            return 2.0 / n if n > 1e-12 else 1.0
        return span(0), span(2)

    @staticmethod
    def _light_vp(d, lo, hi, fit=None):
        """The sun's orthographic view-projection fitted around the model —
        and around the z=0 ground patch that receives. Ortho keeps the
        packed depth linear.

        ``fit`` = ``(cx, cy, half)`` in the sun's view plane narrows the map
        to that square (#958: what the settled view shows, see
        ``_settle_shadow_fit``); along the sun it still spans the whole
        model, so a caster off screen still throws its shadow in."""
        view, r = DocumentInteraction._light_view(d, lo, hi)
        proj = QMatrix4x4()
        # Eye sits 2r from center; everything lives within r of it.
        if fit is None:
            proj.ortho(-r, r, -r, r, r * 0.5, r * 3.5)
        else:
            cx, cy, half = fit
            proj.ortho(cx - half, cx + half, cy - half, cy + half,
                       r * 0.5, r * 3.5)
        return proj * view

    @staticmethod
    def _light_view(d, lo, hi):
        """The sun's view matrix round the model, and the radius ``r`` of
        the sphere the model and its ground patch fit in."""
        cx = (lo.x() + hi.x()) / 2.0
        cy = (lo.y() + hi.y()) / 2.0
        loz = min(lo.z(), 0.0)
        cz = (loz + hi.z()) / 2.0
        # Half-diagonal of the dominant extent (≤ 0.87×max side), with
        # margin so rims never clip against the map's edge.
        r = max(hi.x() - lo.x(), hi.y() - lo.y(), hi.z() - loz, 2.0) * 0.87
        r *= 1.25
        center = QVector3D(cx, cy, cz)
        eye = center + QVector3D(d[0], d[1], d[2]) * (r * 2.0)
        up = (QVector3D(0.0, 0.0, 1.0) if abs(d[2]) < 0.95
              else QVector3D(0.0, 1.0, 0.0))
        view = QMatrix4x4()
        view.lookAt(eye, center, up)
        return view, r

    #: How long the view must stay put before the fitted map is drawn (#958).
    SHADOW_SETTLE_MS = 200

    # ---- Dynamic uploads ----------------------------------------------------
    def prune_stale_selection(self):
        """Release references to deleted elements before notifying document consumers."""
        hover = getattr(self, "_hover_entity", None)
        cands = [hover] if isinstance(hover, (Edge, Face)) else []
        cands += [s for s in self.scene.selection
                  if isinstance(s, (Edge, Face))]
        if cands:
            if len(cands) > 64:
                # Selected/hovered Edge/Face refs only ever point into the
                # active mesh (loose, or the group being edited) — the old
                # render_edges()/render_faces() sets walked EVERY group's
                # entities (~283k) per edit frame after an explode.
                alive_e: set = set(map(element_identity, self.scene.mesh.edges))
                alive_f: set = set(map(element_identity, self.scene.mesh.faces))

                def alive(ent):
                    return ent.key in (alive_e if isinstance(ent, Edge)
                                       else alive_f)
            else:
                # A mesh the experimental engine has not built yet holds no
                # Edge/Face object, so nothing can be alive in it: asking it
                # built every lazy group of the villa (~0.5 s) on the first
                # click inside an opened group (issue #1006). The active
                # mesh — where a selection inside a group lives — first.
                act = self.scene.mesh
                gmeshes = [g.mesh for g in self._placements()
                           if self.scene.entity_visible(g)
                           and not getattr(g, "billboard", False)
                           and "_deferred" not in g.mesh.__dict__]
                gmeshes.sort(key=lambda gm: gm is not act)

                # A hidden face/edge the hidden view shows is selectable,
                # and a selection it took must not be dropped here: a
                # single click on a ghost face looked like nothing (only
                # a double click, which selected its visible edges too,
                # seemed to work — Marco, 2026-09-21, issue #53).
                def shown(ent):
                    return (self.scene.entity_visible(ent)
                            or self.scene.entity_selectable(ent))

                def alive(ent):
                    if isinstance(ent, Edge):
                        if ent in self.scene.loose_mesh.edges:
                            return shown(ent)
                        return any(ent in gm.edges for gm in gmeshes)
                    if ent in self.scene.loose_mesh.faces:
                        return shown(ent)
                    return any(ent in gm.faces for gm in gmeshes)

            if isinstance(hover, (Edge, Face)) and not alive(hover):
                self._hover_entity = None
            for s in [s for s in self.scene.selection
                      if isinstance(s, (Edge, Face)) and not alive(s)]:
                self.scene.selection.discard(s)

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
        """Enter a groups-only transform preview: the per-version caches
        freeze, the moving groups leave the consolidated VBOs (one resync)
        and their chunk arrays upload ONCE to scratch VBOs that paintGL
        draws with a translated MVP — a drag frame touches no arrays at
        all. Smooth group dragging on a 230k-face group.

        ``external=True`` previews OFF-scene template groups (Paste's
        clipboard): same scratch upload, but nothing leaves the consolidated
        VBOs and the caches stay live — no freeze, no resyncs."""
        # A component keeps placements inside itself, and THOSE hold the
        # geometry: previewing only the top-level group uploaded an empty
        # scratch copy while the children kept drawing, frozen, at the old
        # spot — the hedge stood still through the whole drag and teleported
        # on release (Marco: "cuando roto o muevo tiene laj bastante").
        movers: list = []
        # Inside an open group the movers sit in its paint: without it a
        # plain piece turned unpainted for the drag and came back on release
        # (#1101).
        open_paint = self.scene.open_group_paint()
        self._placements()                     # the proxies _preview_chunk bakes
        for g in groups:
            self._expand_placements(g, movers, above=open_paint)
        groups = movers
        self._preview_external = external
        if not external:
            self._frozen_cache_version = self.scene.version
            self._edges_version = -1      # one resync WITHOUT the movers
        self._preview_groups = set(map(id, groups))
        self._preview_epoch = getattr(self, "_preview_epoch", 0) + 1
        self._preview_offset = QVector3D(0.0, 0.0, 0.0)

    def _preview_chunk(self, g):
        """The arrays of a mover's scratch copy. A nested piece handed in as
        itself bakes through its placement proxy, which knows the paint above
        it; the piece alone only knows its own (#1101)."""
        proxies = getattr(self, "_placement_proxies", None) or {}
        return self._group_chunk(proxies.get(id(g), g))

    def end_groups_preview(self) -> None:
        if not getattr(self, "_preview_external", False):
            self._frozen_cache_version = None
            self._edges_version = -1      # resync with the movers back in
        else:
            # Off-scene templates never left the consolidated VBOs (nothing
            # to resync) and their scratch chunks are one-shot — drop them
            # so a big clipboard doesn't pin megabytes of arrays.
            scene_ids = {id(g) for g in self._placements()}
            for cache_name in ("_group_chunks", "_inst_chunks"):
                cache = getattr(self, cache_name, None)
                if cache:
                    for gid in self._preview_groups - scene_ids:
                        cache.pop(gid, None)
        self._preview_external = False
        self._preview_groups = set()
        self._preview_epoch = getattr(self, "_preview_epoch", 0) + 1
        self._preview_offset = QVector3D(0.0, 0.0, 0.0)
        self._preview_matrix = None
        self._pv_edges_count = 0
        self._pv_vcol_count = 0
        self._pv_tex_runs = []

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
        """The world transform the groups preview draws its movers through:
        the full matrix (Rotate/Scale) or the translation (Move)."""
        m = getattr(self, "_preview_matrix", None)
        if m is not None:
            return QMatrix4x4(m)
        m = QMatrix4x4()
        off = getattr(self, "_preview_offset", None)
        if off is not None:
            m.translate(off)
        return m


    def _newell_of(self, face):
        """The face's raw Newell vector, memoised per scene version — ONE
        computation feeds normal, area and triangulation (each used to pay
        its own on an exploded import's edit frame)."""
        memo = getattr(self, "_newell_memo", None)
        if memo is None or memo[0] != _cache_ver(self):
            memo = self._newell_memo = (_cache_ver(self), {})
        hit = memo[1].get(element_identity(face))
        if hit is None or hit[0] != face:
            hit = memo[1][element_identity(face)] = (face, face._newell())
        return hit[1]

    def _area_of(self, face) -> float:
        if not hasattr(face, "loop"):
            return face.area()          # preview face (core.geometry)
        if len(face.loop) < 3:
            return 0.0
        return 0.5 * self._newell_of(face).length()

    def _tris_of(self, face):
        """``face.triangulate()`` memoised per scene version: one edit frame
        triangulates every loose face for the colour VBOs, the textured
        VBOs AND the pick index — 3x the earcut/_newell cost on an exploded
        import. The memo holds the face itself so a recycled id() can never
        alias, and drops wholesale on the next version bump."""
        if not hasattr(face, "loop"):
            return face.triangulate()   # preview face: own earcut, no memo
        memo = getattr(self, "_tri_memo", None)
        if memo is None or memo[0] != _cache_ver(self):
            memo = self._tri_memo = (_cache_ver(self), {})
        hit = memo[1].get(element_identity(face))
        if hit is None or hit[0] != face:
            hit = memo[1][element_identity(face)] = (
                face, face.triangulate(self._normal_of(face)))
        return hit[1]

    #: A face this big (corners, holes included) is worth handing to the
    #: native earcut ahead of ``_tris_of``.
    NATIVE_TRIS_MIN_CORNERS = 64

    def _seed_tris_native(self, faces) -> None:
        """Fill ``_tris_of``'s memo for the big faces among ``faces`` in one
        call to the engine's earcut (``face_geometry``: the same port, node
        for node, triangulating with the same unit normal — its parity is
        tests/test_face_geometry_native.py). A vector-art figure from an
        imported file is faces with thousands of holes: the Python earcut
        took ~40 s over the villa's 18 (issue #1006). Without the Rust
        engine nothing happens and ``_tris_of`` works as always."""
        import sys
        import numpy as np
        import core._native as _native
        nat = _native.native
        if nat is None or _native.qt_hypot() is None:
            return
        memo = getattr(self, "_tri_memo", None)
        if memo is None or memo[0] != _cache_ver(self):
            memo = self._tri_memo = (_cache_ver(self), {})
        big = []
        for f in faces:
            if not hasattr(f, "loop") or len(f.loop) < 3:
                continue
            hit = memo[1].get(element_identity(f))
            if hit is not None and hit[0] == f:
                continue
            rings = [f.vertices] + [h for h in f.holes if len(h) >= 3]
            if sum(len(r) for r in rings) >= self.NATIVE_TRIS_MIN_CORNERS:
                big.append((f, rings))
        if not big:
            return
        pts, sizes, counts = [], [], []
        for _f, rings in big:
            counts.append(len(rings))
            for r in rings:
                sizes.append(len(r))
                pts.extend(r)
        pos = np.array([(p.x(), p.y(), p.z()) for p in pts], np.float64)
        try:
            out = nat.face_geometry(
                pos, np.arange(len(pts), dtype=np.int64),
                np.asarray(sizes, np.int64), np.asarray(counts, np.int64),
                _native.qt_hypot(), sys.version_info >= (3, 12))
        except Exception:  # noqa: BLE001 — then the Python earcut
            return
        tris_u, tri_face_u = out[6], out[7]
        per_face: list = [[] for _ in big]
        for (a, b, c), fi in zip(tris_u.tolist(), tri_face_u.tolist()):
            per_face[fi].append((pts[a], pts[b], pts[c]))
        for (f, _rings), tris in zip(big, per_face):
            memo[1][element_identity(f)] = (f, tris)

    def _normal_of(self, face):
        """``face.normal()`` memoised per scene version (see _tris_of):
        the shading of every loose face recomputes the Newell normal 3-4
        times per edit frame otherwise. Tool PREVIEW faces (core.geometry,
        no ``loop``) take their own methods unmemoised — fresh objects every
        frame; treating them as mesh faces crashed EVERY paint of a loose
        paste preview (nothing followed the cursor)."""
        if not hasattr(face, "loop"):
            return face.normal()
        if len(face.loop) < 3:
            return QVector3D(0.0, 0.0, 1.0)
        n = self._newell_of(face)
        return (n.normalized() if n.length() > 1e-9
                else QVector3D(0.0, 0.0, 1.0))

    def _static_draw_groups(self, groups):
        """Of ``groups`` (the placements in draw order), the ones the
        consolidated passes draw — visible, not face-me, not instanced —
        in the same order. Kept while only matrices move (a Move: see
        ``_epoch_only_matrices``), which change none of the three."""
        epoch_of = getattr(self, "_placements_epoch", None)   # stub VPs in tests
        if epoch_of is None:
            return [g for g in groups
                    if self.scene.entity_visible(g)
                    and not getattr(g, "billboard", False)
                    and not self._instanced_eligible(g)]
        epoch = epoch_of()
        ids = tuple(map(id, groups))
        memo = getattr(self, "_static_groups_memo", None)
        if (memo is not None and memo[1] == ids and memo[2] is self.scene.mesh
                and (memo[0] == epoch
                     or self._epoch_only_matrices(memo[0], epoch))):
            if memo[0] != epoch:
                self._static_groups_memo = (epoch,) + memo[1:]
            return memo[3]
        def static(g):
            return (self.scene.entity_visible(g)
                    and not getattr(g, "billboard", False)
                    and not self._instanced_eligible(g))
        out = None
        if memo is not None:
            # Visible, face-me, instanced: what a placement is, not what
            # the open context makes of it — kept across a group opened or
            # closed too (~0.12 s each way on the plant of #158).
            out = self._refilter(memo[0], memo[3], groups, static, memo[1],
                                 ctx=memo[2] is not self.scene.mesh)
        if out is None:
            out = [g for g in groups if static(g)]
        self._static_groups_memo = (epoch, ids, self.scene.mesh, out)
        return out

    #: Past this many objects gone, or this many bytes of their zeros in
    #: the faces buffer, the static buffers are laid out afresh.
    _SLOTS_MAX_GONE = 64
    _SLOTS_MAX_BYTES = 64 << 20

    def _static_slots(self, walk):
        """``walk`` (the placements the static buffers draw, in order) as
        slots ``(slot, gone)``, keeping the places of objects taken away
        since the last layout: a Delete leaves zeros of the deleted object's
        size where it was, its undo fills them back, a Paste comes at the
        end. Everything else keeps its offset, so the upload re-sends what
        changed and not the whole tail after it — up to 0.4 GB of faces on
        the plant of #158, ~0.4 s each way. The order of what is drawn is
        the walk's, as laid out afresh. A walk that changed otherwise (an
        object come in between, an order changed), or too many objects
        gone, is laid out afresh."""
        prev = getattr(self, "_slot_layout", None)
        ids = {id(g): g for g in walk}
        out = None
        if prev:
            known = {id(sl.g) for sl in prev}
            kept = [id(sl.g) for sl in prev if id(sl.g) in ids]
            order = [id(g) for g in walk if id(g) in known]
            if kept == order:
                n_old = len(order)
                tail = [g for g in walk if id(g) not in known]
                if (all(id(g) not in known for g in walk[n_old:])
                        and len(tail) == len(walk) - n_old):
                    gone = [sl for sl in prev if id(sl.g) not in ids]
                    if (len(gone) <= self._SLOTS_MAX_GONE
                            and sum(sl.sizes.get("f", 0) for sl in gone)
                            <= self._SLOTS_MAX_BYTES):
                        out = ([(sl, id(sl.g) not in ids) for sl in prev]
                               + [(_Slot(g), False) for g in tail])
        if out is None:
            old = {id(sl.g): sl for sl in prev or ()}
            out = [(old.get(id(g)) or _Slot(g), False) for g in walk]
        self._slot_layout = [sl for sl, _gone in out]
        return out

    def _announce_after_paint(self, version: int) -> None:
        """``sceneVersionChanged`` for what this frame found, once the frame
        is out. ``_sync_edges`` runs inside ``paintGL``, and whoever hears
        the signal may show or hide widgets beside the view: the scene tab
        row coming up resized the view in the middle of its frame, and
        macOS crashed on the flush after (#1316). Every listener runs on the
        next turn instead, with the latest version; a burst says it once.
        ``notify_scene_changed`` still says it at once."""
        self._version_to_announce = version
        if self.__dict__.get("_announce_queued"):
            return
        self._announce_queued = True
        later(self, 0, self._announce_queued_version)

    def _announce_queued_version(self) -> None:
        self._announce_queued = False
        _publish_scene_version(self, self._version_to_announce)

    def _sync_content_key(self) -> tuple:
        """What the edge and face buffers of ``_sync_edges`` depend on:
        everything but the selection."""
        sc = self.scene
        sup = self._suppressed_faces
        return (_content_ver(self), self._placements_epoch(), id(sc.mesh),
                id(sc.edit_group), self._rest_is_hidden(),
                tuple(sorted(getattr(self, "_preview_groups", None) or ())),
                frozenset(map(element_identity, sup)) if sup else None,
                bool(getattr(sc, "show_hidden_objects", False)),
                bool(getattr(sc, "show_hidden_geometry", False)),
                getattr(self, "_edges_front_tint", None))

    def _selected_group_placements(self):
        selected = frozenset(e for e in self.scene.selection
                             if isinstance(e, Group))
        if not selected:
            return []
        key = (self._placements_epoch(), selected)
        memo = getattr(self, "_selected_placements_cache", None)
        if memo is None or memo[0] != key:
            pv = getattr(self, "_preview_groups", None) or ()
            # Structure can select members from several containment contexts.
            # A rendered proxy retains its placement UID across those contexts.
            selected_uids = {g.uid for g in selected}
            groups = [g for g in self._placements()
                      if (self._owner_of(g) in selected or g.uid in selected_uids)
                      and self.scene.entity_visible(g)
                      and not getattr(g, "billboard", False)
                      and id(g) not in pv]
            memo = self._selected_placements_cache = (key, groups)
        return memo[1]

    def _bucket_back_face(self, face, back):
        """Build the back-side override copy of a two-side-painted face.
        Returns ``(kind, payload)`` — ``("bvcol", bytes)`` /
        ``("btex", (key, bytes))`` for opaque overrides (culled opaque back
        pass), or ``("btcol", (op, bytes))`` / ``("bttex", ((key, op),
        bytes))`` for translucent ones (culled blended pass). ``(None,
        None)`` when the back carries nothing drawable."""
        op = float(back.get("opacity", 1.0))
        tex = back.get("texture")
        if tex is not None and tex.get("path"):
            tmp: dict = {}
            self._append_textured_face(tmp, face, tex)
            for key, arr in tmp.items():
                if op < 0.999:
                    return "bttex", ((key, round(op, 3)), arr.tobytes())
                return "btex", (key, arr.tobytes())
            return None, None
        col = back.get("color")
        if col is None:
            return None, None
        r, g, b = self._shaded_color(tuple(col), self._normal_of(face))
        buf = array("f")
        for t0, t1, t2 in self._tris_of(face):
            buf.extend([
                t0.x(), t0.y(), t0.z(), r, g, b,
                t1.x(), t1.y(), t1.z(), r, g, b,
                t2.x(), t2.y(), t2.z(), r, g, b,
            ])
        if op < 0.999:
            return "btcol", (round(op, 3), buf.tobytes())
        return "bvcol", buf.tobytes()

    def _append_textured_face(self, by_texture: dict, face, tex: dict) -> None:
        """Triangulate ``face`` into ``by_texture[(path, shade)]`` as
        interleaved ``pos(3) + uv(2)`` floats — the quantised diffuse shade
        keys the draw run (the run sets ``u_shade``, so textures get the same
        matte face shading as colour faces). UVs come from
        ``core.texture.face_uv_axes`` — the face's fitted world→UV affine
        map when present (``uvw`` — an import carrying its own texture
        coordinates), else the .skp planar projection of each vertex's
        world position. The SAME function the .skp exporter writes from, so
        the viewport cannot drift from what the .skp file will show (it did: a
        private copy of the projection here used a different basis)."""
        key = (tex["path"], self._shade_factor(self._normal_of(face)))
        buf = by_texture.get(key)
        if buf is None:
            buf = by_texture[key] = array("f")
        gu, cu, gv, cv = face_uv_axes(tex, face.normal())
        for tri in self._tris_of(face):
            for p in tri:
                buf.extend([
                    p.x(), p.y(), p.z(),
                    QVector3D.dotProduct(gu, p) + cu,
                    QVector3D.dotProduct(gv, p) + cv,
                ])

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
        """What a face-me figure's cached data (placed mesh, extent, draw
        arrays) depends on: its OWN mesh — the object and a content
        fingerprint (coordinates, paint, counts) — and its matrix. Unlike
        the model's content version it does not move when something ELSE
        is drawn or painted, or when a group is opened or closed: keyed on
        that version, every edit re-triangulated the
        villa's 18 vector-art figures (~4 s per stroke, paint or Esc,
        issue #1006). The coordinate walk (~3 ms for 9 000 vertices) is
        paid only when the figure's own mesh was edited."""
        memo = getattr(self, "_figure_tokens", None)
        if memo is None:
            memo = self._figure_tokens = {}
        mesh = group.mesh
        cv = _content_ver(self)
        xf = getattr(group, "xform", None)
        # The matrix is read every time: a placement can be moved in place
        # without a version bump (the face-me quad of a moved copy).
        xft = None if xf is None else tuple(xf.data())
        hit = memo.get(id(group))
        if hit is not None and hit[0] == cv and hit[1] is mesh \
                and hit[2] is group and hit[3][1] == xft:
            return hit[3]
        # Geometry moves only through the mesh's mutation primitives, which
        # bump its serial: while it holds, the coordinate walk is skipped
        # and only the paint — which bypasses them — is re-read (17 faces).
        # The serial is not in the token itself: the experimental engine
        # builds a lazy mesh on first touch (drawing elsewhere does it),
        # bumping the serial of a figure whose content did not change.
        serial = getattr(mesh, "_mut_serial", None)
        if (hit is not None and hit[1] is mesh and serial is not None
                and hit[4] == serial):
            geo = hit[5]
        else:
            geo = self._mesh_fingerprint(mesh)
        token = (id(mesh), xft, geo, self._faces_paint_fp(mesh))
        if len(memo) > 4096:
            memo.clear()
        memo[id(group)] = (cv, mesh, group, token, serial, geo)
        return token

    def _billboard_mesh(self, group):
        """A billboard's mesh in WORLD space. A face-me placed INSIDE a
        component — the torito on the arch, brought in with the arch —
        keeps its prototype in the component's frame and reaches the draw
        list as a proxy with a matrix; reading the raw mesh put the bull at
        the file's origin, not on the cornice (Marco, 2026-09-11). Sprites
        are a face or two, so the placed copy is cached per matrix."""
        xf = getattr(group, "xform", None)
        if xf is None:
            return group.mesh
        cache = getattr(self, "_billboard_world", None)
        if cache is None:
            cache = self._billboard_world = {}
        key = self._figure_token(group)
        hit = cache.get(id(group))
        if hit is not None and hit[0] == key:
            return hit[1]
        from core.group import transformed_mesh
        placed = transformed_mesh(group.mesh, xf)
        if len(cache) > 256:
            cache.clear()
        cache[id(group)] = (key, placed)
        return placed

    def _billboard_extent(self, group):
        """(anchor, width, height, texture path) of a billboard as placed,
        before any turn or drag preview — or ``None`` for an empty one.
        Remembered per model content: a vector-art figure from an imported
        file carries thousands of vertices, and the walk over them ran for
        every figure on every click (38 figures, ~250 ms per click on the
        villa of issue #1006)."""
        key = self._figure_token(group)
        cache = getattr(self, "_billboard_extents", None)
        if cache is None:
            cache = self._billboard_extents = {}
        hit = cache.get(id(group))
        if hit is not None and hit[0] == key:
            return hit[1]
        verts = self._billboard_mesh(group).vertices
        if not verts:
            ext = None
        else:
            xs = [v.position.x() for v in verts]
            ys = [v.position.y() for v in verts]
            zs = [v.position.z() for v in verts]
            anchor = QVector3D((min(xs) + max(xs)) / 2,
                               (min(ys) + max(ys)) / 2, min(zs))
            # Planar width: hypot handles a sprite plane at any yaw (an
            # imported face-me sits wherever its baked transform left it);
            # an axis-aligned quad gives the same value as before.
            w = math.hypot(max(xs) - min(xs), max(ys) - min(ys))
            h = max(zs) - min(zs)
            tex = None
            for f in group.mesh.faces:
                t = f.attrs.get("texture")
                if t and t.get("path"):
                    tex = t["path"]
                    break
            # tex may stay None for a vector-art face-me (solid-colour
            # faces, e.g. a 2D person from an imported file): picking,
            # outline and snap still need the quad; the legacy textured-quad
            # render skips texture-less groups.
            ext = (anchor, w, h, tex)
        if len(cache) > 4096:
            cache.clear()
        cache[id(group)] = (key, ext)
        return ext

    def _billboard_quad(self, group, face_dir=None):
        """The face-me quad of a billboard group, rotated around its vertical
        anchor axis to face the camera NOW (or ``face_dir`` when given).
        Returns (corners[4], tex_path) or ``None``. Shared by the render
        pass, picking and the shadow casters."""
        ext = self._billboard_extent(group)
        if ext is None:
            return None
        anchor, w, h, tex = ext
        # A billboard being Move/Rotate-dragged previews through THIS pass
        # (its raw mesh quad in the scratch VBOs showed the texture swimming
        # and ignored the face-me turn): carry the preview transform on the
        # anchor so the figure follows the cursor, camera-facing, fixed UVs.
        if id(group) in (getattr(self, "_preview_groups", None) or ()):
            m = getattr(self, "_preview_matrix", None)
            if m is not None:
                anchor = m.map(anchor)
            else:
                off = getattr(self, "_preview_offset", None)
                if off is not None:
                    anchor = anchor + off
        if w < 1e-9 or h < 1e-9:
            return None
        # Facing: the camera by default; the shadow pass passes the SUN so a
        # figure's cast silhouette holds still while the camera orbits
        # (face-me "shadows face sun").
        d = face_dir if face_dir is not None else self._faceme_dir(anchor)
        d = QVector3D(d.x(), d.y(), 0.0)
        if d.length() < 1e-6:
            d = QVector3D(1.0, 0.0, 0.0)
        d = d.normalized()
        r = QVector3D(-d.y(), d.x(), 0.0)
        up = QVector3D(0.0, 0.0, 1.0)
        c0 = anchor - r * (w / 2)
        c1 = anchor + r * (w / 2)
        return ([c0, c1, c1 + up * h, c0 + up * h], tex)

    def _faceme_base(self, g):
        """Cached base data of a mesh face-me billboard: interleaved pos+uv
        arrays per image, the vertical anchor axis and the horizontal base
        normal. Rebuilt when the MODEL changes, not on a click: a vector-art
        figure from an imported file is faces with thousands of holes, and
        triangulating the villa's 18 again after every selection was the
        ~5 s per click of issue #1006."""
        cache = getattr(self, "_faceme_cache", None)
        if cache is None:
            cache = self._faceme_cache = {}
        key = self._figure_token(g)
        cur = cache.get(id(g))
        if cur is not None and cur[0] == key:
            return cur[1]
        import numpy as np
        mesh = self._billboard_mesh(g)       # world space, matrix applied
        self._seed_tris_native(mesh.faces)
        by_tex: dict = {}
        by_color: dict = {}      # rgb tuple -> interleaved pos+uv (uv unused)
        n0 = None
        best_area = 0.0
        for f in mesh.faces:
            area = f.area() if hasattr(f, "area") else 0.0
            if n0 is None or area > best_area:
                n0 = f.normal()
                best_area = area
            tex = f.attrs.get("texture")
            if tex is not None and tex.get("path"):
                self._append_textured_face(by_tex, f, tex)
                continue
            # Solid-colour face (a vector-art person from a .skp file):
            # same interleaved layout, uv ignored, drawn with u_color.
            col = tuple(f.attrs.get("color") or (0.96, 0.95, 0.925))
            buf = by_color.setdefault(col, array("f"))
            for t0, t1, t2 in self._tris_of(f):
                for p in (t0, t1, t2):
                    buf.extend([p.x(), p.y(), p.z(), 0.0, 0.0])
        verts = mesh.vertices
        entry = None
        if n0 is not None and verts and (by_tex or by_color):
            xs = [v.position.x() for v in verts]
            ys = [v.position.y() for v in verts]
            zs = [v.position.z() for v in verts]
            # The anchor is the figure's FEET (x, y, min z), like the simple
            # quad's: _draw_faceme_mesh aims the face-me from it in 3D. A 2-D
            # anchor here raised IndexError inside paintGL and left the whole
            # viewport blank for any model carrying a mesh face-me (Susan in
            # plaza Yanque(2).skp — v0.3.9 shipped with it).
            anchor = np.array([(min(xs) + max(xs)) / 2,
                               (min(ys) + max(ys)) / 2, min(zs)])
            nh = np.array([n0.x(), n0.y()])
            ln = float(np.hypot(nh[0], nh[1]))
            if ln > 1e-9:
                nh = nh / ln
                arrays = {p: np.frombuffer(buf.tobytes(),
                                           dtype=np.float32).reshape(-1, 5)
                          for p, buf in by_tex.items()}
                colors = {c: np.frombuffer(buf.tobytes(),
                                           dtype=np.float32).reshape(-1, 5)
                          for c, buf in by_color.items()}
                entry = (arrays, colors, anchor, nh)
        cache[id(g)] = (key, entry)
        return entry

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

    def _section_cut_active(self):
        return _active_cut(self.scene)

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
        """Return the dimension whose lines (extension + dimension line) are
        closest to the cursor within the pick threshold, or ``None``."""
        dims = getattr(self.scene, "dimensions", None)
        if not dims:
            return None
        best = None
        best_d = self.pick_threshold_px
        for dim in dims:
            if not self.scene.entity_selectable(dim):   # hidden / locked
                continue
            ap, bp = dim.line_points()
            for s, e in ((dim.a, ap), (dim.b, bp), (ap, bp)):
                ps = self._world_to_pixel(s)
                pe = self._world_to_pixel(e)
                if ps is None or pe is None:
                    continue
                d = _point_to_segment_distance_2d((screen_x, screen_y), ps, pe)
                if d < best_d:
                    best_d = d
                    best = dim
        return best

    def pick_text_label(self, screen_x: float, screen_y: float,
                        rect_only: bool = False):
        """Return the leader-text label under the cursor, or ``None``.

        A click anywhere on the drawn text block (same font-metrics layout
        as ``_draw_text_labels``) is a hit — the glyphs are what the user
        aims at. Failing that, the label position and the leader line count
        within the pick threshold. ``rect_only`` restricts to the text
        block: the block overdraws all geometry, so that hit outranks edge
        and face picks, while the (thin) leader keeps normal priority."""
        labels = getattr(self.scene, "text_labels", None)
        if not labels:
            return None
        style = getattr(self.scene, "dimension_style", {})
        font = QFont(style.get("font_family", ""))
        font.setPointSize(int(style.get("font_size", 9)))
        font.setBold(style.get("font_bold", True))
        font.setItalic(style.get("font_italic", False))
        # Measured on THIS widget, as the painter that draws the labels
        # does: bare QFontMetrics(font) reads the primary screen's DPI, and
        # on a scaled display the glyph box it gave missed the letters
        # (Marco, 2026-09-23: the eraser only took a text by its leader).
        if NATIVE:
            fm = font
        else:
            from PySide6.QtGui import QPaintDevice
            fm = QFontMetrics(font, self) if isinstance(self, QPaintDevice) else QFontMetrics(font)
        best, best_d = None, self.pick_threshold_px * 2.0
        for lab in labels:
            if not self.scene.entity_selectable(lab):   # hidden / locked
                continue
            pp = self._world_to_pixel(lab.position())
            if pp is None:
                continue
            pa = self._world_to_pixel(lab.anchor)
            # The text block: lines start at _text_block_x(...), first
            # baseline at pp.y - 4, one fm.height() apart (mirror of
            # _draw_text_labels).
            lines = lab.text.splitlines() or [""]
            x0 = self._text_block_x(
                pp, pa, max(fm.horizontalAdvance(ln) for ln in lines))
            for i, line in enumerate(lines):
                base = pp[1] - 4 + i * fm.height()
                if (x0 - 3 <= screen_x <= x0 + fm.horizontalAdvance(line) + 3
                        and base - fm.ascent() - 3 <= screen_y
                        <= base + fm.descent() + 3):
                    return lab
            if rect_only:
                continue
            d = math.hypot(pp[0] - screen_x, pp[1] - screen_y)
            if pa is not None:
                d = min(d, _point_to_segment_distance_2d(
                    (screen_x, screen_y), pa, pp))
            if d < best_d:
                best_d = d
                best = lab
        return None if rect_only else best

    def pick_geopath(self, screen_x: float, screen_y: float):
        """Return the georef path whose polyline is closest to the cursor within
        the pick threshold, or ``None`` (Track G)."""
        paths = getattr(self.scene, "geo_paths", None)
        if not paths:
            return None
        best, best_d = None, self.pick_threshold_px
        for path in paths:
            for a, b in path.segments():
                pa = self._world_to_pixel(self.drape(a))
                pb = self._world_to_pixel(self.drape(b))
                if pa is None or pb is None:
                    continue
                d = _point_to_segment_distance_2d((screen_x, screen_y), pa, pb)
                if d < best_d:
                    best_d = d
                    best = path
        return best

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
        """Screen distance from the cursor to a guide circle's rim, or None
        when it cannot be projected. Sampling is display-only; the stored
        circle stays analytic."""
        pts = []
        for p in g.sample_points(64):
            px = self._world_to_pixel(p)
            if px is None:
                return None            # part of the circle is behind the eye
            pts.append(px)
        best = float("inf")
        n = len(pts)
        for i in range(n):
            d = _point_to_segment_distance_2d((screen_x, screen_y),
                                              pts[i], pts[(i + 1) % n])
            if d < best:
                best = d
        return best

    def pick_guide(self, screen_x: float, screen_y: float):
        """Return the construction guide nearest the cursor within the pick
        threshold, or ``None`` (for the Eraser). One on a hidden or
        locked layer is not there to pick."""
        selectable = getattr(self.scene, "entity_selectable", None)
        guides = [g for g in getattr(self.scene, "guides", None) or ()
                  if selectable is None or selectable(g)]
        if not guides:
            return None
        best, best_d = None, self.pick_threshold_px
        for g in guides:
            if getattr(g, "is_circle", False):
                d = self._circle_pick_distance(g, screen_x, screen_y)
                if d is not None and d < best_d:
                    best_d = d
                    best = g
                continue
            seg = self._clip_segment_front(*g.segment())
            if seg is None:
                continue
            pa = self._world_to_pixel(seg[0])
            pb = self._world_to_pixel(seg[1])
            if pa is None or pb is None:
                continue
            d = _point_to_segment_distance_2d((screen_x, screen_y), pa, pb)
            if d < best_d:
                best_d = d
                best = g
        return best

    def pick_vertex(self, screen_x: float, screen_y: float):
        """Return the scene vertex (corner) closest to the cursor within the
        pick threshold, or ``None``. Used to acquire a corner as a 'from point'
        reference while drawing. Occluded vertices are ignored (tested in
        ascending screen distance, so the nearest visible corner wins — the
        same answer the old per-edge scan produced)."""
        import numpy as np
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        if nat is not None:
            return self._pick_vertex_unbaked(screen_x, screen_y, nat)
        idx = self._pick_index(near=("px", screen_x, screen_y))
        # On the projections the viewport already caches per camera pose
        # (this runs on EVERY hover now — encouraged points): one numpy
        # distance pass, no re-projection of the model per mouse move.
        xs, ys, oks = [], [], []
        n = 0
        ledge = getattr(self, "_ledge_screen", None)      # stub VPs in tests
        lp = ledge() if ledge is not None else None
        if lp is None and idx.edge_a is not None and len(idx.edge_a):
            ax, ay, oka = self._project_px(idx.edge_a)
            bx, by, okb = self._project_px(idx.edge_b)
            lp = (ax, ay, bx, by, oka & okb)
        if lp is not None:
            ax, ay, bx, by, ok = lp
            xs += [ax, bx]; ys += [ay, by]; oks += [ok, ok]
            n = len(idx.edges)
        m = 0
        gp = self._gedge_screen()
        if gp is not None:
            # Group corners too — dimensioning/drawing over an imported
            # reference model needs its vertices as 'from points'.
            ax, ay, bx, by, ok = gp
            xs += [ax, bx]; ys += [ay, by]; oks += [ok, ok]
            m = len(idx.gedge_a)
        if not xs:
            return None
        # The concatenation is the same until the scene or camera moves.
        ckey = (id(lp), id(gp), n, m)
        cat = getattr(self, "_vertex_px_cache", None)
        if cat is None or cat[0] != ckey:
            cat = (ckey, np.concatenate(xs), np.concatenate(ys), np.concatenate(oks))
            self._vertex_px_cache = cat
        _c, px, py, ok = cat
        d = np.where(ok, np.hypot(px - screen_x, py - screen_y), np.inf)
        cand = np.where(d < self.pick_threshold_px)[0]

        def vertex_of(i):
            if i < 2 * n:
                e = idx.edges[int(i) % n]
                return e.a if i < n else e.b
            j = int(i) - 2 * n
            arr = idx.gedge_a if j < m else idx.gedge_b
            p = arr[j % m]
            return QVector3D(p[0], p[1], p[2])
        order = cand[np.argsort(d[cand])]
        if not len(order):
            return None
        # A corner only an edge that draws nothing holds is not on screen:
        # acquiring it as a from-point is how a phantom point gets into the
        # next face (the same rule the snap scene applies to the edges).
        snap_ok = None if n == 0 else getattr(idx, "edge_snap", None)
        if snap_ok is not None:
            order = np.asarray([i for i in order
                                if i >= 2 * n or snap_ok[int(i) % n]],
                               dtype=order.dtype)
            if not len(order):
                return None
        # The nearest one, alone first (usually the answer); then the
        # rest in batches of 8, 16, 32… sharing one occlusion pass, as the
        # snap engine reads its candidates. One ray per candidate, in turn,
        # took minutes a pointer move over a dense mesh whose hundreds of
        # nearby corners were nearly all hidden (a 128 788-face STL in #158).
        # A corner shared by several edges comes once per end: asked once
        # (the same point, the same answer), in batches of at most 1 024 —
        # zoomed out over the plant of #158 some 65 000 hidden corners lay
        # under the cursor, and a batch of 32 768 rays built ~1.4 GB of box
        # tests and took 7 s.
        first = vertex_of(order[0])
        if not self._is_occluded(first):
            return first
        if len(order) > 1:
            # Each point once, where it first comes (the nearest of its
            # copies on screen): the same answer as asking every copy.
            xyz = np.empty((len(order), 3))
            for lo_i, hi_i, arr, base in (
                    (0, n, idx.edge_a, 0), (n, 2 * n, idx.edge_b, n),
                    (2 * n, 2 * n + m, idx.gedge_a, 2 * n),
                    (2 * n + m, 2 * n + 2 * m, idx.gedge_b, 2 * n + m)):
                w = np.flatnonzero((order >= lo_i) & (order < hi_i))
                if len(w):
                    xyz[w] = arr[order[w] - base]
            _u, at = np.unique(xyz, axis=0, return_index=True)
            at.sort()
            order = order[at]
        many = getattr(self, "_occluded_many", None)
        k, size = 1, 8
        while k < len(order):
            batch = [vertex_of(i) for i in order[k:k + size]]
            hidden = (many(batch) if many is not None
                      else [self._is_occluded(v) for v in batch])
            for v, h in zip(batch, hidden):
                if not h:
                    return v
            k += size
            size = min(size * 2, 1024)
        return None

    def _pick_vertex_unbaked(self, screen_x, screen_y, nat):
        """``pick_vertex`` with the lazy placements' corners from the
        native engine, none of them baked (#158): the loose edges' and the
        fixed placements' corners as the index holds them, every lazy
        placement's as its bake would hold them, at the same pixels —
        nearest first, the hidden ones left out, as there."""
        import numpy as np
        iset, snap, _vis = nat
        idx = self._pick_index(near="skip")
        thr = self.pick_threshold_px
        parts = []                       # (px, py, in front, points)
        ledge = getattr(self, "_ledge_screen", None)
        lp = ledge() if ledge is not None else None
        if lp is None and idx.edge_a is not None and len(idx.edge_a):
            ax, ay, oka = self._project_px(idx.edge_a)
            bx, by, okb = self._project_px(idx.edge_b)
            lp = (ax, ay, bx, by, oka & okb)
        if lp is not None:
            ax, ay, bx, by, ok = lp
            # Corners only an edge that draws nothing holds are not on
            # screen (see pick_vertex).
            snap_ok = getattr(idx, "edge_snap", None)
            if snap_ok is not None:
                ok = ok & snap_ok
            parts += [(ax, ay, ok, idx.edge_a), (bx, by, ok, idx.edge_b)]
        fx = getattr(idx, "gedge_fixed", None)
        m = fx[1] if fx is not None else 0
        gp = self._gedge_screen() if m else None
        if gp is not None:
            # The fixed placements' edges lead the index's; the lazy ones
            # it holds now are among those the engine answers for.
            ax, ay, bx, by, ok = gp
            parts += [(ax[:m], ay[:m], ok[:m], idx.gedge_a[:m]),
                      (bx[:m], by[:m], ok[:m], idx.gedge_b[:m])]
        # What the frame shows in front of a corner hides it: zoomed out
        # over the plant of #158 some 800 000 corners of the copies lay
        # under the cursor, nearly all hidden, and a ray each took 15 s.
        # The rule that kept such placements out of the index unbaked (see
        # _pick_materialize), per corner at its own pixel. A corner whose
        # pixel and the 8 around it all show something nearer is not to be
        # seen on screen, even where an ideal ray to its very point grazes
        # past (a post rising beside it, seen from afar): one such corner
        # in 109 cursor positions over that plant, left for another a pixel
        # away. Whatever comes back is still a ray's visible answer.
        seen = self._depth_limits(float(screen_x), float(screen_y), thr)
        ds, ps = [], []
        for x, y, ok, pts in parts:
            k, d = _px_near(x, y, ok, screen_x, screen_y, thr, pts, seen)
            if len(k):
                ds.append(d)
                ps.append(pts[k])
        if len(snap):
            M = self._np_mvp()
            pl, _e, _n, xyz, rx, ry = iset.corners_near(
                snap, np.ascontiguousarray(M.T).ravel().tolist(),
                float(self.width()), float(self.height()),
                float(screen_x), float(screen_y), thr * (1.0 + 1e-9) + 1e-9,
                seen)
            if len(pl):
                d = np.hypot(rx - screen_x, ry - screen_y)
                k = np.flatnonzero(d < thr)
                if len(k):
                    ds.append(d[k])
                    ps.append(xyz[k])
        if not ds:
            return None
        d = np.concatenate(ds)
        pts = np.concatenate(ps)[np.argsort(d, kind="stable")]
        at = self._first_clear(pts)
        if at is not None:
            return QVector3D(*pts[at].tolist()) if at >= 0 else None
        first = QVector3D(*pts[0].tolist())
        if not self._is_occluded(first):
            return first
        if len(pts) > 1:
            # Each point once, where it first comes (see pick_vertex): a
            # stable sort by x, y, z puts the copies of a point together,
            # the first first (np.unique by rows took 6 ms on 11 000).
            by = np.lexsort((pts[:, 2], pts[:, 1], pts[:, 0]))
            srt = pts[by]
            lead = np.ones(len(srt), dtype=bool)
            lead[1:] = (srt[1:] != srt[:-1]).any(axis=1)
            pts = pts[np.sort(by[lead])]
        # Batches of 8, 32, 128… up to 1 024: with the rays this cheap the
        # calls were the cost (~0.4 ms each, 13 over the 7 000 corners a
        # roof hides near the cursor over the plant of #158).
        k, size = 1, 8
        while k < len(pts):
            batch = pts[k:k + size]
            seen_at = np.flatnonzero(~np.array(self._occluded_many(batch),
                                               dtype=bool))
            if len(seen_at):
                return QVector3D(*batch[seen_at[0]].tolist())
            k += size
            size = min(size * 4, 1024)
        return None

    def hlr_geometry(self, tags: bool = False):
        """World-space geometry for the hidden-line pass (core.hlr) as
        arrays, straight from the caches the viewport already keeps: the
        pick index's triangles, every placement chunk's hard edges and soft
        edges with their two face normals, plus the loose mesh's edges.
        ``collect_geometry``'s per-face Python walk took 5 s on the fountain
        (106k triangles) — and the composer paid it on every "Update view"
        and on every frame added; this is tens of milliseconds.

        Returns ``(tris (T,3,3), hard (E,2,3), soft (S,2,3), soft_n
        (S,2,3))``; ``soft_n[:, 1]`` is NaN where a soft edge bounds an
        open surface (no second face).

        ``tags``: also the layer each triangle, hard edge and soft edge is
        inked with on a sheet — its own, else its object's, as
        ``collect_geometry(layers=True)`` tags them — three more object
        arrays ``(…, tri_tags, hard_tags, soft_tags)``. Read from the pick
        index and the chunks (a lazy placement's from the document), never
        from the faces: the walk was 12.8 s of the 16 s of the plaza's
        line drawing by layer."""
        import numpy as np
        from core.layers import DEFAULT_LAYER, layer_of
        scene = self.scene
        idx = self._pick_flat(self._pick_index(near="all"))
        ttags = htags = stags = None
        if getattr(idx, "tri_v0", None) is not None and len(idx.tri_v0):
            keep = idx.ent_vis[idx.tri_ent]
            v0 = idx.tri_v0[keep]
            tris = np.stack([v0, v0 + idx.tri_e1[keep],
                             v0 + idx.tri_e2[keep]], axis=1)
            tris = tris.astype(np.float64)
            if tags:
                # One lookup per entity, not per triangle.
                ents, back = np.unique(idx.tri_ent[keep],
                                       return_inverse=True)
                ttags = DocumentInteraction._cut_layer_tags(idx, ents)[back]
        else:
            tris = np.empty((0, 3, 3))
            if tags:
                ttags = np.empty(0, dtype=object)
        hard_parts: list = []
        htag_parts: list = []
        if getattr(idx, "gedge_a", None) is not None and len(idx.gedge_a):
            gvis = np.array([scene.entity_visible(g)
                             and not getattr(g, "billboard", False)
                             for g in idx.gedge_groups], dtype=bool)
            m = (gvis[idx.gedge_gi] if len(gvis)
                 else np.zeros(len(idx.gedge_a), dtype=bool))
            hard_parts.append(np.stack([idx.gedge_a[m], idx.gedge_b[m]],
                                       axis=1).astype(np.float64))
            if tags:
                # Its object's layer, unless the edge carries one of its own.
                owners = np.array([layer_of(g) for g in idx.gedge_groups]
                                  + [DEFAULT_LAYER], dtype=object)
                t = owners[idx.gedge_gi[m]]
                own = getattr(idx, "gedge_layer", None)
                if own is not None:
                    own = own[m]
                    has = np.array([o is not None for o in own], dtype=bool)
                    t[has] = own[has]
                htag_parts.append(t)
        soft_parts: list = []
        n_parts: list = []
        stag_parts: list = []
        nan3 = (float("nan"),) * 3
        lh: list = []
        lht: list = []
        ls: list = []
        ln: list = []
        lst: list = []
        for e in scene.loose_mesh.edges:
            if getattr(e, "hidden", False) or not scene.entity_visible(e):
                continue
            p0 = (e.v0.position.x(), e.v0.position.y(), e.v0.position.z())
            p1 = (e.v1.position.x(), e.v1.position.y(), e.v1.position.z())
            if getattr(e, "soft", False):
                fs = [f for f in e.faces if scene.entity_visible(f)]
                if not fs:
                    continue
                na = fs[0].normal()
                nb = fs[1].normal() if len(fs) > 1 else None
                ls.append((p0, p1))
                ln.append(((na.x(), na.y(), na.z()),
                           nan3 if nb is None else (nb.x(), nb.y(), nb.z())))
                lst.append(layer_of(e))
            else:
                lh.append((p0, p1))
                lht.append(layer_of(e))
        if lh:
            hard_parts.append(np.asarray(lh, dtype=np.float64))
            htag_parts.append(np.asarray(lht, dtype=object))
        if ls:
            soft_parts.append(np.asarray(ls, dtype=np.float64))
            n_parts.append(np.asarray(ln, dtype=np.float64))
            stag_parts.append(np.asarray(lst, dtype=object))
        for g in self._placements():
            if (getattr(g, "billboard", False)
                    or not scene.entity_visible(g)):
                continue
            ch = self._group_chunk(g)
            sp = ch.get("soft_pts")
            if sp is None or not len(sp):
                continue
            soft_parts.append(
                np.asarray(sp, dtype=np.float64).reshape(-1, 2, 3))
            n1 = np.where(np.asarray(ch["soft_single"])[:, None], np.nan,
                          np.asarray(ch["soft_n1"], dtype=np.float64))
            n_parts.append(np.stack(
                [np.asarray(ch["soft_n0"], dtype=np.float64), n1], axis=1))
            if tags:
                owner = layer_of(g)
                own = ch.get("soft_layer")
                n = len(soft_parts[-1])
                if own is not None and len(own) == n:
                    stag_parts.append(np.array(
                        [o or owner for o in own], dtype=object))
                else:
                    stag_parts.append(np.array([owner] * n, dtype=object))
        hard = (np.concatenate(hard_parts) if hard_parts
                else np.empty((0, 2, 3)))
        soft = (np.concatenate(soft_parts) if soft_parts
                else np.empty((0, 2, 3)))
        soft_n = (np.concatenate(n_parts) if n_parts
                  else np.empty((0, 2, 3)))
        if not tags:
            return tris, hard, soft, soft_n
        htags = (np.concatenate(htag_parts) if htag_parts
                 else np.empty(0, dtype=object))
        stags = (np.concatenate(stag_parts) if stag_parts
                 else np.empty(0, dtype=object))
        return tris, hard, soft, soft_n, ttags, htags, stags

    def hlr_geometry_now(self):
        """:meth:`hlr_geometry` (without tags) with the layers as they are
        switched NOW, for a sheet frame whose scene shows other layers than
        the live model. The pick index behind ``hlr_geometry`` leaves out
        the groups hidden when it was built, so a frame showing them found
        no walls (#1249); the fallback, ``collect_geometry``'s per-face
        walk, took ~20 s on Marco's Queñuamarca plaza (2026-11-12) before
        the first dimension could snap. Here each visible placement's
        cached chunk lends its arrays — a group hidden from the live view
        is baked once, the first time a frame asks for it."""
        import numpy as np
        scene = self.scene
        tris_of = getattr(self, "_tris_of", None) or \
            (lambda f: f.triangulate())
        nan3 = (float("nan"),) * 3
        tri_parts: list = []
        hard_parts: list = []
        soft_parts: list = []
        n_parts: list = []
        lt: list = []
        for f in scene.faces:
            if not scene.entity_visible(f):
                continue
            for t0, t1, t2 in tris_of(f):
                lt.append(((t0.x(), t0.y(), t0.z()), (t1.x(), t1.y(), t1.z()),
                           (t2.x(), t2.y(), t2.z())))
        if lt:
            tri_parts.append(np.asarray(lt, dtype=np.float64))
        lh: list = []
        ls: list = []
        ln: list = []
        for e in scene.loose_mesh.edges:
            if getattr(e, "hidden", False) or not scene.entity_visible(e):
                continue
            p0 = (e.v0.position.x(), e.v0.position.y(), e.v0.position.z())
            p1 = (e.v1.position.x(), e.v1.position.y(), e.v1.position.z())
            if getattr(e, "soft", False):
                fs = [f for f in e.faces if scene.entity_visible(f)]
                if not fs:
                    continue
                na = fs[0].normal()
                nb = fs[1].normal() if len(fs) > 1 else None
                ls.append((p0, p1))
                ln.append(((na.x(), na.y(), na.z()),
                           nan3 if nb is None else (nb.x(), nb.y(), nb.z())))
            else:
                lh.append((p0, p1))
        if lh:
            hard_parts.append(np.asarray(lh, dtype=np.float64))
        if ls:
            soft_parts.append(np.asarray(ls, dtype=np.float64))
            n_parts.append(np.asarray(ln, dtype=np.float64))
        # Component copies not baked yet go in one batch per prototype:
        # baking them one by one (``_instance_chunk``) cost 1.2 s for the
        # 3 400 placements of Marco's plaza on opening its sheet. Same
        # maps, term by term, so the arrays are the ones a bake makes.
        batches: dict = {}
        inst_cache = getattr(self, "_inst_chunks", None) or {}
        # Nested proxies inherit their ancestors' switches. Build them for
        # the sheet, independently of the visible viewport's cached proxies.
        sheet = types.SimpleNamespace(scene=scene)
        placements = (g for root in scene.groups
                      for g in DocumentInteraction._expand_placements(sheet, root))
        for g in placements:
            if (getattr(g, "billboard", False)
                    or not scene.entity_visible(g)):
                continue
            xf = getattr(g, "xform", None)
            if xf is not None and getattr(g, "mesh", None) is not None:
                base = self._group_chunk(
                    _proto_wrapper(self, g.mesh, _paint_of(self, g)))
                cur = inst_cache.get(id(g))
                if cur is None or cur["ikey"] != (
                        base["uid"], tuple(xf.data()), base.get("rev")):
                    batches.setdefault(id(base), (base, []))[1].append(xf)
                    continue
            if xf is None and getattr(g, "owner", None) is not None:
                # A sheet-only proxy has no persistent identity for caching.
                g = _proto_wrapper(self, g.mesh, _paint_of(self, g))
            ch = self._group_chunk(g)
            v0 = ch.get("v0")
            if v0 is not None and len(v0):
                v0 = np.asarray(v0, dtype=np.float64)
                tri_parts.append(np.stack(
                    [v0, v0 + np.asarray(ch["e1"], dtype=np.float64),
                     v0 + np.asarray(ch["e2"], dtype=np.float64)], axis=1))
            if ch.get("edges"):
                hard_parts.append(
                    np.frombuffer(ch["edges"], dtype=np.float32)
                    .reshape(-1, 2, 3).astype(np.float64))
            sp = ch.get("soft_pts")
            if sp is not None and len(sp):
                soft_parts.append(
                    np.asarray(sp, dtype=np.float64).reshape(-1, 2, 3))
                n1 = np.where(np.asarray(ch["soft_single"])[:, None],
                              np.nan,
                              np.asarray(ch["soft_n1"], dtype=np.float64))
                n_parts.append(np.stack(
                    [np.asarray(ch["soft_n0"], dtype=np.float64), n1],
                    axis=1))

        for base, xfs in batches.values():
            _batch_instances(base, xfs, tri_parts, hard_parts, soft_parts,
                             n_parts)

        def cat(parts, shape):
            return np.concatenate(parts) if parts else np.empty(shape)
        return (cat(tri_parts, (0, 3, 3)), cat(hard_parts, (0, 2, 3)),
                cat(soft_parts, (0, 2, 3)), cat(n_parts, (0, 2, 3)))

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
        """``_is_occluded`` for what the overlay draws this frame, read from
        the depth the frame just rendered (``_capture_depth``). The point is
        hidden when the nearest surface at its pixel lies clearly in front
        of it along the view — with a tolerance, so a dimension ending ON a
        face or an edge is not hidden by that same surface. Without a
        snapshot (an export, a failed read-back) it asks the rays."""
        snap = getattr(self, "_depth_snap", None)
        if snap is None:
            if (not self.camera.perspective and _active_cut(self.scene) is None
                    and self._effective_style().face_mode not in ("xray", "wireframe")):
                from ingetrazo_render import parallel_annotation_ray
                forward = self.camera.forward()
                ray = parallel_annotation_ray(self.camera.eye().toTuple(), forward.toTuple(), world.toTuple())
                return ray is not None and self.ray_distance(QVector3D(*ray[0]), forward, ray[1]) is not None
            return self._is_occluded(world)
        buf, mvp, inv, w, h, eye, fwd = snap
        c = mvp.map(QVector4D(world.x(), world.y(), world.z(), 1.0))
        if c.w() <= 1e-9:
            return False
        nx, ny = c.x() / c.w(), c.y() / c.w()
        px, py = (nx + 1.0) * 0.5 * w, (ny + 1.0) * 0.5 * h
        ix, iy = int(px), int(py)
        if not (0 <= ix < w and 0 <= iy < h):
            return False
        # The FARTHEST depth of the 3×3 around the pixel: a point on a
        # silhouette must not be hidden by the face next to it.
        d = float(buf[max(iy - 1, 0):iy + 2, max(ix - 1, 0):ix + 2].max())
        if d >= 1.0:
            return False                       # background: nothing in front
        q = inv.map(QVector4D(nx, ny, d * 2.0 - 1.0, 1.0))
        if abs(q.w()) < 1e-12:
            return False
        surf = QVector3D(q.x() / q.w(), q.y() / q.w(), q.z() / q.w())
        along_pt = QVector3D.dotProduct(world - eye, fwd)
        along_surf = QVector3D.dotProduct(surf - eye, fwd)
        tol = max(0.02, 0.003 * abs(along_pt))
        return along_surf < along_pt - tol

    def _is_occluded(self, world: QVector3D) -> bool:
        """Whether geometry sits between the camera and ``world`` — i.e. the
        point is hidden from the current view. Used to keep snaps from firing
        on edges and vertices the user cannot see.

        Asks the PICK INDEX, which is the only place that knows the whole
        model: it holds the loose mesh AND every group's cached triangles, in
        world space, with per-chunk boxes to prefilter the ray. The old pass
        triangulated ``scene.faces`` alone — the loose mesh — so nothing
        inside a group ever hid anything, and drawing on a box snapped
        straight through it to the edge on the far side (Marco, 2026-08-27).
        Sharing the index also means occlusion and picking can never disagree
        about what is in front.

        Only VISIBLE geometry occludes (``ent_vis``, not ``ent_sel``): an
        object on a locked layer is still solid to the eye, while one on a
        hidden layer must not hide anything. A small epsilon on the far end
        keeps a point lying ON a face — an edge on that face's boundary —
        from being reported as occluded by its own face.

        X-ray and wireframe show everything, so nothing hides a snap there
        (the usual trick: switch to X-ray to dimension the floor of a pool through
        its water). Hidden-line and shaded keep the visible-only rule.

        The active section cut is checked first, in every style: what it
        cuts away is gone, not behind something. Dropping only the edges
        with BOTH ends on the hidden side (``_snap_scene``) left the edge
        that crosses the cut — a wall's vertical, cut halfway up — and its
        hidden end and stretch still snapped (#384).
        """
        sp = _active_cut(self.scene)
        if sp is not None and sp.side(world) > 1e-6:
            return True
        if self._effective_style().face_mode in ("xray", "wireframe"):
            return False
        if (sp is None and hasattr(self, "_lazy_native")
                and self._lazy_native() is not None):
            # Every lazy placement, unbaked, through the batched pass.
            return self._occluded_many([world])[0]
        idx = self._pick_index()
        if getattr(idx, "tri_v0", None) is None \
                and not getattr(idx, "own_spans", None):
            return False
        origin = self.camera.eye()
        delta = world - origin
        dist = delta.length()
        if dist < 1e-9:
            return False
        d = delta / dist
        sp = _active_cut(self.scene)
        if sp is None:
            # Nothing to sort out per face: the single nearest hit decides,
            # and asking for it skips allocating a per-face array on every
            # query (this fires dozens of times a frame).
            nearest = self._ray_hits(idx, origin, d, idx.ent_vis,
                                     reduce_global=True)
            return nearest is not None and nearest < dist - 1e-3
        face_t = self._ray_hits(idx, origin, d, idx.ent_vis)
        if face_t is None:
            return False
        import numpy as np
        hit = np.isfinite(face_t) & (face_t < dist - 1e-3)
        if not hit.any():
            return False
        # An active cut removes what it hides, so a hit on the clipped-away
        # side is not an occluder. A ray meets a planar face at most once, so
        # the per-face nearest hit IS the hit — testing it is exact.
        eye = np.array([origin.x(), origin.y(), origin.z()])
        dv = np.array([d.x(), d.y(), d.z()])
        n = np.array([sp.normal.x(), sp.normal.y(), sp.normal.z()])
        c = float(n @ [sp.point.x(), sp.point.y(), sp.point.z()])
        pts = eye + dv * face_t[hit][:, None]
        return bool(((pts @ n - c) <= 1e-6).any())

    #: Cap on rays × triangles in one NumPy block of ``_occluded_many``
    #: (×3 float64 per array: ~50 MB at most).
    _OCCL_BLOCK = 2_000_000

    def _occluded_many(self, worlds) -> list:
        """``_occluded_many_now``, each point asked once a view: drawing over
        the plant of #158 every pointer move asked again the ~7000 corners
        a roof hides near the cursor, the same ones as the move before, and
        they were 17 ms of it. What a point's ray meets hangs on the eye,
        the model and the occluders the engine holds (``_occluded_memo_key``),
        never on the cursor — only with the engine's placements (#158):
        without them the index bakes what lies near the cursor."""
        import numpy as np
        if not len(worlds):
            return []
        key = self._occluded_memo_key()
        if key is None:
            return self._occluded_many_now(worlds)
        memo = getattr(self, "_occluded_memo", None)
        if memo is None or memo[0] != key or len(memo[1]) > 400_000:
            memo = self._occluded_memo = (key, {})
        known = memo[1]
        if isinstance(worlds, np.ndarray):
            pts = [tuple(p) for p in worlds.reshape(-1, 3).tolist()]
        else:
            worlds = list(worlds)
            pts = [(w.x(), w.y(), w.z()) for w in worlds]
        ask = [i for i, p in enumerate(pts) if p not in known]
        if ask:
            new = self._occluded_many_now(
                np.array([pts[i] for i in ask], dtype=np.float64)
                .reshape(-1, 3))
            for i, h in zip(ask, new):
                known[pts[i]] = bool(h)
        return [known[p] for p in pts]

    def _first_clear(self, pts):
        """The index of the first of ``pts`` (N, 3), in order, that
        ``_occluded_many`` calls seen (-1: none), asked of the native
        engine in one call — the same rays, the same occluders, the rays it
        cannot call bit for bit asked here as before — or None where it
        cannot answer for ``_occluded_many_now``: no engine's placements,
        a section cut (it asks face by face), no static set for the index's
        triangles, or a library whose vector length the rays cannot copy.
        Over the plant of #158 the ~7000 corners a roof hides near the
        cursor took a dozen batches, the memo's bookkeeping and the rays'."""
        import numpy as np
        if not _FIRST_CLEAR or not len(pts):
            return None
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        if nat is None or not hasattr(nat[0], "first_clear"):
            return None
        if _active_cut(self.scene) is not None or not _qv_hypot3():
            return None
        if self._effective_style().face_mode in ("xray", "wireframe"):
            return 0                     # nothing hides anything
        idx = self._pick_index(near="skip")
        stat = None
        if getattr(idx, "tri_v0", None) is not None:
            stat = self._static_occluders(idx)
            if stat is None:
                return None
        iset, _snap, vis = nat
        e = self.camera.eye()
        pts = np.ascontiguousarray(pts, dtype=np.float64).reshape(-1, 3)
        first, doubts = iset.first_clear(
            np.ascontiguousarray(vis, np.int64), (e.x(), e.y(), e.z()), pts,
            stat[0] if stat is not None else None,
            stat[1] if stat is not None else None, _qv_hypot3())
        for j in doubts.tolist():
            if not self._occluded_many_now(pts[j:j + 1])[0]:
                return j
        return int(first)

    def _occluded_memo_key(self):
        """What ``_occluded_many_now``'s answer for a point hangs on, or
        None where it may hang on more (a section cut asks face by face,
        X-ray and wireframe hide nothing, and without the engine's
        placements the index bakes around the cursor)."""
        if not _OCCLUSION_MEMO:
            return None
        if _active_cut(self.scene) is not None:
            return None
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        if nat is None or not len(nat[2]):
            return None
        idx = self._pick_index(near="skip")
        eye = self.camera.eye()
        sc = self.scene
        return (eye.x(), eye.y(), eye.z(), id(idx), id(nat[0]),
                id(sc), sc.version, _cache_ver(self), _content_ver(self),
                self._placements_epoch(), id(sc.edit_group),
                self._rest_is_hidden(), self._effective_style().face_mode)

    def _occluded_many_now(self, worlds) -> list:
        """``_is_occluded`` for several points at once — the snap engine's
        candidates, which all cast from the same eye. One ray at a time,
        each paid for its own span cull and a Möller–Trumbore pass per span
        it met, in small NumPy calls whose overhead was the cost (0.8 ms a
        ray, ~41 rays a hover on a 330 k-face city, 60 % of the inference
        pass). Here the rays share it: one box test for all of them against
        every span, then each span's triangles against the rays that reach
        it. Same rays (built with the same QVector3D arithmetic), same
        acceptance as ``_ray_hits``; with an active section cut — per-face
        rules — it asks ``_is_occluded`` point by point.

        ``worlds`` may also be an (N, 3) array of points, read as the
        QVector3D of each would hold them."""
        import numpy as np
        arr = None
        if isinstance(worlds, np.ndarray):
            arr = worlds.reshape(-1, 3)
            worlds = arr
        else:
            worlds = list(worlds)
        if len(worlds) and _active_cut(self.scene) is not None:
            if arr is not None:
                worlds = [QVector3D(*p) for p in arr.tolist()]
            return [self._is_occluded(w) for w in worlds]   # the cut first (#384)
        out = [False] * len(worlds)
        if not len(worlds) or self._effective_style().face_mode in (
                "xray", "wireframe"):
            return out
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        # Unbaked, the lazy placements need nothing around the cursor here.
        idx = self._pick_index(near="skip" if nat is not None else None)
        own = getattr(idx, "own_spans", None) or ()
        if nat is not None:
            # The lazy placements are asked of the engine, every one the
            # index would take — not only those baked now (#158).
            own = ()
            if not len(nat[2]):
                nat = None
        if getattr(idx, "tri_v0", None) is None and not own and nat is None:
            return out
        origin = self.camera.eye()
        rays = _unit_rays(origin, arr if arr is not None else
                          [(w.x(), w.y(), w.z()) for w in worlds])
        if rays is not None:
            rows, D, dists = rays
        else:
            if arr is not None:
                worlds = [QVector3D(*p) for p in arr.tolist()]
            rows, dirs, dists = [], [], []
            for i, w in enumerate(worlds):
                delta = w - origin
                dist = delta.length()
                if dist < 1e-9:
                    continue
                d = delta / dist
                rows.append(i)
                dirs.append((d.x(), d.y(), d.z()))
                dists.append(dist)
            D = np.array(dirs, dtype=np.float64).reshape(-1, 3)
        if not len(rows):
            return out
        o = np.array([origin.x(), origin.y(), origin.z()])
        best = np.full(len(rows), np.inf)
        settled = None
        stat = (self._static_occluders(idx) if nat is not None
                and getattr(idx, "tri_v0", None) is not None else None)
        if stat is not None:
            # The loose and fixed triangles in the engine too: span by span
            # in NumPy, the tens of thousands of corners a roof hides from
            # far off took a second a pointer move (#158). Only the rays it
            # cannot call bit for bit go through the spans below.
            hid, doubt = stat[0].occluded(
                stat[1], (float(o[0]), float(o[1]), float(o[2])),
                D, np.array(dists) - 1e-3)
            best[hid] = -np.inf
            settled = hid | ~doubt
        whole = None
        if settled is not None and settled.any() and not settled.all():
            # The spans below for the rays the engine left open only: the
            # box tests are rays × spans, and it settles nearly all of them.
            whole = (np.flatnonzero(~settled), D, best, np.asarray(dists))
            D, best = D[whole[0]], best[whole[0]]
            dists, settled = whole[3][whole[0]], None
        spans = ([] if idx.tri_v0 is None
                 or (settled is not None and settled.all()) else
                 list(getattr(idx, "tri_spans", None)
                      or [(None, 0, len(idx.tri_v0))]))
        spans += list(own)
        inf = float("inf")
        boxes = getattr(idx, "_span_boxes", None)
        if not spans:
            boxes = (0, np.empty((0, 3)), np.empty((0, 3)))   # not kept
        elif boxes is None or boxes[0] != len(spans):
            blo = np.array([sp[0][0] if sp[0] is not None else (-inf,) * 3
                            for sp in spans], dtype=np.float64).reshape(-1, 3)
            bhi = np.array([sp[0][1] if sp[0] is not None else (inf,) * 3
                            for sp in spans], dtype=np.float64).reshape(-1, 3)
            boxes = idx._span_boxes = (len(spans), blo, bhi)
        _n, blo, bhi = boxes
        # The rays leave one eye inside a cone; a span whose bounding
        # sphere lies outside it meets none of them, and is left out of the
        # box test below (rays x spans x 3: zoomed out over the plant of
        # #158, hundreds of rays against 1 700 spans, most of them far off
        # to the side). Conservative, so the answer is the same; the
        # regime below is still chosen by the whole count.
        sel = np.arange(len(spans))
        if len(rows) > 1 and len(spans) > 64:
            axis = D.sum(axis=0)
            na = float(np.linalg.norm(axis))
            cos_min = float((D @ axis).min()) / na if na > 0 else -1.0
            fin = np.isfinite(blo).all(axis=1) & np.isfinite(bhi).all(axis=1)
            if cos_min > 0.5 and fin.any():
                a = axis / na
                theta = float(np.arccos(min(cos_min, 1.0)))
                with np.errstate(divide="ignore", invalid="ignore"):
                    c = (blo + bhi) * 0.5 - o           # unboxed: kept anyway
                    rad = (np.linalg.norm(bhi - blo, axis=1) * 0.5
                           * (1 + 1e-6) + 1e-6)
                    L = np.linalg.norm(c, axis=1)
                    ang = np.arccos(np.clip((c @ a) / L, -1.0, 1.0))
                    widen = np.arcsin(np.clip(rad / L, 0.0, 1.0))
                keep = (~fin | (L <= rad)
                        | (ang - widen <= theta + 1e-6))
                if not keep.all():
                    sel = np.flatnonzero(keep)
        many_spans = len(spans) > 64
        blo, bhi = blo[sel], bhi[sel]
        # The box prefilter _ray_hits applies, in both its regimes: the
        # batched slab past 64 spans, a scalar slab (with its tolerance
        # for a ray parallel to an axis) below — they differ on exactly
        # those rays, the standard views'.
        with np.errstate(divide="ignore", invalid="ignore"):
            if many_spans:
                inv_d = 1.0 / np.where(np.abs(D) > 1e-12, D, 1e-12)  # (R, 3)
                t1 = (blo[None, :, :] - o) * inv_d[:, None, :]        # (R, S, 3)
                t2 = (bhi[None, :, :] - o) * inv_d[:, None, :]
                tmin = _slab_far(np.minimum(t1, t2))
                tmax = _slab_near(np.maximum(t1, t2))
                meet = tmax >= np.maximum(tmin, 0.0)
            else:
                par = np.abs(D) < 1e-12                               # (R, 3)
                inv_d = 1.0 / np.where(par, 1.0, D)
                t1 = (blo[None, :, :] - o) * inv_d[:, None, :]
                t2 = (bhi[None, :, :] - o) * inv_d[:, None, :]
                lo_t = np.where(par[:, None, :], -inf, np.minimum(t1, t2))
                hi_t = np.where(par[:, None, :], inf, np.maximum(t1, t2))
                tmin = np.maximum(lo_t.max(axis=2), 0.0)
                tmax = hi_t.min(axis=2)
                inside = (o >= blo - 1e-9) & (o <= bhi + 1e-9)        # (S, 3)
                meet = ((~par[:, None, :] | inside[None, :, :]).all(axis=2)
                        & (tmin <= tmax))
                unboxed = np.array([spans[j][0] is None for j in sel],
                                   dtype=bool)
                meet |= unboxed[None, :]
                tmin = np.where(unboxed[None, :], -inf, tmin)
        limit = np.array(dists) - 1e-3                                # (R,)
        # Only what lies BEFORE the point can hide it: a span whose box the
        # ray enters past the point is skipped (an occluding hit at t < limit
        # enters its box before that), and spans go nearest first so a ray
        # settled as hidden stops asking.
        meet = meet & (tmin < limit[:, None])                         # (R, S)
        if settled is not None:
            meet &= ~settled[:, None]
        ent_mask = idx.ent_vis
        native = _occlusion_native()
        o3 = (float(o[0]), float(o[1]), float(o[2]))
        live_spans = np.flatnonzero(meet.any(axis=0))
        order = np.argsort(np.where(meet[:, live_spans],
                                    tmin[:, live_spans], np.inf).min(axis=0))
        for si in live_spans[order]:
            span = spans[sel[si]]
            if len(span) == 3:
                _bb, s0, n = span
                if not n:
                    continue
                v0 = idx.tri_v0[s0:s0 + n]
                e1 = idx.tri_e1[s0:s0 + n]
                e2 = idx.tri_e2[s0:s0 + n]
                te = idx.tri_ent[s0:s0 + n]
            else:
                _bb, v0, e1, e2, te_local, eoff = span
                if not len(v0):
                    continue
                te = te_local + eoff
            live = ent_mask[te]
            if not live.any():
                continue
            if not live.all():
                v0, e1, e2 = v0[live], e1[live], e2[live]
            r_all = np.flatnonzero(meet[:, si] & (best >= limit))
            if not len(r_all):
                continue
            if native is not None:
                # The same block in the native engine; the rays it cannot
                # call bit for bit (core/occlusion.rs) come back to NumPy
                # below, from where they stood before this span.
                before = best[r_all]
                nb, doubt = native.occlusion_span(
                    o3, np.ascontiguousarray(D[r_all]), before,
                    np.ascontiguousarray(limit[r_all]), v0, e1, e2)
                best[r_all] = np.where(doubt, before, nb)
                r_all = r_all[doubt]
                if not len(r_all):
                    continue
            s = o - v0                                                # (n, 3)
            q = np.cross(s, e1)                                       # (n, 3)
            uq = np.einsum("ij,ij->i", e2, q)                         # (n,)
            step = max(1, self._OCCL_BLOCK // max(len(v0), 1))
            for k in range(0, len(r_all), step):
                r = r_all[k:k + step]
                d = D[r]                                              # (r, 3)
                p = np.cross(d[:, None, :], e2[None, :, :])           # (r, n, 3)
                det = np.einsum("nj,rnj->rn", e1, p)
                ok = np.abs(det) > 1e-6
                inv = np.where(ok, 1.0 / np.where(ok, det, 1.0), 0.0)
                u = np.einsum("nj,rnj->rn", s, p) * inv
                v = (d @ q.T) * inv
                t = uq[None, :] * inv
                hit = (ok & (u >= -_BARY_EPS) & (u <= 1.0 + _BARY_EPS) & (v >= -_BARY_EPS)
                       & (u + v <= 1.0 + _BARY_EPS) & (t > 1e-6))
                tm = np.where(hit, t, np.inf).min(axis=1)
                best[r] = np.minimum(best[r], tm)
        if whole is not None:
            u, D, best_all, dists = whole
            best_all[u] = best
            best = best_all
            limit = dists - 1e-3
        if nat is not None:
            left = np.flatnonzero(best >= limit)
            if len(left):
                iset, _snap, vis = nat
                hid, doubt = iset.occluded(
                    vis, o3, np.ascontiguousarray(D[left]),
                    np.ascontiguousarray(limit[left]))
                best[left[hid]] = -np.inf
                for r in left[doubt]:
                    # Too close to call bit for bit: the block above, on
                    # the triangles the ray may meet, as baked.
                    v0, e1, e2 = iset.ray_tris(vis, o3, tuple(D[r].tolist()),
                                               float(limit[r]))
                    if _occlusion_block(o, D[r:r + 1], v0, e1, e2)[0] \
                            < limit[r]:
                        best[r] = -np.inf
        res = np.zeros(len(worlds), dtype=bool)
        res[np.asarray(rows, dtype=np.intp)] = best < limit
        return res.tolist()

    def pick_face(self, screen_x: float, screen_y: float):
        """Return the face the cursor ray hits, or ``None``.

        Normally that's the front-most face. But when several *coplanar* faces
        overlap at the cursor — e.g. a small rectangle drawn on a larger face
        that didn't subdivide it — the ray hits them at the same depth. In that
        case prefer the smallest one, so push/pull and select grab the inner
        face the user is pointing at instead of the big face behind it (the
        old behaviour silently pushed the whole face)."""
        origin, direction = self._pixel_to_ray(screen_x, screen_y)
        if origin is None or direction is None:
            return None
        import numpy as np
        idx = self._pick_index(near=DocumentInteraction._ray_near(self, screen_x, screen_y))
        hits = self._hover_hits(idx, origin, direction)
        if hits is None or not len(hits.t):
            return None
        face_t = np.where(hits.loose, hits.t, np.inf)
        best_t = face_t.min()
        if not np.isfinite(best_t):
            return None
        eps = self._coplanar_eps(origin, direction, float(best_t))
        cand = np.where(face_t <= best_t + eps)[0]
        if len(cand) == 1:
            return hits.ent(int(cand[0]))[0]
        return hits.ent(int(cand[np.argmin(hits.area[cand])]))[0]

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
        """``pick_face_any`` behind ``which`` (see :meth:`_face_hits`):
        ``(face, owner, placement, mesh)``, memoised per variant."""
        if which == "infer" and (
                self.scene.edit_group is None
                or bool(getattr(self, "_rest_is_hidden",
                                lambda: False)())):
            # At the root (and with the rest hidden) there is nothing outside
            # the context to widen to: the same faces answer, and the hover
            # pays for one ray pass, not two.
            return DocumentInteraction._face_any(self, screen_x, screen_y, "sel")
        idx = self._pick_index(near=DocumentInteraction._ray_near(self, screen_x, screen_y))
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        try:
            cam = self.camera
            from views.array_bake import reading_now
            key = (which, round(screen_x, 2), round(screen_y, 2), id(idx),
                   cam.yaw, cam.pitch, cam.distance, cam.perspective,
                   cam.target.x(), cam.target.y(), cam.target.z(),
                   self.width(), self.height(), reading_now())
        except AttributeError:            # a bare stand-in (tests): no memo
            key = None
        slot = ("_face_any_memo" if which == "sel"
                else "_face_infer_memo")
        cached = getattr(self, slot, None)
        # Keyed by the index itself, not its id(): a rebuilt index can be
        # given the address of the one it replaced.
        if (key is not None and cached is not None and cached[0] == key
                and len(cached) > 3 and cached[2] is idx
                and cached[3] is nat):
            return cached[1]
        result = (None, None, None, None)
        origin, direction = self._pixel_to_ray(screen_x, screen_y)
        if origin is not None and direction is not None:
            import numpy as np
            hits = (self._hover_hits(idx, origin, direction) if which == "sel"
                    else self._infer_hits(idx, origin, direction))
            if hits is not None and len(hits.t):
                face_t = hits.t
                best_t = face_t.min()
                if np.isfinite(best_t):
                    eps = self._coplanar_eps(origin, direction, float(best_t))
                    cand = np.where(face_t <= best_t + eps)[0]
                    if len(cand) == 1:
                        chosen = int(cand[0])
                    else:
                        chosen = int(cand[np.argmin(hits.area[cand])])
                    face, owner = hits.ent(chosen)
                    place = hits.place(chosen)
                    mesh = (getattr(place, "mesh", None)
                            if place is not None else self.scene.mesh)
                    result = (face, owner, place, mesh)
        if key is not None:
            setattr(self, slot, (key, result, idx, nat))
        return result

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
        """Rows of ``hits`` a ray pick lands on: of the faces at the nearest
        depth (see :meth:`_coplanar_eps`), the smallest, with those of the
        same area up to float noise. Empty when nothing was hit. The same
        tiebreak as :meth:`pick_face`, for the picks that answer an object."""
        import numpy as np
        nearest = float(hits.t.min())
        if not np.isfinite(nearest):
            return np.empty(0, np.intp)
        eps = self._coplanar_eps(origin, direction, nearest)
        cand = np.flatnonzero(hits.t <= nearest + eps)
        area = hits.area[cand]
        return cand[area <= float(area.min()) * (1.0 + 1e-6)]

    def pick_group(self, screen_x: float, screen_y: float):
        """The group whose geometry the cursor hits (front-most face, or nearest
        edge for a group that's only lines), or ``None``. Faces sharing a plane
        go to the smallest, as in :meth:`pick_face`: a disc grouped on a wall
        group's face lies at the wall's depth to the last float (#982)."""
        if (self.scene.edit_group is not None
                and not getattr(self.scene.edit_group, "children", None)):
            return None                     # inside a plain group: its content
        origin, direction = self._pixel_to_ray(screen_x, screen_y)
        if origin is not None and direction is not None:
            import numpy as np
            best = None  # (t, group)
            idx = self._pick_index(near=DocumentInteraction._ray_near(self, screen_x, screen_y))
            loose_t = np.inf
            hits = self._hover_hits(idx, origin, direction)
            if hits is not None and len(hits.t):
                loose_t = float(np.where(hits.loose, hits.t, np.inf).min())
                if np.isfinite(np.where(hits.loose, np.inf, hits.t).min()):
                    # A loose face in FRONT hides the group: what the cursor
                    # shows is that face, and every caller asks for the group
                    # it can see (#276, #363, #378 — a loose box before a
                    # component could not be clicked). So does a loose face
                    # ON a group's face, unless the group's is the smaller
                    # (#982); at the same size the loose one is the target.
                    front = DocumentInteraction._smallest_at_front(self, hits, origin,
                                                        direction)
                    best = (float(hits.t[front].min()),
                            None if hits.loose[front].any()
                            else hits.owner(int(front[0])))
            faces_answer = best
            for g in self._context_placements():
                # The figure test first: it is an attribute, the layer test
                # walks tags — asked of every placement on every hover it
                # cost ~90 ms on 21 406 placements (issue #158).
                if not getattr(g, "billboard", False):
                    continue
                if not self.scene.entity_selectable(g):
                    continue                    # hidden or locked layer
                if True:
                    quad = self._billboard_quad(g)
                    if quad is not None:
                        c = quad[0]
                        for tri in ((c[0], c[1], c[2]), (c[0], c[2], c[3])):
                            t = _ray_triangle(origin, direction, *tri)
                            if t is not None and (best is None or t < best[0]):
                                best = (t, self._owner_of(g))
            if best is not None:
                if best is faces_answer:
                    return best[1]
                # A figure: a loose face in front of it or on it hides it.
                eps = self._coplanar_eps(origin, direction, best[0])
                if np.isfinite(loose_t) and loose_t <= best[0] + eps:
                    return None
                return best[1]
        # Edge fallback (cursor over empty space, or a lines-only group):
        # screen-space distance to every group hard edge, batched over the
        # cached projected arrays — the Python per-edge walk took ~300 ms per
        # mouse move against a 300k-edge imported model.
        import numpy as np
        best_d = self.pick_threshold_px
        best_g = None
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        if nat is not None:
            best_d, best_g = self._pick_group_edge_unbaked(
                screen_x, screen_y, nat)
        else:
            # The index first, then its projection (see
            # _nearby_group_edges).
            idx = self._pick_index(near=("px", screen_x, screen_y))
            proj = self._gedge_screen()
        if nat is None and proj is not None:
            ax, ay, bx, by, ok = proj
            if ok.any():
                dx, dy = bx - ax, by - ay
                l2 = dx * dx + dy * dy
                safe = np.where(l2 > 1e-12, l2, 1.0)
                t = np.clip(((screen_x - ax) * dx
                             + (screen_y - ay) * dy) / safe, 0.0, 1.0)
                d = np.hypot(ax + t * dx - screen_x,
                             ay + t * dy - screen_y)
                # The index also carries the edges of the model OUTSIDE the
                # open group (snap targets); those cannot be picked.
                sel = getattr(idx, "gedge_sel", None)
                if sel is not None:
                    ok = ok & sel
                d = np.where(ok, d, np.inf)
                # Only an edge the user can SEE takes the click: the far
                # edge of a slab, hidden behind a loose box standing on it,
                # lies a few pixels from a click on that box and handed the
                # slab the selection. The nearest visible one decides.
                near = np.where(d < best_d)[0]
                for i in near[np.argsort(d[near])]:
                    a = QVector3D(*idx.gedge_a[i])
                    p = a + (QVector3D(*idx.gedge_b[i]) - a) * float(t[i])
                    if not self._is_occluded(p):
                        best_d = float(d[i])
                        best_g = idx.gedge_groups[int(idx.gedge_gi[i])]
                        break
        # Billboard outlines: clicking a figure exactly on its snapped feet
        # point lands ON the quad's boundary, which the ray-triangle test
        # rejects — the outline proximity test catches it.
        for g in self._placements():
            if not getattr(g, "billboard", False):
                continue
            if not self.scene.entity_selectable(g):
                continue
            quad = self._billboard_quad(g)
            if quad is None:
                continue
            c = quad[0]
            for i, j in ((0, 1), (1, 2), (2, 3), (3, 0)):
                pa = self._world_to_pixel(c[i])
                pb = self._world_to_pixel(c[j])
                if pa is None or pb is None:
                    continue
                d2 = _point_to_segment_distance_2d(
                    (screen_x, screen_y), pa, pb)
                if d2 < best_d:
                    best_d = d2
                    best_g = self._owner_of(g)
        return best_g

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
        """Copy the selected faces, edges and groups into the clipboard (as
        positions, with a reference corner). Returns False if nothing is
        selected."""
        faces = [f for f in self.scene.selection if isinstance(f, Face)]
        edges = [e for e in self.scene.selection if isinstance(e, Edge)]
        groups = [g for g in self.scene.selection if isinstance(g, Group)]
        if not faces and not edges and not groups:
            return False
        # Attrs (colour, texture, layer, BIM tag) travel with each face — a
        # deep copy, so later re-paints of the original never touch the
        # clipboard snapshot.
        face_data = [
            ([QVector3D(v) for v in f.vertices],
             [[QVector3D(v) for v in h] for h in f.holes],
             copy.deepcopy(f.attrs) if f.attrs else {})
            for f in faces
        ]
        # Keep soft/curve flags so a pasted circle stays ONE selectable curve
        # (ids are remapped to fresh ones at paste time).
        edge_data = [(QVector3D(e.a), QVector3D(e.b), e.soft, e.curve)
                     for e in edges]
        # Groups are snapshotted NOW (instances keep sharing their prototype;
        # classic groups deep-copy) so editing or deleting the original later
        # never changes what Paste stamps. No preview wireframe is built here
        # any more: Paste previews the groups through the frozen-scratch VBO
        # pipeline (the old per-edge tuple walk took seconds on a leafy
        # plant, and again on EVERY preview frame — the piscina hang).
        group_data = [copy_group(g) for g in groups]
        # A classic group's snapshot becomes an identity INSTANCE of its
        # fresh mesh: every stamp is an O(1) sibling of the clipboard
        # prototype instead of a full deep copy per paste (the 230k-face
        # hedge took seconds per stamp). Component semantics hold: pasted
        # copies share the definition until edited — begin_group_edit
        # already materializes instances on entry. The prototype's chunk is
        # pre-seeded from the SOURCE group's cached entry (content-identical
        # deep copy), so preview and stamps skip the from-scratch rebuild.
        old_clip = getattr(self, "clipboard", None)
        seed = getattr(self, "_seed_proto_chunk", None)
        for src, tpl in zip(groups, group_data):
            if tpl.xform is None:
                tpl.xform = QMatrix4x4()
                # Still a GROUP: the matrix is only how the stamps share
                # until one is edited (issue #90).
                tpl.component = False
                if callable(seed):
                    seed(tpl.mesh, src)
        # Reference corner (what the cursor holds the set by): min corner of
        # everything copied, via NumPy — one pass over vertices instead of
        # four Python min() scans over every edge endpoint.
        import numpy as np
        from core.group import placement_points
        lo = None
        pts = [p for loop, holes, _a in face_data for p in loop]
        pts += [p for _, holes, _a in face_data for h in holes for p in h]
        pts += [p for a, b, _, _ in edge_data for p in (a, b)]
        if pts:
            lo = np.array([[p.x(), p.y(), p.z()] for p in pts]).min(axis=0)
        for g in group_data:
            # The whole placement, children included: a container's own
            # mesh is usually empty, and reading only that found nothing to
            # copy — Ctrl+C on an imported lamp post silently left the
            # previous clipboard in place, and Ctrl+V pasted the pergola
            # (Marco, 2026-09-11).
            arr = placement_points(g)
            if not len(arr):
                continue
            gmin = arr.min(axis=0)
            lo = gmin if lo is None else np.minimum(lo, gmin)
        if lo is None:
            return False
        ref = QVector3D(float(lo[0]), float(lo[1]), float(lo[2]))
        drop = getattr(self, "_drop_clip_protos", None)
        if old_clip and callable(drop):
            drop(old_clip)
        # The tags travel with the copy — the loose edges' (parallel to
        # ``edges``) and the definitions of every tag used, with their
        # folders — so another document can give the pasted geometry its
        # tags instead of names it does not know.
        from core.insert import used_layers
        from core.layers import layer_of, template_for
        edge_layers = [layer_of(e) for e in edges]
        names = used_layers([a for _l, _h, a in face_data], edge_layers,
                            group_data)
        self.clipboard = {"faces": face_data, "edges": edge_data,
                          "registry": {n: m for n, m in getattr(self.scene, "materials", {}).items()
                                       if m.pbr is not None},
                          "edge_layers": edge_layers,
                          "layers": template_for(self.scene, names),
                          "groups": group_data, "ref": ref}
        # Also on the system clipboard, so another IngeTrazo window can
        # paste it (issue #76); encoded only if that window asks.
        if publish and NATIVE:
            from app_shell.files import publish_clipboard
            publish_clipboard(self.window(), self.clipboard)
        elif publish:
            from formats import clip as clip_transfer
            clip_transfer.publish(self.clipboard)
        return True

    def _seed_proto_chunk(self, mesh, src_group) -> None:
        """Pre-seed the prototype-chunk cache of a clipboard snapshot with
        its SOURCE group's cached chunk. The snapshot is a content-identical
        deep copy, so the seeded entry revalidates through the fingerprint
        self-heal (~120 ms) instead of a from-scratch rebuild (~7 s on the
        230k-face hedge). A wrong guess is harmless: a fingerprint mismatch
        just rebuilds."""
        cache = getattr(self, "_group_chunks", None)
        if not cache or getattr(src_group, "xform", None) is not None:
            return
        src_entry = cache.get(id(src_group))
        if src_entry is None:
            return
        from types import SimpleNamespace
        wrappers = getattr(self, "_proto_wrappers", None)
        if wrappers is None:
            wrappers = self._proto_wrappers = {}
        w = wrappers.get(id(mesh))
        if w is None:
            w = wrappers[id(mesh)] = SimpleNamespace(mesh=mesh, xform=None)
        cache[id(w)] = dict(src_entry)

    def _drop_clip_protos(self, clip) -> None:
        """A replaced clipboard's prototype chunks are dead weight (hundreds
        of MB for a big copy) — drop them unless a stamped sibling in the
        scene still shares the mesh."""
        wrappers = getattr(self, "_proto_wrappers", None)
        cache = getattr(self, "_group_chunks", None)
        if not wrappers:
            return
        from core.purge import iter_groups
        live = {id(pg.mesh) for pg in iter_groups(self.scene.groups)}
        for g in clip.get("groups", ()):
            mid = id(g.mesh)
            if mid in live:
                continue
            w = wrappers.pop(mid, None)
            if w is not None and cache:
                cache.pop(id(w), None)

    def cut_selection(self, *, publish=True) -> bool:
        """Copy the selection, then erase it (one undoable step)."""
        copied = self.copy_selection() if publish else self.copy_selection(publish=False)
        if not copied:
            return False
        from core.history import CompoundCommand, DeleteGroupCommand
        faces = [f for f in self.scene.selection if isinstance(f, Face)]
        edges = [e for e in self.scene.selection if isinstance(e, Edge)]
        groups = [g for g in self.scene.selection if isinstance(g, Group)]
        cmds: list = []
        if edges or faces:
            cmds.append(EraseSelectionCommand(edges, faces))
        cmds.extend(DeleteGroupCommand(g) for g in groups)
        if cmds:
            self.history.execute(
                cmds[0] if len(cmds) == 1 else CompoundCommand(cmds))
        self.update()
        return True

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
        if mode not in EDIT_REST_MODES or mode == self._edit_rest_mode:
            return
        self._edit_rest_mode = mode
        from core.settings import settings as QSettings
        QSettings().setValue("display/edit_rest_mode", mode)
        # "hide" keeps the rest out of the VBOs entirely, so switching to or
        # from it changes what is uploaded, not just how it is drawn.
        self._edges_version = -1
        self.update()

    @property
    def edit_rest_fade(self) -> float:
        """0..1 strength of the grey wash applied outside the edit context."""
        return float(getattr(self, "_edit_rest_fade_strength",
                             EDIT_REST_FADE))

    def set_edit_rest_fade(self, strength: float) -> None:
        """Set and persist the rest-of-model fade strength."""
        strength = max(0.0, min(1.0, float(strength)))
        if math.isclose(strength, self.edit_rest_fade, abs_tol=1e-9):
            return
        self._edit_rest_fade_strength = strength
        from core.settings import settings as QSettings
        QSettings().setValue("display/edit_rest_fade", strength)
        self.update()

    def set_hide_similar_components(self, on: bool) -> None:
        """Hide Similar Components: the other instances of the
        component being edited leave the frame. Persisted like the rest
        mode; the version bump re-keys the pick index."""
        on = bool(on)
        from core.settings import settings as QSettings
        QSettings().setValue("display/hide_similar_components",
                             "1" if on else "0")
        if bool(getattr(self.scene, "hide_similar_components", False)) == on:
            return
        self.scene.hide_similar_components = on
        if on:
            for ent in list(self.scene.selection):
                if self.scene.similar_hidden(ent):
                    self.scene.selection.discard(ent)
        self.scene.version += 1
        self._edges_version = -1
        self.update()

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

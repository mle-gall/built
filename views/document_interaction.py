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
import inspect
import re
import sys
import time as _time_mod
import types
from array import array
from pathlib import Path
from typing import Optional
from core.units import fmt_len as _fmt_len
from core.units import typed_value_text as _typed_value_text
from core.units import is_expression
from core.units import parse as parse_units

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

from PySide6.QtCore import (QEvent, QObject, Qt, QPointF, QRectF, QTimer,
                            Signal)
from PySide6.QtGui import QColor, QFont, QFontMetrics, QImage, QPainter, QPen, QPolygonF, QVector2D
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
from tools.rust_tools import selection_mode
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
        from tools.select import _local_bounds as local_bounds
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


def _ray_aabb_span(o, d, lo, hi):
    """``(t_in, t_out)`` of the forward ray (t >= 0) through the AABB, or
    ``None`` if it misses. Plain floats."""
    tmin, tmax = 0.0, float("inf")
    for i in range(3):
        di = d[i]
        if -1e-12 < di < 1e-12:
            if o[i] < lo[i] - 1e-9 or o[i] > hi[i] + 1e-9:
                return None
            continue
        t1 = (lo[i] - o[i]) / di
        t2 = (hi[i] - o[i]) / di
        if t1 > t2:
            t1, t2 = t2, t1
        tmin, tmax = max(tmin, t1), min(tmax, t2)
        if tmin > tmax:
            return None
    return tmin, tmax


def _ray_aabb(o, d, lo, hi, reach=float("inf")) -> bool:
    """Slab test: does the forward ray (``0 <= t <= reach``) touch the AABB?
    Plain floats — the pick prefilter tests ~tens of chunk boxes per ray."""
    tmin = 0.0
    tmax = reach
    for i in range(3):
        di = d[i]
        if -1e-12 < di < 1e-12:
            if o[i] < lo[i] - 1e-9 or o[i] > hi[i] + 1e-9:
                return False
            continue
        inv = 1.0 / di
        t1 = (lo[i] - o[i]) * inv
        t2 = (hi[i] - o[i]) * inv
        if t1 > t2:
            t1, t2 = t2, t1
        if t1 > tmin:
            tmin = t1
        if t2 < tmax:
            tmax = t2
        if tmin > tmax:
            return False
    return True


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

#: The edit box must sit clear of the geometry. A tight box disappears on a
#: cuboid because every dotted segment lands exactly on a model edge. Keep a
#: small screen-space gutter, allowed to grow a little with a large object but
#: capped so a long beam does not acquire a huge empty envelope.
EDIT_BOX_PAD_MIN_PX = 8.0
EDIT_BOX_PAD_MAX_PX = 32.0
EDIT_BOX_PAD_RATIO = 0.05


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
    from PySide6.QtCore import QSettings
    mode = str(QSettings().value("display/edit_rest_mode", "fade"))
    return mode if mode in EDIT_REST_MODES else "fade"


def _load_edit_rest_fade() -> float:
    """Saved strength of the grey edit context, safely clamped to 0..1."""
    from PySide6.QtCore import QSettings
    try:
        value = float(QSettings().value("display/edit_rest_fade",
                                        EDIT_REST_FADE))
    except (TypeError, ValueError):
        value = EDIT_REST_FADE
    return max(0.0, min(1.0, value))


def _load_hide_similar() -> bool:
    from PySide6.QtCore import QSettings
    return str(QSettings().value("display/hide_similar_components",
                                 "0")) == "1"


def _load_invert_wheel() -> bool:
    from PySide6.QtCore import QSettings
    return str(QSettings().value("nav/invert_wheel", "0")) != "0"


def _load_invert_orbit_y() -> bool:
    """Preferences ▸ orbit with the vertical axis reversed.

    Off by default: `OrbitCamera.orbit` grabs the model in both axes, the
    classic way. This is for the hands that learned the other feel —
    including the ones that learned it from IngeTrazo before 0.3.16, when
    the vertical axis was inverted by mistake.
    """
    from PySide6.QtCore import QSettings
    return str(QSettings().value("nav/invert_orbit_y", "0")) != "0"


#: Soft sun shadows (#1281), Preferences ▸ General: off (0) or a level, and
#: how fast the penumbra widens with the distance from what casts it, in
#: world width per world distance. The real sun is about 0.009; the levels
#: are broader, as a picture of soft daylight asks.
SOFT_SHADOW_SPREAD = {1: 0.012, 2: 0.03, 3: 0.06}


def _load_soft_shadows() -> int:
    """The soft shadows level (Preferences ▸ General): 0 = off, the
    default, since it costs the graphics card more while the sun is on."""
    from PySide6.QtCore import QSettings
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
    """The twelve edges of a box (``corners`` in ``oriented_box_corners``
    order) as world-space LINES for the GL pass — so the part behind the
    geometry is hidden like any line, the way guides are (issue #23). The
    edit box used to be painted in the QPainter overlay, which has no depth:
    inside a cube its back edges showed through the faces (Marco, 23-09).

    Whole edges, not dashes: the pattern is the shader's (u_stipple 4,
    measured in window pixels, like Back Edges). Tessellating dashes here
    sized them by camera distance, and the count scaled as
    edge_length/(0.012·distance) — zooming into a large open group pushed
    it past a million segments rebuilt and re-uploaded EVERY frame (the
    «viewport dies when I zoom in close» report: 5.8 s/frame at 5 cm on a
    50 m box)."""
    coords = array("f")
    for i in range(8):
        for bit in (1, 2, 4):
            j = i | bit
            if j == i:
                continue
            a, b = corners[i], corners[j]
            if (b - a).length() < 1e-9:
                continue
            coords.extend([a.x(), a.y(), a.z(), b.x(), b.y(), b.z()])
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


def _point_in_screen_hull(x: float, y: float, points,
                          tolerance_px: float = 1.5) -> bool:
    """Whether a screen point lies in the convex hull of ``points``.

    The projection of a 3D box is a convex polygon (usually a hexagon), not
    its axis-aligned screen rectangle. Testing that hull makes the visible
    dotted frame the honest boundary for leaving group edit: its empty corner
    triangles do not silently count as inside. The small tolerance keeps a
    click directly on a one-pixel dotted edge inside the context.
    """
    pts = sorted({(float(px), float(py)) for px, py in points
                  if math.isfinite(px) and math.isfinite(py)})
    if not pts:
        return False

    def cross(a, b, p):
        return ((b[0] - a[0]) * (p[1] - a[1])
                - (b[1] - a[1]) * (p[0] - a[0]))

    def half(seq):
        out = []
        for p in seq:
            while len(out) >= 2 and cross(out[-2], out[-1], p) <= 0.0:
                out.pop()
            out.append(p)
        return out

    if len(pts) == 1:
        return math.hypot(x - pts[0][0], y - pts[0][1]) <= tolerance_px
    lower, upper = half(pts), half(reversed(pts))
    hull = lower[:-1] + upper[:-1]
    if len(hull) == 2:
        a, b = hull
        dx, dy = b[0] - a[0], b[1] - a[1]
        length2 = dx * dx + dy * dy
        t = (0.0 if length2 <= 1e-12 else
             max(0.0, min(1.0, ((x - a[0]) * dx + (y - a[1]) * dy)
                              / length2)))
        return math.hypot(x - (a[0] + t * dx),
                          y - (a[1] + t * dy)) <= tolerance_px
    point = (float(x), float(y))
    for i, a in enumerate(hull):
        b = hull[(i + 1) % len(hull)]
        edge_len = math.hypot(b[0] - a[0], b[1] - a[1])
        if cross(a, b, point) < -tolerance_px * max(edge_len, 1.0):
            return False
    return True


#: Fewest dashes a guide circle is ever cut into: below this a circle too
#: small for the camera's dash spacing would collapse into a single dash
#: and stop reading as a dashed circle.
_MIN_CIRCLE_DASHES = 12


def _circle_guide_dashes(g, spacing: float, eye=None,
                         base_dist: float = 0.0) -> list:
    """World-space dashes along one guide circle, as a flat
    ``[x,y,z, x,y,z, …]`` float list for the GL vertex stream.

    The circle is analytic; only this display tessellation is polygonal,
    and only into the few chords that keep a dash reading round on screen.
    The dash period follows the camera's spacing like a guide line, but a
    circle too small for that spacing gets proportionally smaller dashes
    (never fewer than ``_MIN_CIRCLE_DASHES``) so it stays visibly dashed.
    """
    r = float(g.radius)
    if r <= 0.0:
        return []
    from core.axes import plane_axes
    u, v = plane_axes(g.normal)
    k = 1.0
    if eye is not None and base_dist > 0.0:
        k = max(1.0, (QVector3D(g.center) - QVector3D(eye)).length()
                / base_dist)
    sp = max(float(spacing), 1e-6) * k
    circ = 2.0 * math.pi * r
    n = min(max(int(circ / sp), _MIN_CIRCLE_DASHES), 4000)
    # Half of each angular period is drawn, half is a gap — the same 1:1
    # dash/gap ratio the guide lines use.
    dash_angle = math.pi / n
    chords = max(1, int(math.ceil(dash_angle / 0.15)))
    out: list = []
    c = QVector3D(g.center)
    for i in range(n):
        t0 = 2.0 * math.pi * i / n
        step = dash_angle / chords
        for j in range(chords):
            a0 = t0 + step * j
            a1 = a0 + step
            pa = c + u * (r * math.cos(a0)) + v * (r * math.sin(a0))
            pb = c + u * (r * math.cos(a1)) + v * (r * math.sin(a1))
            out.extend([pa.x(), pa.y(), pa.z(), pb.x(), pb.y(), pb.z()])
    return out


def _guide_vertices(guides, spacing: float, selection=(), centre=None,
                    reach: float = 0.0, eye=None):
    """Guides as world-space DASHES for the GL pass, so they are hidden by
    the geometry in front of them like any other line (issue #23,
    @pacaeiro: «guide lines are always in front of other objects, even if
    they are behind the object»).

    The QPainter overlay they used to live in has no depth buffer at all —
    it paints after the render, so a guide could only ever be on top. And
    the cheap fix does not exist: asking the pick index per sample point
    (``_is_occluded``, what the snap engine uses) measured **618 µs a ray**
    on the Plaza Yanque model, so ten guides would have cost 200 ms a
    frame. The depth buffer already knows the answer for free.

    A guide is a HALF-MILLION-METRE line (``GUIDE_HALF_LEN``), so it is not
    dashed end to end: the run is centred on the guide's closest point to
    ``centre`` (the camera target) and reaches ``reach`` metres each way,
    with ``spacing`` scaled to the camera distance — the recipe of
    ``_axes_vertices``, so the on-screen density stays put across zoom and
    the dashes converge toward the horizon in perspective. Dashing the
    whole length instead, under any sane cap, gave 27-metre dashes with
    22-metre gaps: at a 16-metre view the guide fell in a gap and vanished.

    With ``eye`` given, each guide LINE scales ``spacing`` and ``reach``
    by how much farther from the eye its looked-at stretch lies than the
    camera's own distance (never less): a guide 30 m away while the view
    sits a metre from a detail drew 3 cm dashes — under a pixel — over
    15 m, and read as faint or gone on big models (#481); and a 2 cm orbit
    distance left after a deep zoom (#579) did the same to every guide.

    Returns ``(coords, spans)``: a flat float array and
    ``[(first_vertex, count, selected)]``, one entry per guide LINE, so the
    caller can colour the selected ones without a second buffer. Guide
    points are not here — they stay screen-space crosses in the overlay.
    """
    import numpy as np
    coords = array("f")
    spans: list[tuple[int, int, bool]] = []
    spacing = max(float(spacing), 1e-6)
    reach = float(reach) if reach > 0 else spacing * 500.0
    dash = spacing * 0.5
    origin = QVector3D(centre) if centre is not None else QVector3D(0, 0, 0)
    sel = {id(g) for g in (selection or ())}
    n = max(1, min(int(2.0 * reach / spacing), 4000))
    # The camera distance the spacing was made for (eye to centre).
    base_dist = ((QVector3D(eye) - origin).length()
                 if eye is not None else 0.0)
    for g in guides or ():
        if getattr(g, "is_circle", False):
            flat = _circle_guide_dashes(g, spacing, eye, base_dist)
            if flat:
                start = len(coords) // 3
                coords.extend(flat)
                count = len(coords) // 3 - start
                if count:
                    spans.append((start, count, id(g) in sel))
            continue
        if not getattr(g, "is_line", False):
            # A guide POINT measured from a vertex draws its guide SEGMENT
            # (Rafael, Revisión 3): finite, dashed the same way, from the
            # vertex to the point. The cross itself stays in the overlay.
            o = getattr(g, "origin", None)
            p = getattr(g, "point", None)
            if o is None or p is None:
                continue
            seg = p - o
            length = seg.length()
            if length < 1e-9:
                continue
            d = seg / length
            start = len(coords) // 3
            t = 0.0
            while t < length:
                a = o + d * t
                b = o + d * min(t + dash, length)
                coords.extend([a.x(), a.y(), a.z(), b.x(), b.y(), b.z()])
                t += spacing
            count = len(coords) // 3 - start
            if count:
                spans.append((start, count, id(g) in sel))
            continue
        d = getattr(g, "direction", None)
        p = getattr(g, "point", None)
        if d is None or p is None:
            continue
        # Foot of the perpendicular from the centre: the stretch of guide
        # the camera is actually looking at.
        tf = QVector3D.dotProduct(origin - p, d)
        g_sp, g_reach, g_dash = spacing, reach, dash
        if eye is not None and base_dist > 0.0:
            k = max(1.0, (p + d * tf - eye).length() / base_dist)
            g_sp, g_reach, g_dash = spacing * k, reach * k, dash * k
        t0 = tf - g_reach
        start = len(coords) // 3
        # The n dashes in one NumPy pass: a Python loop of QVector3D maths
        # here cost ~2 ms a guide on EVERY frame, so forty guides took a
        # frame from 18 to 110 ms (#816).
        ts = t0 + np.arange(n, dtype=np.float64) * g_sp
        pv = np.array((p.x(), p.y(), p.z()), dtype=np.float64)
        dv = np.array((d.x(), d.y(), d.z()), dtype=np.float64)
        seg = np.empty((n, 2, 3), dtype=np.float32)
        seg[:, 0] = pv + ts[:, None] * dv
        seg[:, 1] = pv + (ts + g_dash)[:, None] * dv
        coords.frombytes(seg.tobytes())
        count = len(coords) // 3 - start
        if count:
            spans.append((start, count, id(g) in sel))
    return coords, spans


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


def _is_chord(modifiers) -> bool:
    """Ctrl or Alt held — a shortcut, not typing. Both together is AltGr on
    Windows, which types characters, so that one still types."""
    ctrl = bool(modifiers & Qt.ControlModifier)
    alt = bool(modifiers & Qt.AltModifier)
    return (ctrl or alt or bool(modifiers & Qt.MetaModifier)) \
        and not (ctrl and alt)


def _expression_fields(buffer: str) -> list:
    """The ``;`` fields of a calculation typed in the value box, each one
    starting with ``=`` (#630): ``=3/2;1+0,5`` → ``["=3/2", "=1+0,5"]``.
    A comma followed by a space separates too, as everywhere in the box."""
    fields = re.split(r";|,\s+", buffer.strip())
    return [f.strip() if f.strip().startswith("=") else "=" + f.strip()
            for f in fields]


def _expression_result_text(value, plain: bool) -> str:
    """What a calculation drew, for the value box: ``3.725 m``, or the
    number alone for an angle or a factor (``plain``)."""
    from core.units import localized

    def one(v):
        if plain:
            return localized(f"{v:.6g}")
        return _fmt_len(v)

    if isinstance(value, tuple) and value and value[0] == "abs_len":
        return _fmt_len(value[1])
    if isinstance(value, tuple):
        return "; ".join(one(v) for v in value)
    return one(value)


def _parse_length_field(field: str):
    """One typed length in metres: metric or imperial, the usual forms.

    ``2`` (the document's unit, issue #33) · ``30cm`` · ``1500mm`` ·
    ``1m20`` · ``2"`` / ``2in`` · ``1'`` / ``1ft`` · ``1'6"`` · ``3/4"`` ·
    ``1'3/4"``. A leading minus is kept. Returns None when the field is not
    a length. The reading itself lives in ``core.units.parse``, the one
    every length field shares."""
    from core.units import parse_length_field
    return parse_length_field(field)


class _AltWatch(QObject):
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


class DocumentInteraction(DocumentTools, ViewNavigation):
    """Editing and input shared by the document presentation hosts."""

    def _render_framed_image(self, width_px, overlays, annotations_only,
                             sharp_shadows, transparent):
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
            full = self.render_image(
                max(int(round(out_w * lw / fw)), 64), None, overlays,
                annotations_only, sharp_shadows, transparent, False)
            if full is None:
                return None
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
        return self.render_image(width_px, None, overlays, annotations_only,
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

    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self.view_filter = None
        self.view_tint = None
        self.active_view_key = None
        # The model and its camera are the same in every language: a right-
        # to-left interface (#1067) does not mirror what is drawn here.
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
        # style was changed in this one (views/multi_view.py); None: the
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
                timer = self._nav_detail_timer = QTimer(self)
                timer.setSingleShot(True)
                timer.timeout.connect(self.update)
            timer.start(370)
        if now < getattr(self, "_nav_detail_until", 0.0):
            return True
        return False


    #: This picture's background is alpha 0 (``render_image``'s ``transparent``).
    _export_transparent = False

    #: Off in a view of several (views/multi_view.py) other than the one being
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
        if (_NO_INSTANCING or getattr(g, "xform", None) is None
                or getattr(g, "billboard", False)):
            return False
        # Not the mesh open for editing, which takes the consolidated path
        # like an open top-level group. A nested group or a container, opened,
        # keeps a matrix (its parents': see Scene._bake_container_xform), but
        # everything the instanced caches trust (the proto entry, the pool's
        # box and eligibility, the recorded passes) is keyed on the placements
        # epoch, and the epoch leaves this mesh's edits out on purpose: paint,
        # new faces and moves inside it were not drawn until it was closed.
        scene = getattr(self, "scene", None)
        active = getattr(scene, "mesh", None)
        if (g.mesh is active or (active is not None and
                g.mesh is getattr(active, "_instance_definition", None) and
                g.uid == scene.edit_group.uid)):
            return False
        # A MIRRORED placement turns the prototype's triangles inside out
        # for GL (front becomes back). It used to fall back to the
        # consolidated path, one baked copy per placement — «mirrors are
        # rare», until an industrial model brought 6 203 of them, 4.2
        # million faces baked one by one (issue #158). It draws instanced
        # now, in batches of its own under a clockwise front face
        # (``_front_face``).
        paint = _paint_of(self, g)
        # The answer is the prototype's (and its paint's), which the
        # placements epoch covers: once per prototype, not per placement
        # (64 000 calls per rebuild on the plant of #158, ~1.5 s an undo).
        epoch_of = getattr(self, "_placements_epoch", None)
        memo = None
        if epoch_of is not None:
            epoch = epoch_of()
            memo = getattr(self, "_elig_memo", None)
            if memo is None or memo[0] != epoch:
                memo = self._elig_memo = (epoch, {})
            mkey = (id(g.mesh), _material_sig(paint))
            hit = memo[1].get(mkey)
            if hit is not None and hit[0] is g.mesh:
                return hit[1]
        base = self._proto_base_chunk(g.mesh, paint)
        # Translucent / back-side / glass content still rides the
        # consolidated passes (they need global draw ordering).
        ok = not (base.get("tcol") or base.get("ttex")
                  or base.get("back_vcol") or base.get("back_tex")
                  or base.get("back_tcol") or base.get("back_ttex")
                  or base.get("fvcol"))
        if memo is not None:
            memo[1][mkey] = (g.mesh, ok)
        return ok

    def _proto_base_chunk(self, mesh, paint=None):
        """The prototype's chunk in LOCAL coordinates (the same wrapper
        ``_instance_chunk`` derives from). ``paint`` is the container
        material the variant is baked in (issue #47): a painted instance
        has its own bake of the prototype, unpainted siblings share the
        plain one."""
        return self._group_chunk(_proto_wrapper(self, mesh, paint))

    def _placements_epoch(self):
        """A signature of everything the per-frame placement passes read:
        the group tree (ids, matrices, mesh mutation serials, tags), the
        layer states, the open context and the preview. Passes that used
        to key on the scene version — which every frame of a loose-mesh
        drag bumps — key on this instead, so moving a face beside 1 300
        placements no longer re-gathers, re-signs and re-syncs them all
        (~30 ms a frame on the plaza, 2026-09-14). One walk (~2 ms),
        memoised per tick (a paint or a hover) and scene version."""
        sc = self.scene
        tick = getattr(self, "_tick", 0)
        memo = getattr(self, "_epoch_memo", None)
        if memo is not None and memo[0] == (tick, _cache_ver(self)):
            return memo[1]
        # Across ticks: while the version is live (not frozen by a groups
        # preview, whose matrices move without a version bump) and none of
        # the cheap switches moved, nothing the walk reads can have changed
        # — every edit of a group goes through a command that bumps the
        # version. Orbiting a model of 21 406 placements walked them all on
        # every frame, ~55 ms (issue #158). The CONTENT version: a click
        # (a view-only bump) changes none of what the walk reads either.
        cheap = (getattr(self, "_navigator_revision", 0),
                 getattr(sc, "geometry_version",
                         getattr(sc, "content_version", sc.version)),
                 id(sc.edit_group), id(sc.mesh),
                 getattr(self, "_preview_epoch", 0),
                 bool(getattr(self, "_preview_groups", None)),
                 getattr(self, "_edit_rest_mode", None),
                 bool(getattr(sc, "show_hidden_objects", False)),
                 bool(getattr(sc, "show_hidden_geometry", False)),
                 tuple((ly.key, _effective_layer_visible(self.scene, ly), ly.locked) for ly in sc.layers),
                 _layer_look_sig(sc, self._view_style()),
                 len(sc.groups))
        live = getattr(self, "_frozen_cache_version", None) is None
        same = getattr(self, "_epoch_same", None)
        if live and same is not None and same[0] == cheap:
            self._epoch_memo = ((tick, _cache_ver(self)), same[1])
            return same[1]
        definition = getattr(sc.mesh, "_instance_definition", sc.mesh)
        parts: list = [id(sc.edit_group), id(definition),
                       getattr(self, "_preview_epoch", 0),
                       bool(getattr(self, "_preview_groups", None)),
                       getattr(self, "_edit_rest_mode", None),
                       # View ▸ Hidden Objects / Geometry decide which
                       # placements the pick block lists as selectable. The
                       # per-group ``hidden`` below was in the key, the two
                       # switches were not: Hide rebuilt the block without
                       # the object, and turning the view on left that
                       # block in place — the ghost was drawn and could not
                       # be clicked, only window-selected (issue #53,
                       # @pacaeiro).
                       bool(getattr(sc, "show_hidden_objects", False)),
                       bool(getattr(sc, "show_hidden_geometry", False)),
                       # Hide Similar Components drops placements too.
                       bool(getattr(sc, "hide_similar_components", False)),
                       tuple((ly.key, _effective_layer_visible(self.scene, ly), ly.locked) for ly in sc.layers),
                       # Colour by layer and each layer's colour and
                       # transparency change the paint a placement draws in.
                       _layer_look_sig(sc, self._view_style()),
                       getattr(self, "_navigator_revision", 0)]

        loose = sc.mesh

        def part(g):
            m = g.mesh
            xf = g.xform
            # The mesh open for editing IS the loose mesh: its content is
            # the version-keyed half (rebuilt per frame of a drag), not the
            # placements half — counting its serial here re-keyed every
            # cache on every drag frame (81 ms a frame instead of 56).
            serial = (getattr(m, "_mut_serial", None)
                      if m is not None and m is not loose else None)
            # ``locked`` too: the pick block lists a group as selectable or
            # not, and a padlock that did not re-key it left the group
            # movable and openable from the view.
            return (id(g), id(m), serial,
                    tuple(xf.data()) if xf is not None else None,
                    g.layer, bool(g.billboard), bool(g.hidden),
                    bool(getattr(g, "locked", False)),
                    _material_sig(getattr(g, "material", None)),
                    # Empty container meshes can be shared by unrelated
                    # assemblies. Opening one must not invalidate the
                    # entire other assembly just because its empty mesh
                    # becomes the active mesh. Never expand a lazy mesh.
                    (m is not None and "_deferred" not in m.__dict__
                     and not m.edges and not m.faces))

        pv = getattr(self, "_preview_groups", None) or ()
        base = getattr(self, "_epoch_frozen_base", None)
        if (not live and pv and base is not None and base[0] == cheap
                and base[1] == parts):
            # A frozen preview (a drag): only the previewed groups can
            # move, so only their parts are made again — the walk over
            # every placement ran on every frame (21 406 of them, #158).
            # The same parts in the same order: the very same epoch.
            whole = base[2]
            for i, g in base[3]:
                whole[i] = part(g)
            epoch = hash(tuple(whole))
            self._epoch_remember(epoch, len(base[1]), list(whole), base[4])
            self._epoch_memo = ((tick, _cache_ver(self)), epoch)
            self._epoch_same = None
            return epoch
        movers: list = []
        head = len(parts)

        def walk(g):
            if id(g) in pv:
                movers.append((len(parts), g))
            parts.append(part(g))
            for c in (g.children or ()):
                walk(c)
        # Where each top-level object's subtree sits in ``parts``: what
        # ``_epoch_kept_tops`` compares, object by object.
        tops: list = []
        for g in sc.groups:
            start = len(parts)
            walk(g)
            tops.append((id(g), start, len(parts)))
        epoch = hash(tuple(parts))
        self._epoch_remember(epoch, head, parts, tops)
        # (cheap key, the parts before the walk, every part, the movers'
        # slots, the subtrees) — what the next frozen frame starts from.
        self._epoch_frozen_base = ((cheap, parts[:head], list(parts), movers,
                                    tops)
                                   if not live and pv else None)
        self._epoch_memo = ((tick, _cache_ver(self)), epoch)
        self._epoch_same = (cheap, epoch) if live else None
        return epoch

    #: How many past epochs keep their parts for ``_epoch_only_matrices``.
    _EPOCH_HISTORY = 4

    def _epoch_remember(self, epoch, head, parts, tops=()) -> None:
        """Keep the parts an epoch was made of, for the few last ones."""
        hist = getattr(self, "_epoch_hist", None)
        if hist is None:
            from collections import OrderedDict
            hist = self._epoch_hist = OrderedDict()
        hist[epoch] = (head, parts, tops)
        hist.move_to_end(epoch)
        while len(hist) > self._EPOCH_HISTORY:
            hist.popitem(last=False)

    def _epoch_only_matrices(self, old, new=None) -> bool:
        """Whether, from placements epoch ``old`` to ``new`` (the current
        one by default), only group matrices and the preview changed — the
        same groups in the same order, with the same meshes, layers, flags
        and paints — as far as the component placements go: a plain group
        (no matrix) whose mesh no placement draws may have been edited too.
        A Move picks an object up (the preview) and puts it down (one
        matrix, or a plain group's moved vertices): the per-placement
        passes that rebuilt every placement for that (~1 s each on the
        21 406 of #158) can update the rows whose matrices moved instead.
        ``False`` when unsure."""
        if new is None:
            new = self._placements_epoch()
        if old == new:
            return True
        hist = getattr(self, "_epoch_hist", None) or {}
        a, b = hist.get(old), hist.get(new)
        if a is None or b is None or a[0] != b[0] or len(a[1]) != len(b[1]):
            return False
        head = a[0]
        pa, pb = a[1], b[1]
        # The head: context, loose mesh, preview, view switches, layers;
        # the preview's two slots (2, 3) are the caller's to handle.
        for i in range(head):
            if i not in (2, 3) and pa[i] != pb[i]:
                return False
        edited = set()
        for x, y in zip(pa[head:], pb[head:]):
            if x is not y and x != y and (x[:2] != y[:2] or x[4:] != y[4:]):
                return False
            # Gaining or losing a matrix is no move: the group changes draw
            # path. A classic group made a component kept the old instanced
            # pool, which never held it, while the static pass skipped it as
            # instanced — it vanished from the view.
            if (x[3] is None) != (y[3] is None):
                return False
            if x[2] != y[2]:
                if x[3] is not None or y[3] is not None:
                    return False          # a prototype's mesh: not a move
                edited.add(x[1])
        if edited and any(y[1] in edited for y in pb[head:]
                          if y[3] is not None):
            return False
        return True

    def _epoch_kept_tops(self, old, new=None, ctx=False):
        """The ids of the top-level objects whose whole subtree — every
        placement in it, with its mesh, matrix, layer, flags and paint —
        reads the same in placements epoch ``old`` as in ``new`` (the
        current one by default), the rest of the scene (context, loose
        mesh, view switches, layers) being the same too; the preview's two
        slots are the caller's to handle, as in ``_epoch_only_matrices``.
        ``None`` when that cannot be told.

        Deleting, pasting or undoing either adds or takes away an object
        or two: the per-placement passes can keep the rows of every other
        object instead of making all 21 406 again (~2 s each way on the
        plant of #158).

        ``ctx``: for passes that read only what a placement IS (its mesh,
        matrix, box), not what the open context makes of it — a group
        opened or closed changes the context and the loose mesh, and they
        made all 21 406 rows again (~0.9 s each way). The objects with a
        placement of the loose mesh, old or new (the one being edited,
        every copy of a component being edited), are not kept; nor
        anything under Hide Similar Components, which hides by context."""
        if new is None:
            new = self._placements_epoch()
        memo = getattr(self, "_kept_tops_memo", None)
        if memo is not None and memo[0] == (old, new, ctx):
            return memo[1]
        hist = getattr(self, "_epoch_hist", None) or {}
        a, b = hist.get(old), hist.get(new)
        out = None
        free = (0, 1, 2, 3) if ctx else (2, 3)
        if (a is not None and b is not None and len(a) > 2 and len(b) > 2
                and a[0] == b[0]
                and all(i in free or a[1][i] == b[1][i]
                        for i in range(a[0]))):
            pa, pb = a[1], b[1]
            moved = a[0] > 1 and (pa[0] != pb[0] or pa[1] != pb[1])
            if moved and (a[0] <= 7 or pa[7] or pb[7]):
                self._kept_tops_memo = ((old, new, ctx), None)
                return None
            loose = {pa[1], pb[1]} if moved else ()
            was = {t: (s, e) for t, s, e in a[2]}
            kept = set()
            for t, s, e in b[2]:
                ab = was.get(t)
                if ab is None or ab[1] - ab[0] != e - s:
                    continue
                o = ab[0]
                for k in range(s, e):
                    x, y = pa[o + k - s], pb[k]
                    empty = len(x) > 9 and len(y) > 9 and x[9] and y[9]
                    if x is not y and x != y:
                        # The active mesh's serial is intentionally None.
                        # For two empty meshes that change alone has no
                        # effect on geometry, eligibility or bounds.
                        if not (empty and x[:2] == y[:2] and x[3:] == y[3:]):
                            break
                    if loose and y[1] in loose and not empty:
                        break
                else:
                    kept.add(t)
            out = frozenset(kept)
        self._kept_tops_memo = ((old, new, ctx), out)
        return out

    def _refilter(self, old_epoch, old_out, source, test, old_source=None,
                  ctx=False):
        """``[g for g in source if test(g)]`` worked out only for the
        placements of objects that changed since placements epoch
        ``old_epoch``, whose answer was ``old_out``: an unchanged object's
        placements are in it exactly when they were. ``old_source``: the
        ids the old answer was taken from, when not every placement.
        ``None`` when ``_epoch_kept_tops`` cannot tell; ``ctx`` as there,
        for a ``test`` that reads what a placement is, not the context."""
        kept = self._epoch_kept_tops(old_epoch, ctx=ctx)
        if kept is None:
            return None
        was = {id(g) for g in old_out}
        top_of = self._placement_top
        if old_source is not None:
            src = set(old_source)
            return [g for g in source
                    if (id(g) in was if top_of(g) in kept and id(g) in src
                        else test(g))]
        return [g for g in source
                if (id(g) in was if top_of(g) in kept else test(g))]

    def _placement_top(self, g):
        """The id of the top-level object placement ``g`` belongs to (the
        one ``_placements`` expanded it from)."""
        tops = getattr(self, "_placement_tops", None)
        return tops.get(id(g), id(g)) if tops else id(g)

    def _placements(self):
        """``scene.groups`` with every nested placement expanded into a proxy
        group carrying its composed world matrix.

        Drawing and picking want one entry per PLACEMENT — each renders its
        own prototype through its own matrix, which is the whole point of
        keeping a component's internal sharing. The USER wants one entry per
        top-level object. The proxies bridge that: they look exactly like
        ordinary component instances, so no draw or pick path needs a special
        case, and ``owner`` maps a hit back to the group in ``scene.groups``.

        Proxies are cached per child object and mutated in place: a fresh
        object per frame would miss ``_instance_chunk``'s cache (keyed on
        ``id(group)``) and re-transform every instance's arrays every frame.
        A scene with no nested placements returns ``scene.groups`` itself."""
        groups = self.scene.groups
        ctx = self.scene.edit_group
        # The expansion only changes with the placements (their matrices
        # included, in the epoch): rebuilt twice a frame it cost ~65 ms on
        # a model of 21 406 placements (issue #158).
        epoch_of = getattr(self, "_placements_epoch", None)
        key = (epoch_of(), id(groups), len(groups)) if epoch_of else None
        memo = getattr(self, "_placements_memo", None)
        if key is not None and memo is not None and memo[0] == key:
            return memo[1]
        allowed = getattr(self.scene, "_navigator_allowed", None)
        anidado = ctx is not None and getattr(ctx, "children", None)
        if not anidado and not any(getattr(g, "children", None) for g in groups):
            shown = groups if allowed is None else [g for g in groups if self.scene.navigator_key(g) in allowed]
            self._placements_memo = (key, shown)
            return shown                       # unchanged for unfiltered flat scenes
        cache = getattr(self, "_placement_proxies", None)
        if cache is None:
            cache = self._placement_proxies = {}
        out: list = []
        seen: set = set()
        from core.doc_model import NATIVE
        handles, parents, *_ = NATIVE.doc_tree(groups)
        DocumentInteraction._bind_placement_rows(self, handles, parents.tolist(), out, seen)
        # Proxies are pinned while a preview is running: the movers' ids are
        # what the draw passes skip on, and dropping one mid-drag would make
        # a fresh object (chunk cache miss) that the preview no longer knows.
        if len(cache) > len(seen) and not getattr(self, "_preview_groups",
                                                  None):
            tops = getattr(self, "_placement_tops", None) or {}
            for k in [k for k in cache if k not in seen]:
                gone = cache.pop(k, None)
                if gone is not None:
                    tops.pop(id(gone), None)
        # Filter after expansion: a matching child can remain visible without
        # its ancestor. Ghost passes also consume this list, so hidden-object
        # display cannot bring excluded placements back into the viewport.
        if allowed is not None:
            out = [g for g in out if self.scene.navigator_key(g) in allowed]
        self._placements_memo = (key, out)
        return out

    def _expand_placements(self, group, out=None, seen=None, above=None):
        """``group`` followed by its nested placements, as stable proxies.

        Used both for the whole scene (``_placements``) and for ONE group's
        subtree — a caller that used to treat a group as "its mesh" has to
        take the subtree instead, or a component drags an empty ghost while
        its children sit frozen at the old spot. ``above`` is the paint
        ``group`` sits in, when it is not a top-level object."""
        from core.doc_model import NATIVE
        if out is None:
            out = []
        if not getattr(group, "children", None):
            out.append(group)
            return out
        handles, parents, *_ = NATIVE.doc_tree([group])
        return DocumentInteraction._bind_placement_rows(self, handles, parents.tolist(), out, seen, above)

    def _bind_placement_rows(self, handles, parents, out, seen=None, above=None):
        """Bind native traversal rows to the editor's persistent pick and paint handles."""
        cache = getattr(self, "_placement_proxies", None)
        if cache is None:
            cache = self._placement_proxies = {}
        tops = getattr(self, "_placement_tops", None)
        if tops is None:
            tops = self._placement_tops = {}
        ctx = self.scene.edit_group
        # Keep inherited paint, visibility and edit ownership for rows with children.
        ancestors = {}
        for i, (child, parent) in enumerate(zip(handles, parents)):
            has_children = i + 1 < len(parents) and parents[i + 1] == i
            if parent < 0:
                root = child
                out.append(root)
                if has_children:
                    vis, locked = self.scene._layer_state(root)
                    forced = root.layer if not vis or locked else None
                    ancestors[i] = (root.xform, None, bool(root.hidden), forced,
                                    root.layer, root.material or above)
                continue
            world, inside, hidden, forced, inherited, paint = ancestors[parent]
            local = child.xform
            world = local if world is None else (world * local if local is not None else world)
            proxy = cache.get(id(child))
            if proxy is None:
                proxy = cache[id(child)] = PlacementProxy(child.mesh, name=child.name, uid=child.uid)
            if seen is not None:
                seen.add(id(child))
            tops[id(proxy)] = id(root)
            proxy.mesh = child.mesh
            proxy.source_occurrence = child
            proxy.uid = self.scene.navigator_key(child)
            own = None if is_link_untagged(child.layer) else child.layer
            proxy.layer = forced or own or inherited or handles[parent].layer or child.layer
            proxy.billboard = child.billboard
            proxy.hidden = hidden or bool(child.hidden)
            proxy.material = child.material or paint
            if ctx is not None and handles[parent] is ctx:
                inside = child
            if child is ctx or inside is not None:
                proxy.owner, proxy.context = (child if child is ctx else inside), ctx
            else:
                proxy.owner, proxy.context = root, None
            proxy.xform = world
            out.append(proxy)
            if has_children:
                if forced is None and proxy.layer:
                    vis, locked = self.scene._layer_state(proxy)
                    if not vis or locked:
                        forced = proxy.layer
                ancestors[i] = (world, inside, proxy.hidden, forced, proxy.layer, proxy.material)
        return out

    def prepare_drawing(self, progress=None) -> None:
        """Build the drawing data of every group and prototype now, with
        ``progress(fraction, text)`` called a few times a second — what the
        first paint would otherwise do in one go: 55 s on a model of 21 406
        placements, long enough for the desktop to call IngeTrazo «not
        responding» (Marco, 01-10, #158). Pure NumPy, no GL needed."""
        import time
        groups = self._placements()
        n = max(1, len(groups))
        from views import array_bake
        paints = [_paint_of(self, g) for g in groups]
        array_bake.restore_cached(self, ((g.mesh, paint)
                                         for g, paint in zip(groups, paints)),
                                  progress=(lambda f, t: progress(0.65 * f, t))
                                  if progress is not None else None)
        if progress is not None:
            progress(0.65, "Preparing the drawing…")
        array_bake.prefetch(g.mesh for g in groups)
        # Bake side by side what the drawing cache did not restore (a
        # restored bake is staged on its mesh and needs no work).
        from core.materials import material_sig
        painted = [(g, p) for g, p in zip(groups, paints)
                   if not getattr(g, "billboard", False)
                   and not getattr(g.mesh, "__dict__", {}).get(
                       "_drawing_staged")]
        array_bake.prebake(self, ((g.mesh, p, material_sig(p))
                                  for g, p in painted))
        if progress is not None:
            progress(0.80, "Preparing the drawing…")
        last = time.monotonic()
        # Eligibility depends on the prototype and paint, not the placement
        # matrix. Check it once per variant during this preparation pass.
        done: dict = {}
        for i, (g, paint) in enumerate(zip(groups, paints)):
            if getattr(g, "billboard", False):
                continue
            try:
                if getattr(g, "xform", None) is not None:
                    key = (id(g.mesh), _material_sig(paint))
                    if key not in done:
                        self._proto_base_chunk(g.mesh, paint)
                        done[key] = self._instanced_eligible(g)
                    if not done[key]:
                        self._group_chunk(g)
                else:
                    self._group_chunk(g)
            except Exception:                  # noqa: BLE001 - paint retries
                pass
            now = time.monotonic()
            if progress is not None and now - last > 0.25:
                progress(0.80 + 0.20 * (i + 1) / n, "Preparing the drawing…")
                last = now
        self._prebaked = None                # those no placement asked for
        if progress is not None:
            progress(1.0, "Preparing the drawing…")

    def _placement_bbox(self, g):
        """World AABB of an instanced placement from its PROTOTYPE's local
        box and the placement matrix — eight corners, no bake. Reading it
        off ``_group_chunk(g)`` baked the whole placement to world
        coordinates only to take its box: on the model of issue #158 that
        was 21 000 baked copies of geometry the instanced pass never uses."""
        return self._placement_frame(g)[0]

    def _placement_frame(self, g):
        """``(world bbox, inverse matrix)`` of an instanced placement,
        cached per placement and keyed on its matrix and its prototype's
        bake: the silhouette pass asks for both for every placement at up
        to 12 Hz, and working them out in Python each time cost ~7 ms a
        pass on the plaza's 1 359 placements (release check, 30-09)."""
        base = self._proto_base_chunk(g.mesh, _paint_of(self, g))
        m = g.xform
        key = (base.get("uid"), tuple(m.data()))
        cache = getattr(self, "_placement_frames", None)
        if cache is None:
            cache = self._placement_frames = {}
        hit = cache.get(id(g))
        if hit is not None and hit[0] == key:
            return hit[1]
        bb = base.get("bbox")
        box = None
        if bb:
            (x0, y0, z0), (x1, y1, z1) = bb
            pts = [m.map(QVector3D(x, y, z))
                   for x in (x0, x1) for y in (y0, y1) for z in (z0, z1)]
            box = ((min(p.x() for p in pts), min(p.y() for p in pts),
                    min(p.z() for p in pts)),
                   (max(p.x() for p in pts), max(p.y() for p in pts),
                    max(p.z() for p in pts)))
        inv, ok = m.inverted()
        # The instance pool is not assigned until its first build finishes.
        # Sizing this cache from that pool capped a 100k-placement import
        # at 256 rows during opening, discarding the bounds just computed
        # and making the first pick compute them all again.
        placements = getattr(self, "_placements_memo", None)
        population = (len(placements[1]) if placements is not None
                      else len(self.scene.groups))
        if len(cache) > 4 * max(64, population):
            cache.clear()                  # ids of placements long gone
        val = (box, inv if ok else None)
        cache[id(g)] = (key, val)
        return val

    #: How far past the view the instanced batches reach, as a fraction of
    #: its width and height, so an orbit keeps drawing the same ones
    #: (_gather_instanced).
    _INST_VIEW_MARGIN = 0.10
    #: ... and while they hold at most this many times what is in view.
    _INST_SUPERSET_MAX = 2

    @staticmethod
    def _widened_planes(planes, frac):
        """The six planes of ``_frustum_planes`` with the four sides moved
        out by ``frac`` of the view on each side — the clip test |x|, |y|
        ≤ w becomes ≤ (1 + 2·frac)·w — and no near plane (what is behind
        the eye is clipped anyway)."""
        # Each side is r3 ± row: half the sum of a pair is r3 itself.
        r3 = [(a + b) * 0.5 for a, b in zip(planes[0], planes[1])]
        k = 2.0 * frac
        out = [tuple(c + k * r for c, r in zip(pl, r3)) for pl in planes[:4]]
        out.append(planes[5])                                       # far
        return out

    @staticmethod
    def _instances_in_frustum(lo, hi, planes, margin=0.0):
        """Mask of the boxes ``lo``/``hi`` not wholly outside one of the
        frustum ``planes`` — moved ``margin`` world units outward.

        A box whose bounds are not finite is always kept: ±inf is what a
        placement with no known extent is stored as, a NaN can creep into
        a huge model's coordinates, and neither can be proven outside —
        the rule ``_bounding_spheres`` and ``_visible_spans`` already
        keep for unknown extents. Running the products on them anyway
        multiplied inf by a zero plane component (``invalid value
        encountered in multiply``, issue #952) and let the NaN comparison
        drop the box."""
        import numpy as np
        lo = np.asarray(lo)
        hi = np.asarray(hi)
        keep = np.ones(len(lo), dtype=bool)
        finite = np.isfinite(lo).all(axis=1) & np.isfinite(hi).all(axis=1)
        idx = np.flatnonzero(finite)
        if not len(idx):
            return keep
        flo, fhi = lo[idx], hi[idx]
        fkeep = np.ones(len(idx), dtype=bool)
        # Same p-vertex test as ``_aabb_visible``, all boxes at once — a
        # plane at a time on the box columns (the (planes, boxes, 3) array
        # it used to build was most of a frame's gather, #158); the same
        # products and sums, in the same order.
        for pl in planes:
            nx, ny, nz, d = (float(c) for c in pl)
            if margin:
                d += margin * (nx * nx + ny * ny + nz * nz) ** 0.5
            px = fhi[:, 0] if nx >= 0.0 else flo[:, 0]
            py = fhi[:, 1] if ny >= 0.0 else flo[:, 1]
            pz = fhi[:, 2] if nz >= 0.0 else flo[:, 2]
            fkeep &= ((px * nx + py * ny) + pz * nz) + d >= 0.0
        keep[idx] = fkeep
        return keep

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
        """Centres and radii of boxes ``lo``/``hi``; a box with an unknown
        (infinite) extent gets an infinite radius: it is never «small»."""
        import numpy as np
        with np.errstate(invalid="ignore"):
            ctr = (lo + hi) * 0.5
            rad = np.linalg.norm(hi - lo, axis=1) * 0.5
        known = np.isfinite(ctr).all(axis=1) & np.isfinite(rad)
        return np.where(known[:, None], ctr, 0.0), np.where(known, rad, np.inf)

    def _spheres_below_a_pixel(self, ctr, rad, floor=None):
        """Mask of the spheres that span under ``floor`` (by default
        ``_INST_MIN_PX``) framebuffer pixels across on this frame."""
        import numpy as np
        h = (self._export_size or self._fb_size())[1]
        m = self._np_mvp()
        # pixels per world unit at w = 1: the projection's own y scale
        sy = abs(self.camera.projection_matrix().data()[5]) * h * 0.5
        w = ctr @ m[3, :3] + m[3, 3]
        if self.camera.perspective:
            w = w - rad                         # the sphere's nearest point
        with np.errstate(invalid="ignore", divide="ignore"):
            px = 2.0 * rad * sy / np.where(w > 1e-9, w, np.nan)
        if floor is None:
            floor = self._INST_MIN_PX
        return np.isfinite(px) & (px < floor)

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
        DocumentInteraction._drop_past_cut(self, hits, origin, direction)
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
            img = QImage(path)
            if img.isNull():
                c = (0.78, 0.78, 0.78)
            else:
                small = img.scaled(4, 4, Qt.IgnoreAspectRatio,
                                   Qt.SmoothTransformation)
                rs = gs = bs = n = 0
                for yy in range(small.height()):
                    for xx in range(small.width()):
                        px = small.pixelColor(xx, yy)
                        rs += px.red()
                        gs += px.green()
                        bs += px.blue()
                        n += 1
                c = ((rs / (255.0 * n), gs / (255.0 * n), bs / (255.0 * n))
                     if n else (0.78, 0.78, 0.78))
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
        from PySide6.QtCore import QByteArray
        from PySide6.QtGui import QImage
        img = QImage()
        return img if img.loadFromData(QByteArray(data)) else None

    def _schedule_tile_requests(self) -> None:
        """Start missing tile requests after the current GL paint returns."""
        if self._tile_requests_scheduled:
            return
        self._tile_requests_scheduled = True
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
        self.sceneVersionChanged.emit(self.scene.version)
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
        QTimer.singleShot(0, self, self._announce_queued_version)

    def _announce_queued_version(self) -> None:
        self._announce_queued = False
        self.sceneVersionChanged.emit(self._version_to_announce)

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
        """What the ScreenTip adds after the snap's own name: the geometry
        is inside a group / component, or — while one is open — outside the
        one being edited. Empty when there is nothing to say (the open
        context's own geometry, or the root)."""
        if context == "component":
            return " " + tr("in component")
        if context == "group":
            return " " + tr("in group")
        if context == "outside":
            # ``edit_group`` is already on its world copy (xform None) while
            # it is open: ask the shared definition, not is_instance().
            is_comp = (self.scene.edit_definition() is not None
                       if self.scene.edit_group is not None else None)
            word = ("outside the component" if is_comp
                    else "outside the group")
            return " " + tr(word)
        return ""

    def _draw_snap_indicator(self, painter: QPainter, snap: SnapResult) -> None:
        # Axis-lock and inference state is conveyed by the coloured rubber
        # band; no badge follows the cursor along the lock line. Only the
        # discrete point snaps get a marker.
        if snap.kind in ("axis_inference", "axis"):
            return
        pixel = self._world_to_pixel(snap.point)
        if pixel is None:
            return
        r, g, b = snap.color
        color = QColor.fromRgbF(r, g, b, 1.0)
        # Dashed guide line (the extension inference shows the edge's dashed
        # continuation to the cursor).
        if snap.guide is not None:
            gp0 = self._world_to_pixel(snap.guide[0])
            gp1 = self._world_to_pixel(snap.guide[1])
            if gp0 is not None and gp1 is not None:
                # A snap can colour its guide differently from the marker (the
                # 'from point' guide is axis-coloured while the point is green).
                gc = snap.guide_color if snap.guide_color is not None else (r, g, b)
                dash = QPen(QColor.fromRgbF(gc[0], gc[1], gc[2], 0.9), 2.0, Qt.DashLine)
                painter.setPen(dash)
                vis = self._clip_pixel_line(gp0, gp1)
                if vis is not None:
                    painter.drawLine(QPointF(*vis[0]), QPointF(*vis[1]))
        for ga, gb_, gc in (snap.guides or []):
            gp0 = self._world_to_pixel(ga)
            gp1 = self._world_to_pixel(gb_)
            if gp0 is not None and gp1 is not None:
                painter.setPen(QPen(QColor.fromRgbF(gc[0], gc[1], gc[2], 0.9), 2.0, Qt.DashLine))
                vis = self._clip_pixel_line(gp0, gp1)
                if vis is not None:
                    painter.drawLine(QPointF(*vis[0]), QPointF(*vis[1]))
        # A white halo under the marker lifts it off busy geometry, then
        # the coloured marker on top — bigger and bolder than before so the
        # snap point reads at a glance (a common request: the dots were too
        # small to aim with).
        halo = QPen(QColor(255, 255, 255, 230), 4.5)
        mark = QPen(color, 2.6)
        painter.setBrush(QColor.fromRgbF(r, g, b, 0.30))
        px, py = pixel
        if snap.kind == "intersection":
            # X marker at the crossing (drawn line × projected guide).
            for pen in (halo, mark):
                painter.setPen(pen)
                painter.drawLine(QPointF(px - 8, py - 8), QPointF(px + 8, py + 8))
                painter.drawLine(QPointF(px - 8, py + 8), QPointF(px + 8, py - 8))
        elif snap.kind == "center":
            # The centre of a circle: a filled dot.
            painter.setPen(halo)
            painter.setBrush(Qt.NoBrush)
            painter.drawEllipse(QPointF(px, py), 6.5, 6.5)
            painter.setPen(mark)
            painter.setBrush(QColor.fromRgbF(r, g, b, 0.85))
            painter.drawEllipse(QPointF(px, py), 6.5, 6.5)
        elif snap.kind in ("endpoint", "origin", "component_origin", "on_edge",
                           "on_axis", "intersection_edge", "intersection_face",
                           "on_line", "on_circle", "extension", "from_point",
                           "aligned", "tangent"):
            rect = QRectF(px - 7, py - 7, 14, 14)
            painter.setPen(halo)
            painter.setBrush(Qt.NoBrush)
            painter.drawRect(rect)
            painter.setPen(mark)
            painter.setBrush(QColor.fromRgbF(r, g, b, 0.30))
            painter.drawRect(rect)
        elif snap.kind in ("midpoint", "arc_midpoint", "quadrant"):
            # Cyan diamond.
            diamond = QPolygonF([
                QPointF(px, py - 9), QPointF(px + 9, py),
                QPointF(px, py + 9), QPointF(px - 9, py),
            ])
            painter.setPen(halo)
            painter.setBrush(Qt.NoBrush)
            painter.drawPolygon(diamond)
            painter.setPen(mark)
            painter.setBrush(QColor.fromRgbF(r, g, b, 0.30))
            painter.drawPolygon(diamond)
        elif snap.kind == "on_face":
            painter.setPen(halo)
            painter.drawEllipse(QPointF(px, py), 5.5, 5.5)
            painter.setPen(mark)
            painter.drawEllipse(QPointF(px, py), 5.5, 5.5)
        elif snap.kind == "face_center":
            # A filled ring with a cross inside: the one point of the face,
            # not just somewhere on it. The size of the endpoint's square
            # (14 px), so every point marker reads the same.
            for pen in (halo, mark):
                painter.setPen(pen)
                painter.setBrush(Qt.NoBrush if pen is halo
                                 else QColor.fromRgbF(r, g, b, 0.30))
                painter.drawEllipse(QPointF(px, py), 7.0, 7.0)
                painter.drawLine(QPointF(px - 7, py), QPointF(px + 7, py))
                painter.drawLine(QPointF(px, py - 7), QPointF(px, py + 7))
        elif snap.kind == "close":
            painter.setPen(halo)
            painter.drawEllipse(QPointF(px, py), 9.0, 9.0)
            painter.setPen(mark)
            painter.drawEllipse(QPointF(px, py), 9.0, 9.0)
        elif snap.kind in ("reference", "through_point", "perp_face"):
            painter.setPen(halo)
            painter.drawEllipse(QPointF(px, py), 6.5, 6.5)
            painter.setPen(mark)
            painter.drawEllipse(QPointF(px, py), 6.5, 6.5)

        # Tooltip text next to the marker ("On Edge", etc.).
        label = self._SNAP_LABELS.get(snap.kind)
        if snap.kind == "aligned" and snap.axis == "z":
            label = "Level with point"
        if label:
            label = tr(label)
        if getattr(snap, "label", None):
            label = snap.label               # an extension's own words
        if label:
            label += self._snap_context_suffix(getattr(snap, "context", None))
            font = QFont()
            font.setPointSize(9)
            painter.setFont(font)
            painter.setPen(QPen(QColor(255, 255, 255, 220)))
            painter.drawText(QPointF(px + 11, py + 17), label)
            painter.setPen(QPen(color))
            painter.drawText(QPointF(px + 10, py + 16), label)

    def _edit_group_box_corners(self):
        """The eight corners of the dashed box around the group being edited
        — the visual cue that you are INSIDE it (the classic dashed
        box) — or None. Drawn in the GL pass, depth-tested."""
        group = self.scene.edit_group
        if group is None:
            return None
        if not group.mesh.vertices and not getattr(group, "children", None):
            return None     # nothing to wrap
        # A CONTAINER's own mesh is usually empty — its geometry is in its
        # children — so testing that mesh alone left you inside a group with
        # no box at all, the one cue that tells you where you are.
        # In the group's OWN axes, like the selection cue: on a rotated
        # object a world-aligned box reads as skewed and wraps mostly air.
        from core.group import oriented_box_corners
        frame, lo, hi = self._group_obb(group)
        tight = oriented_box_corners(frame, lo, hi)

        # Size the gutter in screen pixels. This keeps the dotted cue clearly
        # separate from a cuboid's solid edges at every useful zoom level,
        # without making the box of a long object needlessly large.
        projected = [p for p in (self._world_to_pixel(c) for c in tight)
                     if p is not None]
        extent_px = 0.0
        if projected:
            xs, ys = zip(*projected)
            extent_px = max(max(xs) - min(xs), max(ys) - min(ys))
        pad_px = min(EDIT_BOX_PAD_MAX_PX,
                     max(EDIT_BOX_PAD_MIN_PX,
                         extent_px * EDIT_BOX_PAD_RATIO))

        centre = sum(tight[1:], QVector3D(tight[0])) / len(tight)
        span = float(self.camera.distance)
        if self.camera.perspective:
            depth = QVector3D.dotProduct(
                centre - self.camera.eye(), self.camera.forward())
            if math.isfinite(depth) and depth > 1e-9:
                span = depth
        world_per_px = (2.0 * max(span, 1e-9)
                        * math.tan(math.radians(self.camera.fov_deg) / 2.0)
                        / max(self.height(), 1))
        pad = pad_px * world_per_px
        padded_lo = tuple(float(v) - pad for v in lo)
        padded_hi = tuple(float(v) + pad for v in hi)
        return oriented_box_corners(frame, padded_lo, padded_hi)

    def edit_group_box_contains_pixel(self, x: float, y: float) -> bool:
        """Whether ``(x, y)`` lies inside the visible dotted edit box.

        This is the click boundary for stepping out of a group/component.
        Geometry need not fill its box: clicking the deliberate empty margin,
        or a hole in an irregular object, keeps the current edit context.
        """
        corners = self._edit_group_box_corners()
        if corners is None:
            return False
        projected = [p for p in (self._world_to_pixel(c) for c in corners)
                     if p is not None]
        return _point_in_screen_hull(x, y, projected)

    # ---- Section planes ------------------------------------------------------

    def _section_cut_active(self):
        return _active_cut(self.scene)

    def section_plane_frame(self, sp) -> list:
        """Four world corners of the plane's drawn frame: the model's bounds
        projected onto the plane, with a margin (the plane object is sized
        to the model)."""
        u, v = plane_axes(sp.normal)
        lo, hi = self.scene.bounds()
        if lo is None:
            half_u = half_v = 1.5
            cu = cv = 0.0
        else:
            corners = [QVector3D(x, y, z)
                       for x in (lo.x(), hi.x())
                       for y in (lo.y(), hi.y())
                       for z in (lo.z(), hi.z())]
            us = [QVector3D.dotProduct(c - sp.point, u) for c in corners]
            vs = [QVector3D.dotProduct(c - sp.point, v) for c in corners]
            cu, cv = (min(us) + max(us)) * 0.5, (min(vs) + max(vs)) * 0.5
            half_u = (max(us) - min(us)) * 0.56 + 0.2
            half_v = (max(vs) - min(vs)) * 0.56 + 0.2
        c = sp.point + u * cu + v * cv
        return [c - u * half_u - v * half_v, c + u * half_u - v * half_v,
                c + u * half_u + v * half_v, c - u * half_u + v * half_v]

    def _draw_section_planes(self, painter: QPainter) -> None:
        """The section plane OBJECTS (frame + corner brackets + symbol):
        orange when active, light grey when not, a solid heavier frame when
        selected. Hidden by View ▸ Section Planes."""
        planes = getattr(self.scene, "section_planes", None)
        if not planes or not getattr(self.scene, "show_section_planes", True):
            return
        selection = self.scene.selection
        for sp in planes:
            if getattr(sp, "hidden", False) \
                    or not self.scene.entity_visible(sp):
                continue                    # its frame is off; a cut stays
            corners = self.section_plane_frame(sp)
            px = []
            ok = True
            for i in range(4):
                seg = self._clip_segment_front(corners[i],
                                               corners[(i + 1) % 4])
                if seg is None:
                    ok = False
                    break
                a = self._world_to_pixel(seg[0])
                b = self._world_to_pixel(seg[1])
                if a is None or b is None:
                    ok = False
                    break
                px.append((a, b))
            if not ok:
                continue
            # Active or not, at a glance (@pacaeiro, #62, second round): the
            # ACTIVE plane in orange, every other one light grey — the axis
            # colours of 0.4.9 said which way a plane cut, which stops
            # meaning anything once it is rotated. Orange is also the
            # selection's colour, so a selected plane draws its frame SOLID
            # and heavier instead of dashed.
            selected = sp in selection
            if sp.active:
                col = QColor(243, 115, 41)
                width = 2.0
            else:
                col = QColor(178, 182, 188)
                width = 1.4
            if selected:
                width += 1.2
            # The frame edges are drawn dashed.
            pen = QPen(col, width, Qt.SolidLine if selected else Qt.DashLine)
            painter.setPen(pen)
            for a, b in px:
                painter.drawLine(QPointF(*a), QPointF(*b))
            # Corner brackets: short SOLID ticks into the frame.
            painter.setPen(QPen(col, width))
            for i in range(4):
                c = self._world_to_pixel(corners[i])
                n1 = self._world_to_pixel(corners[(i + 1) % 4])
                n2 = self._world_to_pixel(corners[(i + 3) % 4])
                if c is None or n1 is None or n2 is None:
                    continue
                for n_ in (n1, n2):
                    dx, dy = n_[0] - c[0], n_[1] - c[1]
                    ln = math.hypot(dx, dy) or 1.0
                    k = min(14.0, ln * 0.18) / ln
                    painter.drawLine(QPointF(c[0], c[1]),
                                     QPointF(c[0] + dx * k, c[1] + dy * k))
            # The section SYMBOL rides in a little balloon at EVERY
            # corner of the frame.
            symbol = (sp.symbol or "").strip()
            if symbol:
                painter.save()
                font = painter.font()
                font.setPointSizeF(8.5)
                font.setBold(True)
                painter.setFont(font)
                r = 9.0
                for i in range(4):
                    c = self._world_to_pixel(corners[i])
                    if c is None:
                        continue
                    centre = QPointF(c[0], c[1])
                    painter.setBrush(QColor(255, 255, 255, 220))
                    painter.setPen(QPen(col, width))
                    painter.drawEllipse(centre, r, r)
                    painter.setPen(QPen(col, 1))
                    painter.drawText(
                        QRectF(c[0] - r, c[1] - r, 2 * r, 2 * r),
                        Qt.AlignCenter, symbol[:3])
                painter.restore()

    def pick_section_plane(self, screen_x: float, screen_y: float):
        """The section plane whose frame is nearest the cursor within the
        pick threshold, or ``None`` (Select / context menu / Eraser)."""
        planes = getattr(self.scene, "section_planes", None)
        if not planes or not getattr(self.scene, "show_section_planes", True):
            return None
        best, best_d = None, self.pick_threshold_px
        for sp in planes:
            if getattr(sp, "hidden", False) \
                    or not self.scene.entity_selectable(sp):
                continue
            corners = self.section_plane_frame(sp)
            for i in range(4):
                seg = self._clip_segment_front(corners[i],
                                               corners[(i + 1) % 4])
                if seg is None:
                    continue
                a = self._world_to_pixel(seg[0])
                b = self._world_to_pixel(seg[1])
                if a is None or b is None:
                    continue
                d = _point_to_segment_distance_2d((screen_x, screen_y), a, b)
                if d < best_d:
                    best_d = d
                    best = sp
        return best

    def _section_cut_segments(self):
        """World-space cut segments: the active plane ∩ every visible model
        triangle, vectorized over the pick index's (v0, e1, e2) arrays.
        Cached per (scene version, plane pose)."""
        sp = self.scene.active_section()
        if sp is None:
            return None
        n, o = sp.normal, sp.point
        key = (self.scene.version, round(n.x(), 9), round(n.y(), 9),
               round(n.z(), 9), round(o.x(), 6), round(o.y(), 6),
               round(o.z(), 6))
        cached = getattr(self, "_section_cut_cache", None)
        if cached is not None and cached[0] == key:
            return cached[1]
        idx = self._pick_flat(self._pick_index(near="all"))
        from core.section import cut_chords
        segs = tags = None
        if idx.tri_v0 is not None and len(idx.tri_v0):
            # Each triangle's height along the normal is kept while the
            # model and the normal stay: a plane dragged by its arrow then
            # only compares three arrays with its new offset (a furnished
            # house froze ~2 s per drag measuring every triangle again).
            reach = getattr(self, "_section_reach", None)
            if reach is None or not reach.fits(idx.tri_v0, n, idx.tri_e1,
                                               idx.tri_e2):
                from core.section import PlaneReach
                reach = self._section_reach = PlaneReach(
                    idx.tri_v0, idx.tri_e1, idx.tri_e2, n)
            segs, rows = cut_chords(idx.tri_v0, idx.tri_e1, idx.tri_e2, n, o,
                                    idx.ent_vis[idx.tri_ent], rows=True,
                                    reach=reach)
            if segs is not None:
                tags = self._cut_layer_tags(idx, idx.tri_ent[rows])
        self._section_cut_cache = (key, segs, tags)
        return segs

    @staticmethod
    def _cut_layer_tags(idx, ents):
        """The layer each cut triangle's section is filled with, as a sheet
        tags it: the face's own layer, else its object's."""
        import numpy as np
        from core.layers import DEFAULT_LAYER, layer_of
        memo: dict = {}
        out = []
        # A lazy placement's face is built with its whole mesh when asked
        # for: its layer is read from the document instead.
        quick = getattr(idx.entities, "layer_at", None)
        for e in ents.tolist():
            tag = memo.get(e)
            if tag is None:
                if quick is not None:
                    own, owner = quick(e)
                    tag = own or DEFAULT_LAYER
                else:
                    face, owner = idx.entities[e]
                    tag = layer_of(face)
                if tag == DEFAULT_LAYER and owner is not None:
                    tag = layer_of(owner)
                memo[e] = tag
            out.append(tag)
        return np.asarray(out, dtype=object)

    def _section_fill_tris(self):
        """The fill's triangles for the active cut, cached with the cut
        segments they come from (same scene version, same plane pose)."""
        import numpy as np
        segs = self._section_cut_segments()
        cached = getattr(self, "_section_fill_cache", None)
        if cached is not None and (cached[0] is segs or (
                segs is not None and cached[0] is not None
                and cached[0].shape == segs.shape
                and np.array_equal(cached[0], segs))):
            # A selection click moves the scene version, not the cut.
            self._section_fill_cache = (segs, cached[1])
            return cached[1]
        # Chaining the rings costs a fraction of a second on a model with
        # thousands of cut solids — nothing once, too much per frame while
        # a plane is dragged. A cut that keeps changing shows its lines
        # live and its fill once it rests for a moment.
        import time as _t
        now = _t.monotonic()
        last = getattr(self, "_section_fill_changed", 0.0)
        self._section_fill_changed = now
        if cached is not None and now - last < 0.15:
            from PySide6.QtCore import QTimer
            QTimer.singleShot(200, self.update)
            return None
        from core.section import fill_triangles
        tris = fill_triangles(segs, self.scene.active_section().normal)
        self._section_fill_cache = (segs, tris)
        self._section_hatch_cache = {}
        return tris

    def _model_linetype_edges(self) -> list:
        """``[(rgb or None, dashes, gaps)]``: the edges of the layers drawn
        with their line type in the model (core.layers.model_line_styles),
        cut into their dashes and gaps (world space, as a sheet at 1:50
        dashes them) — rebuilt when the model or the line types change."""
        import numpy as np
        from core.layers import dash_segments, model_line_styles
        scene = self.scene
        styles = model_line_styles(scene)
        if not styles:
            return []
        # The document's version, not the view's: picking something moves
        # ``scene.version`` too, and every click on a model with dashed
        # layers walked all its edges and cut them into dashes again (a
        # second per click with 24 000 of them).
        key = (id(scene), scene.content_version,
               tuple(sorted(styles.items())))
        memo = getattr(self, "_model_lines_cache", None)
        if memo is not None and memo[0] == key:
            return memo[1]
        # From the viewport's arrays with their layers, not a walk of the
        # model: every change of the model walked 85 000 faces of a plaza.
        _t, hard, _s, _n, _tt, hard_tags, _st = self.hlr_geometry(tags=True)
        by_layer: dict = {}
        for seg, tag in zip(hard, hard_tags):
            if tag in styles:
                by_layer.setdefault(tag, []).append(seg)
        from core.layers import linetype_text
        names = {ly.key: ly.linetype for ly in scene.layers}
        out = []
        for tag, segs in by_layer.items():
            rgb, pattern = styles[tag]
            dashes, gaps = dash_segments(np.asarray(segs), pattern)
            words = self._model_line_words(
                np.asarray(segs), linetype_text(names.get(tag), scene))
            out.append((rgb, dashes, gaps, words))
        self._model_lines_cache = (key, out)
        return out

    @staticmethod
    def _model_line_words(segs, spec):
        """A line type's words (GAS…) on the model's edges, in the gaps the
        pattern leaves for them, as a sheet at 1:50 places them: their
        letters' outlines as world segments, lying in the plane of the
        edge and the horizontal across it, reading upright from above (a
        rising edge: from its side)."""
        import numpy as np
        from core.layers import MODEL_DASH_SCALE, text_marks
        empty = np.zeros((0, 2, 3), dtype=np.float32)
        if spec is None or not len(segs):
            return empty
        from views.hatch_paint import line_text_strokes
        strokes = line_text_strokes(spec[0], spec[1])
        if not strokes:
            return empty
        unit = MODEL_DASH_SCALE
        up_z = np.array([0.0, 0.0, 1.0])
        out: list = []
        for a, b in np.asarray(segs, dtype=np.float64).reshape(-1, 2, 3):
            d = b - a
            total = float(np.linalg.norm(d))
            if total <= 1e-9:
                continue
            u = d / total
            if u[0] < -1e-9 or (abs(u[0]) <= 1e-9 and u[1] < 0):
                u = -u                  # read left to right
                a, b = b, a
            side = np.cross(up_z, u)
            if np.linalg.norm(side) < 1e-6:
                side = np.array([0.0, 1.0, 0.0])
            side /= np.linalg.norm(side)
            marks = text_marks((0.0, 0.0), (total / unit, 0.0), spec)
            if len(marks) * len(strokes) > 4000:
                continue
            for x, _y, _deg in marks:
                centre = a + u * (x * unit)
                for poly in strokes:
                    pts = [centre + u * (px * unit) + side * (py * unit)
                           for px, py in poly]
                    out += [(p0, p1) for p0, p1 in zip(pts, pts[1:])]
        if not out:
            return empty
        return np.asarray(out, dtype=np.float32)



    def _section_layer_fills(self, color):
        """The cut filled layer by layer with the layers' hatches
        (core.section.layer_fills) — once the cut rests, as its plain fill,
        and again only when the cut or a layer's hatch changes."""
        if self._section_fill_tris() is None:
            return None
        cut = getattr(self, "_section_cut_cache", None)
        if cut is None or cut[1] is None:
            return None
        scene = self.scene
        sign = (id(cut[1]), tuple(color[:3]),
                repr([ly.to_dict() for ly in scene.layers]),
                repr(getattr(scene, "bricks", None)))
        memo = getattr(self, "_section_layer_cache", None)
        if memo is not None and memo[0] == sign:
            return memo[1]
        from core.section import layer_fills
        fills = layer_fills(cut[1], cut[2], scene.active_section().normal,
                            scene, color)
        self._section_layer_cache = (sign, fills)
        return fills



    def _section_hatch(self, spacing: float, cross: bool):
        """The hatch lines of the cut whose fill is cached, per (spacing,
        cross) — computed once, while the cut stays the same."""
        cached = getattr(self, "_section_fill_cache", None)
        if cached is None:
            return None
        memo = getattr(self, "_section_hatch_cache", None)
        if memo is None:
            memo = self._section_hatch_cache = {}
        key = (round(spacing, 9), bool(cross))
        if key not in memo:
            from core.section import hatch_segments
            memo[key] = hatch_segments(
                cached[0], self.scene.active_section().normal, spacing,
                cross)
        return memo[key]

    #: The sheet's cut pen (mm) a layer's own cut weight is measured
    #: against in the model: a layer at 0.50 draws the usual ~2.5 px line.
    CUT_PEN_MM = 0.5

    def _cut_edge_weights(self, plane, keep):
        """How much thicker or thinner each cut line is drawn than usual:
        with «Layers' hatches» each layer's cut weight (Muros at 0.70 a
        heavy line, an insulation at 0.18 a fine one), as on a sheet;
        otherwise all alike."""
        import numpy as np
        from core.section import effective_fill
        count = int(keep.sum())
        pattern = effective_fill(plane, self._frame_style)[0]
        cut = getattr(self, "_section_cut_cache", None)
        if pattern != "layers" or cut is None or cut[2] is None                 or len(cut[2]) != len(keep):
            return np.ones(count)
        pens = {ly.key: float(ly.cut_weight) / self.CUT_PEN_MM
                for ly in self.scene.layers
                if getattr(ly, "cut_weight", None) is not None}
        if not pens:
            return np.ones(count)
        out = np.array([pens.get(t, 1.0) for t in cut[2][keep].tolist()])
        return np.clip(out, 0.4, 2.4)

    def _clip_segment_front(
        self, a: QVector3D, b: QVector3D
    ) -> Optional[tuple[QVector3D, QVector3D]]:
        """Clip a world segment to the part in FRONT of the camera. The long
        'infinite' guide segments (±10 km) routinely have an endpoint behind
        the eye, where ``_world_to_pixel`` returns None — which used to make
        the whole guide vanish from render, snap and pick in ordinary 3D
        views. Returns the visible sub-segment, or ``None``."""
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        wa = mvp.map(QVector4D(a.x(), a.y(), a.z(), 1.0)).w()
        wb = mvp.map(QVector4D(b.x(), b.y(), b.z(), 1.0)).w()
        eps = 1e-3
        if wa <= eps and wb <= eps:
            return None
        if wa <= eps or wb <= eps:
            t = (eps - wa) / (wb - wa)   # w is linear along the segment
            cut = a + (b - a) * t
            return (cut, QVector3D(b)) if wa <= eps else (QVector3D(a), cut)
        return (QVector3D(a), QVector3D(b))

    def _guide_snap_segment(
        self, g, margin: float = 1.5
    ) -> Optional[tuple[QVector3D, QVector3D]]:
        """The span of guide line ``g`` the snap engine works on: the part
        inside the view (widened by ``margin`` so the ends stay well off
        screen), computed in double precision from the guide's own point
        and direction.

        The ±10 km segment of ``Guide.segment()`` is fine for drawing but not
        for arithmetic: float32 rounds a coordinate of 10 000 m to ~1 mm, so
        every crossing, 'on line' point and lock-line hit computed from those
        endpoints was off by up to 2 mm — lines drawn from the X of two
        diagonal guides visibly left the guides when zoomed in (#110).
        Clipped to the view the endpoints are as large as what is on
        screen, and the error scales with it (µm at a working zoom)."""
        from core.guide import GUIDE_HALF_LEN
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        p, u = g.point, g.direction
        c0 = mvp.map(QVector4D(p.x(), p.y(), p.z(), 1.0))
        c1 = mvp.map(QVector4D(u.x(), u.y(), u.z(), 0.0))
        c0 = (c0.x(), c0.y(), c0.w())
        c1 = (c1.x(), c1.y(), c1.w())
        lo, hi = -GUIDE_HALF_LEN, GUIDE_HALF_LEN
        # Clip coordinates are affine in s along the line: keep
        # w >= eps and |x|, |y| <= margin * w (Liang–Barsky on four planes).
        planes = ((c0[2] - 1e-3, c1[2]),
                  (margin * c0[2] - c0[0], margin * c1[2] - c1[0]),
                  (margin * c0[2] + c0[0], margin * c1[2] + c1[0]),
                  (margin * c0[2] - c0[1], margin * c1[2] - c1[1]),
                  (margin * c0[2] + c0[1], margin * c1[2] + c1[1]))
        for f0, f1 in planes:           # need f0 + f1 * s >= 0
            if abs(f1) < 1e-15:
                if f0 < 0.0:
                    return None
                continue
            s = -f0 / f1
            if f1 > 0.0:
                lo = max(lo, s)
            else:
                hi = min(hi, s)
            if lo >= hi:
                return None
        px, py, pz = p.x(), p.y(), p.z()
        ux, uy, uz = u.x(), u.y(), u.z()
        return (QVector3D(px + ux * lo, py + uy * lo, pz + uz * lo),
                QVector3D(px + ux * hi, py + uy * hi, pz + uz * hi))

    def _draw_scale_box(self, painter: QPainter) -> None:
        """The scaling box: yellow edges, green grips, the grabbed grip
        and its anchor in red. Drawn from whatever the active tool reports via
        ``scale_box_state()`` (duck-typed, so this file needs no tool import);
        while an operation is live the box rides the same matrix the geometry
        is being scaled by."""
        tool = self.active_tool
        state_fn = getattr(tool, "scale_box_state", None)
        if state_fn is None:
            return
        state = state_fn()
        if state is None:
            return
        box_pen = QPen(QColor(214, 174, 0), 1)          # grip-box yellow
        painter.setPen(box_pen)
        for a, b in state["segments"]:
            seg = self._clip_segment_front(a, b)
            if seg is None:
                continue
            pa = self._world_to_pixel(seg[0])
            pb = self._world_to_pixel(seg[1])
            if pa is not None and pb is not None:
                painter.drawLine(QPointF(*pa), QPointF(*pb))
        green = QColor(60, 160, 60)
        red = QColor(220, 40, 40)
        # Far grips first, so a grip at the front of the box is drawn over
        # the one behind it rather than under it (#833).
        eye = self.camera.eye()
        fwd = self.camera.target - eye
        grips = sorted(state["grips"], key=lambda g: -QVector3D.dotProduct(
            g[0] - eye, fwd))
        for pos, role in grips:
            px = self._world_to_pixel(pos)
            if px is None:
                continue
            half = 5.0 if role in ("hover", "active", "anchor") else 4.0
            color = green if role == "idle" else red
            painter.fillRect(QRectF(px[0] - half, px[1] - half,
                                    half * 2, half * 2), color)
            painter.setPen(QPen(QColor(30, 30, 30), 1))
            painter.drawRect(QRectF(px[0] - half, px[1] - half,
                                    half * 2, half * 2))
        painter.setPen(box_pen)

    def _clip_pixel_line(
        self, p0: tuple[float, float], p1: tuple[float, float],
        margin: float = 64.0,
    ) -> Optional[tuple[tuple[float, float], tuple[float, float]]]:
        """Liang-Barsky clip of a 2D pixel segment to the widget rect (plus a
        margin). Overlay guides are only clipped in 3D to the part in front of
        the camera (``_clip_segment_front``), so their on-screen endpoints land
        MILLIONS of pixels from origin. Qt's dash stroker runs in fixed point
        and collapses a dashed pen into a solid line at that scale (measured:
        a guide of 400 k px still dashes, 4 M px comes out solid). Trimming to
        the visible area first keeps the dashes — and spares the rasteriser
        hundreds of thousands of dash repeats. Returns the clipped endpoints,
        or ``None`` when the segment misses the widget entirely."""
        x0, y0 = p0
        x1, y1 = p1
        dx, dy = x1 - x0, y1 - y0
        xmin, ymin = -margin, -margin
        xmax, ymax = self.width() + margin, self.height() + margin
        t0, t1 = 0.0, 1.0
        for p, q in ((-dx, x0 - xmin), (dx, xmax - x0),
                     (-dy, y0 - ymin), (dy, ymax - y0)):
            if p == 0.0:
                if q < 0.0:
                    return None
                continue
            r = q / p
            if p < 0.0:
                if r > t1:
                    return None
                if r > t0:
                    t0 = r
            else:
                if r < t0:
                    return None
                if r < t1:
                    t1 = r
        return ((x0 + t0 * dx, y0 + t0 * dy),
                (x0 + t1 * dx, y0 + t1 * dy))

    def _draw_guides(self, painter: QPainter) -> None:
        """The small crosses of guide POINTS, the usual scaffolding.

        Guide LINES are not here any more: they are drawn in the GL pass so
        the depth buffer hides them behind geometry (issue #23). A cross is
        a screen-space mark a few pixels wide, not a line through the
        model, so it stays in the overlay where its size is in pixels."""
        guides = shown_guides(self.scene)
        if not guides:
            return
        pen = QPen(QColor(70, 90, 120), 1, Qt.DashLine)
        pen.setDashPattern([14.0, 10.0])
        sel_pen = QPen(QColor(243, 115, 41), 2, Qt.DashLine)  # selection orange
        sel_pen.setDashPattern([14.0, 10.0])
        selection = self.scene.selection
        for g in guides:
            if g.is_line or getattr(g, "is_circle", False):
                continue
            painter.setPen(sel_pen if g in selection else pen)
            q = self._world_to_pixel(g.point)
            if q is not None:
                painter.drawLine(QPointF(q[0] - 5, q[1]), QPointF(q[0] + 5, q[1]))
                painter.drawLine(QPointF(q[0], q[1] - 5), QPointF(q[0], q[1] + 5))

    def _draw_geo_surfaces(self, painter: QPainter) -> None:
        """Draw terrain-surface fills as shaded, back-to-front triangles so the
        relief reads in 3D (Track G). Semi-transparent, so the base map shows."""
        paths = getattr(self.scene, "geo_paths", None)
        if not paths:
            return
        eye = self.camera.eye()
        light = QVector3D(0.3, 0.4, 0.85)
        light = light.normalized()
        for path in paths:
            tris = getattr(path, "_surface_tris", None)
            if not tris:
                continue
            flat = getattr(path, "surface", None) == "flat"
            # Painter's algorithm: far triangles first.
            ordered = sorted(
                tris, key=lambda t: -((t[0] + t[1] + t[2]) - eye * 3.0).lengthSquared())
            painter.setPen(Qt.NoPen)
            for v0, v1, v2 in ordered:
                p0 = self._world_to_pixel(v0)
                p1 = self._world_to_pixel(v1)
                p2 = self._world_to_pixel(v2)
                if p0 is None or p1 is None or p2 is None:
                    continue
                # Flat shading from the face normal (relief legibility).
                n = QVector3D.crossProduct(v1 - v0, v2 - v0)
                if n.length() > 1e-9:
                    n = n.normalized()
                shade = 0.55 + 0.45 * max(0.0, abs(QVector3D.dotProduct(n, light)))
                if flat:
                    col = QColor(int(90 * shade), int(120 * shade), int(200 * shade), 150)
                else:
                    col = QColor(int(120 * shade), int(160 * shade), int(90 * shade), 150)
                painter.setBrush(col)
                painter.drawPolygon(QPolygonF([QPointF(*p0), QPointF(*p1), QPointF(*p2)]))
            painter.setBrush(Qt.NoBrush)

    def ground_height(self, x: float, y: float) -> float | None:
        """Ground elevation at a plan position, or ``None`` where none is known.

        One place to ask, so the survey-beats-DEM precedence lives here instead
        of being re-decided by every caller (drape, the elevation readout, the
        profile sampler chooser).
        """
        survey = getattr(self.scene, "photo_mesh", None)
        if survey is not None and getattr(survey, "visible", False):
            z = survey.height_at(x, y)
            if z is not None:
                return z
        t = getattr(self.scene, "terrain", None)
        if t is not None and getattr(t, "visible", False):
            return t.height_at(x, y)
        return None

    def _emit_coordinate(self, ev) -> None:
        """Publish the UTM coordinate under the cursor for the status bar.

        Rides the coalesced hover rather than every mouse event: the readout is
        for reading, and 16 updates a second is already more than an eye can
        follow. Silent without a datum — local scene metres are not a
        coordinate anybody can use.
        """
        datum = getattr(self.scene, "georef", None)
        if datum is None:
            if self._last_coordinate:
                self._last_coordinate = ""
                self.coordinateChanged.emit("")
            return
        p = ev.position().toPoint()
        world = self._world_from_pixel(p.x(), p.y())
        if world is None:
            return
        east, north, _ = datum.local_to_utm(world)
        z = self.ground_elevation(world.x(), world.y())
        # Compact on the bar — a decimetre is the pointer's own precision on
        # a site plan, and the zone does not change under the mouse — and in
        # full on hover (Marco, 23-09: the bar had no room for the hint).
        text = f"E {east:.1f}  N {north:.1f}"
        full = (f"E {east:,.2f}  N {north:,.2f}  "
                f"{datum.zone}{datum.hemisphere}")
        if z is not None:
            text += f"  ·  {z:.1f} m"
            full += f"  ·  {z:,.2f} m"
        self._last_coordinate_full = full
        if text != self._last_coordinate:
            self._last_coordinate = text
            self.coordinateChanged.emit(text)

    def ground_elevation(self, x: float, y: float) -> float | None:
        """Ground elevation as a REAL altitude, not a local offset.

        ``ground_height`` answers in scene metres, which is what geometry
        needs; this adds the datum's altitude back so what reaches the user is
        the number they would read off a level — 1783.19, not 37.19. Anything
        shown to a person goes through here.
        """
        z = self.ground_height(x, y)
        if z is None:
            return None
        datum = getattr(self.scene, "georef", None)
        return z + float(getattr(datum, "alt", 0.0) or 0.0)

    def vertical_reference(self) -> str:
        """Token naming what the scene's elevations are measured against, for
        labelling. See :mod:`georef.photomesh` for the meanings."""
        from georef.photomesh import VERTICAL_DEM, VERTICAL_LOCAL, VERTICAL_ODM
        survey = getattr(self.scene, "photo_mesh", None)
        if survey is not None and getattr(survey, "visible", False):
            return getattr(survey, "vertical_reference", VERTICAL_ODM)
        t = getattr(self.scene, "terrain", None)
        if t is not None and getattr(t, "visible", False):
            return VERTICAL_DEM
        return VERTICAL_LOCAL

    def has_ground_surface(self) -> bool:
        """Whether anything is showing that a trace could be draped onto."""
        for name in ("photo_mesh", "terrain"):
            obj = getattr(self.scene, name, None)
            if obj is not None and getattr(obj, "visible", False):
                return True
        return False

    def drape(self, v: QVector3D) -> QVector3D:
        """Lift a Z=0 georef point onto the ground (its relief height) when a
        ground surface is showing, so routes/markers sit on it instead of
        floating at the Z=0 reference plane. A no-op otherwise.

        The **survey wins over the DEM terrain** where both are present: it is
        the user's own flight at centimetres per pixel against 30 m global
        cells, so a trace should follow what was actually captured.
        """
        z = self.ground_height(v.x(), v.y())
        return QVector3D(v.x(), v.y(), z) if z is not None else v

    def _draw_geo_points(self, painter: QPainter) -> None:
        """Draw imported survey points (GPS / total station): a small cross
        marker with the point's name — the surveyed skeleton the user traces
        over. Reference-only; they carry their own elevation (no draping)."""
        points = getattr(self.scene, "geo_points", None)
        if not points:
            return
        ink = QColor(206, 66, 87)               # survey red — distinct from
        painter.setPen(QPen(ink, 1.6))          # cyan paths / orange selection
        font = QFont()
        font.setPointSize(8)
        painter.setFont(font)
        for gp in points:
            q = self._world_to_pixel(gp.position)
            if q is None:
                continue
            x, y = q
            painter.setPen(QPen(ink, 1.6))
            painter.drawLine(QPointF(x - 5, y), QPointF(x + 5, y))
            painter.drawLine(QPointF(x, y - 5), QPointF(x, y + 5))
            if gp.name:
                painter.setPen(QColor(120, 34, 48))
                painter.drawText(QPointF(x + 7, y - 4), gp.name)

    def _draw_rubber_band_overlay(self, painter: QPainter) -> None:
        if self._overlay_rubber is None:
            return
        segments, color, width = self._overlay_rubber
        r, g, b = color[0], color[1], color[2]
        a = color[3] if len(color) > 3 else 1.0
        pen = QPen(QColor.fromRgbF(r, g, b, a), width)
        pen.setCapStyle(Qt.RoundCap)
        pen.setJoinStyle(Qt.RoundJoin)
        painter.setPen(pen)
        for p0, p1 in segments:
            q = self._segment_to_pixels(p0, p1)
            if q is not None:
                painter.drawLine(QPointF(*q[0]), QPointF(*q[1]))

    def _draw_guide_preview(self, painter: QPainter) -> None:
        """The guide the Tape or the Protractor is about to leave, dashed
        like the guides themselves, following the cursor (#89, @pacaeiro)."""
        tool = self.active_tool
        lines = (tool.guide_preview_lines()
                 if tool is not None
                 and hasattr(tool, "guide_preview_lines") else [])
        if not lines:
            return
        pen = QPen(QColor(70, 90, 120), 1.5, Qt.DashLine)
        pen.setDashPattern([10.0, 7.0])
        painter.setPen(pen)
        for p0, p1 in lines:
            q = self._segment_to_pixels(p0, p1)
            if q is not None:
                painter.drawLine(QPointF(*q[0]), QPointF(*q[1]))

    def _draw_route_marker(self, painter: QPainter) -> None:
        """Profile→plan marker: the route point at the station hovered in
        the profile panel (Track G). A method so the Metal overlay paints
        the same dot."""
        if getattr(self, "_route_marker", None) is None:
            return
        q = self._world_to_pixel(self.drape(self._route_marker))
        if q is not None:
            painter.setBrush(QColor(243, 115, 41))
            painter.setPen(QPen(QColor(255, 255, 255), 1.5))
            painter.drawEllipse(QPointF(*q), 6, 6)
            painter.setBrush(Qt.NoBrush)

    def _draw_linear_mode_label(self, painter: QPainter) -> None:
        if getattr(self, '_chrome_owns_labels', False):
            return  # the egui options strip shows it (views/wgpu_ui.py)
        # The mirror image of the bug above: these had the Spanish written
        # into the source, so an English UI showed Spanish.
        text = {
            "off": tr("Linear inferences: OFF (Alt)"),
            "parallel_perp": tr("Inferences: parallel / perpendicular only (Alt)"),
        }.get(self.linear_inference_mode)
        if not text:
            return
        font = QFont()
        font.setPointSize(9)
        painter.setFont(font)
        painter.setPen(QPen(QColor(210, 150, 40)))
        painter.drawText(QPointF(14, self.height() - 16), text)

    def _draw_geo_paths(self, painter: QPainter) -> None:
        """Draw committed georef paths (roads / boundaries): a coloured polyline
        with node handles. Selected paths and the hovered node highlight."""
        paths = getattr(self.scene, "geo_paths", None)
        if not paths:
            return
        base_ink = QColor(0, 190, 210)          # cyan — reads over terrain
        sel_ink = QColor(243, 115, 41)          # selection orange
        selection = self.scene.selection
        hover_node = getattr(self, "_hover_geo_node", None)
        for path in paths:
            ink = sel_ink if path in selection else base_ink
            pix = [self._world_to_pixel(self.drape(p)) for p in path.points]
            painter.setPen(QPen(ink, 2.5))
            for a, b in zip(pix, pix[1:]):
                if a is not None and b is not None:
                    painter.drawLine(QPointF(*a), QPointF(*b))
            if path.closed and len(pix) > 2 and pix[0] and pix[-1]:
                painter.drawLine(QPointF(*pix[-1]), QPointF(*pix[0]))
            # Node handles.
            painter.setBrush(ink)
            painter.setPen(QPen(QColor(255, 255, 255), 1))
            for i, q in enumerate(pix):
                if q is None:
                    continue
                r = 5.0 if (hover_node == (path, i)) else 3.5
                painter.drawEllipse(QPointF(*q), r, r)
            painter.setBrush(Qt.NoBrush)
            # Area + perimeter label at the centroid of a selected polygon.
            if path in selection and path.closed and len(path.points) >= 3:
                self._draw_geo_path_label(painter, path)

    def _draw_geo_path_label(self, painter: QPainter, path) -> None:
        n = len(path.points)
        cx = sum(p.x() for p in path.points) / n
        cy = sum(p.y() for p in path.points) / n
        cz = sum(p.z() for p in path.points) / n
        q = self._world_to_pixel(self.drape(QVector3D(cx, cy, cz)))
        if q is None:
            return
        area = path.area()
        text = f"{tr('Area')}: {area:.1f} m²  ({area / 10000:.3f} ha)\n" \
               f"{tr('Perimeter')}: {_fmt_len(path.perimeter())}"
        font = QFont()
        font.setPointSize(9)
        font.setBold(True)
        painter.setFont(font)
        fm = painter.fontMetrics()
        lines = text.split("\n")
        tw = max(fm.horizontalAdvance(ln) for ln in lines)
        th = fm.height() * len(lines)
        x, y = q[0] - tw / 2, q[1] - th / 2
        painter.fillRect(int(x) - 5, int(y) - 3, tw + 10, th + 6,
                         QColor(20, 24, 30, 190))
        painter.setPen(QColor(255, 255, 255))
        for i, ln in enumerate(lines):
            painter.drawText(QPointF(x, y + fm.ascent() + i * fm.height()), ln)

    @staticmethod
    def _text_block_x(p_pos, p_anchor, block_w: float) -> float:
        """Left edge of a leader-text block whose leader ends at ``p_pos``.

        The block sits on the side of the leader end AWAY from the anchor
        (the usual convention): to the right when the anchor is left of the label, to the
        LEFT when the leader arrives from the right — otherwise the leader
        runs straight through the words (Marco's "Pileta de piedra basalto"
        label, 2026-09-02). Shared by the paint pass and the text pick so
        the hit box is the drawn box."""
        if p_anchor is not None and p_pos[0] < p_anchor[0]:
            return p_pos[0] - 6 - block_w
        return p_pos[0] + 6

    def _draw_text_labels(self, painter: QPainter) -> None:
        """Draw every leader-text annotation: a leader line from the anchor
        (hidden where geometry occludes it, like dimensions) up to the label,
        an anchor dot, and the multi-line text with a white halo."""
        labels = getattr(self.scene, "text_labels", None)
        if not labels:
            return
        style = getattr(self.scene, "dimension_style", {})
        col = style.get("color", [45, 55, 75])
        default_ink = QColor(col[0], col[1], col[2])
        sel_ink = QColor(243, 115, 41)
        selection = self.scene.selection
        font = QFont(style.get("font_family", ""))
        font.setPointSize(int(style.get("font_size", 9)))
        font.setBold(style.get("font_bold", True))
        font.setItalic(style.get("font_italic", False))
        painter.setFont(font)
        fm = painter.fontMetrics()
        for lab in labels:
            if not self.scene.entity_visible(lab):      # hidden layer
                continue
            ink = (sel_ink if (lab in selection or lab is self._hover_entity)
                   else default_ink)
            pos = lab.position()
            p_anchor = self._world_to_pixel(lab.anchor)
            p_pos = self._world_to_pixel(pos)
            if p_pos is None:
                continue
            painter.setPen(QPen(ink, 1.2))
            self._draw_occluded_segment(painter, lab.anchor, pos)   # leader
            if p_anchor is not None and not self._occluded_on_screen(lab.anchor):
                painter.setBrush(ink)
                painter.drawEllipse(QPointF(*p_anchor), 2.5, 2.5)
                painter.setBrush(Qt.NoBrush)
            lines = lab.text.splitlines() or [""]
            x = self._text_block_x(
                p_pos, p_anchor, max(fm.horizontalAdvance(ln) for ln in lines))
            for i, line in enumerate(lines):
                y = p_pos[1] - 4 + i * fm.height()
                painter.setPen(QPen(QColor(255, 255, 255, 230)))
                painter.drawText(QPointF(x + 1, y + 1), line)
                painter.setPen(QPen(ink))
                painter.drawText(QPointF(x, y), line)

    def _draw_dimensions(self, painter: QPainter) -> None:
        """Draw every committed static dimension: extension lines from the
        measured endpoints out to the dimension line, the dimension line with
        end ticks, and the value label at its midpoint."""
        dims = getattr(self.scene, "dimensions", None)
        if not dims:
            return
        style = getattr(self.scene, "dimension_style", {})
        col = style.get("color", [45, 55, 75])
        default_ink = QColor(col[0], col[1], col[2])
        sel_ink = QColor(243, 115, 41)  # selection orange
        selection = self.scene.selection
        font = QFont(style.get("font_family", ""))
        font.setPointSize(int(style.get("font_size", 9)))
        font.setBold(style.get("font_bold", True))
        font.setItalic(style.get("font_italic", False))
        # A groups preview (Move/Rotate) moves no geometry until the commit:
        # the dimensions held by the movers' vertices are drawn where the
        # commit will put them, or they lagged behind the whole drag (#621).
        riders = getattr(self, "_preview_groups", None)
        carry = self._groups_preview_transform() if riders else None
        for dim in dims:
            if not self.scene.entity_visible(dim):      # hidden layer
                continue
            ink = (sel_ink if (dim in selection or dim is self._hover_entity)
                   else default_ink)
            if riders:
                ends = dim.ends_riding(riders)
                if ends:
                    dim = dim.carried(carry, ends)
            ap, bp = dim.line_points()
            pap = self._world_to_pixel(ap)
            pbp = self._world_to_pixel(bp)
            if pap is None or pbp is None:
                continue
            # Lines are hidden where solid geometry sits in front of them, so a
            # dimension reads as part of the model instead of floating over it.
            # Each 3D segment is sampled and only its visible runs are drawn.
            painter.setPen(QPen(ink, 1.0))
            self._draw_occluded_segment(painter, dim.a, ap)      # extension
            self._draw_occluded_segment(painter, dim.b, bp)
            painter.setPen(QPen(ink, 1.5))
            self._draw_occluded_segment(painter, ap, bp)         # dimension line
            # The ends, in screen space, drawn only where the end point
            # itself is visible: filled arrows pointing outward (the
            # document's default), oblique ticks, or nothing.
            dx, dy = pbp[0] - pap[0], pbp[1] - pap[1]
            ln = math.hypot(dx, dy)
            ends = style.get("ends", "arrow") or "arrow"
            if ln > 1e-6 and ends != "none":
                ux, uy = dx / ln, dy / ln
                for (cx, cy), w, sign in ((pap, ap, 1.0), (pbp, bp, -1.0)):
                    if self._occluded_on_screen(w):
                        continue
                    if ends == "tick":
                        # 45° to the line, «/» on a horizontal one, as on
                        # the sheet (composer.oblique_tick_angle); it was
                        # square to the line, not oblique (#741)
                        k = 4.0 * math.sqrt(0.5)
                        ox, oy = (ux + uy) * k, (uy - ux) * k
                        painter.drawLine(QPointF(cx - ox, cy - oy),
                                         QPointF(cx + ox, cy + oy))
                    else:
                        # tip at the end, wings trailing INTO the line
                        # (inside arrows: they point at the extension lines)
                        L, half = 10.0, 2.2
                        bx, by = cx + sign * ux * L, cy + sign * uy * L
                        painter.save()
                        painter.setBrush(ink)
                        painter.setPen(Qt.NoPen)
                        painter.drawPolygon(QPolygonF([
                            QPointF(cx, cy),
                            QPointF(bx - uy * half, by + ux * half),
                            QPointF(bx + uy * half, by - ux * half)]))
                        painter.restore()
            # Value label at the dimension line's midpoint — hidden if that
            # point is behind the solid.
            mid_world = dim.midpoint()
            mid = self._world_to_pixel(mid_world)
            if mid is not None and not self._occluded_on_screen(mid_world):
                text = dim.display_text(
                    self._format_dim_value(dim.value(), style))
                painter.setFont(font)
                painter.setPen(QPen(QColor(255, 255, 255, 230)))
                painter.drawText(QPointF(mid[0] + 5, mid[1] - 4), text)
                painter.setPen(QPen(ink))
                painter.drawText(QPointF(mid[0] + 4, mid[1] - 5), text)

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
        """Draw the 3D segment ``p3a``–``p3b`` in screen space, skipping the
        parts hidden behind solid geometry (CPU occlusion sample).

        The line is sampled about every ``_OCCLUSION_STEP_PX`` on screen (at
        least ``samples`` steps), and where one sample is seen and the next
        is not, the point where it goes behind is pinned by halving the
        step — the line stops AT the silhouette. Drawing only the steps
        whose both ends were seen cut it back a whole sixteenth at a time,
        and the cut jumped from step to step as the view turned: the
        dimensions clipped in and out behind other surfaces (issue #918).

        The samples are projected and tested as one batch
        (``_project_samples``): doing it point by point rebuilt the camera's
        projection*view matrix for every sample — 432 rebuilds a frame for
        eight dimensions — and ran a NumPy 3x3 reduction per sample (#439)."""
        d = p3b - p3a

        def at(ts):
            return [p3a + d * t for t in ts]

        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        ea = self._world_to_pixel(p3a, mvp)
        eb = self._world_to_pixel(p3b, mvp)
        n = samples
        if ea is not None and eb is not None:
            # With both ends in front of the eye, the projected segment
            # stays between them. Allow for the annotation pen and AA.
            if (max(ea[0], eb[0]) < -2 or min(ea[0], eb[0]) > self.width() + 2
                    or max(ea[1], eb[1]) < -2 or min(ea[1], eb[1]) > self.height() + 2):
                return
            span = math.hypot(eb[0] - ea[0], eb[1] - ea[1])
            n = max(samples, min(self._OCCLUSION_MAX_SAMPLES,
                                 int(span / self._OCCLUSION_STEP_PX)))
        ts = [i / n for i in range(n + 1)]
        res = self._project_samples(at(ts))
        pxs = [px for px, _v in res]
        vis = [px is not None and v for px, v in res]
        # The steps that change from seen to hidden (or back): each is cut
        # in ``_OCCLUSION_SPLIT`` and narrowed to the part where it flips,
        # every cut step in one batch per round.
        cuts = [i for i in range(n)
                if pxs[i] is not None and pxs[i + 1] is not None
                and vis[i] != vis[i + 1]]
        edge: dict = {}
        if cuts:
            k = self._OCCLUSION_SPLIT
            lo = {i: ts[i] for i in cuts}          # t on sample i's side
            hi = {i: ts[i + 1] for i in cuts}
            edge = {i: pxs[i] if vis[i] else pxs[i + 1] for i in cuts}
            for _ in range(self._OCCLUSION_ROUNDS):
                probe = [(i, lo[i] + (hi[i] - lo[i]) * j / k)
                         for i in cuts for j in range(1, k)]
                got = self._project_samples(at([t for _i, t in probe]))
                for c, i in enumerate(cuts):
                    row = got[c * (k - 1):(c + 1) * (k - 1)]
                    tt = [t for _i, t in probe[c * (k - 1):(c + 1) * (k - 1)]]
                    new_lo, new_hi = lo[i], hi[i]
                    for (px, v), t in zip(row, tt):
                        if (px is not None and v) == vis[i]:
                            new_lo = t
                            if vis[i]:
                                edge[i] = px      # the last seen, so far
                        else:
                            new_hi = t
                            if not vis[i]:
                                edge[i] = px      # the first seen, so far
                            break
                    lo[i], hi[i] = new_lo, new_hi
        for i in range(n):
            a, b = pxs[i], pxs[i + 1]
            if a is None or b is None:
                continue
            if vis[i] and vis[i + 1]:
                painter.drawLine(QPointF(*a), QPointF(*b))
            elif edge.get(i) is not None:
                start = a if vis[i] else b
                painter.drawLine(QPointF(*start), QPointF(*edge[i]))

    def _project_samples(self, pts: list[QVector3D]):
        """Project ``pts`` to screen pixels and test each one against the
        frame's depth snapshot, as a single batch: the camera's MVP is built
        once for the whole run, and the snapshot's 3x3 depth test runs as one
        NumPy gather instead of a slice-and-max per point (#439).

        The answers are the ones ``_world_to_pixel`` and
        ``_occluded_on_screen`` give point by point — the live camera's
        matrix for the pixel, the snapshot's own for the test, the same
        tolerances — and without a snapshot (or NumPy) this defers to those
        per-point paths. Returns a list of ``(pixel, visible)`` pairs, with
        ``pixel`` None for a point behind the camera."""
        mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        snap = getattr(self, "_depth_snap", None)
        if snap is None:
            return [(self._world_to_pixel(p, mvp),
                     not self._occluded_on_screen(p)) for p in pts]
        try:
            import numpy as np
        except Exception:  # noqa: BLE001 — never break the paint
            return [(self._world_to_pixel(p, mvp),
                     not self._occluded_on_screen(p)) for p in pts]
        buf, smvp, inv, sw, sh, eye, fwd = snap

        def rows(m):
            return np.array([[m.row(r).x(), m.row(r).y(),
                              m.row(r).z(), m.row(r).w()]
                             for r in range(4)], dtype=np.float64)

        p4 = np.array([(p.x(), p.y(), p.z(), 1.0) for p in pts],
                      dtype=np.float64)
        with np.errstate(all="ignore"):
            # The pixel, with the live camera (as _world_to_pixel).
            clip = p4 @ rows(mvp).T
            cw = clip[:, 3]
            ndc = clip[:, :2] / cw[:, None]
            pxs = (ndc[:, 0] * 0.5 + 0.5) * self.width()
            pys = (1.0 - (ndc[:, 1] * 0.5 + 0.5)) * self.height()
            pixels = [None if c <= 0 else (float(x), float(y))
                      for c, x, y in zip(cw, pxs, pys)]
            # The occlusion test, with the snapshot's matrices (as
            # _occluded_on_screen): untested points stay visible.
            occluded = np.zeros(len(pts), dtype=bool)
            c = p4 @ rows(smvp).T
            w = c[:, 3]
            nx = c[:, 0] / w
            ny = c[:, 1] / w
            ix = ((nx + 1.0) * 0.5 * sw).astype(np.int64)
            iy = ((ny + 1.0) * 0.5 * sh).astype(np.int64)
            live = (w > 1e-9) & (ix >= 0) & (ix < sw) & (iy >= 0) & (iy < sh)
            if live.any():
                # The FARTHEST depth of the 3x3 around each pixel: gathering
                # with the indices clamped to the frame reads the edge
                # pixels twice, which a max does not see — the same values
                # the per-point clamped slice takes, without copying the
                # frame to pad it.
                gx, gy = ix[live], iy[live]
                d = np.maximum.reduce(
                    [buf[np.clip(gy + dy, 0, sh - 1),
                         np.clip(gx + dx, 0, sw - 1)]
                     for dy in (-1, 0, 1) for dx in (-1, 0, 1)]
                ).astype(np.float64)
                near = d < 1.0                       # else background
                if near.any():
                    idx = np.flatnonzero(live)[near]
                    q = np.column_stack(
                        [nx[idx], ny[idx], d[near] * 2.0 - 1.0,
                         np.ones(len(idx))]) @ rows(inv).T
                    qw = q[:, 3]
                    ok = np.abs(qw) >= 1e-12
                    surf = q[:, :3] / np.where(ok, qw, 1.0)[:, None]
                    eye_a = np.array([eye.x(), eye.y(), eye.z()])
                    fwd_a = np.array([fwd.x(), fwd.y(), fwd.z()])
                    along_pt = (p4[idx, :3] - eye_a) @ fwd_a
                    along_surf = (surf - eye_a) @ fwd_a
                    tol = np.maximum(0.02, 0.003 * np.abs(along_pt))
                    occluded[idx] = ok & (along_surf < along_pt - tol)
        return [(px, not occ) for px, occ in zip(pixels, occluded)]

    def _draw_inference_marker(self, painter: QPainter) -> None:
        """Snap cue where the active tool's distance inference is engaged."""
        tool = self.active_tool
        provider = getattr(tool, "inference_marker", None) if tool is not None else None
        if not callable(provider):
            return
        result = provider()
        if result is None:
            return
        world, kind = result
        # The same colours and words as the snap markers: a corner is the
        # endpoint green, an edge the on-edge red, a face the on-face blue
        # — the classic cue says "On edge" while you push level with one.
        from core.snap import (COLOR_ENDPOINT, COLOR_MIDPOINT, COLOR_ON_EDGE,
                               COLOR_ON_FACE, COLOR_ORIGIN)
        palette = {
            "midpoint": (COLOR_MIDPOINT, "midpoint"),
            "origin": (COLOR_ORIGIN, "origin"),
            "edge": (COLOR_ON_EDGE, "on_edge"),
            "face": (COLOR_ON_FACE, "on_face"),
            "guide_line": (COLOR_ON_EDGE, "on_line"),
            "guide_point": (COLOR_ENDPOINT, "guide_point"),
        }
        rgb, label = palette.get(kind, (COLOR_ENDPOINT, "endpoint"))
        color = QColor.fromRgbF(*rgb, 1.0)
        # The dotted «level with» reference line from the geometry the
        # inference locked onto to the moving cap, dashed in that snap-kind
        # colour. Drawn before the marker's own point is projected: a point
        # behind the eye is dropped whole, a segment crossing it is merely
        # clipped, and the reference must survive (Fix 3).
        guide_provider = getattr(tool, "inference_guide_lines", None)
        if callable(guide_provider):
            pen = QPen(color, 1.5, Qt.DashLine)
            pen.setDashPattern([10.0, 7.0])
            for a, b, g_kind in guide_provider():
                g_rgb = palette.get(g_kind, (COLOR_ENDPOINT, "endpoint"))[0]
                pen.setColor(QColor.fromRgbF(*g_rgb, 1.0))
                painter.setPen(pen)
                seg = self._segment_to_pixels(a, b)
                if seg is not None:
                    painter.drawLine(QPointF(*seg[0]), QPointF(*seg[1]))
        pixel = self._world_to_pixel(world)
        if pixel is None:
            return
        px, py = pixel
        diamond = QPolygonF([
            QPointF(px, py - 9), QPointF(px + 9, py),
            QPointF(px, py + 9), QPointF(px - 9, py),
        ]) if kind == "midpoint" else None
        painter.setPen(QPen(QColor(255, 255, 255, 230), 4.0))
        painter.setBrush(Qt.NoBrush)
        if diamond is not None:
            painter.drawPolygon(diamond)
        else:
            painter.drawRect(QRectF(px - 6, py - 6, 12, 12))
        painter.setPen(QPen(color, 2.4))
        painter.setBrush(QColor.fromRgbF(*rgb, 0.30))
        if diamond is not None:
            painter.drawPolygon(diamond)
        else:
            painter.drawRect(QRectF(px - 6, py - 6, 12, 12))
        text = tr(self._SNAP_LABELS.get(label, ""))
        if text:
            font = QFont()
            font.setPointSize(9)
            painter.setFont(font)
            painter.setPen(QPen(QColor(255, 255, 255, 220)))
            painter.drawText(QPointF(px + 11, py + 17), text)
            painter.setPen(QPen(color))
            painter.drawText(QPointF(px + 10, py + 16), text)

    def _length_label_at(self):
        """``(text, world point, typing)`` of the value the tool shows on the
        geometry — its ``value_label`` or a single segment's length, what is
        typed instead while typing — or None. The bold label below and the
        chrome's value field (views/wgpu_ui.py) both read it."""
        tool = self.active_tool
        if tool is None:
            return None

        # Tool-provided label takes priority (e.g. PushPullTool's signed
        # extrusion distance). Otherwise fall back to the single-segment
        # length used by LineTool.
        label_provider = getattr(tool, "value_label", None)
        if callable(label_provider):
            result = label_provider()
            if result is None:
                return None
            # Extension tools are outside the viewport's control. A malformed
            # value_label() must not break the OpenGL paint loop repeatedly.
            if not isinstance(result, (tuple, list)) or len(result) != 2:
                return None
            text, mid_world = result
            if mid_world is None:        # VCB-only value (Walkthrough eye height)
                return None
        else:
            segments = tool.rubber_band_lines()
            if len(segments) != 1:
                return None
            start, hover = segments[0]
            text = _fmt_len((hover - start).length())
            mid_world = QVector3D(
                (start.x() + hover.x()) * 0.5,
                (start.y() + hover.y()) * 0.5,
                (start.z() + hover.z()) * 0.5,
            )
        if self._value_buffer:
            unitless = getattr(tool, "value_is_unitless", None)
            if callable(unitless) and unitless():
                text = self._value_buffer   # an angle or a count: no unit mark
            else:
                text = _typed_value_text(self._value_buffer)
        return text, mid_world, bool(self._value_buffer)

    def _draw_length_label(self, painter: QPainter) -> None:
        if getattr(self, '_chrome_owns_labels', False):
            return  # the value field on the geometry shows it (views/wgpu_ui.py)
        label = self._length_label_at()
        if label is None:
            return
        text, mid_world, typing = label
        pixel = self._world_to_pixel(mid_world)
        if pixel is None:
            return
        if typing:
            fg = QColor("#0F141B")
            shadow = QColor(255, 220, 130, 235)  # warm tint while typing
        else:
            fg = QColor("#0F141B")
            shadow = QColor(255, 255, 255, 220)
        font = QFont()
        font.setPointSize(10)
        font.setBold(True)
        painter.setFont(font)
        painter.setPen(QPen(shadow))
        painter.drawText(QPointF(pixel[0] + 12, pixel[1] - 7), text)
        painter.setPen(QPen(fg))
        painter.drawText(QPointF(pixel[0] + 11, pixel[1] - 8), text)

    def _measurement_text(self) -> str:
        """Live measurement string for the VCB box: the active tool's
        ``value_label`` text (rectangle dimensions, push distance, …) or the
        single-segment length a line is drawing. Empty when nothing applies."""
        tool = self.active_tool
        if tool is None:
            return ""
        provider = getattr(tool, "value_label", None)
        if callable(provider):
            result = provider()
            if result is not None:
                return result[0]
        segments = tool.rubber_band_lines()
        if len(segments) == 1:
            start, hover = segments[0]
            return _fmt_len((hover - start).length())
        return ""

    def _draw_axis_lock_label(self, painter: QPainter) -> None:
        if getattr(self, '_chrome_owns_labels', False):
            return  # the egui options strip shows it (views/wgpu_ui.py)
        label = {
            "x": ("X", QColor(220, 56, 69)),
            "y": ("Y", QColor(40, 158, 92)),
            "z": ("Z", QColor(51, 102, 199)),
        }[self.axis_lock]
        font = QFont()
        font.setPointSize(11)
        font.setBold(True)
        painter.setFont(font)
        painter.setPen(QPen(label[1]))
        # Through tr(): this read "X axis locked" in English even with the
        # Spanish UI, because an f-string never reaches the catalogue.
        painter.drawText(QPointF(14, 24),
                         tr("{axis} axis locked", axis=label[0]))

    def _draw_plane_lock_label(self, painter: QPainter) -> None:
        """The arrow-key plane lock of a planar tool, in the colour of the
        axis the plane is normal to."""
        from tools.base import PLANE_LOCK_NAMES
        axis = self.active_tool.plane_lock
        color = {"x": QColor(220, 56, 69), "y": QColor(40, 158, 92),
                 "z": QColor(51, 102, 199)}[axis]
        font = QFont()
        font.setPointSize(11)
        font.setBold(True)
        painter.setFont(font)
        painter.setPen(QPen(color))
        painter.drawText(QPointF(14, 24), tr(
            "Drawing plane locked: {plane}", plane=PLANE_LOCK_NAMES[axis]))

    def _draw_inference_label(self, painter: QPainter) -> None:
        """Show 'On Red Axis' style label when an axis inference is active.

        The soft cue offers Shift; the magnetic lock (``kind == "axis"``,
        reached here only when no explicit arrow lock is held) is already
        committed, so it reads without the Shift offer (#962)."""
        snap = self.last_snap
        if snap is None:
            return
        if snap.kind == "axis_inference":
            suffix = f' ({tr("hold Shift to lock")})'
        elif snap.kind == "axis":
            suffix = ""
        else:
            return
        names = {"x": tr("On Red Axis"), "y": tr("On Green Axis"),
                 "z": tr("On Blue Axis")}
        name = names.get(snap.axis or "", "?")
        r, g, b = snap.color
        font = QFont()
        font.setPointSize(10)
        font.setItalic(True)
        painter.setFont(font)
        painter.setPen(QPen(QColor.fromRgbF(r, g, b, 0.95)))
        painter.drawText(QPointF(14, 44), f"{name}{suffix}")

    def _draw_reference_label(self, painter: QPainter) -> None:
        if self.reference_mode is None or self.reference_edge is None:
            return
        r, g, b = (0.85, 0.30, 0.80)
        font = QFont()
        font.setPointSize(11)
        font.setBold(True)
        painter.setFont(font)
        painter.setPen(QPen(QColor.fromRgbF(r, g, b, 1.0)))
        word = "Parallel" if self.reference_mode == "parallel" else "Perpendicular"
        painter.drawText(QPointF(14, 24), f"{word} to reference edge")

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
        """Pixel → world hit on the *current* work plane.

        Plane choice priority:
        1. ``tool.work_plane``, captured at first click when the user clicked
           on a face — keeps the rest of the chain coplanar with that face.
        2. If a tool has a ``start_point``, the plane goes through it; its
           orientation is horizontal for most camera tilts and vertical only
           near the horizon (so dragging up/down can move in Z).
        3. If no ``start_point`` yet and the cursor is over an existing
           face, that face's plane (this is what lets the user draw a new
           polygon *inside* an existing one — e.g. on top of a box).
        4. Ground (Z=0).
        """
        origin, direction = self._pixel_to_ray(x, y)
        if origin is None or direction is None:
            return None
        plane_point, plane_normal = self._current_work_plane(cursor=(x, y))
        # The snap engine needs the plane too — "perpendicular to this edge"
        # only means something within a plane. Stashed instead of asked for
        # again: this runs on every mouse move, and the answer can cost a pick.
        self._last_work_plane = (plane_point, plane_normal)
        tool = self.active_tool
        allow_planar_grazing = False
        if (tool is not None
                and getattr(tool, "work_plane", None) is None
                and getattr(tool, "plane_lock", None) is None
                and getattr(tool, "plane_ref", None) is None):
            from tools.base import PlaneLock
            if isinstance(tool, PlaneLock):
                from core import axes
                normal = QVector3D(plane_normal).normalized()
                # Once started, the plane goes through the first point and
                # keeps the one shown before the click, so it must not turn
                # unreachable toward the horizon either.
                anchored = (
                    getattr(tool, "start_point", None) is not None
                    or (QVector3D(plane_point) - axes.origin()).length() < 1e-5)
                principal = any(
                    abs(QVector3D.dotProduct(normal, axes.axis(name))) > 0.999
                    for name in ("x", "y", "z")
                )
                allow_planar_grazing = anchored and principal
        hit = self._ray_plane(
            origin, direction, plane_point, plane_normal,
            allow_grazing=allow_planar_grazing)
        if hit is not None:
            return hit
        # The ray only grazes the work plane here, so the intersection is
        # junk: it runs off toward the horizon and a single pixel is worth
        # hundreds of metres (measured with the ground plane captured and the
        # camera 16 deg below the horizon: 85 m at mid screen, 405 m twenty
        # pixels higher, and Marco got 1757.25 m on a 100 m plaza while the
        # status bar said "On face"). If a real face IS under the cursor,
        # believe the face — that is what the cursor is pointing at, and what
        # the inference is already promising.
        face, group = self.pick_face_inference(x, y)
        if face is not None:
            from core.snap import face_plane_world
            fpoint, fnormal = face_plane_world(face, getattr(group, "xform", None))
            if fnormal is not None:
                return self._ray_plane(origin, direction, fpoint, fnormal)
        return None

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
        """Whether the ``(point, normal)`` planes ``a`` and ``b`` are the
        same plane: parallel normals AND the same offset — two opposite
        walls are parallel and are not the same plane."""
        na = QVector3D(a[1]).normalized()
        nb = QVector3D(b[1]).normalized()
        if abs(QVector3D.dotProduct(na, nb)) < 1e-3:
            return False
        return abs(QVector3D.dotProduct(QVector3D(b[0]) - QVector3D(a[0]),
                                        na)) < tol

    def _planar_face_plane(self, tool, cursor, captured=None, face_plane=None):
        """The plane of the face under the cursor when a PLANAR shape is
        being drawn over one that is not its captured plane, else ``None``.

        A rectangle, circle, polygon or arc lives in ONE plane, and its
        first click captures the plane of the face it started on — the
        viewport sets ``tool.work_plane`` and the tool lays its geometry
        out there (``PlaneLock.drawing_plane``). A capture, though, also
        FROZE the shape: moving the cursor onto another face left the point
        and the preview on the captured plane while the badge went on
        reading «On face» — @pacaeiro, #545: «se eu coloco o rato
        directamente na face diz (on face), mas o plano permanece
        imutável». A rectangle started on the top vertex of a cube and
        drawn over its front face was the plain case: the cursor's point
        stayed up on the horizontal plane through that vertex.

        An EDGE off the plane already re-planned the shape, through the
        tool's ``snapped_plane``, because its point comes off the plane; a
        face did not, because the axis inference answered with a camera
        plane while the status bar named the face. The face the cursor is
        over is the same bargain and the plainer one: it is what the cursor
        points at and what the badge names, so the shape lands on it.

        A snap that names a POINT of the model (``core.snap.
        POINT_SNAP_KINDS``) is not this bargain and is not decided here:
        the user aimed at that point, not at the face it happens to lie
        on, and the tool re-plans onto it through ``PlaneLock.
        snapped_plane`` — which is where the face's claim is settled too
        (``Tool.snap_kind``). Here the viewport only reads which face the
        cursor is over.

        ``None`` keeps the caller's plane: no cursor, no face under it, the
        face the shape started on (``captured``), a tool that is not a
        planar shape (a LINE keeps its chain coplanar with the face it was
        started on), the arrows already locking a direction (anywhere else
        in here a locked direction leaves the plane where it was), or the
        user's own plane lock — ``plane_lock`` / the magenta ``plane_ref`` —
        which never yields."""
        if cursor is None or tool is None or self.axis_lock is not None:
            return None
        if (getattr(tool, "plane_lock", None) is not None
                or getattr(tool, "plane_ref", None) is not None):
            return None
        from tools.base import PlaneLock
        if not isinstance(tool, PlaneLock):
            return None
        if face_plane is None:
            face_plane = self._hover_face_plane(cursor)
        if face_plane is None:
            return None
        if captured is not None and self._same_plane(captured, face_plane):
            return None        # the face it started on: nothing to re-plan
        return face_plane

    def _ray_plane(self, origin: QVector3D, direction: QVector3D,
                   plane_point: QVector3D, plane_normal: QVector3D,
                   *, allow_grazing: bool = False,
                   ) -> Optional[QVector3D]:
        """Ray/plane hit, or ``None`` when it is behind the camera or the ray
        merely grazes the plane."""
        length = plane_normal.length()
        if length < 1e-9:
            return None
        normal = plane_normal / length
        denom = QVector3D.dotProduct(normal, direction)
        if abs(denom) < 1e-12:
            return None
        if (not allow_grazing
                and abs(denom) < math.sin(math.radians(
                    self.GRAZING_PLANE_DEG))):
            return None
        t = QVector3D.dotProduct(normal, plane_point - origin) / denom
        if t < 0:
            return None
        return origin + direction * t

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
        """Return ``(point, normal)`` of the current drawing plane.

        Priority: tool-captured plane > camera-aware plane through the active
        ``start_point`` > face under cursor (face-plane inference) > ground —
        except that a PLANAR shape re-plans onto the face the cursor is over,
        capture or not (#545, see :meth:`_planar_face_plane`). A shape lives
        in ONE plane, and the face the cursor is over is the one the user
        means.
        """
        tool = self.active_tool
        if tool is not None:
            # A tool mid-operation may know better than any plane below —
            # Push/Pull reads the cursor along its normal and must keep
            # hearing hovers and clicks above the horizon (see
            # ``Tool.drag_plane``).
            own_plane = getattr(tool, "drag_plane", None)
            own = own_plane(self) if own_plane is not None else None
            if own is not None:
                return own
        captured = getattr(tool, "work_plane", None) if tool is not None else None
        if captured is not None:
            # (#545) A planar shape re-plans onto the face the cursor is
            # over, and takes its capture with it: the preview (drawn on the
            # tool's own plane, ``PlaneLock.drawing_plane``) and the point
            # would otherwise sit on two different planes — and that split
            # IS the bug. ``_planar_face_plane`` holds the whole argument.
            face_plane = self._planar_face_plane(tool, cursor, captured)
            if face_plane is not None:
                tool.work_plane = face_plane
                return face_plane
            # The escape hatch: orbiting down to the horizon means "I
            # want to draw UPWARD now". A first click on a horizontal face
            # (the ground, a slab) captures its plane and would pin the whole
            # chain flat forever; at near-horizon views, where the horizontal
            # plane is unreadable anyway, a HORIZONTAL captured plane yields
            # to the vertical plane through the start point so the line can
            # rise in Z. Vertical captured planes (walls) already allow it.
            start = (getattr(tool, "start_point", None)
                     if tool is not None else None)
            from tools.base import PlaneLock
            if (start is not None and abs(captured[1].normalized().z()) > 0.94
                    and getattr(tool, "plane_lock", None) is None
                    and not isinstance(tool, PlaneLock)):
                # (an arrow-key plane lock is exactly what the user asked
                # for: it never yields; and a PLANAR shape — rectangle,
                # circle, arc — lives in its captured plane by definition:
                # handing it points on another plane squashed it to a line)
                vertical = self._near_horizon_vertical(start)
                if vertical is not None:
                    return vertical
            return captured

        start = getattr(tool, "start_point", None) if tool is not None else None
        if start is None:
            from core import axes
            from tools.base import PlaneLock
            if isinstance(tool, PlaneLock):
                locked = tool.locked_work_plane(axes.origin())
                if locked is not None:
                    # A reference point (a dwell, like the Line's) is only
                    # reachable by alignment if the plane passes through it:
                    # otherwise its axis lines never meet the plane. When the
                    # cursor is on one of them, the lock takes that offset.
                    if cursor is not None:
                        through = self._locked_plane_through_reference(
                            tool, cursor)
                        if through is not None:
                            return through
                    # The arrow chooses the plane's ORIENTATION only. Over a
                    # face, "On face" names a point, so the locked plane is
                    # anchored there (same normal); with no face under the
                    # cursor it stays the world plane.
                    if cursor is not None:
                        face_plane = self._hover_face_plane(cursor)
                        if face_plane is not None:
                            hit = self._ray_hit_plane(cursor, face_plane)
                            if hit is not None:
                                return tool.locked_work_plane(hit)
                    return locked
            # First-click hover: if the cursor is over an existing face —
            # loose OR a group's (drawing/measuring on top of an imported
            # reference model) — use that face's plane so a new polygon
            # drawn "inside" it lands on the face instead of the ground.
            if cursor is not None and tool is not None:
                pick = (getattr(self, "pick_face_inference", None)
                        or self.pick_face_placement)
                face, grp = pick(cursor[0], cursor[1])
                if face is not None:
                    from core.snap import face_plane_world
                    return face_plane_world(face, getattr(grp, "xform", None))
                # No geometry under the cursor, but maybe a reference image:
                # its plane becomes the drawing plane, which is the whole
                # point of importing a scan — you trace ON it, at its own
                # tilt, not on the ground underneath.
                image = self.image_plane_at(cursor[0], cursor[1])
                if image is not None:
                    return image.plane()
            if isinstance(tool, PlaneLock) and cursor is not None:
                plane = self._camera_planar_work_plane(cursor)
                if plane is not None:
                    return plane
            # In an axis-aligned (standard) view, an unsnapped first point
            # belongs on the plane FACING the camera through the model
            # centre — so measuring/drawing in a front view stays in the
            # frontal plane instead of dropping to the ground. Oblique
            # orbit views keep the ground fallback (freehand unchanged).
            from core.snap import first_point_work_plane
            forward = self.camera.target - self.camera.eye()
            lo, hi = self.scene.bounds()
            center = ((lo + hi) * 0.5 if lo is not None
                      else QVector3D(0.0, 0.0, 0.0))
            plane = first_point_work_plane(forward, center)
            if plane is not None:
                return plane
            return QVector3D(0.0, 0.0, 0.0), QVector3D(0.0, 0.0, 1.0)

        base = self._start_point_plane(tool, start)
        from tools.base import PlaneLock
        hover_plane = getattr(tool, "hover_plane", None)
        if isinstance(tool, PlaneLock) and hover_plane is not None:
            # A planar shape started on a free point lives in the plane the
            # cursor showed at the first click. The near-horizon vertical
            # escape in ``_start_point_plane`` is for a Line to rise in Z;
            # here it flipped the shape from XY to XZ on that click.
            base = (QVector3D(start), QVector3D(hover_plane[1]))
        if cursor is not None and self.axis_lock is None:
            # On Face for the SECOND point (Move's target, the
            # tape's far end, a line's end): the cursor over another face
            # lands ON that face. The axis inference still wins — a line
            # drawn straight up over a slab keeps rising instead of dropping
            # onto it — and the geometry being moved never offers its own
            # (already vacated) faces.
            fp = self._hover_face_plane(cursor)
            if fp is not None:
                from tools.base import PlaneLock
                if isinstance(tool, PlaneLock):
                    # A PLANAR shape does not get the axis escape below
                    # (#545, @pacaeiro: «se eu coloco o rato directamente
                    # na face diz (on face), mas o plano permanece
                    # imutável»). An EDGE already re-planned the shape,
                    # through ``snapped_plane``, because its point comes
                    # off the plane; a face did not, because the axis
                    # inference handed back the camera plane through the
                    # first corner while the status bar went on reading
                    # «On face» — and the shape's own axes had already
                    # taken the wall's, so the preview and the point sat
                    # on two different planes. A rectangle started on the
                    # top vertex of a cube was the plain case: over the
                    # front face the point came back at (2.00, -1.20,
                    # 2.00), the horizontal plane through that vertex.
                    # The point the cursor is over is the one the user
                    # means: a shape lives in ONE plane, so the face wins
                    # outright. The arrows and the Down reference never
                    # reach this branch — a lock is captured as the
                    # tool's ``work_plane`` by the first click and
                    # returned above. A line drawn straight up over a
                    # slab is not a PlaneLock and keeps rising. A snap
                    # that names a POINT of the model is not this case
                    # either: ``PlaneLock.snapped_plane`` re-plans the
                    # shape onto the point (``Tool.snap_kind``).
                    return fp
                cand = self._ray_hit_plane(cursor, base)
                angle = (getattr(tool, "magnetic_axis_deg", None)
                         or self.inference_angle_deg)
                from core.snap import _detect_axis_alignment
                if cand is None or _detect_axis_alignment(
                        start, cand, angle) is None:
                    return fp
        return base

    def _front_face_at(self, x: float, y: float, which: str = "sel"):
        """``(face, placement)`` of the front-most face on the ray through
        pixel ``(x, y)`` that passes ``which`` (see :meth:`_face_hits`), or
        ``None``. A bare pass with no memo: the face-crossing search asks it
        at pixels around the cursor, and :meth:`pick_face_any`'s memo holds
        the cursor's own answer."""
        o, d = self._pixel_to_ray(x, y)
        if o is None or d is None:
            return None
        import numpy as np
        idx = self._pick_index(near=DocumentInteraction._ray_near(self, x, y))
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        if nat is not None and _FRONT_LEAN:
            return self._front_face_lean(idx, nat, o, d, which)
        hits = DocumentInteraction._face_hits(self, idx, o, d, which)
        if hits is None or len(hits.t) == 0:
            return None
        i = int(np.argmin(hits.t))
        return hits.ent(i)[0], hits.place(i)

    def _front_face_lean(self, idx, nat, o, d, which: str = "sel"):
        """_front_face_at's answer over the engine's lazy placements (#158)
        without gathering every face the ray meets: the index's nearest
        face, then the lazy placements' only nearer than it (a tie goes to
        the index's, as the argmin over the index's then the lazy faces
        does), and of those only the nearest made into a face — the
        first of equals in the order _lazy_face_hits keeps. Each probe of
        a face crossing (see _crossing_on_screen) is one of these, ~2 ms
        over the plant when every face met was made."""
        import numpy as np
        best_t, best = float("inf"), None
        if len(idx.entities):
            if which == "sel":
                mask = idx.ent_sel
            elif which == "infer":
                mask = getattr(idx, "ent_infer", idx.ent_sel)
            else:
                mask = idx.ent_vis
            face_t = self._ray_hits(idx, o, d, mask, own=False)
            if face_t is not None and len(face_t):
                i = int(np.argmin(face_t))
                if np.isfinite(face_t[i]):
                    best_t = float(face_t[i])
                    pidx = getattr(idx, "ent_place_idx", None)
                    place = (idx.ent_placements[int(pidx[i])]
                             if pidx is not None and i < len(pidx)
                             and pidx[i] >= 0
                             else _entity_owner(idx.entities, i))
                    best = (idx.entities[i][0], place)
        if which == "sel":
            rows = DocumentInteraction._lazy_sel_rows(self, nat)
        elif which == "infer":
            rows = nat[1]
        else:
            rows = nat[2]
        if not len(rows):
            return best
        iset = nat[0]
        oo = np.array([o.x(), o.y(), o.z()])
        dd = np.array([d.x(), d.y(), d.z()])
        # A margin past the index's nearest: the engine's boxes decide
        # which triangles to hand, the exact test below which win.
        reach = (best_t * 1.001 + 1e-6 if np.isfinite(best_t)
                 else float("inf"))
        pl, tr, v0, e1, e2 = iset.ray_cands(
            np.ascontiguousarray(rows, np.int64), tuple(oo.tolist()),
            tuple(dd.tolist()), reach)
        if not len(pl):
            return best
        t, hit = _moller_trumbore(oo, dd, v0, e1, e2)
        k = np.flatnonzero(hit & (t < best_t))
        if not len(k):
            return best
        tmin = t[k].min()
        k = k[t[k] == tmin]
        groups = self._pick_lazy_table()[0]
        lazy_pos = self._pick_cands_memo[3]
        win = None
        for i, r in zip(pl[k].tolist(), tr[k].tolist()):
            g = groups[i]
            base = self._proto_base_chunk(g.mesh, _paint_of(self, g))
            j = int(base["tri_ent"][r])
            key = (lazy_pos[id(g)][0], j)
            if win is None or key < win[0]:
                win = (key, base, i, j)
        _key, base, i, j = win
        return base["faces"][j], groups[i]

    #: How far from the cursor (px) a face crossing snaps, and how far
    #: either side of the crossing the search checks that each face shows.
    _CROSSING_RING_PX = 8.0
    _CROSSING_SIDE_PX = 3.0

    def _face_crossing_point(self, px: float, py: float):
        """Where the face under the cursor meets another face that crosses
        it with no edge between them — a box sunk into the floor, a beam
        through a wall: ``(world point, context)`` or ``None`` (#819,
        the modeller's «Intersection»). A face that goes THROUGH the one in
        front is met by the cursor's own ray a little behind it, so the
        candidates come free from that ray's hits (already cast for this
        hover); a crossing counts only if, either side of it on screen,
        one face shows and then the other — the two really meet there, in
        view, and neither is merely in front of the other. ``context`` is
        "group" / "component" when one of them is inside one, for the
        «in group» tip."""
        if _active_cut(self.scene) is not None:
            return None
        f_face, f_place = self.pick_face_inference(px, py)   # memoised
        if f_face is None:
            return None
        o, d = self._pixel_to_ray(px, py)
        if o is None or d is None:
            return None
        import numpy as np
        idx = self._pick_index(near=DocumentInteraction._ray_near(self, px, py))
        hits = self._infer_hits(idx, o, d)                    # memoised
        if hits is None or not np.isfinite(hits.t).any():
            return None
        face_t = hits.t
        t_f = float(face_t.min())
        r = self._CROSSING_RING_PX
        o2, d2 = self._pixel_to_ray(px + r, py)
        if o2 is None or d2 is None:
            return None
        reach = ((o2 + d2 * t_f) - (o + d * t_f)).length() * 50.0
        # ``>=``: with the cursor right on the crossing both faces are hit
        # at the same depth (the face itself is skipped below).
        cand = np.where((face_t >= t_f) & (face_t <= t_f + reach))[0]
        if len(cand) == 0:
            return None
        cand = cand[np.argsort(face_t[cand], kind="stable")][:7]
        from core.snap import face_plane_world
        p_f, n_f = face_plane_world(f_face, getattr(f_place, "xform", None))
        f = (f_face, f_place, p_f, n_f)
        pairs = []
        for i in cand:
            i = int(i)
            g_face = hits.ent(i)[0]
            g_place = hits.place(i)
            if g_face == f_face and g_place is f_place:
                continue
            p_g, n_g = face_plane_world(g_face, getattr(g_place, "xform", None))
            u = QVector3D.crossProduct(n_f, n_g)
            if u.length() < 0.02:               # (near) coplanar: no crossing
                continue
            g = (g_face, g_place, p_g, n_g, u)
            sides = self._crossing_sides(px, py, o, d, f, g)
            if sides is not None:
                pairs.append((g, sides))
        # The probes either side of every crossing bake what they cross
        # first, together: one at a time, each that met a new lazy
        # placement rebuilt the index for the next (up to five rebuilds a
        # pointer move with Rectangle over the plant of #158).
        mat = getattr(self, "_pick_materialize", None)
        if (mat is not None and hasattr(self, "_pick_lazy_table")
                and DocumentInteraction._ray_near(self, px, py) != "skip"):
            for _g, (_x, probes) in pairs:
                for q in probes:
                    mat(("px", q[0], q[1]))
        for g, sides in pairs:
            found = self._crossing_on_screen(f, g, sides)
            if found is not None:
                return found
        return None

    def _crossing_sides(self, px, py, o, d, f, g):
        """The crossing of plane ``f`` with plane ``g`` nearest the cursor
        ray, if it is within reach on screen: ``(world point, (pixel one
        side, pixel the other side))``, or ``None``."""
        from core.snap import face_crossing_near_ray
        _f_face, _f_place, p_f, n_f = f
        _g_face, _g_place, p_g, n_g, _u = g
        return face_crossing_near_ray(
            px, py, o, d, (p_f, n_f), (p_g, n_g), self._world_to_pixel,
            max(self.edge_snap_threshold_px, self._CROSSING_RING_PX),
            self._CROSSING_SIDE_PX)

    def _crossing_on_screen(self, f, g, found):
        """The crossing ``found`` by :meth:`_crossing_sides` if it shows
        face ``f`` on one side and face ``g`` on the other (see
        :meth:`_face_crossing_point`): ``(world point, context)`` or
        ``None``."""
        f_face, f_place = f[0], f[1]
        g_face, g_place = g[0], g[1]
        x, probes = found
        want = {(element_identity(f_face), id(f_place)), (element_identity(g_face), id(g_place))}
        ids = set()
        for q in probes:
            # Each side must show one of the two, the sides not the same
            # one: a side that does not ends the search there (each probe
            # is a ray through the model, ~2 ms over the plant of #158,
            # and dense spots try five crossings a move that fail).
            h = self._front_face_at(q[0], q[1], "infer")
            if h is None or (element_identity(h[0]), id(h[1])) not in want:
                return None
            ids.add((element_identity(h[0]), id(h[1])))
        if ids != want:
            return None
        context = None
        found_ctx = []
        for place in (f_place, g_place):
            if place is None:
                continue        # the open context's own loose geometry
            word = _snap_context_of(self, place)
            if word is not None:
                found_ctx.append(word)
        if "outside" in found_ctx:
            context = "outside"
        elif found_ctx:
            context = found_ctx[0]
        return x, context

    def _face_outline_world(self, px: float, py: float):
        """The face under the cursor as ``(outer loop, holes, context)`` in
        world space (``context``: "group" / "component" when it is inside
        one), or ``None``. What a line piercing the face is tested against.
        The rest of the model around an open group counts: a line drawn
        inside it can pierce a face left outside."""
        face, place = self.pick_face_inference(px, py)    # memoised
        if face is None:
            return None
        xf = getattr(place, "xform", None)

        def w(p):
            return xf.map(QVector3D(p)) if xf is not None else QVector3D(p)
        outer = [w(p) for p in face.vertices]
        holes = [[w(p) for p in h] for h in face.holes]
        context = None
        if place is not None:
            owner = self._owner_of(place)
            is_comp = getattr(owner, "is_instance", None)
            if is_comp is not None:
                # A reference wrapper (the outer loose mesh) has no
                # ``is_instance``: it is not "in" a group, it IS the root.
                context = ("component" if is_comp() else "group")
        return outer, holes, context

    def _still_face_under(self, px: float, py: float) -> bool:
        """Whether a face the drag is NOT bending is under the cursor —
        the «On face» cue never names the face being moved."""
        face, grp = self.pick_face_inference(px, py)
        if face is None:
            return False
        moving = getattr(self.active_tool, "moving_face_ids", None)
        return not (grp is None and callable(moving) and element_identity(face) in moving())

    def _face_center_under(self, px: float, py: float):
        """The «Face center» inference: ``(point, context)`` for the centre
        of area of the face under the cursor, or ``None`` — no face, the
        centre falls outside it (an L, a ring), it is too far from the
        cursor to matter, or the face is in motion (a face being bent, a
        group being moved: the point would chase the cursor)."""
        face, place = self.pick_face_placement(px, py)
        if face is None:
            return None
        tool = self.active_tool
        if place is None:
            moving = getattr(tool, "moving_face_ids", None)
            if callable(moving) and element_identity(face) in moving():
                return None
            context = None
        else:
            owner = self._owner_of(place)
            excl = getattr(tool, "snap_excluded", None)
            excl = excl() if callable(excl) else None
            if excl is not None and (id(owner) in excl[1] or id(place) in excl[1]):
                return None
            is_comp = getattr(owner, "is_instance", None)
            context = "component" if (callable(is_comp) and is_comp()) else "group"
        from core.snap import face_center_of_face
        c = face_center_of_face(face, getattr(place, "xform", None))
        if c is None:
            return None
        pc = self._world_to_pixel(c)
        if pc is None or math.hypot(pc[0] - px, pc[1] - py) > self.POINT_INFERENCE_PX:
            return None
        return c, context

    #: More selected faces than this and their centres are neither drawn
    #: nor offered: a «select all» would bury the model in dots.
    FACE_CENTER_SELECTION_MAX = 500

    def _selected_face_centers(self) -> list:
        """The «Face center» of every selected face, ``[QVector3D]`` in
        world space: drawn as dots and offered to the snap engine wherever
        the cursor is, so a ring's centre — in its hole, where no face is
        under the cursor — can be reached. Only faces picked one by one: a
        selected group offers none (a whole object would fill with dots).
        An open group's faces are loose and already in world space. A face
        a tool is bending is left out (its centre would chase the cursor).
        Cached per scene version and selection."""
        faces = [e for e in self.scene.selection if isinstance(e, Face)]
        if not faces or len(faces) > self.FACE_CENTER_SELECTION_MAX:
            return []
        moving = getattr(self.active_tool, "moving_face_ids", None)
        moving = moving() if callable(moving) else ()
        key = (self.scene.version, tuple(element_identity(f) for f in faces),
               frozenset(moving))
        cached = getattr(self, "_sel_face_centers", None)
        if cached is not None and cached[0] == key:
            return cached[1]
        from core.snap import face_center_of_face
        pts = []
        for f in faces:
            if element_identity(f) in moving:
                continue
            c = face_center_of_face(f)
            if c is not None:
                pts.append(c)
        self._sel_face_centers = (key, pts)
        return pts

    def _face_centers_near(self, px: float, py: float) -> list:
        """The «Face center» points for the snap engine, ``[(point,
        context)]``: the face under the cursor, and the selected faces whose
        centre is near it."""
        out = []
        under = self._face_center_under(px, py)
        if under is not None:
            out.append(under)
        for c in self._selected_face_centers():
            pc = self._world_to_pixel(c)
            if pc is not None and math.hypot(pc[0] - px, pc[1] - py) <= self.POINT_INFERENCE_PX:
                out.append((c, None))
        return out

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
        """The ScreenTip's context for the face under the cursor: "outside"
        while a group / component is open and the face is not part of it;
        "group" / "component" for a placement; ``None`` for the open
        context's own loose mesh (and at the root)."""
        face, place = self.pick_face_inference(px, py)
        if face is None:
            return None
        return _snap_context_of(self, place)

    def _hover_face_plane(self, cursor):
        """World plane of the face under the cursor, or None.

        Selected geometry remains a valid plane target for planar drawing
        tools; other tools must not land on geometry they are moving. The
        rest of the model outside an open group / component IS a valid
        target (faded, never selectable): that is how a part is aligned to
        the floor it sits on when the model is made of components.
        """
        face, grp = self.pick_face_inference(cursor[0], cursor[1])
        if face is None:
            return None
        sel = self.scene.selection
        owner = self._owner_of(grp) if grp is not None else None
        from tools.base import PlaneLock
        if (not isinstance(self.active_tool, PlaneLock)
                and ((owner is not None and owner in sel)
                     or (grp is None and face in sel))):
            return None
        if grp is None:
            # A face the drag is bending (one corner on a grabbed vertex)
            # is not somewhere to land: the point would follow the face,
            # the face the point, and the geometry shook under a still
            # mouse.
            moving = getattr(self.active_tool, "moving_face_ids", None)
            if callable(moving) and element_identity(face) in moving():
                return None
        from core.snap import face_plane_world
        return face_plane_world(face, getattr(grp, "xform", None))

    def _locked_plane_through_reference(self, tool, cursor):
        """The tool's locked plane (same normal) through an acquired or
        encouraged point, when the cursor lies on one of that point's axis
        lines inside the plane; else ``None``.

        The arrow only chooses the plane's ORIENTATION. A point the cursor
        dwelt on (a circle's centre) is aligned with by moving along one of
        the two world axes the plane holds, and those lines exist only in
        the plane through the point."""
        from core import axes
        refs = list(self._encouraged)
        if self._acquired_point is not None:
            refs.append(self._acquired_point)
        for ref in reversed(refs):
            plane = tool.locked_work_plane(ref)
            if plane is None:
                continue
            hit = self._ray_hit_plane(cursor, plane)
            if hit is None:
                continue
            normal = QVector3D(plane[1]).normalized()
            for name in ("x", "y", "z"):
                a = axes.axis(name)
                if abs(QVector3D.dotProduct(a, normal)) > 1e-3:
                    continue
                foot = ref + a * QVector3D.dotProduct(hit - ref, a)
                px = self._world_to_pixel(foot)
                if px is None:
                    continue
                if math.hypot(px[0] - cursor[0], px[1] - cursor[1]) \
                        <= self.snap_threshold_px:
                    return plane
        return None

    def _ray_hit_plane(self, cursor, plane):
        origin, direction = self._pixel_to_ray(cursor[0], cursor[1])
        if origin is None or direction is None:
            return None
        point, normal = plane
        denom = QVector3D.dotProduct(normal, direction)
        if abs(denom) < 1e-6:
            return None
        t = QVector3D.dotProduct(normal, point - origin) / denom
        return None if t < 0 else origin + direction * t

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
        """Cheap content fingerprint of a group's mesh — counts, a coordinate
        checksum, render-relevant attrs and soft flags. Self-healing cache key
        (no dirty-flag invariant to maintain): any geometry/paint change on
        the group produces a new value; ~20 ms on a 17k-face mesh versus the
        ~1.5 s rebuild it lets untouched groups skip."""
        s = 0.0
        for v in mesh.vertices:
            p = v.position
            s += p.x() + p.y() * 1.000003 + p.z() * 1.000007
        a = DocumentInteraction._faces_paint_fp(mesh)
        soft = 0
        flat = 0
        hid = 0
        for i, e in enumerate(mesh.edges):
            if getattr(e, "soft", False):
                soft += 1
                if getattr(e, "flat", False):
                    # Index-sensitive like ``hid``: which seams shade flat.
                    flat ^= ((i + 1) * 2654435761) & 0xFFFFFFFF
            if getattr(e, "hidden", False):
                # Index-sensitive (not a count): Hide edge A + undo + hide
                # edge B leaves the count equal while the baked edge VBO
                # differs. Deterministic mix, no hash() — the disk digest
                # reuses this term across runs.
                hid ^= ((i + 1) * 2654435761) & 0xFFFFFFFF
        if flat:
            # Soft only («Smooth normals» off) bakes otherwise; meshes
            # with none keep their digests.
            soft = (soft, flat)
        fp = (len(mesh.vertices), len(mesh.edges), len(mesh.faces),
              round(s, 4), a, soft, hid)
        # Reverse Faces changes none of the above (#197): the mesh counts
        # its flips, and only a flipped mesh carries the extra term.
        wr = getattr(mesh, "_winding_rev", 0)
        return fp + (("winding", wr),) if wr else fp

    @staticmethod
    def _faces_paint_fp(mesh) -> int:
        """The paint term of :meth:`_mesh_fingerprint`: every face attr a
        render bakes, mixed with the face's index."""
        a = 0
        for i, f in enumerate(mesh.faces):
            if f.attrs:
                c = f.attrs.get("color")
                t = f.attrs.get("texture")
                # Everything the chunk bakes, or a repaint that changes
                # only that term serves the stale chunk: the back side, the
                # translucency and a positioned map were missing here, so
                # painting the BACK of a face inside a group changed
                # nothing on screen (Marco, 2026-09-11, the first thing he
                # tried with two-sided paint). The disk digest already
                # walked all of them.
                back = f.attrs.get("back")
                a ^= hash((i,
                           None if c is None else tuple(c),
                           None if not t else (t.get("path"), t.get("sw"),
                                               t.get("sh"), t.get("rot", 0),
                                               tuple(t.get("uvw") or ())),
                           f.attrs.get("layer"),
                           f.attrs.get("opacity"),
                           repr(back) if back else None,
                           bool(f.attrs.get("hidden"))))
        return a

    def _group_fp(self, group):
        """Fingerprint of a group's mesh, memoised per scene version — the
        chunk is consulted several times per frame/stroke (faces, edges,
        silhouette, pick index) and the fingerprint walk over a 130k-vertex
        reference model costs ~200 ms."""
        key = (id(group), self.scene.version, id(self.scene.mesh))
        memo = getattr(self, "_fp_memo", None)
        if memo is None:
            memo = self._fp_memo = {}
        fp = memo.get(key)
        if fp is None:
            if len(memo) > 64:
                memo.clear()
            _f0 = _time_mod.perf_counter() if _PERF else 0.0
            fp = memo[key] = self._mesh_fingerprint(group.mesh)
            if _PERF:
                _plog("fingerprint", (_time_mod.perf_counter() - _f0) * 1000.0,
                      extra=f"nv={fp[0]}")
        return fp

    @staticmethod
    def _translation_probe(entry, mesh):
        """When the group's mesh is the chunk, purely TRANSLATED, return the
        delta; else ``None``. Counts must match and every sampled vertex must
        have moved by the same vector. This is what keeps dragging a 100k-face
        reference group interactive: Move live-deforms the mesh per frame, and
        a full chunk rebuild at that scale takes ~15 s."""
        verts = mesh.vertices
        if (len(verts) != entry["nv"] or len(mesh.edges) != entry["ne"]
                or len(mesh.faces) != entry["nf"] or not entry["samples"]):
            return None
        i0, p0 = entry["samples"][0]
        d = verts[i0].position - QVector3D(*p0)
        if d.length() < 1e-9:
            return None                   # unchanged, or a non-geometric edit
        for i, p in entry["samples"][1:]:
            # float32 storage rounds each translated vertex differently (up
            # to ~1e-5 at building-scale coordinates); a real rotation moves
            # samples apart by millimetres — no ambiguity at this tolerance.
            if ((verts[i].position - QVector3D(*p)) - d).length() > 2e-4:
                return None
        return d

    @staticmethod
    def _samples_match(entry, mesh) -> bool:
        verts = mesh.vertices
        if len(verts) != entry["nv"]:
            return False
        for i, p in entry["samples"]:
            if (verts[i].position - QVector3D(*p)).length() > 1e-6:
                return False
        return True

    @staticmethod
    def _shift_obb(entry, d) -> None:
        """Carry the cached selection box along a pure translation.

        The box is world-space geometry cached ON the chunk, and the shift
        fast paths moved every array except this one — so moving a component
        left its selection box behind, drawing a ghost rectangle where the
        object used to be (Marco, on the hedge). ``lo``/``hi`` are the
        extents along the box's own axes, so a translation moves them by the
        projection of ``d`` on each axis; the frame itself never turns."""
        obb = entry.get("obb")
        if obb is None:
            return
        frame, lo, hi = obb
        off = [a.x() * d.x() + a.y() * d.y() + a.z() * d.z() for a in frame]
        entry["obb"] = (frame,
                        tuple(c + o for c, o in zip(lo, off)),
                        tuple(c + o for c, o in zip(hi, off)))

    def _shift_chunk(self, entry, d, mesh) -> None:
        """Translate every cached array of ``entry`` by ``d`` in place —
        NumPy adds instead of a rebuild — and refresh samples + fingerprint
        analytically."""
        import numpy as np
        dx = np.array([d.x(), d.y(), d.z()])
        self._shift_obb(entry, d)
        bb = entry.get("bbox")
        if bb is not None:
            entry["bbox"] = (
                (bb[0][0] + d.x(), bb[0][1] + d.y(), bb[0][2] + d.z()),
                (bb[1][0] + d.x(), bb[1][1] + d.y(), bb[1][2] + d.z()))

        def flat3(b):
            a = np.frombuffer(b, dtype=np.float32).reshape(-1, 3).copy()
            a += dx
            return a.astype(np.float32).tobytes()

        entry["edges"] = flat3(entry["edges"])
        if entry.get("dback"):
            entry["dback"] = flat3(entry["dback"])
        vc = np.frombuffer(entry["vcol"], dtype=np.float32).reshape(-1, 6).copy()
        vc[:, :3] += dx
        entry["vcol"] = vc.astype(np.float32).tobytes()
        tex = {}
        for k, v in entry["by_texture"].items():
            a = np.frombuffer(v, dtype=np.float32).reshape(-1, 5).copy()
            a[:, :3] += dx
            tex[k] = a.astype(np.float32).tobytes()
        entry["by_texture"] = tex
        tc = {}
        for k, v in entry.get("tcol", {}).items():
            a = np.frombuffer(v, dtype=np.float32).reshape(-1, 6).copy()
            a[:, :3] += dx
            tc[k] = a.astype(np.float32).tobytes()
        entry["tcol"] = tc
        tt = {}
        for k, v in entry.get("ttex", {}).items():
            a = np.frombuffer(v, dtype=np.float32).reshape(-1, 5).copy()
            a[:, :3] += dx
            tt[k] = a.astype(np.float32).tobytes()
        entry["ttex"] = tt
        if entry.get("back_vcol"):
            a = np.frombuffer(entry["back_vcol"], dtype=np.float32).reshape(-1, 6).copy()
            a[:, :3] += dx
            entry["back_vcol"] = a.astype(np.float32).tobytes()
        bt = {}
        for k, v in entry.get("back_tex", {}).items():
            a = np.frombuffer(v, dtype=np.float32).reshape(-1, 5).copy()
            a[:, :3] += dx
            bt[k] = a.astype(np.float32).tobytes()
        entry["back_tex"] = bt
        btt = {}
        for k, v in entry.get("back_ttex", {}).items():
            a = np.frombuffer(v, dtype=np.float32).reshape(-1, 5).copy()
            a[:, :3] += dx
            btt[k] = a.astype(np.float32).tobytes()
        entry["back_ttex"] = btt
        btc = {}
        for k, v in entry.get("back_tcol", {}).items():
            a = np.frombuffer(v, dtype=np.float32).reshape(-1, 6).copy()
            a[:, :3] += dx
            btc[k] = a.astype(np.float32).tobytes()
        entry["back_tcol"] = btc
        if entry.get("fvcol"):
            a = np.frombuffer(entry["fvcol"], dtype=np.float32).reshape(-1, 6).copy()
            a[:, :3] += dx
            entry["fvcol"] = a.astype(np.float32).tobytes()
        if entry["v0"] is not None:
            entry["v0"] = entry["v0"] + dx
        for kk in ("soft_c0", "soft_c1"):
            if len(entry[kk]):
                entry[kk] = entry[kk] + dx
        if entry["soft_pts"] is not None:
            sp = entry["soft_pts"].reshape(-1, 3) + dx
            entry["soft_pts"] = sp.astype(np.float32).reshape(-1, 6)
        entry["samples"] = [(i, (mesh.vertices[i].position.x(),
                                 mesh.vertices[i].position.y(),
                                 mesh.vertices[i].position.z()))
                            for i, _p in entry["samples"]]
        entry["coordsum"] += entry["nv"] * (
            d.x() + d.y() * 1.000003 + d.z() * 1.000007)
        fp = entry["fp"]
        entry["fp"] = ((fp[0], fp[1], fp[2], round(entry["coordsum"], 4))
                       + fp[4:])
        # float32 vertex storage makes the analytic checksum drift from a
        # fresh walk — mark it approximate so the next full comparison
        # verifies by samples instead of rebuilding 100k faces for nothing.
        entry["fp_approx"] = True


    def _instance_chunk(self, group):
        """Chunk of a component INSTANCE: the shared prototype's chunk (built
        once per prototype mesh, in local coordinates) transformed to world by
        ``group.xform``. Cached per instance and re-derived only when the
        transform or the prototype changes; a pure translation delta shifts
        the cached arrays instead of re-transforming."""
        import numpy as np
        mesh = group.mesh
        # A painted instance (issue #47) bakes its own variant of the
        # prototype: default faces in the container's paint. Unpainted
        # siblings keep sharing the plain bake.
        base = self._group_chunk(
            _proto_wrapper(self, mesh, _paint_of(self, group)))
        xf = group.xform
        # The base's rev too: moving the whole of an open mesh shifts its
        # bake in place, same uid (_shift_chunk).
        key = (base["uid"], tuple(xf.data()), base.get("rev"))
        cache = getattr(self, "_inst_chunks", None)
        if cache is None:
            cache = self._inst_chunks = {}
        cur = cache.get(id(group))
        if cur is not None and cur["ikey"] == key:
            return cur
        if (cur is not None and cur["ikey"][0] == base["uid"]
                and cur["ikey"][2] == key[2]):
            old, new = cur["ikey"][1], key[1]
            # QMatrix4x4.data() is column-major: translation at 12/13/14.
            lin = (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 15)
            if all(abs(old[i] - new[i]) < 1e-12 for i in lin):
                d = QVector3D(new[12] - old[12], new[13] - old[13],
                              new[14] - old[14])
                self._shift_instance_entry(cur, d)
                # What the pick index reads is made as a fresh bake makes
                # it: shifted, it was the old bake plus a float32 offset,
                # which the native engine — answering for placements it
                # does not bake (#158) — cannot know.
                _L, tv, tp = _placed_maps(xf)
                a = np.frombuffer(base["edges"], np.float32).reshape(-1, 3)
                cur["edges"] = tp(a.astype(np.float64)).astype(
                    np.float32).tobytes()
                if base["v0"] is not None:
                    cur["v0"] = tp(base["v0"])
                    cur["e1"] = tv(base["e1"])
                    cur["e2"] = tv(base["e2"])
                cur["ikey"] = key
                cur["rev"] += 1
                return cur
        L, tv, tp = _placed_maps(xf)

        det = float(np.linalg.det(L))
        mirrored = det < 0.0

        def tp32(raw, stride, cols=3, tris=True):
            a = np.frombuffer(raw, np.float32).reshape(-1, stride).copy()
            a[:, :cols] = tp(a[:, :cols].astype(np.float64)).astype(np.float32)
            if mirrored and tris and len(a) % 3 == 0:
                # A mirror turns every triangle inside out: what wound
                # counter-clockwise now winds clockwise, and GL would call
                # the painted side the BACK — the flipped lamp post came
                # out in the style's back colour, its yellow base and blue
                # panels gone («hice mirror a un componente y sus texturas
                # desaparecen», Marco, 2026-09-11). Swapping two corners of
                # each triangle keeps the front the front.
                a = a.reshape(-1, 3, stride)[:, [0, 2, 1], :].reshape(-1, stride)
            return a.tobytes()
        try:
            n_mat = np.linalg.inv(L).T
        except np.linalg.LinAlgError:
            n_mat = np.eye(3)

        def tn(a):                       # unit normals
            if not len(a):
                return a
            out = a @ n_mat.T
            ln = np.linalg.norm(out, axis=1, keepdims=True)
            return out / np.where(ln > 1e-12, ln, 1.0)

        # The prototype's tones were baked from ITS normals against the fixed
        # world light: a rotated copy re-shades them for the faces as placed
        # (#482), with the bake's own law (``_shade_factor``) per triangle.
        light = np.array([self._LIGHT.x(), self._LIGHT.y(), self._LIGHT.z()])

        # Only a turn (or a mirror, or an uneven scale) changes the light; a
        # copy that is just moved or evenly scaled keeps the bake as it is.
        diag = np.diag(L)
        turned = not (np.allclose(L, np.diag(diag), atol=1e-9)
                      and diag[0] > 0.0 and np.allclose(diag, diag[0]))

        def matte(tri, quantise=False):  # (n, 3, 3) corners → shade (n,)
            n = np.cross(tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0])
            ln = np.linalg.norm(n, axis=1)
            d = np.abs(n @ light) / np.where(ln > 1e-9, ln, 1.0)
            f = 0.62 + 0.38 * d
            if quantise:                 # a texture run's key, as baked
                f = np.round(f * 32.0) / 32.0
            return np.where(ln > 1e-9, f, 0.84375)

        def tri_pos(raw, stride):
            a = np.frombuffer(raw, np.float32).reshape(-1, 3, stride)
            return a[:, :, :3].astype(np.float64)

        def reshade6(raw, placed, normals=None):  # local → placed
            if (not turned or not raw or len(raw) % 72
                    or len(placed) != len(raw)):
                return placed
            if normals:
                local = np.frombuffer(normals, np.float32).reshape(-1, 3)
                local = local.astype(np.float64)
                world = tn(local)
                k = ((0.62 + 0.38 * np.abs(world @ light))
                     / (0.62 + 0.38 * np.abs(local @ light)))
                if mirrored:
                    k = k.reshape(-1, 3)[:, [0, 2, 1]].reshape(-1)
                a = np.frombuffer(placed, np.float32).reshape(-1, 6).copy()
                a[:, 3:] = np.minimum(a[:, 3:] * k[:, None], 1.0)
                return a.tobytes()
            k = (matte(tri_pos(placed, 6), quantise=True)
                 / matte(tri_pos(raw, 6), quantise=True))
            a = np.frombuffer(placed, np.float32).reshape(-1, 3, 6).copy()
            rgb = a[:, :, 3:6].astype(np.float64) * k[:, None, None]
            a[:, :, 3:6] = np.round(np.minimum(rgb, 1.0) * 64.0) / 64.0
            return a.tobytes()

        def with_shade(key, s):
            # The shade sits in the (path, shade) pair, at the front of
            # the key or as its first element.
            if isinstance(key[0], str):
                return (key[0], s) + tuple(key[2:])
            return ((key[0][0], s),) + tuple(key[1:])

        def rekey5(placed):             # {key: pos(3)+uv(2)} by new shade
            if not turned:
                return placed
            out: dict = {}
            for key, raw in placed.items():
                if not raw or len(raw) % 60:
                    out[key] = out.get(key, b"") + raw
                    continue
                f = matte(tri_pos(raw, 5), quantise=True)
                a = np.frombuffer(raw, np.float32).reshape(-1, 15)
                for s in np.unique(f):
                    nk = with_shade(key, float(s))
                    out[nk] = out.get(nk, b"") + a[f == s].tobytes()
            return out

        def soft_pts():
            sp = base["soft_pts"]
            if sp is not None:
                s6 = sp.reshape(-1, 6).astype(np.float64)
                s6[:, 0:3] = tp(s6[:, 0:3])
                s6[:, 3:6] = tp(s6[:, 3:6])
                sp = s6.astype(np.float32)
            return sp

        # What the pick index reads is made now; what only drawing reads
        # waits until something draws it (see _LazyChunk).
        entry = _LazyChunk({
            "ikey": key,
            "uid": next(_chunk_uid),
            "rev": (cur["rev"] + 1) if cur is not None else 0,
            "nv": base["nv"], "ne": base["ne"], "nf": base["nf"],
            "edges": tp32(base["edges"], 3, tris=False),
            "faces": base["faces"],
            "areas": base["areas"] * (abs(det) ** (2.0 / 3.0)),
            "v0": tp(base["v0"]) if base["v0"] is not None else None,
            "e1": tv(base["e1"]) if base["e1"] is not None else None,
            "e2": tv(base["e2"]) if base["e2"] is not None else None,
            "tri_ent": base["tri_ent"],
            "soft_single": base["soft_single"],
        }, {
            "dback": lambda: tp32(base.get("dback", b""), 3),
            "vcol": lambda: reshade6(base["vcol"], tp32(base["vcol"], 6),
                                    base.get("smooth_vcol")),
            "by_texture": lambda: rekey5(
                {p: tp32(raw, 5) for p, raw in base["by_texture"].items()}),
            "tcol": lambda: {a: reshade6(raw, tp32(raw, 6))
                             for a, raw in base.get("tcol", {}).items()},
            "ttex": lambda: rekey5({k: tp32(raw, 5) for k, raw
                                    in base.get("ttex", {}).items()}),
            "back_vcol": lambda: reshade6(
                base.get("back_vcol", b""),
                tp32(base.get("back_vcol", b""), 6)),
            "back_tex": lambda: rekey5({k: tp32(raw, 5) for k, raw
                                        in base.get("back_tex", {}).items()}),
            "back_tcol": lambda: {
                k: reshade6(raw, tp32(raw, 6))
                for k, raw in base.get("back_tcol", {}).items()},
            "back_ttex": lambda: rekey5({k: tp32(raw, 5) for k, raw in
                                         base.get("back_ttex", {}).items()}),
            "fvcol": lambda: reshade6(base.get("fvcol", b""),
                                      tp32(base.get("fvcol", b""), 6)),
            "soft_pts": soft_pts,
            "soft_n0": lambda: tn(base["soft_n0"]),
            "soft_c0": lambda: (tp(base["soft_c0"]) if len(base["soft_c0"])
                                else base["soft_c0"]),
            "soft_n1": lambda: tn(base["soft_n1"]),
            "soft_c1": lambda: (tp(base["soft_c1"]) if len(base["soft_c1"])
                                else base["soft_c1"]),
        })
        bb = base.get("bbox")
        if bb is not None:
            corners = np.array([(x, y, z)
                                for x in (bb[0][0], bb[1][0])
                                for y in (bb[0][1], bb[1][1])
                                for z in (bb[0][2], bb[1][2])])
            tc = tp(corners)
            entry["bbox"] = (tuple(float(v) for v in tc.min(axis=0)),
                             tuple(float(v) for v in tc.max(axis=0)))
        else:
            entry["bbox"] = None
        cache[id(group)] = entry
        return entry

    def _shift_instance_entry(self, entry, d: QVector3D) -> None:
        """Translate a cached instance chunk in place (Move drag fast path)."""
        import numpy as np
        dx = np.array([d.x(), d.y(), d.z()])
        self._shift_obb(entry, d)
        bb = entry.get("bbox")
        if bb is not None:
            entry["bbox"] = (
                (bb[0][0] + d.x(), bb[0][1] + d.y(), bb[0][2] + d.z()),
                (bb[1][0] + d.x(), bb[1][1] + d.y(), bb[1][2] + d.z()))

        def shift(raw, stride):
            a = np.frombuffer(raw, np.float32).reshape(-1, stride).copy()
            a[:, :3] += dx
            return a.tobytes()

        entry["edges"] = shift(entry["edges"], 3)
        if entry.get("dback"):
            # The back-tint triangles ride along too — left behind, they
            # drew a white ghost of the arch where it stood before the
            # move (Marco, 2026-09-11, «roté el componente, moví y quedó
            # eso fantasma»).
            entry["dback"] = shift(entry["dback"], 3)
        entry["vcol"] = shift(entry["vcol"], 6)
        entry["by_texture"] = {p: shift(raw, 5)
                               for p, raw in entry["by_texture"].items()}
        entry["tcol"] = {a: shift(raw, 6)
                         for a, raw in entry.get("tcol", {}).items()}
        entry["ttex"] = {k: shift(raw, 5)
                         for k, raw in entry.get("ttex", {}).items()}
        if entry.get("back_vcol"):
            entry["back_vcol"] = shift(entry["back_vcol"], 6)
        entry["back_tex"] = {k: shift(raw, 5)
                             for k, raw in entry.get("back_tex", {}).items()}
        entry["back_tcol"] = {k: shift(raw, 6)
                              for k, raw in entry.get("back_tcol", {}).items()}
        entry["back_ttex"] = {k: shift(raw, 5)
                              for k, raw in entry.get("back_ttex", {}).items()}
        if entry.get("fvcol"):
            entry["fvcol"] = shift(entry["fvcol"], 6)
        if entry["v0"] is not None:
            entry["v0"] = entry["v0"] + dx
        if entry["soft_pts"] is not None:
            s6 = entry["soft_pts"].reshape(-1, 6).copy()
            s6[:, 0:3] += dx
            s6[:, 3:6] += dx
            entry["soft_pts"] = s6.astype(np.float32)
        for kk in ("soft_c0", "soft_c1"):
            if len(entry[kk]):
                entry[kk] = entry[kk] + dx

    def _group_chunk(self, group):
        """Cached render + pick payload of one group, keyed by its content
        fingerprint. A big imported reference model (17k faces) made EVERY
        stroke beside it pay a full VBO + pick-index rebuild (~1.5 s); with
        the chunk, untouched groups just re-concatenate, and a pure
        translation (Move drag) shifts the arrays instead of rebuilding.
        Component instances resolve to their prototype's chunk transformed
        by the instance matrix."""
        if getattr(group, "xform", None) is not None:
            return self._instance_chunk(group)
        cache = getattr(self, "_group_chunks", None)
        if cache is None:
            cache = self._group_chunks = {}
        entry = cache.get(id(group))
        vkey = (self.scene.version, id(self.scene.mesh))
        mesh = group.mesh
        # The container's paint (issue #47) changes what default faces
        # bake to without touching the mesh, so no serial or sample can
        # notice it: it is part of the entry's identity instead.
        paint = _paint_of(self, group)
        msig = _material_sig(paint)
        if entry is not None and entry.get("msig") != msig:
            entry = None
        # The entry answers for ONE mesh object. Explode hands every lifted
        # child a NEW mesh (moved by the container's matrix), and a copy's
        # mesh built the same way has the same mutation serial and counts:
        # the O(1) check below took the new mesh for the old one and drew
        # the group where it stood before it was grouped and moved, while
        # its selection box showed where it is (issue #134, @fafecm).
        if entry is not None and entry.get("mesh") is not mesh:
            entry = None
        # Copies painted differently bake one mesh separately, and the dirty
        # flags are the mesh's: the first bake to read them clears them. The
        # others learn from the generation that they have to look (#1129:
        # a face painted inside a component never reached the copy that had
        # been baked in a paint of its own).
        behind = (entry is not None
                  and entry.get("flags_gen", 0) != getattr(mesh, "_flags_gen", 0))
        array_bake = _array_bake()
        if array_bake.can_bake(mesh):
            # A lazy mesh (experimental engine) is baked from its arrays,
            # and stays unbuilt; while it is, it cannot have changed.
            if entry is None:
                entry = array_bake.bake(self, mesh, paint, msig, vkey,
                                        next(_chunk_uid))
                if msig is not None:
                    entry["fp"] = entry["fp"] + (("paint",) + msig,)
                cache[id(group)] = entry
            entry["vkey"] = vkey
            return entry
        # Reversed faces show in nothing the checks below read, and the dirty
        # marks that say so are cleared by the first chunk rebuilt: a painted
        # copy, with a chunk of its own, then took itself for current.
        if (entry is not None and _fp_winding(entry["fp"])
                != getattr(mesh, "_winding_rev", 0)):
            entry = None
        if entry is not None:
            if entry.get("vkey") == vkey:
                return entry
            # Untouched since it was built or last accepted: no mutation
            # primitive bumped the mesh's serial and the counts agree — an
            # O(1) yes. A scene-version bump (any edit anywhere) used to send
            # EVERY placement's chunk through the probe and the samples on
            # the next paint: ~3300 validations, 15 ms per edit on the plaza
            # (2026-09-14).
            if (entry.get("serial") is not None
                    and entry["serial"] == getattr(mesh, "_mut_serial", None)
                    and not behind
                    and not getattr(mesh, "_attrs_dirty", False)
                    and len(mesh.vertices) == entry["nv"]
                    and len(mesh.edges) == entry["ne"]
                    and len(mesh.faces) == entry["nf"]):
                entry["vkey"] = vkey
                return entry
            # The probe judges GEOMETRY only, and _shift_chunk reuses the
            # cached texture buckets as they are — so a repaint must not be
            # allowed to ride along inside a translation.
            if not behind and not getattr(mesh, "_attrs_dirty", False):
                d = self._translation_probe(entry, mesh)
                if d is not None:
                    self._shift_chunk(entry, d, mesh)
                    entry["vkey"] = vkey
                    entry["rev"] += 1
                    entry["serial"] = getattr(mesh, "_mut_serial", None)
                    return entry
            # Clean fast path: no mutation primitive touched this mesh since
            # the last validation (O(1) dirty flag) and the samples agree —
            # skip the 100 ms fingerprint walk that used to run on EVERY
            # scene change (each stroke/drag frame beside a big import).
            if (not behind
                    and not getattr(mesh, "_chunk_dirty", True)
                    and len(mesh.vertices) == entry["nv"]
                    and len(mesh.edges) == entry["ne"]
                    and len(mesh.faces) == entry["nf"]
                    and self._samples_match(entry, mesh)):
                entry["vkey"] = vkey
                entry["serial"] = getattr(mesh, "_mut_serial", None)
                return entry
        fp = self._group_fp(group)
        if msig is not None:
            fp = fp + (("paint",) + msig,)   # past [:3]/[4:]: the disk key too
        if entry is not None:
            # The fingerprint's checksum is a SUM of coordinates, and a turn
            # about the centroid keeps the sum: Move's rotation grips turn a
            # group about its box centre every time, and the viewport went
            # on drawing the group where it was while its box showed where
            # it is (Marco, 25-09, «imagen fantasma»). The sampled vertices
            # settle it — up to 32 lookups.
            same = entry["fp"] == fp and self._samples_match(entry, mesh)
            if not same and entry.get("fp_approx"):
                # Post-shift: the checksum is approximate (float32 drift).
                # Counts/attrs/soft equal + every sampled vertex in place is
                # the real test; heal the stored fingerprint on acceptance.
                same = (entry["fp"][:3] == fp[:3]
                        and entry["fp"][4:] == fp[4:]
                        and self._samples_match(entry, group.mesh))
            if same:
                entry["fp"] = fp
                entry["fp_approx"] = False
                entry["vkey"] = vkey
                entry["serial"] = getattr(mesh, "_mut_serial", None)
                _flags_read(mesh, entry)
                return entry
        _c0 = _time_mod.perf_counter() if _PERF else 0.0
        # P4: before the expensive rebuild, try the on-disk chunk cache —
        # the arrays are deterministic from the mesh content, so a cold
        # open loads the 230k-face hedge in ~0.3 s instead of building it
        # for ~6 s. Keyed by a STABLE content digest (the in-session
        # fingerprint uses process-salted hash()).
        _loader = getattr(self, "_chunk_cache_load", None)   # stub VPs in tests
        disk = _loader(group, fp, vkey) if callable(_loader) else None
        if disk is not None and not self._samples_match(disk, mesh):
            disk = None      # same digest, turned geometry (see above)
        if disk is not None:
            cache[id(group)] = disk
            disk["msig"] = msig
            disk["mesh"] = mesh
            disk["serial"] = getattr(mesh, "_mut_serial", None)
            _flags_read(mesh, disk)
            if _PERF:
                _plog("chunk_from_disk",
                      (_time_mod.perf_counter() - _c0) * 1000.0,
                      extra=f"faces={disk['nf']}", floor=0.0)
            return disk
        import numpy as np
        mesh = group.mesh
        edges_data = array("f")
        # Soft-edge silhouette source data: per-frame the view test runs
        # vectorised over these (99k soft edges walked in Python per frame
        # made orbiting a full imported project a 4-second slide show).
        soft_pts: list = []
        soft_n0: list = []
        soft_c0: list = []
        soft_n1: list = []
        soft_c1: list = []
        soft_single: list = []
        # The layer each drawn edge carries of its own (None: its
        # object's), in the order of ``edges`` and of ``soft_pts``.
        edge_layer: list = []
        soft_layer: list = []
        fprops: dict = {}

        # Faces first: one Newell walk per face yields normal + area, and the
        # normal feeds triangulate(), the shaded colour AND the soft-edge
        # props below (the naive per-call chain re-ran Newell ~3.6× per face —
        # a third of the whole build on a 160k-face import).
        from core.triangulate import triangulate as _triangulate
        vcol = array("f")             # interleaved pos(3)+rgb(3) per vertex
        smooth_vcol = array("f")      # optional corner normals for instances
        by_texture: dict = {}
        tcol: dict = {}               # opacity -> interleaved pos+rgb (translucent)
        by_ttexture: dict = {}        # (op, fcull) -> {path: pos+uv}
        back_vcol_parts: list = []    # back-side colour overrides
        back_tex: dict = {}           # (path, shade) -> [byte parts]
        back_tcol: dict = {}          # opacity -> [parts] (translucent back)
        back_ttex: dict = {}          # ((path, shade), op) -> [parts]
        fcull_vcol_parts: list = []   # front copies culled to the front side
        dback_faces: list = []        # face index → its back is the default
        faces: list = []
        areas: list = []
        tris = array("d")             # 9 doubles per triangle
        tri_ent: list = []
        from core.materials import back_is_default
        # Per-face geometry memo (normal, area, triangles as floats, centroid)
        # keyed on the face's exact vertex positions: an edit inside a big
        # group touches a few faces, and every other face's Newell walk and
        # triangulation was redone on each edit — ~1 s per stroke inside the
        # Plaza's 25 797-face group (09-10). Reused only when every corner is
        # where it was, so the chunk is the one a full rebuild makes.
        editing_now = getattr(self, "_editing_meshes", None)
        if callable(editing_now) and id(mesh) not in editing_now():
            memo = None             # see below: only the edited mesh keeps one
            built: dict = {}        # this build's geometry, never kept
        else:
            memo = mesh.__dict__.get("_face_geom_memo")
            if memo is None or len(memo) > 2 * len(mesh.faces) + 64:
                memo = mesh.__dict__["_face_geom_memo"] = {}
            built = memo
        from formats.meshexport import SmoothNormals
        curved = _curved_faces(mesh)
        smoother = SmoothNormals()
        for f in mesh.faces:
            if f.attrs.get("hidden"):
                # Hide on a face: out of the chunk entirely — not
                # drawn, not picked, not snapped. Its edges stay (below).
                continue
            fattrs = effective_attrs(f.attrs, paint)
            i = len(faces)
            faces.append(f)
            geom = _face_geometry(f, memo, _triangulate)
            if memo is None:
                built[element_identity(f)] = geom     # this build's, for the soft edges
            normal, area, tri_list = geom[1], geom[2], geom[3]
            areas.append(area)
            fprops[element_identity(f)] = normal
            back = fattrs.get("back")
            fcull = 0
            if isinstance(back, dict):
                kind, payload = self._bucket_back_face(f, back)
                if kind == "bvcol":
                    back_vcol_parts.append(payload)
                elif kind == "btex":
                    back_tex.setdefault(payload[0], []).append(payload[1])
                elif kind == "btcol":
                    back_tcol.setdefault(payload[0], []).append(payload[1])
                elif kind == "bttex":
                    back_ttex.setdefault(payload[0], []).append(payload[1])
                if kind in ("btcol", "bttex"):
                    fcull = 1
            tex = fattrs.get("texture")
            op = float(fattrs.get("opacity", 1.0))
            if tex is not None and tex.get("path"):
                if op < 0.999:
                    self._append_textured_face(
                        by_ttexture.setdefault((round(op, 3), fcull), {}),
                        f, tex)
                else:
                    self._append_textured_face(by_texture, f, tex)
            else:
                col = fattrs.get("color")
                base = tuple(col) if col is not None else self.DEFAULT_FACE_COLOR
                shade = geom[5]
                rgb = shade.get(base)
                if rgb is None:
                    rgb = shade[base] = self._shaded_color(base, normal)
                r, g, b = rgb
                if op < 0.999:
                    dest = tcol.setdefault((round(op, 3), fcull), array("f"))
                elif fcull:
                    dest = None
                else:
                    dest = vcol
                if f in curved:
                    rows = _smooth_vcol(f, base, rgb, smoother, tri_list,
                                        self._LIGHT)
                else:
                    rows = [x for t0, t1, t2 in tri_list
                            for x in (*t0, r, g, b, *t1, r, g, b,
                                      *t2, r, g, b)]
                if dest is None:
                    fcull_vcol_parts.append(array("f", rows).tobytes())
                else:
                    dest.extend(rows)
                    if dest is vcol and curved:
                        cn = smoother.corners(f) if f in curved else {}
                        smooth_vcol.extend(c for tri in tri_list for p in tri
                                           for c in cn.get(p, normal).toTuple())
            tris.extend(geom[6])
            tri_ent.extend([i] * len(tri_list))
            if tri_list and back_is_default(fattrs):
                dback_faces.append(i)

        sprops: dict = {}

        def props(f):
            r = sprops.get(element_identity(f))
            if r is None:
                hit = built.get(element_identity(f))
                if hit is not None and hit[7] is not None:
                    r = sprops[element_identity(f)] = hit[7]
                    return r
                n = fprops.get(element_identity(f))
                if n is None:
                    n = f.normal()
                c = f.centroid()
                r = sprops[element_identity(f)] = ((n.x(), n.y(), n.z()),
                                     (c.x(), c.y(), c.z()))
            return r

        for e in mesh.edges:
            if getattr(e, "hidden", False):
                continue                  # invisible: no line, no profile
            if not getattr(e, "soft", False):
                edges_data.extend([e.a.x(), e.a.y(), e.a.z(),
                                   e.b.x(), e.b.y(), e.b.z()])
                edge_layer.append(getattr(e, "layer", None))
                continue
            fs = e.faces
            if len(fs) not in (1, 2):
                continue                  # dangling / non-manifold: not drawn
            soft_pts.append([e.a.x(), e.a.y(), e.a.z(),
                             e.b.x(), e.b.y(), e.b.z()])
            soft_layer.append(getattr(e, "layer", None))
            n0, c0 = props(fs[0])
            soft_n0.append(n0)
            soft_c0.append(c0)
            if len(fs) == 2:
                n1, c1 = props(fs[1])
                soft_single.append(False)
            else:
                n1, c1 = n0, c0
                soft_single.append(True)
            soft_n1.append(n1)
            soft_c1.append(c1)
        if tris:
            t = np.frombuffer(tris.tobytes(), dtype=np.float64).reshape(-1, 3, 3)
            v0, e1, e2 = t[:, 0], t[:, 1] - t[:, 0], t[:, 2] - t[:, 0]
            tri_ent_a = np.asarray(tri_ent, dtype=np.int64)
            # The default-back tint triangles, cut from the pick triangles
            # already assembled instead of re-walked per face in Python
            # (that walk cost the chunk build a fifth on the plaza).
            if dback_faces:
                keep = np.zeros(len(faces), dtype=bool)
                keep[dback_faces] = True
                dback_raw = t[keep[tri_ent_a]].astype(np.float32).tobytes()
            else:
                dback_raw = b""
        else:
            v0 = e1 = e2 = tri_ent_a = None
            dback_raw = b""
        # World AABB of the chunk (frustum culling): triangle corners cover
        # every face; hard-edge endpoints cover edge-only content.
        blo = bhi = None
        if v0 is not None:
            allv = np.concatenate((v0, v0 + e1, v0 + e2))
            blo, bhi = allv.min(axis=0), allv.max(axis=0)
        if len(edges_data):
            ea = np.frombuffer(edges_data.tobytes(),
                               dtype=np.float32).reshape(-1, 3)
            elo, ehi = ea.min(axis=0), ea.max(axis=0)
            blo = elo if blo is None else np.minimum(blo, elo)
            bhi = ehi if bhi is None else np.maximum(bhi, ehi)
        bbox = (((float(blo[0]), float(blo[1]), float(blo[2])),
                 (float(bhi[0]), float(bhi[1]), float(bhi[2])))
                if blo is not None else None)
        verts = mesh.vertices
        nv = len(verts)
        coordsum = 0.0
        for v in verts:
            p = v.position
            coordsum += p.x() + p.y() * 1.000003 + p.z() * 1.000007
        idxs = sorted({k * max(nv - 1, 0) // 31 for k in range(32)}) if nv else []
        samples = [(i, (verts[i].position.x(), verts[i].position.y(),
                        verts[i].position.z())) for i in idxs]
        entry = {"fp": fp, "vkey": vkey, "rev": 0, "uid": next(_chunk_uid),
                 "msig": msig,
                 "serial": getattr(mesh, "_mut_serial", None),
                 "nv": nv, "ne": len(mesh.edges), "nf": len(mesh.faces),
                 "samples": samples, "coordsum": coordsum, "bbox": bbox,
                 # Lazily filled by ``_group_obb``: the box in the group's own
                 # axes, which is the one the selection cue draws.
                 "obb": None,
                 "edges": edges_data.tobytes(),
                 "vcol": vcol.tobytes(),
                 "smooth_vcol": smooth_vcol.tobytes(),
                 "by_texture": {k: v.tobytes() for k, v in by_texture.items()},
                 "tcol": {a: v.tobytes() for a, v in tcol.items()},
                 "ttex": {(pth, a, fc): v.tobytes()
                          for (a, fc), d in by_ttexture.items()
                          for pth, v in d.items()},
                 "back_vcol": b"".join(back_vcol_parts),
                 "back_tex": {k: b"".join(v) for k, v in back_tex.items()},
                 "back_tcol": {k: b"".join(v) for k, v in back_tcol.items()},
                 "back_ttex": {k: b"".join(v) for k, v in back_ttex.items()},
                 "fvcol": b"".join(fcull_vcol_parts),
                 "dback": dback_raw,
                 "faces": faces,
                 "areas": np.asarray(areas, dtype=np.float64),
                 "v0": v0, "e1": e1, "e2": e2, "tri_ent": tri_ent_a,
                      "soft_pts": (np.asarray(soft_pts, dtype=np.float32)
                              if soft_pts else None),
                 "soft_n0": np.asarray(soft_n0, dtype=np.float64),
                 "soft_c0": np.asarray(soft_c0, dtype=np.float64),
                 "soft_n1": np.asarray(soft_n1, dtype=np.float64),
                 "soft_c1": np.asarray(soft_c1, dtype=np.float64),
                 "soft_single": np.asarray(soft_single, dtype=bool),
                 "edge_layer": edge_layer, "soft_layer": soft_layer}
        if _PERF:
            _plog("chunk_rebuild", (_time_mod.perf_counter() - _c0) * 1000.0,
                  extra=f"faces={len(faces)}")
        _flags_read(mesh, entry)
        entry["mesh"] = mesh
        cache[id(group)] = entry
        _store = getattr(self, "_chunk_cache_store", None)   # stub VPs in tests
        editing = getattr(self, "_editing_meshes", None)
        if not (callable(editing) and id(mesh) in editing()):
            # The per-face memo only pays off for the mesh being edited
            # (each stroke rebuilds its chunk); kept for every mesh it cost
            # +140 MB on the Plaza (release_check 10-10). Entering a group
            # rebuilds it on the first stroke, as before the memo existed.
            mesh.__dict__.pop("_face_geom_memo", None)
        # Not while the group is open: each stroke inside it rebuilds this
        # chunk, and pickling 25 000 faces after every one held the GIL
        # ~350 ms beside the UI (Plaza, 09-10) for an entry the next stroke
        # made stale. The cold build on opening still lands on disk.
        if callable(_store) and not (callable(editing)
                                     and id(mesh) in editing()):
            _store(mesh, entry)
        return entry

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
        """Stable content digest: the fingerprint's counts/coordsum/soft
        plus a per-face attrs walk covering everything the chunk bakes
        (colour, texture path/uvw/scale/rot, opacity, back side). The
        session fingerprint's attrs term uses salted hash() — useless
        across runs."""
        import hashlib
        h = hashlib.sha1()
        h.update(repr((fp[0], fp[1], fp[2], fp[3], fp[5])).encode())
        # The winding (#197): the same mesh with a face reversed must never
        # find the chunk stored before. Read from the loops, not from the
        # session's flip count: a saved file reopens with none, and two
        # sessions with one flip each may have reversed different faces. A
        # ring's first two corners are enough, reversing one changes them.
        corners = array("d")
        for f in mesh.faces:
            for v in f.loop[:2]:
                p = v.position
                corners.extend((p.x(), p.y(), p.z()))
        h.update(corners.tobytes())
        if fp[5]:
            # Soft edges shade smooth since #131: a chunk stored before
            # bakes the curved surface flat. Only meshes with soft edges
            # take the term, so the rest keep their digests.
            h.update(b"smooth-shading-angle-weighted-v3")
        if fp[6]:
            # Hidden-edge term, only when some edge IS hidden: meshes without
            # any (the overwhelming majority) keep their pre-existing digests,
            # so adding the term didn't cold-invalidate the whole cache.
            h.update(repr(("ehidden", fp[6])).encode())
        for i, f in enumerate(mesh.faces):
            a = f.attrs
            if not a:
                continue
            t = a.get("texture")
            h.update(repr((
                i, a.get("color") and tuple(a["color"]),
                None if not t else (t.get("path"), t.get("sw"), t.get("sh"),
                                    t.get("rot", 0),
                                    tuple(t.get("uvw") or ())),
                a.get("opacity"), repr(a.get("back")) if a.get("back")
                else None, a.get("layer"))).encode())
            if a.get("hidden"):
                # Only when hidden, so meshes without one keep their
                # pre-existing digests (same reason as the edge term).
                h.update(repr(("fhidden", i)).encode())
        return h.hexdigest()

    _CHUNK_CACHE_FIELDS = (
        "edges", "vcol", "smooth_vcol", "by_texture", "tcol", "ttex", "back_vcol",
        "back_tex", "back_tcol", "back_ttex", "fvcol", "dback", "areas",
        "v0", "e1", "e2", "tri_ent", "soft_pts", "soft_n0", "soft_c0",
        "soft_n1", "soft_c1", "soft_single", "edge_layer", "soft_layer",
        "bbox", "coordsum", "nv", "ne", "nf", "samples")

    def _chunk_cache_load(self, group, fp, vkey):
        mesh = group.mesh
        if len(mesh.faces) < self._CHUNK_CACHE_MIN_FACES:
            return None
        import pickle
        try:
            path = self._chunk_cache_dir() / \
                (self._chunk_cache_key(mesh, fp) + ".chunk")
            if not path.is_file():
                return None
            with open(path, "rb") as fh:
                stored = pickle.load(fh)     # our own cache dir only
            if (stored.get("nf") != len(mesh.faces) or "dback" not in stored
                    or "edge_layer" not in stored):
                return None    # a cache predating the back-side tint or
                #                the edges' layers
            entry = dict(stored)
            entry.update(fp=fp, vkey=vkey, rev=0, uid=next(_chunk_uid),
                         faces=list(mesh.faces))
            path.touch()                     # LRU freshness
            return entry
        except Exception:                    # noqa: BLE001 — a cache can only miss
            return None

    def _chunk_cache_store(self, mesh, entry) -> None:
        if entry["nf"] < self._CHUNK_CACHE_MIN_FACES:
            return
        import pickle
        import threading
        try:
            key = self._chunk_cache_key(mesh, entry["fp"])
            payload = {k: entry.get(k, b"") if k == "smooth_vcol" else entry[k]
                       for k in self._CHUNK_CACHE_FIELDS}
            cdir = self._chunk_cache_dir()
        except Exception:                    # noqa: BLE001
            return

        def write():
            try:
                path = cdir / (key + ".chunk")
                tmp = path.with_suffix(".part")
                with open(tmp, "wb") as fh:
                    pickle.dump(payload, fh, protocol=5)
                tmp.replace(path)
                files = sorted(cdir.glob("*.chunk"),
                               key=lambda p: p.stat().st_mtime)
                for old in files[:-self._CHUNK_CACHE_KEEP]:
                    old.unlink(missing_ok=True)
            except Exception:                # noqa: BLE001
                pass

        threading.Thread(target=write, daemon=True).start()

    def _np_mvp(self):
        """Current MVP as a (4, 4) float64 NumPy matrix (row-major indexing).
        ``QMatrix4x4.data()`` is column-major, hence the Fortran reshape."""
        import numpy as np
        m = self.camera.projection_matrix() * self.camera.view_matrix()
        return np.array(m.data(), dtype=np.float64).reshape(4, 4, order="F")

    def _camera_px(self):
        """``(mvp, width, height)``: what the native pick kernels project
        with (``mvp`` column-major, as ``Matrix4.data()``)."""
        m = self.camera.projection_matrix() * self.camera.view_matrix()
        return list(m.data()), float(self.width()), float(self.height())

    def _project_px(self, pts):
        """Batch world points (N, 3) → ``(px, py, in_front)`` arrays — the
        exact math of :meth:`_world_to_pixel` (``ingetrazo_core.project_px``)."""
        import numpy as np
        from ingetrazo_core import project_px
        return project_px(np.ascontiguousarray(pts, dtype=np.float64).reshape(-1, 3),
                          list(self._np_mvp().reshape(-1, order="F")),
                          float(self.width()), float(self.height()))

    @staticmethod
    def _frustum_planes(mvp):
        """The six view-frustum planes of ``mvp`` as (nx, ny, nz, d) tuples
        (Gribb–Hartmann rows); a point is inside when n·p + d ≥ 0 for all
        six. Plain floats — the per-frame consumer tests ~tens of chunk
        boxes, where NumPy overhead would dominate."""
        m = mvp.data()                    # column-major
        rows = [(m[i], m[i + 4], m[i + 8], m[i + 12]) for i in range(4)]
        r3 = rows[3]
        planes = []
        for row in rows[:3]:
            planes.append(tuple(r3[i] + row[i] for i in range(4)))
            planes.append(tuple(r3[i] - row[i] for i in range(4)))
        return planes

    @staticmethod
    def _aabb_visible(planes, lo, hi) -> bool:
        """Conservative AABB-vs-frustum p-vertex test: culled only when the
        box lies fully outside one plane (never drops visible geometry)."""
        lx, ly, lz = lo
        hx, hy, hz = hi
        for nx, ny, nz, d in planes:
            px = hx if nx >= 0.0 else lx
            py = hy if ny >= 0.0 else ly
            pz = hz if nz >= 0.0 else lz
            if nx * px + ny * py + nz * pz + d < 0.0:
                return False
        return True

    def _visible_spans(self, spans, planes, split=None):
        """Filter draw spans ``[(bbox, start, count)]`` by the frustum and
        merge adjacent survivors: returns ``([(start, count)], culled)``.
        ``bbox None`` = always drawn (loose geometry, unknown extents).

        ``split`` (a vertex offset) is a boundary merging never crosses, so
        the caller can still tell the two sides apart — the faded context
        before it, the group being edited from it on."""
        if len(spans) >= _SPAN_VECTOR_MIN and planes:
            return self._visible_spans_np(spans, planes, split)
        out: list = []
        culled = 0
        for bbox, start, count in spans:
            if not count:
                continue
            if bbox is not None and not self._aabb_visible(
                    planes, bbox[0], bbox[1]):
                culled += count
                continue
            if (out and out[-1][0] + out[-1][1] == start
                    and not (split is not None
                             and out[-1][0] < split <= start)):
                out[-1] = (out[-1][0], out[-1][1] + count)
            else:
                out.append((start, count))
        return out, culled

    def _visible_spans_np(self, spans, planes, split=None):
        """:meth:`_visible_spans` over many spans at once: the same p-vertex
        test, float op for float op, on arrays kept per span list (a model
        of 21 406 placements tested ~4 800 boxes one by one per frame)."""
        import numpy as np
        memo = getattr(self, "_span_arrays", None)
        if memo is None:
            memo = self._span_arrays = {}
        hit = memo.get(id(spans))
        if hit is None or hit[0] is not spans:
            if len(memo) > 16:
                memo.clear()
            n = len(spans)
            has = np.zeros(n, dtype=bool)
            lo = np.zeros((n, 3))
            hi = np.zeros((n, 3))
            start = np.empty(n, dtype=np.int64)
            count = np.empty(n, dtype=np.int64)
            for i, (bbox, s0, c0) in enumerate(spans):
                start[i] = s0
                count[i] = c0
                if bbox is not None:
                    has[i] = True
                    lo[i], hi[i] = bbox[0], bbox[1]
            hit = memo[id(spans)] = (spans, has, lo, hi, start, count,
                                     *self._bounding_spheres(lo, hi))
        _s, has, lo, hi, start, count, ctr, rad = hit
        vis = np.ones(len(start), dtype=bool)
        for nx, ny, nz, d in planes:
            px = hi[:, 0] if nx >= 0.0 else lo[:, 0]
            py = hi[:, 1] if ny >= 0.0 else lo[:, 1]
            pz = hi[:, 2] if nz >= 0.0 else lo[:, 2]
            vis &= ~(((nx * px + ny * py) + nz * pz) + d < 0.0)
        # What comes out under a pixel is not drawn either, as with the
        # instances (see _instances_below_a_pixel).
        vis &= ~(has & self._spheres_below_a_pixel(ctr, rad))
        live = count > 0
        drawn = live & (vis | ~has)
        culled = int(count[live & ~drawn].sum())
        idx = np.flatnonzero(drawn)
        if not len(idx):
            return [], culled
        st = start[idx]
        ct = count[idx]
        brk = np.ones(len(idx), dtype=bool)
        brk[1:] = st[1:] != st[:-1] + ct[:-1]
        if split is not None:
            # A run never crosses the split (see _visible_spans).
            brk[1:] |= (st[:-1] < split) & (split <= st[1:])
        heads = np.flatnonzero(brk)
        sums = np.add.reduceat(ct, heads)
        return list(zip(st[heads].tolist(), sums.tolist())), culled

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
        """The component placements the pick index takes lazily — only when
        a query comes near them — as ``(groups, lo, hi, nf, ids)``, or
        ``None`` when they are too few to bother (the index then holds
        everything, as it always did).

        Baking every placement to world coordinates for the index was what
        a model of 21 406 placements (14 million faces) could not survive:
        the first hover with Select went past 8 GB (issue #158)."""
        import numpy as np
        key = self._placements_epoch()
        memo = getattr(self, "_pick_lazy_memo", None)
        if memo is not None and memo[0] == key:
            return memo[1]
        if memo is not None and self._epoch_only_matrices(memo[0], key):
            # A Move: picking up changes no row, putting down the rows of
            # what moved. Made again, the table framed 20 500 placements
            # (~0.55 s) on the first pointer move after either.
            table, xfs = memo[1], memo[2]
            if table is not None:
                # The nested placements' proxies take their composed
                # matrices here: the rows are read through them.
                self._placements()
                groups, lo, hi = table[0], table[1], table[2]
                moved = [i for i, g in enumerate(groups)
                         if tuple(g.xform.data()) != xfs[i]]
                if moved:
                    inf = float("inf")
                    lo, hi, xfs = lo.copy(), hi.copy(), list(xfs)
                    for i in moved:
                        box = self._placement_frame(groups[i])[0]
                        lo[i], hi[i] = (box if box
                                        else ((-inf,) * 3, (inf,) * 3))
                        xfs[i] = tuple(groups[i].xform.data())
                    table = (groups, lo, hi) + table[3:]
            self._pick_lazy_memo = (key, table, xfs)
            return table
        groups = [g for g in self._placements()
                  if getattr(g, "xform", None) is not None
                  and not getattr(g, "billboard", False)]
        # An object or two added or taken away (a Delete, a Paste): the
        # rows of the others are the old table's.
        kept = (self._epoch_kept_tops(memo[0], key, ctx=True)
                if memo is not None and memo[1] is not None else None)
        src = None
        if kept:
            o_groups, o_lo, o_hi, o_nf = memo[1][:4]
            row = {id(g): i for i, g in enumerate(o_groups)}
            top_of = self._placement_top
            src = np.fromiter(
                (row.get(id(g), -1) if top_of(g) in kept else -1
                 for g in groups), np.int64, count=len(groups))
        from core.mesh import face_count
        if src is None:
            nf = np.fromiter((face_count(g.mesh) for g in groups), np.int64,
                             count=len(groups))
        else:
            nf = np.fromiter((face_count(g.mesh) if k < 0 else 0
                              for g, k in zip(groups, src.tolist())),
                             np.int64, count=len(groups))
            at = np.flatnonzero(src >= 0)
            nf[at] = o_nf[src[at]]
        table = None
        if int(nf.sum()) >= self._PICK_LAZY_MIN_FACES:
            inf = float("inf")
            lo = np.empty((len(groups), 3))
            hi = np.empty((len(groups), 3))
            if src is None:
                todo = range(len(groups))
            else:
                lo[at], hi[at] = o_lo[src[at]], o_hi[src[at]]
                todo = np.flatnonzero(src < 0).tolist()
            for i in todo:
                box = self._placement_frame(groups[i])[0]
                lo[i], hi[i] = box if box else ((-inf,) * 3, (inf,) * 3)
            table = (groups, lo, hi, nf, {id(g) for g in groups})
        if table is None:
            xfs = None
        elif src is None:
            xfs = [tuple(g.xform.data()) for g in groups]
        else:
            o_xfs = memo[2]
            xfs = [tuple(g.xform.data()) if k < 0 else o_xfs[k]
                   for g, k in zip(groups, src.tolist())]
        self._pick_lazy_memo = (key, table, xfs)
        return table

    def _pick_instance_set(self, table):
        """core._native's InstanceSet over the lazy placements of ``table``
        (see _pick_lazy_table), or ``None`` without the experimental
        engine. A prototype's index (a BVH of its pick triangles, its hard
        edges, from its local bake) is kept while its bake is."""
        if not _INSTANCE_FILTER:
            return None
        from core import _native
        mod = _native.native
        if mod is None or not hasattr(mod, "InstanceSet"):
            return None
        memo = getattr(self, "_inst_set_memo", None)
        if memo is not None and memo[0] is table:
            return memo[1]
        import numpy as np
        _pick_warm_take(self)
        groups = table[0]
        cache = getattr(self, "_inst_proto_cache", None) or {}
        keep: dict = {}
        by_mesh: dict = {}
        protos: list = []
        proto_of = np.empty(len(groups), dtype=np.int64)
        mats = np.empty((len(groups), 16), dtype=np.float64)
        todo: list = []                       # (position in protos, key, bake)
        for i, g in enumerate(groups):
            k = by_mesh.get(id(g.mesh))
            if k is None:
                # Every placement of a mesh has the same triangles and hard
                # edges, whatever its paint: one bake (the first one's).
                base = self._proto_base_chunk(g.mesh, _paint_of(self, g))
                ckey = (base.get("uid"), base.get("rev"))
                ip = cache.get(ckey)
                if ip is None:
                    todo.append((len(protos), ckey, base))
                keep[ckey] = ip
                k = by_mesh[id(g.mesh)] = len(protos)
                protos.append(ip)
            proto_of[i] = k
            mats[i] = g.xform.data()

        def build(job):
            pos, ckey, base = job
            ip = _inst_proto_of(mod, base)
            protos[pos] = ip
            keep[ckey] = ip
        # The engine lets go of the GIL for each tree: side by side, not the
        # 2.4 s of a big model's first hover one after another (#1092).
        if len(todo) > 1:
            import os
            from concurrent.futures import ThreadPoolExecutor
            with ThreadPoolExecutor(
                    max_workers=min(8, os.cpu_count() or 2)) as ex:
                list(ex.map(build, todo))
        else:
            for job in todo:
                build(job)
        self._inst_proto_cache = keep
        iset = mod.InstanceSet(protos, proto_of, mats,
                               np.ascontiguousarray(table[1], np.float64),
                               np.ascontiguousarray(table[2], np.float64))
        self._inst_set_memo = (table, iset)
        return iset

    def start_pick_warmup(self) -> None:
        """A big document is on screen: work out the heavy parts of the pick
        index behind it, so the first move of the pointer does not wait for
        them (4 s on a model of 5 M faces, #1092). Whoever moves the mouse
        needs them, whatever the tool, and the answers cannot change: the
        same calls with the same arrays, only sooner. Only with the engine
        and on a model big enough for lazy placements."""
        if not _PICK_WARM or not _INSTANCE_FILTER:
            return
        from core import _native
        mod = _native.native
        if mod is None or not hasattr(mod, "InstanceSet"):
            return
        table = self._pick_lazy_table()
        if table is None:
            return
        memo = getattr(self, "_inst_set_memo", None)
        if memo is not None and memo[0] is table:
            return                       # the pointer got there first
        seen: set = set()
        protos: list = []
        for g in table[0]:
            if id(g.mesh) not in seen:
                seen.add(id(g.mesh))
                base = self._proto_base_chunk(g.mesh, _paint_of(self, g))
                protos.append(((base.get("uid"), base.get("rev")), base))
        grids: list = []
        if not _NO_SPAN_GRID:
            fixed = DocumentInteraction._pick_candidates(self, False, table)[1]
            for g in fixed:
                if not getattr(g, "billboard", False):
                    c = self._group_chunk(g)
                    if (c["v0"] is not None and len(c["v0"]) > _SPAN_GRID_MIN
                            and "_span_grid" not in c):
                        grids.append(c)
        # What a box selection asks of each, the first time (2 s over 257).
        bounds = [base for _k, base in protos]
        self._pick_warm = _PickWarm(mod, protos, grids, bounds)

    def _static_occluders(self, idx):
        """core._native's InstanceSet over the pick index's own triangles
        that hide what is behind them, placed by the identity (which the
        engine maps bit for bit): the fixed placements' as one prototype,
        kept while those triangles and their visibility are — a stroke
        changes only the loose part, the other prototype. Made once per
        index."""
        import numpy as np
        hit = getattr(idx, "_static_occl", None)
        if hit is not None:
            return hit
        mod = _occlusion_native()
        if mod is None or not hasattr(mod, "InstanceSet"):
            return None
        st = getattr(self, "_pick_static_memo", None)
        fv0 = st[1]["v0"] if st is not None else None
        n = len(idx.tri_v0)
        nf = len(fv0) if fv0 is not None else 0
        if nf > n or (nf and not np.array_equal(idx.tri_v0[n - nf], fv0[0])):
            nf = 0                       # not the index's tail: one part
        vis = idx.ent_vis[idx.tri_ent]

        def proto(lo, hi):
            keep = np.flatnonzero(vis[lo:hi]) + lo
            return mod.InstanceProto(
                np.ascontiguousarray(idx.tri_v0[keep], np.float64),
                np.ascontiguousarray(idx.tri_e1[keep], np.float64),
                np.ascontiguousarray(idx.tri_e2[keep], np.float64),
                np.zeros((0, 6))), keep
        memo = getattr(self, "_static_occl_memo", None)
        fkeep = vis[n - nf:]
        if (nf and memo is not None and memo[0] is fv0
                and np.array_equal(memo[1], fkeep)):
            fixed = memo[2]
        elif nf:
            fixed = proto(n - nf, n)
            self._static_occl_memo = (fv0, fkeep.copy(), fixed)
        else:
            fixed = None
        parts = [proto(0, n - nf)] + ([fixed] if fixed is not None else [])
        iset = mod.InstanceSet([p for p, _k in parts],
                               np.arange(len(parts), dtype=np.int64),
                               np.tile(np.eye(4).reshape(1, 16),
                                       (len(parts), 1)))
        # Prototype p's triangle r is index row rows[start[p] + r].
        rows = np.concatenate([k for _p, k in parts])
        start = np.cumsum([0] + [len(k) for _p, k in parts[:-1]])
        idx._static_occl = (iset, np.arange(len(parts), dtype=np.int64),
                            rows, start)
        return idx._static_occl

    def _static_ray_rows(self, idx, ent_mask, o3, d3):
        """The index rows of the triangles the ray from ``o3`` along ``d3``
        may meet, in order, from the engine's BVH over the visible ones
        (_static_occluders) — or None where that cannot answer for
        ``ent_mask``: no engine, or an entity it passes that is not visible
        (its triangles are not in there). A superset of what the exact test
        accepts, so the hits are the same as over every span the ray's box
        test keeps (hundreds of thousands of triangles of a big fixed part,
        30 ms a hover over the plant of #158)."""
        import numpy as np
        if not (hasattr(self, "_lazy_native")
                and self._lazy_native() is not None):
            return None
        memo = idx.__dict__.setdefault("_mask_in_vis", {})
        ok = memo.get(id(ent_mask))
        if ok is None or ok[0] is not ent_mask:
            ok = memo[id(ent_mask)] = (
                ent_mask, not bool((ent_mask & ~idx.ent_vis).any()))
        if not ok[1]:
            return None
        stat = self._static_occluders(idx)
        if stat is None:
            return None
        iset, cand, rows, start = stat
        pl, tr, _v0, _e1, _e2 = iset.ray_cands(cand, o3, d3, float("inf"))
        return np.sort(rows[start[pl] + tr])

    def _lazy_native(self):
        """What the experimental engine answers for the context's lazy
        placements without baking them (#158): ``(iset, snap, vis)`` —
        core._native's InstanceSet over the lazy table, and the table rows
        of the placements the pick index would take that a snap may land
        on and that hide what is behind them, in the index's order — or
        ``None`` (no lazy table, no engine)."""
        if not _LAZY_NATIVE:
            return None
        table = self._pick_lazy_table()
        if table is None:
            return None
        iset = self._pick_instance_set(table)
        if iset is None or not hasattr(iset, "corners_near"):
            return None
        oculto = getattr(self, "_rest_is_hidden", None)
        oculto = bool(oculto()) if callable(oculto) else False
        ckey = (self._placements_epoch(), id(self.scene.edit_group), oculto,
                id(table))
        cmemo = getattr(self, "_pick_cands_memo", None)
        if cmemo is None or cmemo[0] != ckey:
            self._pick_index(near="skip")         # the candidates it takes
            cmemo = getattr(self, "_pick_cands_memo", None)
            if cmemo is None or cmemo[0] != ckey:
                return None
        memo = getattr(self, "_lazy_native_memo", None)
        if memo is not None and memo[0] is cmemo and memo[1] is iset:
            return memo[2]
        import numpy as np
        scene = self.scene
        row = {id(g): i for i, g in enumerate(table[0])}
        ordered = [g for _p, g in sorted(cmemo[3].values(),
                                         key=lambda pg: pg[0])]
        snap = np.fromiter((row[id(g)] for g in ordered
                            if scene.group_snappable(g)), np.int64)
        vis = np.fromiter((row[id(g)] for g in ordered
                           if scene.entity_visible(g)), np.int64)
        out = (iset, snap, vis)
        self._lazy_native_memo = (cmemo, iset, out)
        return out

    def _pick_materialize(self, near) -> None:
        """Bake into the pick index the lazy placements a query needs:
        ``near`` = ``None`` (around the cursor), ``("px", x, y)`` (around a
        widget pixel), ``("ray", origin, direction)`` or ``"all"``. What was
        baked stays until the cap needs room (least recently needed first);
        ``_pick_gen`` counts the changes, which key the index."""
        import numpy as np
        from collections import OrderedDict
        table = self._pick_lazy_table()
        live = getattr(self, "_pick_live", None)
        if table is None:
            if live:
                live.clear()
                self._pick_gen = getattr(self, "_pick_gen", 0) + 1
            return
        groups, lo, hi, nf, _ids = table
        if live is None:
            live = self._pick_live = OrderedDict()
        if near == "skip":
            return                              # a loose-only query
        px_at = None
        if isinstance(near, tuple) and near[0] == "px":
            px_at = (float(near[1]), float(near[2]))
            # A pick at a pixel only meets what the ray through it crosses:
            # the 32 px footprint is for snaps (``("snap", x, y)``), and on a
            # dense plant it took in thousands of placements per hover.
            o, d = self._pixel_to_ray(float(near[1]), float(near[2]))
            if o is None or d is None:
                return
            near = ("ray", o, d)
        elif isinstance(near, tuple) and near[0] == "snap":
            near = (None, near[1], near[2])
        footprint = False
        if near == "all":
            need = np.arange(len(groups))
        elif isinstance(near, tuple) and near[0] == "ray":
            o, d = near[1], near[2]
            reach = near[3] if len(near) > 3 else None
            o = np.array([o.x(), o.y(), o.z()])
            d = np.array([d.x(), d.y(), d.z()])
            with np.errstate(divide="ignore", invalid="ignore"):
                inv = 1.0 / np.where(np.abs(d) > 1e-12, d, 1e-12)
                t1 = (lo - o) * inv
                t2 = (hi - o) * inv
            tmin = _slab_far(np.minimum(t1, t2))
            tmax = _slab_near(np.maximum(t1, t2))
            hit = tmax >= np.maximum(tmin, 0.0)
            if reach is not None:
                # A bounded query (the walkthrough's wall feelers, half a
                # metre ahead) needs nothing past its reach: an unbounded
                # one baked every placement down the street, and turning
                # the head brought in new ones and rebuilt the index.
                hit &= tmin <= reach
            if px_at is not None and hit.sum() > 1:
                # Whatever starts behind the surface the frame shows at this
                # pixel can be neither picked nor seen.
                seen = self._depth_world_at(*px_at)
                if seen is not None and seen[0] == "hit":
                    t_vis = float(np.linalg.norm(
                        np.array([seen[1].x(), seen[1].y(), seen[1].z()]) - o))
                    hit &= tmin <= t_vis * 1.02 + 0.05
            need = np.flatnonzero(hit)
            iset = self._pick_instance_set(table) if len(need) else None
            if iset is not None:
                # Of the boxes the ray crosses, the placements whose
                # triangles it can meet (the spatial index, with margins
                # past the exact test's): only those are baked.
                need = np.asarray(iset.ray_filter(
                    need.astype(np.int64), tuple(o.tolist()),
                    tuple(d.tolist()), reach), dtype=np.int64)
        else:
            if near is None:
                pos = getattr(self, "_last_mouse_pos", None)
                if pos is None:
                    return
                x, y = float(pos.x()), float(pos.y())
            else:
                x, y = float(near[1]), float(near[2])
            footprint = True
            need = self._footprint_need(table, x, y)
        if footprint and len(need):
            # A footprint takes in the placements nearest the eye that fit
            # in most of the cap, and those stay (see the eviction below):
            # zoomed out over the plant of #158 it met ~3 400 placements,
            # far more than fit, and each pointer move the first ray let go
            # of 300 the footprint then brought in again — two rebuilds of
            # the lazy part a move, for the same set.
            room = self._PICK_LIVE_MAX_FACES * 0.8
            need = np.asarray(need, dtype=np.int64)
            if float(nf[need].sum()) > room:
                e = self.camera.eye()
                e = np.array([e.x(), e.y(), e.z()])
                dist = np.linalg.norm(np.clip(e, lo[need], hi[need]) - e,
                                      axis=1)
                srt = need[np.argsort(dist, kind="stable")]
                fit = np.cumsum(nf[srt]) <= room
                fit[0] = True
                need = srt[fit]
            self._pick_foot_keep = {id(groups[int(i)]) for i in need}
        new = [int(i) for i in need if id(groups[int(i)]) not in live]
        # The footprint queries of one input event (the snap, the cursor's
        # own) share the cap: each brought in its own 300 over the same
        # spot, and each change rebuilt the lazy part of the index — five
        # rebuilds a pointer move over the plant of #158.
        serial = getattr(self, "_input_serial", None)
        cap = self._PICK_NEW_PER_QUERY
        budget = None
        if footprint and serial is not None:
            budget = getattr(self, "_pick_budget", None)
            if budget is None or budget[0] != serial:
                budget = self._pick_budget = [serial, cap]
            cap = budget[1]
        if len(new) > cap and near != "all":
            # A first snap over a dense spot met thousands at once (2.3 s):
            # the nearest to the eye come in now, the rest on the next moves.
            e = self.camera.eye()
            e = np.array([e.x(), e.y(), e.z()])
            nn = np.array(new)
            dist = np.linalg.norm(np.clip(e, lo[nn], hi[nn]) - e, axis=1)
            keep_new = set(nn[np.argsort(dist)[:cap]].tolist())
            need = [int(i) for i in need
                    if id(groups[int(i)]) in live or int(i) in keep_new]
        if budget is not None:
            budget[1] = max(0, cap - min(len(new), cap))
        changed = False
        for i in need:
            g = groups[int(i)]
            k = id(g)
            if k in live:
                live.move_to_end(k)
            else:
                live[k] = (g, int(nf[int(i)]))
                changed = True
        faces = sum(v[1] for v in live.values())
        # Making room once per input event: the ray picks that follow the
        # snap over the same spot let go of what it had just brought in,
        # and every such change rebuilt the index (up to twice the cap).
        evict = faces > self._PICK_LIVE_MAX_FACES and near != "all"
        if (evict and serial is not None
                and getattr(self, "_pick_evicted_at", None) == serial
                and faces <= 2 * self._PICK_LIVE_MAX_FACES):
            evict = False
        if evict:
            self._pick_evicted_at = serial
            keep = {id(groups[int(i)]) for i in need}
            keep |= getattr(self, "_pick_foot_keep", None) or set()
            inst = getattr(self, "_inst_chunks", None)
            for k in list(live):
                if faces <= self._PICK_LIVE_MAX_FACES:
                    break
                if k in keep:
                    continue
                gone, nf_k = live.pop(k)
                faces -= nf_k
                # Its world bake goes with it — unless the static buffers
                # draw it, and then the next sync baked it all over again
                # (13 placements, ~0.15 s of picking an object up on #158).
                if inst is not None and self._instanced_eligible(gone):
                    inst.pop(k, None)
                changed = True
        if changed:
            self._pick_gen = getattr(self, "_pick_gen", 0) + 1

    def _depth_limits(self, x: float, y: float, r: float):
        """What the last frame shows within ``r`` px of widget pixel
        ``(x, y)``: per device pixel of that patch, the distance from the
        eye past which a point there is hidden by something drawn — the
        farthest surface drawn at the pixel and the 8 around it, plus what
        the depth buffer cannot tell apart (8 steps of a 24-bit depth),
        0.2 % and 2 cm; inf where it cannot say (sky at the pixel, the edge
        of the frame). ``(eye, limits (gh, gw), x0, y0, dpr, device height)``
        as core.instances.DepthLimits reads it, or None without a frame."""
        import numpy as np
        if getattr(self, "_depth_snap", None) is None:
            return None
        buf, inv, w, h = (self._depth_snap[0], self._depth_snap[2],
                          self._depth_snap[3], self._depth_snap[4])
        dpr = w / max(1, self.width())
        x0 = int(np.floor((x - r) * dpr)) - 1
        x1 = int(np.floor((x + r) * dpr)) + 1
        yb0 = h - 1 - (int(np.floor((y + r) * dpr)) + 1)
        yb1 = h - 1 - (int(np.floor((y - r) * dpr)) - 1)
        ix, iy = np.meshgrid(np.arange(x0, x1 + 1), np.arange(yb0, yb1 + 1))
        inside = (ix >= 1) & (ix < w - 1) & (iy >= 1) & (iy < h - 1)
        cx, cy = np.clip(ix, 1, w - 2), np.clip(iy, 1, h - 2)
        # The farthest of what is drawn among the nine (the sky among them
        # is nothing in front): a corner on a silhouette is measured
        # against the farther side of it.
        d = np.zeros(ix.shape, dtype=np.float64)
        for oy in (-1, 0, 1):
            for ox in (-1, 0, 1):
                b = buf[cy + oy, cx + ox]
                d = np.maximum(d, np.where(b < 1.0, b, 0.0))
        M = np.array(inv.data(), dtype=np.float64).reshape(4, 4, order="F")
        e = self.camera.eye()
        e = np.array([e.x(), e.y(), e.z()])

        def dist_at(depth):
            ndc = np.stack([(ix + 0.5) / w * 2.0 - 1.0,
                            (iy + 0.5) / h * 2.0 - 1.0,
                            depth * 2.0 - 1.0, np.ones(ix.shape)], axis=-1)
            q = ndc @ M.T
            with np.errstate(divide="ignore", invalid="ignore"):
                r_ = np.linalg.norm(q[..., :3] / q[..., 3:4] - e, axis=-1)
            return np.where(np.abs(q[..., 3]) >= 1e-12, r_, np.nan)
        far = dist_at(d)
        step = dist_at(np.minimum(d + 8.0 / (1 << 24), 1.0)) - far
        lim = far + np.abs(step) + 0.0005 * far + 0.01
        ok = (inside & (buf[cy, cx] < 1.0) & np.isfinite(lim)
              & (far <= 1.01 * self.camera.far_plane()))
        lim = np.ascontiguousarray(np.where(ok, lim, np.inf))
        return (e.tolist(), lim, x0, yb0, float(dpr), int(h))

    @staticmethod
    def _depth_hidden(seen, pts, sx, sy):
        """core.instances.DepthLimits.hides for arrays: per point (N,3) at
        widget pixel ``(sx, sy)``, whether it lies past its pixel's limit."""
        import numpy as np
        eye, lim, x0, y0, dpr, hdev = seen
        gh, gw = lim.shape
        ix = np.floor(sx * dpr).astype(np.int64) - x0
        iy = hdev - 1 - np.floor(sy * dpr).astype(np.int64) - y0
        at = (ix >= 0) & (iy >= 0) & (ix < gw) & (iy < gh)
        out = np.zeros(len(pts), dtype=bool)
        k = np.flatnonzero(at)
        if len(k):
            dist = np.sqrt(((pts[k] - np.array(eye)) ** 2).sum(axis=1))
            out[k] = dist > lim[iy[k], ix[k]]
        return out

    def _footprint_need(self, table, x: float, y: float, reach=None):
        """The rows of the lazy ``table`` a snap's footprint at widget pixel
        ``(x, y)`` takes in (see _pick_materialize): the boxes near the
        pixel, but what lies wholly behind what the frame shows there or
        under a pixel, and of those the ones the spatial index finds
        something near the cursor in. ``reach`` widens the boxes' margin
        past ``_PICK_NEAR_PX`` (the edges near the cursor are looked for
        farther out). Memoised per view and pixel."""
        import numpy as np
        lo, hi = table[1], table[2]
        r = max(float(reach or 0.0), self._PICK_NEAR_PX)
        ckey = (self._placements_epoch(), round(x), round(y),
                tuple(self._np_mvp().ravel()), r)
        # One per margin: a hover asks for both (the pick's, the edges').
        slot = ("_pick_near_memo" if r == self._PICK_NEAR_PX
                else "_edge_near_memo")
        cmemo = getattr(self, slot, None)
        if cmemo is not None and cmemo[0] == ckey:
            return cmemo[1]
        need = self._placements_under_px(lo, hi, x, y, r)
        far = self._visible_depth_limit(x, y)
        if len(need) and (far is not None or self._PICK_MIN_PX > 0.0):
            dist, small = self._footprint_rows(table)
            if far is not None:
                # What lies wholly behind what is on screen around the
                # cursor can be neither picked nor snapped to (hidden
                # snaps are dropped): a dense plant stacks thousands of
                # placements under one pixel (#158).
                need = need[dist[need] <= far]
            if small is not None:
                # Nor what the frame leaves out for being under a pixel
                # (see _instances_below_a_pixel): from the whole plant
                # of #158 the snap's footprint met more placements
                # than the cap, and each pointer move threw out the
                # last move's to bring in as many again.
                need = need[~small[need]]
        iset = self._pick_instance_set(table) if len(need) else None
        if iset is not None:
            # Of the boxes near the cursor, the placements with a
            # hard edge within the farthest reach a snap measures
            # or a triangle within the footprint (with margins).
            M = self._np_mvp()
            need = np.asarray(iset.screen_filter(
                need.astype(np.int64),
                np.ascontiguousarray(M.T).ravel().tolist(),
                float(self.width()), float(self.height()), x, y,
                DocumentInteraction.GEDGE_REACH_PX + 2.0,
                r + 2.0), dtype=np.int64)
        setattr(self, slot, (ckey, need))
        return need

    def _footprint_rows(self, table):
        """For every row of the lazy ``table``, how far its box lies from
        the eye and whether it spans under ``_PICK_MIN_PX`` (None when
        that floor is off): what _footprint_need cuts by, the same for
        any pixel of the view. Kept for the view: over the 16 000 rows
        near the cursor in the plant of #158 they cost ~2 ms a search,
        two searches a pointer move."""
        import numpy as np
        lo, hi = table[1], table[2]
        e = self.camera.eye()
        h = (self._export_size or self._fb_size())[1]
        key = (self._placements_epoch(), tuple(self._np_mvp().ravel()),
               (e.x(), e.y(), e.z()), h, bool(self.camera.perspective),
               self._PICK_MIN_PX)
        memo = getattr(self, "_footprint_rows_memo", None)
        if (memo is not None and memo[0] == key and memo[1] is lo
                and memo[2] is hi):
            return memo[3]
        eye = np.array([e.x(), e.y(), e.z()])
        dist = np.linalg.norm(np.clip(eye, lo, hi) - eye, axis=1)
        small = None
        if self._PICK_MIN_PX > 0.0:
            ctr, rad = self._bounding_spheres(lo, hi)
            small = self._spheres_below_a_pixel(ctr, rad, self._PICK_MIN_PX)
        self._footprint_rows_memo = (key, lo, hi, (dist, small))
        return dist, small

    def _visible_depth_limit(self, x: float, y: float):
        """The farthest distance from the eye of what the last frame shows
        within ``_PICK_NEAR_PX`` of widget pixel ``(x, y)``, with a margin —
        or ``None`` when the frame cannot say (no read-back, the camera
        moved since, or sky in the window: then anything may be there)."""
        import numpy as np
        if getattr(self, "_depth_snap", None) is None:
            return None
        snap = self._depth_snap
        buf, inv, w, h = snap[0], snap[2], snap[3], snap[4]
        dpr = w / max(1, self.width())
        r = int(self._PICK_NEAR_PX * dpr) + 1
        cx, cy = int(x * dpr), int(h - 1 - y * dpr)
        win = buf[max(cy - r, 0):cy + r + 1, max(cx - r, 0):cx + r + 1]
        if not win.size or float(win.max()) >= 1.0:
            return None
        iy, ix = np.unravel_index(int(np.argmax(win)), win.shape)
        px = max(cx - r, 0) + ix
        py = max(cy - r, 0) + iy
        d = float(win.max())
        q = inv.map(QVector4D((px + 0.5) / w * 2 - 1, (py + 0.5) / h * 2 - 1,
                              d * 2 - 1, 1.0))
        if abs(q.w()) < 1e-12:
            return None
        far = (QVector3D(q.x() / q.w(), q.y() / q.w(), q.z() / q.w())
               - self.camera.eye()).length()
        return far * 1.02 + 0.05

    def _placements_under_px(self, lo, hi, x, y, r=None):
        """Indices of the boxes ``lo``/``hi`` whose screen footprint comes
        within ``r`` (``_PICK_NEAR_PX``) of widget pixel ``(x, y)``; a box that
        crosses the eye plane counts (it can be anywhere on screen)."""
        import numpy as np
        from core import _native
        M = self._np_mvp()
        r = self._PICK_NEAR_PX if r is None else r
        mod = _native.native
        if mod is not None and hasattr(mod, "boxes_near_px"):
            # The same test in parallel: ~9 ms of NumPy a pointer move over
            # the 21 406 boxes of #158, twice (the pick's, the edges').
            return mod.boxes_near_px(
                np.ascontiguousarray(lo, np.float64),
                np.ascontiguousarray(hi, np.float64),
                np.ascontiguousarray(M, np.float64).ravel().tolist(),
                float(self.width()), float(self.height()), float(x),
                float(y), float(r))
        corners = np.stack([np.where(np.array([(i >> k) & 1 for k in range(3)],
                                              bool), hi, lo)
                            for i in range(8)], axis=1)          # (P, 8, 3)
        finite = np.isfinite(corners).all(axis=(1, 2))
        c = np.where(np.isfinite(corners), corners, 0.0)
        clip = c @ M[:, :3].T + M[:, 3]                          # (P, 8, 4)
        w = clip[..., 3]
        front = w > 1e-9
        ww = np.where(front, w, 1.0)
        sx = (clip[..., 0] / ww * 0.5 + 0.5) * self.width()
        sy = (1.0 - (clip[..., 1] / ww * 0.5 + 0.5)) * self.height()
        big = 1e12
        x0 = np.where(front, sx, big).min(axis=1)
        x1 = np.where(front, sx, -big).max(axis=1)
        y0 = np.where(front, sy, big).min(axis=1)
        y1 = np.where(front, sy, -big).max(axis=1)
        inside = (x0 - r <= x) & (x <= x1 + r) & (y0 - r <= y) & (y <= y1 + r)
        straddle = front.any(axis=1) & ~front.all(axis=1)
        return np.flatnonzero((inside & front.all(axis=1)) | straddle
                              | ~finite)

    def _pick_block_part(self, candidatos, tocables, lazy_set, base):
        """One part of the pick block (see _pick_index): the faces, owners,
        triangles and hard edges of ``candidatos``, numbered after ``base``
        (the part before it, or None). Lists, joined by _pick_join_parts."""
        import numpy as np
        scene = self.scene
        chunks = []
        for g in candidatos:
            if getattr(g, "billboard", False):
                continue          # per-frame quad; picked separately
            gvis = scene.entity_visible(g)
            # Locked objects remain inference references, but cannot be
            # edited. The context further limits what a pick may touch.
            gsnap = scene.group_snappable(g)
            gsel = scene.entity_selectable(g) and id(g) in tocables
            if not (gvis or gsnap):
                continue
            chunk = self._group_chunk(g)
            if not (chunk["faces"] or chunk["edges"]):
                continue          # nothing in it to pick or snap to
            # Note the `or edges`: a group of nothing but lines and arcs
            # has no faces, and skipping it here dropped it out of the
            # index ENTIRELY — so inference found none of its edges and
            # the edge fallback below, written for "a lines-only group",
            # read an empty array and never found it either (GitHub #8).
            # The OWNER a hit resolves to depends on the open context
            # (inside the plaza its children are the objects): it keys
            # the block too, or a double-click into the plaza kept
            # answering "the plaza" for the arch inside (Marco,
            # 2026-09-14).
            chunks.append((g, chunk, gvis, gsel, gsnap))
        _pick_warm_take(self)
        # Each big chunk's grid is its own work, mostly NumPy: side by side
        # (0.9 s of a big model's first hover one after another, #1092). Each
        # keeps it on its chunk, where the loop below finds it.
        big = [c for g, c, _v, _s, _n in chunks
               if c["v0"] is not None and id(g) not in lazy_set
               and len(c["v0"]) > _SPAN_GRID_MIN and "_span_grid" not in c]
        if len(big) > 1 and not _NO_SPAN_GRID:
            import os
            from concurrent.futures import ThreadPoolExecutor
            with ThreadPoolExecutor(
                    max_workers=min(8, os.cpu_count() or 2)) as ex:
                list(ex.map(_split_chunk_spans, big))
        n_ent = base["n_ent"] if base else 0
        n_plist = base["n_plist"] if base else 0
        b_tri_off = base["n_tri"] if base else 0
        n_ggroups = base["n_ggroups"] if base else 0
        b_entities: list = []
        # Per entity, the PLACEMENT (proxy) whose matrix puts the
        # face in the world — the owner above is what a click
        # selects. Run-length, one int per face: a Python object
        # per face doubled the cold build (30 → 65 ms).
        b_pidx: list = []
        b_plist: list = []
        b_v0, b_e1, b_e2, b_te = [], [], [], []
        b_area, b_vis, b_sel, b_infer = [], [], [], []
        b_spans: list = []    # (bbox, tri start, count) per chunk
        # Group hard edges, flat — pick_group's fallback (cursor over
        # empty space / lines-only group) walked every group edge in
        # Python (~300 ms per mouse move against a 300k-edge import).
        b_gea, b_geb, b_ggi = [], [], []
        b_gel: list = []
        b_gbase: list = []    # each chunk's (n, 2, 3) edge array, kept
        b_gsel: list = []     # per edge: may pick_group land on it
        b_ggroups: list = []
        b_gctx: list = []     # per group: the tip's context word
        # Per group: a placement of lines only (no faces), picked by its
        # lines before a face under the cursor (SelectTool._linework).
        b_gwire: list = []
        # Lazy placements (#158) keep their triangles in their own
        # arrays — spans the ray walks as they are — and go after
        # the static chunks, whose concatenation is kept: joining
        # every array on each change of the lazy set copied
        # hundreds of MB per hover.
        b_own: list = []
        static_key: list = []
        count = 0
        for g, chunk, gvis, gsel, gsnap in chunks:
            n = len(chunk["faces"])
            off = n_ent + count
            owner = self._owner_of(g)
            # A chunk keeps its (face, owner) list: the block is
            # rebuilt whenever the lazy placements change (#158),
            # and re-making a tuple per face of everything already
            # in it cost ~250 ms a hover on 14 million faces.
            ents = chunk.get("_pick_ents")
            if ents is None or ents[0] is not owner:
                from views import array_bake
                if isinstance(chunk["faces"], array_bake.DeferredFaces):
                    # A lazy mesh (experimental engine): its faces
                    # are built when a pick lands on one.
                    pairs = array_bake.LazyPairs(chunk["faces"], owner)
                else:
                    pairs = [(f, owner) for f in chunk["faces"]]
                ents = chunk["_pick_ents"] = (owner, pairs)
            b_entities.append(ents[1])
            count += n
            b_pidx.append(np.full(n, n_plist + len(b_plist), dtype=np.int32))
            b_plist.append(g)
            b_area.append(chunk["areas"])
            b_vis.append(np.full(n, gvis, dtype=bool))
            b_sel.append(np.full(n, gsel, dtype=bool))
            # Inferred from even when the open context makes it untouchable:
            # the rest of the model is a reference, not an answer.
            b_infer.append(np.full(n, gsnap, dtype=bool))
            if chunk["v0"] is not None and id(g) in lazy_set:
                b_own.append((chunk.get("bbox"), chunk["v0"],
                              chunk["e1"], chunk["e2"],
                              chunk["tri_ent"], off))
            elif chunk["v0"] is not None:
                static_key.append((chunk.get("uid", id(chunk)), chunk.get("rev"), off))
                v0, e1, e2, te, cells = _split_chunk_spans(chunk)
                b_v0.append(v0)
                b_e1.append(e1)
                b_e2.append(e2)
                b_te.append(te + off)
                if cells is None:
                    b_spans.append((chunk.get("bbox"), b_tri_off,
                                    len(v0)))
                else:
                    # One span per grid cell, each with its own box:
                    # a ray meets the cells it crosses, not the chunk.
                    b_spans += [(bb, b_tri_off + s0, n)
                                for bb, s0, n in cells]
                b_tri_off += len(v0)
            if gsnap and chunk["edges"]:
                ge = chunk.get("_pick_ge")
                if ge is None or ge[0] is not chunk["edges"]:
                    ge = chunk["_pick_ge"] = (
                        chunk["edges"],
                        np.frombuffer(chunk["edges"], dtype=np.float32)
                        .reshape(-1, 2, 3).astype(np.float64))
                ge = ge[1]
                b_gbase.append(ge)
                b_gea.append(ge[:, 0])
                b_geb.append(ge[:, 1])
                b_ggi.append(np.full(len(ge), n_ggroups + len(b_ggroups),
                                     dtype=np.int64))
                own = chunk.get("edge_layer")
                gel = np.empty(len(ge), dtype=object)
                if own is not None and len(own) == len(ge):
                    gel[:] = own
                b_gel.append(gel)
                b_gsel.append(np.full(len(ge), gsel, dtype=bool))
                b_ggroups.append(owner)
                # "outside" while a group/component is open and this
                # placement is not part of it (see _snap_context_of).
                b_gctx.append(_snap_context_of(self, g))
                b_gwire.append(n == 0)
        part = {"entities": b_entities, "pidx": b_pidx, "plist": b_plist,
                "area": b_area, "vis": b_vis, "sel": b_sel,
                "infer": b_infer,
                "own": b_own, "static_key": static_key,
                "v0": b_v0, "e1": b_e1, "e2": b_e2, "te": b_te,
                "spans": b_spans, "gea": b_gea, "geb": b_geb, "ggi": b_ggi,
                "gel": b_gel,
                "gsel": b_gsel, "ggroups": b_ggroups, "gctx": b_gctx,
                "gwire": b_gwire, "gbase": b_gbase,
                "n_ent": n_ent + count, "n_plist": n_plist + len(b_plist),
                "n_tri": b_tri_off,
                "n_ggroups": n_ggroups + len(b_ggroups)}
        if base is None:
            # The fixed part joins its own lists once, here: it is kept.
            for k in ("pidx", "area", "vis", "sel", "infer", "gea", "geb",
                      "ggi", "gel", "gsel"):
                part[k] = [np.concatenate(part[k])] if part[k] else []
            if all(isinstance(e, list) for e in b_entities):
                part["entities"] = [[x for e in b_entities for x in e]]
        return part

    def _pick_join_parts(self, fixed, lazy) -> dict:
        """The pick block of the fixed part followed by the lazy one — the
        very arrays the single pass made, in the same order."""
        import numpy as np
        from views.array_bake import PickEntities

        def cat(k, empty):
            parts = fixed[k] + lazy[k]
            return np.concatenate(parts) if parts else empty
        ents = fixed["entities"] + lazy["entities"]
        if not lazy["entities"] and len(ents) == 1 and \
                isinstance(ents[0], list):
            entities: list = ents[0]      # the fixed part's, kept
        elif all(isinstance(e, list) for e in ents) and \
                sum(map(len, ents)) < 200_000:
            entities = [x for e in ents for x in e]
        else:
            entities = PickEntities()
            for e in ents:
                entities.extend(e)
        static = fixed["static_key"] + lazy["static_key"]
        gea = fixed["gea"] + lazy["gea"]
        return {
            "entities": entities,
            "place_idx": cat("pidx", np.empty(0, np.int32)),
            "placements": fixed["plist"] + lazy["plist"],
            "areas": cat("area", np.empty(0)),
            "vis": cat("vis", np.empty(0, bool)),
            "sel": cat("sel", np.empty(0, bool)),
            "infer": cat("infer", np.empty(0, bool)),
            **DocumentInteraction._pick_static_tris(self, tuple(static),
                                         fixed["v0"] + lazy["v0"],
                                         fixed["e1"] + lazy["e1"],
                                         fixed["e2"] + lazy["e2"],
                                         fixed["te"] + lazy["te"]),
            "own": fixed["own"] + lazy["own"],
            "gedge_a": np.concatenate(gea) if gea else None,
            "gedge_b": cat("geb", None) if gea else None,
            "gedge_gi": cat("ggi", None) if gea else None,
            "gedge_layer": cat("gel", None) if gea else None,
            "gedge_sel": cat("gsel", None) if gea else None,
            "gedge_groups": fixed["ggroups"] + lazy["ggroups"],
            "gedge_ctx": fixed.get("gctx", []) + lazy.get("gctx", []),
            "gedge_wire": fixed.get("gwire", []) + lazy.get("gwire", []),
            # The fixed part's edges lead, and that part outlives many lazy
            # rebuilds: their screen projection is kept across them.
            "gedge_fixed": (fixed, sum(map(len, fixed["gea"])),
                            lazy["gbase"]),
            "spans": fixed["spans"] + lazy["spans"],
        }

    def _outer_loose_meshes(self):
        """The root loose mesh drawn AROUND an open group / component, as a
        tuple of meshes (usually one, or none).

        Opening a group points ``scene.mesh`` at its mesh: the loose
        geometry of the outer context stops being the loose part of the
        index, and it was nowhere else in it — the rest of the model could
        be inferred on only where it was grouped («estoy dentro del
        componente y no me snapea a lo de fuera»). It is drawn faded
        (``scene.loose_mesh``), so it must occlude, snap and be acquirable
        as a reference like every other faded thing.

        Hidden with «Hide rest of model» it is not drawn, so it is not
        offered either (same rule as the placements, see
        ``_pick_candidates``). A mesh a placement already draws is left out
        or the model would be snapped through twice (a copied group can
        share the loose mesh)."""
        scene = self.scene
        if scene.edit_group is None:
            return ()
        rest_hidden = getattr(self, "_rest_is_hidden", None)
        if callable(rest_hidden) and rest_hidden():
            return ()
        outer = getattr(scene, "loose_mesh", None)
        if outer is None or outer is scene.mesh:
            return ()
        key = (_content_ver(self), id(outer), id(scene.edit_group),
               id(scene.mesh))
        memo = getattr(self, "_outer_loose_memo", None)
        if memo is not None and memo[0] == key:
            return memo[1]
        carried = {id(g.mesh) for g in self._placements()}
        meshes = () if id(outer) in carried else (outer,)
        self._outer_loose_memo = (key, meshes)
        return meshes

    def _loose_mesh_wrapper(self, mesh):
        """A stable stand-in for a mesh that is a *reference*, not an
        object: what an outer loose face rides in the index so the snap
        paths can tell its mesh (and its world frame: identity) from the
        open context's. Cached, as ``_group_chunk`` keys on ``id``."""
        wrappers = getattr(self, "_loose_wrappers", None)
        if wrappers is None:
            wrappers = self._loose_wrappers = {}
        w = wrappers.get(id(mesh))
        if w is None:
            from types import SimpleNamespace
            w = wrappers[id(mesh)] = SimpleNamespace(
                mesh=mesh, xform=None, material=None, name=None,
                billboard=False, hidden=False, locked=False, layer=None)
        return w

    def _pick_index(self, near=None):
        """Flat NumPy pick index of the scene — triangles of every loose and
        group face (with visibility/selectability masks and areas) plus the
        loose edges — rebuilt when the scene changes.

        Every mouse-move pick used to walk the mesh in Python re-running
        earcut per face (~1–2 s per move against an imported 17k-triangle
        building — the app read as frozen); batched over this index a pick
        is a couple of milliseconds.

        On a model whose component placements hold more than
        ``_PICK_LAZY_MIN_FACES`` faces, those placements enter only when a
        query comes near them (``near``, see ``_pick_materialize``): the
        rest of the index is the same."""
        if hasattr(self, "_pick_lazy_table"):
            # The snap asks occlusion hundreds of times per mouse move, each
            # through here: the same cursor query in the same tick has
            # nothing new to bake (#158).
            pos = getattr(self, "_last_mouse_pos", None)
            qkey = (getattr(self, "_tick", 0), _content_ver(self),
                    (pos.x(), pos.y()) if near is None and pos is not None
                    else repr(near) if not isinstance(near, tuple) or
                    near[0] != "ray" else None)
            if qkey[2] is None or getattr(self, "_pick_last_q", None) != qkey:
                self._pick_materialize(near)
                self._pick_last_q = qkey
        oculto = getattr(self, "_rest_is_hidden", None)
        oculto = bool(oculto()) if callable(oculto) else False
        # The open context decides what a hit RESOLVES to (inside the
        # plaza its children are the objects, not the plaza): a stale index
        # kept answering "the plaza" after a double-click opened it, so the
        # arch inside never opened (Marco, 2026-09-14).
        # The content version: a click (a view-only bump) changes nothing
        # in it — on the plant of #158 it rebuilt it on every click.
        key = (_content_ver(self), id(self.scene.mesh), oculto,
               id(self.scene.edit_group), getattr(self, "_pick_gen", 0))
        cached = getattr(self, "_pick_index_cache", None)
        if cached is not None and cached[0] == key:
            return cached[1]
        if (cached is not None and len(cached) > 2
                and cached[0][:4] == key[:4]
                and getattr(self, "_frozen_cache_version", None) is not None
                and cached[2] is not None
                and cached[2] is getattr(self, "_pick_block", None)):
            # Only the lazy set moved, under a frozen preview (a drag): the
            # block is kept frozen as it is, the loose part is the same —
            # the rebuild would make this very index again (it did, ~35 ms
            # a pointer move dragging beside the plant of #158).
            self._pick_index_cache = (key, cached[1], cached[2])
            return cached[1]
        _p0 = _time_mod.perf_counter() if _PERF else 0.0
        import numpy as np
        from types import SimpleNamespace
        scene = self.scene
        entities: list = []           # (face, group_or_None)
        place_idx = None              # per entity: index into placements, -1 = loose
        placements: list = []
        ent_area: list = []
        ent_sel: list = []
        ent_vis: list = []
        ent_loose: list = []
        ent_infer: list = []
        tris: list = []
        tri_ent: list = []

        # Stub viewports in tests call this unbound — fall back to the
        # uncached face methods when the memo helpers aren't there.
        area_of = getattr(self, "_area_of", None) or (lambda f: f.area())
        tris_of = getattr(self, "_tris_of", None) or \
            (lambda f: f.triangulate())

        def add_face(f, grp, vis, sel, infer=None, loose=None):
            i = len(entities)
            entities.append((f, grp))
            ent_area.append(area_of(f))
            ent_vis.append(vis)
            ent_sel.append(sel)
            # What the snap engine may INFER FROM: selectable, except that
            # the rest of the model outside the open context is a reference
            # even though it cannot be picked (see _face_hits "infer").
            ent_infer.append(sel if infer is None else infer)
            ent_loose.append((grp is None) if loose is None else loose)
            for t0, t1, t2 in tris_of(f):
                tris.append((t0.x(), t0.y(), t0.z(),
                             t1.x(), t1.y(), t1.z(),
                             t2.x(), t2.y(), t2.z()))
                tri_ent.append(i)

        # The loose part lasts while the content and the context do: a
        # change of the lazy set (a few times a pointer move over the plant
        # of #158) rebuilds the index, and walked the loose faces again.
        lmemo = getattr(self, "_pick_loose_memo", None)
        if lmemo is not None and lmemo[0] == key[:4]:
            loose = lmemo[1]
        else:
            for f in scene.faces:
                add_face(f, None, scene.entity_visible(f),
                         scene.entity_selectable(f))
            # The loose mesh drawn AROUND an open group / component: the
            # rest of the model when it was never grouped. It cannot be
            # picked or selected from inside the context, but it is drawn
            # (faded), so it occludes and the snap engine infers from it
            #. Its own mesh rides with the face (the wrapper) so
            # the plane and the centre come from the right mesh.
            outer_of = getattr(self, "_outer_loose_meshes", None)
            for om in (outer_of() if outer_of is not None else ()):
                wrap = self._loose_mesh_wrapper(om)
                for f in om.faces:
                    add_face(f, wrap, scene.entity_visible(f),
                             False, infer=scene.entity_selectable(f),
                             loose=True)
            # Loose part → arrays; groups append their cached chunk arrays.
            if tris:
                t = np.asarray(tris, dtype=np.float64).reshape(-1, 3, 3)
                lt = (t[:, 0], t[:, 1] - t[:, 0], t[:, 2] - t[:, 0],
                      np.asarray(tri_ent, dtype=np.int64))
            else:
                lt = None
            loose = (list(entities), lt, len(tris),
                     np.asarray(ent_area, dtype=np.float64),
                     np.asarray(ent_vis, dtype=bool),
                     np.asarray(ent_sel, dtype=bool),
                     np.asarray(ent_infer, dtype=bool),
                     np.asarray(ent_loose, dtype=bool))
            self._pick_loose_memo = (key[:4], loose)
        entities = list(loose[0])
        lt, n_loose_tris = loose[1], loose[2]
        if lt is not None:
            v0s, e1s, e2s, tents = [lt[0]], [lt[1]], [lt[2]], [lt[3]]
        else:
            v0s, e1s, e2s, tents = [], [], [], []
        areas, vis_parts, sel_parts, infer_parts, loose_parts = (
            [loose[3]], [loose[4]], [loose[5]], [loose[6]], [loose[7]])

        # Per-chunk triangle spans (P3): a hover/zoom ray prefilters chunks
        # by AABB and runs Möller-Trumbore only on the spans it crosses.
        tri_spans: list = [(None, 0, n_loose_tris)] if n_loose_tris else []
        own_spans: list = []      # lazy placements' own arrays (#158)
        # What the current context lets you TOUCH: the whole scene at the
        # root, the children of the group you are inside otherwise. This
        # used to be `if scene.edit_group is None`, which took EVERY group
        # out of the index while you were inside one — so a group's children
        # were drawn and could not be clicked, and «todo está combinado»
        # (Marco, 2026-09-11).
        lazy = (self._pick_lazy_table()
                if hasattr(self, "_pick_lazy_table") else None)
        epoch_of = getattr(self, "_placements_epoch", None)   # stub VPs in tests
        # The context's placements, split into fixed and lazy, last as long
        # as the placements and the context: walking all 21 000 of them on
        # each change of the lazy set was ~20 ms of every such rebuild, a
        # few times a mouse move over the plant of #158.
        ckey = ((epoch_of(), id(scene.edit_group), oculto, id(lazy))
                if epoch_of is not None else None)
        cmemo = getattr(self, "_pick_cands_memo", None)
        if ckey is None or cmemo is None or cmemo[0] != ckey:
            cmemo = (ckey, *DocumentInteraction._pick_candidates(self, oculto, lazy))
            if ckey is not None:
                self._pick_cands_memo = cmemo
        _ck, tocables, fixed_c, lazy_pos = cmemo
        if lazy is not None:
            live = getattr(self, "_pick_live", None) or {}
            lazies_c = [lazy_pos[k][1] for k in sorted(
                (k for k in live if k in lazy_pos),
                key=lambda k: lazy_pos[k][0])]
        else:
            lazies_c = [g for _p, g in lazy_pos.values()]
        candidatos = fixed_c + lazies_c
        if candidatos:
            # The group block is cached across versions (keyed by each
            # chunk's identity + rev + flags): re-deriving per-face masks and
            # re-offsetting 300k triangle rows per scene change cost ~130 ms
            # per stroke/drag frame beside a big import.
            # The block (every placement's faces, owners and hard edges) is
            # keyed on the placements epoch: a loose edit — every frame of a
            # Move drag — no longer walks 1 300 chunks to learn that none of
            # them changed (2026-09-14).
            sig = (epoch_of() if epoch_of is not None else _cache_ver(self),
                   oculto, len(candidatos), getattr(self, "_pick_gen", 0))
            blk = getattr(self, "_pick_block", None)
            frozen = getattr(self, "_frozen_cache_version", None) is not None
            if blk is None or (blk[0] != sig and not frozen):
                # Two parts, fixed placements first, as the block always
                # ran: the fixed part lasts while the placements do, and a
                # change of the lazy set rebuilds only the lazy part on its
                # offsets. Re-making the fixed one each time (900
                # placements, half a million faces) was most of every hover
                # that met new lazy placements on the plant of #158.
                lazy_set = (lazy[4] if lazy is not None else ())
                fixed, lazies = fixed_c, lazies_c
                fkey = (sig[0], oculto, len(fixed), id(scene.edit_group))
                fpart = getattr(self, "_pick_fixed_part", None)
                if fpart is None or fpart[0] != fkey:
                    # Called through the class: the stub viewports of the
                    # tests run this unbound.
                    fpart = self._pick_fixed_part = (fkey, DocumentInteraction._pick_block_part(
                        self, fixed, tocables, lazy_set, None))
                lpart = DocumentInteraction._pick_block_part(self, lazies, tocables,
                                                  lazy_set, fpart[1])
                blk = (sig, DocumentInteraction._pick_join_parts(self, fpart[1], lpart))
                self._pick_block = blk
            block = blk[1]
            if block["entities"]:
                offset = len(entities)
                loose_count = len(entities)
                if not isinstance(block["entities"], list):
                    from views.array_bake import PickEntities
                    entities = PickEntities(entities)
                entities.extend(block["entities"])
                place_idx = np.concatenate([
                    np.full(loose_count, -1, dtype=np.int32),
                    block.get("place_idx", np.full(len(block["entities"]), -1, np.int32))])
                placements = block.get("placements", [])
                areas.append(block["areas"])
                vis_parts.append(block["vis"])
                sel_parts.append(block["sel"])
                infer_parts.append(block.get("infer", block["sel"]))
                loose_parts.append(
                    np.zeros(len(block["entities"]), dtype=bool))
                if block["v0"] is not None:
                    v0s.append(block["v0"])
                    e1s.append(block["e1"])
                    e2s.append(block["e2"])
                    tents.append(_offset_kept(self, block["te"], offset))
                    tri_spans += [(bb, n_loose_tris + s, n)
                                  for bb, s, n in block.get("spans", ())]
                own_spans = [(bb, a, b, c, te, o + offset)
                             for bb, a, b, c, te, o in block.get("own", ())]

        gedge_a = gedge_b = gedge_gi = gedge_sel = gedge_fixed = None
        gedge_layer = None
        gedge_groups: list = []
        gedge_ctx: list = []
        gedge_wire: list = []
        if candidatos:
            block = self._pick_block[1] if getattr(self, "_pick_block", None) \
                else {}
            gedge_a = block.get("gedge_a")
            gedge_b = block.get("gedge_b")
            gedge_gi = block.get("gedge_gi")
            gedge_layer = block.get("gedge_layer")
            gedge_sel = block.get("gedge_sel")
            gedge_groups = block.get("gedge_groups", [])
            gedge_ctx = block.get("gedge_ctx", [])
            gedge_wire = block.get("gedge_wire", [])
            gedge_fixed = block.get("gedge_fixed")

        edges: list = []
        ea: list = []
        eb: list = []
        esel: list = []
        esnap: list = []
        esoft: list = []
        for e in scene.edges:
            edges.append(e)
            esoft.append(bool(getattr(e, "soft", False)))
            ea.append([e.a.x(), e.a.y(), e.a.z()])
            eb.append([e.b.x(), e.b.y(), e.b.z()])
            # A hidden EDGE (Hide on raw geometry, a smoothed surface's
            # inner edges) is selectable only while the hidden view shows
            # it — what window selection already did; a click found it
            # through here with the view off.
            esel.append(scene.entity_selectable(e)
                        and (not getattr(e, "hidden", False)
                             or bool(getattr(scene, "show_hidden_geometry",
                                             False))))
            # Snapping is a different question from selecting: a locked
            # edge is no target for a click and a fine target for a
            # dimension, while an edge that draws nothing is neither. The
            # same rule the snap scene applies (Scene.edge_snappable), kept
            # here so the corner a from-point acquires cannot come from a
            # line the user cannot see.
            esnap.append(scene.edge_snappable(e))
        # The rest of the model's loose edges — drawn around an open group /
        # component, a reference to snap to and never a thing to pick. They
        # are what a move started inside the component aligns to, and what
        # occludes the corners behind them.
        outer_of = getattr(self, "_outer_loose_meshes", None)
        for om in (outer_of() if outer_of is not None else ()):
            for e in om.edges:
                edges.append(e)
                esoft.append(bool(getattr(e, "soft", False)))
                ea.append([e.a.x(), e.a.y(), e.a.z()])
                eb.append([e.b.x(), e.b.y(), e.b.z()])
                esel.append(False)
                esnap.append(scene.edge_snappable(e))

        idx = SimpleNamespace(
            entities=entities,
            ent_place_idx=(place_idx if place_idx is not None
                           else np.full(len(entities), -1, np.int32)),
            ent_placements=placements,
            ent_area=np.concatenate(areas) if entities else np.empty(0),
            ent_sel=np.concatenate(sel_parts) if entities else np.empty(0, bool),
            ent_infer=(np.concatenate(infer_parts) if entities
                       else np.empty(0, bool)),
            ent_vis=np.concatenate(vis_parts) if entities else np.empty(0, bool),
            ent_loose=(np.concatenate(loose_parts) if entities
                       else np.empty(0, bool)),
            tri_v0=_cat_kept(self, "v0", v0s) if v0s else None,
            tri_e1=_cat_kept(self, "e1", e1s) if v0s else None,
            tri_e2=_cat_kept(self, "e2", e2s) if v0s else None,
            tri_ent=_cat_kept(self, "te", tents) if v0s else None,
            edges=edges,
            edge_a=np.asarray(ea, dtype=np.float64) if edges else None,
            edge_b=np.asarray(eb, dtype=np.float64) if edges else None,
            edge_sel=np.asarray(esel, dtype=bool) if edges else None,
            edge_snap=np.asarray(esnap, dtype=bool) if edges else None,
            edge_soft=np.asarray(esoft, dtype=bool) if edges else None,
            gedge_a=gedge_a,
            gedge_b=gedge_b,
            gedge_gi=gedge_gi,
            gedge_layer=gedge_layer,
            gedge_sel=gedge_sel,
            gedge_groups=gedge_groups,
            gedge_ctx=gedge_ctx,
            gedge_wire=gedge_wire,
            gedge_fixed=gedge_fixed,
            tri_spans=None,
        )
        # Spans must tile the triangle array exactly, or the prefilter would
        # silently drop triangles (a stale cached block predating spans).
        total_tris = len(idx.tri_v0) if idx.tri_v0 is not None else 0
        if tri_spans and sum(n for _, _, n in tri_spans) == total_tris:
            idx.tri_spans = tri_spans
        idx.own_spans = own_spans
        if _PERF:
            _plog("pick_index", (_time_mod.perf_counter() - _p0) * 1000.0)
        self._pick_index_cache = (key, idx, getattr(self, "_pick_block", None))
        return idx

    def _pick_static_tris(self, key, v0, e1, e2, te) -> dict:
        """The static chunks' triangles joined, kept while the same chunks
        sit at the same entity offsets: the lazy set changes on many
        hovers (#158), the static part does not."""
        import numpy as np
        memo = getattr(self, "_pick_static_memo", None)
        if memo is not None and memo[0] == key:
            return memo[1]
        out = {"v0": np.concatenate(v0) if v0 else None,
               "e1": np.concatenate(e1) if v0 else None,
               "e2": np.concatenate(e2) if v0 else None,
               "te": np.concatenate(te) if v0 else None}
        self._pick_static_memo = (key, out)
        return out

    @staticmethod
    def _pick_flat(idx):
        """``idx`` with the lazy placements' own spans joined into the flat
        triangle arrays — for the passes that read every triangle (hidden
        lines, section cuts); kept on the index once made."""
        own = getattr(idx, "own_spans", None)
        if not own:
            return idx
        flat = getattr(idx, "_flat", None)
        if flat is not None:
            return flat
        import numpy as np
        from types import SimpleNamespace
        parts = ([(idx.tri_v0, idx.tri_e1, idx.tri_e2, idx.tri_ent)]
                 if idx.tri_v0 is not None else [])
        parts += [(a, b, c, te + o) for _bb, a, b, c, te, o in own]
        flat = SimpleNamespace(**vars(idx))
        flat.tri_v0 = np.concatenate([p[0] for p in parts])
        flat.tri_e1 = np.concatenate([p[1] for p in parts])
        flat.tri_e2 = np.concatenate([p[2] for p in parts])
        flat.tri_ent = np.concatenate([p[3] for p in parts])
        flat.tri_spans = None
        flat.own_spans = []
        idx._flat = flat
        return flat

    def ray_distance(self, origin: QVector3D, direction: QVector3D,
                     reach: float | None = None):
        """Distance along ``direction`` (unit) from ``origin`` to the nearest
        VISIBLE face, or ``None`` when the ray meets nothing. What the
        walkthrough tools ask — the floor under the eye, the wall ahead —
        over the same index every pick uses (hidden objects and hidden
        layers are not there to bump into). With ``reach``, only faces
        within that distance count: the query bakes and tests only what
        lies within it."""
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        near = (("ray", origin, direction) if reach is None
                else ("ray", origin, direction, float(reach)))
        idx = self._pick_index(near="skip" if nat is not None else near)
        if nat is not None:
            # The lazy placements from the engine, none baked (#158).
            t = self._ray_hits(idx, origin, direction, idx.ent_vis,
                               reduce_global=True, reach=reach, own=False)
            lz = DocumentInteraction._lazy_face_hits(self, nat, origin, direction,
                                          "vis", reach)
            if lz is not None:
                tl = float(lz[0].min())
                t = tl if t is None else min(t, tl)
        elif idx is None or (getattr(idx, "tri_v0", None) is None
                             and not getattr(idx, "own_spans", None)):
            return None
        else:
            t = self._ray_hits(idx, origin, direction, idx.ent_vis,
                               reduce_global=True, reach=reach)
        if t is None or t == float("inf") or (reach is not None
                                               and t > reach):
            return None
        return float(t)

    def _ray_hits(self, idx, origin, direction, ent_mask,
                  reduce_global: bool = False, reach: float | None = None,
                  own: bool = True):
        """Per-entity nearest ray parameter over the index triangles whose
        entity passes ``ent_mask``. Returns an (E,) array of t (``inf`` = no
        hit), the single nearest t as a float when ``reduce_global``, or
        ``None`` when the index has no triangles. Same acceptance
        thresholds as :func:`_ray_triangle`.

        P3: the Möller–Trumbore pass runs per chunk SPAN on array views,
        after a ray-vs-AABB prefilter — a hover/zoom ray only pays for the
        chunks it actually crosses (the full 700k-row pass cost 25–45 ms
        per pick against the piscina scene). ``own`` False leaves out the
        lazy placements' own spans (the engine answers for them, see
        _face_hits)."""
        import numpy as np
        own = (getattr(idx, "own_spans", None) or ()) if own else ()
        if idx.tri_v0 is None and not own:
            return None
        o = np.array([origin.x(), origin.y(), origin.z()])
        d = np.array([direction.x(), direction.y(), direction.z()])
        fast = (DocumentInteraction._static_ray_rows(
                    self, idx, ent_mask,
                    (float(o[0]), float(o[1]), float(o[2])),
                    (float(d[0]), float(d[1]), float(d[2])))
                if (reach is None and not own and idx.tri_v0 is not None
                    and hasattr(self, "_static_occluders"))
                else None)
        if fast is not None:
            # The engine's BVH in place of the span boxes below.
            spans = ([(None, idx.tri_v0[fast], idx.tri_e1[fast],
                       idx.tri_e2[fast], idx.tri_ent[fast], 0)]
                     if len(fast) else [])
        elif idx.tri_v0 is None:
            spans = []
        else:
            spans = (getattr(idx, "tri_spans", None)
                     or [(None, 0, len(idx.tri_v0))])
        # Lazy placements' own arrays (#158) ride the same loop: a span is
        # then (bbox, v0, e1, e2, local tri_ent, entity offset).
        spans = list(spans) + list(own)
        if len(spans) > 64 and fast is None:
            # Thousands of spans on a huge model: which boxes the ray meets
            # is one NumPy pass, not a Python test per span (900 000 of
            # them in one snap hover, #158).
            boxes = getattr(idx, "_span_boxes", None)
            if boxes is None or boxes[0] != len(spans):
                inf = float("inf")
                blo = np.array([sp[0][0] if sp[0] is not None else (-inf,) * 3
                                for sp in spans], dtype=np.float64)
                bhi = np.array([sp[0][1] if sp[0] is not None else (inf,) * 3
                                for sp in spans], dtype=np.float64)
                boxes = idx._span_boxes = (len(spans), blo, bhi)
            _n, blo, bhi = boxes
            with np.errstate(divide="ignore", invalid="ignore"):
                inv_d = 1.0 / np.where(np.abs(d) > 1e-12, d, 1e-12)
                t1 = (blo - o) * inv_d
                t2 = (bhi - o) * inv_d
            tmin = _slab_far(np.minimum(t1, t2))
            tmax = _slab_near(np.maximum(t1, t2))
            meet = tmax >= np.maximum(tmin, 0.0)
            if reach is not None:
                meet &= tmin <= reach
            spans = [(None,) + tuple(spans[i][1:])
                     for i in np.flatnonzero(meet)]
        o3 = (float(o[0]), float(o[1]), float(o[2]))
        d3 = (float(d[0]), float(d[1]), float(d[2]))
        lim = float("inf") if reach is None else float(reach)
        best = float("inf")
        face_t = None if reduce_global else np.full(len(idx.entities), np.inf)

        # The static spans the ray meets go through Möller–Trumbore in ONE
        # pass: with the span grid a ray meets dozens of small spans, and a
        # NumPy call per span (~30 µs) became the ray's whole cost.
        ranges = []
        rest = []
        for span in spans:
            if len(span) == 3:
                bb, s0, n = span
                if n and (bb is None or _ray_aabb(o3, d3, bb[0], bb[1])):
                    ranges.append((s0, n))
            else:
                rest.append(span)
        if ranges:
            if len(ranges) == 1:
                s0, n = ranges[0]
                sel = slice(s0, s0 + n)
            else:
                sel = np.concatenate([np.arange(s0, s0 + n)
                                      for s0, n in ranges])
            rest.insert(0, (None, idx.tri_v0[sel], idx.tri_e1[sel],
                            idx.tri_e2[sel], idx.tri_ent[sel], 0))
        spans = rest
        for span in spans:
            if len(span) == 3:
                bb, s0, n = span
                if not n:
                    continue
                if bb is not None and not _ray_aabb(o3, d3, bb[0], bb[1],
                                                    lim):
                    continue
                v0 = idx.tri_v0[s0:s0 + n]
                e1 = idx.tri_e1[s0:s0 + n]
                e2 = idx.tri_e2[s0:s0 + n]
                te = idx.tri_ent[s0:s0 + n]
            else:
                bb, v0, e1, e2, te_local, eoff = span
                if not len(v0):
                    continue
                if bb is not None and not _ray_aabb(o3, d3, bb[0], bb[1],
                                                    lim):
                    continue
                te = te_local + eoff
            t, hit = _moller_trumbore(o, d, v0, e1, e2)
            hit &= ent_mask[te]
            if reduce_global:
                # Zoom focus wants ONE nearest t, not per-entity buckets.
                th = t[hit]
                if len(th):
                    tm = float(th.min())
                    if tm < best:
                        best = tm
            else:
                hit_rows = np.where(hit)[0]
                if len(hit_rows):
                    # minimum.at only over the triangles the ray actually
                    # hits (a handful) — over the full array it alone cost
                    # ~100 ms per snap hover.
                    np.minimum.at(face_t, te[hit_rows], t[hit_rows])
        return best if reduce_global else face_t

    def _ray_near(self, x: float, y: float):
        """What a ray pick at widget pixel ``(x, y)`` asks the pick index
        to bake: nothing with the engine, which answers for the lazy
        placements itself (see _face_hits), the placements the ray meets
        otherwise."""
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        return "skip" if nat is not None else ("px", x, y)

    def _face_hits(self, idx, origin, direction, which: str = "sel"):
        """The faces the ray from ``origin`` along ``direction`` meets, each
        at its nearest t, of the index entities that pass ``which``
        ("sel": selectable, "vis": visible, "infer": a face the snap
        engine may infer from — the selectable ones PLUS the rest of the
        model around an open group / component, which cannot be picked but
        is drawn as a reference) — in the index's order, which the ties of
        every pick go by: ``SimpleNamespace(t, area, loose, ent(k) →
        (face, owner), place(k) → placement)``, or None when the index has
        no triangles. With the experimental engine the lazy placements'
        faces come from it, none of them baked (#158), after the index's
        own, in the order their bakes would follow them."""
        import numpy as np
        from types import SimpleNamespace
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        if which == "sel":
            mask = idx.ent_sel
        elif which == "infer":
            mask = getattr(idx, "ent_infer", idx.ent_sel)
        else:
            mask = idx.ent_vis
        face_t = (self._ray_hits(idx, origin, direction, mask,
                                 own=nat is None)
                  if len(idx.entities) else None)
        lazy = (DocumentInteraction._lazy_face_hits(self, nat, origin, direction, which)
                if nat is not None else None)
        if face_t is None and lazy is None:
            return None
        rows = (np.flatnonzero(np.isfinite(face_t)) if face_t is not None
                else np.empty(0, np.int64))
        pidx = getattr(idx, "ent_place_idx", None)
        n = len(rows)

        def ent(k):
            if k < n:
                return idx.entities[int(rows[k])]
            return lazy[3](k - n)

        def owner(k):
            if k < n:
                return _entity_owner(idx.entities, int(rows[k]))
            return lazy[5](k - n)

        def place(k):
            if k < n:
                i = int(rows[k])
                if pidx is not None and i < len(pidx) and pidx[i] >= 0:
                    return idx.ent_placements[int(pidx[i])]
                return _entity_owner(idx.entities, i)
            return lazy[4](k - n)
        t = face_t[rows] if n else np.empty(0)
        area = idx.ent_area[rows] if n else np.empty(0)
        loose = idx.ent_loose[rows] if n else np.empty(0, bool)
        if lazy is not None:
            t = np.concatenate([t, lazy[0]])
            area = np.concatenate([area, lazy[1]])
            loose = np.concatenate([loose, np.zeros(len(lazy[0]), bool)])
        return SimpleNamespace(t=t, area=area, loose=loose, ent=ent,
                               owner=owner, place=place)

    def _lazy_sel_rows(self, nat):
        """The rows of the lazy table whose faces a pick may land on (see
        _lazy_native's ``snap``, less what the open context keeps from
        being touched), in the index's order."""
        import numpy as np
        memo = getattr(self, "_lazy_sel_memo", None)
        if memo is not None and memo[0] is nat:
            return memo[1]
        toc = self._pick_cands_memo[1]
        groups = self._pick_lazy_table()[0]
        snap = nat[1]
        sel = np.fromiter((i for i in snap.tolist() if id(groups[i]) in toc
                          and self.scene.entity_selectable(groups[i])),
                          np.int64)
        self._lazy_sel_memo = (nat, sel)
        return sel

    def _lazy_face_hits(self, nat, origin, direction, which,
                        reach: float | None = None):
        """The lazy placements' faces the ray meets, from the engine (see
        _face_hits): ``(t, area, order, ent(k), place(k))`` as their bakes
        would give them, or None. The engine hands the triangles the ray
        may meet; which it meets, and where, is decided here by the pick
        index's own test."""
        import numpy as np
        iset, snap, vis = nat
        if which == "sel":
            rows = DocumentInteraction._lazy_sel_rows(self, nat)
        elif which == "infer":
            # The lazy placements the snap may land on (visible, unlocked),
            # touchable or not — see _face_hits.
            rows = snap
        else:
            rows = vis
        if not len(rows):
            return None
        o = np.array([origin.x(), origin.y(), origin.z()])
        d = np.array([direction.x(), direction.y(), direction.z()])
        pl, tr, v0, e1, e2 = iset.ray_cands(
            np.ascontiguousarray(rows, np.int64), tuple(o.tolist()),
            tuple(d.tolist()), float("inf") if reach is None else float(reach))
        if not len(pl):
            return None
        t, hit = _moller_trumbore(o, d, v0, e1, e2)
        k = np.flatnonzero(hit)
        if not len(k):
            return None
        groups = self._pick_lazy_table()[0]
        lazy_pos = self._pick_cands_memo[3]
        bases: dict = {}
        best: dict = {}                     # (row, entity) → nearest t
        for i, r, tt in zip(pl[k].tolist(), tr[k].tolist(), t[k].tolist()):
            base = bases.get(i)
            if base is None:
                g = groups[i]
                base = bases[i] = self._proto_base_chunk(
                    g.mesh, _paint_of(self, g))
            key = (i, int(base["tri_ent"][r]))
            if tt < best.get(key, float("inf")):
                best[key] = tt
        keys = sorted(best, key=lambda ij: (lazy_pos[id(groups[ij[0]])][0],
                                            ij[1]))
        scale: dict = {}
        for i in bases:
            # The placed areas as _instance_chunk makes them.
            L = _placed_maps(groups[i].xform)[0]
            scale[i] = abs(float(np.linalg.det(L))) ** (2.0 / 3.0)
        tt = np.array([best[ij] for ij in keys], dtype=np.float64)
        area = np.array([bases[i]["areas"][j] * scale[i] for i, j in keys],
                        dtype=np.float64)

        def ent(m):
            i, j = keys[m]
            return bases[i]["faces"][j], self._owner_of(groups[i])

        def place(m):
            return groups[keys[m][0]]

        def owner(m):
            return self._owner_of(groups[keys[m][0]])
        return tt, area, keys, ent, place, owner

    def _hover_hits(self, idx, origin, direction):
        """:meth:`_face_hits` of the selectable faces, memoised for the
        current cursor ray, without what lies past the active cut. One
        SelectTool hover fires pick_group + pick_face on the same mouse
        move, and each used to re-run the full Möller–Trumbore pass (~20 ms
        over a 260k-triangle import); they now share one pass and post-mask
        by loose/group."""
        key = (self.scene.version, id(self.scene.mesh),
               origin.x(), origin.y(), origin.z(),
               direction.x(), direction.y(), direction.z())
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        cached = getattr(self, "_hover_hits_cache", None)
        # The answer is per entity OF THIS INDEX: a lazy index (#158) is
        # rebuilt between two asks of one hover when a query brings new
        # placements in, and the old index's per-entity times read against
        # the new one picked the wrong face, or ran past its end.
        if (cached is not None and cached[0] == key and cached[2] is idx
                and cached[3] is nat):
            return cached[1]
        hits = DocumentInteraction._face_hits(self, idx, origin, direction, "sel")
        DocumentInteraction._drop_past_cut(self, hits, origin, direction)
        self._hover_hits_cache = (key, hits, idx, nat)
        return hits

    def _infer_hits(self, idx, origin, direction):
        """:meth:`_hover_hits` of the faces the snap engine may INFER FROM:
        every selectable one plus the rest of the model around an open group
        / component (drawn faded, never pickable). Geometry being moved is
        still left out by the caller's own rules — this only widens the
        context, not the moving set."""
        key = (self.scene.version, id(self.scene.mesh),
               origin.x(), origin.y(), origin.z(),
               direction.x(), direction.y(), direction.z())
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        cached = getattr(self, "_infer_hits_cache", None)
        if (cached is not None and cached[0] == key and cached[2] is idx
                and cached[3] is nat):
            return cached[1]
        hits = DocumentInteraction._face_hits(self, idx, origin, direction, "infer")
        DocumentInteraction._drop_past_cut(self, hits, origin, direction)
        self._infer_hits_cache = (key, hits, idx, nat)
        return hits

    def _drop_past_cut(self, hits, origin, direction) -> None:
        """Hits beyond the active cut are on clipped-away geometry: their
        ``t`` becomes ``inf``, in place."""
        sp = _active_cut(self.scene)
        if sp is not None and hits is not None and len(hits.t):
            import numpy as np
            o = np.array([origin.x(), origin.y(), origin.z()])
            d = np.array([direction.x(), direction.y(), direction.z()])
            n = np.array([sp.normal.x(), sp.normal.y(), sp.normal.z()])
            c = float(n @ [sp.point.x(), sp.point.y(), sp.point.z()])
            pts = o + d * hits.t[:, None]
            hidden = (pts @ n - c) > 1e-6
            hits.t = np.where(hidden, np.inf, hits.t)

    def edge_point_under_cursor(self, edge, screen_x: float, screen_y: float):
        """The world point of ``edge`` nearest the cursor (closest point of
        its screen-space segment, mapped back along the edge), or None when
        the edge does not project."""
        pa = self._world_to_pixel(edge.a)
        pb = self._world_to_pixel(edge.b)
        if pa is None or pb is None:
            return None
        dx, dy = pb[0] - pa[0], pb[1] - pa[1]
        l2 = dx * dx + dy * dy
        if l2 < 1e-12:
            return QVector3D(edge.a)
        t = ((screen_x - pa[0]) * dx + (screen_y - pa[1]) * dy) / l2
        t = max(0.0, min(1.0, t))
        return edge.a + (edge.b - edge.a) * t

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
        """``(idx, rows, ax, ay, bx, by)``: the pick index's group edges of
        placements made of lines only (no faces) that a click may land
        on — their rows, kept on the index, and their screen endpoints —
        or ``None`` when there are none (or the lazy engine holds the
        placements: it keeps no such flag)."""
        import numpy as np
        if hasattr(self, "_lazy_native") and self._lazy_native() is not None:
            return None
        idx = self._pick_index(near=("px", screen_x, screen_y))
        wire = getattr(idx, "gedge_wire", None)
        if (idx.gedge_a is None or not wire
                or len(wire) != len(idx.gedge_groups)):
            return None
        kept = getattr(idx, "_linework_rows", None)
        if kept is None or kept[0] is not idx.gedge_gi:
            rows = np.flatnonzero(np.asarray(wire, bool)[idx.gedge_gi])
            sel = getattr(idx, "gedge_sel", None)
            if sel is not None:
                rows = rows[sel[rows]]
            kept = idx._linework_rows = (idx.gedge_gi, rows)
        rows = kept[1]
        if not len(rows):
            return None
        proj = self._gedge_screen()
        if proj is None:
            return None
        ax, ay, bx, by, ok = (p[rows] for p in proj)
        rows, ax, ay, bx, by = rows[ok], ax[ok], ay[ok], bx[ok], by[ok]
        return (idx, rows, ax, ay, bx, by) if len(rows) else None

    def pick_linework_groups(self, screen_x: float, screen_y: float):
        """``[(group, pixels)]``, nearest first: the objects made of lines
        only whose visible lines pass within the pick threshold of the
        cursor — what a click selects there before the face under the
        cursor, as a lone loose line would be (a hatch or a set of
        lines grouped on a slab had no hitbox but its 1-px lines, and the
        slab took the click). ``group`` is what a click selects in the
        open context."""
        import numpy as np
        found = self._linework_edge_rows(screen_x, screen_y)
        if found is None:
            return []
        idx, rows, ax, ay, bx, by = found
        dx, dy = bx - ax, by - ay
        l2 = dx * dx + dy * dy
        safe = np.where(l2 > 1e-12, l2, 1.0)
        t = np.clip(((screen_x - ax) * dx + (screen_y - ay) * dy) / safe,
                    0.0, 1.0)
        d = np.hypot(ax + t * dx - screen_x, ay + t * dy - screen_y)
        near = np.flatnonzero(d < self.pick_threshold_px)
        out, seen = [], set()
        for k in near[np.argsort(d[near])][:64]:
            i = int(rows[k])
            owner = idx.gedge_groups[int(idx.gedge_gi[i])]
            if id(owner) in seen or not self.scene.entity_selectable(owner):
                continue
            a = QVector3D(*idx.gedge_a[i])
            p = a + (QVector3D(*idx.gedge_b[i]) - a) * float(t[k])
            if self.linework_hidden(p):
                continue
            seen.add(id(owner))
            out.append((owner, float(d[k])))
        return out

    def linework_groups_around(self, screen_x: float, screen_y: float):
        """The objects made of lines only whose screen box holds the
        cursor, smallest box first: what a click between the lines of a
        hatch may select (SelectTool takes one when nothing else is under
        the cursor, and offers them on a repeated click)."""
        import numpy as np
        found = self._linework_edge_rows(screen_x, screen_y)
        if found is None:
            return []
        idx, rows, ax, ay, bx, by = found
        return _boxes_around(idx.gedge_gi[rows], np.minimum(ax, bx),
                             np.minimum(ay, by), np.maximum(ax, bx),
                             np.maximum(ay, by), screen_x, screen_y,
                             lambda g: idx.gedge_groups[g], self.scene)

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
        import numpy as np
        idx = self._pick_index(near="skip")
        if idx.edge_a is None:
            return None
        ax, ay, oka = self._project_px(idx.edge_a)
        bx, by, okb = self._project_px(idx.edge_b)
        allowed = (idx.edge_sel if which == "sel"
                   else getattr(idx, "edge_snap", idx.edge_sel))
        ok = oka & okb & allowed
        soft = getattr(idx, "edge_soft", None)
        if (visible_only and soft is not None
                and not getattr(self.scene, "show_hidden_geometry", False)):
            ok &= ~soft
        sp = _active_cut(self.scene)
        if sp is not None:
            n = np.array([sp.normal.x(), sp.normal.y(), sp.normal.z()])
            c = float(n @ [sp.point.x(), sp.point.y(), sp.point.z()])
            da = idx.edge_a @ n - c
            db = idx.edge_b @ n - c
            ok &= ~((da > 1e-6) & (db > 1e-6))
        if not ok.any():
            return None
        dx, dy = bx - ax, by - ay
        l2 = dx * dx + dy * dy
        safe = np.where(l2 > 1e-12, l2, 1.0)
        t = np.clip(((screen_x - ax) * dx + (screen_y - ay) * dy) / safe,
                    0.0, 1.0)
        d = np.hypot(ax + t * dx - screen_x, ay + t * dy - screen_y)
        d = np.where(ok, d, np.inf)
        if not visible_only:
            i = int(np.argmin(d))
            return ((idx.edges[i], float(d[i]))
                    if d[i] < self.pick_threshold_px else None)
        hidden = hidden or self._is_occluded
        # Nearest first, skipping the ones hidden behind the model; a few
        # tries are plenty (the threshold holds only a handful).
        for i in np.argsort(d)[:8]:
            i = int(i)
            if not d[i] < self.pick_threshold_px:
                return None
            e = idx.edges[i]
            p = self.edge_point_under_cursor(e, screen_x, screen_y)
            if p is None or not hidden(p):
                return e, float(d[i])
        return None

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
        from PySide6.QtGui import QPaintDevice
        fm = (QFontMetrics(font, self) if isinstance(self, QPaintDevice)
              else QFontMetrics(font))        # the test stubs bind this
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
        """Screen-projected endpoints of every group hard edge —
        ``(ax, ay, bx, by, ok)`` arrays, cached until the scene or the
        camera moves. Shared by pick_group's edge fallback and the snap
        prefilter (during a drawing hover the camera is still, so the two
        big projections run once, not per mouse move)."""
        import numpy as np
        idx = self._pick_index(near="skip")
        if idx.gedge_a is None or not len(idx.gedge_a):
            return None
        M = self._np_mvp()
        # The rest-of-model mode changes WHICH edges the index holds
        # (hidden surroundings leave it), so it keys the projection too.
        key = (id(idx.gedge_a), id(self.scene.mesh), M.tobytes(),
               self.width(), self.height(),
               getattr(self, "_edit_rest_mode", None))
        cached = getattr(self, "_gedge_px_cache", None)
        # The array itself, not only its id(): a rebuilt index's edges can
        # be given the address of the ones they replaced.
        if (cached is not None and cached[0] == key and len(cached) > 2
                and cached[2] is idx.gedge_a):
            return cached[1]
        # A hover that brings new lazy placements in rebuilds the index:
        # the fixed part's edges lead it unchanged and each lazy chunk
        # keeps its edge array, so only what is new to this view is
        # projected (all 1.6 M edges, four times a mouse move with Line
        # beside the plant of #158).
        fx = getattr(idx, "gedge_fixed", None)
        head = getattr(self, "_gedge_px_head", None)
        vkey = key[1:]
        if fx is None or fx[1] + sum(map(len, fx[2])) != len(idx.gedge_a):
            ax, ay, oka = self._project_px(idx.gedge_a)
            bx, by, okb = self._project_px(idx.gedge_b)
            data = (ax, ay, bx, by, oka & okb)
        else:
            n = fx[1]
            if (head is None or head[0] != vkey or head[1] is not fx[0]
                    or head[2] != n):
                ax, ay, oka = self._project_px(idx.gedge_a[:n])
                bx, by, okb = self._project_px(idx.gedge_b[:n])
                head = self._gedge_px_head = (vkey, fx[0], n,
                                              (ax, ay, bx, by, oka & okb))
            memo = getattr(self, "_gedge_px_pieces", None)
            if memo is None or memo[0] != vkey:
                memo = self._gedge_px_pieces = (vkey, {})
            pieces, todo = memo[1], []
            for ge in fx[2]:
                hit = pieces.get(id(ge))
                if hit is None or hit[0] is not ge:
                    todo.append(ge)
            if todo:
                # The new chunks in one projection, split back after.
                flat = np.concatenate([ge.reshape(-1, 3) for ge in todo])
                x, y, ok = self._project_px(flat)
                o = 0
                for ge in todo:
                    m = 2 * len(ge)
                    sx, sy, sk = x[o:o + m], y[o:o + m], ok[o:o + m]
                    pieces[id(ge)] = (ge, (sx[0::2], sy[0::2], sx[1::2],
                                           sy[1::2], sk[0::2] & sk[1::2]))
                    o += m
            parts = [head[3]] + [pieces[id(ge)][1] for ge in fx[2]]
            data = tuple(np.concatenate([p[k] for p in parts])
                         for k in range(5))
        self._gedge_px_cache = (key, data, idx.gedge_a)
        return data

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
        """Screen distance from the cursor to every group hard edge, shared
        by the hovered-edge pick and the snap prefilter (both used to
        compute it); see ``GEDGE_NEAR_PX`` for how far it is exact.

        On a 330 k-face city the full pass was ~30 ms of every mouse move
        for the few dozen edges that matter. A camera that stays put for a
        second hover gets the edges binned into a screen grid (once — the
        projection's own lifetime), and each move then measures the cells
        around the cursor: 0.2 ms zoomed onto a building, ~2 ms with the
        whole city in view (11 edges per pixel: the near radius already
        holds hundreds). The grid lives as long as the projection: the
        first hover of a projection keeps the full pass, so one that
        changes every event (the camera moving) never pays for a grid it
        would use once. City-L, the interaction benchmark: hover with Line
        103 -> 60 ms, a Move drag 101 -> 49 ms per event (median)."""
        import numpy as np
        from core.edge_grid import EdgeGrid, segment_distances
        proj = self._gedge_screen()
        # Whole pixels: the hovered-edge pick gets the float position and
        # the snap scene the rounded one — the same pass must serve both.
        key = (id(proj), int(round(px)), int(round(py)))
        cached = getattr(self, "_gedge_dist_cache", None)
        if (cached is not None and cached[0] == key and len(cached) > 2
                and cached[2] is proj):
            return cached[1]
        ax, ay, bx, by, ok = proj
        d = None
        if (not _NO_SNAP_FAST and len(ax) >= DocumentInteraction.GEDGE_GRID_MIN
                and getattr(self, "pick_threshold_px", 0.0)
                <= DocumentInteraction.GEDGE_REACH_PX):
            # (projection, grid or None): the grid waits for the second hover.
            slot = getattr(self, "_gedge_grid_cache", None)
            if slot is None or slot[0] is not proj:
                self._gedge_grid_cache = (proj, None)
            else:
                grid = slot[1]
                if grid is None:
                    grid = EdgeGrid(ax, ay, bx, by, ok, self.width(),
                                    self.height(), DocumentInteraction.GEDGE_REACH_PX)
                    self._gedge_grid_cache = (proj, grid)
                near_px = max(DocumentInteraction.GEDGE_NEAR_PX,
                              getattr(self, "pick_threshold_px", 0.0))
                for reach in (near_px, DocumentInteraction.GEDGE_REACH_PX):
                    ids = grid.near(px, py, reach)
                    if ids is None:
                        break
                    sub = segment_distances(ax, ay, bx, by, ok, px, py, ids)
                    if (reach >= DocumentInteraction.GEDGE_REACH_PX
                            or int((sub < reach).sum()) >= DocumentInteraction.GEDGE_ENOUGH):
                        d = np.full(len(ax), np.inf)
                        d[ids] = sub
                        break
        if d is None:
            d = segment_distances(ax, ay, bx, by, ok, px, py)
        self._gedge_dist_cache = (key, d, proj)
        return d

    def _ledge_screen(self):
        """Screen-projected endpoints of every loose edge — the
        ``_gedge_screen`` twin for the snap prefilter after an explode
        leaves a big loose mesh. While a context is open the rest of the
        model's loose edges ride along (see ``_outer_loose_meshes``); their
        ``edge_sel`` is False, so picks and the box leave them alone.
        Cached until the scene/camera moves."""
        idx = self._pick_index(near="skip")
        if idx.edge_a is None or not len(idx.edge_a):
            return None
        M = self._np_mvp()
        # Whether the rest of the model's loose edges are in the index
        # changes with «Hide rest of model», which bumps no scene version.
        key = (_content_ver(self), id(self.scene.mesh), M.tobytes(),
               self.width(), self.height(),
               bool(getattr(self, "_rest_is_hidden", lambda: False)()))
        cached = getattr(self, "_ledge_px_cache", None)
        if cached is not None and cached[0] == key:
            return cached[1]
        ax, ay, oka = self._project_px(idx.edge_a)
        bx, by, okb = self._project_px(idx.edge_b)
        data = (ax, ay, bx, by, oka & okb)
        self._ledge_px_cache = (key, data)
        return data

    def _nearby_loose_edges(self, px: float, py: float,
                            radius_px: float = 64.0, cap: int = 400) -> list:
        """The loose edges whose screen segment passes near the cursor —
        real Edge objects, nearest first, at most ``cap``."""
        proj = self._ledge_screen()
        if proj is None:
            return []
        import numpy as np
        ax, ay, bx, by, ok = proj
        if not ok.any():
            return []
        dx, dy = bx - ax, by - ay
        l2 = dx * dx + dy * dy
        safe = np.where(l2 > 1e-12, l2, 1.0)
        t = np.clip(((px - ax) * dx + (py - ay) * dy) / safe, 0.0, 1.0)
        d = np.hypot(ax + t * dx - px, ay + t * dy - py)
        d = np.where(ok, d, np.inf)
        cand = np.where(d < radius_px)[0]
        if len(cand) > cap:
            cand = cand[np.argsort(d[cand])[:cap]]
        idx = self._pick_index(near="skip")
        edges = idx.edges
        n = len(edges)
        return [edges[int(i)] for i in cand if int(i) < n]

    def _nearby_group_edges(self, px: float, py: float,
                            radius_px: float = 48.0, cap: int = 48) -> list:
        """Group hard edges whose screen-space segment passes within
        ``radius_px`` of the cursor, as :class:`_SnapEdge` pseudo-edges —
        at most ``cap``, nearest first. Vectorised prefilter:
        ``compute_snap`` walks its edge list in Python several times per
        hover, so feeding it ALL 160k edges of an import would freeze every
        mouse move; the ~dozens near the cursor cover the point/edge snaps
        the user can actually see."""
        # The placements near THIS pixel enter the index first (lazy mode,
        # #158): the projection below reads the index as it stands, and the
        # edges are read from that same one. Asking again bakes more and
        # reorders the lazy part: the measured positions then pointed at
        # other edges, or past the end (the SPR, hovering with Line).
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        if (nat is not None and radius_px <= DocumentInteraction.GEDGE_REACH_PX
                and cap <= DocumentInteraction.GEDGE_ENOUGH):
            return self._nearby_edges_unbaked(px, py, radius_px, cap, nat)
        idx = self._pick_index(near=("snap", px, py))
        proj = self._gedge_screen()
        if proj is None:
            return []
        import numpy as np
        if not proj[4].any():
            return []
        dist = getattr(self, "_gedge_dist", None)          # stub VPs in tests
        if radius_px > DocumentInteraction.GEDGE_REACH_PX or cap > DocumentInteraction.GEDGE_ENOUGH:
            # Past what _gedge_dist promises (GEDGE_NEAR_PX): every edge.
            from core.edge_grid import segment_distances
            d = segment_distances(*proj, px, py)
        else:
            d = (dist(px, py) if dist is not None
                 else DocumentInteraction._gedge_dist(self, px, py))
        cand = np.where(d < radius_px)[0]
        if len(cand) > cap:
            # The nearest ``cap`` first, then those in order: sorting all
            # of them was 90 ms a move with the whole plant of #158 in view
            # (half a million edges inside the radius).
            dc = d[cand]
            if len(cand) > 4 * cap:
                part = np.argpartition(dc, cap - 1)[:cap]
                cand, dc = cand[part], dc[part]
            cand = cand[np.lexsort((cand, dc))[:cap]]
        return [_group_pseudo_edge(idx, int(i)) for i in cand]

    def _nearby_edges_unbaked(self, px, py, radius, cap, nat,
                              with_d=False) -> list:
        """``_nearby_group_edges`` with the lazy placements' edges from the
        native engine, none of them baked (#158): the fixed placements'
        edges as the index holds them, those of the placements the snap's
        footprint takes in as their bakes would hold them — the nearest
        ``cap`` within ``radius`` px, nearest first (with their distances
        as ``(d, edge)`` when ``with_d``).

        Within one input event the cursor's own edge (``pick_edge_any``:
        the nearest under the pick threshold) and the snap's (the nearest
        48 within 48 px) are both read off one search of the wider: by
        distance, then fixed before lazy, then index or (candidate, edge),
        the order is total, so the nearest ``cap`` within ``radius`` are
        those of the wider within it. Over the plant of #158 each search
        was ~10 ms, twice a pointer move."""
        wide, ncap = DocumentInteraction.GEDGE_SHARED_PX, DocumentInteraction.GEDGE_ENOUGH
        serial = getattr(self, "_input_serial", None)
        if (not _EDGES_SHARED or serial is None or not radius <= wide
                or not 0 < cap <= ncap):
            out = self._nearby_edges_found(px, py, radius, cap, nat)
        else:
            key = (serial, float(px), float(py), id(nat[0]), id(nat[1]),
                   self._placements_epoch(),
                   getattr(self.scene, "version", None),
                   tuple(self._np_mvp().ravel()), self.width(),
                   self.height(), id(self._pick_index(near="skip")),
                   id(self._gedge_screen()),
                   id(getattr(self, "_depth_snap", None)))
            memo = getattr(self, "_edges_near_shared", None)
            if memo is None or memo[0] != key:
                memo = self._edges_near_shared = (key, self._nearby_edges_found(
                    px, py, wide, ncap, nat))
            out = [(d, _copy_snap_edge(e)) for d, e in memo[1]
                   if d < radius][:cap]
        return out if with_d else [e for _d, e in out]

    def _nearby_edges_found(self, px, py, radius, cap, nat) -> list:
        """_nearby_edges_unbaked's search: ``(d, edge)``, nearest first."""
        import numpy as np
        iset, snap, _vis = nat
        idx = self._pick_index(near="skip")
        fx = getattr(idx, "gedge_fixed", None)
        m = fx[1] if fx is not None else 0
        out = []                           # (d, edge)
        if m and self._gedge_screen() is not None:
            d = self._gedge_dist(px, py)[:m]
            cand = _nearest_within(d, radius, cap)
            out += [(float(d[i]), _group_pseudo_edge(idx, int(i)))
                    for i in cand]
        table = self._pick_lazy_table()
        rows = self._footprint_need(table, float(px), float(py), radius)
        rows = rows[np.isin(rows, snap)] if len(rows) else rows
        if len(rows):
            M = self._np_mvp()
            pl, _e, a, b, d = iset.edges_near(
                np.ascontiguousarray(rows, np.int64),
                np.ascontiguousarray(M.T).ravel().tolist(),
                float(self.width()), float(self.height()), float(px),
                float(py), float(radius), int(cap))
            groups = table[0]
            for i, aa, bb, dd in zip(pl.tolist(), a.tolist(), b.tolist(),
                                     d.tolist()):
                owner = self._owner_of(groups[i])
                out.append((dd, _SnapEdge(
                    QVector3D(*aa), QVector3D(*bb), in_group=True,
                    context=_snap_context_of(self, groups[i]), group=owner)))
        out.sort(key=lambda de: de[0])        # stable: the fixed ones first
        return out[:cap]

    def _pick_group_edge_unbaked(self, px, py, nat):
        """pick_group's edge fallback with the lazy placements' edges from
        the engine, none baked (#158): ``(d, owner)`` of the nearest group
        edge within the pick threshold that a pick may land on and the
        user can see, or ``(threshold, None)``."""
        import numpy as np
        best = self.pick_threshold_px
        idx = self._pick_index(near="skip")
        cands = []                         # (d, a, b, t, owner)
        fx = getattr(idx, "gedge_fixed", None)
        m = fx[1] if fx is not None else 0
        proj = self._gedge_screen() if m else None
        if proj is not None:
            ax, ay, bx, by, ok = (v[:m] for v in proj)
            dx, dy = bx - ax, by - ay
            l2 = dx * dx + dy * dy
            safe = np.where(l2 > 1e-12, l2, 1.0)
            t = np.clip(((px - ax) * dx + (py - ay) * dy) / safe, 0.0, 1.0)
            d = np.hypot(ax + t * dx - px, ay + t * dy - py)
            sel = getattr(idx, "gedge_sel", None)
            if sel is not None:
                ok = ok & sel[:m]
            d = np.where(ok, d, np.inf)
            near = np.where(d < best)[0]
            for i in near[np.argsort(d[near], kind="stable")]:
                cands.append((float(d[i]), QVector3D(*idx.gedge_a[i]),
                              QVector3D(*idx.gedge_b[i]), float(t[i]),
                              idx.gedge_groups[int(idx.gedge_gi[i])]))
        table = self._pick_lazy_table()
        rows = self._footprint_need(table, float(px), float(py), best)
        if len(rows):
            rows = rows[np.isin(rows, DocumentInteraction._lazy_sel_rows(self, nat))]
        if len(rows):
            M = self._np_mvp()
            pl, _e, a, b, d = nat[0].edges_near(
                np.ascontiguousarray(rows, np.int64),
                np.ascontiguousarray(M.T).ravel().tolist(),
                float(self.width()), float(self.height()), float(px),
                float(py), float(best), int(DocumentInteraction.GEDGE_ENOUGH))
            if len(pl):
                # Where along each the cursor is, as the index's
                # projection of it reads it.
                ex, ey, _ok = self._project_px(
                    np.concatenate([np.asarray(a, np.float64),
                                    np.asarray(b, np.float64)]))
                n = len(pl)
                ax, ay, bx, by = ex[:n], ey[:n], ex[n:], ey[n:]
                dx, dy = bx - ax, by - ay
                l2 = dx * dx + dy * dy
                safe = np.where(l2 > 1e-12, l2, 1.0)
                t = np.clip(((px - ax) * dx + (py - ay) * dy) / safe,
                            0.0, 1.0)
                groups = table[0]
                for k, i in enumerate(pl.tolist()):
                    cands.append((float(d[k]), QVector3D(*a[k]),
                                  QVector3D(*b[k]), float(t[k]),
                                  self._owner_of(groups[i])))
        cands.sort(key=lambda c: c[0])         # stable: the fixed ones first
        for dd, a, b, t, owner in cands:
            if not self._is_occluded(a + (b - a) * t):
                return dd, owner
        return best, None

    def pick_edge_any(self, screen_x: float, screen_y: float):
        """The edge under the cursor, loose OR a group's (as a world
        pseudo-edge with ``in_group``), nearest first — what the linear
        inferences hover: parallel / perpendicular to a component's edge,
        the reference lock, Push/Pull level with it (groups are read
        from outside without opening them).

        The loose half is picked with the SNAP rule, not the click one: the
        rest of the model drawn around an open group / component is a
        reference too, even though a click cannot select it."""
        loose = self._pick_edge(screen_x, screen_y, visible_only=False,
                                which="snap")
        nat = (self._lazy_native() if hasattr(self, "_lazy_native")
               else None)
        if nat is not None:
            # The nearest group edge under the threshold, fixed or lazy
            # (see _nearby_edges_unbaked), against the loose one as below.
            near = self._nearby_edges_unbaked(screen_x, screen_y,
                                              self.pick_threshold_px, 1, nat,
                                              with_d=True)
            if not near:
                return loose
            dg, g = near[0]
            if loose is not None:
                pa = self._world_to_pixel(loose.a)
                pb = self._world_to_pixel(loose.b)
                if pa is not None and pb is not None:
                    from core.snap import _closest_on_segment_2d
                    dl, _t = _closest_on_segment_2d((screen_x, screen_y),
                                                    pa, pb)
                    if dl <= dg:
                        return loose
            return g
        # The index this pixel asks for, then its projection: the edge is
        # read from the index it was measured on (see _nearby_group_edges).
        idx = self._pick_index(near=("px", screen_x, screen_y))
        proj = self._gedge_screen()
        if proj is None:
            return loose
        import numpy as np
        ax, ay, bx, by, ok = proj
        if not ok.any():
            return loose
        d = self._gedge_dist(screen_x, screen_y)
        i = int(np.argmin(d))
        if d[i] >= self.pick_threshold_px:
            return loose
        if loose is not None:
            pa = self._world_to_pixel(loose.a)
            pb = self._world_to_pixel(loose.b)
            if pa is not None and pb is not None:
                from core.snap import _closest_on_segment_2d
                dl, _t = _closest_on_segment_2d((screen_x, screen_y), pa, pb)
                if dl <= d[i]:
                    return loose
        return _group_pseudo_edge(idx, i)

    #: How close (px) the cursor must come to a component's origin or an
    #: arc's midpoint for it to enter the snap scene at all.
    POINT_INFERENCE_PX = 40.0

    def _component_origin_points(self, px: float, py: float) -> list:
        """The "Component Origin Point": each placed group's own
        origin (its insertion point), as a degenerate pseudo-edge when it
        lies near the cursor. One projection per placement."""
        import numpy as np
        cache = getattr(self, "_corigin_cache", None)
        # The content's version: a view-only bump (a hover's highlight, a
        # click) moves no origin — 21 000 of them re-mapped a mouse move.
        key = (_content_ver(self), id(self.scene.edit_group))
        if cache is None or cache[0] != key:
            groups, pts = [], []
            for g in self._placements():
                xf = getattr(g, "xform", None)
                if xf is None or getattr(g, "billboard", False):
                    continue
                o = xf.map(QVector3D(0.0, 0.0, 0.0))
                groups.append(g)
                pts.append((o.x(), o.y(), o.z()))
            cache = (key, groups, np.array(pts, dtype=np.float64).reshape(-1, 3))
            self._corigin_cache = cache
        _k, groups, pts = cache
        if not len(pts):
            return []
        sx, sy, ok = self._project_px(pts)
        d = np.where(ok, np.hypot(sx - px, sy - py), np.inf)
        moving = getattr(self.active_tool, "_group", None)
        out: list = []
        near = np.flatnonzero(d <= self.POINT_INFERENCE_PX)
        cap = DocumentInteraction.GEDGE_ENOUGH
        if len(near) > cap:
            # The nearest ones, as with the group edges: with the whole
            # plant of #158 in view 17 000 origins fell inside the radius
            # and the snap weighed each in Python (~0.1 s a mouse move).
            near = near[np.lexsort((near, d[near]))[:cap + 1]]
        for i in near:
            if groups[i] is moving:
                continue
            if len(out) >= cap:
                break
            o = QVector3D(*pts[i])
            out.append(_SnapEdge(o, QVector3D(o), component_origin=True,
                                 context=_snap_context_of(self, groups[i])))
        return out

    def _arc_midpoints(self, px: float, py: float) -> list:
        """The "Arc Midpoint": the middle of each loose arc's sweep
        (the point on the arc bisecting its two ends — not any facet's
        midpoint). Closed curves (circles) have none. Computed once per
        scene version, offered when near the cursor."""
        cache = getattr(self, "_arc_mid_cache", None)
        # On the content, not the version: a click's selection bump made
        # every placement's midpoints over again.
        ckey = (_content_ver(self), id(self.scene.mesh),
                id(self.scene.edit_group),
                bool(getattr(self, "_rest_is_hidden", lambda: False)()))
        if cache is None or cache[0] != ckey:
            # Per MESH, keyed by its mutation serial: a scene version bump
            # (every frame of a drag) used to walk every prototype's curves
            # again — 31 ms a frame while moving a face (2026-09-14). Now a
            # mesh's midpoints are recomputed only when that mesh changed;
            # the version pass only maps them through the placements.
            by_mesh = getattr(self, "_arc_mid_by_mesh", None)
            if by_mesh is None:
                by_mesh = self._arc_mid_by_mesh = {}
            # Only the meshes met in this pass are kept. Emptying the whole
            # memo past 2 048 meshes emptied it on EVERY pass on a model
            # with more (2 721 on the plant of #158): all of them were
            # worked out again, ~0.5 s of the first hover after an edit.
            seen: dict = {}

            def local_mids(mesh):
                serial = getattr(mesh, "_mut_serial", None)
                hit = seen.get(id(mesh)) or by_mesh.get(id(mesh))
                if hit is None or hit[0] != serial or hit[2] is not mesh:
                    hit = (serial, self._mesh_arc_midpoints(mesh), mesh)
                seen[id(mesh)] = hit
                return hit[1]
            mids = [(m, None) for m in local_mids(self.scene.mesh)]
            # …the root loose mesh drawn around an open context (a reference
            # outside it), in its own world coordinates…
            outer_of = getattr(self, "_outer_loose_meshes", None)
            for om in (outer_of() if outer_of is not None else ()):
                mids += [(m, "outside") for m in local_mids(om)]
            # …and the arcs inside every placed group / component, through
            # each placement's matrix.
            for g in self._placements():
                mesh = getattr(g, "mesh", None)
                if mesh is None or getattr(g, "billboard", False):
                    continue
                lm = local_mids(mesh)
                if not lm:
                    continue
                xf = getattr(g, "xform", None)
                ctx = _snap_context_of(self, g)
                mids += [(xf.map(m) if xf is not None else QVector3D(m), ctx)
                         for m in lm]
            self._arc_mid_by_mesh = seen
            cache = (ckey, mids)
            self._arc_mid_cache = cache
        out: list = []
        for m, ctx in cache[1]:
            pc = self._world_to_pixel(m)
            if pc is None or math.hypot(pc[0] - px, pc[1] - py) > self.POINT_INFERENCE_PX:
                continue
            out.append(_SnapEdge(QVector3D(m), QVector3D(m),
                                 arc_midpoint=True, context=ctx))
        return out

    @staticmethod
    def _mesh_arc_midpoints(mesh) -> list:
        """The sweep midpoint of every open curve chain of ``mesh`` (in the
        mesh's own space)."""
        from core.snap import arc_sweep_midpoint
        from core.lazy_mesh import plan_of
        plan = plan_of(mesh)
        by_curve: dict = {}
        if plan is not None:
            # A lazy mesh: its curves from the planned edges, the same
            # chains in the same order, nothing built.
            # (only the curve edges listed: listing all of them was
            # most of the first hover over the plant of #158)
            curved = _plan_curved_edges(plan)
            ab = plan.edges[curved].tolist()
            for k, i in enumerate(curved.tolist()):
                by_curve.setdefault(plan.curve[i], []).append(ab[k])
        else:
            for e in mesh.edges:
                cid = getattr(e, "curve", None)
                if cid is not None:
                    by_curve.setdefault(cid, []).append(e)
        mids = []
        for edges in by_curve.values():
            degree: dict = {}
            pos: dict = {}
            for e in edges:
                if plan is not None:
                    for v in e:
                        degree[v] = degree.get(v, 0) + 1
                        if v not in pos:
                            pos[v] = QVector3D(*plan.pos[v])
                    continue
                for v in (e.v0, e.v1):
                    degree[element_identity(v)] = degree.get(element_identity(v), 0) + 1
                    pos[element_identity(v)] = v.position
            ends = [pos[k] for k, n in degree.items() if n == 1]
            if len(ends) != 2 or len(pos) < 3:
                continue                     # a circle, or not a chain
            mid = arc_sweep_midpoint(list(pos.values()))
            if mid is not None:
                mids.append(mid)
        return mids

    def _billboard_placements(self) -> list:
        """The face-me placements of ``_placements()``, kept while it is:
        walking the 21 406 placements of #158 for the two of them was
        ~2 ms of every pointer move."""
        groups = self._placements()
        epoch_of = getattr(self, "_placements_epoch", None)
        if epoch_of is None:                         # stub viewports
            return [g for g in groups if getattr(g, "billboard", False)]
        key = (epoch_of(), id(groups), len(groups))
        memo = getattr(self, "_billboard_memo", None)
        if memo is None or memo[0] != key or memo[1] is not groups:
            memo = self._billboard_memo = (
                key, groups,
                [g for g in groups if getattr(g, "billboard", False)])
        return memo[2]

    def _billboard_snap_edges(self) -> list:
        """The one snap a face-me billboard offers: its FEET, the anchor
        point, as a degenerate pseudo-edge (a point, like a guide point) —
        enough to place a figure or measure from where it stands. It used
        to offer its base edge and vertical axis too (corners, midpoints,
        head, and two whole lines to ride), and the scale figure beside the
        origin stole the inference from the drawing around it («me está
        cogiendo el paisano», Rafael, revision 4; Marco: «siempre toma
        protagonismo»). The group a transform tool is currently dragging is
        excluded (it would snap to itself)."""
        moving = getattr(self.active_tool, "_group", None)
        out: list = []
        for g in self._billboard_placements():
            if g is moving:
                continue
            if not self.scene.group_snappable(g):
                continue
            quad = self._billboard_quad(g)
            if quad is None:
                continue
            c = quad[0]
            feet = (c[0] + c[1]) * 0.5
            out.append(_SnapEdge(QVector3D(feet), QVector3D(feet), figure=True))
        return out

    def _snap_scene(self, px: Optional[float] = None,
                    py: Optional[float] = None):
        """The scene the snap engine sees: loose edges, construction guides as
        pseudo-edges (``Guide.a/.b`` span the long segment), and — when the
        cursor position is given — the group edges near it plus the face-me
        billboard anchors, so drawing and dimensioning over an imported
        reference model snaps to its corners and edges.

        Loose edges the scene itself says are not there (hidden, or on a
        hidden layer) are left out — see :func:`_snap_dropped_loose`."""
        # Guide lines are clipped to the part in front of the camera so the
        # snap engine gets projectable endpoints (an endpoint behind the eye
        # used to kill on-line snapping in ordinary 3D views); guide POINTS
        # snap as degenerate pseudo-edges, exactly like survey points.
        lines = []
        circles = []
        # Guides that Move is dragging pass through the cursor: left out,
        # or the cursor locks onto its own guide (#627).
        moving = getattr(self.active_tool, "moving_guide_ids", None)
        moving = moving() if callable(moving) else ()
        for g in shown_guides(self.scene):
            if id(g) in moving:
                continue
            if getattr(g, "is_circle", False):
                circles.append(g)   # rim is resolved in compute_snap; centre below
                continue
            if g.is_line:
                seg = self._guide_snap_segment(g)
                if seg is not None:
                    lines.append(_SnapEdge(*seg, guide=True))
            else:
                lines.append(_SnapEdge(QVector3D(g.point), QVector3D(g.point)))
        # Reference-image borders snap like guides: aligning a scan against
        # real geometry (or drawing to its edge) is what tracing needs.
        for im in getattr(self.scene, "image_planes", None) or []:
            if getattr(im, "locked", False) or not self.scene.entity_selectable(im):
                continue
            for a, b in im.border_edges():
                seg = self._clip_segment_front(a, b)
                if seg is not None:
                    lines.append(_SnapEdge(*seg))
        near = self._nearby_group_edges(px, py) if px is not None else []
        # What a transform tool is dragging must not attract snaps (the
        # entities in motion stay out of inference; issue #19): the
        # tool names the edges/groups in motion and they are dropped here.
        excl = getattr(self.active_tool, "snap_excluded", None)
        excl = excl() if callable(excl) else None
        if excl is not None and excl[1]:
            near = [e for e in near
                    if id(getattr(e, "group", None)) not in excl[1]]
        if px is not None:
            near += self._billboard_snap_edges()
            origins = getattr(self, "_component_origin_points", None)   # stub VPs in tests
            if origins is not None:
                near += origins(px, py)
            arcs = getattr(self, "_arc_midpoints", None)
            if arcs is not None:
                near += arcs(px, py)
        box_pts = self._selection_box_points()
        if excl is not None and excl[1]:
            # A selected group's box corners are the group itself: Scale
            # dragging it must not land its grip on its own box (#233).
            box_pts = self._selection_box_points(skip=excl[1])
        near += box_pts
        valid = getattr(self, "_valid_center_ref", None)   # stub VPs in tests
        ref = valid() if callable(valid) else None
        if ref is not None:
            # No dot, no snap: a circle too small on screen keeps quiet.
            pc = self._world_to_pixel(ref[0])
            hint = getattr(self, "_center_hint_px", None)
            if pc is not None and (hint is None or hint(ref, pc) > 0.0):
                near.append(_SnapEdge(QVector3D(ref[0]), QVector3D(ref[0]),
                                      center=True))
        # 10/Oct/26 added BastelBaus: add snapping to dimension edges.
        # Added before the early return below (a scene with only dimensions
        # must still snap to them) and before the cut filter (a dimension
        # the section hides must not attract snaps).
        for dim in getattr(self.scene, "dimensions", None) or []:
            if not self.scene.entity_selectable(dim):
                continue
            near.append(_SnapEdge(*dim.line_points()))
        # A guide circle's centre is a permanent snap target, like the centre
        # of a visited mesh circle (the rim is resolved in compute_snap).
        for c in circles:
            near.append(_SnapEdge(QVector3D(c.center), QVector3D(c.center),
                                  center=True))
        sp = _active_cut(self.scene)
        if sp is not None:
            # What the cut hides must not attract snaps: drop
            # edges whose BOTH endpoints are on the hidden side.
            def _kept(e):
                return not (sp.side(e.a) > 1e-6 and sp.side(e.b) > 1e-6)
            near = [e for e in near if _kept(e)]
        # Survey points snap as degenerate pseudo-edges: the endpoint snap
        # lands EXACTLY on the surveyed coordinate (the municipal flow's whole
        # point); direction-based inferences skip zero-length edges safely.
        for gp in getattr(self.scene, "geo_points", None) or []:
            near.append(_SnapEdge(QVector3D(gp.position),
                                  QVector3D(gp.position)))
        # The rest of the model's loose geometry — the root mesh drawn faded
        # around an open group / component — is a reference too.
        # Its edges are filtered one by one (each carries its own layer and
        # hidden flag), and «Hide rest of model» leaves them out already.
        outer_of = getattr(self, "_outer_loose_meshes", None)
        outer = []
        for om in (outer_of() if outer_of is not None else ()):
            outer += [e for e in om.edges if self.scene.edge_snappable(e)]
        big = (px is not None
               and (len(self.scene.edges) + len(outer)) > _LOOSE_SNAP_CAP)
        if not lines and not near and not circles and sp is None \
                and not big and not outer and excl is None \
                and not _snap_dropped_loose(self):
            return self.scene
        if big:
            # Only the edges near the cursor are considered, so ask each one
            # (dozens) rather than walking a mesh of 160k to build the drop
            # set above. The index holds the rest of the model's loose
            # edges too, so this reaches them.
            loose = [e for e in self._nearby_loose_edges(px, py)
                     if self.scene.edge_snappable(e)]
        else:
            loose = list(self.scene.edges) + outer
            drops = _snap_dropped_loose(self)
            if drops:
                loose = [e for e in loose if element_identity(e) not in drops]
        if excl is not None and excl[0]:
            loose = [e for e in loose if element_identity(e) not in excl[0]]
        if sp is not None:
            loose = [e for e in loose if _kept(e)]
        if outer:
            # The ScreenTip must say these are outside the open context:
            # a real Edge carries no context of its own.
            outer_ids = {element_identity(e) for e in outer}
            loose = [_SnapEdge(e.a, e.b, context="outside")
                     if element_identity(e) in outer_ids else e for e in loose]
        from types import SimpleNamespace
        return SimpleNamespace(edges=loose + lines + near, circles=circles)

    def _group_obb(self, group):
        """``(frame, lo, hi)`` of the group in its OWN axes, cached on its
        chunk. A world-aligned box on a rotated object reads as skewed and
        wraps far more air than object — and its corners, which are the
        handles you grab, end up nowhere near the thing.

        A CONTAINER's box comes from its children, and its own chunk knows
        nothing about them: cached there, the box outlived a deleted child
        (the frame stayed long after one of two fountains was gone) and
        followed the container's own matrix when entering pushed that
        matrix down into the children — the box slid to the axes while
        the fountain stayed put (Marco, 2026-09-11). So a container's box
        is cached per scene version instead: it is recomputed on any
        change, and ``placement_points`` makes that one matrix multiply
        per placement."""
        kids = getattr(group, "children", None)
        if kids:
            cache = getattr(self, "_container_obb", None)
            if cache is None:
                cache = self._container_obb = {}
            key = _content_ver(self)
            hit = cache.get(id(group))
            if hit is not None and hit[0] == key:
                return hit[1]
            fetch = getattr(self, "_proto_points_cache", None)   # stub VPs in tests
            protos = fetch(group) if fetch is not None else None
            obb = self._compute_obb(group, protos)
            if fetch is not None:
                self._proto_points_store(group, protos)
            if len(cache) > 256:            # ids of groups long gone
                cache.clear()
            cache[id(group)] = (key, obb)
            return obb
        # The box follows the group's axes (issue #44), and Change Axes can
        # turn them without touching one vertex — so the cached box is only
        # good for the frame it was measured on.
        from core.group import group_frame
        fr = group_frame(group)
        fkey = tuple(fr.data()) if fr is not None else None
        xf = getattr(group, "xform", None)
        if xf is not None and id(group) not in (
                getattr(self, "_inst_chunks", None) or ()):
            # A placement nobody has baked to the world: the box is measured
            # on the points, the bake would only have carried it. Selecting
            # the 590 objects of a big model baked them all (#1092). Good for
            # what its bake would have been good for.
            base = self._group_chunk(
                _proto_wrapper(self, group.mesh, _paint_of(self, group)))
            key = (base["uid"], tuple(xf.data()), base.get("rev"), fkey)
            memo = getattr(self, "_inst_obb", None)
            if memo is None:
                memo = self._inst_obb = {}
            hit = memo.get(id(group))
            if hit is not None and hit[0] == key:
                return hit[1]
            obb = self._compute_obb(group)
            if len(memo) > 65536:          # ids of placements long gone
                memo.clear()
            memo[id(group)] = (key, obb)
            return obb
        entry = self._group_chunk(group)
        obb = entry.get("obb")
        if obb is not None and entry.get("obb_frame") == fkey:
            return obb
        obb = entry["obb"] = self._compute_obb(group)
        entry["obb_frame"] = fkey
        return obb

    def _proto_points_cache(self, group) -> dict:
        """``{id(mesh): local points}`` of the prototypes under ``group``
        that are still current. A prototype's LOCAL array only changes
        when its mesh is edited — never when a placement moves — so an
        entry is dropped when the mesh is the one open for editing (or
        under a share-back) or a mutation primitive flagged it
        (``_chunk_dirty``, O(1)). No chunk is built to find out: validating
        against fresh local chunks built one per nested child, 384 ms on
        the first box after entering the plaza."""
        from core.purge import iter_groups
        store = getattr(self, "_proto_pts_store", None)
        if store is None:
            store = self._proto_pts_store = {}
        editing = self._editing_meshes()
        valid: dict = {}
        from core.mesh import DeferredMesh
        for g in iter_groups((group,)):
            mesh = g.mesh
            if mesh is None or id(mesh) in valid:
                continue
            # A lazy mesh still unbuilt was never edited (an edit builds
            # it): its cached points stand, and asking for its vertices
            # would build it -- the box over a big model built them all.
            lazy = type(mesh) is DeferredMesh
            if not lazy and not mesh.vertices:
                continue
            hit = store.get(id(mesh))
            if hit is None:
                continue
            serial, arr = hit
            if (id(mesh) in editing
                    or getattr(mesh, "_mut_serial", None) != serial
                    or (not lazy and len(arr) != len(mesh.vertices))):
                del store[id(mesh)]
                continue
            valid[id(mesh)] = arr
        return valid

    def _editing_meshes(self) -> set:
        """ids of the meshes a command can be mutating right now: the open
        context's, and every level's share-back prototype."""
        scene = self.scene
        ids = {id(scene.mesh)}
        for level in getattr(scene, "_edit_stack", None) or ():
            share = level.get("share") if isinstance(level, dict) else None
            if share:
                ids.add(id(share[1]))                  # the prototype
                ids.add(id(share[0].mesh))             # the working copy
        return ids

    def _proto_points_store(self, group, protos: dict) -> None:
        store = self._proto_pts_store
        if len(store) > 512:
            store.clear()
        from core.purge import iter_groups
        editing = self._editing_meshes()
        serials = {id(g.mesh): getattr(g.mesh, "_mut_serial", None)
                   for g in iter_groups((group,)) if g.mesh is not None}
        for mid, arr in protos.items():
            if mid not in editing:
                store[mid] = (serials.get(mid), arr)

    @staticmethod
    def _compute_obb(group, protos=None):
        from core.group import (frame_axes, frame_from_points, group_frame,
                                oriented_bounds, placement_points)
        # The corners come from the POINTS — never from a merged copy of the
        # component (see ``placement_points``).
        pos = placement_points(group, protos)
        frame = group_frame(group)
        if frame is not None:
            # A group that knows its own axes is boxed on them — the
            # selection box and Scale grips follow the object's axes
            # (issue #44), turned however the object is.
            _o, x, y, z = frame_axes(frame)
            return oriented_bounds(None, (x, y, z), points=pos)
        world = oriented_bounds(None, points=pos)         # world axes
        own = oriented_bounds(None, frame_from_points(pos if len(pos) else None),
                              points=pos)

        def volume(b):
            _f, lo, hi = b
            # A flat group has zero volume; rank those by their largest face.
            side = sorted(hi[i] - lo[i] for i in range(3))
            return side[1] * side[2] * max(side[0], 1e-6)

        # Derived axes are meaningless on organic geometry — a hedge has no
        # dominant plane, and its "own" box came out 25% LARGER than the world
        # one. Keep the derived frame only when it earns its place.
        return own if volume(own) < 0.9 * volume(world) else world

    def _update_center_ref(self, x: float, y: float) -> None:
        """Acquire the centre of the circle or arc under the cursor.

        A loose curve edge gives its own curve; a face gives the curves on
        its boundary — a cylinder's lid, a rounded corner, a circular hole
        — placed through the group's matrix when the face is inside one.
        With several on one face, the one whose points come closest to the
        cursor wins. Nothing under the cursor keeps the last centre: you
        hover the rim, then travel to the centre and snap to it."""
        found = None
        edge = self._hover_edge
        if getattr(edge, "curve", None) is not None:
            found = self._center_of_edge(edge, self.scene.mesh)
        elif getattr(edge, "in_group", False) and getattr(edge, "group", None) is not None:
            found = self._center_of_group_edge(edge)
        if found is None:
            pick = (getattr(self, "pick_face_inference", None)
                    or getattr(self, "pick_face_placement", None)
                    or self.pick_face_any)   # stub VPs in tests
            face, group = pick(x, y)
            if face is not None:
                mesh = getattr(group, "mesh", None) if group is not None \
                    else self.scene.mesh
                if mesh is None:
                    mesh = self.scene.mesh
                found = self._center_of_face(face, group, mesh, x, y)
        if found is not None:
            self._center_ref = found
            return found
        self._valid_center_ref()
        return None

    def _center_of_group_edge(self, pseudo):
        """The rim of a circle inside a top-level group / component: the
        world pseudo-edge is matched to the group's own edge (positions in
        its space) and the curve's centre comes back through its matrix."""
        g = pseudo.group
        mesh = getattr(g, "mesh", None)
        xf = getattr(g, "xform", None)
        if mesh is None:
            return None
        inv = None
        if xf is not None:
            inv, ok = xf.inverted()
            if not ok:
                return None
        la = inv.map(pseudo.a) if inv is not None else pseudo.a
        lb = inv.map(pseudo.b) if inv is not None else pseudo.b
        if not _lazy_edge_may_be_curve(mesh, la, lb):
            # A lazy mesh with no curve edge where the lookups below could
            # land: building it (~0.5 s for a big group of the plant of
            # #158, on the hover that crosses into it) would answer None.
            return None
        if not mesh.edges:
            return None
        # The mesh's own registry: two O(1) vertex lookups (tolerant, the
        # weld grid) and the edge between them by incidence. This used to
        # build a dict of EVERY edge of the group, keyed by rounded
        # positions, cached for one group at a time — so a cursor crossing
        # from one group to the next rebuilt it on nearly every move:
        # 9 ms of a 14 ms hover on pileta-fuente (2 500 edges per group).
        edge = None
        if not _NO_SNAP_FAST:
            va, vb = mesh.vertex_at(la), mesh.vertex_at(lb)
            if va is not None and vb is not None:
                edge = mesh.find_edge(va, vb)
        if edge is None:
            # The registry missed (a position past the weld tolerance):
            # the rounded-position table, one per mesh, built once.
            tables = getattr(self, "_group_edge_tables", None)
            if tables is None or tables[0] != self.scene.version:
                tables = self._group_edge_tables = (self.scene.version, {})
            def k(p):
                return (round(p.x(), 4), round(p.y(), 4), round(p.z(), 4))
            table = tables[1].get(id(mesh))
            if table is None:
                table = tables[1][id(mesh)] = {
                    frozenset((k(e.a), k(e.b))): e for e in mesh.edges}
            edge = table.get(frozenset((k(la), k(lb))))
        if edge is None or getattr(edge, "curve", None) is None:
            return None
        found = self._center_of_edge(edge, mesh)
        if found is None:
            return None
        c = xf.map(found[0]) if xf is not None else found[0]
        return (c, found[1], found[2], found[3], found[4], found[5], g)

    def _center_of_edge(self, edge, mesh):
        from core.snap import fit_circle
        pts = {}
        for e in mesh.curve_edges(edge):
            pts[element_identity(e.v0)] = e.v0.position
            pts[element_identity(e.v1)] = e.v1.position
        fit = fit_circle(list(pts.values())) if len(pts) >= 3 else None
        if fit is None:
            return None
        return (fit[0], fit[1], ("edge", edge.curve), self.scene.version,
                edge, mesh, None)

    def _center_of_face(self, face, group, mesh, x, y):
        from core.snap import curve_centers_of_face
        xf = getattr(group, "xform", None) if group is not None else None
        centers = curve_centers_of_face(face, xf)
        if not centers:
            return None
        if len(centers) == 1:
            c, r, key = centers[0]
        else:
            # Several arcs on one face (a rounded slab): the one whose
            # centre projects nearest the cursor.
            best = None
            for c, r, key in centers:
                pc = self._world_to_pixel(c)
                if pc is None:
                    continue
                d = math.hypot(pc[0] - x, pc[1] - y)
                if best is None or d < best[0]:
                    best = (d, c, r, key)
            if best is None:
                return None
            _d, c, r, key = best
        return (c, r, (element_identity(face), key), self.scene.version, face, mesh, group)

    def _center_hint_px(self, ref, pc) -> float:
        """How close (px) the cursor must come for the centre's dot to show:
        a share of the circle's own size on screen, capped — so a fountain
        seen from far, a few pixels across, does not light up whenever the
        cursor wanders over it («estoy lejos y me puse cerca de la pileta y
        aparece ese círculo», Marco, 2026-09-12). Zero when the circle is
        too small on screen to mean anything."""
        rim = self._world_to_pixel(ref[0] + QVector3D(ref[1], 0.0, 0.0))
        rim2 = self._world_to_pixel(ref[0] + QVector3D(0.0, ref[1], 0.0))
        r_px = 0.0
        for q in (rim, rim2):
            if q is not None:
                r_px = max(r_px, math.hypot(q[0] - pc[0], q[1] - pc[1]))
        if r_px < self.CENTER_MIN_RADIUS_PX:
            return 0.0
        return min(self.CENTER_HINT_PX, 0.5 * r_px)

    def _valid_center_ref(self):
        """The acquired centre, re-checked against the scene: once the
        circle is gone the dot goes with it — it lingered after the circle
        was erased («elimino el círculo y el punto verde continúa allí»,
        Marco, 2026-09-11) — and once it moved, the centre follows.
        Costs nothing until the scene changes."""
        ref = getattr(self, "_center_ref", None)
        if ref is None:
            return None
        if ref[3] == self.scene.version:
            return ref
        src, mesh, group = ref[4], ref[5], ref[6]
        fresh = None
        real = getattr(src, "real", None)
        from core.mesh import DeferredMesh
        lazy = real is not None and type(mesh) is DeferredMesh
        if real is not None and not lazy:
            # Found over a lazy mesh's light face, since built: an edit asks
            # of the real one. Still lazy, the mesh has not changed: only the
            # matrix can have, and building it here cost a big group's whole
            # build on the frame after any edit (#1092).
            src = real()
        if isinstance(src, Face):
            if lazy or src in mesh.faces:
                # the same arc of the face, wherever it is now
                from core.snap import curve_centers_of_face
                xf = getattr(group, "xform", None) if group is not None else None
                same = [c for c in curve_centers_of_face(src, xf)
                        if c[2] == ref[2][1]]
                if same:
                    fresh = (same[0][0], same[0][1], ref[2],
                             self.scene.version, src, mesh, group)
        elif src in mesh.edges:
            fresh = self._center_of_edge(src, mesh)
            xf = getattr(group, "xform", None) if group is not None else None
            if fresh is not None and xf is not None:
                fresh = (xf.map(fresh[0]), fresh[1], fresh[2], fresh[3],
                         fresh[4], fresh[5], group)
        self._center_ref = fresh
        return fresh

    def _selection_box_points(self, skip=()) -> list:
        """The corners of a selected group's bounding box, as degenerate
        pseudo-edges so the snap engine offers them as endpoints.

        Those corners are grabbable: with a group selected they are
        what you take hold of to move it somewhere exact, and the green dot
        tells you the grab landed. The box is the group's selection cue here
        too (see ``_sync_edges``), so the corners were already on screen —
        they just were not snap targets. Costs nothing: the chunk's bbox is
        already cached, so a 230k-face group contributes eight points."""
        pts: list = []
        for ent in self.scene.selection:
            if not isinstance(ent, Group) or getattr(ent, "billboard", False):
                continue
            if id(ent) in skip:
                continue
            from core.group import oriented_box_corners
            for p in oriented_box_corners(*self._group_obb(ent)):
                pts.append(_SnapEdge(p, QVector3D(p)))
        return pts

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
        # vectorised slab past 64 spans, ``_ray_aabb`` (with its tolerance
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

    def _apply_tool_cursor(self) -> None:
        """The pointer becomes the active tool's icon; Select and
        unknown tools keep the standard arrow.

        A tool that does something else under a modifier says so with the
        pointer: Paint holds Alt to SAMPLE a face's material instead of
        painting it, and the cursor swaps the bucket for an eyedropper while
        it is down."""
        from PySide6.QtWidgets import QApplication
        from views.icons import tool_cursor
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
        cur = (tool_cursor(icon, plus)
               if self.active_tool is not None else None)
        if cur is None and self.active_tool is not None:
            # A tool without a drawn icon can still ask for a stock Qt
            # cursor (Position Texture shows a hand).
            shape = getattr(self.active_tool, "qt_cursor", None)
            if shape is not None:
                from PySide6.QtGui import QCursor
                cur = QCursor(shape)
        if cur is not None:
            self.setCursor(cur)
        else:
            self.unsetCursor()

    def _apply_nav_cursor(self) -> None:
        """The pointer for a camera nav mode: orbit / pan / the magnifier
        (each navigation tool shows its own icon)."""
        from views.icons import tool_cursor
        if self.nav_mode in ("orbit", "pan", "zoom", "zoom_window"):
            cur = tool_cursor(self.nav_mode)
            if cur is not None:
                self.setCursor(cur)
                return
            self.setCursor(Qt.CrossCursor
                           if self.nav_mode in ("zoom", "zoom_window")
                           else Qt.OpenHandCursor)

    # ---- Copy / paste -------------------------------------------------------
    def copy_selection(self) -> bool:
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

    def cut_selection(self) -> bool:
        """Copy the selection, then erase it (one undoable step)."""
        if not self.copy_selection():
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
        from PySide6.QtCore import QSettings
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
        from PySide6.QtCore import QSettings
        QSettings().setValue("display/edit_rest_fade", strength)
        self.update()

    def set_hide_similar_components(self, on: bool) -> None:
        """Hide Similar Components: the other instances of the
        component being edited leave the frame. Persisted like the rest
        mode; the version bump re-keys the pick index."""
        on = bool(on)
        from PySide6.QtCore import QSettings
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
        """What the pick index takes from the placements: the ids the
        context lets you touch, the fixed placements in order, and the lazy
        ones as ``id → (position, group)`` (see ``_pick_index``)."""
        scene = self.scene
        del_contexto = self._context_placements()
        tocables = {id(g) for g in del_contexto}
        # What the current context lets you TOUCH: the whole scene at the
        # root, the children of the group you are inside otherwise. This
        # used to be `if scene.edit_group is None`, which took EVERY group
        # out of the index while you were inside one — so a group's children
        # were drawn and could not be clicked, and «todo está combinado»
        # (Marco, 2026-09-11).
        # What the snap engine may LAND ON is the whole model. Inside a
        # group the snap still infers against the rest — faded, but every
        # corner and edge of it is a reference. Holding only the context
        # here meant no green dot on the pavement while moving a nested
        # group onto it («me debería salir un punto verde de la referencia»,
        # Marco, 2026-09-11). So the index carries every placement and the
        # context decides SELECTABILITY, not membership: the rest occludes
        # and snaps, and cannot be picked. What the open group itself owns is
        # already here as the loose part. «Hide» is the exception — what is
        # not drawn is not there.
        ctx = scene.edit_group
        if ctx is None or oculto:
            candidatos = del_contexto
        else:
            candidatos = [g for g in self._placements()
                          if g is not ctx and self._owner_of(g) is not ctx]
        lazy_ids = lazy[4] if lazy is not None else ()
        fixed = [g for g in candidatos if id(g) not in lazy_ids]
        lazy_pos = {id(g): (i, g) for i, g in enumerate(candidatos)
                    if id(g) in lazy_ids}
        return tocables, fixed, lazy_pos

    def _context_placements(self):
        """The placements the CURRENT context lets you touch.

        At the root that is the whole scene. Inside a group it is its
        children and their subtrees — never the model around it, which is
        exactly why you cannot grab the rest of the drawing while you are
        inside a group.
        """
        ctx = self.scene.edit_group
        todo = self._placements()
        if ctx is None:
            return todo
        return [g for g in todo
                if getattr(g, "context", None) is ctx
                and getattr(g, "owner", None) is not ctx]

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
        """The model point under pixel ``(x, y)`` -- what the frame on screen
        shows there, else the nearest face the pick ray hits -- or ``None``
        when the cursor is on sky or ground."""
        origin, direction = self._pixel_to_ray(x, y)
        seen = self._depth_world_at(x, y) if hasattr(self, "_depth_world_at") \
            else None
        if seen is not None and seen[0] == "hit":
            return seen[1]
        if origin is not None and direction is not None and seen is None:
            try:
                idx = self._pick_index(near=DocumentInteraction._ray_near(self, x, y))
                import numpy as np
                hits = self._hover_hits(idx, origin, direction)
                if hits is not None and len(hits.t):
                    t = float(np.min(hits.t))
                    if math.isfinite(t) and t > 0.0:
                        return origin + direction * t
            except Exception:                  # noqa: BLE001 - no point then
                pass
        return None


    def _orbit_pivot_at(self, x: float, y: float):
        """Where an orbit gesture that starts at pixel ``(x, y)`` turns (#164).

        1. The model point under the cursor -- the nearest face the pick
           ray hits (loose geometry and groups alike);
        2. else, with the cursor on sky or ground, the middle of the
           stretch of the view's central ray that crosses the model's
           bounding box -- the middle of what is on screen;
        3. else the camera target (an empty scene, a model off screen).
        Worked out once per gesture: the pivot stays put while dragging.
        """
        point = DocumentInteraction._model_point_at(self, x, y)
        if point is not None:
            return point
        lo, hi = self.scene.bounds()
        if lo is not None:
            o, d = self._pixel_to_ray(self.width() / 2.0, self.height() / 2.0)
            if o is not None and d is not None:
                span = _ray_aabb_span(
                    (o.x(), o.y(), o.z()), (d.x(), d.y(), d.z()),
                    (lo.x(), lo.y(), lo.z()), (hi.x(), hi.y(), hi.z()))
                if span is not None:
                    return o + d * ((span[0] + span[1]) * 0.5)
        return QVector3D(self.camera.target)

    def _end_camera_drag(self) -> None:
        """Forget a camera drag in progress (orbit, pan or zoom by drag)."""
        self._last_pos = None
        self._orbit_pivot = None
        self._pan_mode = False
        self._middle_drag_active = False
        if self.nav_mode is not None:
            self._apply_nav_cursor()
        else:
            self._apply_tool_cursor()

    def _set_middle_drag_pan_mode(self, pan: bool) -> None:
        """Switch an active MMB drag between orbit and pan."""
        if not self._middle_drag_active or self.nav_mode is not None:
            return
        pan = bool(pan)
        if self._pan_mode == pan:
            return
        self._pan_mode = pan
        from views.icons import tool_cursor
        cur = tool_cursor("pan" if pan else "orbit")
        if cur is not None:
            self.setCursor(cur)

    def set_nav_mode(self, mode: Optional[str]) -> None:
        """Enter a camera navigation mode ("orbit" / "pan").

        For trackpad users with no middle mouse button: while a nav mode is
        active the left-drag drives the camera (orbit or pan). The active
        drawing tool is suspended with its state (``suspended_tool``):
        picking it again resumes it where it was, a line's first point
        still placed; picking any other tool, or Space (Select), resets it.
        ``None`` clears the nav mode and leaves no tool, as before: the
        suspended one is reset.
        """
        if mode is None and self.suspended_tool is not None:
            suspended, self.suspended_tool = self.suspended_tool, None
            suspended.on_deactivate(self)
        if self.active_tool is not None:
            if mode is not None:
                self.suspended_tool = self.active_tool
                suspend = getattr(self.active_tool, "on_suspend", None)
                if callable(suspend):
                    suspend(self)
            else:
                self.active_tool.on_deactivate(self)
            self.active_tool = None
        self._hover_entity = None
        self.last_snap = None
        self.nav_mode = mode
        self._last_pos = None             # a drag in progress ends here
        self._orbit_pivot = None
        self._pan_mode = False
        self._middle_drag_active = False
        if mode is not None:
            self._apply_nav_cursor()      # orbit / pan / magnifier icons
        else:
            self.unsetCursor()
        self.update()

    def leaveEvent(self, ev) -> None:
        if self._hover_entity is not None:
            self._hover_entity = None
            self.update()
        super().leaveEvent(ev)

    # ---- Input --------------------------------------------------------------
    def _context_pick(self, x: float, y: float):
        """Right-click at ``(x, y)``: select the thing under the cursor
        before the menu opens — unless it is already selected, which keeps
        the selection as it is. Returns that thing (or ``None``).

        «Under the cursor» is what the cursor SHOWS: a group behind a
        selected loose face used to take the right-click and drop the
        selection, and «Make Group» with it (#378).

        Something already selected under the cursor wins outright: loose
        geometry flush on a group (a block standing on it, a face drawn on
        its surface) is coplanar with the group there, and the group took
        the click (#744)."""
        mine = self.pick_edge(x, y) or self.pick_face(x, y)
        if mine is not None and mine in self.scene.selection:
            return mine
        picked = (self.pick_text_label(x, y, rect_only=True)
                  or self.pick_group(x, y) or self.pick_edge(x, y)
                  or self.pick_geopath(x, y) or self.pick_dimension(x, y)
                  or self.pick_text_label(x, y)
                  or self.pick_section_plane(x, y)
                  or self.pick_guide(x, y)
                  or self._face_or_image_in_front(x, y)
                  or self.pick_image_plane(x, y)
                  or self.pick_locked_image_plane(x, y))
        if picked is not None and picked not in self.scene.selection:
            self.scene.select([picked])
            self.update()
        return picked

    def _face_or_image_in_front(self, x: float, y: float):
        face = self.pick_face(x, y)
        return self.image_in_front_of(face, x, y) or face

    # ---- A side-tray tab dropped here: the window moves it to that side ----
    def _dock_drag(self, ev) -> bool:
        win = self.window()
        mime = getattr(win, "DOCK_MIME", None)
        return bool(mime) and ev.mimeData().hasFormat(mime)

    def dragEnterEvent(self, ev) -> None:
        if self._dock_drag(ev):
            ev.acceptProposedAction()
            return
        super().dragEnterEvent(ev)

    def dragMoveEvent(self, ev) -> None:
        if self._dock_drag(ev):
            ev.acceptProposedAction()
            return
        super().dragMoveEvent(ev)

    def dropEvent(self, ev) -> None:
        if self._dock_drag(ev):
            self.window().drop_dock_on_viewport(ev)
            return
        super().dropEvent(ev)

    def contextMenuEvent(self, ev) -> None:
        """Right-click: select what's under the cursor and open
        a context menu of actions relevant to the current selection."""
        win = self.window()
        # A tool with its own right-click menu (Position Texture: Done /
        # Reset / Flip / Rotate) takes the click instead.
        hook = getattr(self.active_tool, "context_menu", None)
        if callable(hook) and hook(self, ev.globalPos()):
            return
        if not hasattr(win, "show_viewport_context_menu"):
            return
        x, y = ev.pos().x(), ev.pos().y()
        # Where the menu was opened, for entries that act at the cursor
        # (Texture ▸ Position puts its pins on the tile under it).
        self._context_pixel = (x, y)
        picked = self._context_pick(x, y)
        # A locked image under the cursor is offered even when the click
        # landed on geometry drawn over it (the plaza on its orthomosaic):
        # the menu is the only way back to Unlock / Delete for it.
        under = self.pick_locked_image_plane(x, y)
        # A locked group the same way: it can no longer be selected, and
        # its Unlock lives here (#782).
        win.show_viewport_context_menu(
            ev.globalPos(), locked_image=under if under is not picked else None,
            locked_group=self.pick_locked_group(x, y))

    def mousePressEvent(self, ev) -> None:
        # A new gesture starts with the inferences back on: the Alt
        # toggle lasts ONE operation. Mid-operation (the
        # second click of a line) the tool is still busy and nothing moves.
        self._release_linear_mode()
        self._input_t = _time_mod.monotonic()   # P0: input→paint latency
        self._input_serial = getattr(self, "_input_serial", 0) + 1
        if (ev.button() in (Qt.LeftButton, Qt.RightButton)
                and self.nav_mode is None and self._look_drag is None
                and callable(getattr(self.active_tool, "on_look", None))):
            self._begin_look(ev)
            return
        if ev.button() == Qt.MiddleButton:
            self._last_pos = ev.position().toPoint()
            self._pan_mode = bool(ev.modifiers() & Qt.ShiftModifier)
            self._middle_drag_active = True
            self._orbit_pivot = self._orbit_pivot_at(
                ev.position().x(), ev.position().y())
            self._pan_depth = self._depth_of(self._orbit_pivot)
            # While the wheel-drag lasts, the pointer becomes the
            # orbit (or pan) icon; the tool cursor comes back on release.
            from views.icons import tool_cursor
            cur = tool_cursor("pan" if self._pan_mode else "orbit")
            if cur is not None:
                self.setCursor(cur)
            return
        # Nav buttons: left-drag orbits/pans the camera.
        # Hold Shift while orbiting to pan temporarily (matches MMB+Shift).
        if ev.button() == Qt.LeftButton and self.nav_mode is not None:
            if self.nav_mode == "zoom_window":
                # Drag a rectangle; the camera zooms to it on release.
                self._zoom_box_active = True
                self._zoom_box_start = ev.position()
                self._zoom_box_cur = ev.position()
                return
            self._last_pos = ev.position().toPoint()
            self._pan_mode = (
                self.nav_mode == "pan"
                or bool(ev.modifiers() & Qt.ShiftModifier)
            )
            self._orbit_pivot = (
                self._orbit_pivot_at(ev.position().x(), ev.position().y())
                if self.nav_mode == "orbit" else None)
            self._pan_depth = (
                self._depth_of(self._orbit_pivot_at(
                    ev.position().x(), ev.position().y()))
                if self._pan_mode else None)
            # The orbit/pan icon stays through the drag.
            self._apply_nav_cursor()
            return
        if ev.button() == Qt.LeftButton and self.active_tool is not None:
            # Triple click: a press landing right after a double-click at the
            # same spot (Qt has no native triple event): select all
            # connected.
            if self._is_triple_click(ev):
                self._last_double = None
                ctx = self._build_ctx(ev)
                if ctx is not None:
                    self.active_tool.on_triple_click(ctx)
                    self.update()
                return
            # Box-select tools defer the decision to release: a tiny drag is a
            # click, a real drag is a rubber-band box.
            if self.active_tool.box_select:
                self._box_active = True
                self._box_start = ev.position()
                self._box_cur = ev.position()
                return
            self._dispatch_tool_click(ev)

    def _is_triple_click(self, ev) -> bool:
        from PySide6.QtWidgets import QApplication
        last = self._last_double
        if last is None:
            return False
        t, pos = last
        return (ev.timestamp() - t
                <= QApplication.doubleClickInterval()
                and (ev.position() - pos).manhattanLength() < 8)

    def _dispatch_tool_click(self, ev, double: bool = False) -> None:
        """Forward a (double-)click to the active tool, then run the shared
        follow-ups: lock the chain to a clicked face's plane, clear the VCB."""
        had_start = getattr(self.active_tool, "start_point", None) is not None
        had_plane = getattr(self.active_tool, "work_plane", None) is not None
        face_at_click = None
        group_at_click = None
        from views.array_bake import reading_only
        if not had_start and not had_plane:
            # Only its plane is read, below: no need to build a big group's
            # mesh for that (seconds and gigabytes, #1092).
            with reading_only():
                face_at_click, group_at_click = self.pick_face_placement(
                    ev.position().x(), ev.position().y())
        ctx = self._build_ctx(ev)
        if ctx is not None:
            # The tool hears the cursor's snap KIND before it is called, as on
            # every hover (``_note_snap``): a click that never had its own
            # hover would otherwise leave a stale — or absent — kind in the
            # tool, and ``PlaneLock.snapped_plane`` decides with it whether
            # the shape re-plans onto the clicked point (#37) or stays on the
            # face under the cursor (#545).
            self._note_snap(ctx.snap)
            _before = len(self.history.undo_stack)
            if double:
                self.active_tool.on_double_click(ctx)
            else:
                self.active_tool.on_click(ctx)
            # Did this click actually MAKE something? That, not «the tool
            # went idle», is when the arrow lock should go — @pacaeiro on
            # 0.4.4: «I draw a line with hard lock (arrows) and, after the
            # line is created the lock is still active». The Line tool
            # chains, so it is still busy the instant a segment exists, and
            # the first fix waited for the whole polyline to end.
            _drew = len(self.history.undo_stack) > _before
            # If the click established a new start point on top of an
            # existing face, lock the rest of the chain to that face's
            # plane so subsequent clicks stay coplanar.
            now_start = getattr(self.active_tool, "start_point", None)
            if (
                not had_start
                and now_start is not None
                and face_at_click is not None
                and hasattr(self.active_tool, "work_plane")
                and self.active_tool.work_plane is None   # a plane lock wins
            ):
                # In WORLD space: a component's face keeps its own
                # coordinates, and the plane it captured for a rectangle
                # on the pergola's post was the untransformed one — the
                # second corner landed on a plane nowhere near the post
                # («la inferencia se atasca», Marco, 2026-09-14).
                from core.snap import face_plane_world
                self.active_tool.work_plane = face_plane_world(
                    face_at_click, getattr(group_at_click, "xform", None))
            # Any pending typed value is invalidated once the user
            # commits a point with the mouse.
            self._set_value_buffer("")
            # A command that failed was rolled back (History is
            # transactional) — surface it instead of failing silently.
            if self.history.last_error:
                self.flash_status(
                    tr("Operation failed and was undone: {err}",
                       err=self.history.last_error), 8000)
            self._release_axis_lock_after_operation(committed=_drew)
            if _drew:
                self._reassert_tool_cursor()
            self.update()

    def _reassert_tool_cursor(self) -> None:
        """Put the active tool's pointer back after an operation (#191,
        fafecm on Windows: «after the push/pull cursor is used once, the
        cursor symbol disappears» — the Line did the same). On Windows the
        viewport is a native OpenGL window and the system brought the arrow
        back when an operation ended, while Qt still believed its cursor
        set, so setting the same one again changed nothing: unset first,
        then set. Nothing to do while a camera mode or a look drag owns
        the pointer."""
        if (self.active_tool is None or self.nav_mode is not None
                or self._look_drag is not None):
            return
        self.unsetCursor()
        self._apply_tool_cursor()

    def enterEvent(self, ev) -> None:
        # Coming back from a panel or a dialog is the other moment Windows
        # shows the arrow over the viewport (#191).
        self._reassert_tool_cursor()
        super().enterEvent(ev)

    def mouseDoubleClickEvent(self, ev) -> None:
        """Qt replaces the second press of a double-click with this event, so
        route it to ``tool.on_double_click`` — whose default re-runs
        ``on_click``, keeping fast click-click rhythms working for drawing
        tools while Push/Pull overrides it to repeat its last distance.

        Box-select tools (Select) get the double-click DIRECTLY (no drag box
        starts on a double), and the event is remembered so the next press in
        place counts as a triple click.

        A double click of the wheel recentres the view on the point under
        the cursor and starts no orbit; the tool in use never hears of it
        (#979, #1154)."""
        if ev.button() == Qt.MiddleButton:
            self._recentre_on(ev.position().x(), ev.position().y())
            return
        if ev.button() != Qt.LeftButton or self.active_tool is None \
                or self.nav_mode is not None:
            self.mousePressEvent(ev)
            return
        self._last_double = (ev.timestamp(), ev.position())
        if self.active_tool.box_select:
            ctx = self._build_ctx(ev)
            if ctx is not None:
                self.active_tool.on_double_click(ctx)
                self.update()
            return
        self._dispatch_tool_click(ev, double=True)

    # ---- Mouse look (a tool with ``on_look``: First Person) -----------------


    def _end_look(self) -> None:
        self._look_drag = None
        self._apply_tool_cursor()

    def mouseMoveEvent(self, ev) -> None:
        self._input_t = _time_mod.monotonic()   # P0: input→paint latency
        self._input_serial = getattr(self, "_input_serial", 0) + 1
        # An Alt released while another window had the keyboard never
        # reaches any filter here; the pointer re-reads it on the move.
        alt = bool(ev.modifiers() & Qt.AltModifier)
        if alt != self._alt_down:
            self._apply_alt_cursor(alt)
        if self._look_drag is not None and ev.buttons() == Qt.NoButton:
            # The look drag's release was lost (the same hole as the orbit
            # below): with the pointer hidden and put back on every move it
            # would be trapped in the window. No button held ends it.
            self._end_look()
        if self._look_drag is not None:
            if callable(getattr(self.active_tool, "on_look", None)):
                self._look_move(ev)
                return
            self._end_look()                    # the tool changed mid-drag
        if self._last_pos is not None and ev.buttons() == Qt.NoButton:
            # A camera drag with no button held: its release never reached
            # us (a Windows mouse or touchpad driver, a click on another
            # window, a tool switched by keyboard mid-drag). Left alone,
            # EVERY mouse move orbited the view whatever tool was picked —
            # «el orbital se mantiene activo… no se desactiva con nada»
            # (Andrés Rodríguez, Windows 11 + NVIDIA, 0.5.0).
            self._end_camera_drag()
        if self._middle_drag_active:
            self._set_middle_drag_pan_mode(
                bool(ev.modifiers() & Qt.ShiftModifier))
        if self._last_pos is not None:
            p = ev.position().toPoint()
            dx = p.x() - self._last_pos.x()
            dy = p.y() - self._last_pos.y()
            self._last_pos = p
            if self.nav_mode == "zoom":
                self.camera.zoom(-dy * 0.035)        # drag up = zoom in
            elif self._pan_mode:
                self.camera.pan(dx, dy, self.height(),
                                depth=getattr(self, "_pan_depth", None))
            else:
                pivot = getattr(self, "_orbit_pivot", None)
                ody = -dy if self._invert_orbit_y else dy
                if pivot is not None:
                    self.camera.orbit_about(pivot, dx, ody, self.height())
                else:
                    self.camera.orbit(dx, ody, self.height())
            self.update()
            return

        if self._box_active:
            self._box_cur = ev.position()
            self.update()
            return

        if self._zoom_box_active:
            self._zoom_box_cur = ev.position()
            self.update()
            return

        # Adaptive hover coalescing: against a big imported model one hover
        # costs ~25-30 ms (picks + snap) while the mouse delivers 60-125
        # events/s — processing every event backlogs the queue and the whole
        # app reads as frozen. Gate by the measured cost of the LAST hover
        # (a small model gates at ~0 → immediate), and let a trailing-edge
        # timer process the final position so the highlight never sticks.
        self._pending_hover = (ev.position(), ev.modifiers())
        now = _time_mod.monotonic()
        gate = min(0.06, getattr(self, "_hover_cost", 0.0) * 1.5)
        elapsed = now - getattr(self, "_hover_last_t", 0.0)
        if elapsed < gate:
            timer = getattr(self, "_hover_timer", None)
            if timer is None:
                from PySide6.QtCore import QTimer
                timer = self._hover_timer = QTimer(self)
                timer.setSingleShot(True)
                timer.timeout.connect(self._run_pending_hover)
            if not timer.isActive():
                timer.start(max(1, int((gate - elapsed) * 1000) + 1))
            return
        self._run_pending_hover()

    def _run_pending_hover(self) -> None:
        pending = getattr(self, "_pending_hover", None)
        if pending is None:
            return
        self._pending_hover = None
        pos, modifiers = pending
        t0 = _time_mod.monotonic()
        self._process_hover(pos, modifiers)
        self._hover_last_t = _time_mod.monotonic()
        self._hover_cost = self._hover_last_t - t0
        if _PERF:
            # A per-move cost near ~100 ms starves paints during a tool drag
            # (the push "sticks"): name it in the log instead of hiding
            # under the frame telemetry's floor.
            _plog("hover.move", self._hover_cost * 1000.0, floor=80.0)

    # -- Snapping inference, delegated to self.inference -----------------
    # The reference state (the hover / acquired edges and points, the centre
    # reference, the axis and reference locks, the last snap) lives in
    # core.inference.InferenceEngine, and so do the rules that used to be the
    # two post-processors of the search (the axis-source cue, the extension
    # hooks). These properties and thin wrappers keep the surface the tools,
    # the extensions, the probe and the tests read and write, so the
    # extraction stays behaviour-neutral.
    @property
    def _acquired_point(self) -> Optional[QVector3D]:
        return self.inference._acquired_point

    @_acquired_point.setter
    def _acquired_point(self, value: Optional[QVector3D]) -> None:
        self.inference._acquired_point = value

    @property
    def _encouraged(self) -> list:
        return self.inference._encouraged

    @_encouraged.setter
    def _encouraged(self, value: list) -> None:
        self.inference._encouraged = value

    @property
    def _dwell_point(self) -> Optional[QVector3D]:
        return self.inference._dwell_point

    @_dwell_point.setter
    def _dwell_point(self, value: Optional[QVector3D]) -> None:
        self.inference._dwell_point = value

    @property
    def _dwell_timer(self) -> QTimer:
        return self.inference._dwell_timer

    @_dwell_timer.setter
    def _dwell_timer(self, value: QTimer) -> None:
        self.inference._dwell_timer = value

    @property
    def axis_lock(self) -> Optional[str]:
        return self.inference.axis_lock

    @axis_lock.setter
    def axis_lock(self, value: Optional[str]) -> None:
        self.inference.axis_lock = value

    @property
    def last_snap(self) -> Optional[SnapResult]:
        return self.inference.last_snap

    @last_snap.setter
    def last_snap(self, value) -> None:
        self.inference.last_snap = value

    @property
    def reference_edge(self):
        return self.inference.reference_edge

    @reference_edge.setter
    def reference_edge(self, value) -> None:
        self.inference.reference_edge = value

    @property
    def reference_mode(self) -> Optional[str]:
        return self.inference.reference_mode

    @reference_mode.setter
    def reference_mode(self, value: Optional[str]) -> None:
        self.inference.reference_mode = value

    @property
    def _shift_lock(self):
        return self.inference._shift_lock

    @_shift_lock.setter
    def _shift_lock(self, value) -> None:
        self.inference._shift_lock = value

    @property
    def _shift_lock_ref(self):
        return self.inference._shift_lock_ref

    @_shift_lock_ref.setter
    def _shift_lock_ref(self, value) -> None:
        self.inference._shift_lock_ref = value

    @property
    def _hover_edge(self):
        return self.inference._hover_edge

    @_hover_edge.setter
    def _hover_edge(self, value) -> None:
        self.inference._hover_edge = value

    @property
    def _hover_center(self):
        return self.inference._hover_center

    @_hover_center.setter
    def _hover_center(self, value) -> None:
        self.inference._hover_center = value

    @property
    def _center_ref(self):
        return self.inference._center_ref

    @_center_ref.setter
    def _center_ref(self, value) -> None:
        self.inference._center_ref = value

    @property
    def _acquired_edge(self):
        return self.inference._acquired_edge

    @_acquired_edge.setter
    def _acquired_edge(self, value) -> None:
        self.inference._acquired_edge = value

    @property
    def _acquired_face_normal(self):
        return self.inference._acquired_face_normal

    @_acquired_face_normal.setter
    def _acquired_face_normal(self, value) -> None:
        self.inference._acquired_face_normal = value

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
        if (self._last_pos is not None or self._box_active
                or self.nav_mode is not None):
            # Navigation tools don't select or snap. In particular, a
            # pointer move between orbit drags must not build the editing
            # pick index and block the next press on a large assembly.
            return
        self._tick = getattr(self, "_tick", 0) + 1     # a new epoch memo
        ev = _HoverEvent(pos, modifiers)
        _hp0 = _time_mod.perf_counter() if _PERF else 0.0

        def _hmark(name):
            nonlocal _hp0
            if _PERF:
                now = _time_mod.perf_counter()
                _plog(f"hover.{name}", (now - _hp0) * 1000.0, floor=200.0)
                _hp0 = now

        # Track cursor + hover edge so Down can capture a reference edge.
        self._last_mouse_pos = ev.position()
        self._emit_coordinate(ev)
        _hmark("coord")
        # What the snap reads of a face under the cursor builds no lazy mesh
        # (#1092); a tool's on_hover below may keep what it picks, and a click
        # gets the real faces.
        from views.array_bake import reading_only
        with reading_only():
            self.inference.acquire(ev.position().x(), ev.position().y())

            if self.active_tool is None:
                return
            _hmark("acquire")
            ctx = self._build_ctx(ev)
        _hmark("ctx")
        if ctx is None:
            return
        self._note_snap(ctx.snap)
        if (self.active_tool.uses_snap
                and getattr(self.active_tool, "allow_inferences", True)):
            # Encouraging: the point the snap already found — no extra pick
            # per hover. A circle's rim encourages its CENTRE (found by the
            # centre reference while the cursor is on the rim); on a face
            # with arcs a real corner under the cursor still wins.
            center = getattr(self, "_hover_center", None)
            point = None
            if ctx.snap.kind in self._ENCOURAGING_KINDS:
                point = ctx.snap.point
            if center is not None and (point is None or center[2][0] == "edge"):
                point = QVector3D(center[0])
            self._dwell_on(point)
        self.active_tool.on_hover(ctx)
        _hmark("tool")
        self.measurementChanged.emit(self._measurement_text())
        _hmark("measure")
        self.update()

    # Below this many pixels of drag, a left press/release is a click, not a box.
    BOX_DRAG_THRESHOLD_PX = 4.0

    def mouseReleaseEvent(self, ev) -> None:
        if self._look_drag is not None and ev.button() == self._look_drag[0]:
            self._end_look()
            return
        if ev.button() == Qt.MiddleButton:
            self._last_pos = None
            self._orbit_pivot = None
            self._pan_mode = False
            self._middle_drag_active = False
            if self.nav_mode is not None:
                self._apply_nav_cursor()
            else:
                self._apply_tool_cursor()
            return

        if ev.button() == Qt.LeftButton and self._zoom_box_active:
            self._zoom_box_active = False
            start, end = self._zoom_box_start, self._zoom_box_cur
            self._zoom_box_start = None
            self._zoom_box_cur = None
            if start is not None and end is not None:
                self._zoom_to_box(start, end)
            self.update()
            return

        if ev.button() == Qt.LeftButton and self.nav_mode is not None:
            self._last_pos = None
            self._orbit_pivot = None
            self._pan_mode = False
            self._apply_nav_cursor()
            return

        # Stroke tools (the Eraser): notify the release so a press-drag-release
        # stroke can commit as one step. No-op default on other tools.
        if (ev.button() == Qt.LeftButton and self.active_tool is not None
                and not self._box_active and self.nav_mode is None):
            _before = len(self.history.undo_stack)
            self.active_tool.on_release(self)
            self._release_axis_lock_after_operation()
            if len(self.history.undo_stack) > _before:
                self._reassert_tool_cursor()     # a drag committed (#191)

        if ev.button() == Qt.LeftButton and self._box_active:
            self._box_active = False
            start = self._box_start
            end = ev.position()
            self._box_start = None
            self._box_cur = None
            tool = self.active_tool
            if tool is None or start is None:
                self.update()
                return
            dx = end.x() - start.x()
            dy = end.y() - start.y()
            # The box reads the same modifiers as a click: Shift
            # toggles what it catches — the way you rub a wall out of a big
            # catch — Ctrl adds and Shift+Ctrl removes.
            mode = selection_mode(ev.modifiers())
            additive = mode != "replace"
            if math.hypot(dx, dy) < self.BOX_DRAG_THRESHOLD_PX:
                # A click: pick the single entity under the cursor.
                ctx = self._build_ctx(ev)
                if ctx is not None:
                    tool.on_click(ctx)
            else:
                rect = (
                    min(start.x(), end.x()), min(start.y(), end.y()),
                    max(start.x(), end.x()), max(start.y(), end.y()),
                )
                crossing = dx < 0  # right-to-left drag = crossing selection
                tool.on_box_select(self, rect, crossing, additive, mode=mode)
            self.update()



    def wheelEvent(self, ev) -> None:
        # While a 3D mouse moves the view, wheel notches are its driver's
        # imitation of the same push (3Dconnexion does that for programs it
        # does not know): zooming on them too made the view jump a step
        # about eight times a second over the smooth move.
        from views.ndof_input import cap_in_use
        if cap_in_use():
            ev.accept()
            return
        self._input_t = _time_mod.monotonic()   # P0: input→paint latency
        self._input_serial = getattr(self, "_input_serial", 0) + 1
        # A finger slide (trackpad, or the touch screen) orbits or pans
        # instead, by preference; it says so by taking the event.
        from ingetrazo_render import trackpad_nav
        if trackpad_nav.slide(self, ev):
            self.update()
            return
        import time as _time
        from views.wheel_zoom import zoom_steps
        steps = zoom_steps(ev, self._invert_wheel)
        if steps == 0.0:
            ev.accept()            # trackpad begin/end events do not zoom or pick
            return
        now = _time.monotonic()
        reproj, reused = self._zoom_at(steps, ev.position())
        if _PERF:
            _plog("wheel", (_time_mod.monotonic() - now) * 1000.0,
                  extra=f"reused={'proj' if reproj else reused}", floor=10.0)
        self.update()


    #: The least a notch of the wheel backs out, from a close-up, once the
    #: camera stands 2 cm or more from what it looks at — and always in an
    #: empty scene, which has no size to go by.
    _ZOOM_OUT_STEP = 0.05


    def _model_diag(self) -> float:
        """The model's size, for the zoom step. On a huge model it comes
        from the placement boxes the pick index already holds: the exact
        box read every vertex, ~2 s on the first wheel notch (#158)."""
        cached = getattr(self.scene, "_bounds_cache", None)
        if cached is None or cached[0] != self.scene.version:
            table = (self._pick_lazy_table()
                     if hasattr(self, "_pick_lazy_table") else None)
            if table is not None:
                import numpy as np
                lo, hi = table[1], table[2]
                ok = np.isfinite(lo).all(axis=1) & np.isfinite(hi).all(axis=1)
                if ok.any():
                    return float(np.linalg.norm(hi[ok].max(axis=0)
                                                - lo[ok].min(axis=0)))
        lo, hi = self.scene.bounds()
        return (hi - lo).length() if lo is not None else 0.0

    _MODIFIER_KEYS = frozenset({Qt.Key_Control, Qt.Key_Shift, Qt.Key_Alt,
                                Qt.Key_Meta, Qt.Key_AltGr})

    def ctrl_tapped(self) -> bool:
        """Whether the Ctrl being released was pressed ALONE — no other key
        in between, not even one a menu shortcut took (Ctrl+Z reaches us
        only as a ShortcutOverride). Tools that toggle on a Ctrl tap ask
        this on the release (#183)."""
        return getattr(self, "_ctrl_alone", False)

    def event(self, ev) -> bool:
        if ev.type() == QEvent.NativeGesture:
            # Pinch, smart zoom: a trackpad's, or the touch screen's (#248).
            from ingetrazo_render import trackpad_nav
            if trackpad_nav.native_gesture(self, ev):
                return True
        if (ev.type() in (QEvent.ShortcutOverride, QEvent.KeyPress)
                and ev.key() not in self._MODIFIER_KEYS):
            self._ctrl_alone = False
        # With a VCB buffer in progress, claim keys that continue it (unit
        # suffixes m/cm/mm, separators, sign) before the window's QAction
        # shortcuts swallow them — otherwise typing "2m" would fire the Move
        # tool instead of finishing the length. Bare letters with no buffer
        # still reach the shortcuts.
        # A tool that plays letters as keys (First Person's W/A/S/D) takes
        # them from the window's tool shortcuts while it is active.
        if ev.type() == QEvent.ShortcutOverride and self._tool_claims_key(ev):
            ev.accept()
            return True
        if (ev.type() == QEvent.ShortcutOverride
                and self._measurement_claims_key(ev, self._value_buffer)):
            ev.accept()
            return True
        return super().event(ev)

    def _measurement_claims_key(self, ev, buffer) -> bool:
        """Reserve measurement text before Qt dispatches window shortcuts."""
        if _is_chord(ev.modifiers()):
            return False
        text = ev.text().lower()
        zoom = self.active_tool is None and getattr(self, "nav_mode", None) == "zoom"
        if not text or (self.active_tool is None and not zoom):
            return False
        if (text == "=" and not buffer) or (is_expression(buffer) and text.isprintable()):
            return True
        if zoom:
            return (text.isdigit() or text in (".", ",") or
                    (any(c.isdigit() for c in buffer) and text in "mdeg °"))
        if buffer:
            return (text.isdigit() or text in ".,; -:\"'/mcrinftx*s" or
                    (text == "p" and hasattr(self.active_tool, "on_profile_value")))
        return (text == "x" and getattr(self.active_tool, "accepts_array", False)
                and self._array_pending())

    def _tool_claims_key(self, ev) -> bool:
        claims = getattr(self.active_tool, "claims_key", None)
        return callable(claims) and bool(claims(ev.key(), ev.modifiers()))

    def keyPressEvent(self, ev) -> None:
        if ev.key() == Qt.Key_Control and not ev.isAutoRepeat():
            self._ctrl_alone = True       # until another key says otherwise
        # Held keys (First Person): the press and the release are the
        # tool's; auto-repeat says nothing new.
        if self._tool_claims_key(ev):
            if not ev.isAutoRepeat():
                self.active_tool.on_key(self, ev.key(), ev.modifiers())
            return
        # 0. Shift state change → refresh snap immediately so the user sees
        #the contextual lock take effect without moving the mouse.
        if ev.key() == Qt.Key_Down and not ev.isAutoRepeat():
            # A Down press takes the plane over from a held Shift (see
            # _mark_plane_for_shift): what the user locks with it now
            # outlives the key, so the Shift release must not undo it.
            self._shift_plane_mark = None
        if ev.key() == Qt.Key_Shift and not ev.isAutoRepeat():
            self._set_middle_drag_pan_mode(True)
            self._capture_shift_lock()
            self._refresh_snap()
            # Do not return — Shift is a modifier; let the rest fall through.
        # 0b. Alt turns Paint into the eyedropper: show it on the pointer the
        #     moment the key goes down, not after the next mouse move.
        if ev.key() == Qt.Key_Alt and not ev.isAutoRepeat():
            self._apply_alt_cursor(True)
        elif self._alt_tap and not ev.isAutoRepeat():
            # Alt+something is a shortcut: we are not taking its release.
            self._alt_tap = False

        # 1. Numeric value buffer (VCB-style length input).
        if self._handle_value_key(ev):
            return

        # 2. Active tool gets first shot at the key.
        if self.active_tool is not None:
            if self.active_tool.on_key(self, ev.key(), ev.modifiers()):
                return

        # 2b. Delete in any other tool (Paint, Push/Pull, Offset…): with the
        #     tool idle, erase the selection or — nothing selected — the edge
        #     or face highlighted under the cursor. Mid-operation it would
        #     pull the geometry out from under the tool, so it waits.
        if ev.key() == Qt.Key_Delete and not ev.isAutoRepeat():
            tool = self.active_tool
            if tool is None or not self._tool_busy_for_delete(tool):
                from tools.select import delete_selection_or_hover
                delete_selection_or_hover(self)
                return

        # 3. Esc, escalating (standard CAD): first clear the typed value buffer,
        #    then cancel the tool's in-progress action (an unfinished chain, a
        #    drag), and finally — nothing in progress — clear the selection.
        if ev.key() == Qt.Key_Escape:
            self.escape()
            return

        # 3. Projection toggle.
        if ev.key() == Qt.Key_P:
            self.toggle_projection()
            return

        # 3b. Alt: cycle linear inferences — ONLY while an operation is in
        #     progress, the classic behaviour. Its status bar offers
        #     «Alt = Activar/desactivar "Inferencias lineales"» after the
        #     first click of a line and not before, and the three states are
        #     ours exactly: all → off → parallel/perp.
        #
        #     We had it global, at any moment, and that is the whole of
        #     issue #26: Pedro Caeiro switches windows with Alt+Tab all day
        #     and every switch moved a mode he was not using. Idle, there is
        #     now nothing to steal. Cycling on the PRESS (not the release) is
        #     deliberate: the release is what makes Qt's menu bar take focus,
        #     so it lands on the QMenuBar, not here — measured on Marco's
        #     desktop, one tap in six reached us.
        if ev.key() == Qt.Key_Alt and not ev.isAutoRepeat():
            if self.active_tool is not None and self._tool_busy(
                    self.active_tool):
                self._alt_tap = True        # the release is ours too
                self._cycle_linear_inference_mode()
            return

        # 4. Axis lock (arrow keys). Pressing the same arrow toggles it off.
        if ev.key() == Qt.Key_Right:
            self.axis_lock = None if self.axis_lock == "x" else "x"
            self._refresh_snap()
            return
        if ev.key() == Qt.Key_Left:
            self.axis_lock = None if self.axis_lock == "y" else "y"
            self._refresh_snap()
            return
        if ev.key() == Qt.Key_Up:
            self.axis_lock = None if self.axis_lock == "z" else "z"
            self._refresh_snap()
            return

        # 5. Reference edge — Down cycles None → parallel → perpendicular → None.
        if ev.key() == Qt.Key_Down:
            self._cycle_reference_mode()
            self._refresh_snap()
            return

        super().keyPressEvent(ev)

    def _release_linear_mode(self) -> None:
        """Back to «all inferences» once the operation is over.

        The toggle lasts ONE operation (the classic behaviour), and Marco checked it
        against the real thing (2026-09-17): «desactivo con alt, termino de
        dibujar la línea, aprieto la flechita y aprieto otra vez la línea y
        está activo la inferencia». Ours was sticky — switched off it stayed
        off for the rest of the session, which is a trap: you turn it off
        for one line and then wonder for an hour why nothing snaps. It is
        also what made issue #26 bite so hard, since an accidental Alt+Tab
        left it off for good.

        Called where an operation can END: a click that commits, a release,
        Esc, and any change of tool."""
        if self.linear_inference_mode == "all":
            return
        tool = self.active_tool
        if tool is not None and self._tool_busy(tool):
            return
        self.linear_inference_mode = "all"
        self._refresh_snap()
        self.update()

    def _release_axis_lock_after_operation(self, committed: bool = False) -> None:
        """Drop the arrow-key axis lock when the operation that used it ends.

        The classic rule, reported by @pacaeiro (issue #30): «if you're
        drawing a line and press Arrow to lock X and finish drawing the
        line, the X lock is released automatically. In ingeTrazo the Axis
        lock keeps active». Ours only came off in the Esc cascade, so the
        lock — and its label on screen — outlived the line it was for and
        silently constrained whatever you drew next.

        Same shape as the Alt toggle two issues ago (#26): a mode meant to
        last one operation that lasted the session. The Down-arrow
        reference is deliberately NOT released here — it is bound to an
        edge the user went and picked, which is a more deliberate act than
        tapping an arrow, and it already has Esc.

        Called where an operation can END: a click that commits, a stroke
        released, a typed value applied. A no-op mid-operation, so the
        second click of a line keeps its lock.
        """
        if self.axis_lock is None:
            return
        tool = self.active_tool
        if not committed and tool is not None and self._tool_busy(tool):
            return
        self.axis_lock = None
        self._refresh_snap()
        self.update()

    def _cycle_linear_inference_mode(self) -> None:
        """Alt: cycle linear inferences all → off → parallel/perp → all.

        The classic toggle, states and all — the usual status bar reads «Alt =
        Activar/desactivar "Inferencias lineales"» while a line is being
        drawn. Offered only DURING an operation there, and here too since
        issue #26. Point snaps (endpoint, midpoint, …) stay on; explicit
        locks (arrow keys, Down reference) keep working in every mode."""
        order = {"all": "off", "off": "parallel_perp", "parallel_perp": "all"}
        self.linear_inference_mode = order[self.linear_inference_mode]
        label = {
            "all": "Linear inferences: all on",
            "off": "Linear inferences: off",
            "parallel_perp": "Linear inferences: parallel / perpendicular only",
        }[self.linear_inference_mode]
        self.measurementChanged.emit(label)
        self._refresh_snap()

    def escape(self) -> None:
        """The Esc cascade (standard CAD), ONE step per press: a typed value
        buffer, then a sticky constraint (axis lock / reference), then the
        array window a copy just opened, then the tool's in-progress action, then — nothing in progress — step out ONE level of the open group,
        then clear the selection at the root, and
        finally the tool's own cancel. The window's «Cancel current tool»
        action (shortcut Esc, which fires BEFORE this widget's key event)
        calls this too: it used to carry its own copy of the cascade,
        without the group step, so Esc never left a group (Marco,
        2026-09-14: «he hecho Esc varias veces y no salgo»)."""
        if self._value_buffer:
            self._set_value_buffer("")
            return
        if self.release_constraints():
            return
        tool = self.active_tool
        close = getattr(tool, "close_array_window", None)
        if callable(close) and self._array_pending():
            # Right after a copy the array window is what is open: Esc
            # closes it before touching the selection, so a bare X is the
            # Text shortcut again (#542).
            close()
            return
        if tool is not None and type(tool).__name__ == "PasteTool":
            win = self.window()
            activate = getattr(win, "_activate_tool", None)
            if activate is not None:
                activate("select")
                return
        if tool is not None and self._tool_busy(tool):
            tool.on_cancel(self)
            self._release_linear_mode()
            return
        if self.scene.edit_group is not None:
            self.clear_extension_pick()
            self.end_one_group_edit()       # selection must not consume Esc
            return
        if self.scene.selection or self.extension_pick is not None:
            self.scene.clear_selection()
            self.clear_extension_pick()
            self.update()
            return
        if tool is not None:
            tool.on_cancel(self)

    # ---- Items an extension lets the user select (issue #205) ---------------
    #: The one extension item selected — ``(entry, item_id, scene version)``
    #: — or None. Kept APART from ``scene.selection``, which only ever holds
    #: the model's own entities, so nothing that walks the selection (Move,
    #: layers, zoom, copy) meets a thing it does not know.
    extension_pick = None

    def pick_extension_item(self, px: float, py: float):
        """The first extension item under the pixel, as ``(entry, id)``."""
        for entry in getattr(self, "_ext_pickables", ()):
            try:
                item = entry["pick"](self, px, py)
            except Exception:  # noqa: BLE001 — an extension's bug, not ours
                import logging
                logging.getLogger(__name__).exception("pickable failed")
                continue
            if item is not None:
                return entry, item
        return None

    def set_extension_pick(self, hit) -> None:
        """Select one extension item (``hit`` from pick_extension_item),
        telling the extension what was let go and what was taken."""
        self.clear_extension_pick()
        entry, item = hit
        self.extension_pick = (entry, item, self.scene.version)
        if entry.get("on_select") is not None:
            entry["on_select"](item)
        self.update()

    def clear_extension_pick(self, notify: bool = True) -> None:
        pick = self.extension_pick
        self.extension_pick = None
        if pick is not None and notify and pick[0].get("on_select") is not None:
            pick[0]["on_select"](None)

    def delete_extension_pick(self) -> bool:
        """Supr on a selected extension item: the extension deletes it (as
        its own undo step). A pick older than the document's last change
        is dropped instead — after an undo the id may name another item."""
        pick = self.extension_pick
        if pick is None:
            return False
        entry, item, version = pick
        self.clear_extension_pick()
        if version != self.scene.version or entry.get("delete") is None:
            return version != self.scene.version
        entry["delete"](item)
        self.update()
        return True

    def release_constraints(self) -> bool:
        """Drop the sticky drawing constraints — the arrow-key axis lock and
        the Down-arrow parallel/perpendicular reference. First stop of the
        Esc cascade: returns True if there was one to release (that Esc press
        is consumed; the next one cancels the tool action as before)."""
        if self.axis_lock is None and self.reference_mode is None:
            return False
        self.axis_lock = None
        self.reference_edge = None
        self.reference_mode = None
        self._refresh_snap()
        return True

    def _cycle_reference_mode(self, edge=None) -> None:
        """Down arrow: cycle None → parallel → perpendicular → None.

        the modeller's: on entry to parallel mode it captures ``edge`` (the
        Metal view's pick under the cursor), else the edge under the cursor,
        else the last edge the line being drawn referenced (the soft
        parallel/perpendicular the cursor picked up and then left: Down
        makes that reference a lock). With no edge at all, do nothing —
        there is nothing to be parallel/perpendicular to.
        """
        if self.reference_mode is None:
            edge = (edge if edge is not None else self._hover_edge
                    if self._hover_edge is not None
                    else self.inference._acquired_edge)
            if edge is None:
                return  # nothing to capture
            self.reference_edge = edge
            self.reference_mode = "parallel"
        elif self.reference_mode == "parallel":
            self.reference_mode = "perpendicular"
        else:
            self.reference_edge = None
            self.reference_mode = None

    def _refresh_snap(self) -> None:
        """Re-run snap with the last known cursor position. Used when modifier
        state changes (axis lock, reference mode, Shift) without mouse motion."""
        self._sync_axes()
        self.update()
        if (
            self._last_mouse_pos is None
            or self.active_tool is None
            or not self.active_tool.uses_snap
        ):
            return
        from PySide6.QtGui import QGuiApplication

        modifiers = QGuiApplication.keyboardModifiers()
        p = self._last_mouse_pos.toPoint()
        snap = self._snap_at(p.x(), p.y(), modifiers)
        if snap is None:
            return                              # the ray casts no world point
        # Same bookkeeping as a hover with movement: the tool reads the KIND
        # of the snap it is being handed (``_note_snap``), and a modifier
        # changing with the hand still is exactly when a planar shape has to
        # know whether the cursor is on a FACE or on a point of the model.
        self._note_snap(snap)
        ctx = ToolContext(
            viewport=self,
            world=snap.point,
            screen=self._last_mouse_pos,
            modifiers=modifiers,
            snap=snap,
        )
        self.active_tool.on_hover(ctx)
        self.measurementChanged.emit(self._measurement_text())

    def refresh_status_hint(self) -> None:
        """Redraw the status bar's base line now. The hint otherwise rides
        the 12 Hz text timer, which only ticks on mouse movement — a tool
        that changes mode on a KEY needs to say so without waiting for the
        hand to move (the Tape's Ctrl, issue #29)."""
        win = self.window()
        fn = getattr(win, "_update_status_hint", None)
        if fn is not None:
            fn()

    def _alt_tapped(self) -> bool:
        """A clean Alt tap (press and release, nothing in between) toggles
        Paint's eyedropper and it STAYS until one sample is taken or Alt is
        tapped again — the usual behaviour (Marco, 2026-09-23: «se alterna
        con Alt, no es que se mantenga presionado»). Holding Alt and
        clicking still samples, as before."""
        if self.nav_mode is not None or getattr(
                self.active_tool, "icon", None) != "paint":
            return False
        from tools.paint import PaintTool
        win = self.window()
        act = getattr(win, "_act_eyedropper", None)
        if act is not None:
            act.setChecked(not act.isChecked())   # the button shows it too
        else:
            PaintTool.sample_armed = not PaintTool.sample_armed
        self._apply_tool_cursor()
        return True

    def _apply_alt_cursor(self, down: bool) -> None:
        """Refresh the pointer for an Alt state change (Paint <-> eyedropper);
        no-op while a camera nav mode owns the cursor. ``down`` is the key's
        state after this event — see ``_apply_tool_cursor``."""
        self._alt_down = bool(down)
        if self.nav_mode is None and getattr(
                self.active_tool, "icon", None) == "paint":
            self._apply_tool_cursor()

    def keyReleaseEvent(self, ev) -> None:
        release = getattr(self.active_tool, "on_key_release", None)
        if callable(release):
            if ev.isAutoRepeat() and self._tool_claims_key(ev):
                return
            if not ev.isAutoRepeat() and release(self, ev.key()):
                return
        if ev.key() == Qt.Key_Alt and not ev.isAutoRepeat():
            self._apply_alt_cursor(False)
            if self._alt_tap:
                # We acted on the press, so the release is ours: swallowing
                # it keeps Qt's menu bar from taking focus on it. Otherwise
                # the focus ping-pongs viewport → menu bar → viewport and
                # every other Alt lands on the menu bar instead of here
                # (issue #26; measured: one tap in six got through).
                self._alt_tap = False
                ev.accept()
                return
        if ev.key() == Qt.Key_Shift and not ev.isAutoRepeat():
            self._set_middle_drag_pan_mode(False)
            self._shift_lock = None
            self._shift_lock_ref = None
            self._release_shift_plane_mark()
            self._refresh_snap()
        super().keyReleaseEvent(ev)

    #: The view shown in this one's place (the Metal view), which takes the
    #: keyboard whenever this hidden one is given it.
    keys_stand_in = None

    def setFocus(self, *args) -> None:  # noqa: N802 — Qt's name
        """Orbit / Pan, a scene picked in its list, the camera panel, a
        project switch… all give the keyboard back with
        ``viewport.setFocus()``. Under Metal this viewport is hidden and Qt
        ignores it there, so the keys stayed in the list or on a button:
        the view the user sees takes them instead."""
        shown = self.keys_stand_in
        if shown is not None and not self.isVisible():
            try:
                visible = shown.isVisible()
            except RuntimeError:        # its C++ side is gone
                self.keys_stand_in = visible = None
            if visible:
                shown.setFocus(*args)
                return
        super().setFocus(*args)

    def focusNextPrevChild(self, next) -> bool:  # noqa: A002 — Qt's name
        """Tab is a tool key (``views.shortcuts.reserved_reason``): Qt's
        focus chain took it and left the keyboard on a tray button, where
        the arrows, typed values and Enter did nothing."""
        return False

    def focusOutEvent(self, ev) -> None:
        """Focus gone means the Alt release will land somewhere else, so the
        claim on it lapses (issue #26)."""
        self._alt_tap = False
        self._alt_down = False        # its release will not reach us either
        self._ctrl_alone = False      # nor will Ctrl's
        self._release_shift_plane_mark()   # nor Shift's: unmark the plane
        # Nor will the release of a held walking key (First Person).
        lost = getattr(self.active_tool, "on_focus_out", None)
        if callable(lost):
            lost(self)
        super().focusOutEvent(ev)

    # Inferences whose direction can be captured by a Shift lock.
    _SHIFT_LOCKABLE = frozenset({
        "axis", "axis_inference", "reference", "through_point", "perp_face",
        "extension", "from_point",
    })

    def _capture_shift_lock(self) -> None:
        """On Shift press, mark the plane the shape is drawing on and freeze
        the active inference's direction so it holds even as the cursor
        wanders off it (inference lock). Both at once, one does not take
        the other's place: the mark only says WHICH plane, and the pointer
        goes on marking midpoints, endpoints and alignments (Marco: «100%
        aditivo»). Each side keeps its own guard."""
        self._shift_lock = None
        self._shift_lock_ref = None
        self._mark_plane_for_shift()
        snap = self.last_snap
        tool = self.active_tool
        owns = getattr(tool, "owns_shift_lock", None)
        if owns is not None and owns():
            return  # the tool reads the cursor's motion itself
        start = getattr(tool, "start_point", None) if tool is not None else None
        if snap is None or snap.kind not in self._SHIFT_LOCKABLE:
            return
        axis = snap.axis if snap.axis in _AXIS_VECTORS else None
        if start is None and axis is None:
            return          # no start point and no axis: nothing to lock

        # Lock to the exact inferred axis, preserving its direction.
        if axis is not None:
            d = QVector3D(_AXIS_VECTORS[axis])
            if start is not None and QVector3D.dotProduct(snap.point - start, d) < 0.0:
                d = -d
        else:
            d = snap.point - start
        if d.length() > 1e-6:
            self._shift_lock = (d.normalized(), snap.color)
            if (snap.kind == "from_point" and axis is not None
                    and getattr(snap, "guide", None) is not None):
                self._shift_lock_ref = (snap.guide[0], axis)

    def _mark_plane_for_shift(self) -> None:
        """Shift marks the plane the shape would draw on — the tool's call
        (``PlaneLock.mark_plane``). Unlike the inference lock above this is
        HELD, not locked: the release hands the plane back
        (``_release_shift_plane_mark``)."""
        self._shift_plane_mark = None
        tool = self.active_tool
        if tool is None:
            return
        from tools.base import PlaneLock
        if not isinstance(tool, PlaneLock):
            return
        if tool.mark_plane(self):
            self._shift_plane_mark = tool

    def _release_shift_plane_mark(self) -> None:
        """Shift let go (or the window lost focus): hand the plane back.
        Only the mark that Shift itself put there goes — a plane locked with
        an arrow or with Down in the meantime outlives the key."""
        tool = getattr(self, "_shift_plane_mark", None)
        self._shift_plane_mark = None
        if tool is None:
            return
        ref = getattr(tool, "plane_ref", None)
        if ref is not None and ref[1] == "plane":
            tool.plane_ref = None

    # ---- Zoom: the lens typed in the value box ------------------------------
    def _handle_zoom_value_key(self, ev) -> bool:
        """With Zoom on, the value box takes the lens: «35mm» a focal
        length, «60deg», «60°» or a bare «60» the field of view
        (:meth:`apply_zoom_value`). Letters only follow a digit, so Zoom's
        neighbours' shortcuts still work before anything is typed."""
        key = ev.key()
        buf = self._value_buffer
        if key in (Qt.Key_Return, Qt.Key_Enter):
            if not buf:
                return False
            # The lens first: the box, emptied, then reads the new focal.
            if is_expression(buf):
                # A lens length has a unit. Bare lens angles are degrees.
                result = parse_units(buf, "angle", "deg")
                if result.ok:
                    buf = f"{math.degrees(result.value):.12f}deg"
                else:
                    result = parse_units(buf, "length", "mm")
                    if not result.ok:
                        self.flash_status(result.message(), 4000)
                        return True
                    buf = f"{result.value * 1000:.12f}mm"
            self.apply_zoom_value(buf)
            self._set_value_buffer("")
            return True
        if key == Qt.Key_Backspace:
            if not buf:
                return False
            self._set_value_buffer(buf[:-1])
            return True
        if key == Qt.Key_Escape and buf:
            self._set_value_buffer("")
            return True
        if _is_chord(getattr(ev, "modifiers", lambda: Qt.NoModifier)()):
            return False
        text = ev.text()
        if not text or len(text) != 1:
            return False
        if ((text == "=" and not buf) or (is_expression(buf) and text.isprintable())
                or text.isdigit() or text in (".", ",")):
            self._set_value_buffer(buf + text)
            return True
        if any(c.isdigit() for c in buf) and (
                text.lower() in ("m", "d", "e", "g", " ") or text == "°"):
            self._set_value_buffer(buf + text)
            return True
        return False

    def apply_zoom_value(self, typed: str) -> bool:
        """Set the lens from what was typed with Zoom on: ``35mm`` is a
        focal length on the camera's sensor (Camera Settings), ``60deg``,
        ``60°`` or ``60`` the vertical field of view. Only in perspective
        — a parallel view has no lens. ``False`` (and nothing changes) when
        it cannot be done; the status bar says why."""
        from core.camera import FOV_RANGE
        m = re.fullmatch(r"\s*(\d+(?:[.,]\d*)?|[.,]\d+)\s*(mm|deg|°)?\s*",
                         typed.lower())
        value = float(m.group(1).replace(",", ".")) if m else 0.0
        if m is None or not math.isfinite(value) or value <= 0.0:
            self.flash_status(tr(
                "Type a focal length like 35mm, or an angle like 60deg."))
            return False
        cam = self.camera
        if not cam.perspective:
            self.flash_status(tr(
                "Field of view applies to perspective; switch to "
                "Perspective first."))
            return False
        fov = cam.fov_for_focal(value) if m.group(2) == "mm" else value
        lo, hi = FOV_RANGE
        cam.fov_deg = float(min(hi, max(lo, fov)))
        self.update()
        self.flash_status(tr(
            "Lens {mm} mm, field of view {deg}°",
            mm=f"{cam.focal_mm:.0f}", deg=f"{cam.fov_deg:.1f}"), 4000)
        return True

    # ---- Numeric value buffer (VCB-style) ----------------------------------
    def _handle_value_key(self, ev) -> bool:
        """Buffer digit / dot / comma / semicolon / space / backspace.

        Enter applies the buffer via ``active_tool.on_value(...)``.

        Input forms:
        - ``"5"`` or ``"5,3"`` or ``"5.3"`` → single length (float).
        - ``"3;4;5"`` or ``"3 4 5"``       → 3D delta from the start point
                                              (passed as a ``(dx, dy, dz)`` tuple).
        - ``"-2"``                          → negative value (tools that take a
                                              direction flip it).
        - ``"30cm"`` / ``"1500mm"`` / ``"2m"`` → unit suffix per field; bare
                                              numbers are metres (project unit).
        Comma is the decimal separator; ``;`` and space are field
        separators (the usual convention adapted to our locale) -- except for
        a tool that only takes several values (``vcb_comma_lists``, the
        Rectangle): there ``200,100`` is two fields (#152).
        See :meth:`_parse_value_buffer`.
        """
        if self.active_tool is None:
            if getattr(self, "nav_mode", None) == "zoom":
                return self._handle_zoom_value_key(ev)
            return False

        text = ev.text()
        key = ev.key()

        if key in (Qt.Key_Return, Qt.Key_Enter):
            if not self._value_buffer:
                return False
            # An angle, a factor or a count is not a length: the tool says
            # so and the document's unit stays out of it (#176).
            unitless = getattr(self.active_tool, "value_is_unitless", None)
            if callable(unitless) and unitless():
                from core.units import unitless_numbers
                with unitless_numbers():
                    value = self._parse_value_buffer(
                        self._value_buffer,
                        comma_lists=getattr(self.active_tool,
                                            "vcb_comma_lists", False))
            else:
                value = self._parse_value_buffer(
                    self._value_buffer,
                    comma_lists=getattr(self.active_tool, "vcb_comma_lists",
                                        False))
            if value is None and is_expression(self._value_buffer):
                # A calculation that doesn't add up stays in the box with
                # the reason, to be fixed with Backspace — not thrown away.
                self.flash_status(
                    self._expression_error(self._value_buffer), 4000)
                return True
            if value is None:
                self._set_value_buffer("")
                return True
            if isinstance(value, tuple) and value and value[0] == "ratio":
                # A slope typed as rise:run ("3:12") — only angle
                # tools understand it; it reaches them as plain degrees.
                if getattr(self.active_tool, "accepts_angle_ratio", False):
                    self.active_tool.on_value(self, value[1])
                self._set_value_buffer("")
                return True
            if isinstance(value, tuple) and value and value[0] == "segments":
                handler = getattr(self.active_tool, "on_segments_value", None)
                if handler is not None:
                    handler(self, value[1])
                self._set_value_buffer("")
                return True
            if isinstance(value, tuple) and value and value[0] == "array":
                # Arrays: "3x" (external) / "/3" (internal) right
                # after a Move-copy. Only tools that declare it understand.
                handler = getattr(self.active_tool, "on_array_value", None)
                if handler is not None:
                    if len(value) <= 3:
                        handler(self, value[1], value[2])
                    elif "step" in inspect.signature(handler).parameters:
                        handler(self, value[1], value[2], step=value[3])
                    else:                       # "5x10m" means nothing here
                        self.flash_status(tr(
                            "Type the count alone here, e.g. 3x"))
                self._set_value_buffer("")
                return True
            if isinstance(value, tuple) and value and value[0] == "profile":
                # "0.3p": the profile of a bevel. Only tools that declare
                # it understand.
                handler = getattr(self.active_tool, "on_profile_value", None)
                if handler is not None:
                    handler(self, value[1])
                self._set_value_buffer("")
                return True
            if isinstance(value, tuple) and value and value[0] == "radius":
                # "2r": the 2-point arc takes a RADIUS instead
                # of the bulge. Only tools that declare it understand.
                handler = getattr(self.active_tool, "on_radius_value", None)
                if handler is not None:
                    handler(self, value[1])
                self._set_value_buffer("")
                return True
            if getattr(self.active_tool, "accepts_absolute_length", False):
                # Scale reads "2" as a factor but "2m" as the new
                # absolute size; the parser collapses both to metres, so the
                # unit's presence is re-detected here and tagged, the same
                # pattern as the arc's "2r" radius suffix.
                import re as _re
                if is_expression(self._value_buffer):
                    absolute = parse_units(self._value_buffer).explicit_measurement
                else:
                    absolute = _re.search(r"[\d\"']\s*(mm|cm|m|in|ft)\b|\d\s*[\"']",
                                          self._value_buffer.lower())
                if absolute:
                    value = ("abs_len", value)
            # Did this value actually commit geometry? Same signal as a
            # click that commits (see _dispatch_tool_click): a chaining tool
            # like Line stays busy the instant the segment exists, so "busy"
            # is not "the operation that used the arrow lock has ended". The
            # typed length must drop the lock exactly like the second click.
            history = getattr(self, "history", None)
            _before = len(history.undo_stack) if history is not None else 0
            _top = history.undo_stack[-1] if _before else None
            typed = self._value_buffer
            self.active_tool.on_value(self, value)
            _drew = history is not None and len(history.undo_stack) > _before
            # A hot retype (Push/Pull, Move, Rotate… typed right after a
            # commit) undoes the last command and commits it again: the
            # stack keeps its length but its top is a new command. That
            # also changed the model, and the box went on showing the
            # value from before the retype (tutorial video, #859).
            _redrew = (not _drew and _before > 0
                       and len(history.undo_stack) == _before
                       and history.undo_stack[-1] is not _top)
            self._set_value_buffer("")
            # The box falls back to the live reading, which only refreshes on
            # a mouse move: it went on showing the preview from BEFORE the
            # Enter, as if the typed size had been ignored (#607). What was
            # drawn is what was typed, read as the canvas showed it. Nothing
            # drawn: the preview the box shows is still the live one.
            if _drew or _redrew:
                unitless = getattr(self.active_tool, "value_is_unitless", None)
                plain = callable(unitless) and unitless()
                if is_expression(typed):
                    # «=1.25*2+2.45/2» drew 3.725 m: the box says so.
                    self.measurementChanged.emit(
                        _expression_result_text(value, plain))
                else:
                    self.measurementChanged.emit(
                        typed if plain else _typed_value_text(typed))
            self._release_axis_lock_after_operation(committed=_drew)
            return True

        if key == Qt.Key_Backspace:
            if not self._value_buffer:
                return False
            self._set_value_buffer(self._value_buffer[:-1])
            return True

        if _is_chord(getattr(ev, "modifiers", lambda: Qt.NoModifier)()):
            # Alt+1, Ctrl+2…: a shortcut, not a digit for the value box. It
            # was taken as one while a value was being typed, so the view
            # shortcut never fired and each try added a «1» — a 35-digit
            # number, inf in the geometry's floats, then NaN (#185).
            return False
        from core.units import VULGAR_FRACTIONS, plain_marks
        if text in VULGAR_FRACTIONS:
            # ⅜, ½…: the fraction as digits, after a space when it follows
            # a whole number (1⅜ → 1 3/8). A keyboard that types them, or
            # smart quotes (”), made the value box drop keys (#189).
            if not self._value_buffer:
                self._set_value_buffer(plain_marks(text))
            else:
                self._set_value_buffer(plain_marks(self._value_buffer + text))
            return True
        text = plain_marks(text)
        if text == "=" and not self._value_buffer:
            # «=» opens a calculation: =1.25*2+2.45/2 (#630).
            self._set_value_buffer("=")
            return True
        if is_expression(self._value_buffer):
            # Inside a calculation every printable key is part of it —
            # operators, parentheses, units — and none is a shortcut.
            if text and text.isprintable():
                self._set_value_buffer(self._value_buffer + text)
            return bool(text and text.isprintable())
        arrays = getattr(self.active_tool, "accepts_array", False)
        if arrays and text == "*":
            # "*3" / "3*": the array multiplier ("x" works too).
            self._set_value_buffer(self._value_buffer + text)
            return True
        if arrays and text.lower() == "x" and (
                self._current_token_tail()
                or (not self._value_buffer and self._array_pending())):
            # "3x" — after a digit; bare X stays the Text tool shortcut,
            # except right after a copy, where "x5" is the array written
            # multiplier first (#542), as common a habit as "5x".
            self._set_value_buffer(self._value_buffer + text)
            return True
        if (text.lower() == "p" and self._current_token_tail()
                and hasattr(self.active_tool, "on_profile_value")):
            # "0.3p" — only after a digit, and only for a tool that takes a
            # profile: bare P stays the Push/Pull shortcut.
            self._set_value_buffer(self._value_buffer + text)
            return True
        if arrays and text == "/" and not self._value_buffer:
            # "/3": the internal-array divisor opens the buffer.
            self._set_value_buffer(text)
            return True
        if text and (text.isdigit()
                     or text in (".", ",", ";", " ", "-", ":", "\"", "'", "/")
                     or text.lower() in ("m", "c", "r", "i", "n", "f", "t", "s")):
            # A field separator (space / ;) with an empty buffer isn't VCB
            # input — let it fall through so Space can act as the Select
            # shortcut. It only separates fields mid-number.
            if text in (";", " ") and not self._value_buffer:
                return False
            # A unit letter with an empty buffer is a tool shortcut (M = Move,
            # C = Circle, R = Rectangle), not VCB input — only buffer it
            # after a digit. "r" is the radius suffix for arcs (2r), "s"
            # the segment count of an arc / sides of a circle (16s).
            if (text.lower() in ("m", "c", "r", "i", "n", "f", "t", "s")
                    and not self._current_token_tail()):
                return False
            # Imperial marks and the fraction bar belong after a digit too:
            # 2", 1'6", 3/4" (bare they are nothing a tool wants).
            if text in ("\"", "'", "/") and not self._current_token_tail():
                return True
            # Minus only opens a token (a sign, not an operator).
            if text == "-" and self._current_token_tail():
                return True
            # Colon (slope rise:run, "3:12"): mid-token only, once.
            if text == ":":
                tail = self._current_token_tail()
                if not tail or ":" in self._value_buffer:
                    return True
            # A second decimal separator in the current numeric token: a
            # "." is dropped, a "," starts the next value (#598) — dropping
            # it turned «0.9,2.1» into 0.921.
            if text in (".", ","):
                tail = self._current_token_tail()
                comma_fields = (
                    getattr(self.active_tool, "vcb_comma_lists", False)
                    and ";" not in self._value_buffer
                    and " " not in self._value_buffer)
                if comma_fields:
                    # Every comma separates a field here (#152).
                    tail = tail.rsplit(",", 1)[-1]
                if "." in tail or "," in tail:
                    if text == ".":
                        return True
                    if not comma_fields:
                        text = ";"
            self._set_value_buffer(self._value_buffer + text)
            return True

        return False

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
        """Whether the active tool just made a copy that «x5» would turn
        into an array (#542)."""
        pending = getattr(self.active_tool, "array_pending", None)
        return bool(pending(self)) if callable(pending) else False

    @staticmethod
    def _expression_error(buffer: str) -> str:
        """Why the calculation in ``buffer`` can't be read, translated."""
        from core.units import plain_marks
        buffer = plain_marks(buffer)
        for field in _expression_fields(buffer):
            r = parse_units(field, "length")
            if not r.ok:
                return r.message()
        return tr("Type one, two or three values")

    def _current_token_tail(self) -> str:
        """The portion of the buffer after the last ``;`` or space."""
        normalized = self._value_buffer.replace(";", " ")
        idx = normalized.rfind(" ")
        if idx < 0:
            return self._value_buffer
        return self._value_buffer[idx + 1 :]

    def _set_value_buffer(self, text: str) -> None:
        if text == self._value_buffer:
            return
        self._value_buffer = text
        self.valueBufferChanged.emit(text)
        self.update()

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
        """The snap engine's answer for the cursor at ``(px_x, px_y)``.

        THE one code path: both ``_build_ctx`` (mouse motion and clicks) and
        ``_refresh_snap`` (a modifier changing with the hand still) go through
        here, so the :func:`compute_snap` argument list lives in exactly one
        place. Returns ``None`` when the pixel casts no world point (the ray
        misses the plane / the point is behind the camera). A tool that does
        not snap (Select, Push/Pull) gets a ``"none"`` result instead - no
        snap engine, no occlusion raycasts, no marker.

        A point that is not finite counts as no point: a NaN that reached a
        tool showed «nan m» in the length box and failed the Line's commit
        with «AddEdgeCommand: cannot convert float NaN to integer» (#466,
        #754). Whatever inference produced it, the cursor just gives
        nothing for that pixel.

        The answer is values, no face: whatever the search picks it only
        reads, so a big lazy mesh under the cursor stays unbuilt (#1092)."""
        from views.array_bake import reading_only
        with reading_only():
            snap = self.inference.snap(px_x, px_y, modifiers)
        if snap is not None and not _finite_point(snap.point):
            return None
        return snap

    def _build_ctx(self, ev) -> Optional[ToolContext]:
        self._sync_axes()
        p = ev.position().toPoint()
        snap = self._snap_at(p.x(), p.y(), ev.modifiers())
        if snap is None:
            return None
        return ToolContext(
            viewport=self,
            world=snap.point,
            screen=ev.position(),
            modifiers=ev.modifiers(),
            snap=snap,
        )

    def _note_snap(self, snap: SnapResult) -> None:
        """Record the cursor's snap and hand its KIND to the active tool
        (``Tool.snap_kind``).

        The kind is known only once the snap has been computed — the plane
        comes first, the point and its snap second — so a planar tool
        asking the viewport which face the cursor is over cannot be told
        whether the cursor is on that face as a FACE or snapped to a point
        that merely lies on it. The tool hears it here, on every hover and
        click, before it is called: that is where ``PlaneLock.
        snapped_plane`` decides whether the shape re-plans onto a point
        (#37) or stays on the face under the cursor (#545)."""
        self.last_snap = snap
        tool = self.active_tool
        if tool is not None:
            tool.snap_kind = getattr(snap, "kind", None)

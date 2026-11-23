# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Select's host side: what lies under a pixel, what a box catches, and
how a pick is selected or erased.

The Select tool itself is a Rust state machine
(``rust/ingetrazo_core/src/tools/select.rs``, adapter ``tools.rust_tools``):
the gesture, the modifiers, the click cycle and what a double or triple
click does. The queries here move to native picking next
(docs/rust-tools.md §12, docs/python-glue.md).
"""
from __future__ import annotations


from views import prompts as _prompts

from core.dimension import Dimension
from core.textlabel import TextLabel
from core.group import Group
from core.mesh import Edge, Face
from core.guide import is_guide
from core.section import SectionPlane
from core.history import (
    CompoundCommand,
    DeleteDimensionsCommand,
    DeleteGeoPathsCommand,
    DeleteGroupCommand,
    DeleteGuidesCommand,
    DeleteSectionPlanesCommand,
    DeleteTextLabelsCommand,
    SetActiveSectionCommand,
    EditTextLabelCommand,
    EraseSelectionCommand,
)
from georef.geopath import GeoPath



def _seg_rect_mask(ax, ay, bx, by, ok, rect, crossing):
    """Vectorized rect test for N screen segments: window mode = both
    endpoints inside; crossing mode = an endpoint inside or the segment
    intersecting one of the four borders. NumPy twin of _pt_in_rect /
    _seg_rect_overlap for import-scale meshes."""
    import numpy as np
    x0, y0, x1, y1 = rect
    in_a = ok & (ax >= x0) & (ax <= x1) & (ay >= y0) & (ay <= y1)
    in_b = ok & (bx >= x0) & (bx <= x1) & (by >= y0) & (by <= y1)
    if not crossing:
        return in_a & in_b
    cross = in_a | in_b
    todo = ok & ~cross
    if todo.any():
        for px, py, qx, qy in ((x0, y0, x1, y0), (x1, y0, x1, y1),
                               (x1, y1, x0, y1), (x0, y1, x0, y0)):
            d1 = (qx - px) * (ay - py) - (qy - py) * (ax - px)
            d2 = (qx - px) * (by - py) - (qy - py) * (bx - px)
            d3 = (bx - ax) * (py - ay) - (by - ay) * (px - ax)
            d4 = (bx - ax) * (qy - ay) - (by - ay) * (qx - ax)
            hit = todo & (d1 * d2 < 0) & (d3 * d4 < 0)
            cross |= hit
            todo &= ~hit
            if not todo.any():
                break
    return cross


def _box_loose_fast(viewport, rect, crossing):
    """Vectorized box select over the loose mesh: edges via the cached
    screen projection, faces derived from their boundary edges. Returns
    (edges, faces) or None to fall back to the per-entity walk (stub
    viewports in tests, or a viewport without the caches)."""
    if (getattr(viewport, "_ledge_screen", None) is None
            or getattr(viewport, "_pick_index", None) is None):
        return None
    try:
        proj = viewport._ledge_screen()
        idx = viewport._pick_index()
    except Exception:  # noqa: BLE001 — any stub oddity: python path
        return None
    if proj is None or idx.edge_a is None:
        return None
    import numpy as np
    ax, ay, bx, by, ok = proj
    edges_all = idx.edges
    if len(edges_all) != len(ax):
        return None
    mask = _seg_rect_mask(ax, ay, bx, by, ok, rect, crossing)
    sel = viewport.scene.entity_selectable
    # What the context lets a click take. The index also carries the rest of
    # the model's loose edges (a reference while a group is open); those are
    # drawn faded and must not be caught by the box.
    pick_ok = getattr(idx, "edge_sel", None)
    hit_idx = np.where(mask)[0]
    if pick_ok is not None:
        hit_idx = hit_idx[pick_ok[hit_idx]]
    edges = [edges_all[int(i)] for i in hit_idx
             if sel(edges_all[int(i)])
             and not getattr(edges_all[int(i)], "hidden", False)]
    faces = []
    if crossing:
        seen = set()
        for i in hit_idx:
            for f in edges_all[int(i)].faces:
                if f.key not in seen:
                    seen.add(f.key)
                    if sel(f):
                        faces.append(f)
    else:
        hit_ids = {edges_all[int(i)].key for i in hit_idx}
        face_edges: dict = {}
        for e in edges_all:
            for f in e.faces:
                face_edges.setdefault(f.key, []).append(e)
        for f in viewport.scene.faces:
            es = face_edges.get(f.key)
            if es and all(e.key in hit_ids for e in es) and sel(f):
                faces.append(f)
    return edges, faces


def _box_loose_arrays(viewport, rect, crossing):
    """The loose mesh's box test from the store's dense arrays, for a view
    without the GL caches (the Metal view): one projection of every vertex,
    vectorised edge and face tests, and element views made only for what
    the box takes. Returns (edges, faces), or None when the loose mesh is
    not store-backed (the per-entity walk then runs)."""
    project = getattr(viewport, "_project_px", None)
    scene = viewport.scene
    mesh = getattr(scene, "mesh", None)
    if project is None or mesh is None:
        return None
    from core.store_mesh import StoreMesh
    if mesh.__class__ is not StoreMesh:
        return None
    import numpy as np
    try:
        a = mesh.__dict__["_store"].arrays()
        pos = np.asarray(a["pos"], dtype=float).reshape(-1, 3)
        if not len(pos):
            return [], []
        xs, ys, ok = project(pos)
    except Exception:  # noqa: BLE001 — a stub viewport: the walk answers
        return None
    x0, y0, x1, y1 = rect
    inside = ok & (xs >= x0) & (xs <= x1) & (ys >= y0) & (ys <= y1)
    ends = np.asarray(a["edges"], dtype=np.int64).reshape(-1, 2)
    ea, eb = ends[:, 0], ends[:, 1]
    # The walk skips what has an end behind the eye, in both modes.
    emask = _seg_rect_mask(xs[ea], ys[ea], xs[eb], ys[eb], ok[ea] & ok[eb],
                           rect, crossing)
    corners = np.asarray(a["corners"], dtype=np.int64)
    sizes = np.asarray(a["ring_sizes"], dtype=np.int64)
    counts = np.asarray(a["ring_counts"], dtype=np.int64)
    fmask = np.zeros(len(counts), dtype=bool)
    if len(corners) and len(counts):
        # Corners per face, then each face's first corner.
        ring_face = np.repeat(np.arange(len(counts)), counts)
        per_face = np.bincount(ring_face, weights=sizes,
                               minlength=len(counts)).astype(np.int64)
        starts = np.concatenate(([0], np.cumsum(per_face)[:-1]))
        live = per_face > 0
        s = starts[live]
        face_ok = np.zeros(len(counts), dtype=bool)
        face_ok[live] = np.logical_and.reduceat(ok[corners], s)
        if not crossing:
            hit = np.zeros(len(counts), dtype=bool)
            hit[live] = np.logical_and.reduceat(inside[corners], s)
        else:
            # Each ring's sides: corner i to the next corner of its ring.
            nxt = np.arange(len(corners)) + 1
            ring_end = np.cumsum(sizes)
            nxt[ring_end - 1] = ring_end - sizes
            ca, cb = corners, corners[nxt]
            side = _seg_rect_mask(xs[ca], ys[ca], xs[cb], ys[cb],
                                  ok[ca] & ok[cb], rect, True)
            hit = np.zeros(len(counts), dtype=bool)
            hit[live] = np.logical_or.reduceat(side, s)
        fmask = hit & face_ok
    sel = scene.entity_selectable
    edges = []
    seq = mesh.edges
    for i in np.flatnonzero(emask):
        e = seq[int(i)]
        if sel(e):
            edges.append(e)
    faces = []
    seq = mesh.faces
    for i in np.flatnonzero(fmask):
        f = seq[int(i)]
        if sel(f):
            faces.append(f)
    return edges, faces


# Points (or segment ends) one exact test of a packed instance projects at
# once: bounded scratch whatever the prototype's or the scene's size.
_PACKED_BATCH = 1 << 18

#: The eight corners of a box from its (lo, hi): which coordinates take hi.
_BOX_CORNERS = None


def _box_corner_mask():
    global _BOX_CORNERS
    if _BOX_CORNERS is None:
        import numpy as np
        _BOX_CORNERS = np.array([[x, y, z] for x in (0, 1) for y in (0, 1)
                                 for z in (0, 1)], dtype=bool)
    return _BOX_CORNERS


def _packed_view(viewport):
    """The packed scene the Metal view draws, when ``viewport`` is its
    tool facade (the GL viewport has its own chunk caches instead)."""
    if getattr(viewport, "_group_chunk", None) is not None:
        return None
    view = getattr(viewport, "view", None)
    packed = getattr(view, "packed", None)
    if (packed is None or getattr(packed, "groups", None) is None
            or getattr(viewport, "_project_px", None) is None):
        return None
    import numpy as np
    if not isinstance(getattr(packed, "instances", None), np.ndarray):
        return None
    return packed


def _packed_world(points, world):
    """``points`` (K, 3) placed by each of ``world`` (R, 4, 4) — Qt's
    column-major matrices as packed, so a row vector times the 3×3 plus
    the last row — as (R, K, 3) float64."""
    import numpy as np
    w = world.astype(np.float64)
    return (np.einsum("kj,rji->rki", points, w[:, :3, :3])
            + w[:, None, 3, :3])


def _packed_rows_exact(viewport, packed, code, world, rect, crossing):
    """Exact box test of the instances of prototype ``code`` placed by
    ``world`` (R, 4, 4): per instance, every triangle corner and edge end
    inside (window), or one inside or a side or edge crossing the box
    (crossing). Projects at most ``_PACKED_BATCH`` points at a time."""
    import numpy as np
    pos = packed.vertices["pos"]
    c, e = packed.commands[code], packed.edge_commands[code]
    first, count, base = int(c["first"]), int(c["count"]), int(c["base"])
    tri = pos[base + packed.indices[first:first + count].astype(np.int64)]
    first, count, base = int(e["first"]), int(e["count"]), int(e["base"])
    edg = pos[base + packed.indices[first:first + count].astype(np.int64)]
    points = np.concatenate((tri, edg)).astype(np.float64)
    n_rows = len(world)
    if not len(points):
        return np.zeros(n_rows, dtype=bool)
    x0, y0, x1, y1 = rect
    result = np.ones(n_rows, dtype=bool) if not crossing \
        else np.zeros(n_rows, dtype=bool)

    def chunks(k):
        """(row slice, point slice) pairs of at most _PACKED_BATCH points."""
        rows_at = max(1, _PACKED_BATCH // max(1, k))
        step = min(k, _PACKED_BATCH)
        for r in range(0, n_rows, rows_at):
            for p in range(0, k, step):
                yield slice(r, min(n_rows, r + rows_at)), slice(p, p + step)

    for rs, ps in chunks(len(points)):
        todo = result[rs] if not crossing else ~result[rs]
        if not todo.any():
            continue
        placed = _packed_world(points[ps], world[rs])
        xs, ys, ok = viewport._project_px(placed.reshape(-1, 3))
        shape = placed.shape[:2]
        xs, ys, ok = xs.reshape(shape), ys.reshape(shape), ok.reshape(shape)
        inside = ok & (xs >= x0) & (xs <= x1) & (ys >= y0) & (ys <= y1)
        if crossing:
            result[rs] |= inside.any(axis=1)
        else:
            result[rs] &= inside.all(axis=1)
    if not crossing or result.all():
        return result
    # Crossing, no point inside: a triangle side or an edge through the box.
    sides_a = np.concatenate((tri[0::3], tri[1::3], tri[2::3], edg[0::2]))
    sides_b = np.concatenate((tri[1::3], tri[2::3], tri[0::3], edg[1::2]))
    sides_a = sides_a.astype(np.float64)
    sides_b = sides_b.astype(np.float64)
    for rs, ps in chunks(2 * len(sides_a)):
        sl = slice(ps.start // 2, ps.stop // 2)
        todo = ~result[rs]
        if not todo.any() or sl.start >= len(sides_a):
            continue
        wa = _packed_world(sides_a[sl], world[rs])
        wb = _packed_world(sides_b[sl], world[rs])
        ax, ay, oka = viewport._project_px(wa.reshape(-1, 3))
        bx, by, okb = viewport._project_px(wb.reshape(-1, 3))
        cross = _seg_rect_mask(ax, ay, bx, by, oka & okb, rect, True)
        result[rs] |= cross.reshape(wa.shape[:2]).any(axis=1)
    return result


def _box_groups_packed(viewport, candidates, rect, crossing):
    """Box test of every candidate object at once from the Metal view's
    packed scene: per instance its prototype's box carried by its matrix
    and projected (eight corners), and the exact per-point test only for
    the instances whose box straddles the rectangle — one numpy pass over
    the instances instead of a Python walk over every vertex of every
    placement (the ride: 91k placements, 14M faces, minutes on the main
    thread). A linked file is one candidate, like a component: its
    placements decide for it as a whole.

    Returns ``(hits, tested)`` — sets of ``id(candidate)`` — or None when
    there is no packed scene. Candidates with no packed instance at all
    are not in ``tested``: the caller tests them its usual way."""
    packed = _packed_view(viewport)
    if packed is None or not candidates:
        return None
    import numpy as np
    owner_of: dict = {}
    for i, group in enumerate(candidates):
        stack = [group]
        while stack:
            node = stack.pop()
            owner_of[id(node)] = i
            stack.extend(getattr(node, "children", None) or ())
    groups = packed.groups
    owner = np.fromiter((owner_of.get(id(g), -1) if g is not None else -1
                         for g in groups), dtype=np.int64, count=len(groups))
    if len(owner) != len(packed.instances):
        return None
    rows = np.flatnonzero(owner >= 0)
    tested = {id(candidates[i]) for i in np.unique(owner[rows])}
    info = packed.instances["info"]
    # What the view does not draw (hidden objects, tags off) takes no part.
    rows = rows[(info[rows, 2] & 1) == 0]
    hits: set = set()
    if not len(rows):
        return hits, tested
    codes = info[rows, 0].astype(np.int64)
    world = packed.instances["world"][rows]
    lo = packed.bounds[codes, 0, :3].astype(np.float64)
    hi = packed.bounds[codes, 1, :3].astype(np.float64)
    mask = _box_corner_mask()
    corners = np.where(mask[None], hi[:, None, :], lo[:, None, :])
    w = world.astype(np.float64)
    placed = (np.einsum("nkj,nji->nki", corners, w[:, :3, :3])
              + w[:, None, 3, :3])
    xs, ys, ok = viewport._project_px(placed.reshape(-1, 3))
    xs, ys, ok = (xs.reshape(-1, 8), ys.reshape(-1, 8), ok.reshape(-1, 8))
    x0, y0, x1, y1 = rect
    front = ok.all(axis=1)
    inside = front & ((xs >= x0) & (xs <= x1)
                      & (ys >= y0) & (ys <= y1)).all(axis=1)
    out = front & ((xs < x0).all(axis=1) | (xs > x1).all(axis=1)
                   | (ys < y0).all(axis=1) | (ys > y1).all(axis=1))
    unsure = ~inside & ~out
    row_owner = owner[rows]
    n = len(candidates)
    if crossing:
        hit = np.zeros(n, dtype=bool)
        hit[row_owner[inside]] = True
    else:
        present = np.zeros(n, dtype=bool)
        present[row_owner] = True
        hit = present.copy()
        hit[row_owner[out]] = False
    # The straddling instances, prototype by prototype, skipping those
    # whose object is already decided.
    todo = np.flatnonzero(unsure)
    if len(todo):
        order = todo[np.argsort(codes[todo], kind="stable")]
        bounds = np.flatnonzero(np.diff(codes[order])) + 1
        for group_rows in np.split(order, bounds):
            live = group_rows[hit[row_owner[group_rows]] == (not crossing)]
            if not len(live):
                continue
            res = _packed_rows_exact(viewport, packed, int(codes[live[0]]),
                                     world[live], rect, crossing)
            if crossing:
                hit[row_owner[live[res]]] = True
            else:
                hit[row_owner[live[~res]]] = False
    hits = {id(candidates[i]) for i in np.flatnonzero(hit)}
    return hits, tested


# Bound temporary triangle/edge arrays and camera projections independently
# of the import's size (#1175). Even-sized batches keep edge pairs together.
_BOX_BATCH_POINTS = 8192


def _box_source(viewport, pg):
    """A placement's cached world chunk, or its prototype and lazy maps.

    An uncached instance must not materialize its entire transformed mesh
    just to answer a selection rectangle.
    """
    xf = getattr(pg, "xform", None)
    if xf is None or getattr(pg, "mesh", None) is None:
        return viewport._group_chunk(pg), None, None
    from views.document_interaction import _paint_of, _placed_maps, _proto_wrapper
    base = viewport._group_chunk(
        _proto_wrapper(viewport, pg.mesh, _paint_of(viewport, pg)))
    cur = (getattr(viewport, "_inst_chunks", None) or {}).get(id(pg))
    if cur is not None and cur["ikey"] == (base["uid"], tuple(xf.data())):
        return cur, None, None
    _L, tv, tp = _placed_maps(xf)
    return base, tv, tp


def _box_point_batches(ch, tv=None, tp=None):
    """Yield ``(points, paired_edges)`` in bounded batches, without writes.

    Match the renderer's term order: transform each triangle's origin and
    directions separately before adding them. Placed edge buffers round to
    float32, just as an instance chunk does.
    """
    import numpy as np
    v0, e1, e2 = ch["v0"], ch["e1"], ch["e2"]
    if v0 is not None:
        for i in range(0, len(v0), _BOX_BATCH_POINTS):
            sl = slice(i, i + _BOX_BATCH_POINTS)
            a = tp(v0[sl]) if tp is not None else v0[sl]
            yield a, False
            yield a + (tv(e1[sl]) if tv is not None else e1[sl]), False
            yield a + (tv(e2[sl]) if tv is not None else e2[sl]), False
    pair = np.frombuffer(ch["edges"], np.float32).reshape(-1, 3)
    for i in range(0, len(pair), _BOX_BATCH_POINTS):
        points = pair[i:i + _BOX_BATCH_POINTS].astype(np.float64)
        if tp is not None:
            points = tp(points).astype(np.float32).astype(np.float64)
        yield points, True


def _local_bounds(ch):
    """``(lo, hi)`` of everything the box test reads of chunk ``ch`` (the
    triangle corners and the edges), cached on it; None when empty."""
    import numpy as np
    import weakref
    # Keyed on the very arrays: a moved chunk gets new ones (a shift
    # rebuilds them), and stale bounds would answer for where it was.
    key = (ch["v0"], ch["e1"], ch["e2"], ch["edges"])
    hit = ch.get("_box_bounds")
    if (hit is not None and len(hit) == 3 and isinstance(hit[0], tuple)
            and hit[1] is key[3]
            and all((ref is None and array is None)
                    or (array is not None and ref is not None and ref() is array)
                    for ref, array in zip(hit[0], key[:3]))):
        return hit[2]
    lo = hi = None
    for points, _paired in _box_point_batches(ch):
        low, high = points.min(axis=0), points.max(axis=0)
        lo = low if lo is None else np.minimum(lo, low)
        hi = high if hi is None else np.maximum(hi, high)
    bounds = (lo, hi) if lo is not None else None
    # Do not retain obsolete triangle arrays after a chunk changes.
    refs = tuple(weakref.ref(a) if a is not None else None for a in key[:3])
    ch["_box_bounds"] = (refs, key[3], bounds)
    return bounds


def _placement_side(viewport, pg, rect):
    """``"in"`` when everything the box test reads of placement ``pg``
    projects inside ``rect``, ``"out"`` when all of it lies past one side
    of it, ``"empty"`` when it has nothing, None when that takes the full
    test. Decided on the eight
    corners of its bounding box, all in front of the camera: a projection
    keeps what is inside a box inside the hull of its corners."""
    import numpy as np
    xf = getattr(pg, "xform", None)
    if xf is None or getattr(pg, "mesh", None) is None:
        ch = viewport._group_chunk(pg)
        tp = None
    else:
        from views.document_interaction import _paint_of, _placed_maps, _proto_wrapper
        ch = viewport._group_chunk(
            _proto_wrapper(viewport, pg.mesh, _paint_of(viewport, pg)))
        tp = _placed_maps(xf)[2]
    bb = _local_bounds(ch)
    if bb is None:
        return "empty"
    lo, hi = bb
    box = np.array([[x, y, z] for x in (lo[0], hi[0])
                    for y in (lo[1], hi[1]) for z in (lo[2], hi[2])])
    if tp is not None:
        box = tp(box)
    xs, ys, ok = viewport._project_px(box)
    if not ok.all():
        return None
    x0, y0, x1, y1 = rect
    if ((xs >= x0) & (xs <= x1) & (ys >= y0) & (ys <= y1)).all():
        return "in"
    if (xs < x0).all() or (xs > x1).all() or (ys < y0).all() \
            or (ys > y1).all():
        return "out"
    return None


def _box_group_fast(viewport, group, rect, crossing):
    """Vectorized box test for one (non-billboard) group via its chunk
    arrays. True/False, or None to fall back to the per-vertex walk."""
    if (getattr(group, "billboard", False)
            or getattr(viewport, "_group_chunk", None) is None
            or getattr(viewport, "_project_px", None) is None):
        return None
    # A component's geometry lives in the placements it holds, each with its
    # own chunk: reading only the top-level one found an EMPTY array for an
    # imported component and dropped the whole test to the per-vertex Python
    # walk — 1.15M vertices for the hedge.
    expand = getattr(viewport, "_expand_placements", None)
    places = expand(group) if expand is not None else [group]
    seen = False
    try:
        for pg in places:
            # A placement whose box lands wholly inside the rectangle, or
            # wholly past one of its sides, answers without its triangles:
            # on a model of 2 000 instances the corners of every triangle
            # carried to world and projected were most of a box's 5 s.
            side = _placement_side(viewport, pg, rect)
            if side == "empty":
                continue
            seen = True
            if side == "out":
                if not crossing:
                    return False
                continue
            if side == "in":
                if crossing:
                    return True
                continue
            ch, tv, tp = _box_source(viewport, pg)
            for points, paired in _box_point_batches(ch, tv, tp):
                xs, ys, ok = viewport._project_px(points)
                x0, y0, x1, y1 = rect
                inside = ok & (xs >= x0) & (xs <= x1) & (ys >= y0) & (ys <= y1)
                if not crossing:
                    if not inside.all():
                        return False
                elif inside.any():
                    return True
                elif paired and _seg_rect_mask(
                        xs[0::2], ys[0::2], xs[1::2], ys[1::2],
                        ok[0::2] & ok[1::2], rect, True).any():
                    return True
    except Exception:  # noqa: BLE001
        return None
    return (not crossing) if seen else None


def _frame_projector(viewport):
    """``viewport._world_to_pixel`` with this frame's projection*view built
    once: the box walks every vertex of the groups it cannot test from
    their chunks (face-me figures), and rebuilding the camera matrices for
    each point cost ~1.6 s of a 3 s box on the villa of issue #1006. Same
    math, same pixels."""
    w2p = viewport._world_to_pixel
    cam = getattr(viewport, "camera", None)
    try:
        import inspect
        if "mvp" not in inspect.signature(w2p).parameters:
            return w2p
        mvp = cam.projection_matrix() * cam.view_matrix()
    except (AttributeError, TypeError, ValueError):
        return w2p                      # a stand-in viewport (tests)
    return lambda point: w2p(point, mvp)


def _pt_in_rect(p, rect) -> bool:
    return rect[0] <= p[0] <= rect[2] and rect[1] <= p[1] <= rect[3]


def _seg_seg_2d(p1, p2, p3, p4) -> bool:
    """Whether 2D segments ``p1-p2`` and ``p3-p4`` properly cross."""
    def ccw(a, b, c):
        return (c[1] - a[1]) * (b[0] - a[0]) - (b[1] - a[1]) * (c[0] - a[0])
    d1 = ccw(p3, p4, p1)
    d2 = ccw(p3, p4, p2)
    d3 = ccw(p1, p2, p3)
    d4 = ccw(p1, p2, p4)
    return (d1 > 0) != (d2 > 0) and (d3 > 0) != (d4 > 0)


def _seg_rect_overlap(a, b, rect) -> bool:
    """Whether segment ``a-b`` touches the rectangle (endpoint inside or an
    edge crossing) — the crossing-selection test."""
    if _pt_in_rect(a, rect) or _pt_in_rect(b, rect):
        return True
    x0, y0, x1, y1 = rect
    corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    return any(_seg_seg_2d(a, b, corners[i], corners[(i + 1) % 4]) for i in range(4))


def describe_pick(entity) -> str:
    """A few words for the status bar naming what a click picked."""
    from core.i18n import tr
    if isinstance(entity, Group):
        kind = tr("Component") if entity.is_instance() else tr("Group")
        name = (getattr(entity, "name", "") or "").strip()
        return f"{kind} «{name}»" if name else kind
    for cls, word in ((Edge, "Edge"), (Face, "Face"), (Dimension, "Dimension"),
                      (TextLabel, "Text"), (SectionPlane, "Section Plane")):
        if isinstance(entity, cls):
            return tr(word)
    return tr("Image") if type(entity).__name__ == "ImagePlane" \
        else type(entity).__name__


# ---- Host queries for the Rust Select (tools/rust_tools.py) ------------------
# The machine (rust/ingetrazo_core/src/tools/select.rs) owns the gesture,
# the modes, the click cycle and what a double or triple click does. These
# answer what lies under a pixel and apply a selection, until picking is
# native (docs/rust-tools.md §12, docs/python-glue.md).

#: A repeated plain click within this many pixels takes the next candidate
#: (the machine's ``CYCLE_PX``).
CYCLE_PX = 4.0


def linework(viewport, screen_x: float, screen_y: float) -> list:
    """``[(entity, pixels)]``, nearest first: the visible lines within
    the pick threshold — a loose edge, or an object made of lines only
    — that a click takes BEFORE the face under the cursor, whoever owns
    that face, as a thin line is the more deliberate target. A loose
    edge wins a tie. Empty on a viewport without the linework picks."""
    found = []
    pick_edge = getattr(viewport, "pick_linework_edge", None)
    if pick_edge is not None:
        edge, d = pick_edge(screen_x, screen_y)
        if edge is not None:
            found.append((edge, d))
    pick_groups = getattr(viewport, "pick_linework_groups", None)
    if pick_groups is not None:
        found.extend(pick_groups(screen_x, screen_y))
    found.sort(key=lambda hit: hit[1])
    return found


def pick(viewport, screen_x: float, screen_y: float, lines: bool = True):
    """The visible line under the cursor first (:func:`linework`),
    then a group (picked as a unit), then the edge under the cursor
    (screen-space priority), then a dimension annotation, then the
    front face; last, an object of lines only whose box holds the
    cursor. ``lines=False``: what the click takes once the lines
    are passed over (the next candidates of a repeated click)."""
    pick_label = getattr(viewport, "pick_text_label", None)
    if pick_label is not None:
        # The text block overdraws all geometry, so a click on the glyphs
        # is unambiguous — it outranks every 3D pick. The label's thin
        # leader line keeps its normal (post-edge) priority below.
        label = pick_label(screen_x, screen_y, rect_only=True)
        if label is not None:
            return label
    has_linework = hasattr(viewport, "pick_linework_edge")
    if lines and has_linework:
        # A line on a group's face (loose, or grouped as a hatch), or a
        # hair above or below it, outranks the object behind; only a
        # line the user can see — one hidden behind the faces leaves
        # the object as the pick.
        found = linework(viewport, screen_x, screen_y)
        if found:
            return found[0][0]
    group = viewport.pick_group(screen_x, screen_y)
    if has_linework:
        edge = None     # linework answered for the visible edges
    else:
        # Only an edge the user can SEE takes the click (issue #71): the
        # hidden seams of a smooth surface, or edges behind the model,
        # left a cylinder's side all but unclickable.
        pick_visible = getattr(viewport, "pick_visible_edge", None)
        edge = (pick_visible or viewport.pick_edge)(screen_x, screen_y)
    if group is not None and edge is not None:
        point = getattr(viewport, "edge_point_under_cursor", None)
        occluded = getattr(viewport, "_is_occluded", None)
        if point is not None and occluded is not None:
            p = point(edge, screen_x, screen_y)
            if p is not None and not occluded(p):
                return edge
    if group is not None:
        return group
    if edge is not None:
        return edge
    rest = pick_rest(viewport, screen_x, screen_y)
    if rest is not None:
        return rest
    # Between the lines of a hatch with nothing else under the cursor:
    # the object of lines whose box holds it (the smallest).
    around = getattr(viewport, "linework_groups_around", None)
    boxes = around(screen_x, screen_y) if around is not None else ()
    return boxes[0] if boxes else None


def pick_rest(viewport, screen_x: float, screen_y: float):
    """:func:`pick` past the lines and the groups: a dimension, a
    label, a geo path, a section plane, a guide, the front face, a
    reference image."""
    dim = viewport.pick_dimension(screen_x, screen_y)
    if dim is not None:
        return dim
    pick_label = getattr(viewport, "pick_text_label", None)
    label = pick_label(screen_x, screen_y) if pick_label else None
    if label is not None:
        return label
    path = viewport.pick_geopath(screen_x, screen_y)
    if path is not None:
        return path
    # Section planes pick by their frame, above guides.
    pick_sec = getattr(viewport, "pick_section_plane", None)
    sec = pick_sec(screen_x, screen_y) if pick_sec else None
    if sec is not None:
        return sec
    # Guides select the usual way (click + Delete / right-click). Real
    # geometry outranks them; a guide crossing a face still beats the face
    # (a thin line is the more deliberate target).
    pick_guide = getattr(viewport, "pick_guide", None)
    guide = pick_guide(screen_x, screen_y) if pick_guide else None
    if guide is not None:
        return guide
    face = viewport.pick_face(screen_x, screen_y)
    if face is not None:
        # Unless a picture lies in front of it (#426).
        in_front = getattr(viewport, "image_in_front_of", None)
        image = in_front(face, screen_x, screen_y) if in_front else None
        return image if image is not None else face
    # A reference image is the last candidate: it is an underlay, so
    # anything drawn on top of it must win the click. Only bare picture
    # selects the picture — which is how you then move or resize it.
    pick_image = getattr(viewport, "pick_image_plane", None)
    return pick_image(screen_x, screen_y) if pick_image else None


def candidates(viewport, screen_x: float, screen_y: float) -> list:
    """Everything a click at ``(x, y)`` may select, in the order
    repeated clicks there go through them: what a click takes
    (:func:`pick`), the other lines within reach, what the click
    takes past the lines (the face's object, the face), the objects
    of lines whose box holds the cursor. Each once."""
    found: list = []

    def add(entity):
        if entity is not None and all(entity != e for e in found):
            found.append(entity)
    add(pick(viewport, screen_x, screen_y))
    for entity, _d in linework(viewport, screen_x, screen_y):
        add(entity)
    add(pick(viewport, screen_x, screen_y, lines=False))
    add(viewport.pick_group(screen_x, screen_y))
    add(pick_rest(viewport, screen_x, screen_y))
    around = getattr(viewport, "linework_groups_around", None)
    for entity in (around(screen_x, screen_y) if around else ()):
        add(entity)
    return found


def pick_kind(entity) -> str:
    """The kind the machine decides a double click by."""
    for cls, kind in ((Face, "face"), (Edge, "edge"), (Group, "group"),
                      (SectionPlane, "section_plane"), (TextLabel, "text_label"),
                      (Dimension, "dimension")):
        if isinstance(entity, cls):
            return kind
    return "other"


def cycle_status(viewport, index: int, count: int, entity) -> None:
    flash = getattr(viewport, "flash_status", None)
    if callable(flash):
        from core.i18n import tr
        flash(tr("{index} of {count}: {what} — click again for "
                 "the next").format(index=index + 1, count=count,
                                    what=describe_pick(entity)))


def _bounded(viewport, entity) -> list:
    """A double click's catch: a face with its bounding edges, an edge
    with its faces."""
    mesh = viewport.scene.mesh
    picked = list(expand_pick(viewport, entity))
    if isinstance(entity, Face):
        for f in list(picked):
            if not isinstance(f, Face):
                continue
            for lp in (f.loop, *f.hole_loops):
                n = len(lp)
                for i in range(n):
                    e = mesh.find_edge(lp[i], lp[(i + 1) % n])
                    if e is not None:
                        picked.append(e)
    else:
        for e in list(picked):
            if isinstance(e, Edge):
                picked.extend(e.faces)
    return picked


def apply(viewport, entities, mode: str, grow: str) -> None:
    """Select what the machine picked, grown as it says: ``natural`` (a
    smooth surface, a whole curve), ``bounded`` (double click) or
    ``connected`` (triple click: the whole solid)."""
    picked = []
    for entity in entities:
        if grow == "connected":
            from core.select_ops import all_connected
            picked.extend(all_connected([entity]))
        elif grow == "bounded":
            picked.extend(_bounded(viewport, entity))
        else:
            picked.extend(expand_pick(viewport, entity))
    viewport.scene.select(picked, mode=mode)


def _toggle_section(viewport, entity) -> None:
    # Double-clicking a section plane toggles the active cut.
    viewport.history.execute(SetActiveSectionCommand(
        None if entity.active else entity))
    viewport.scene.select([entity])


def _edit_text(viewport, entity) -> None:
    # As usual, double-clicking a leader text edits its text.
    from core.i18n import tr
    from views import chrome_forms

    def edit(text):
        if text is not None and text.strip() and text.strip() != entity.text:
            viewport.history.execute(
                EditTextLabelCommand(entity, text.strip()))
        viewport.update()
    chrome_forms.ask_text(viewport.window(), tr("Text"), tr("Label text:"),
                          entity.text, done=edit, multiline=True)


def _edit_dimension(viewport, entity) -> None:
    # Double-clicking the dimension text edits it; "<>" stands for the
    # measured value, and an empty text (or a bare "<>") goes back to the
    # automatic value.
    from core.i18n import tr
    from core.history import EditDimensionTextCommand
    fmt = getattr(viewport, "_format_dim_value", None)
    style = getattr(viewport.scene, "dimension_style", {}) or {}
    measured = (fmt(entity.value(), style) if fmt is not None
                else entity.label())
    current = entity.text if entity.text else measured
    from views import chrome_forms

    def retext(text):
        if text is not None:
            new = text.strip()
            if new in ("", "<>", measured):
                new = None
            if new != entity.text:
                viewport.history.execute(
                    EditDimensionTextCommand(entity, new))
        viewport.update()
    chrome_forms.ask_text(viewport.window(), tr("Dimension"),
                          tr("Dimension text (<> = measured value):"),
                          current, done=retext)


#: The double-click verbs the host still owns (the machine names them).
HOST_VERBS = {"toggle_section": _toggle_section, "edit_text": _edit_text,
              "edit_dimension": _edit_dimension}


def box_select(viewport, rect, crossing: bool, mode: str) -> None:
    """What a released box catches (``rect`` normalised, ``crossing`` when
    dragged right to left), selected the way ``mode`` says."""
    w2p = _frame_projector(viewport)
    picked = []
    fast = _box_loose_fast(viewport, rect, crossing)
    if fast is None:
        fast = _box_loose_arrays(viewport, rect, crossing)
    if fast is not None:
        picked.extend(fast[0])
        picked.extend(fast[1])
    loose_iter = ((), ()) if fast is not None else (
        viewport.scene.edges, viewport.scene.faces)
    for edge in loose_iter[0]:
        if not viewport.scene.entity_selectable(edge):
            continue                        # hidden or locked layer
        pa = w2p(edge.a)
        pb = w2p(edge.b)
        if pa is None or pb is None:
            continue
        if crossing:
            if _seg_rect_overlap(pa, pb, rect):
                picked.append(edge)
        elif _pt_in_rect(pa, rect) and _pt_in_rect(pb, rect):
            picked.append(edge)
    for face in loose_iter[1]:
        if not viewport.scene.entity_selectable(face):
            continue                        # hidden or locked layer
        pts = [w2p(v) for v in face.vertices]
        if any(p is None for p in pts):
            continue
        if crossing:
            n = len(pts)
            touches = any(_pt_in_rect(p, rect) for p in pts) or any(
                _seg_rect_overlap(pts[i], pts[(i + 1) % n], rect) for i in range(n)
            )
            if touches:
                picked.append(face)
        elif all(_pt_in_rect(p, rect) for p in pts):
            picked.append(face)
    # Groups and component instances (the "box select skips groups"
    # report). Window mode: every vertex inside. Crossing mode: any vertex
    # inside, else any wireframe edge touching the box. Early exits keep
    # the common reject cheap. Inside a group the box works on what the
    # open group holds: its loose geometry above, and its
    # CHILDREN here — the parts of an imported fountain are groups, and
    # a box that skipped them could never take the whole fountain
    # («quiero seleccionar toda esa pileta, no selecciona», Marco,
    # 2026-09-11).
    ctx = viewport.scene.edit_group
    candidates = (getattr(viewport.scene, "groups", []) if ctx is None
                  else (getattr(ctx, "children", None) or []))
    if candidates:
        candidates = [g for g in candidates
                      if viewport.scene.entity_selectable(g)]
        packed = _box_groups_packed(viewport, candidates, rect, crossing)
        for group in candidates:
            if packed is not None and id(group) in packed[1]:
                if id(group) in packed[0]:
                    picked.append(group)
                continue
            verdict = _box_group_fast(viewport, group, rect, crossing)
            if verdict is not None:
                if verdict:
                    picked.append(group)
                continue
            # A component's geometry lives in the placements it holds,
            # so the box has to test the whole subtree — the top-level
            # mesh of an imported component is often empty, and reading
            # only that made it unselectable by rubber band.
            from core.group import iter_placements
            places = [(pg, xf) for pg, xf in iter_placements(group)
                      if pg.mesh.vertices]
            if not places:
                continue

            def gw2p(point, xf):
                return w2p(xf.map(point) if xf is not None else point)

            if crossing:
                hit = False
                for pg, xf in places:
                    for v in pg.mesh.vertices:
                        p = gw2p(v.position, xf)
                        if p is not None and _pt_in_rect(p, rect):
                            hit = True
                            break
                    if hit:
                        break
                if not hit:
                    for pg, xf in places:
                        for e in pg.mesh.edges:
                            pa, pb = gw2p(e.a, xf), gw2p(e.b, xf)
                            if (pa is not None and pb is not None
                                    and _seg_rect_overlap(pa, pb, rect)):
                                hit = True
                                break
                        if hit:
                            break
                if hit:
                    picked.append(group)
            else:
                inside = True
                for pg, xf in places:
                    for v in pg.mesh.vertices:
                        p = gw2p(v.position, xf)
                        if p is None or not _pt_in_rect(p, rect):
                            inside = False
                            break
                    if not inside:
                        break
                if inside:
                    picked.append(group)
    # Guides: an infinite line can never be fully enclosed, so a window
    # box skips it and only a crossing box takes it. Guide
    # points behave like any point. The line is clipped to the part in
    # front of the camera before projecting (as render/snap do).
    for g in getattr(viewport.scene, "guides", []):
        if not viewport.scene.entity_selectable(g):
            continue
        if g.is_line:
            if not crossing:
                continue
            clip = getattr(viewport, "_clip_segment_front", None)
            seg = clip(*g.segment()) if clip else g.segment()
            if seg is None:
                continue
            pa, pb = w2p(seg[0]), w2p(seg[1])
            if (pa is not None and pb is not None
                    and _seg_rect_overlap(pa, pb, rect)):
                picked.append(g)
        elif getattr(g, "is_circle", False):
            # A finite circle: a window box takes it whole, a crossing
            # box takes it when its rim passes through.
            rim = [w2p(p) for p in g.sample_points(24)]
            if any(p is None for p in rim):
                continue
            if crossing:
                if any(_pt_in_rect(p, rect) for p in rim):
                    picked.append(g)
            elif all(_pt_in_rect(p, rect) for p in rim):
                picked.append(g)
        else:
            p = w2p(g.point)
            if p is not None and _pt_in_rect(p, rect):
                picked.append(g)
    selectable = getattr(viewport.scene, "entity_selectable",
                         lambda _e: True)
    for dim in getattr(viewport.scene, "dimensions", []):
        if not selectable(dim):
            continue
        ap, bp = dim.line_points()
        pa, pb = w2p(ap), w2p(bp)
        if pa is None or pb is None:
            continue
        if crossing:
            if _seg_rect_overlap(pa, pb, rect):
                picked.append(dim)
        elif _pt_in_rect(pa, rect) and _pt_in_rect(pb, rect):
            picked.append(dim)
    for lab in getattr(viewport.scene, "text_labels", []):
        if not selectable(lab):
            continue
        pa, pp = w2p(lab.anchor), w2p(lab.position())
        if pp is None:
            continue
        if crossing:
            if _pt_in_rect(pp, rect) or (
                    pa is not None and _seg_rect_overlap(pa, pp, rect)):
                picked.append(lab)
        elif _pt_in_rect(pp, rect) and (
                pa is None or _pt_in_rect(pa, rect)):
            picked.append(lab)
    if mode == "replace" and getattr(viewport, "extension_pick",
                                     None) is not None:
        viewport.clear_extension_pick()
    viewport.scene.select(picked, mode=mode)
    viewport.update()


# ---- Delete: the selection, or what is under the cursor ------------------------

def expand_pick(viewport, entity) -> list:
    """Grow a pick to its natural whole: a curved surface (faces joined by
    soft edges) for a face, or the whole drawn curve (circle/arc) for one of
    its segments — the classic behaviour. Plain entities select alone."""
    if isinstance(entity, Face):
        return viewport.scene.mesh.surface_of(entity)
    if isinstance(entity, Edge) and getattr(entity, "curve", None) is not None:
        return viewport.scene.mesh.curve_edges(entity)
    return [entity]


def erase_entities(viewport, entities) -> bool:
    """Erase ``entities`` (any mix of pickable types) as ONE undoable step.
    True if anything was erased."""
    edges = [e for e in entities if isinstance(e, Edge)]
    faces = [f for f in entities if isinstance(f, Face)]
    groups = [g for g in entities if isinstance(g, Group)]
    dims = [d for d in entities if isinstance(d, Dimension)]
    labels = [t for t in entities if isinstance(t, TextLabel)]
    paths = [p for p in entities if isinstance(p, GeoPath)]
    guides = [g for g in entities if is_guide(g)]
    splanes = [p for p in entities if isinstance(p, SectionPlane)]
    commands = []
    if edges or faces:
        # Erasing an edge between two coplanar faces merges them back
        # into one (as usual); any other erased edge takes its faces.
        commands.append(EraseSelectionCommand(edges, faces))
    commands.extend(DeleteGroupCommand(g) for g in groups)
    if guides:
        commands.append(DeleteGuidesCommand(guides))
    if splanes:
        commands.append(DeleteSectionPlanesCommand(splanes))
    if dims:
        commands.append(DeleteDimensionsCommand(dims))
    if labels:
        commands.append(DeleteTextLabelsCommand(labels))
    if paths:
        commands.append(DeleteGeoPathsCommand(paths))
    if not commands:
        return False
    cmd = commands[0] if len(commands) == 1 else CompoundCommand(commands)
    viewport.history.execute(cmd)
    viewport.update()
    return True


def delete_selection_or_hover(viewport) -> bool:
    """The Delete key: erase the selection; with NOTHING selected, erase
    what is highlighted under the cursor — hover an edge or a face and press
    Supr, no click needed. The hover grows like a click would (a whole
    circle, a whole smooth surface), so what goes is what a click-then-Supr
    would have erased. True if anything was erased."""
    if getattr(viewport, "extension_pick", None) is not None:
        return viewport.delete_extension_pick()      # a render light, say
    selection = list(viewport.scene.selection)
    if selection:
        return erase_entities(viewport, selection)
    hovered = getattr(viewport, "_hover_entity", None)
    if hovered is None:
        return False
    viewport.set_hover(None)        # it is about to be gone
    return erase_entities(viewport, expand_pick(viewport, hovered))


def __getattr__(name):
    # Extensions written against the Python tool import it from here.
    if name == "SelectTool":
        from tools.rust_tools import SelectTool
        return SelectTool
    raise AttributeError(name)

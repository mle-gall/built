"""Resident inference API for P1 (no GL bake or viewport construction).

``prepare(packed, scene=None) -> ResidentInference`` runs in the existing
background preparation job. Pass scene to retain loose hidden/soft edges;
packed.groups supplies placement meshes. Reprepare on hidden-geometry/layer
policy changes, as for the render pack. ``update(ids, packed, changed_codes=())``
retains unchanged BVHs; call after placement, geometry and flag changes, including
billboards and edit-rest flags (64 = rest, 128 = hidden rest). No document-sized BVH rebuild on hover.

``nearby_edges(index, packed, camera, width, height, x, y, *, radius_px=48,
cap=48, rows=None, edit_group=None)`` returns nearest world-space pseudo edges
(a, b, row, edge_id, distance, group, in_group, context). Use uid_rows to supply
an optional row subset; use group/edit_group to split loose/group facade calls.
These are inference records, NOT editable topology Edges. edge_id is the filtered
prototype edge ordinal, not a mesh edge ID. Resolve topology before editing.
``reference_edge(index, view, x, y, radius_px=None)`` returns one visible pseudo
edge or None for Tape/Protractor/Rotate. ``tangent_direction(index, packed, row,
point)`` returns a world unit direction at a curve endpoint or None.
``scene_edges(view)`` returns guide/point/image-border pseudo edges (source is
the original record). Image records implement border_edges() and locked; scene
supplies entity_selectable(). ``snap(index, row, view, x, y, **options)`` returns
GL's SnapResult, even with row=None. All compute_snap lock/reference/acquisition
keywords pass through; view supplies scene, packed, camera, width/height,
_pixel_to_ray, _world_to_pixel and optionally tools.work_point/tool and GL pixel
tolerances. P1 must call snap without a picked face too. ``surface(index, row,
origin, direction)`` also accepts row arrays and returns a world point or None.

Kinds/priorities are owned by core.snap (endpoint, midpoint, on_edge, on_face,
intersection, center, parallel, perpendicular, reference, axis, from_point,
etc.). Arc tools consume tangent_direction to produce their tangent preview;
tangent is not a new competing compute_snap rule. Defaults retain the previous
12 px snap tolerance when the GL thresholds are not supplied by the facade.
"""
from collections.abc import Sequence
from types import SimpleNamespace
import math

import numpy as np
from ingetrazo_core import Vec3 as QVector3D
from core import _native
from core.snap import SurfacePatch, compute_snap, face_crossing_near_ray


def _source_visible(source, scene):
    layers = source.get('layers')
    visible = getattr(scene, 'entity_visible', None)
    names = source.get('meta', {}).get('edge_layers') or ()
    if layers is None or visible is None or not names:
        return np.ones(len(source['edges']), bool)
    # The negative sentinel (untagged) always passes. Evaluate layer policy
    # once per name and gather the mask in a single pass over edges.
    allowed = np.array([visible(SimpleNamespace(layer=name)) for name in names] + [True])
    return allowed[np.where(layers >= 0, layers, -1)]


def _prototype_arrays(packed, code, scene, first_row, cache):
    """Gather snapshots; expansion and curve grouping belong to the native worker."""
    face, edge = packed.commands[code], packed.edge_commands[code]
    start, count = int(face['base']), int(face['count'])
    triangles = np.ascontiguousarray(packed.vertices['pos'][start:start+count])
    groups = getattr(packed, 'groups', None)
    group = groups[first_row] if groups is not None and first_row >= 0 else None
    mesh = group.mesh if group is not None else getattr(scene, 'loose_mesh', None)
    key = (id(mesh), group is not None)
    if mesh is not None and key in cache:
        return (triangles, *cache[key])
    reveal = bool(getattr(scene, 'show_hidden_geometry', False))
    source = mesh.__dict__.get('_deferred_source') if mesh is not None else None
    store = mesh.__dict__.get('_store') if mesh is not None else None
    frame = (0., 0., 0.)
    if source is None and store is not None:
        # Pin one native snapshot for the whole read. The worker can overlap
        # a tool gesture or undo turning the mesh into another coordinate frame.
        saved = getattr(packed, '_snap_sources', {}).get(id(mesh))
        snapshot, frame = saved if saved is not None else (store.snapshot(), store.frame_origin)
        arrays = snapshot.arrays()
        source = dict(pos=arrays['pos'], edges=arrays['edges'], flags=arrays['flags'],
                      curves=arrays['curve'], layers=arrays['layer'],
                      meta={'edge_layers': snapshot.layer_names()})
    if source is not None:
        pos = np.ascontiguousarray(source['pos'], dtype=float)
        edges = np.ascontiguousarray(source['edges'], dtype=np.int64)
        keep = _source_visible(source, scene)
        flags = source['flags']
        if flags is not None and not reveal:
            keep &= (flags & (3 if group is not None else 2)) == 0
        curves = source.get('curves')
        curves = (np.full(len(edges), -1, np.int64) if curves is None else
                  np.ascontiguousarray(curves, dtype=np.int64))
    else:
        lines, named = [], []
        if mesh is None:
            pos = packed.vertices['pos'][start+count:start+count+int(edge['count'])]
        else:
            visible = getattr(scene, 'edge_snappable', lambda e: not e.hidden or reveal)
            for e in mesh.edges:
                if (group is None or reveal or not e.soft) and visible(e):
                    lines.extend((e.v0.position_tuple(), e.v1.position_tuple()))
                    named.append(-1 if e.curve is None else e.curve)
            pos = lines
        pos = np.ascontiguousarray(pos, dtype=float).reshape(-1, 3)
        edges = np.arange(len(pos), dtype=np.int64).reshape(-1, 2)
        keep = np.ones(len(edges), bool)
        curves = np.asarray(named, np.int64) if mesh is not None else np.full(len(edges), -1, np.int64)
    if mesh is not None and code < len(packed.prototype_origins):
        origin = packed.prototype_origins[code]
        if tuple(frame) != origin:
            pos = pos + np.subtract(frame, origin)
    arrays = (pos, edges, keep, curves)
    if mesh is not None:
        cache[key] = arrays
    return (triangles, *arrays)


def _prototype(packed, code, scene, first_row):
    protos, _ = _native.available().build_snap_index(
        [_prototype_arrays(packed, code, scene, first_row, {})],
        np.empty(0, np.int64), np.empty((0, 16)), np.empty((0, 3)), np.empty((0, 3)))
    return protos[0], protos[0]


def _curve_features(curves):
    """Fit only a queried prototype's named curves from its visibility
    snapshot: ``(centres, tangents, arc midpoints)`` in prototype space.
    The arc midpoint is GL's «Arc midpoint» (``core.snap.
    arc_sweep_midpoint``), for an open chain only: two ends, three or more
    vertices — a circle has none."""
    from core.snap import fit_circle, arc_sweep_midpoint
    if hasattr(curves, 'curve_segments'):
        curves = curves.curve_segments()
    centers, tangents, mids = [], [], []
    for segments in curves:
        ends = np.asarray(segments, float).reshape(-1, 3)
        points, counts = np.unique(ends, axis=0, return_counts=True)
        vertices = [QVector3D(*p) for p in points]
        fit = fit_circle(vertices) if len(points) >= 3 else None
        if fit is None:
            continue
        if int((counts == 1).sum()) == 2:
            mid = arc_sweep_midpoint(vertices, fit)
            if mid is not None:
                mids.append(np.array(mid.toTuple()))
        center = np.array(fit[0].toTuple())
        centers.append(center)
        for point in points[counts == 1]:
            at = np.flatnonzero(np.all(ends == point, axis=1))[0]
            leaving = point - ends[at ^ 1]
            radial = point-center
            radial /= np.linalg.norm(radial)
            tangent = leaving-radial*np.dot(leaving, radial)
            length = np.linalg.norm(tangent)
            if length > 1e-9:
                tangents.append((point, tangent/length))
    return centers, tangents, mids


def tangent_direction(index, packed, row, point):
    if row not in eligible_rows(index, packed):
        return None
    value = index.rows.get(row, packed.instances[row])
    matrix = value['world'].astype(float).T
    for endpoint, tangent in index.features[int(value['info'][0])][1]:
        world = matrix[:3, :3] @ endpoint + matrix[:3, 3]
        if (point-QVector3D(*world)).length() <= 1e-6:
            direction = QVector3D(*(matrix[:3, :3] @ tangent))
            if direction.length() > 1e-9:
                return direction.normalized()
    return None


class _CurveFeatures(Sequence):
    """Prototype-indexed curve fits; discard the input segments once fitted."""

    def __init__(self, curves):
        self.pending = curves
        self.fitted = [None] * len(curves)

    def __len__(self):
        return len(self.fitted)

    def __getitem__(self, code):
        if isinstance(code, slice):
            return [self[i] for i in range(*code.indices(len(self)))]
        if self.fitted[code] is None:
            self.fitted[code] = _curve_features(self.pending[code])
            self.pending[code] = None
        return self.fitted[code]

    def replace(self, code, curves):
        while len(self) <= code:
            self.pending.append(())
            self.fitted.append(None)
        self.pending[code] = curves
        self.fitted[code] = None


class ResidentInference:
    """Keep immutable prototype BVHs; edited IDs get a tiny local placement set.

    Queries merge the original placement set with edited local sets. Retaining it
    set avoids both rebuilding and destroying a document-sized BVH on input.
    Only queried edited rows construct a replacement placement acceleration.
    """
    def __init__(self, base, protos, curves, packed, scene=None):
        self.packed = packed
        self.scene = scene
        self.features = _CurveFeatures(curves)
        self.screen_cache = None
        self.screen_dirty = set()
        self.epoch = 0
        self.base = base
        self.protos = protos
        self.bounds = [bound.copy() for bound in packed.bounds]
        self.rows = {}
        self.changed = set()
        self.local = {}
        self.replaced = np.zeros(len(packed.instances), bool)

    def update(self, ids, packed, changed_codes=(), *, code_rows=None):
        self.epoch += 1
        self.packed = packed
        if len(self.replaced) < len(packed.instances):
            self.replaced = np.pad(self.replaced, (0, len(packed.instances)-len(self.replaced)))
        for code in code_rows if code_rows is not None else dict.fromkeys(changed_codes):
            while len(self.protos) <= code:
                self.protos.append(None)
                self.bounds.append(None)

            affected = (code_rows[code] if code_rows is not None else
                        np.flatnonzero(packed.instances['info'][:, 0] == code))
            self.protos[code], curves = _prototype(
                packed, code, self.scene, affected[0] if len(affected) else -1)
            self.bounds[code] = packed.bounds[code].copy()
            self.features.replace(code, curves)
            self.changed.add(code)
            self.replaced[affected] = True
            self.screen_dirty.update(affected.tolist())
            for row, (cached_code, _) in list(self.local.items()):
                if cached_code == code:
                    del self.local[row]
        for row in ids:
            row = int(row)
            self.rows[row] = packed.instances[row].copy()
            self.local.pop(row, None)
            self.replaced[row] = True
            self.screen_dirty.add(row)

    def _query(self, name, rows, *args):
        rows = np.asarray(rows, np.int64)
        if not self.rows and not self.changed:
            return getattr(self.base, name)(rows, *args)
        if len(rows) != 1:
            edited = self.replaced[rows]
            parts = [getattr(self.base, name)(rows[~edited], *args)]
            for row in rows[edited]:
                parts.append(self._query(name, np.array([row], np.int64), *args))
            merged = tuple(np.concatenate([part[k] for part in parts]) for k in range(5))
            if name == 'edges_near':
                order = np.lexsort((merged[1], merged[0], merged[4]))[:args[-1]]
                merged = tuple(a[order] for a in merged)
            return merged
        row = int(rows[0])
        code, world = self.committed(row)
        if row not in self.rows and code not in self.changed:
            return getattr(self.base, name)(rows, *args)
        if row not in self.local:
            matrix = world.T
            bounds = self.bounds[code][:, :3].astype(np.float64)
            center, extent = bounds.mean(axis=0), (bounds[1]-bounds[0])*.5
            wc = matrix[:3, :3] @ center + matrix[:3, 3]
            we = abs(matrix[:3, :3]) @ extent
            index = _native.available().InstanceSet(
                [self.protos[code]], np.array([0], np.int64),
                world.reshape(1, 16),
                np.ascontiguousarray((wc-we).reshape(1, 3)),
                np.ascontiguousarray((wc+we).reshape(1, 3)))
            self.local[row] = (code, index)
        result = getattr(self.local[row][1], name)(np.array([0], np.int64), *args)
        return (np.full(len(result[0]), row, np.int64), *result[1:])

    def committed(self, row):
        """``(code, world)`` of ``row`` as the queries see it: an updated
        row, else the placement the native index was built with (no copy
        of the instance table is kept for it). ``world`` is the instance
        row's 4 × 4 (translation in the last row), f64."""
        value = self.rows.get(row)
        if value is not None:
            return int(value['info'][0]), value['world'].astype(np.float64)
        code, flat = self.base.placement(row)
        return int(code), np.asarray(flat, np.float64).reshape(4, 4)

    def ray_cands(self, rows, *args):
        return self._query('ray_cands', rows, *args)

    def edges_near(self, rows, *args):
        return self._query('edges_near', rows, *args)


def _placement_arrays(packed):
    mats = packed.instances['world'].astype(np.float64).reshape(-1, 16)
    ids = packed.instances['info'][:, 0].astype(np.int64)
    bounds = packed.bounds[ids, :, :3].astype(np.float64)
    matrix = mats.reshape(-1, 4, 4).transpose(0, 2, 1)
    center = bounds.mean(axis=1)
    extent = (bounds[:, 1] - bounds[:, 0]) / 2
    wc = np.einsum('nij,nj->ni', matrix[:, :3, :3], center) + matrix[:, :3, 3]
    we = np.einsum('nij,nj->ni', abs(matrix[:, :3, :3]), extent)
    return ids, mats, wc, we


def rebase(index, packed):
    """Keep prototype BVHs and replace their placement rows in the new frame."""
    ids, mats, wc, we = _placement_arrays(packed)
    base = _native.available().InstanceSet(index.protos, ids, mats,
                                         np.ascontiguousarray(wc-we), np.ascontiguousarray(wc+we))
    result = ResidentInference(base, index.protos, list(index.protos), packed, index.scene)
    result.world_box_cache = wc, we
    return result


def prepare(packed, scene=None):
    native = _native.available()
    codes, rows = np.unique(packed.instances['info'][:, 0], return_index=True)
    first = np.full(len(packed.commands), -1, dtype=np.int64)
    first[codes] = rows
    cache = {}
    arrays = [_prototype_arrays(packed, code, scene, row, cache)
              for code, row in enumerate(first)]
    ids, mats, wc, we = _placement_arrays(packed)
    protos, base = native.build_snap_index(arrays, ids, mats, np.ascontiguousarray(wc-we),
                                           np.ascontiguousarray(wc+we))
    index = ResidentInference(base, protos, list(protos), packed, scene)
    index.world_box_cache = wc, we
    return index


def surface(index, row, origin, direction, limit=float('inf')):
    """Use the GL picker's exact triangle test after the prototype BVH."""
    hit = surface_hit(index, row, origin, direction, limit)
    return hit[0] if hit is not None else None


def surface_hit(index, row, origin, direction, limit=float('inf')):
    """:func:`surface` and the unit normal of the triangle it hit, from the
    same candidates: ``(point, normal)`` or ``None``."""
    from views.document_interaction import _moller_trumbore
    rows = np.asarray(row, np.int64).reshape(-1)
    rows = rows[(rows >= 0) & (rows < len(index.packed.instances))]
    rows = rows[(index.packed.instances['info'][rows, 2] & 129) == 0]
    _, _, v0, e1, e2 = index.ray_cands(rows, origin.toTuple(), direction.toTuple(), limit)
    if not len(v0):
        return None
    t, hit = _moller_trumbore(np.array(origin.toTuple()), np.array(direction.toTuple()), v0, e1, e2)
    if not hit.any():
        return None
    k = int(np.argmin(np.where(hit, t, np.inf)))
    n = np.cross(e1[k], e2[k])
    length = float(np.linalg.norm(n))
    normal = QVector3D(*(n / length)) if length > 1e-30 else None
    return origin + direction * float(t[k]), normal


def packed_surface(packed, row, origin, direction):
    """:func:`surface` without the snap index: the picked placement's own
    triangles, straight from the pack. The index is built on a worker
    (seconds on the ride) and dropped on edits and at open; the orbit pivot
    and the zoom fell back to the middle of the model meanwhile, so the view
    turned about a point nowhere near the cursor."""
    from views.document_interaction import _moller_trumbore
    if row is None or not 0 <= int(row) < len(packed.instances):
        return None
    info = packed.instances['info'][int(row)]
    if info[2] & 129:
        return None
    face = packed.commands[int(info[0])]
    start, count = int(face['base']), int(face['count'])
    if count < 3:
        return None
    tris = packed.vertices['pos'][start:start + count - count % 3].astype(np.float64).reshape(-1, 3, 3)
    matrix = packed.instances['world'][int(row)].astype(np.float64).reshape(4, 4).T
    world = tris @ matrix[:3, :3].T + matrix[:3, 3]
    v0 = world[:, 0]
    t, hit = _moller_trumbore(np.array(origin.toTuple()), np.array(direction.toTuple()),
                              v0, world[:, 1] - v0, world[:, 2] - v0)
    valid = t[hit]
    if not len(valid):
        return None
    return origin + direction * float(valid.min())


def eligible_rows(index, packed=None):
    """Visible placements, including overrides from incremental/billboard updates."""
    values = (index.packed if packed is None else packed).instances
    flags = values['info'][:, 2]
    return np.flatnonzero((flags & (1 | 128)) == 0).astype(np.int64)


def screen_boxes(index, packed, mvp, width, height):
    """Cached conservative screen boxes; only moved rows reproject after edits."""
    key = (tuple(mvp.data()), width, height, len(packed.instances))
    cache = index.screen_cache
    if cache is None or cache[0] != key:
        lo = np.empty((len(packed.instances), 2))
        hi = np.empty_like(lo)
        changed = np.arange(len(lo))
        grid, moved = None, set()
    else:
        _, lo, hi, grid, moved = cache
        changed = np.fromiter(index.screen_dirty, np.int64)
    if len(changed):
        wc, we = _world_boxes(index, packed)
        low, high = _project_boxes(wc[changed], we[changed], mvp, width, height)
        lo[changed], hi[changed] = low, high
        index.screen_dirty.clear()
        if grid is None:
            from core.edge_grid import EdgeGrid
            # Cross-eye boxes must remain conservative; a full-screen box is
            # equivalent for cursors inside this view.
            low = np.where(np.isneginf(lo), -128., lo)
            high = np.where(np.isposinf(hi) & np.isfinite(low),
                            np.array([width+128., height+128.]), hi)
            grid = EdgeGrid(low[:, 0], low[:, 1], high[:, 0], high[:, 1],
                            np.isfinite(low).all(axis=1), width, height, 128.)
        else:
            moved.update(changed.tolist())
        index.screen_cache = (key, lo, hi, grid, moved)
    return lo, hi, grid, moved


def _world_boxes(index, packed):
    """Each row's world box as ``(centre, half extent)``, ``(n, 3)`` each:
    camera-independent, so kept on the index and redone only for the rows
    an update marked (``screen_dirty``) — a camera move used to rebuild all
    of them, ~2 ms of the first hover over 30 000 placements."""
    n = len(packed.instances)
    cache = getattr(index, 'world_box_cache', None)
    if cache is None or len(cache[0]) != n:
        rows = np.arange(n)
        wc, we = np.empty((n, 3)), np.empty((n, 3))
    else:
        wc, we = cache
        rows = np.fromiter(index.screen_dirty, np.int64)
    if len(rows):
        values = packed.instances[rows]
        matrices = values['world'].astype(float).transpose(0, 2, 1)
        bounds = packed.bounds[values['info'][:, 0], :, :3].astype(float)
        center = bounds.mean(axis=1)
        extent = (bounds[:, 1]-bounds[:, 0])*.5
        wc[rows] = np.einsum('nij,nj->ni', matrices[:, :3, :3], center)+matrices[:, :3, 3]
        we[rows] = np.einsum('nij,nj->ni', abs(matrices[:, :3, :3]), extent)
    index.world_box_cache = (wc, we)
    return wc, we


def _project_boxes(wc, we, mvp, width, height):
    """Conservative screen boxes ``(low, high)`` of the world boxes: the
    eight corners projected, ``±inf`` for a box crossing the eye plane,
    ``+inf`` for one wholly behind it. The clip transform is linear, so
    each corner is the clip centre plus signed clip extents — eight passes
    over contiguous rows, not reductions over a short middle axis (half of
    the first hover's 17 ms over 30 000 placements)."""
    matrix = np.asarray(mvp.data()).reshape(4, 4).T
    centre = wc @ matrix[:, :3].T + matrix[:, 3]
    axes = [we[:, k, None] * matrix[:, k] for k in range(3)]
    n = len(wc)
    low = np.full((n, 2), np.inf)
    high = np.full((n, 2), -np.inf)
    any_front = np.zeros(n, bool)
    all_front = np.ones(n, bool)
    half_w, half_h = width / 2, height / 2
    for sx in (-1., 1.):
        for sy in (-1., 1.):
            for sz in (-1., 1.):
                clip = centre + sx * axes[0] + sy * axes[1] + sz * axes[2]
                w = clip[:, 3]
                front = w > 1e-3
                denom = np.where(front, w, 1.)
                px = (clip[:, 0] / denom + 1.) * half_w
                py = (1. - clip[:, 1] / denom) * half_h
                np.minimum(low[:, 0], px, out=low[:, 0])
                np.minimum(low[:, 1], py, out=low[:, 1])
                np.maximum(high[:, 0], px, out=high[:, 0])
                np.maximum(high[:, 1], py, out=high[:, 1])
                any_front |= front
                all_front &= front
    crossing = any_front & ~all_front
    low[crossing], high[crossing] = -np.inf, np.inf
    low[~any_front], high[~any_front] = np.inf, np.inf
    return low, high


def _screen_rows(index, packed, mvp, width, height, x, y, radius, rows):
    lo, hi, grid, moved = screen_boxes(index, packed, mvp, width, height)
    candidates = grid.near(x, y, radius) if grid is not None and radius <= 128 else None
    if candidates is not None:
        if moved:
            candidates = np.union1d(candidates, np.fromiter(moved, np.int64))
        if rows is not None:
            candidates = candidates[np.isin(candidates, rows)]
        rows = candidates
    if rows is None:
        rows = eligible_rows(index, packed)
    else:
        rows = rows[(packed.instances['info'][rows, 2] & 129) == 0]
    gap = np.maximum(np.maximum(lo[rows]-[x,y], [x,y]-hi[rows]), 0.)
    return rows[np.einsum('ij,ij->i', gap, gap) < radius*radius]


def context_hit(view, context, x, y):
    from views.document_interaction import _moller_trumbore
    index = view.inference_index
    rows = np.asarray(view._rows_for_group(context), np.int64)
    flags = view.packed.instances['info'][rows, 2]
    rows = rows[(flags & 129) == 0]
    camera = view.camera
    mvp = camera.projection_matrix() * camera.view_matrix()
    radius = getattr(view.tools, "pick_threshold_px", 8.)
    rows = _screen_rows(index, view.packed, mvp, view.width(), view.height(), x, y, radius, rows)
    origin, direction = view._pixel_to_ray(x, y)
    placements, _, v0, e1, e2 = index.ray_cands(
        rows, origin.toTuple(), direction.toTuple(), float('inf'))
    if len(v0):
        t, hit = _moller_trumbore(np.array(origin.toTuple()), np.array(direction.toTuple()), v0, e1, e2)
        if np.any(hit):
            return int(placements[np.argmin(np.where(hit, t, np.inf))])
    placements, *_ = index.edges_near(rows, mvp.data(), float(view.width()),
        float(view.height()), float(x), float(y), float(radius), 1)
    return int(placements[0]) if len(placements) else None


class ResidentEdges(Sequence):
    """Keep nearby placement/edge ordinals packed until a rule needs a Qt reference."""
    def __init__(self, columns, groups, context):
        self.rows, self.ids, self.a, self.b, self.distances = columns
        self.groups = groups
        self.context = context
        self.prefix = []
        self.features = []

    def __len__(self):
        return len(self.prefix) + len(self.rows) + len(self.features)

    def __getitem__(self, index):
        if isinstance(index, slice):
            return [self[i] for i in range(*index.indices(len(self)))]
        if index < 0:
            index += len(self)
        if index < 0 or index >= len(self):
            raise IndexError(index)
        if index < len(self.prefix):
            return self.prefix[index]
        i = index - len(self.prefix)
        if i >= len(self.rows):
            return self.features[i-len(self.rows)]
        row = int(self.rows[i])
        group = self.groups[row] if self.groups is not None else None
        loose = group is self.context
        word = None if loose else ('component' if group is not None and group.is_instance() else 'group')
        return SimpleNamespace(a=QVector3D(*self.a[i]), b=QVector3D(*self.b[i]),
            row=row, edge_id=int(self.ids[i]), distance=float(self.distances[i]), group=group,
            in_group=not loose, context=word)

    def keep(self, mask):
        """The same edges less the masked-out placement ones (prefix and
        features stay), still as columns."""
        mask = np.asarray(mask, bool)
        out = ResidentEdges(tuple(column[mask] for column in (self.rows, self.ids, self.a, self.b, self.distances)),
                            self.groups, self.context)
        out.prefix, out.features = self.prefix, self.features
        return out

    def in_group(self):
        """Per edge: whether its placement is not the open context."""
        if self.groups is None:
            return np.full(len(self.rows), self.context is not None)
        rows, inverse = np.unique(self.rows, return_inverse=True)
        return np.asarray([self.groups[r] is not self.context for r in rows], bool)[inverse]

    def snap_columns(self):
        from core.snap_query import edge_columns
        count = len(self.rows)
        grouped = self.in_group().tolist()
        tags = [0] * count
        if not self.prefix and not self.features:
            return self.a, self.b, tags, grouped
        first_a, first_b, first_tags, first_groups = edge_columns(self.prefix)
        last_a, last_b, last_tags, last_groups = edge_columns(self.features)
        return (np.concatenate((first_a, self.a, last_a)),
                np.concatenate((first_b, self.b, last_b)),
                first_tags + tags + last_tags, first_groups + grouped + last_groups)


def nearby_edges(index, packed, camera, width, height, x, y, *,
                 radius_px=48., cap=48, rows=None, edit_group=None):
    """World-space pseudo edges, nearest first; row/edge_id identify source."""
    if cap <= 0:
        return []
    if rows is not None:
        rows = np.asarray(rows, np.int64)
        rows = rows[(rows >= 0) & (rows < len(packed.instances))]
    mvp = camera.projection_matrix() * camera.view_matrix()
    rows = _screen_rows(index, packed, mvp, width, height, x, y, radius_px, rows)
    pl, ids, a, b, distances = index.edges_near(
        rows, mvp.data(), float(width), float(height), float(x), float(y),
        float(radius_px), int(cap))
    groups = getattr(packed, 'groups', None)
    result = ResidentEdges((pl, ids, a, b, distances), groups, edit_group)
    return list(result)


def scene_edges(view):
    """Guides, guide points and selectable image borders using GL clipping."""
    from views.document_interaction import DocumentInteraction, shown_guides
    result = []
    tool = getattr(getattr(view, 'tools', None), 'tool', None)
    moving = getattr(tool, 'moving_guide_ids', lambda: ())()
    for guide in shown_guides(view.scene):
        if id(guide) in moving:
            continue
        segment = (DocumentInteraction._guide_snap_segment(view, guide) if guide.is_line else
                   (QVector3D(guide.point), QVector3D(guide.point)))
        if segment is not None:
            result.append(SimpleNamespace(a=segment[0], b=segment[1],
                                          guide=guide.is_line, source=guide))
    selectable = getattr(view.scene, 'entity_selectable', lambda e: True)
    for plane in getattr(view.scene, 'image_planes', ()) or ():
        if getattr(plane, 'locked', False) or not selectable(plane):
            continue
        for a, b in plane.border_edges():
            segment = DocumentInteraction._clip_segment_front(view, a, b)
            if segment is not None:
                result.append(SimpleNamespace(a=segment[0], b=segment[1], source=plane))
    return result


def reference_edge(index, view, x, y, radius_px=None):
    """Nearest visible world edge for Tape/Protractor/Rotate and arc tangency."""
    radius = radius_px if radius_px is not None else getattr(view, 'pick_threshold_px', 8.)
    edges = nearby_edges(index, view.packed, view.camera, view.width(), view.height(),
                         x, y, radius_px=radius, cap=48,
                         edit_group=view.scene.edit_group)
    from views.document_interaction import DocumentInteraction
    for edge in edges:
        pa, pb = view._world_to_pixel(edge.a), view._world_to_pixel(edge.b)
        if pa is None or pb is None:
            continue
        delta = edge.b-edge.a
        projected = DocumentInteraction._project_to_lock_line(view, edge.a, delta, x, y)
        t = (QVector3D.dotProduct(projected-edge.a, delta)/delta.lengthSquared()
             if delta.lengthSquared() > 1e-12 else 0.)
        point = edge.a + delta*max(0., min(1., t))
        if not _occluded(index, view, point):
            return edge
    return None


def _see_through(view):
    """X-ray and wireframe show everything, so nothing hides a snap there:
    GL's ``DocumentInteraction._is_occluded`` answers False in those styles, and the
    snap rules that skip hidden references (a lock taking a hidden back
    edge's height, upstream #1085) must still see through them here."""
    style = getattr(getattr(view, 'source_viewport', None), '_effective_style', None)
    if style is None:
        return False
    try:
        return getattr(style(), 'face_mode', None) in ('xray', 'wireframe')
    except Exception:
        return False


def _occluded(index, view, candidate, rows=None):
    if _see_through(view):
        return False
    pixel = view._world_to_pixel(candidate)
    if pixel is None:
        return True
    eye, ray = view._pixel_to_ray(*pixel)
    if eye is None or ray is None:
        return True
    if rows is None:
        mvp = view.camera.projection_matrix() * view.camera.view_matrix()
        rows = _screen_rows(index, view.packed, mvp, view.width(), view.height(), *pixel, 1., None)
    reach = (candidate-eye).length() - 1e-4
    fixed = rows[~index.replaced[rows]]
    hidden, doubt = index.base.occluded(fixed, eye.toTuple(),
        np.array([ray.toTuple()], float), np.array([reach], float))
    if hidden[0] and not doubt[0]:
        return True
    check = rows if doubt[0] else rows[index.replaced[rows]]
    if not len(check):
        return False
    front = surface(index, check, eye, ray, reach)
    return front is not None and (front-eye).length() < reach



def _occluded_many(index, view, points, rows=None):
    if not points:
        return []
    if _see_through(view):
        return [False] * len(points)
    if not getattr(view.camera, 'perspective', False) or index.rows or index.changed:
        return [_occluded(index, view, p, rows) for p in points]
    if rows is None:
        rows = eligible_rows(index, view.packed)
    eye = view.camera.eye()
    delta = np.array([p.toTuple() for p in points], float)-eye.toTuple()
    lengths = np.linalg.norm(delta, axis=1)
    directions = delta/np.maximum(lengths[:, None], 1e-30)
    hidden, doubt = index.base.occluded(rows, eye.toTuple(),
        np.ascontiguousarray(directions), np.maximum(lengths-1e-4, 0.))
    result = hidden.tolist()
    for i in np.flatnonzero(doubt):
        result[i] = _occluded(index, view, points[i], rows)
    return result


def _acquire_rim(view, index, x, y):
    """Resolve only the hovered resident edge to its document curve."""
    edge = reference_edge(index, view, x, y)
    if edge is None:
        return None
    picked = view.tools._mesh_for_row(edge.row)
    if picked is None:
        return None
    mesh, group, matrix = picked
    inverse, ok = matrix.inverted()
    if not ok:
        return None
    a, b = (mesh.vertex_at(inverse.map(point)) for point in (edge.a, edge.b))
    real = mesh.find_edge(a, b) if a is not None and b is not None else None
    if getattr(real, 'curve', None) is None:
        return None
    ref = view.source_viewport._center_of_edge(real, mesh)
    if ref is None:
        return None
    place = SimpleNamespace(xform=matrix, source_occurrence=group) if group is not None else None
    return (matrix.map(ref[0]), *ref[1:6], place)


def snap(index, row, view, x, y, *, work_point=..., **options):
    """GL SnapResult; row may be None (guides/edges do not need a face ID)."""
    if index is None:
        return None
    mvp = view.camera.projection_matrix() * view.camera.view_matrix()
    rows = _screen_rows(index, view.packed, mvp, view.width(), view.height(), x, y, 64., None)
    picked = row is not None and 0 <= row < len(view.packed.instances) and not (
        view.packed.instances['info'][row, 2] & 129)
    origin, direction = view._pixel_to_ray(x, y)
    if origin is None or direction is None:
        return None
    engine = options.pop('inference', None)
    modifiers = options.pop('modifiers', None)
    hit = surface_hit(index, row, origin, direction) if picked else None
    point, face_normal = hit if hit is not None else (None, None)
    on_face = point is not None
    tool = getattr(getattr(view, 'tools', None), 'tool', None)
    if point is None:
        work = getattr(getattr(view, 'tools', None), 'work_point', lambda x, y: None)
        point = work_point
        if point is ...:
            point = work(x, y) if tool is not None else None
        if point is None:
            normal = getattr(view.camera, 'forward', lambda: direction)()
            denominator = QVector3D.dotProduct(direction, normal)
            point = (origin + direction * (QVector3D.dotProduct(
                view.camera.target-origin, normal)/denominator)
                if abs(denominator) > 1e-9 else QVector3D(view.camera.target))
    context = view.scene.edit_group
    # Classify just the nearby placements. Hidden or far-away links cannot
    # contribute edges inside either snap radius.
    groups = getattr(view.packed, 'groups', None)
    is_loose = np.array([groups is not None and groups[r] is context for r in rows], dtype=bool)
    loose_rows, group_rows = rows[is_loose], rows[~is_loose]
    batches = []

    def gather(subset, reach, cap):
        return index.edges_near(subset, mvp.data(), float(view.width()), float(view.height()),
                                float(x), float(y), reach, cap)

    if len(loose_rows):
        batches.append(gather(loose_rows, 64., 400))
    # The outer window is 64 px; group edges keep the original 48 px reach.
    group_rows = _screen_rows(index, view.packed, mvp, view.width(), view.height(),
                              x, y, 48., group_rows)
    batches.append(gather(group_rows, 48., 48))
    if picked and len(rows) - len(loose_rows) > 1 and row in group_rows:
        extra = gather(np.array([row], np.int64), 48., 48)
        seen = set(zip(batches[-1][0], batches[-1][1]))
        keep = np.array([key not in seen for key in zip(extra[0], extra[1])], dtype=bool)
        batches.append(tuple(column[keep] for column in extra))
    columns = (batches[0] if len(batches) == 1 else
               tuple(np.concatenate(c) for c in zip(*batches)))
    edges = ResidentEdges(columns, groups, context)
    tool = getattr(getattr(view, 'tools', None), 'tool', None)
    moving = getattr(tool, '_verts', ()) if getattr(tool, 'grab', None) is not None else ()
    # What is in motion leaves the candidates as columns: turning every
    # edge into Python objects here sent the snap engine down its walk
    # over edge objects (architecture review 12).
    if moving and not getattr(tool, '_copy', False):
        from core.topology import _KEY_DECIMALS, _key
        excluded = {_key(v.position) for v in moving}

        def free(i):
            # Only the open context's own edges can hold what moves.
            return all(tuple(round(c, _KEY_DECIMALS) for c in p) not in excluded
                       for p in (edges.a[i].tolist(), edges.b[i].tolist()))
        keep = edges.in_group()
        for i in np.flatnonzero(~keep).tolist():
            keep[i] = free(i)
        edges = edges.keep(keep)
        point = view.tools.work_point(x, y) or point
        on_face = False
    exclude = getattr(tool, 'snap_excluded', lambda: None)()
    if exclude is not None and exclude[1] and groups is not None:
        moved = {r for r in np.unique(edges.rows).tolist() if id(groups[r]) in exclude[1]}
        edges = edges.keep(~np.isin(edges.rows, list(moved)))
    elif exclude is not None and exclude[1]:
        edges = edges.keep(np.full(len(edges.rows), id(None) not in exclude[1]))
    if isinstance(edges, ResidentEdges):
        _, first = np.unique(edges.rows, return_index=True)
        edge_rows = edges.rows[np.sort(first)].tolist()
    else:
        edge_rows = [e.row for e in edges]
    feature_rows = list(dict.fromkeys(edge_rows + ([row] if picked else [])))
    named_edges = edges.features if isinstance(edges, ResidentEdges) else edges
    may_acquire_rim = False
    for r in feature_rows:
        value = index.rows.get(r, index.packed.instances[r])
        matrix = value['world'].astype(float).T
        group = view.packed.groups[r] if view.packed.groups is not None else None
        # The placement's context, as nearby_edges gives its edges: a named
        # point keeps it (the tip, and a near-tie going to the loose point;
        # core.snap._point_snap_candidates), as GL's pseudo-edges do.
        named = (None if group is context else
                 'component' if group is not None and group.is_instance()
                 else 'group')
        centers, _tangents, mids = index.features[int(value['info'][0])]
        if centers and np.any((columns[0] == r) & (columns[-1] <= getattr(view, 'pick_threshold_px', 8.))):
            may_acquire_rim = True
        for center in centers:
            center = QVector3D(*(matrix[:3, :3] @ center + matrix[:3, 3]))
            named_edges.append(SimpleNamespace(a=center, b=center, center=True,
                                         context=named))
        # GL's «Arc midpoint» (DocumentInteraction._arc_midpoints): the middle of each
        # open arc's sweep, from the same per-prototype fit as the centres —
        # only for the placements already met near the cursor.
        # Their tips say what GL's do (_snap_context_of): «outside» for a
        # placement around the open group / component.
        word = snap_context(view, r, context) if mids or (
            group is not None and group.is_instance()) else None
        for mid in mids:
            mid = QVector3D(*(matrix[:3, :3] @ mid + matrix[:3, 3]))
            named_edges.append(SimpleNamespace(a=mid, b=mid, arc_midpoint=True,
                                         context=word))
        if group is not None and group.is_instance():
            anchor = QVector3D(*index.committed(r)[1][3, :3])
            named_edges.append(SimpleNamespace(a=anchor, b=anchor, component_origin=True,
                                         context=word))
    if isinstance(edges, ResidentEdges):
        edges.prefix = scene_edges(view)
    else:
        edges = scene_edges(view) + edges
    threshold = options.pop('threshold_px', getattr(view, 'snap_threshold_px', 12.))
    options.setdefault('edge_threshold_px', getattr(view, 'edge_snap_threshold_px', None))
    memo = {}
    def hidden_many(points):
        keys = [p.toTuple() for p in points]
        missing = {key: point for key, point in zip(keys, points) if key not in memo}
        if missing:
            memo.update(zip(missing, _occluded_many(index, view, list(missing.values()))))
        return [memo[key] for key in keys]
    options.setdefault('is_occluded', lambda p: hidden_many([p])[0])
    options.setdefault('are_occluded', hidden_many)
    options.setdefault('face_under_cursor', on_face)
    # Like GL's _pixel_projector: one matrix per query, not per candidate.
    from views.document_interaction import DocumentInteraction
    mvp = view.camera.projection_matrix() * view.camera.view_matrix()
    project = lambda p: DocumentInteraction._world_to_pixel(view, p, mvp)
    project.snap_projection = (mvp.data(), (view.width(), view.height()))
    source = getattr(view, 'source_viewport', None)
    acquire = getattr(tool, 'uses_snap', False) and hasattr(source, '_valid_center_ref')
    rim = _acquire_rim(view, index, x, y) if acquire and may_acquire_rim else None
    if 'face_centers' not in options:
        options['face_centers'] = face_centers_near(view, x, y, on_face, project,
                                                    acquire=acquire and rim is None)
    if acquire:
        if rim is not None:
            source._center_ref = rim
        ref = source._valid_center_ref()
        if ref is not None:
            pc = project(ref[0])
            if pc is not None and source._center_hint_px(ref, pc) > 0.:
                named_edges.append(SimpleNamespace(a=ref[0], b=ref[0], center=True, context=None))

    if on_face and face_normal is not None:
        # The face rules GL's InferenceEngine.snap hands in from its own
        # picks, here from the triangle under the cursor: the ScreenTip's
        # «in group / outside» for the face, an edge piercing it (5d) and
        # another face crossing it with no edge between them (7c). Both
        # are callables, asked by compute_snap only when nothing nearer
        # with a name answered, and both stay on the placements under the
        # cursor.
        options.setdefault('face_context', snap_context(view, row, context))
        options.setdefault('face_outline', lambda p=point, n=face_normal: SurfacePatch(
            p, n, lambda q: on_surface(index, view, row, q, project),
            placement_context(view, row, context)))
        options.setdefault('face_crossing', lambda p=point, n=face_normal: face_crossing(
            index, view, x, y, row, p, n, None, project, context))
    plane = getattr(getattr(view, 'tools', None), '_last_work_plane', None)
    if plane is not None:
        options.setdefault('work_plane_normal', plane[1])
    elif on_face and face_normal is not None:
        options.setdefault('work_plane_normal', face_normal)
    start = options.get('start_point')
    if start is not None:
        options.setdefault('start_edges', start_edges(index, view, start, None))
    if engine is not None:
        _acquire(engine, view, edges, on_face, point, face_normal,
                 options, x, y, modifiers)
    return compute_snap(point, (x, y), SimpleNamespace(edges=edges),
                        project, threshold, **options)


def _row_group(view, row):
    groups = getattr(view.packed, 'groups', None)
    return groups[row] if groups is not None and row is not None else None


def placement_context(view, row, context):
    """What GL's ``_face_outline_world`` says a face belongs to:
    ``"group"`` / ``"component"`` for a placement, ``None`` for the open
    context's own loose geometry and for the root's around it."""
    group = _row_group(view, row)
    if group is None or group is context:
        return None
    return 'component' if group.is_instance() else 'group'


def snap_context(view, row, context):
    """GL's ``_snap_context_of`` for a resident row: ``None`` for the open
    context's own geometry (and at the root), ``"outside"`` for what is
    drawn around an open group / component, else ``"group"`` /
    ``"component"``."""
    group = _row_group(view, row)
    if group is context:
        return None
    if context is not None:
        inside = getattr(view, 'context_placement', None)
        if group is None or (inside is not None and inside(group) is None):
            return 'outside'
    return placement_context(view, row, context)


def on_surface(index, view, row, point, project=None):
    """Whether ``point`` lies on the visible surface of placement ``row``:
    the ray through its pixel meets that placement's triangles there. What
    stands for GL's «inside the face's loops, outside its holes» (a hole
    has no triangles) — one ray over one placement."""
    project = project or view._world_to_pixel
    pixel = project(point)
    if pixel is None:
        return False
    origin, direction = view._pixel_to_ray(*pixel)
    if origin is None or direction is None:
        return False
    hit = surface(index, row, origin, direction)
    if hit is None:
        return False
    return (hit - point).length() <= 1e-6 + 1e-5 * (point - origin).length()


def _front_at(index, view, x, y, rows, mvp):
    """``(point, unit normal, row)`` of the front surface at pixel
    ``(x, y)``, asked of the placements whose screen box holds the pixel."""
    from views.document_interaction import _moller_trumbore
    origin, direction = view._pixel_to_ray(x, y)
    if origin is None or direction is None:
        return None
    near = _screen_rows(index, view.packed, mvp, view.width(), view.height(),
                        x, y, 1., rows)
    if not len(near):
        return None
    pl, _, v0, e1, e2 = index.ray_cands(near, origin.toTuple(), direction.toTuple(),
                                        float('inf'))
    if not len(v0):
        return None
    t, hit = _moller_trumbore(np.array(origin.toTuple()), np.array(direction.toTuple()),
                              v0, e1, e2)
    if not hit.any():
        return None
    k = int(np.argmin(np.where(hit, t, np.inf)))
    n = np.cross(e1[k], e2[k])
    length = float(np.linalg.norm(n))
    if length < 1e-30:
        return None
    return origin + direction * float(t[k]), QVector3D(*(n / length)), int(pl[k])


def _on_plane(hit, plane, depth):
    p, n = plane
    return (abs(QVector3D.dotProduct(hit[1], n)) > 0.999
            and abs(QVector3D.dotProduct(hit[0] - p, n)) <= 1e-6 + 1e-5 * depth)


#: Distinct planes behind the face under the cursor that a face crossing
#: tries, as GL's search tries its first seven faces.
CROSSING_CANDIDATES = 7


def face_crossing(index, view, x, y, row, point, normal, rows, project, context):
    """GL's ``DocumentInteraction._face_crossing_point`` from the snap index (#819):
    where the face under the cursor meets another face crossing it with no
    edge between them — ``(world point, context)`` or ``None``.

    The candidates are the triangles the cursor ray meets a little behind
    the face (a face that goes THROUGH the one in front is met there), one
    ray over the placements under the cursor; the crossing line and its two
    probes are :func:`core.snap.face_crossing_near_ray`, as GL's; a crossing
    counts if one probe shows the face's plane and the other the crossing
    face's — the two really meet there, in view. A view knows triangles,
    not faces: a plane stands for the face (two coplanar faces meeting the
    same third one meet it along the same line)."""
    from views.document_interaction import DocumentInteraction, _active_cut, _moller_trumbore
    if _active_cut(view.scene) is not None:
        return None
    origin, direction = view._pixel_to_ray(x, y)
    if origin is None or direction is None:
        return None
    t_f = QVector3D.dotProduct(point - origin, direction)
    ring = DocumentInteraction._CROSSING_RING_PX
    o2, d2 = view._pixel_to_ray(x + ring, y)
    if o2 is None or d2 is None:
        return None
    reach = ((o2 + d2 * t_f) - (origin + direction * t_f)).length() * 50.0
    mvp = view.camera.projection_matrix() * view.camera.view_matrix()
    near = _screen_rows(index, view.packed, mvp, view.width(), view.height(),
                        x, y, 1., rows)
    if not len(near):
        return None
    pl, _, v0, e1, e2 = index.ray_cands(near, origin.toTuple(), direction.toTuple(),
                                        t_f + reach)
    if not len(v0):
        return None
    t, hit = _moller_trumbore(np.array(origin.toTuple()), np.array(direction.toTuple()),
                              v0, e1, e2)
    slack = 1e-6 + 1e-5 * t_f
    cand = np.flatnonzero(hit & (t >= t_f - slack) & (t <= t_f + reach))
    if not len(cand):
        return None
    cand = cand[np.argsort(t[cand], kind='stable')]
    f = (point, normal.normalized())
    planes, seen = [], []
    for k in cand.tolist():
        n = np.cross(e1[k], e2[k])
        length = float(np.linalg.norm(n))
        if length < 1e-30:
            continue
        n = QVector3D(*(n / length))
        p = QVector3D(*v0[k])
        if abs(QVector3D.crossProduct(f[1], n).length()) < 0.02:
            continue                        # the face itself, or parallel
        if any(abs(QVector3D.dotProduct(n, m)) > 0.999
               and abs(QVector3D.dotProduct(p - q, m)) <= slack for q, m in seen):
            continue                        # another triangle of that plane
        seen.append((p, n))
        planes.append(((p, n), int(pl[k])))
        if len(planes) >= CROSSING_CANDIDATES:
            break
    reach_px = max(getattr(view, 'edge_snap_threshold_px', None) or 14., ring)
    for g, g_row in planes:
        found = face_crossing_near_ray(x, y, origin, direction, f, g, project,
                                       reach_px, DocumentInteraction._CROSSING_SIDE_PX)
        if found is None:
            continue
        crossing, probes = found
        depth = (crossing - origin).length()
        sides = []
        for q in probes:
            h = _front_at(index, view, q[0], q[1], rows, mvp)
            if h is None:
                break
            side = ('f' if _on_plane(h, f, depth) else
                    'g' if _on_plane(h, g, depth) else None)
            if side is None:
                break
            sides.append(side)
        if sorted(sides) != ['f', 'g']:
            continue
        words = [w for w in (snap_context(view, r, context) for r in (row, g_row))
                 if w is not None]
        word = 'outside' if 'outside' in words else (words[0] if words else None)
        return crossing, word
    return None


def start_edges(index, view, start, rows):
    """The edges through ``start``, for «perpendicular to the edge you
    started on»: the snap only hands in the edges near the cursor, and the
    start edge is far from it once the line is longer than that radius.
    Asked once per start, camera and index state (a 2 px query at the
    start's pixel), then reused for every move of that segment."""
    mvp = view.camera.projection_matrix() * view.camera.view_matrix()
    key = (start.toTuple(), tuple(mvp.data()), view.width(), view.height(),
           index.epoch, id(view.scene.edit_group))
    cached = getattr(index, 'start_edges_cache', None)
    if cached is not None and cached[0] == key:
        return cached[1]
    from views.document_interaction import DocumentInteraction
    pixel = DocumentInteraction._world_to_pixel(view, start, mvp)
    found = []
    if pixel is not None:
        from core.snap import _point_on_segment_world
        found = [e for e in nearby_edges(
            index, view.packed, view.camera, view.width(), view.height(),
            pixel[0], pixel[1], radius_px=2., cap=16, rows=rows,
            edit_group=view.scene.edit_group)
            if _point_on_segment_world(start, e.a, e.b)]
    index.start_edges_cache = (key, found)
    return found


def _acquire(engine, view, edges, on_face, point, face_normal,
             options, x, y, modifiers=None):
    """GL's ``InferenceEngine.acquire`` from this view's own picks: the edge
    and the face under the cursor become the soft references a segment
    being drawn reads (parallel / perpendicular to the edge, perpendicular
    to the face), and, under a direction lock, what the lock line meets.
    The candidates are the ones the snap already holds: the nearest edge
    within the edge reach whose point under the cursor is visible (at most
    three asked, one ray each, only while a segment is drawn)."""
    tool = getattr(getattr(view, 'tools', None), 'tool', None)
    drawing = (tool is not None and getattr(tool, 'uses_snap', False)
               and getattr(tool, 'start_point', None) is not None)
    hover = None
    if drawing:
        reach = options.get('edge_threshold_px') or 14.
        occluded = options.get('is_occluded')
        if isinstance(edges, ResidentEdges):
            indexes = np.flatnonzero(edges.distances <= reach)
            indexes = indexes[np.argsort(edges.distances[indexes], kind='stable')]
            candidates = (edges[len(edges.prefix)+i] for i in indexes)
            import itertools
            near = list(itertools.islice((e for e in candidates
                        if (e.b-e.a).lengthSquared() > 1e-18), 3))
        else:
            near = sorted((e for e in edges if hasattr(e, 'distance')
                       and e.distance <= reach
                       and (e.b - e.a).lengthSquared() > 1e-18),
                      key=lambda e: e.distance)
        from views.document_interaction import DocumentInteraction
        for e in near[:3]:
            delta = e.b - e.a
            foot = DocumentInteraction._project_to_lock_line(view, e.a, delta, x, y)
            t = QVector3D.dotProduct(foot - e.a, delta) / delta.lengthSquared()
            if occluded is None or not occluded(e.a + delta * max(0., min(1., t))):
                hover = e
                break
    engine.note_drawing_refs(tool, edge=hover,
                             face_normal=face_normal if on_face else None)
    options['acquired_edge'] = engine._acquired_edge
    options['acquired_face_normal'] = engine._acquired_face_normal
    from core.input_types import Qt
    if drawing and engine.direction_locked(
            modifiers if modifiers is not None else Qt.NoModifier):
        if hover is not None:
            options.setdefault('hover_lock_edge', hover)
        elif on_face and face_normal is not None:
            options.setdefault('hover_lock_plane', (point, face_normal, point))


#: Faces in the mesh under the cursor above which the Metal view does not
#: look for the face itself to offer its «Face center» (the native face
#: pick walks the mesh in Python); the selected faces' centres still come.
FACE_CENTER_PICK_MAX = 5000


def face_centers_near(view, x, y, on_face, project, *, acquire=False):
    """GL's ``DocumentInteraction._face_centers_near`` for the Metal view (#736):
    ``[(point, context)]`` — the centre of area of the face under the
    cursor (when it falls inside the face and near the cursor) and of the
    selected faces near it."""
    from core.mesh import Face, face_count
    from core.snap import face_center_of_face
    from views.document_interaction import DocumentInteraction
    reach = DocumentInteraction.POINT_INFERENCE_PX
    tools = getattr(view, 'tools', None)
    tool = getattr(tools, 'tool', None)
    moving = getattr(tool, 'moving_face_ids', None)
    moving = moving() if callable(moving) else ()

    def near(c):
        pc = project(c) if c is not None else None
        return pc is not None and math.hypot(pc[0] - x, pc[1] - y) <= reach

    out = []
    picked = (tools._picked_mesh(x, y)
              if on_face and hasattr(tools, '_picked_mesh') else None)
    if picked is not None and face_count(picked[0]) <= FACE_CENTER_PICK_MAX:
        face, place = tools.pick_face_placement(x, y)
        if face is not None and face.key not in moving:
            group = picked[1]
            context = None
            if place is not None and group is not None:
                context = 'component' if group.is_instance() else 'group'
            if acquire:
                ref = view.source_viewport._center_of_face(face, place, picked[0], x, y)
                if ref is not None:
                    view.source_viewport._center_ref = ref
            c = face_center_of_face(face, getattr(place, 'xform', None))
            if near(c):
                out.append((c, context))
    faces = [e for e in view.scene.selection if isinstance(e, Face)]
    if faces and len(faces) <= DocumentInteraction.FACE_CENTER_SELECTION_MAX:
        for f in faces:
            if f.key in moving:
                continue
            c = face_center_of_face(f)
            if near(c):
                out.append((c, None))
    return out

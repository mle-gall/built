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
from types import SimpleNamespace
import math

from ingetrazo_core import ResidentEdges, resident_prepare as prepare, resident_rebase as rebase, tangent_direction
import numpy as np
from ingetrazo_core import (Vec3 as QVector3D, nearest_inference_triangle, context_hit, eligible_rows,
                            packed_surface, screen_boxes, screen_rows as _screen_rows, surface, surface_hit)
from core.snap import SurfacePatch, compute_snap, face_crossing_near_ray


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
    origin, direction = view._pixel_to_ray(x, y)
    if origin is None or direction is None:
        return None
    near = _screen_rows(index, view.packed, mvp, view.width(), view.height(),
                        x, y, 1., rows)
    if not len(near):
        return None
    pl, _, v0, e1, e2 = index.ray_cands(near, origin.toTuple(), direction.toTuple(),
                                        float('inf'))
    hit = nearest_inference_triangle(origin.toTuple(), direction.toTuple(), v0, e1, e2)
    if hit is None:
        return None
    k, distance, normal = hit
    if normal is None:
        return None
    return origin + direction * distance, QVector3D(*normal), int(pl[k])


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
    from core.input_codes import Input as Qt
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

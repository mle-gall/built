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

from ingetrazo_core import ResidentEdges, resident_prepare as prepare, resident_rebase as rebase, tangent_direction
import numpy as np
from ingetrazo_core import (Vec3 as QVector3D, context_hit, eligible_rows,
                            packed_surface, screen_boxes, screen_rows as _screen_rows, surface, surface_hit)
from ingetrazo_core import (acquire as _acquire, acquire_rim as _acquire_rim, face_centers_near,
                            face_crossing, front_at as _front_at, nearby_edges, occluded as _occluded,
                            resident_occluded_many as _occluded_many, on_surface, placement_context,
                            reference_edge, scene_edges, snap_context, start_edges)
from core.snap import SurfacePatch, compute_snap


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

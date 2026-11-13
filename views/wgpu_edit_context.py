"""Edit-context membership and screen scaffolding shared by native captures."""
import math
import numpy as np
from PySide6.QtGui import QVector3D


def rest_rows(groups, context):
    if context is None:
        return []
    from core.group import iter_placements
    subject = {id(g) for g, _ in iter_placements(context)}
    return [row for row, group in enumerate(groups) if id(group) not in subject]


def box_primitives(view):
    from views.viewport import Viewport, _box_edge_vertices
    from views.wgpu_overlay import Primitive
    source = view.source_viewport
    if source is None or source.plano_style is not None or source.style_override is not None:
        return []
    box = view._edit_group_box_corners()
    if box is None:
        return []
    scale = view.devicePixelRatioF()
    dash = min(20., max(3., .003 * view.height() * scale /
                       math.tan(math.radians(view.camera.fov_deg) / 2))) / scale
    result = []
    for a, b in np.asarray(_box_edge_vertices(box)).reshape(-1, 2, 3):
        segment = Viewport._clip_segment_front(view, QVector3D(*a), QVector3D(*b))
        if segment is None:
            continue
        points = tuple(view._world_to_pixel(p) for p in segment)
        if all(p is not None for p in points):
            result.append(Primitive('polyline', points, '#5a6e8c', 1 / scale,
                                    dash=(dash * scale, dash * scale)))
    return result


def selection_box_primitives(view):
    """The box around each selected group, in the group's own axes: GL's
    selection cue (``_box_edges`` in the selected-edges pass) had no
    Metal counterpart, so a selected group only tinted — on the ride a
    support picked in a forest of others gave no sense of its extent.

    Skipped like GL: face-me billboards (their mesh quad is not what is
    on screen) and groups a Move/Rotate drag is previewing. Boxes are
    cached per group and document version: a selection change reuses
    them, an edit (move, scale, geometry) measures again."""
    from core.group import Group, oriented_box_corners
    from views.viewport import Viewport, _box_edge_vertices
    from views.wgpu_overlay import Primitive
    source = view.source_viewport
    if source is None or source.plano_style is not None or source.style_override is not None:
        return []
    groups = [g for g in view.scene.selection if isinstance(g, Group)]
    if not groups:
        return []
    moving = {id(g) for g in (getattr(source, '_preview_groups', None) or ())}
    version = view.scene.version - view.scene.view_version
    cache = view.__dict__.setdefault('_selection_boxes', {})
    if cache.get(None) != version:
        cache.clear()
        cache[None] = version
    scale = view.devicePixelRatioF()
    result = []
    for group in groups:
        if getattr(group, 'billboard', False) or id(group) in moving:
            continue
        box = cache.get(group)
        if box is None:
            box = cache[group] = oriented_box_corners(*Viewport._compute_obb(group))
        for a, b in np.asarray(_box_edge_vertices(box)).reshape(-1, 2, 3):
            segment = Viewport._clip_segment_front(view, QVector3D(*a), QVector3D(*b))
            if segment is None:
                continue
            points = tuple(view._world_to_pixel(p) for p in segment)
            if all(p is not None for p in points):
                result.append(Primitive('polyline', points, '#f2732a', 1.5 / scale))
    return result

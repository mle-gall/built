"""Edit-context membership and screen scaffolding shared by native captures."""
import math
import numpy as np
from ingetrazo_core import Vec3 as QVector3D


def rest_rows(groups, context):
    if context is None:
        return []
    from core.purge import iter_groups
    subject = {id(g) for g in iter_groups((context,))}
    return [row for row, group in enumerate(groups) if id(group) not in subject]


def box_primitives(view):
    from views.document_interaction import DocumentInteraction, _box_edge_vertices
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
        segment = DocumentInteraction._clip_segment_front(view, QVector3D(*a), QVector3D(*b))
        if segment is None:
            continue
        points = tuple(view._world_to_pixel(p) for p in segment)
        if all(p is not None for p in points):
            result.append(Primitive('polyline', points, '#5a6e8c', 1 / scale,
                                    dash=(dash * scale, dash * scale)))
    return result


_NO_BOXES = (None, np.empty((0, 2, 3), np.float32))


def selection_box_segments(view):
    """``(key, segments)``: the box around each selected group, in the
    group's own axes, as world segments ``(N, 2, 3)``. GL's selection cue
    (``_box_edges`` in the selected-edges pass) had no Metal counterpart,
    so a selected group only tinted; on the ride a support picked in a
    forest of others gave no sense of its extent.

    GL draws the box in the scene pass, depth-tested: the edges behind
    the group are hidden. The Metal view sends these segments to the
    native preview's edges (depth-tested) for the same look; ``key``
    changes exactly when the segments do, so the upload is skipped
    otherwise.

    Skipped like GL: face-me billboards (their mesh quad is not what is
    on screen) and groups a Move/Rotate drag is previewing. Boxes are
    cached per group and document version: a selection change reuses
    them, an edit (move, scale, geometry) measures again."""
    from core.group import Group, oriented_box_corners
    from views.document_interaction import DocumentInteraction, _box_edge_vertices
    source = view.source_viewport
    if source is None or source.plano_style is not None or source.style_override is not None:
        return _NO_BOXES
    groups = [g for g in view.scene.selection if isinstance(g, Group)]
    if not groups:
        return _NO_BOXES
    moving = {id(g) for g in (getattr(source, '_preview_groups', None) or ())}
    tools = view.__dict__.get('tools')
    moving.update(id(g) for g in (getattr(tools, '_moving_groups', None) or ()))
    version = view.scene.version - view.scene.view_version
    cache = view.__dict__.setdefault('_selection_boxes', {})
    if cache.get(None) != version:
        cache.clear()
        cache[None] = version
    shown = [g for g in groups
             if not getattr(g, 'billboard', False) and id(g) not in moving]
    key = (version, frozenset(map(id, shown)))
    if cache.get('key') == key:
        return key, cache['segments']
    parts = []
    for group in shown:
        box = cache.get(group)
        if box is None:
            box = cache[group] = oriented_box_corners(*DocumentInteraction._compute_obb(group))
        parts.append(np.asarray(_box_edge_vertices(box), np.float32).reshape(-1, 2, 3))
    segments = np.concatenate(parts) if parts else _NO_BOXES[1]
    cache['key'], cache['segments'] = key, segments
    return key, segments


def selection_box_vertices(view):
    """``(key, VERTEX array)``: the boxes as native preview line vertices
    in GL's selection orange, cached with the segments: a mesh tool's
    feedback asks for them on every frame."""
    from views.wgpu_scene import VERTEX
    key, segments = selection_box_segments(view)
    cache = view.__dict__.setdefault('_selection_boxes', {})
    if cache.get('vertex_key') != key or 'vertices' not in cache:
        lines = np.zeros(len(segments) * 2, VERTEX)
        lines['pos'] = np.asarray(segments).reshape(-1, 3)
        lines['normal'] = (0, 0, 1)
        lines['color'] = (242, 114, 40, 255)        # GL's (.95, .45, .16)
        cache['vertex_key'], cache['vertices'] = key, lines
    return key, cache['vertices']


def selection_box_primitives(view):
    """The same boxes as overlay lines, for a renderer without native
    preview edges (no depth test there)."""
    from views.document_interaction import DocumentInteraction
    from views.wgpu_overlay import Primitive
    _, segments = selection_box_segments(view)
    scale = view.devicePixelRatioF()
    result = []
    for a, b in segments:
        segment = DocumentInteraction._clip_segment_front(view, QVector3D(*a), QVector3D(*b))
        if segment is None:
            continue
        points = tuple(view._world_to_pixel(p) for p in segment)
        if all(p is not None for p in points):
            result.append(Primitive('polyline', points, '#f2732a', 1.5 / scale))
    return result


def billboard_outline_primitives(view):
    """The orange outline GL draws around a selected face-me figure, on
    the quad as it faces the camera this frame (``_draw_billboard_outlines``).
    The Metal view tinted the figure's rows instead, which GL deliberately
    does not do (a stray plane through the figure), and gave no outline.

    The quad is GL's own ``_billboard_quad`` on the source viewport: its
    extent is cached per figure content, so this is four projections per
    selected figure per frame; nothing when no figure is selected."""
    from core.group import Group
    from views.document_interaction import DocumentInteraction
    from views.wgpu_overlay import Primitive
    source = view.source_viewport
    quad_of = getattr(source, '_billboard_quad', None)
    if quad_of is None or source.plano_style is not None or source.style_override is not None:
        return []
    visible = getattr(view.scene, 'entity_visible', lambda g: True)
    result = []
    for group in view.scene.selection:
        if not (isinstance(group, Group) and getattr(group, 'billboard', False)
                and visible(group)):
            continue
        quad = quad_of(group)
        if quad is None:
            continue
        corners = quad[0]
        for a, b in zip(corners, corners[1:] + corners[:1]):
            segment = DocumentInteraction._clip_segment_front(view, a, b)
            if segment is None:
                continue
            points = tuple(view._world_to_pixel(p) for p in segment)
            if all(p is not None for p in points):
                # A GL line is one physical pixel; as the selection box.
                result.append(Primitive('polyline', points, '#f2732a',
                                        1.5 / view.devicePixelRatioF()))
    return result

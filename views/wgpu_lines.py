"""World-space scaffold lines for the native line pass (``lines.rs``).

GL draws the open group's dashed box, the guide lines and a selected
group's hidden edges as GL_LINES in its scene pass: depth-tested, so the
parts behind faces are hidden. The Metal overlay has no depth, so these
showed through the model there. They now go to ``Renderer.set_lines``,
one named slot each, and are uploaded only when their key changes.

The dash of the box and of the hidden edges is a screen pattern computed
on the GPU from the projected segment (GL's ``u_stipple 4``), so a camera
move re-uploads nothing for them. The box's corners do move with the
camera (GL pads it by a screen gutter), which re-sends twelve segments.
Guides keep GL's world-space dashes (``_guide_vertices``), which GL
itself rebuilds when the camera changes; they are sent as solid pieces.
"""
import math

import numpy as np

SEGMENT = np.dtype([('a', '<f4', 3), ('b', '<f4', 3), ('color', 'u1', 4),
                    ('width', '<f4'), ('dash', '<f4'), ('gap', '<f4')])
assert SEGMENT.itemsize == 40

EDIT_BOX_RGBA = (90, 110, 140, 255)          # GL's (90, 110, 140) / 255
GUIDE_RGBA = (69, 89, 120, 255)              # GL's (.27, .35, .47)
SELECTED_RGBA = (242, 115, 41, 255)          # GL's (.95, .45, .16)
EMPTY = np.zeros(0, SEGMENT)


def segments(pairs, rgba, width=1.0, dash=0.0, gap=None):
    """``(N, 2, 3)`` world endpoints as SEGMENT records. ``width``, ``dash``
    and ``gap`` are physical pixels; ``dash`` 0 is a solid line."""
    pairs = np.asarray(pairs, np.float32).reshape(-1, 2, 3)
    out = np.zeros(len(pairs), SEGMENT)
    out['a'], out['b'] = pairs[:, 0], pairs[:, 1]
    out['color'] = rgba
    out['width'] = width
    out['dash'] = dash
    out['gap'] = dash if gap is None else gap
    return out


def _scaffold_shown(view):
    """GL draws no scaffolding on a plan sheet or a styled composer frame."""
    source = view.source_viewport
    return (source is not None and source.plano_style is None
            and source.style_override is None)


def edit_box(view):
    """``(key, segments)``: the twelve dashed edges of the open group's box,
    GL's ``_draw_scaffold`` dash (window pixels from the field of view)."""
    from views.viewport import _box_edge_vertices
    box = view._edit_group_box_corners() if _scaffold_shown(view) else None
    if box is None:
        return None, EMPTY
    height = view.height() * view.devicePixelRatioF()
    dash = min(20., max(3., .003 * height / math.tan(math.radians(view.camera.fov_deg) / 2)))
    key = (tuple(p.toTuple() for p in box), dash)
    return key, segments(np.asarray(_box_edge_vertices(box)), EDIT_BOX_RGBA, 1.0, dash)


def guides(view):
    """``(key, segments)``: GL's world-space guide dashes, keyed as GL keys
    its guide cache (camera distance, target and eye; the guides; which
    are selected), so a hover repaint reuses the upload."""
    from views.viewport import shown_guides
    scene, camera = view.scene, view.camera
    on_screen = shown_guides(scene) if _scaffold_shown(view) else []
    if not on_screen:
        return None, EMPTY
    v3 = lambda v: None if v is None else (v.x(), v.y(), v.z())  # noqa: E731
    key = (float(camera.distance), v3(camera.target),
           v3(camera.eye()) if camera.perspective else None,
           tuple((id(g), v3(g.point), v3(getattr(g, 'direction', None)),
                  v3(getattr(g, 'origin', None))) for g in on_screen),
           tuple(g in scene.selection for g in on_screen))
    cache = view.__dict__.get('_guide_lines')
    if cache is not None and cache[0] == key:
        return key, cache[1]
    from views.wgpu_annotations import guide_segments
    pairs, selected = guide_segments(scene, camera)
    out = segments(pairs, GUIDE_RGBA)
    out['color'][selected] = SELECTED_RGBA
    view._guide_lines = (key, out)
    return key, out


def _local_hidden_edges(scene, mesh):
    """Soft and hidden edges of one mesh in its own coordinates, layer
    filtered: GL's ``_hidden_edge_draw(mesh, 'selected_hidden')``. A lazy
    imported mesh answers from its plan, never materialising its faces."""
    from core.lazy_mesh import plan_of
    plan = plan_of(mesh)
    if plan is not None:
        mask = plan.soft | plan.hidden
        if any(layer is not None for layer in plan.layer):
            from types import SimpleNamespace
            visible = {layer: scene._layer_state(SimpleNamespace(layer=layer))[0]
                       for layer in set(plan.layer)}
            mask = mask & np.fromiter((visible[l] for l in plan.layer), bool)
        return np.asarray(plan.pos, np.float32)[np.asarray(plan.edges)[mask]].reshape(-1, 2, 3)
    pairs = [(e.a.toTuple(), e.b.toTuple()) for e in mesh.edges
             if (e.soft or e.hidden) and scene._layer_state(e)[0]]
    return np.asarray(pairs, np.float32).reshape(-1, 2, 3)


def selected_hidden(view):
    """``(key, segments)``: the selected groups' soft and hidden edges,
    dashed orange, while View > Hidden Geometry is on (GL's
    ``_draw_selected_group_edges`` ``selected_hidden`` kind: 3 px dashes
    times the pixel ratio). Placed by the rows' world matrices, so every
    instance of a shared component shows its own."""
    from core.group import Group
    scene, packed = view.scene, getattr(view, 'packed', None)
    if (not getattr(scene, 'show_hidden_geometry', False) or packed is None
            or not _scaffold_shown(view)):
        return None, EMPTY
    groups = [g for g in scene.selection if isinstance(g, Group)]
    if not groups:
        return None, EMPTY
    source = view.source_viewport
    moving = {id(g) for g in (getattr(source, '_preview_groups', None) or ())}
    tools = view.__dict__.get('tools')
    moving.update(id(g) for g in (getattr(tools, '_moving_groups', None) or ()))
    from core.layers import effective_visible
    layers = tuple((layer.key, effective_visible(scene, layer)) for layer in getattr(scene, 'layers', ()))
    version = scene.version - scene.view_version
    key = (version, id(packed), frozenset(map(id, groups)), frozenset(moving), layers)
    cache = view.__dict__.get('_hidden_lines')
    if cache is not None and cache[0] == key:
        return key, cache[1]
    visible = getattr(scene, 'entity_visible', lambda g: True)
    rows = sorted({r for g in groups for r in view._rows_for_group(g)})
    local = {}
    parts = []
    for row in rows:
        placed = packed.groups[row]
        if (placed is None or getattr(placed, 'billboard', False)
                or id(placed) in moving or not visible(placed)):
            continue
        mesh = placed.mesh
        if id(mesh) not in local:
            local[id(mesh)] = _local_hidden_edges(scene, mesh)
        pairs = local[id(mesh)]
        if not len(pairs):
            continue
        # Instance matrices are stored column by column (the Rust ABI).
        world = np.asarray(packed.instances['world'][row], np.float32)
        parts.append(pairs @ world[:3, :3] + world[3, :3])
    pairs = np.concatenate(parts) if parts else np.empty((0, 2, 3), np.float32)
    dash = 3.0 * max(1.0, float(view.devicePixelRatioF()))
    out = segments(pairs, SELECTED_RGBA, 1.0, dash)
    view._hidden_lines = (key, out)
    return key, out


SLOTS = (('edit_box', edit_box), ('guides', guides), ('selected_hidden', selected_hidden))


def sync(view):
    """Send each slot whose key changed; return the slots sent (tests and
    timing). Called once per frame; an unchanged slot costs its key."""
    renderer = view.renderer
    # A new renderer starts with empty slots (None keys); its keys are its own.
    owner, keys = view.__dict__.get('_line_keys', (None, None))
    if owner is not renderer:
        keys = {}
        view._line_keys = (renderer, keys)
    sent = []
    for slot, build in SLOTS:
        key, data = build(view)
        if keys.get(slot) != key:
            renderer.set_lines(slot, np.ascontiguousarray(data).view(np.uint8).reshape(-1))
            keys[slot] = key
            sent.append(slot)
    return sent

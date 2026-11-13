"""Resident parity handoffs. Work is keyed to scene edits, never screen pixels."""
import numpy as np
from views.wgpu_scene import VERTEX, OBJECT_HIDDEN, VISIBILITY_FILTERED


def raw(array):
    return np.ascontiguousarray(array).view(np.uint8).reshape(-1)


def has_transparency(packed):
    alpha = packed.vertices['color'][:, 3]
    if np.any((alpha > 0) & (alpha < 255)):
        return True
    # Back-side material RGBA records follow the prototype headers.
    for header in packed.materials[:len(packed.commands)*16].view('<u4').reshape(-1, 4):
        if header[2]:
            a = packed.materials[int(header[1])*16:int(header[1])*16+int(header[2])*32].reshape(-1, 32)[:, 19]
            if np.any((a > 0) & (a < 255)):
                return True
    if packed.textures:
        a = np.frombuffer(packed.textures[3][0], np.uint8)[3::4]
        if np.any(a < 255):
            return True
    return False


def hidden_geometry(view):
    renderer, packed = view.renderer, view.packed
    if renderer is None or packed is None or not hasattr(renderer, 'set_hidden_geometry') or getattr(packed, 'hidden_commands', None) is None:
        return
    geometry, objects_enabled = packed.reveal_hidden
    if not geometry and not objects_enabled:
        return  # set_scene cleared ghosts; feature-off packs do no expansion
    flags = packed.instances['info'][:, 2]
    objects = np.flatnonzero(((flags & OBJECT_HIDDEN) != 0) & ((flags & (VISIBILITY_FILTERED | 128)) == 0)).tolist()
    if not objects_enabled:
        objects = []
    renderer.set_hidden_geometry(raw(packed.vertices), raw(packed.indices),
        raw(packed.hidden_commands), raw(packed.hidden_edge_commands), objects)


def face_ranges(view, faces):
    """Resolve editable faces against the pack's stable visible-face order.

    Only the active mesh is materialized. Face.triangulate and the resident bake
    share core's triangulation kernel, including holes and degenerate faces.
    Cache lives on the pack, so replacing geometry invalidates it automatically.
    """
    if not faces:
        return []
    packed = view.packed
    mesh = view.scene.mesh
    cache = getattr(packed, '_face_ranges', None)
    key = id(mesh)
    if cache is None:
        cache = packed._face_ranges = {}
    if key not in cache:
        rows = [i for i, g in enumerate(packed.groups)
                if (view.scene.loose_mesh if g is None else g.mesh) is mesh]
        mapping, first = {}, 0
        for f in mesh.faces:
            if f.attrs.get('hidden'):
                continue
            count = len(f.triangulate())
            mapping[f.key] = (first, count)
            first += count
        codes = sorted({int(packed.instances['info'][i, 0]) for i in rows})
        # Refuse an invalid range rather than suppress unrelated geometry.
        if any(first*3 != int(packed.commands[c]['count']) for c in codes):
            raise ValueError('editable face triangulation differs from resident pack')
        cache[key] = codes, mapping
    codes, mapping = cache[key]
    return [(c, *mapping[f.key]) for f in faces if f.key in mapping for c in codes]


def face_marks(view, faces):
    parts = []
    for face in faces:
        for code, first, count in face_ranges(view, [face]):
            cmd = view.packed.commands[code]
            indices = view.packed.indices[int(cmd['first'])+first*3:int(cmd['first'])+(first+count)*3]
            vertices = view.packed.vertices[indices.astype(np.int64)+int(cmd['base'])].copy()
            choose = getattr(view.source_viewport, '_selection_dot_colors', None)
            color = (.95, .45, .16)
            if choose:
                from PySide6.QtGui import QVector3D
                front, back = choose(face)
                color = front if QVector3D.dotProduct(face.normal(), view.camera.eye()-face.vertices[0]) > 0 else back
            vertices['color'] = [*np.asarray(np.clip(np.array(color)*255, 0, 255), np.uint8), 255]
            parts.append(vertices)
    return raw(np.concatenate(parts) if parts else np.empty(0, VERTEX))


def hover_face_marks(view, face, placement=None):
    """The dots GL puts on the face under the cursor of Push/Pull, Paint,
    Offset, Follow Me... (``_paint_hover``): a selected face's dots, in
    the colour picked against the side you see. The Metal view outlined
    the face in cyan instead, which reads as a selected edge loop.

    Built from the face's own triangulation through ``placement``'s
    matrix (a face inside a component is stored in its part's
    coordinates), not from the pack, so it works on any placement and
    costs one face per hover change."""
    from PySide6.QtGui import QVector3D
    matrix = getattr(placement, 'xform', None)
    points = [p for tri in face.triangulate() for p in tri]
    if matrix is not None:
        points = [matrix.map(p) for p in points]
    if not points:
        return raw(np.empty(0, VERTEX))
    color = (.95, .45, .16)
    choose = getattr(view.source_viewport, '_selection_dot_colors', None)
    if choose:
        front, back = choose(face)
        normal, origin = face.normal(), face.vertices[0]
        if matrix is not None:
            normal, origin = matrix.mapVector(normal), matrix.map(origin)
        color = front if QVector3D.dotProduct(normal, view.camera.eye()-origin) > 0 else back
    rgba = [*np.asarray(np.clip(np.array(color[:3])*255, 0, 255), np.uint8), 255]
    return raw(np.array([(p.toTuple(), (0, 0, 1), rgba) for p in points], VERTEX))


def preview_mesh(faces, edges=()):
    triangles = []
    for face in faces:
        attrs = getattr(face, 'attrs', None) or {}
        color = attrs.get('color', (.96, .95, .925))
        normal = face.normal().toTuple()
        rgba = [*np.asarray(np.clip(np.array(color[:3])*255, 0, 255), np.uint8), max(1, round(attrs.get('opacity', 1)*255))]
        triangles.extend((p.toTuple(), normal, rgba) for tri in face.triangulate() for p in tri)
    # An edge is (a, b) in the dark edge colour or (a, b, rgba) in its own:
    # the rubber band keeps the axis/lock colour GL gives it.
    lines = []
    for edge in edges:
        rgba = (33, 43, 59, 255)
        if len(edge) > 2:
            c = tuple(edge[2]) + (1.0,) * (4 - len(edge[2]))
            rgba = tuple(int(v) for v in np.clip(np.array(c[:4]) * 255, 0, 255).round())
        lines.extend((tuple(map(float, p.toTuple() if hasattr(p, 'toTuple') else p)), (0, 0, 1), rgba)
                     for p in edge[:2])
    return raw(np.array(triangles, VERTEX)), raw(np.array(lines, VERTEX))

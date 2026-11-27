# SPDX-License-Identifier: GPL-3.0-or-later
"""Faceted bodies and local prototype maps without allocating Python faces."""
from core.bim import face_set_volume
from .step import _f

def _brep(w: object, faces, context_id: int) -> int:
    """Faceted geometry for a face set → IfcShapeRepresentation id. Closed
    sets become an IfcFacetedBrep; open ones an IfcShellBasedSurfaceModel."""
    point_ids: dict = {}

    def pt(v) -> int:
        key = tuple(round(x, 6) for x in v.position_tuple())
        pid = point_ids.get(key)
        if pid is None:
            pid = w.add("IFCCARTESIANPOINT",
                        f"({_f(key[0])},{_f(key[1])},{_f(key[2])})")
            point_ids[key] = pid
        return pid

    face_ids = []
    for f in faces:
        bounds = []
        outer = w.add("IFCPOLYLOOP",
                      "(" + ",".join(f"#{pt(v)}" for v in f.loop)
                      + ")")
        bounds.append(f"#{w.add('IFCFACEOUTERBOUND', f'#{outer}', '.T.')}")
        for h in f.hole_loops:
            loop = w.add("IFCPOLYLOOP",
                         "(" + ",".join(f"#{pt(v)}" for v in h) + ")")
            bounds.append(f"#{w.add('IFCFACEBOUND', f'#{loop}', '.T.')}")
        face_ids.append(w.add("IFCFACE", "(" + ",".join(bounds) + ")"))

    face_list = "(" + ",".join(f"#{i}" for i in face_ids) + ")"
    closed = face_set_volume(faces) is not None
    if closed:
        shell = w.add("IFCCLOSEDSHELL", face_list)
        item = w.add("IFCFACETEDBREP", f"#{shell}")
        rep_type = "'Brep'"
    else:
        shell = w.add("IFCOPENSHELL", face_list)
        item = w.add("IFCSHELLBASEDSURFACEMODEL", f"(#{shell})")
        rep_type = "'SurfaceModel'"
    w.last_shape_item = item
    return w.add("IFCSHAPEREPRESENTATION", f"#{context_id}", "'Body'",
                 rep_type, f"(#{item})")



def array_body(w, arrays, context_id, geometry='brep'):
    """Write array rings directly, batching coordinate formatting in NumPy."""
    import numpy as np
    from built_core.geometry_arrays import Arrays, merged, quantities
    a = arrays if isinstance(arrays, Arrays) else merged(arrays)
    closed = quantities(a)[3]
    pos, inverse = np.unique(np.round(a.pos[a.corners], 6), axis=0, return_inverse=True)
    coords = np.char.mod('%.6f', pos)
    rows = np.char.add(np.char.add(np.char.add(np.char.add(coords[:, 0], ','), coords[:, 1]), ','), coords[:, 2])
    if geometry == 'polygonal':
        point = w.entity('IfcCartesianPointList3D', CoordList='(' + ','.join(np.char.add(np.char.add('(', rows), ')')) + ')')
        refs = (inverse + 1).astype(str)
    else:
        ids = np.arange(w._next, w._next + len(pos))
        lines = np.char.add(np.char.add(np.char.add('#', ids.astype(str)), '=IFCCARTESIANPOINT(('), rows)
        w.stream.writelines(line + '));\n' for line in lines)
        w._next += len(pos)
        refs = np.char.add('#', ids[inverse].astype(str))
    rings, offset = [], 0
    for size in a.ring_sizes:
        rings.append('(' + ','.join(refs[offset:offset + size]) + ')')
        offset += size
    face_ids, offset = [], 0
    for count in a.ring_counts:
        loops = rings[offset:offset + count]
        offset += count
        if geometry == 'polygonal':
            if count == 1:
                face_ids.append(w.add('IFCINDEXEDPOLYGONALFACE', loops[0]))
            else:
                face_ids.append(w.add('IFCINDEXEDPOLYGONALFACEWITHVOIDS', loops[0], '(' + ','.join(loops[1:]) + ')'))
        else:
            bounds = []
            for i, loop in enumerate(loops):
                lid = w.add('IFCPOLYLOOP', loop)
                bounds.append(w.add('IFCFACEOUTERBOUND' if i == 0 else 'IFCFACEBOUND', f'#{lid}', '.T.'))
            face_ids.append(w.add('IFCFACE', '(' + ','.join(f'#{b}' for b in bounds) + ')'))
    face_list = '(' + ','.join(f'#{f}' for f in face_ids) + ')'
    if geometry == 'polygonal':
        item = w.add('IFCPOLYGONALFACESET', f'#{point}', '.T.' if closed else '.F.', face_list, '$')
        rep_type = "'Tessellation'"
    elif closed:
        shell = w.add('IFCCLOSEDSHELL', face_list)
        item = w.add('IFCFACETEDBREP', f'#{shell}')
        rep_type = "'Brep'"
    else:
        shell = w.add('IFCOPENSHELL', face_list)
        item = w.add('IFCSHELLBASEDSURFACEMODEL', f'(#{shell})')
        rep_type = "'SurfaceModel'"
    w.last_shape_item = item
    return w.add('IFCSHAPEREPRESENTATION', f'#{context_id}', "'Body'", rep_type, f'(#{item})')


def mapped_body(w, group, base, excluded, context_id, geometry, cache, axis):
    """Reuse local mesh bodies; prune ownership before mapping each occurrence.

    Element placements remain identity. Positive uniform orthogonal transforms
    map local prototypes into world metres; other transforms bake that occurrence.
    """
    import numpy as np
    from core.bim import _compose
    from built_core import group_np_affine as np_affine
    from built_core.geometry_arrays import arrays_of, placed_arrays

    items = []

    def walk(placed, parent):
        matrix = _compose(parent, placed.xform)
        a = arrays_of(placed.mesh)
        if len(a.ring_counts):
            rot, trans = np_affine(matrix) if matrix is not None else (np.eye(3), np.zeros(3))
            scale = float(np.linalg.norm(rot[:, 0]))
            uniform = (scale > 1e-12 and np.linalg.det(rot) > 0 and
                       np.allclose((rot / scale).T @ (rot / scale), np.eye(3),
                                   rtol=1e-6, atol=1e-6))
            if uniform:
                key = id(placed.mesh)
                if key not in cache:
                    rep = array_body(w, a, context_id, geometry)
                    cache[key] = w.add('IFCREPRESENTATIONMAP', f'#{axis}', f'#{rep}')
                directions = [w.add('IFCDIRECTION', '(' + ','.join(_f(v) for v in col) + ')')
                              for col in (rot / scale).T]
                origin = w.add('IFCCARTESIANPOINT', '(' + ','.join(_f(v) for v in trans) + ')')
                transform = w.add('IFCCARTESIANTRANSFORMATIONOPERATOR3D',
                                  f'#{directions[0]}', f'#{directions[1]}',
                                  f'#{origin}', _f(scale) if scale >= 1e-6 else format(scale, '.15E'),
                                  f'#{directions[2]}')
                item = w.add('IFCMAPPEDITEM', f'#{cache[key]}', f'#{transform}')
            else:
                baked = placed_arrays(a, matrix)
                rep = array_body(w, baked, context_id, geometry)
                # A mapped identity wrapper permits mixed baked and shared bodies
                # in one MappedRepresentation, including open surface models.
                mapping = w.add('IFCREPRESENTATIONMAP', f'#{axis}', f'#{rep}')
                origin = w.add('IFCCARTESIANPOINT', '(0.,0.,0.)')
                transform = w.add('IFCCARTESIANTRANSFORMATIONOPERATOR3D', '$', '$', f'#{origin}', '1.', '$')
                item = w.add('IFCMAPPEDITEM', f'#{mapping}', f'#{transform}')
            items.append(item)
        for child in placed.children or ():
            if not child.ifc and child.uid not in excluded:
                walk(child, matrix)

    walk(group, base)
    # Style each occurrence, never the shared prototype.
    w.last_shape_item = items
    return w.add('IFCSHAPEREPRESENTATION', f'#{context_id}', "'Body'",
                 "'MappedRepresentation'", '(' + ','.join(f'#{i}' for i in items) + ')')


def sweep_body(w, group, base, row, context_id, *, uncut=False):
    """Write the extrusion spans of a member, clipping each at its mitre planes."""
    import numpy as np
    from built_core import group_np_affine
    from built_core.command_protocol import sweep_profile
    from core.bim import _compose
    from .step import _s

    points = group.ext['built.live'].get('path', [])
    if group.ext['built.live'].get('body_cut') and not uncut:
        return None
    if len(points) < 2 or group.children or row['state'] != 'ok':
        return None
    matrix = _compose(base, group.xform)
    rot, trans = group_np_affine(matrix) if matrix is not None else (np.eye(3), np.zeros(3))
    scale = np.linalg.norm(rot[:, 0])
    if scale < 1e-12 or np.linalg.det(rot) <= 0 or not np.allclose(
            rot.T @ rot, np.eye(3) * scale**2, rtol=1e-10, atol=1e-14):
        return None
    profile = sweep_profile(row)
    points = np.asarray(points)
    closed = row['path'].get('closed', False)
    if closed:
        points = np.vstack([points, points[0]])
    vectors = np.diff(points, axis=0)
    depths = np.linalg.norm(vectors, axis=1)
    if np.any(depths < 1e-12):
        return None
    axes = vectors / depths[:, None]
    axis = axes[0]
    x = np.cross([0., 0., 1.], axis)
    x = x / np.linalg.norm(x) if np.linalg.norm(x) > 0. else np.array([1., 0., 0.])
    angle = np.deg2rad(row['rotation'])
    x = np.cos(angle) * x + np.sin(angle) * np.cross(axis, x)
    def coords(v):
        return '(' + ','.join(format(float(n), '.16E') for n in v) + ')'
    def add(entity, *args):
        return '#' + str(w.add(entity, *args))
    offset = np.array(profile['offset'])
    factor = np.array([-1 if row['mirror'] else 1, 1]) * scale / 1000
    size = np.array([profile['width'], profile['height']])
    middle = (size / 2 - offset) * factor
    centre = add('IFCAXIS2PLACEMENT2D', add('IFCCARTESIANPOINT', coords(middle)), '$')
    w0, h0, t, f, r = [_f(profile[k] * scale / 1000) for k in ('width', 'height', 'web', 'flange', 'radius')]
    args = ('.AREA.', _s(profile['name'] or 'Drawn profile'), centre)
    family = profile['family']
    section = None
    if family == 'rectangle':
        section = add('IFCRECTANGLEPROFILEDEF', *args, w0, h0)
    elif family == 'chs':
        section = add('IFCCIRCLEHOLLOWPROFILEDEF', *args, _f(float(w0) / 2), t)
    elif family == 'rhs':
        section = add('IFCRECTANGLEHOLLOWPROFILEDEF', *args, w0, h0, t,
                      _f(max(0., float(r) - float(t))), r)
    elif family == 'i':
        section = add('IFCISHAPEPROFILEDEF', *args, w0, h0, t, f, r, '$', '$')
    curves = []
    for ring in ([] if section else profile['rings']):
        pts, segments = [], []
        for i, (vertex, centre) in enumerate(ring):
            p = np.asarray(vertex)
            pts.append(coords((p - offset) * factor))
            start = len(pts)
            if centre is None:
                kind, indices = 'IFCLINEINDEX', [start]
            else:
                c = np.asarray(centre)
                q = np.asarray(ring[(i + 1) % len(ring)][0])
                bisector = (p + q) / 2 - c
                mid = c + bisector * np.linalg.norm(p - c) / np.linalg.norm(bisector)
                pts.append(coords((mid - offset) * factor))
                kind, indices = 'IFCARCINDEX', [start, len(pts)]
            indices.append(1 if i + 1 == len(ring) else len(pts) + 1)
            segments.append(kind + '((' + ','.join(map(str, indices)) + '))')
        plist = add('IFCCARTESIANPOINTLIST2D', '(' + ','.join(pts) + ')')
        curves.append(add('IFCINDEXEDPOLYCURVE', plist, '(' + ','.join(segments) + ')', '.F.'))
    section = section or (add('IFCARBITRARYCLOSEDPROFILEDEF', '.AREA.', _s(profile['name'] or 'Drawn profile'), curves[0])
               if len(curves) == 1 else add('IFCARBITRARYPROFILEDEFWITHVOIDS', '.AREA.',
                    _s(profile['name'] or 'Drawn profile'), curves[0], '(' + ','.join(curves[1:]) + ')'))
    def placement(point, normal, reference=None):
        pos = add('IFCCARTESIANPOINT', coords(rot @ point + trans))
        return add('IFCAXIS2PLACEMENT3D', pos, add('IFCDIRECTION', coords(rot @ normal / scale)),
                   '$' if reference is None else add('IFCDIRECTION', coords(rot @ reference / scale)))

    def joint(previous, following):
        normal = previous + following
        return normal / np.linalg.norm(normal)

    # A square-ended extrusion must extend as far as the outermost point
    # of each oblique trim. The bounding-box diagonal covers analytic fillets.
    radius = max(np.linalg.norm(np.array([u, v]) - offset)
                 for u in (0, size[0]) for v in (0, size[1])) / 1000
    up = add('IFCDIRECTION', '(0.,0.,1.)')
    items = []
    clipped = False
    butt_planes = group.ext['built.live'].get('butt_planes', [])
    for i, (axis, depth) in enumerate(zip(axes, depths)):
        start_plane = joint(axes[i-1], axis) if i or closed else axis
        end_plane = joint(axis, axes[(i+1) % len(axes)]) if i+1 < len(axes) or closed else axis
        if i:
            # The generator projects its frame onto the joint along the
            # incoming span, then onto the new section along the outgoing span.
            x -= axes[i-1] * (x @ start_plane) / (axes[i-1] @ start_plane)
            x -= axis * (x @ axis)
            x /= np.linalg.norm(x)
        start, end = points[i], points[i+1]
        if butt_planes:
            (first, start_plane), (last, end_plane) = np.asarray(butt_planes[i])
            start = start + axis * ((first - start) @ start_plane) / (axis @ start_plane)
            end = end + axis * ((last - end) @ end_plane) / (axis @ end_plane)
            depth = (end - start) @ axis
        def margin(normal):
            cosine = min(1., abs(axis @ normal))
            return radius * np.sqrt(max(0., 1 - cosine**2)) / cosine
        before, after = margin(start_plane), margin(end_plane)
        pos = placement(start - before * axis, axis, x)
        solid = add('IFCEXTRUDEDAREASOLID', section, pos, up, _f((depth + before + after) * scale))
        for distance, point, outward in ((before, start, -start_plane),
                                          (after, end, end_plane)):
            if distance > 1e-12:
                plane = add('IFCPLANE', placement(point, outward))
                cutter = add('IFCHALFSPACESOLID', plane, '.F.')
                solid = add('IFCBOOLEANCLIPPINGRESULT', '.DIFFERENCE.', solid, cutter)
                clipped = True
        items.append(solid)
    w.last_shape_item = int(items[0][1:])
    representation = "'Clipping'" if clipped else "'SweptSolid'"
    rep = w.add('IFCSHAPEREPRESENTATION', f'#{context_id}', "'Body'", representation,
                '(' + ','.join(items) + ')')
    vertices = [add('IFCCARTESIANPOINT', coords(rot @ p + trans)) for p in points]
    return rep, add('IFCPOLYLINE', '(' + ','.join(vertices) + ')')

def live_cut_body(w, group, base, target, tools, cuts, source, ctx):
    """Compose single-span analytic members and planar solids as IFC4 CSG."""
    from built_core.geometry_arrays import merged, world_arrays
    from core.bim import _compose

    body_ops = {'subtract': 'DIFFERENCE', 'trim': 'DIFFERENCE',
                'intersect': 'INTERSECTION', 'union': 'UNION'}
    frame = _compose(base, group.xform)
    if not tools or target['state'] != 'ok':
        return None, 'Cut references could not be evaluated'
    if any(cuts[uid].get('clearance', 0) for uid, _, _ in tools):
        return None, 'Rounded clearance has no exact IFC offset solid'
    if any(any(tool.mesh._store.exact_counts()) for _, tool, _ in tools):
        return None, 'Curved cutter needs an analytic IFC operand'
    line = None
    if target['kind'] == 'sweep':
        if len(group.ext['built.live'].get('path', [])) != 2:
            return None, 'Cut crosses a member with multiple extrusion spans'
        original = sweep_body(w, group, base, target, ctx, uncut=True)
        if original is None:
            return None, 'The member placement cannot represent an exact extrusion'
        _, line = original
    else:
        if source is None or any(source.mesh._store.exact_counts()):
            return None, 'Cut source has no exact planar IFC operand'
        array_body(w, merged(world_arrays(source, frame)), ctx, 'brep')
    result = w.last_shape_item
    for uid, tool, _ in tools:
        array_body(w, merged(world_arrays(tool, frame)), ctx, 'brep')
        op = body_ops[cuts[uid].get('operation', 'subtract')]
        result = w.add('IFCBOOLEANRESULT', f'.{op}.', f'#{result}', f'#{w.last_shape_item}')
    w.last_shape_item = result
    rep = w.add('IFCSHAPEREPRESENTATION', f'#{ctx}', "'Body'", "'CSG'", f'(#{result})')
    return (rep, line), None

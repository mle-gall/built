"""Pack resident GPU arrays without creating faces for deferred geometry.

The ABI is little endian: 28-byte vertices, 32-byte prototype bounds,
80-byte instances and 20-byte indexed indirect commands. Alpha zero marks
unpainted faces so container tint never overwrites explicit face colour.
Alpha 1..255 is rounded effective opacity (clamped to at least 1).
Both-side paint lives in the existing material corner records, never alpha.
Instance info[2]: bit 0 suppresses ordinary drawing; bit 3 is billboard;
bit 4 marks an object hidden by Hide (including descendants). Reveal bit 4
only when bit 5 is clear: bit 5 means tag/navigator/edit filtering also hides
it. Hidden commands use the same vertex/index buffers and COMMAND dtype.
Materials use 16-byte prototype headers followed by 32-byte corner records
only for prototypes with textures or backs that differ from their fronts.
"""
import ingetrazo_core
from types import SimpleNamespace
from typing import NamedTuple
import numpy as np
from core import _native
from core.geometry_arrays import arrays_of
from core.purge import iter_groups
from core.lazy_mesh import plan_of
from views.array_bake import _attrs_table




from ingetrazo_core import (COMMAND, INSTANCE, OBJECT_HIDDEN, VERTEX, VISIBILITY_FILTERED, PackedScene,
                            assemble_scene)


def bake_mesh(mesh):
    """Reuse the array bake's kernel outputs, including hole triangulation."""
    plan = plan_of(mesh)
    if plan is not None:
        arrays = plan
        src = mesh.__dict__['_deferred_source']
        table = _attrs_table(src)
        attr = np.asarray(src['face_attr'], dtype=np.int64)
        pos = plan.pos
    else:
        arrays = arrays_of(mesh)
        pos = arrays.pos
        if type(mesh).__name__ == 'StoreMesh':
            from ingetrazo_core import store_views
            table, attr = store_views.attr_columns(mesh)
            attr = np.asarray(attr, np.int64)
        else:
            table = [f.attrs for f in mesh.faces]
            attr = np.arange(len(table))
    # Gather once through the attribute table; masking every face for each
    # distinct colour is quadratic on large, per-face-painted prototypes.
    palette = np.full((len(table)+1, 4), 255, np.uint8)
    palette[:, :3] = 210
    palette[:, 3] = 0
    hidden_table = np.zeros(len(table)+1, bool)
    for k, own in enumerate(table):
        hidden_table[k+1] = bool(own.get('hidden'))
        if own.get('color') is not None:
            palette[k+1, :3] = np.clip(np.array(own['color'][:3])*255, 0, 255)
            palette[k+1, 3] = 255
    colors = palette[attr+1]
    hidden = hidden_table[attr+1]
    native = _native.available()
    return native.bake_vertices(
        np.asarray(pos, np.float32), arrays.corners, arrays.ring_sizes,
        arrays.ring_counts, colors, hidden, _native.qt_hypot()).view(VERTEX)



def _alpha(attrs):
    return max(1, min(255, round(float(attrs.get('opacity', 1.0))*255)))


def _baked_paint(paint):
    # Opaque plain container RGB remains an instance tint (shared geometry).
    # All nontrivial resolution must retain the entire GL wrapper, especially
    # base/fx_self; dropping those used to lose inherited tag fade/opacity.
    if not paint or paint.get('front_only'):
        return None
    if any(k in paint for k in ('texture', 'pbr', 'by_layer', 'layer_fx', 'opacity')):
        if (paint.get('base') or {}).get('front_only'):
            return dict(paint, base=None)
        return paint
    return None


def _source_arrays(mesh, paint=None, texture_ids=None, face_overrides=None, billboard=False, origins=None, previous_origin=(0., 0., 0.), focus=None, matrix=None):
    """Use source topology directly; the resident renderer needs no welded Plan."""
    if face_overrides:
        # Resolve lazy faces before reading topology; never mutate document attrs.
        list(mesh.faces)
    src = mesh.__dict__.get('_deferred_source')
    if src is None and type(mesh).__name__ == 'StoreMesh':
        # A store-backed mesh (core/store_mesh.py): its welded arrays,
        # vertex indices already shared by corners and edges.
        from ingetrazo_core import store_views
        a = mesh.__dict__['_store'].arrays()
        pos, corners = a['pos'], a['corners']
        sizes, counts = a['ring_sizes'], a['ring_counts']
        edges = a['edges']
        flags = a['flags'] & 7
        table, attr = store_views.attr_columns(mesh, face_overrides)
        attr = np.asarray(attr, np.int64)
    elif src is not None:
        pos, corners, sizes, counts = (src[k] for k in ('pos', 'rings', 'sizes', 'counts'))
        edges = src['edges']
        flags = src['flags']
        if flags is None:
            flags = np.zeros(len(edges), np.uint8)
        table = _attrs_table(src)
        attr = np.asarray(src['face_attr'], np.int64)
    else:
        arrays = arrays_of(mesh)
        # arrays_of stores face vertices only. Append legacy edge endpoints and
        # unused vertices so edge-only prototypes retain their original bounds.
        extra = np.asarray([v.position.toTuple() for v in mesh.vertices], np.float32).reshape(-1, 3)
        lookup = {v.key: i+len(arrays.pos) for i, v in enumerate(mesh.vertices)}
        pos = np.concatenate((arrays.pos, extra))
        corners, sizes, counts = arrays.corners, arrays.ring_sizes, arrays.ring_counts
        edges = np.asarray([(lookup[e.v0.key], lookup[e.v1.key]) for e in mesh.edges], np.int64).reshape(-1, 2)
        # Face corners and edges must share indices for boundary deduplication.
        positions = {tuple(p): i for i, p in enumerate(pos.tolist())}
        remap = np.fromiter((positions[tuple(p)] for p in arrays.pos.tolist()),
                            dtype=np.int64, count=len(arrays.pos))
        corners = remap[corners]
        flags = np.asarray([int(e.soft) | (int(e.hidden) << 1)
                            | (int(e.soft and e.flat) << 2)
                            for e in mesh.edges], np.uint8)
        table = [(face_overrides or {}).get(f.key, f.attrs) for f in mesh.faces]
        attr = np.arange(len(table))
    origin = (0., 0., 0.)
    if origins is not None:
        from core.coordinate_frame import ZERO, choose_origin
        frame = mesh._store.frame_origin if type(mesh).__name__ == 'StoreMesh' else ZERO
        box = (np.array([pos.min(axis=0), pos.max(axis=0)], dtype=float)
               if len(pos) else np.zeros((2, 3)))
        from core.coordinate_frame import REBASE_DISTANCE
        wide = np.max(box[1] - box[0]) > 2 * REBASE_DISTANCE
        center = box.mean(axis=0) + frame
        if wide and focus is not None:
            local = np.asarray(focus)
            if matrix is not None:
                # Singular placements still draw their projected subspace.
                local = (local - matrix[3, :3]) @ np.linalg.pinv(matrix[:3, :3])
            center = np.clip(local, box[0] + frame, box[1] + frame)
        origin = choose_origin(center, previous_origin)
        origins.append((origin, box + np.subtract(frame, origin), wide))
        if tuple(frame) != origin:
            pos = np.asarray(pos, float) + np.subtract(frame, origin)
    # IGZ source streams can repeat a coordinate for each face/edge. Soft fan
    # adjacency requires common vertex IDs; weld only prototypes that need it.
    if np.any(flags & 1):
        native = _native.native
        if native is not None and hasattr(native, 'pack_weld'):
            remap = native.pack_weld(np.asarray(pos, np.float32))
        else:
            _, first, inverse = np.unique(np.asarray(pos, np.float32), axis=0,
                                          return_index=True, return_inverse=True)
            remap = first[inverse]
        corners = remap[corners]
        edges = remap[edges]
    from core.materials import effective_attrs
    table = [effective_attrs(own, paint) or {} for own in [{}] + table]
    palette = np.full((len(table), 4), 210, np.uint8)
    palette[:, 3] = 0
    hidden = np.array([bool(own.get("hidden")) for own in table])
    painted = [(k, own['color'][:3]) for k, own in enumerate(table)
               if own.get('color') is not None]
    if painted:
        ids, rgb = zip(*painted)
        ids = np.asarray(ids, np.int64)
        palette[ids, :3] = np.clip(np.asarray(rgb)*255, 0, 255)
        palette[ids, 3] = [_alpha(table[k]) for k in ids]
    # Texture-only and opacity-only faces also need a nonzero opacity byte.
    for k, own in enumerate(table):
        if 'pbr' in own and own.get('color') is None:
            palette[k, :3] = 255
        if own.get('texture') or 'opacity' in own or 'pbr' in own:
            palette[k, 3] = _alpha(own)
    maps, paints = _material_columns(pos, corners, sizes, counts, table, attr+1,
                                     texture_ids if texture_ids is not None else {}, origin)
    if billboard is True:
        from views.wgpu_billboards import fitted_maps
        maps = fitted_maps(pos, corners, sizes, counts, maps.copy())
        # GL's cutout uses the same image on either side.
        paints[:, 2] = paints[:, 1]
    return (np.asarray(pos, np.float32), np.asarray(corners, np.int64),
            np.asarray(sizes, np.int64), np.asarray(counts, np.int64),
            np.asarray(edges, np.int64), np.asarray(flags, np.uint8),
            palette, hidden[attr+1], attr+1, maps, paints)


def _placement_state(scene):
    """Memoise layer/material work while preserving edit and navigator filters."""
    from core.group import effective_material
    from core.materials import material_sig
    from views.document_interaction import _paint_of
    vp = SimpleNamespace(scene=scene)
    visibility, paints = {}, {}

    def state(group):
        if group is None:
            visible = True
        elif getattr(scene, '_navigator_through', None) and scene.navigator_key(group) in scene._navigator_through:
            # Holds an isolated part: drawn through (its own geometry is
            # filtered in _placement_records), so its parts are not hidden.
            visible = not (scene.entity_hidden(group) or scene.similar_hidden(group)) \
                and scene._layer_state(group)[0]
        elif scene._edit_stack or getattr(scene, '_navigator_allowed', None) is not None:
            visible = scene.entity_visible(group)
        else:
            key = (group.layer, bool(group.hidden))
            if key not in visibility:
                visibility[key] = scene.entity_visible(group)
            visible = visibility[key]
        material = effective_material(group)
        key = (ingetrazo_core.tag_styles.object_layer(group), material_sig(material))
        if key not in paints:
            paint = _paint_of(vp, group) or {}
            # A style-generated Front tint is not material paint. Keep that
            # distinction resident so changing style needs no geometry rebuild.
            from core.materials import effective_attrs
            resolved = effective_attrs({}, paint) or {}
            from core.style import DEFAULT_FACE_COLOR
            rgb = resolved.get('color') or DEFAULT_FACE_COLOR
            base = (paint or {}).get('base', paint) or {}
            unpainted = not resolved or (bool(base.get('front_only')) and not paint.get('fx_self'))
            alpha = 0 if unpainted else _alpha(resolved)
            paints[key] = np.array([*np.clip(np.array(rgb[:3])*255, 0, 255), alpha], np.uint8).view('<u4')[0]
        return visible, paints[key]
    return state


def _doc_worlds(local, has, parent):
    """World matrices of a ``doc_tree`` walk, as ``model_placements``
    composes them (``local @ parent``, f64, None while the chain has no
    matrix): batched by depth, each product NumPy's own 4×4 matmul."""
    n = len(parent)
    local = local.reshape(n, 4, 4)
    world = np.empty((n, 4, 4))
    defined = np.zeros(n, bool)
    depth = np.zeros(n, np.int64)
    par = parent.tolist()
    dep = depth.tolist() if n else []
    for i, p in enumerate(par):
        if p >= 0:
            dep[i] = dep[p] + 1
    depth = np.asarray(dep, np.int64)
    roots = depth == 0
    world[roots] = local[roots]
    defined[roots] = has[roots]
    for d in range(1, int(depth.max()) + 1 if n else 0):
        level = np.flatnonzero(depth == d)
        up = parent[level]
        up_defined = defined[up]
        own = has[level]
        both = level[own & up_defined]
        if len(both):
            world[both] = np.matmul(local[both], world[parent[both]])
        only = level[own & ~up_defined]
        world[only] = local[only]
        inherit = level[~own & up_defined]
        world[inherit] = world[parent[inherit]]
        defined[level] = own | up_defined
    return world, defined


def _placement_records(scene, roots, vp, state):
    """``(group, world, paint group, flags, columns)`` per placement under
    ``roots``, parents first. ``paint group`` is what the paint resolves
    from (``_paint_of``, ``state``): one stand-in per distinct tag and paint
    on the native walk, the editor's proxy otherwise; ``columns`` is
    ``(mesh, billboard, uid)`` from the native walk, or None."""
    if (not scene._edit_stack and getattr(scene, '_navigator_allowed', None) is None
            and not getattr(scene, '_navigator_through', None) and scene.edit_group is None):
        yield from _native_records(scene, roots)
        return
    for g, world, proxy, flags in _proxy_records(scene, roots, vp, state):
        yield g, world, proxy, flags, None


def _native_records(scene, roots):
    """The native walk (``doc_render_rows``): what each nested placement
    inherits (tag, paint, Hide, the tag's visibility) resolved in Rust, the
    paint resolved here once per distinct tag and paint, not per placement
    (architecture review 12)."""
    from core import doc_model
    from core.group import PlacementProxy
    from core.layers import DEFAULT_LAYER, LINK_UNTAGGED_SUFFIX
    invisible, blocked = set(), set()
    for ly in scene.layers:
        if not ingetrazo_core.tag_edits.effective_visible(scene, ly):
            invisible.add(ly.key)
            blocked.add(ly.key)
        elif ly.locked:
            blocked.add(ly.key)
    cols = doc_model.NATIVE.doc_render_rows(roots, invisible, blocked, LINK_UNTAGGED_SUFFIX, DEFAULT_LAYER)
    parent = cols['parent']
    world, defined = _doc_worlds(cols['local'], cols['has'], parent)
    names, materials = cols['names'], cols['materials']
    reps = {}

    def rep(layer, material):
        key = (layer, material)
        out = reps.get(key)
        if out is None:
            out = reps[key] = PlacementProxy(None, name="Paint", uid="paint")
            out.layer = names[layer] if layer >= 0 else None
            out.material = materials[material] if material >= 0 else None
            out.owner = None
        return out
    handles, flags = cols['handles'], cols['flags'].tolist()
    defined = defined.tolist()
    layers, mats = cols['paint_layer'].tolist(), cols['material'].tolist()
    for i, (g, mesh, billboard, uid) in enumerate(zip(handles, cols['mesh'], cols['billboard'], cols['uid'])):
        yield (g, (world[i] if defined[i] else None), rep(layers[i], mats[i]), flags[i],
               (mesh if mesh is not None else g.mesh, billboard, uid if uid is not None else g.uid))


def _proxy_records(scene, roots, vp, state):
    """Read the tree from the Rust
    document in one call (``doc_tree``): handles, parents, f64 matrices and
    Hide flags as arrays, no attribute read per placement for them."""
    from views.document_interaction import DocumentInteraction
    from core import doc_model
    handles, parent, local, has, own_hidden = doc_model.NATIVE.doc_tree(roots)
    world, defined = _doc_worlds(local, has, parent)
    par = parent.tolist()
    proxies = DocumentInteraction._bind_placement_rows(vp, handles, par, [])
    # This adapter is local to one walk. Release each temporary editor proxy
    # with its row instead of tearing down the whole table under the GIL.
    vp._placement_proxies.clear()
    allowed = getattr(scene, '_navigator_allowed', None)
    own_hidden = own_hidden.tolist()
    defined = defined.tolist()
    n = len(par)
    hidden_of = [False] * n
    object_hidden_of = [False] * n
    filtered_of = [False] * n
    layer_state = scene._layer_state
    similar_hidden = scene.similar_hidden
    for i in range(n):
        g, proxy, p = handles[i], proxies[i], par[i]
        proxies[i] = None
        hidden = not state(proxy)[0]
        obj = own_hidden[i]
        nav = allowed is not None and scene.navigator_key(g) not in allowed
        filtered = (not layer_state(proxy)[0] or similar_hidden(proxy)
                    or (nav and scene.navigator_key(g) not in (getattr(scene, '_navigator_through', None) or ())))
        if p >= 0:
            hidden = hidden or hidden_of[p]
            obj = obj or object_hidden_of[p]
            filtered = filtered or filtered_of[p]
        hidden_of[i], object_hidden_of[i], filtered_of[i] = hidden, obj, filtered
        flags = (int(hidden) | (OBJECT_HIDDEN if obj else 0)
                 | (VISIBILITY_FILTERED if filtered or nav else 0))
        yield g, (world[i] if defined[i] else None), proxy, flags


def open_focus(scene):
    home = scene.camera_home
    if isinstance(home, dict) and home.get("target") is not None:
        return home["target"]
    lo, hi = scene.bounds()
    return tuple((a + b) / 2 for a, b in zip(lo.toTuple(), hi.toTuple())) if lo is not None else (0., 0., 0.)


def pack_scene(scene, view_tint=None, *, face_overrides=None, texture_images=True,
               baked_paints=None, progress=None, previous_origin=(0., 0., 0.), previous_pack=None, focus=None, roots=None):
    """``baked_paints`` (id(group) -> paint) bakes those groups with that
    paint instead of the one they resolve to: the incremental bake of a
    changed prototype variant (views.wgpu_updates). ``progress(fraction,
    text)`` is called as the placements are read (the native bake that
    follows reports at 0.9)."""
    state = _placement_state(scene)
    from views.document_interaction import _paint_of, DocumentInteraction
    vp = SimpleNamespace(scene=scene)
    texture_ids = {}
    meshes, lookup, rows, uids, paints = [], {}, [], [], []
    billboard_codes = set()
    groups = []
    prototype_paints = {}
    coordinates = []
    wide_variants = set()
    old_anchors = {}
    if previous_pack is not None:
        for (mesh_id, _, _), code in (previous_pack.prototype_lookup or {}).items():
            if 0 <= code < len(previous_pack.prototype_origins):
                old_anchors[mesh_id] = previous_pack.prototype_origins[code]
    from itertools import chain
    # Off-document clipboard roots share the model bake without changing the
    # document tree or walking its unrelated resident placements.
    placements = (_native_records(scene, roots) if roots is not None else
                  chain([(None, None, None, 0, None)], _placement_records(scene, scene.groups, vp, state)))
    total = 0
    if progress is not None:
        total = max(1, sum(1 for _ in iter_groups(scene.groups)))
        progress(0.0, "Packing geometry for the GPU…")
    # The paint each paint group resolves to, once per paint group: the
    # native walk shares one between every placement of a tag and paint.
    resolved = {}
    mesh_codes = {}
    for index, (group, matrix, paint_group, flags, columns) in enumerate(placements):
        if progress is not None and index % 64 == 63:
            progress(0.9 * index / total, "Packing geometry for the GPU…")
        if columns is not None:
            mesh, billboard, uid = columns
        else:
            mesh = scene.loose_mesh if group is None else group.mesh
            billboard = getattr(group, "billboard", False)
            uid = getattr(group, 'uid', 'loose')
        got = resolved.get(id(paint_group))
        if got is None:
            paint_attrs = _paint_of(vp, paint_group)
            if paint_attrs and paint_attrs.get('front_only'):
                paint_attrs = None  # style tint stays dynamic
            got = resolved[id(paint_group)] = (_baked_paint(paint_attrs), state(paint_group)[1])
        # Texture mapping, opacity and tag effects create paint variants.
        # Opaque plain container RGB stays in the instance packed RGBA word.
        if baked_paints is not None and id(group) in baked_paints:
            baked_paint = baked_paints[id(group)]
        else:
            baked_paint = got[0]
        key = (id(mesh), repr(baked_paint), billboard)
        variant = key
        if variant in wide_variants:
            key = (id(mesh), (key[1], id(group) if group else None), billboard)
        if key not in lookup:
            arrays = _source_arrays(mesh, baked_paint, texture_ids, face_overrides, billboard,
                                    coordinates, old_anchors.get(id(mesh), (0., 0., 0.)), focus, matrix)
            if coordinates[-1][2] and focus is not None:
                wide_variants.add(variant)
                key = (id(mesh), (variant[1], id(group) if group else None), billboard)
            lookup[key] = len(meshes) if len(arrays[0]) else -1
            if len(arrays[0]):
                prototype_paints[len(meshes)] = baked_paint
                meshes.append(arrays)
            else:
                coordinates.pop()
        code = lookup[key]
        mesh_codes.setdefault(id(mesh), (mesh, code))
        if code < 0:
            continue
        if billboard:
            billboard_codes.add(code)
        world = np.eye(4) if matrix is None else matrix
        rows.append((code, world, flags | (8 if billboard else 0)))
        uids.append(uid)
        groups.append(group)
        paints.append(got[1])
    occurrence_keys = {}
    repeated = scene.repeated_uids() if roots is None else ()
    if roots is None and (repeated or any(u.startswith('@[') for u in uids)):
        from core.link_structure import row_id
        for i, group in enumerate(groups):
            if group is not None and (group.uid in repeated or group.uid.startswith('@[')):
                uids[i] = occurrence_keys[group] = row_id(scene.occurrence_path(group))
    prototype_count = len(meshes)
    if progress is not None:
        progress(0.9, "Packing geometry for the GPU…")
    matrices = np.empty((len(rows), 4, 4), dtype=np.float64)
    for begin_row in range(0, len(rows), 2048):
        block = rows[begin_row:begin_row + 2048]
        matrices[begin_row:begin_row + len(block)] = [r[1] for r in block]
    tints = [(view_tint or {}).get(uid) for uid in uids] if view_tint else [None] * len(uids)
    # The bake, the command tables, the rows in the view frame and the
    # frame origin: rust/ingetrazo_core/src/resident/assemble.rs.
    built = assemble_scene(meshes, [r[0] for r in rows], matrices, [r[2] for r in rows], paints, tints,
                           coordinates, sorted(billboard_codes), bool(texture_ids or billboard_codes),
                           previous_origin, bool(wide_variants), _native.qt_hypot())
    # Drop the per-prototype copies before decoding textures. Deferred
    # source arrays remain owned by meshes.
    del meshes, matrices
    textures = _texture_array(texture_ids) if texture_images else None
    return PackedScene(
        built["vertices"], built["indices"], built["bounds"], built["commands"], built["edge_commands"],
        built["instances"], uids, built["world_bounds"], None, groups, built["materials"], textures,
        tuple(texture_ids), built["hidden_commands"], built["hidden_edge_commands"], built["profile_exclusions"],
        (bool(getattr(scene, 'show_hidden_geometry', False)), bool(getattr(scene, 'show_hidden_objects', False))),
        lookup, prototype_paints, origin=built["origin"], prototype_origins=built["prototype_origins"],
        model_translations=built["model_translations"], bake_focus=tuple(focus) if wide_variants else None,
        occurrence_keys=occurrence_keys, mesh_codes=mesh_codes)


def _material_columns(pos, corners, sizes, counts, table, attributes, texture_ids, origin=(0., 0., 0.)):
    """Use the GL mapping and side resolution, before triangulation splits corners."""
    from ingetrazo_core import Vec3 as QVector3D
    from core.materials import back_is_default
    from core.texture import face_uv_axes
    maps = np.broadcast_to(np.zeros(16, np.float32), (len(counts), 16))
    # The common unpainted prototype has no back or texture rows to resolve.
    # Avoid per-face palette gathers and temporary masks for this case.
    if not any(table):
        return maps, np.zeros((len(counts), 3), np.uint32)
    palette = np.zeros((len(table), 3), np.uint32)
    native_pack = getattr(_native.native, "pack_materials", None)
    colors = []
    textures = []
    for k, own in enumerate(table):
        back = own.get('back')
        if not isinstance(back, dict):
            back = {} if back_is_default(own) else own
        color = back.get('color')
        if color is None and 'pbr' in back:
            color = (1., 1., 1.)
        if color is None and (back.get('texture') or 'opacity' in back):
            color = (210/255,)*3
        if native_pack is not None:
            colors.append(None if color is None else (*color[:3], _alpha(back)))
        elif color is not None:
            palette[k, 0] = np.array([*np.clip(np.array(color[:3])*255,0,255),_alpha(back)], np.uint8).view('<u4')[0]
        sides = []
        for side, attrs in enumerate((own, back)):
            tex = attrs.get('texture')
            path = (tex or {}).get('path')
            if attrs.get('pbr') is not None:
                path = _pbr_entry(path, attrs['pbr'])
                tex = tex or {'sw': 1., 'sh': 1.}
            sides.append(tex)
            if path:
                if path not in texture_ids:
                    texture_ids[path] = len(texture_ids)+1
                palette[k, side+1] = texture_ids[path]
        textures.append(sides)
    if native_pack is None:
        paints = palette[attributes]
    else:
        paints = native_pack(colors, palette[:, 1:].tolist(), np.asarray(attributes, np.int64))
    textured = np.flatnonzero(np.any(paints[:,1:] != 0, axis=1))
    if len(textured):
        maps = np.zeros((len(counts), 16), np.float32)
        starts = np.concatenate(([0], np.cumsum(sizes)))
        face_ring = np.concatenate(([0], np.cumsum(counts)))
        points = np.asarray(pos)[corners]
        following = np.roll(points, -1, axis=0)
        following[starts[1:]-1] = points[starts[:-1]]
        normals = np.add.reduceat(np.cross(points, following), starts[:-1])[face_ring[:-1]]
        lengths = np.linalg.norm(normals, axis=1)
        normals /= np.maximum(lengths[:,None], 1e-30)
        pairs = np.column_stack((attributes[textured], normals[textured]))
        unique, inverse = np.unique(pairs, axis=0, return_inverse=True)
        resolved = np.zeros((len(unique),16), np.float32)
        for j, row in enumerate(unique):
            for side, tex in enumerate(textures[int(row[0])]):
                if tex:
                    gu, cu, gv, cv = face_uv_axes(tex, QVector3D(*row[1:]))
                    if any(origin):
                        # Tiled UV phase at prototype zero; retaining integer
                        # tile repetitions here would lose the small local UVs.
                        cu = (cu + np.dot(gu.toTuple(), origin)) % 1.
                        cv = (cv + np.dot(gv.toTuple(), origin)) % 1.
                    resolved[j,side*8:side*8+8] = [*gu.toTuple(),cu,*gv.toTuple(),cv]
        maps[textured] = resolved[inverse]
    return maps, paints


class _PbrEntry(NamedTuple):
    images: tuple  # base, normal, ORM, AO, roughness, metallic
    flip_y: bool
    factors: tuple  # metallic, roughness, normal scale, AO strength


def _pbr_entry(base, recipe):
    # Logical material IDs survive incremental prototype remapping. Image layers
    # are assigned separately so a shared normal or ORM consumes one layer.
    maps = tuple((recipe.get(key) or {}).get("path") for key in
                 ("normal_map", "orm_map", "ao_map", "roughness_map", "metallic_map"))
    factors = tuple(float(recipe.get(key, default)) for key, default in
                    (("metallic", 0), ("roughness", 1), ("normal_scale", 1), ("ao_strength", 1)))
    return _PbrEntry((base, *maps), bool((recipe.get("normal_map") or {}).get("flip_y")), factors)


def _texture_array(paths, *, device_limit=8192, memory_budget=256*1024*1024):
    """Bottom-up RGBA8 array, complete mips, stable 1-based texture IDs.

    Common layer dimensions retain source resolution up to device_limit (8192
    conservative default; caller may pass the adapter's actual 2D limit).
    Halve both dimensions until *all* layers including white layer zero and
    every mip fit the 256 MiB GPU budget. CPU source decoding is not budgeted.
    Array textures require equal layer sizes; changing that needs a new ABI.
    """
    from ingetrazo_render import material_texture_array
    return material_texture_array(paths, device_limit, memory_budget)

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
from dataclasses import dataclass, field
from types import SimpleNamespace
from typing import NamedTuple
import numpy as np
from core import _native
from core.geometry_arrays import arrays_of
from core.group import iter_placements
from core.lazy_mesh import plan_of
from views.array_bake import _attrs_table

VERTEX = np.dtype([('pos', '<f4', 3), ('normal', '<f4', 3), ('color', 'u1', 4)])
INSTANCE = np.dtype([('world', '<f4', (4, 4)), ('info', '<u4', 4)])
OBJECT_HIDDEN = 1 << 4
VISIBILITY_FILTERED = 1 << 5

COMMAND = np.dtype([('count', '<u4'), ('instances', '<u4'), ('first', '<u4'),
                    ('base', '<i4'), ('start', '<u4')])


@dataclass
class PackedScene:
    vertices: np.ndarray
    indices: np.ndarray
    bounds: np.ndarray
    commands: np.ndarray
    edge_commands: np.ndarray
    instances: np.ndarray
    uids: list
    world_bounds: np.ndarray
    timings: dict | None = None
    groups: list | None = None
    materials: np.ndarray | None = None
    textures: tuple | None = None
    texture_paths: tuple = ()
    hidden_commands: np.ndarray | None = None
    hidden_edge_commands: np.ndarray | None = None

    profile_exclusions: tuple = ()
    reveal_hidden: tuple = (False, False)

    prototype_lookup: dict | None = None
    # Code -> the baked paint its vertices carry (None: plain). An edit
    # rebakes each variant of a changed mesh with its own paint.
    prototype_paints: dict | None = None

    # Sparse replacements since the last whole-table handoff. Rebuilding
    # this stream on each edit copies every unrelated material record.
    material_updates: dict = field(default_factory=dict)

    origin: tuple = (0., 0., 0.)
    prototype_origins: tuple = ()
    model_translations: np.ndarray | None = None
    bake_focus: tuple | None = None

    def rebase(self, origin):
        """Project the original placement translations into this view frame."""
        if self.origin == origin:
            return False
        self.instances["world"][:, 3, :3] = self.model_translations - origin
        self.world_bounds = self.world_bounds + np.subtract(self.origin, origin)
        self.origin = origin
        return True

    def material_stream(self):
        if not self.material_updates:
            return self.materials
        count = len(self.commands)
        headers = np.zeros((count, 4), '<u4')
        chunks = [headers.view(np.uint8).ravel()]
        at = count
        old = self.materials.view('<u4').reshape(-1, 4)
        for code in range(count):
            if code in self.material_updates:
                flags, records = self.material_updates[code]
            else:
                _, first, length, flags = map(int, old[code])
                records = self.materials[first*16:first*16 + length*32]
            headers[code, 3] = flags
            if len(records):
                headers[code, :3] = self.commands['base'][code], at, len(records) // 32
                chunks.append(records)
                at += len(records) // 16
        self.materials = np.concatenate(chunks)
        self.material_updates = {}
        return self.materials

    def upload(self, renderer):
        renderer.set_scene(tuple(a.view(np.uint8).reshape(-1) for a in
                                 (self.vertices, self.indices, self.bounds, np.concatenate((self.commands, self.edge_commands)))),
                           self.instances.view(np.uint8).reshape(-1))
        renderer.set_materials(self.material_stream())
        renderer.set_textures(*self.textures)
        if hasattr(renderer, "set_profile_exclusions"):
            renderer.set_profile_exclusions(self.profile_exclusions)
        from views.wgpu_parity import has_transparency, hidden_geometry
        if hasattr(renderer, "set_material_transparency"):
            renderer.set_material_transparency(has_transparency(self))
        hidden_geometry(SimpleNamespace(renderer=renderer, packed=self))


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
            from core.store_mesh import attr_columns
            table, attr = attr_columns(mesh)
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
        from core.store_mesh import attr_columns
        a = mesh.__dict__['_store'].arrays()
        pos, corners = a['pos'], a['corners']
        sizes, counts = a['ring_sizes'], a['ring_counts']
        edges = a['edges']
        flags = a['flags'] & 7
        table, attr = attr_columns(mesh, face_overrides)
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
    from core.layers import object_layer
    from core.materials import material_sig
    from views.viewport import _paint_of
    vp = SimpleNamespace(scene=scene)
    visibility, paints = {}, {}

    def state(group):
        if group is None:
            visible = True
        elif group.uid in (getattr(scene, '_navigator_through', None) or ()):
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
        key = (object_layer(group), material_sig(material))
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


def _placement_records_doc(scene, roots, vp, state):
    """``_placement_records(double=True)`` reading the tree from the Rust
    document in one call (``doc_tree``): handles, parents, f64 matrices and
    Hide flags as arrays, no attribute read per placement for them."""
    from views.viewport import Viewport
    from core import doc_model
    handles, parent, local, has, own_hidden = doc_model.NATIVE.doc_tree(roots)
    world, defined = _doc_worlds(local, has, parent)
    proxies = []
    for root in roots:
        proxies.extend(Viewport._expand_placements(vp, root))
    allowed = getattr(scene, '_navigator_allowed', None)
    par = parent.tolist()
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
        hidden = not state(proxy)[0]
        obj = own_hidden[i]
        filtered = (not layer_state(proxy)[0] or similar_hidden(proxy)
                    or (allowed is not None and g.uid not in allowed))
        if p >= 0:
            hidden = hidden or hidden_of[p]
            obj = obj or object_hidden_of[p]
            filtered = filtered or filtered_of[p]
        hidden_of[i], object_hidden_of[i], filtered_of[i] = hidden, obj, filtered
        flags = (int(hidden) | (OBJECT_HIDDEN if obj else 0)
                 | (VISIBILITY_FILTERED if filtered else 0))
        yield g, (world[i] if defined[i] else None), proxy, flags


def _placement_records(scene, roots, vp, state, *, double=False):
    from views.viewport import Viewport
    if double:
        from core import doc_model
        if doc_model.ENABLED:
            try:
                rows = _placement_records_doc(scene, roots, vp, state)
                first = next(rows, None)
            except TypeError:
                rows = first = None          # not all document placements
            if rows is not None:
                if first is not None:
                    yield first
                    yield from rows
                return
    # Visibility is inherited down the tree, as the GL proxies do it: a
    # placement inside a hidden group, or inside a group whose tag is off,
    # is hidden whatever its own flags say. Taking only the root's state
    # drew the ride's hidden concrete volumes and context walls (R-look).
    parent_of = {}

    def map_parents(group):
        for child in getattr(group, 'children', None) or ():
            parent_of[id(child)] = group
            map_parents(child)
    hidden_of = {}
    object_hidden_of = {}
    filtered_of = {}

    from core.group import model_placements

    for root in roots:
        map_parents(root)
        # GL proxies carry the top-level paint owner and inherited layer.
        # Keep source groups for editing; use proxies only to resolve paint.
        proxies = Viewport._expand_placements(vp, root)
        placements = model_placements(root) if double else iter_placements(root)
        for (g, matrix), proxy in zip(placements, proxies):
            parent = parent_of.get(id(g))
            inherited = hidden_of.get(id(parent), False) if parent is not None else False
            hidden_of[id(g)] = inherited or not state(proxy)[0]
            object_hidden_of[id(g)] = bool(getattr(g, 'hidden', False)) or object_hidden_of.get(id(parent), False)
            allowed = getattr(scene, '_navigator_allowed', None)
            # Isolated out: its own geometry; its parts too, unless it only
            # holds something isolated (scene._navigator_through).
            nav = allowed is not None and g.uid not in allowed
            filtered_of[id(g)] = (filtered_of.get(id(parent), False)
                or not scene._layer_state(proxy)[0] or scene.similar_hidden(proxy)
                or (nav and g.uid not in (getattr(scene, '_navigator_through', None) or ())))
            flags = (int(hidden_of[id(g)]) | (OBJECT_HIDDEN if object_hidden_of[id(g)] else 0)
                     | (VISIBILITY_FILTERED if filtered_of[id(g)] or nav else 0))
            yield g, matrix, proxy, flags


def _profile_endpoints(mesh):
    """Keep sequence semantics for incremental edits without boxing every float."""
    from array import array
    points = mesh[0][mesh[4][(mesh[5] & 3) != 0]]
    result = array('f')
    result.frombytes(np.asarray(points, np.float32).tobytes())
    return result


def pack_scene(scene, view_tint=None, *, face_overrides=None, texture_images=True,
               baked_paints=None, progress=None, previous_origin=(0., 0., 0.), previous_pack=None, focus=None):
    """``baked_paints`` (id(group) -> paint) bakes those groups with that
    paint instead of the one they resolve to: the incremental bake of a
    changed prototype variant (views.wgpu_updates). ``progress(fraction,
    text)`` is called as the placements are read (the native bake that
    follows reports at 0.9)."""
    from time import perf_counter
    try:
        import resource
    except ImportError:
        resource = None
    begin = perf_counter()
    state = _placement_state(scene)
    from views.viewport import _paint_of, Viewport
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
    placements = chain([(None, None, None, 0)], _placement_records(scene, scene.groups, vp, state, double=True))
    total = 0
    if progress is not None:
        total = max(1, sum(1 for root in scene.groups for _ in iter_placements(root)))
        progress(0.0, "Preparing the view…")
    for index, (group, matrix, paint_group, flags) in enumerate(placements):
        if progress is not None and index % 2048 == 2047:
            progress(0.9 * index / total, "Preparing the view…")
        mesh = scene.loose_mesh if group is None else group.mesh
        paint_attrs = _paint_of(vp, paint_group)
        if paint_attrs and paint_attrs.get('front_only'):
            paint_attrs = None  # style tint stays dynamic
        # Texture mapping, opacity and tag effects create paint variants.
        # Opaque plain container RGB stays in the instance packed RGBA word.
        if baked_paints is not None and id(group) in baked_paints:
            baked_paint = baked_paints[id(group)]
        else:
            baked_paint = _baked_paint(paint_attrs)
        billboard = getattr(group, "billboard", False)
        key = (id(mesh), repr(baked_paint), billboard)
        variant = key
        if variant in wide_variants:
            key = (id(mesh), (key[1], group.uid if group else None), billboard)
        if key not in lookup:
            arrays = _source_arrays(mesh, baked_paint, texture_ids, face_overrides, billboard,
                                    coordinates, old_anchors.get(id(mesh), (0., 0., 0.)), focus, matrix)
            if coordinates[-1][2] and focus is not None:
                wide_variants.add(variant)
                key = (id(mesh), (variant[1], group.uid if group else None), billboard)
            lookup[key] = len(meshes) if len(arrays[0]) else -1
            if len(arrays[0]):
                prototype_paints[len(meshes)] = baked_paint
                meshes.append(arrays)
            else:
                coordinates.pop()
        code = lookup[key]
        if code < 0:
            continue
        if billboard:
            billboard_codes.add(code)
        _, paint = state(paint_group)
        world = np.eye(4) if matrix is None else matrix
        rows.append((code, world, flags | (8 if billboard else 0)))
        uids.append(getattr(group, 'uid', 'loose'))
        groups.append(group)
        paints.append(paint)
    offsets = np.zeros((len(meshes)+1, 5), np.int64)
    for i, arrays in enumerate(meshes):
        offsets[i+1] = [len(a) for a in arrays[:5]]
    np.cumsum(offsets, axis=0, out=offsets)
    empty = (np.empty((0,3), np.float32), np.empty(0, np.int64),
             np.empty(0, np.int64), np.empty(0, np.int64),
             np.empty((0,2), np.int64), np.empty(0, np.uint8),
             np.empty((0,4), np.uint8), np.empty(0, bool), np.empty(0, np.int64))
    palette_offsets = np.cumsum([0]+[len(m[6]) for m in meshes])
    attributes = (np.concatenate([m[8]+offset for m, offset in zip(meshes, palette_offsets)])
                  if meshes else empty[8])
    arrays = [np.concatenate([m[k] for m in meshes]) if meshes else empty[k] for k in range(8)]
    profile_exclusions = tuple((code, _profile_endpoints(m))
                               for code, m in enumerate(meshes) if np.any(m[5] & 3))
    prototype_count = len(meshes)
    maps = (np.concatenate([m[9] for m in meshes]) if texture_ids or billboard_codes
            else np.broadcast_to(np.zeros(16, np.float32), (int(offsets[-1,3]), 16)))
    paints_array = np.concatenate([m[10] for m in meshes]) if meshes else np.empty((0,3), np.uint32)
    # Drop the per-prototype copies BEFORE native output allocation, not just
    # before decoding textures. Deferred source arrays remain owned by meshes.
    del meshes
    if progress is not None:
        progress(0.9, "Preparing the view…")
    handoff = perf_counter()
    raw, indices, bounds, table, materials = _native.available().bake_scene(
        *arrays[:4], arrays[6], arrays[7], arrays[4], arrays[5], offsets, attributes, _native.qt_hypot(),
        maps, paints_array, True)
    del arrays, attributes, maps, paints_array
    textures = _texture_array(texture_ids) if texture_images else None
    baked = perf_counter()
    vertices = raw.view(VERTEX)
    bounds = bounds.reshape(-1, 2, 4)
    table = table.reshape(-1, 5)
    commands = np.zeros(prototype_count, COMMAND)
    edge_commands = np.zeros(prototype_count, COMMAND)
    hidden_commands = np.zeros(prototype_count, COMMAND)
    hidden_edge_commands = np.zeros(prototype_count, COMMAND)
    instances = np.zeros(len(rows), INSTANCE)
    origins = tuple(c[0] for c in coordinates)
    origin = previous_origin
    model_translations = None
    world_bounds = np.zeros((2, 3))
    if rows:
        from core.coordinate_frame import choose_origin
        codes = np.asarray([r[0] for r in rows])
        matrices = np.asarray([r[1] for r in rows])
        linear = matrices[:, :3, :3]
        matrices[:, 3, :3] += np.einsum('ni,nij->nj', np.asarray(origins)[codes], linear)
        boxes = np.array([c[1] for c in coordinates])[codes]
        centers = np.einsum('ni,nij->nj', boxes.mean(axis=1), linear) + matrices[:, 3, :3]
        radii = np.einsum('ni,nij->nj', (boxes[:, 1] - boxes[:, 0]) * .5, abs(linear))
        visible = np.array([not r[2] & 1 for r in rows])
        if visible.any():
            world_bounds = np.stack(((centers-radii)[visible].min(axis=0),
                                     (centers+radii)[visible].max(axis=0)))
            origin = choose_origin(world_bounds.mean(axis=0), previous_origin)
            world_bounds -= origin
        from core.coordinate_frame import REBASE_DISTANCE
        if wide_variants or any(origin) or np.any(np.abs(matrices[:, 3, :3]) > REBASE_DISTANCE) or any(map(any, origins)):
            model_translations = matrices[:, 3, :3].copy()
        matrices[:, 3, :3] -= origin
        instances['world'] = matrices
        instances['info'][:, 0] = [r[0] for r in rows]
        instances['info'][:, 1] = paints
        instances['info'][:, 2] = [r[2] for r in rows]
        for i, uid in enumerate(uids):
            tint = (view_tint or {}).get(uid)
            if tint is not None:
                instances['info'][i, 3] = np.array([*np.clip(np.array(tint[:3])*255, 0, 255), 255], np.uint8).view('<u4')[0]
    counts = np.bincount(instances['info'][:, 0], minlength=prototype_count)
    commands['count'] = table[:, 1]
    edge_commands['count'] = table[:, 2]
    if billboard_codes:
        edge_commands['count'][list(billboard_codes)] = 0
    commands['first'] = commands['base'] = edge_commands['base'] = table[:, 0]
    edge_commands['first'] = table[:, 0]+table[:, 1]
    commands['start'] = edge_commands['start'] = np.cumsum(counts)-counts
    hidden_commands['count'] = table[:, 3]
    hidden_edge_commands['count'] = table[:, 4]
    hidden_commands['base'] = hidden_edge_commands['base'] = table[:, 0]
    hidden_commands['first'] = table[:, 0]+table[:, 1]+table[:, 2]
    hidden_edge_commands['first'] = hidden_commands['first']+table[:, 3]
    hidden_commands['start'] = hidden_edge_commands['start'] = commands['start']
    return PackedScene(vertices, indices, bounds, commands, edge_commands, instances, uids, world_bounds, dict(handoff=handoff-begin, native=baked-handoff, finish=perf_counter()-baked, peak_build_rss_bytes=(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss if resource is not None else None)), groups, materials, textures, tuple(texture_ids), hidden_commands, hidden_edge_commands, profile_exclusions,
                               (bool(getattr(scene, 'show_hidden_geometry', False)), bool(getattr(scene, 'show_hidden_objects', False))), lookup, prototype_paints, origin=origin, prototype_origins=tuple(origins), model_translations=model_translations,
                               bake_focus=tuple(focus) if wide_variants else None)


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
    from PySide6.QtGui import QImage
    from PySide6.QtCore import Qt
    descriptors = None
    if any(isinstance(entry, _PbrEntry) for entry in paths):
        layers = {}
        descriptors = np.zeros((len(paths)+1, 12), '<u4')
        for i, entry in enumerate(paths, 1):
            images = entry.images if isinstance(entry, _PbrEntry) else (entry,)
            for channel, path in enumerate(images):
                if path:
                    descriptors[i, channel] = layers.setdefault(path, len(layers)+1)
            if isinstance(entry, _PbrEntry):
                descriptors[i, 6:8] = (1, entry.flip_y)
                descriptors[i, 8:].view('<f4')[:] = entry.factors
        paths = layers
    images = [QImage(path).mirrored() for path in paths]
    if descriptors is not None:
        # A missing normal must leave the geometric normal intact. Zero is
        # the neutral/unused layer for each of the six image roles.
        available = np.array([0] + [0 if im.isNull() else i
                                    for i, im in enumerate(images, 1)], np.uint32)
        descriptors[:, :6] = available[descriptors[:, :6]]
    width = min(device_limit, max([1] + [im.width() for im in images]))
    height = min(device_limit, max([1] + [im.height() for im in images]))
    if device_limit < 1 or memory_budget < (len(images)+1)*4:
        raise ValueError('texture budget cannot hold one texel per layer')
    def mip_bytes(w, h):
        total = 0
        while True:
            total += w*h*4*(len(images)+1)
            if w == h == 1:
                return total
            w, h = max(1, w//2), max(1, h//2)
    while mip_bytes(width, height) > memory_budget:
        width, height = max(1, width//2), max(1, height//2)
    levels = []
    w, h = width, height
    while True:
        layers = [bytes([255])*w*h*4]  # layer zero: no texture
        for im in images:
            if im.isNull():
                layers.append(bytes([255])*w*h*4)
            else:
                scaled = im.scaled(w, h, Qt.IgnoreAspectRatio, Qt.SmoothTransformation).convertToFormat(QImage.Format_RGBA8888)
                layers.append(bytes(scaled.constBits()))
        levels.append(b''.join(layers))
        if w == h == 1:
            break
        w, h = max(1,w//2), max(1,h//2)
    return (width, height, len(images)+1, levels) + (
        (descriptors.tobytes(),) if descriptors is not None else ())

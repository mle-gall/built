"""Diff resident rows and mesh serials without baking record-only edits.

Removed rows are tombstones: selections and GPU pick IDs never shift. A batch
of mutated prototypes uses one R1b bake; transform previews only write rows.
A prototype is (mesh, baked paint, billboard): a changed mesh rebakes every
variant it has, each with its own paint, hidden faces/edges included, so an
edit in a nested, painted or hidden-geometry model never repacks the scene.
"""
import numpy as np
from core.group import Group, iter_placements
from core.scene import Scene
from views.wgpu_scene import (COMMAND, INSTANCE, _baked_paint, _placement_records,
                              _placement_state, pack_scene)


class InstanceUpdates:
    def __init__(self, scene, packed):
        self.scene = scene
        self.packed = packed
        self.meshes = {}
        # id(mesh) -> {code: (baked paint, billboard)}: every resident
        # prototype a mesh has. One mesh may be drawn plain, under an
        # inherited texture/opacity/tag effect and as a face-me sprite.
        self.variants = {}
        paints = packed.prototype_paints or {}
        for key, code in (packed.prototype_lookup or {}).items():
            if code >= 0:
                self.variants.setdefault(key[0], {})[code] = (paints.get(code), key[2])
        for row, group in enumerate(packed.groups):
            mesh = group.mesh if group is not None else scene.loose_mesh
            self.meshes[id(mesh)] = (mesh, mesh._mut_serial,
                                    int(packed.instances['info'][row, 0]))
        # Empty containers have no GPU row, but still need a serial sentinel.
        all_meshes = [scene.loose_mesh]
        all_meshes.extend(g.mesh for root in scene.groups for g, _ in iter_placements(root))
        for mesh in all_meshes:
            self.meshes.setdefault(id(mesh), (mesh, mesh._mut_serial, -1))
        self.mesh_revisions = {key: mesh.revision for key, (mesh, _, _) in self.meshes.items()}
        self.rows = {uid: i for i, uid in enumerate(packed.uids)}
        self.active = set(self.rows)
        self.changed = []
        self.removed = []
        self.version = scene.content_version
        self.pending = set()
        self.pending_meshes = set()
        self.pending_materials = set()
        self.pending_removed = set()
        self.journal_valid = True
        self.unsubscribe = None
        self.roots = set(scene.groups)
        self.world_boxes = self._boxes(np.arange(len(packed.instances)))

    def _boxes(self, ids):
        rows = self.packed.instances[ids]
        bounds = self.packed.bounds[rows['info'][:, 0], :, :3]
        matrix = rows['world'].transpose(0, 2, 1)
        center = bounds.mean(axis=1)
        extent = (bounds[:, 1]-bounds[:, 0])*.5
        wc = np.einsum('nij,nj->ni', matrix[:, :3, :3], center)+matrix[:, :3, 3]
        we = np.einsum('nij,nj->ni', abs(matrix[:, :3, :3]), extent)
        boxes = np.stack((wc-we, wc+we), axis=1)
        hidden = (rows['info'][:, 2] & 1) != 0
        boxes[hidden, 0] = np.inf
        boxes[hidden, 1] = -np.inf
        return boxes

    def observe(self, history):
        """Use History's existing journal callbacks to avoid scanning known edits."""
        if self.unsubscribe is not None or history is None:
            return
        from weakref import WeakKeyDictionary
        material_targets = WeakKeyDictionary()

        def before(command, phase, ran):
            if self.scene.content_version != self.version:
                self.journal_valid = False

        def after(command, phase, ran):
            from core.history import (MoveGroupCommand, CompoundCommand, SetRecordCommand,
                                      SetRegistryCommand, InsertGroupCommand, DeleteGroupCommand,
                                      SnapshotMutation, MoveVerticesCommand, AddEdgeCommand,
                                      AddFaceCommand, EraseSelectionCommand, HideCommand, AssignLayerCommand)
            def visit(command):
                from core.history import (SetFaceTextureCommand, SetFaceBackCommand,
                                          SetFaceColorCommand, SetFaceOpacityCommand)
                if isinstance(command, (SetFaceTextureCommand, SetFaceBackCommand,
                                        SetFaceColorCommand, SetFaceOpacityCommand)):
                    # Attribute commands deliberately leave geometry serials alone.
                    target = material_targets.get(command)
                    if target is None:
                        faces = set(command._faces)
                        mesh = self.scene.mesh
                        if not faces.issubset(set(mesh.faces)) or id(mesh) not in self.meshes:
                            return False
                        target = material_targets[command] = id(mesh)
                    # Undo may occur after leaving the group's edit context.
                    self.pending_materials.add(target)
                    return True

                if isinstance(command, (HideCommand, AssignLayerCommand)):
                    entities = command._entities
                    if all(isinstance(entity, Group) for entity in entities):
                        # These commands also dirty all GL chunk revisions.
                        # The command journal tells us only placements changed.
                        self.pending.update(entities if self.roots.issuperset(entities) else self.roots)
                        self.mesh_revisions = {key: mesh.revision
                                               for key, (mesh, _, _) in self.meshes.items()}
                        return True
                    return False
                if isinstance(command, CompoundCommand):
                    return all(visit(c) for c in command.commands)
                if isinstance(command, (SnapshotMutation, MoveVerticesCommand,
                                        AddEdgeCommand, AddFaceCommand, EraseSelectionCommand)):
                    mesh = command._target(self.scene) if isinstance(command, SnapshotMutation) else self.scene.mesh
                    old = self.meshes.get(id(mesh))
                    if old is None or old[2] < 0:
                        return False
                    self.pending_meshes.add(id(mesh))
                    return True
                if isinstance(command, (InsertGroupCommand, DeleteGroupCommand)) and command._owner is self.scene.groups:
                    adding = isinstance(command, InsertGroupCommand) != (phase == 'undo')
                    if adding:
                        self.roots.add(command.group)
                        self.pending.add(command.group)
                        self.pending_removed.discard(command.group)
                    else:
                        self.roots.discard(command.group)
                        self.pending.discard(command.group)
                        self.pending_removed.add(command.group)
                    return True
                if isinstance(command, MoveGroupCommand) and command.group in self.roots:
                    self.pending.add(command.group)
                    return True
                return isinstance(command, (SetRecordCommand, SetRegistryCommand))
            self.journal_valid = self.journal_valid and ran and visit(command)
            self.version = self.scene.content_version
        self.unsubscribe = history.add_listener(before=before, after=after)

    def rebake_limit(self) -> int:
        """Most prototypes one edit rebakes in place; past it, a full pack."""
        return max(256, len(self.packed.commands) // 8)

    def diff(self, scene):
        """Return changed resident rows, retaining prototype and placement IDs."""
        if scene is not self.scene:
            return None
        local = self.unsubscribe is not None and self.journal_valid and scene.content_version == self.version
        placements = [] if local else [(None, None, None, 0)]
        state = _placement_state(scene)
        loose = self.meshes.get(id(scene.loose_mesh))
        if local and (loose is None or loose[1] != scene.loose_mesh._mut_serial):
            placements.append((None, None, None, 0))
        from types import SimpleNamespace
        from views.viewport import _paint_of
        vp = SimpleNamespace(scene=scene)
        roots = self.pending if local else scene.groups
        placements.extend(_placement_records(scene, roots, vp, state))
        seen, ids, values, changed = (self.active if local else set()), [], [], {}
        if local:
            changed.update((key, self.meshes[key]) for key in self.pending_meshes
                           if self.meshes[key][0]._mut_serial != self.meshes[key][1])
        changed.update((key, self.meshes[key]) for key in self.pending_materials)
        if not local:
            # Attribute-only edits (paint, hidden faces) leave the geometry
            # serial alone but move the revision: rebake those meshes too.
            changed.update((key, (mesh, serial, code))
                           for key, (mesh, serial, code) in self.meshes.items()
                           if code >= 0 and mesh.revision != self.mesh_revisions.get(key))
        self.pending_materials.clear()
        self.pending_meshes.clear()
        additions = []
        fresh = {}      # code -> mesh: variants that exist from this edit on
        for group, matrix, proxy, flags in placements:
            mesh = group.mesh if group is not None else scene.loose_mesh
            mode = getattr(group, 'billboard', False)
            paint = _baked_paint(_paint_of(vp, proxy))
            key = (id(mesh), repr(paint), mode)
            old = self.meshes.get(id(mesh))
            code = self.packed.prototype_lookup.get(key)
            uid = getattr(group, 'uid', 'loose')
            if (old is not None and old[2] < 0 and old[1] == mesh._mut_serial
                    and (code is None or code < 0)):
                continue    # empty when packed, under any paint: no row
            if code == -1:
                code = None
            if code is None:
                # A new mesh, or a known mesh under a paint or billboard
                # mode it was never drawn with: a prototype of its own.
                code = len(self.packed.bounds)
                if old is None or old[2] < 0:
                    old = (mesh, -1, code)
                    self.meshes[id(mesh)] = old
                self.packed.prototype_lookup[key] = code
                self.variants.setdefault(id(mesh), {})[code] = (paint, mode)
                fresh[code] = mesh
                self.packed.bounds = np.concatenate((self.packed.bounds, np.zeros((1, 2, 4), np.float32)))
                self.packed.commands = np.concatenate((self.packed.commands, np.zeros(1, COMMAND)))
                self.packed.edge_commands = np.concatenate((self.packed.edge_commands, np.zeros(1, COMMAND)))
                self.packed.hidden_commands = np.concatenate((self.packed.hidden_commands, np.zeros(1, COMMAND)))
                self.packed.hidden_edge_commands = np.concatenate((self.packed.hidden_edge_commands, np.zeros(1, COMMAND)))
            if old[1] != mesh._mut_serial:
                changed[id(mesh)] = old
            row = self.rows.get(uid)
            if row is None:
                row = len(self.packed.instances) + len(additions)
                additions.append((uid, group, row))
            if row < len(self.packed.groups):
                self.packed.groups[row] = group
            seen.add(uid)
            value = np.zeros((), INSTANCE)
            value['world'] = (np.eye(4, dtype=np.float32) if matrix is None
                              else np.asarray(matrix.data(), np.float32).reshape(4, 4))
            _, paint = state(proxy)
            value['info'][:3] = (code, paint, flags | (8 if mode else 0))
            if row < len(self.packed.instances):
                value['info'][3] = self.packed.instances['info'][row, 3]
                value['info'][2] |= self.packed.instances['info'][row, 2] & 192
            if row >= len(self.packed.instances) or value.tobytes() != self.packed.instances[row].tobytes():
                ids.append(row)
                values.append(value)
        removed = []
        if local:
            for root in self.pending_removed:
                for group, _ in iter_placements(root):
                    row = self.rows.get(group.uid)
                    if row is not None:
                        seen.discard(group.uid)
                        removed.append(row)
        self.pending.clear()
        self.pending_removed.clear()
        self.version = scene.content_version
        self.journal_valid = True
        self.roots = set(scene.groups) if not local else self.roots
        # Every variant of a changed mesh, and each new variant, rebakes.
        bake = dict(fresh)
        for key, (mesh, _, _) in changed.items():
            for code in self.variants.get(key, ()):
                bake[code] = mesh
        if len(bake) > self.rebake_limit():
            # Revisions and serials also move without an edit (a lazy mesh
            # built, commands that flag every group mesh), and a batch the
            # size of the scene costs more than one full pack. It also broke
            # the resident GPU scene (facilities, 4 432 prototypes replaced:
            # a blank frame, no error), so the full pack it is.
            return None
        self.changed = [(mesh, self.meshes[id(mesh)][1], code) for code, mesh in bake.items()]
        for mesh in {id(m): m for m in bake.values()}.values():
            self.mesh_revisions[id(mesh)] = mesh.revision
        self.removed = removed if local else [self.rows[uid] for uid in self.active - seen]
        self.active = seen
        if additions:
            self.packed.instances = np.concatenate((self.packed.instances, np.zeros(len(additions), INSTANCE)))
            for uid, group, row in additions:
                self.rows[uid] = row
                self.packed.uids.append(uid)
                self.packed.groups.append(group)
        return np.asarray(ids, np.int64), np.asarray(values, dtype=INSTANCE)

    def apply(self, ids, rows, renderer=None):
        if self.changed:
            # A single native bake for the changed set, including shared meshes.
            prototypes, paints = [], {}
            for mesh, _, code in self.changed:
                paint, billboard = self.variants[id(mesh)][code]
                prototype = Group(mesh, name="Resident prototype")
                prototype.billboard = billboard
                paints[id(prototype)] = paint
                prototypes.append(prototype)
            baked = pack_scene(Scene(groups=prototypes), baked_paints=paints)
            baked_row = {id(group): j for j, group in enumerate(baked.groups)}
            from views.wgpu_parity import has_transparency
            if renderer is not None and hasattr(renderer, 'set_material_transparency') and has_transparency(baked):
                renderer.set_material_transparency(True)
            exclusions = dict(self.packed.profile_exclusions)
            paths = list(self.packed.texture_paths)
            for path in baked.texture_paths:
                if path not in paths:
                    paths.append(path)
            texture_map = np.array([0] + [paths.index(path)+1 for path in baked.texture_paths], np.uint32)
            if tuple(paths) != self.packed.texture_paths:
                from views.wgpu_scene import _texture_array
                self.packed.texture_paths = tuple(paths)
                self.packed.textures = _texture_array(paths)
                if renderer is not None:
                    renderer.set_textures(*self.packed.textures)
            baked_exclusions = dict(baked.profile_exclusions)
            for i, (mesh, _, code) in enumerate(self.changed):
                uid = baked_row.get(id(prototypes[i]))
                if uid is None:
                    face = edge = hidden_face = hidden_edge = np.zeros((), COMMAND)
                    bounds = np.zeros((2, 4), np.float32)
                else:
                    prototype = int(baked.instances['info'][uid, 0])
                    face, edge = baked.commands[prototype], baked.edge_commands[prototype]
                    hidden_face = baked.hidden_commands[prototype]
                    hidden_edge = baked.hidden_edge_commands[prototype]
                    bounds = baked.bounds[prototype]
                exclusions[code] = baked_exclusions.get(prototype, []) if uid is not None else []
                start = int(face['base'])
                count = int(face['count'] + edge['count'])
                # Hidden faces and edges follow the visible ones in the bake;
                # the renderer takes the visible range, the resident arrays
                # keep all four for Show hidden geometry.
                total = count + int(hidden_face['count'] + hidden_edge['count'])
                vertices = baked.vertices[start:start+total]
                indices = baked.indices[start:start+total]
                if renderer is not None:
                    renderer.replace_prototype(code, vertices[:count].view(np.uint8).reshape(-1),
                                               indices[:count].view(np.uint8).reshape(-1),
                                               bounds.view(np.uint8).reshape(-1), int(face['count']))
                if renderer is not None:
                    material = np.empty(0, np.uint8)
                    if uid is not None:
                        header = baked.materials.view(np.uint32).reshape(-1, 4)[prototype]
                        first, length = int(header[1]), int(header[2])
                        if length:
                            material = baked.materials[first*16:first*16+length*32].copy()
                            words = material.view(np.uint32).reshape(-1, 8)
                            words[:, 5:7] = texture_map[words[:, 5:7]]
                        elif header[3] & 1:
                            # Incremental material API takes corner records. Expand
                            # only this edited prototype's compressed matching back.
                            records = np.zeros((int(face['count']), 32), np.uint8)
                            records[:, 16:20] = vertices['color'][:int(face['count'])]
                            material = records.reshape(-1)
                    renderer.update_prototype_materials(code, material)
                offset = len(self.packed.vertices)
                self.packed.vertices = np.concatenate((self.packed.vertices, vertices))
                self.packed.indices = np.concatenate((self.packed.indices, indices))
                self.packed.bounds[code] = bounds
                first = offset
                for target, source in ((self.packed.commands, face), (self.packed.edge_commands, edge),
                                       (self.packed.hidden_commands, hidden_face),
                                       (self.packed.hidden_edge_commands, hidden_edge)):
                    target[code]['count'] = source['count']
                    target[code]['base'] = offset
                    target[code]['first'] = first
                    first += int(source['count'])
                self.meshes[id(mesh)] = (mesh, mesh._mut_serial, code)
            # The renderer takes the whole list (0.5 s on the ride): send it
            # only when an edited prototype's own exclusions moved.
            before = dict(self.packed.profile_exclusions)
            moved = any((before.get(code) or []) != (exclusions.get(code) or [])
                        for _, _, code in self.changed)
            self.packed.profile_exclusions = tuple((c, p) for c, p in exclusions.items() if p)
            if renderer is not None and hasattr(renderer, 'set_profile_exclusions') and moved:
                renderer.set_profile_exclusions(self.packed.profile_exclusions)
        if len(ids):
            if renderer is not None:
                renderer.update_instances(ids.tolist(), rows.view(np.uint8).reshape(-1))
            self.packed.instances[ids] = rows
        if self.removed:
            if renderer is not None:
                renderer.remove_instances(self.removed)
            self.packed.instances['info'][self.removed, 2] = 1
        if not len(ids) and not self.changed and not self.removed:
            return
        old_count = len(self.world_boxes)
        if len(self.packed.instances) != old_count or self.changed:
            counts = np.bincount(self.packed.instances['info'][:, 0], minlength=len(self.packed.commands))
            self.packed.commands['start'] = self.packed.edge_commands['start'] = np.cumsum(counts)-counts
            self.packed.hidden_commands['start'] = self.packed.hidden_edge_commands['start'] = np.cumsum(counts)-counts
            extra = np.empty((len(self.packed.instances)-old_count, 2, 3), np.float32)
            extra[:, 0] = np.inf
            extra[:, 1] = -np.inf
            self.world_boxes = np.concatenate((self.world_boxes, extra))
        touched = set(ids.tolist()) | set(self.removed)
        if self.changed:
            codes = [code for _, _, code in self.changed]
            touched.update(np.flatnonzero(np.isin(self.packed.instances['info'][:, 0], codes)).tolist())
        touched = np.asarray(sorted(touched), np.int64)
        if len(touched):
            old = self.world_boxes[touched]
            bounds = self.packed.world_bounds
            reduce = np.any(old[:, 0] == bounds[0]) or np.any(old[:, 1] == bounds[1])
            self.world_boxes[touched] = self._boxes(touched)
            subset = self.world_boxes if reduce else self.world_boxes[touched]
            low, high = subset[:, 0].min(axis=0), subset[:, 1].max(axis=0)
            if not reduce:
                low, high = np.minimum(low, bounds[0]), np.maximum(high, bounds[1])
            self.packed.world_bounds = (np.array([low, high]) if np.isfinite(low).all()
                                        else np.zeros((2, 3)))

"""Diff resident rows and mesh serials without baking record-only edits.

Removed rows are tombstones: selections and GPU pick IDs never shift. A batch
of mutated prototypes uses one R1b bake; transform previews only write rows.
A prototype is (mesh, baked paint, billboard): a changed mesh rebakes every
variant it has, each with its own paint, hidden faces/edges included, so an
edit in a nested, painted or hidden-geometry model never repacks the scene.

Entering a component instance (or a transformed container) swaps the group's
mesh for a world-space copy; leaving puts the shared definition back. Both
are *swaps*: the group keeps its row, paint and flags, only its prototype
and matrix change, so they are written from the open edit path instead of
walking every placement of the model (2 s on the ride).
"""
import numpy as np
from core.group import Group
from core.purge import iter_groups
from core.scene import Scene
from views.wgpu_scene import (COMMAND, INSTANCE, _baked_paint, _placement_records,
                              _placement_state, pack_scene)


def _world_copy_edits():
    """Commands that only change the mesh they ran on: inside a session
    folded into a share-back, that is the world copy, drawn by no row."""
    from core.history import (SnapshotMutation, SnapshotCompound, MoveVerticesCommand,
                              AddEdgeCommand, AddFaceCommand, EraseSelectionCommand)
    return (SnapshotMutation, SnapshotCompound, MoveVerticesCommand,
            AddEdgeCommand, AddFaceCommand, EraseSelectionCommand)


def _edits_world_copy_only(command):
    from core.history import CompoundCommand
    if isinstance(command, CompoundCommand):
        return all(_edits_world_copy_only(c) for c in command.commands)
    return isinstance(command, _world_copy_edits())


def _resident_version(scene):
    """The content version less the edit-context bumps: entering or leaving
    a group swaps meshes without moving anything in the world (the view's
    content token reads the same), so the journal stays usable across it."""
    return scene.content_version - getattr(scene, '_placement_context_bumps', 0)


def _has_vertices(mesh):
    src = mesh.__dict__.get('_deferred_source')
    return bool(len(src['pos']) if src is not None else len(mesh.vertices))


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
        # The rows' keys, prototypes, live keys and world boxes, in Rust
        # (rust/ingetrazo_render/src/resident_rows.rs); also the view's
        # uid_rows mapping.
        from ingetrazo_render import ResidentRows
        self.resident = ResidentRows(list(packed.uids), packed.instances.view(np.uint8).reshape(-1),
                                     np.ascontiguousarray(packed.bounds).view(np.uint8).reshape(-1))
        self.changed_code_rows = {}
        if packed.mesh_codes:
            # What the pack met: each mesh once (empty containers' with -1,
            # a serial sentinel without a GPU row).
            self.meshes = {key: (mesh, mesh._mut_serial, code) for key, (mesh, code) in packed.mesh_codes.items()}
        else:
            codes = packed.instances['info'][:, 0].tolist()
            for row, group in enumerate(packed.groups):
                mesh = group.mesh if group is not None else scene.loose_mesh
                self.meshes[id(mesh)] = (mesh, mesh._mut_serial, codes[row])
            all_meshes = [scene.loose_mesh]
            all_meshes.extend(g.mesh for g in iter_groups(scene.groups))
            for mesh in all_meshes:
                self.meshes.setdefault(id(mesh), (mesh, mesh._mut_serial, -1))
        self.mesh_revisions = {key: mesh.revision for key, (mesh, _, _) in self.meshes.items()}
        self.changed = []
        self.removed = []
        self.version = _resident_version(scene)
        # uid -> [root, ..., group]: groups of the open edit path that draw
        # another mesh than the one resident (see the module docstring).
        self.pending_swaps = {}
        self.chains = {}
        # (vertices, indices) with spare room past the resident arrays, so
        # a rebake that outgrows its range appends without copying them.
        self._store = None
        self._reserving = None
        self.last_local = False
        self.added = 0
        self.pending = set()
        self.pending_meshes = set()
        self.pending_materials = set()
        self.pending_removed = set()
        self.journal_valid = True
        self.unsubscribe = None
        # The change feed's records (core/protocol) of the meshes pending
        # a rebake, per id(mesh), and after diff() those of the meshes it
        # rebakes: the element keys a partial rebake would start from.
        self.mesh_changes = {}
        self.changed_records = {}
        self.roots = set(scene.groups)

    @property
    def uid_rows(self):
        """key -> its rows (the view's mapping)."""
        return self.resident

    def observe(self, history):
        """Use History's existing journal callbacks to avoid scanning known edits."""
        if self.unsubscribe is not None or history is None:
            return
        from weakref import WeakKeyDictionary
        material_targets = WeakKeyDictionary()

        def before(command, phase, ran):
            if _resident_version(self.scene) != self.version:
                self.journal_valid = False

        feed = getattr(history, 'feed', None)

        def after(command, phase, ran):
            # The feed published this step before any other listener ran.
            step = feed.step(history.revision) if (ran and feed is not None) else None
            if step is not None:
                for change in step.meshes:
                    mesh = change.mesh
                    if mesh is not None and id(mesh) in self.meshes:
                        self.mesh_changes.setdefault(id(mesh), []).append(change)
            from core.history import (MoveGroupCommand, CompoundCommand, SetRecordCommand,
                                      SetRegistryCommand, InsertGroupCommand, DeleteGroupCommand,
                                      SnapshotMutation, SnapshotCompound, MoveVerticesCommand, AddEdgeCommand,
                                      AddFaceCommand, EraseSelectionCommand, HideCommand, AssignLayerCommand,
                                      AddGuideCommand, ChangeGuideCommand,
                                      DeleteGuidesCommand, MoveGuidesCommand, RotateGuidesCommand,
                                      AddDimensionCommand, AddTextLabelCommand)
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
                        definition = getattr(mesh, "_instance_definition", mesh)
                        if not faces.issubset(set(mesh.faces)) or id(definition) not in self.meshes:
                            return False
                        target = material_targets[command] = id(definition)
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
                def edited(mesh):
                    old = self.meshes.get(id(mesh))
                    if old is None:
                        # The open group's world copy, first edited.
                        chain = self._stack_chain(mesh)
                        if chain is None:
                            return False
                        self.pending_swaps[self.packed.row_key(chain[-1])] = chain
                        return True
                    if old[2] < 0:
                        # diff already checks the loose mesh for its first row.
                        return mesh is self.scene.loose_mesh
                    self.pending_meshes.add(id(mesh))
                    return True
                if isinstance(command, (SnapshotMutation, SnapshotCompound, MoveVerticesCommand,
                                        AddEdgeCommand, AddFaceCommand, EraseSelectionCommand)):
                    if step is not None:
                        # The meshes the step changed, as the feed names them
                        # (store-backed: by element; Python: replaced).
                        return all(edited(c.mesh) for c in step.meshes if c.mesh is not None)
                    mesh = command._target(self.scene) if isinstance(command, SnapshotMutation) else self.scene.mesh
                    return edited(mesh)
                from core.soften import SoftenByAngleCommand
                if isinstance(command, SoftenByAngleCommand):
                    # Soft flags only: each mesh's prototypes rebake, smooth.
                    return all(edited(mesh) for mesh in command.meshes())
                if getattr(command, "reference_instance", False):
                    # Leaving an edited instance: back on the definition,
                    # which every copy shows (all its variants rebake).
                    if not self.queue_swap(command.group):
                        return False
                    old = self.meshes.get(id(command.proto))
                    if old is not None and old[2] >= 0:
                        self.pending_meshes.add(id(command.proto))
                    return True
                if getattr(command, "reference_assembly", False):
                    # Leaving an edited assembly: its row goes back on the
                    # definition and its parts' matrices come back up, which
                    # leaves every world matrix where it was. What ran inside
                    # drew on the world copy, which no row shows any more;
                    # a part edited inside shares back as its own swap. Any
                    # other change to the parts (one deleted, moved,
                    # exploded) rescans the placements.
                    if not self.queue_swap(command.group):
                        return False
                    for inner in command.inner:
                        if getattr(inner, "reference_instance", False) or getattr(inner, "reference_assembly", False):
                            if not visit(inner):
                                return False
                        elif not _edits_world_copy_only(inner):
                            return False
                    old = self.meshes.get(id(command.proto))
                    if old is not None and old[2] >= 0:
                        self.pending_meshes.add(id(command.proto))
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
                from core.history import UnlinkCommand
                if isinstance(command, UnlinkCommand):
                    # Make Unique of a link: the same rows (the parts keep
                    # their uids) on meshes of their own — new prototypes
                    # for those, baked once; undo goes back to the shared
                    # ones, still resident. Only the roots holding them.
                    tops = command.tops
                    if not tops or not self.roots.issuperset(tops):
                        return False
                    self.pending.update(tops)
                    return True
                return isinstance(command, (SetRecordCommand, SetRegistryCommand,
                                            AddGuideCommand, ChangeGuideCommand, DeleteGuidesCommand,
                                            MoveGuidesCommand, RotateGuidesCommand,
                                            AddDimensionCommand, AddTextLabelCommand))
            self.journal_valid = self.journal_valid and ran and visit(command)
            self.version = _resident_version(self.scene)
        self.unsubscribe = history.add_listener(before=before, after=after)

    def remember_edit_path(self):
        """Record ``[root, ..., group]`` for every resident group on the open
        edit path, so leaving it (the definition back on the instance) can
        be a row swap even after the path is closed."""
        chain = []
        from core.outliner import open_path
        for group in open_path(self.scene):
            if chain and group not in (getattr(chain[-1], 'children', None) or ()):
                return
            if not chain and group not in self.roots:
                return
            chain.append(group)
            if self.resident.row(self.packed.row_key(group)) is not None:
                self.chains[self.packed.row_key(group)] = list(chain)

    def _stack_chain(self, mesh):
        """The recorded path to the open group that now draws ``mesh``, or
        None (not a context swap, or not resident)."""
        self.remember_edit_path()
        from core.outliner import open_path
        for group in open_path(self.scene):
            if group.mesh is mesh:
                chain = self.chains.get(self.packed.row_key(group))
                return chain if chain is not None and chain[-1] is group else None
        return None

    def queue_swap(self, group) -> bool:
        """Rewrite ``group``'s row from its current mesh and the edit path
        it was opened on. False when that path is unknown (full diff)."""
        chain = self.chains.get(self.packed.row_key(group))
        if chain is None or chain[-1] is not group:
            return False
        self.pending_swaps[self.packed.row_key(group)] = chain
        return True

    def _resolve_swaps(self):
        """``(row, group, matrix, paint)`` per pending swap, or None when
        one no longer stands (the path was restructured meanwhile)."""
        out = []
        for uid, chain in self.pending_swaps.items():
            row = self.resident.row(uid)
            if row is None or chain[0] not in self.roots or row >= len(self.packed.groups):
                return None
            matrix = None
            for parent, group in zip([None] + chain[:-1], chain):
                if parent is not None and group not in (getattr(parent, 'children', None) or ()):
                    return None
                xform = getattr(group, 'xform', None)
                matrix = xform if matrix is None else (matrix if xform is None else matrix * xform)
            code = int(self.packed.instances['info'][row, 0])
            variant = next((v[code] for v in self.variants.values() if code in v), None)
            if variant is None:
                return None
            world = np.eye(4, dtype=np.float32) if matrix is None else np.asarray(matrix.data(), np.float32).reshape(4, 4)
            out.append((row, chain[-1], world, variant[0]))
        return out

    def _link_swaps(self):
        change = getattr(self.scene, '_link_mesh_changes', None)
        if (not change or change[2] is None or not self.journal_valid
                or self.pending or self.pending_meshes or self.pending_materials
                or self.pending_removed or self.pending_swaps or self.scene._edit_stack
                or change[:2] != (self.version + self.scene._placement_context_bumps,
                                  self.scene.content_version)):
            return None
        swaps = []
        for group, old_mesh in change[2]:
            row = self.resident.row(self.packed.row_key(group))
            if (row is None or not self.resident.is_active(self.packed.row_key(group))
                    or not _has_vertices(group.mesh)):
                return None
            code = int(self.packed.instances['info'][row, 0])
            variant = self.variants.get(old_mesh, {}).get(code)
            if variant is None:
                return None
            swaps.append((row, group, self.packed.instances['world'][row], variant[0]))
        return swaps

    def _new_prototype(self, mesh, key, paint, mode, old, fresh):
        """Allocate a prototype code for a variant never drawn before."""
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
        return code, old

    def rebake_limit(self) -> int:
        """Most prototypes one edit rebakes in place; past it, a full pack."""
        return max(256, len(self.packed.commands) // 8)

    def diff(self, scene):
        """Return changed resident rows, retaining prototype and placement IDs."""
        if self.packed.model_translations is not None:
            return None
        if scene is not self.scene:
            return None
        local = self.unsubscribe is not None and self.journal_valid and _resident_version(scene) == self.version
        swaps = self._resolve_swaps() if local else self._link_swaps()
        local = swaps is not None and all(_has_vertices(self.meshes[k][0]) for k in self.pending_meshes)
        if not local:
            swaps = None
        self.pending_swaps.clear()
        self.last_local = local
        placements = [] if local else [(None, None, None, 0, None)]
        state = _placement_state(scene)
        loose = self.meshes.get(id(scene.loose_mesh))
        if local and (loose is None or loose[1] != scene.loose_mesh._mut_serial):
            placements.append((None, None, None, 0, None))
        from types import SimpleNamespace
        from views.document_interaction import _paint_of
        vp = SimpleNamespace(scene=scene)
        roots = self.pending if local else scene.groups
        placements.extend(_placement_records(scene, roots, vp, state))
        # A local diff adds to the live keys; a full one starts from none.
        self.resident.begin(local)
        ids, values, changed = [], [], {}
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
        new_keys = set()
        fresh = {}      # code -> mesh: variants that exist from this edit on
        resolved = {}
        for group, matrix, proxy, flags, columns in placements:
            if columns is not None:
                mesh, mode, _uid = columns
            else:
                mesh = group.mesh if group is not None else scene.loose_mesh
                mode = getattr(group, 'billboard', False)
            got = resolved.get(id(proxy))
            if got is None:
                got = resolved[id(proxy)] = (_baked_paint(_paint_of(vp, proxy)), state(proxy)[1])
            paint = got[0]
            key = (id(mesh), repr(paint), mode)
            old = self.meshes.get(id(mesh))
            code = self.packed.prototype_lookup.get(key)
            uid = self.packed.row_key(group)
            if old is not None and old[2] < 0 and old[1] == mesh._mut_serial:
                continue    # empty when packed, under any paint: no row
            if (old is None or old[1] != mesh._mut_serial) and not _has_vertices(mesh):
                if old is None:
                    self.meshes[id(mesh)] = (mesh, mesh._mut_serial, -1)
                continue
            if code == -1:
                code = None
            if code is None:
                # A new mesh, or a known mesh under a paint or billboard
                # mode it was never drawn with: a prototype of its own.
                code, old = self._new_prototype(mesh, key, paint, mode, old, fresh)
            if old[1] != mesh._mut_serial:
                changed[id(mesh)] = old
            row = self.resident.row(uid)
            if group in self.packed.occurrence_keys:
                from core.link_structure import row_id
                if row_id(scene.occurrence_path(group)) != uid:
                    return None
            if row is not None and self.resident.is_active(uid):
                previous_group = self.packed.groups[row]
                if previous_group is not group and (previous_group is None or
                        scene.occurrence_path(previous_group) is not None):
                    return None
            if row is None:
                # A new collision changes how the whole set of equal UIDs is
                # addressed. A new pack assigns qualified keys to all of them.
                if uid in new_keys or any(g.uid == uid for g in self.packed.occurrence_keys):
                    return None
                new_keys.add(uid)
                row = len(self.packed.instances) + len(additions)
                additions.append((uid, group, row))
            if row < len(self.packed.groups):
                self.packed.groups[row] = group
            self.resident.see(uid)
            value = np.zeros((), INSTANCE)
            value['world'] = (np.eye(4, dtype=np.float32) if matrix is None
                              else np.asarray(matrix, np.float32))
            value['info'][:3] = (code, got[1], flags | (8 if mode else 0))
            if row < len(self.packed.instances):
                value['info'][3] = self.packed.instances['info'][row, 3]
                value['info'][2] |= self.packed.instances['info'][row, 2] & 192
            if row >= len(self.packed.instances) or value.tobytes() != self.packed.instances[row].tobytes():
                ids.append(row)
                values.append(value)
        for row, group, world, paint in swaps or ():
            # Same group, same paint and flags: its prototype and matrix.
            mesh = group.mesh
            mode = getattr(group, 'billboard', False)
            key = (id(mesh), repr(paint), mode)
            old = self.meshes.get(id(mesh))
            code = self.packed.prototype_lookup.get(key)
            if code is None or code < 0:
                code, old = self._new_prototype(mesh, key, paint, mode, old, fresh)
            if old[1] != mesh._mut_serial:
                changed[id(mesh)] = old
            self.packed.groups[row] = group
            value = self.packed.instances[row].copy()
            value['world'] = world
            value['info'][0] = code
            if value.tobytes() != self.packed.instances[row].tobytes():
                ids.append(row)
                values.append(value)
        removed = []
        if local:
            for root in self.pending_removed:
                for group in iter_groups((root,)):
                    row = self.resident.row(self.packed.row_key(group))
                    if row is not None:
                        self.resident.unsee(self.packed.row_key(group))
                        removed.append(row)
        if any(not self.resident.has_seen(key) and (self.resident.is_active(key) or self.resident.row(key) in removed)
               for key in self.packed.occurrence_keys.values()):
            return None
        self.pending.clear()
        self.pending_removed.clear()
        self.version = _resident_version(scene)
        self.journal_valid = True
        self.roots = set(scene.groups) if not local else self.roots
        # Every variant of a changed mesh, and each new variant, rebakes.
        bake = dict(fresh)
        for key, (mesh, _, _) in changed.items():
            for code in self.variants.get(key, ()):
                bake[code] = mesh
        # Incremental placement writes use unshifted matrices. A new remote
        # placement or remote prototype needs bounds and a new full pack.
        from core.coordinate_frame import REBASE_DISTANCE
        if any(np.max(abs(row['world'][3, :3])) > REBASE_DISTANCE for row in values):
            return None
        for mesh in {id(m): m for m in bake.values()}.values():
            store = mesh.__dict__.get('_store')
            if store is not None:
                pos = store.positions()
                if len(pos) and np.max(abs((pos.min(axis=0) + pos.max(axis=0)) * .5
                                           + store.frame_origin)) > REBASE_DISTANCE:
                    return None
        records, self.mesh_changes = self.mesh_changes, {}
        self.changed_records = {key: records[key] for key in {id(m) for m in bake.values()}
                                if key in records}
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
        gone = self.resident.finish()
        self.removed = removed if local else gone
        self.added = len(additions)
        if additions:
            self.packed.instances = np.concatenate((self.packed.instances, np.zeros(len(additions), INSTANCE)))
            for uid, group, row in additions:
                self.resident.add(uid, row)
                self.packed.uids.append(uid)
                self.packed.groups.append(group)
        return np.asarray(ids, np.int64), np.asarray(values, dtype=INSTANCE)

    def reserve(self, executor):
        """Copy the resident CPU arrays into ones with spare room on
        ``executor``, ahead of the first edit that appends. The first
        append otherwise copies the whole model on the UI thread (65-200 ms
        on the ride's 32 M vertices). Called on entering a group: the
        intent to edit, and nothing is held twice for a model only looked
        at (the copy is adopted as soon as it is done)."""
        if self._store is not None or self._reserving is not None:
            return
        src_v, src_i = self.packed.vertices, self.packed.indices

        def copy():
            n = len(src_v)
            capacity = n + max(n // 32, 4096)
            vbuf = np.empty(capacity, src_v.dtype)
            ibuf = np.empty(capacity, src_i.dtype)
            # As raw bytes: a plain copy that releases the GIL.
            for target, source in ((vbuf, src_v), (ibuf, src_i)):
                if source.flags.c_contiguous:
                    target[:n].view(np.uint8)[:] = source.view(np.uint8).reshape(-1)
                else:
                    target[:n] = source
            return src_v, src_i, vbuf, ibuf
        self._reserving = executor.submit(copy)

    def adopt_reserved(self, wait=False):
        """Swap a finished reservation in: the old ``(vertices, indices)``
        when it was adopted, None when none is pending or it no longer
        matches, False while it is still copying (and not ``wait``)."""
        future = self._reserving
        if future is None:
            return None
        if not wait and not future.done():
            return False
        self._reserving = None
        try:
            src_v, src_i, vbuf, ibuf = future.result()
        except Exception:
            return None
        # Every write to these arrays goes through _place, which adopts
        # first: identical arrays mean identical contents.
        if (self._store is not None or self.packed.vertices is not src_v
                or self.packed.indices is not src_i):
            return None
        n = len(src_v)
        self.packed.vertices, self.packed.indices = vbuf[:n], ibuf[:n]
        self._store = (vbuf, ibuf)
        return src_v, src_i

    def _place(self, code, vertices, indices) -> int:
        """Write a rebaked prototype into the resident CPU arrays; its base.

        Concatenating onto them copied the whole model per edit (0.5 s on
        the ride, 1.4 s the first time). A rebake that fits the range its
        code owns goes back there (a moved face keeps its counts); a longer
        one is appended into spare room, grown by a margin when full."""
        self.adopt_reserved(wait=True)
        total = len(vertices)
        packed = self.packed
        tables = (packed.commands, packed.edge_commands,
                  packed.hidden_commands, packed.hidden_edge_commands)
        if code < len(packed.commands):
            owned = sum(int(t[code]['count']) for t in tables)
            base = int(packed.commands[code]['base'])
            if total <= owned:
                sizes = sum(t['count'].astype(np.int64) for t in tables)
                # Two codes never share a range in the bake; check anyway,
                # a shared one must not be overwritten for both.
                if np.count_nonzero((packed.commands['base'] == base) & (sizes > 0)) == 1:
                    packed.vertices[base:base+total] = vertices
                    packed.indices[base:base+total] = indices
                    return base
        used = len(packed.vertices)
        store = self._store
        if (store is None or packed.vertices.base is not store[0]
                or used + total > len(store[0])):
            capacity = used + total + max(total * 4, used // 32, 4096)
            vbuf = np.empty(capacity, packed.vertices.dtype)
            ibuf = np.empty(capacity, packed.indices.dtype)
            vbuf[:used] = packed.vertices
            ibuf[:used] = packed.indices
            store = self._store = (vbuf, ibuf)
        vbuf, ibuf = store
        vbuf[used:used+total] = vertices
        ibuf[used:used+total] = indices
        packed.vertices = vbuf[:used+total]
        packed.indices = ibuf[:used+total]
        return used

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
            before = dict(exclusions)
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
                # Billboards suppress the visible edge draw, but its endpoint
                # vertices still precede the hidden ranges in the native bake.
                total = int(hidden_edge['first']) + int(hidden_edge['count']) - start
                vertices = baked.vertices[start:start+total]
                indices = baked.indices[start:start+total]
                if renderer is not None:
                    renderer.replace_prototype(code, vertices[:count].view(np.uint8).reshape(-1),
                                               indices[:count].view(np.uint8).reshape(-1),
                                               bounds.view(np.uint8).reshape(-1), int(face['count']))
                material = np.empty(0, np.uint8)
                flags = 0
                if uid is not None:
                    header = baked.materials.view(np.uint32).reshape(-1, 4)[prototype]
                    first, length, flags = map(int, header[1:])
                    if length:
                        material = baked.materials[first*16:first*16+length*32].copy()
                        words = material.view(np.uint32).reshape(-1, 8)
                        words[:, 5:7] = texture_map[words[:, 5:7]]
                self.packed.material_updates[code] = flags, material
                if renderer is not None:
                    if flags & 1:
                        # The live API takes explicit corner records. A full
                        # upload retains the compact matching-back header.
                        records = np.zeros((int(face['count']), 32), np.uint8)
                        records[:, 16:20] = vertices['color'][:int(face['count'])]
                        material = records.reshape(-1)
                    renderer.update_prototype_materials(code, material)
                offset = self._place(code, vertices, indices)
                self.packed.bounds[code] = bounds
                for target, source in ((self.packed.commands, face), (self.packed.edge_commands, edge),
                                       (self.packed.hidden_commands, hidden_face),
                                       (self.packed.hidden_edge_commands, hidden_edge)):
                    target[code]['count'] = source['count']
                    target[code]['base'] = offset
                    target[code]['first'] = offset + int(source['first']) - start
                self.meshes[id(mesh)] = (mesh, mesh._mut_serial, code if uid is not None else -1)
            self.packed.profile_exclusions = tuple((c, p) for c, p in exclusions.items() if p)
            if renderer is not None and hasattr(renderer, 'set_profile_exclusions'):
                if getattr(renderer, 'incremental_profile_exclusions', False):
                    # Only the rebaked prototypes: each came back on a fresh,
                    # unexcluded range; the others keep theirs natively.
                    pairs = [(code, exclusions[code]) for _, _, code in self.changed
                             if exclusions.get(code)]
                    if pairs:
                        renderer.set_profile_exclusions(pairs)
                else:
                    # An older module rebuilds every prototype from the
                    # whole list (0.5 s on the ride): send it only when an
                    # edited prototype's own exclusions moved.
                    moved = any((before.get(code) or []) != (exclusions.get(code) or [])
                                for _, _, code in self.changed)
                    if moved:
                        renderer.set_profile_exclusions(self.packed.profile_exclusions)
        old_count = self.resident.count
        membership_changed = len(self.packed.instances) != old_count
        if len(ids):
            membership_changed = self.resident.set_codes(ids.tolist(), rows['info'][:, 0].tolist()) or membership_changed
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
        # A rebake (self.changed) keeps membership but re-anchors its
        # prototypes' ranges: recompute the starts then too (c5a8bfde).
        if membership_changed or self.changed:
            starts = self.resident.starts(len(self.packed.commands))
            self.packed.commands['start'] = self.packed.edge_commands['start'] = starts
            self.packed.hidden_commands['start'] = self.packed.hidden_edge_commands['start'] = starts
        self.changed_code_rows = {code: self.resident.members(code) for _, _, code in self.changed}
        parts = [ids, np.asarray(self.removed, np.int64), *self.changed_code_rows.values()]
        touched = np.unique(np.concatenate(parts)) if parts else np.empty(0, np.int64)
        if len(touched):
            lo, hi = np.asarray(self.packed.world_bounds, np.float32)
            got = self.resident.refresh_boxes(touched.tolist(), self.packed.instances.view(np.uint8).reshape(-1),
                                              np.ascontiguousarray(self.packed.bounds).view(np.uint8).reshape(-1),
                                              lo.tolist(), hi.tolist())
            self.packed.world_bounds = np.array(got, np.float32) if got is not None else np.zeros((2, 3))

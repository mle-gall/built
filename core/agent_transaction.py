# SPDX-License-Identifier: GPL-3.0-or-later
"""Extend recipe snapshots to edits of existing objects made by agent tools."""
from copy import deepcopy

from core import doc_model as _dm
from core.group import iter_placements
from built_core.history import SnapshotImport


class AgentSnapshot(SnapshotImport):
    """Preserve group identity/sharing while undoing metadata and mesh edits.

    SnapshotImport supplies loose geometry and top-level collection membership.
    Tools also edit existing nested groups and extension records, which are
    outside that import-oriented snapshot.
    """
    def __init__(self, mutate):
        super().__init__(mutate)
        self.extra_before = self.extra_after = None

    @staticmethod
    def capture(scene):
        groups = {g for root in scene.groups + scene.kept_components
                  for g, _ in iter_placements(root)}
        meshes = {g.mesh for g in groups}
        objects = list(groups) + list(scene.layers)
        memo = {id(o): o for o in [*objects, *meshes]}
        attrs = [(o, deepcopy(_dm.object_state(o), memo)) for o in objects]
        from built_core.mesh import DeferredMesh
        mesh_states = {m: None if type(m) is DeferredMesh else m.capture_state()
                       for m in meshes if m is not scene.mesh}
        fields = {k: deepcopy(getattr(scene, k), memo) for k in (
            "plugin_data", "layer_folders", "folder_parents", "layer_folder_uids",
            "folder_visibility",
            "active_layer", "units", "dimension_style", "camera_home", "custom_scales")}
        return attrs, mesh_states, fields

    @staticmethod
    def restore(scene, state, changed_meshes=None):
        attrs, meshes, fields = state
        memo = {id(o): o for o, _ in attrs}
        memo.update((id(m), m) for m in meshes)
        memo[id(scene.mesh)] = scene.mesh
        for obj, values in attrs:
            restored = deepcopy(values, memo)
            # Keep child-list references held by extensions alive.
            if _dm.is_list(getattr(obj, "children", None)):
                obj.children[:] = restored.pop("children", [])
                restored["children"] = obj.children
            _dm.set_object_state(obj, restored)
        for mesh, values in meshes.items():
            if values is None:
                continue
            if changed_meshes is None:
                # Failed call: compare with the before state for rollback.
                if mesh.capture_state() == values:
                    continue
            elif mesh not in changed_meshes:
                continue
            mesh.restore_state(values)
        for name, value in fields.items():
            restored = deepcopy(value, memo)
            current = getattr(scene, name)
            if isinstance(current, dict):
                current.clear()
                current.update(restored)
            elif isinstance(current, list):
                current[:] = restored
            else:
                setattr(scene, name, restored)

    @property
    def changed(self):
        return super().changed or self.extra_before != self.extra_after

    def do(self, scene):
        if self.extra_after is not None:
            super().do(scene)
            self.restore(scene, self.extra_after, self.changed_meshes)
            return
        from built_core.mesh import on_mesh_materialized
        self.extra_before = self.capture(scene)
        before = self.extra_before[1]

        def first_touch(mesh):
            if mesh in before and before[mesh] is None:
                before[mesh] = mesh.capture_state()

        token = on_mesh_materialized.set(first_touch)
        try:
            super().do(scene)
            self.extra_after = self.capture(scene)
            after = self.extra_after[1]
            self.changed_meshes = {m for m in before.keys() & after.keys()
                                   if before[m] != after[m]}
            # Membership is restored separately. A removed or newly attached
            # mesh can also have been edited on the same transaction.
            self.changed_meshes.update(before.keys() ^ after.keys())
            for m in self.changed_meshes & before.keys() & after.keys():
                m.bump_revision()  # raw face/edge attribute edits bypass geometry serials
        except BaseException:
            self.restore(scene, self.extra_before)
            raise
        finally:
            on_mesh_materialized.reset(token)

    def undo(self, scene):
        super().undo(scene)
        self.restore(scene, self.extra_before, self.changed_meshes)


#: The Group fields a structural snapshot keeps: what a script reassigns
#: (``g.mesh = fresh``, ``parent.adopt([...])``, ``g.hidden = True``) as
#: opposed to editing an object in place.
_TREE_FIELDS = ("mesh", "name", "layer", "billboard", "xform", "hidden",
                "material", "component", "locked")


class TreeSnapshot(SnapshotImport):
    """``run_python``'s step: SnapshotImport plus the nested group tree.

    SnapshotImport only sees the loose mesh and the scene's top-level lists,
    so a script that adopted parts into a nested container, swapped a part's
    mesh or hid a sub-group reported ``changed: False`` and left no undo
    step. :class:`AgentSnapshot` covers that by copying every mesh, which on
    the ride (100k groups) is far too heavy to run on every console line.

    This keeps each group's children list and reassignable fields (mesh by
    reference, a copy of its matrix): cheap, and enough to undo a whole
    rebuild done by building new meshes and groups. A mesh edited IN PLACE
    (``move_vertex``, ``add_face`` on a mesh that existed before) is noticed
    through its revision counters (``in_place``) but not restored."""

    def __init__(self, mutate):
        def run(scene):
            mutate(scene)
            # Capture inside SnapshotImport's transaction so malformed tree
            # edits also restore top-level membership when capture fails.
            self.tree_after = self.capture(scene)

        super().__init__(run)
        self.tree_before = self.tree_after = None
        self.in_place = 0

    @staticmethod
    def capture(scene, serials=None):
        from built_core import Matrix4 as QMatrix4x4
        tree = []
        stack = list(scene.groups) + list(getattr(scene, "kept_components", ()) or ())
        seen = set()
        while stack:
            g = stack.pop()
            if id(g) in seen:
                continue
            seen.add(id(g))
            values = [getattr(g, k, None) for k in _TREE_FIELDS]
            if values[4] is not None:
                values[4] = QMatrix4x4(values[4])
            tree.append((g, (g.children, list(g.children or ())), values))
            mesh = g.mesh
            if serials is not None and mesh is not None and mesh is not scene.mesh:
                serials[mesh] = TreeSnapshot._mesh_revision(mesh)
            stack.extend(g.children or ())
        return tree

    @staticmethod
    def _mesh_revision(mesh):
        return (getattr(mesh, "_mut_serial", 0), getattr(mesh, "revision", 0))

    @staticmethod
    def restore(scene, state):
        from built_core import Matrix4 as QMatrix4x4
        for g, (holder, children), values in state:
            # ``adopt`` swaps in a new list; extensions and the view may
            # hold the original one, so that object comes back, refilled.
            if _dm.is_list(holder):
                holder[:] = children
                g.children = holder
            else:
                g.children = list(children)
            for key, value in zip(_TREE_FIELDS, values):
                if key == "xform" and value is not None:
                    value = QMatrix4x4(value)
                setattr(g, key, value)

    @staticmethod
    def _signature(state):
        return [(id(g), [id(c) for c in children[1]],
                 [id(v) if k == "mesh" else (v.data() if k == "xform" and v is not None else v)
                  for k, v in zip(_TREE_FIELDS, values)])
                for g, children, values in state]

    @property
    def changed(self):
        return (super().changed or self.in_place > 0
                or (self.tree_before is not None and self.tree_after is not None
                    and self._signature(self.tree_before) != self._signature(self.tree_after)))

    def do(self, scene):
        if self.tree_after is not None:
            super().do(scene)
            self.restore(scene, self.tree_after)
            return
        serials = {}
        self.tree_before = self.capture(scene, serials)
        try:
            super().do(scene)
        except BaseException:
            self.restore(scene, self.tree_before)
            raise
        finally:
            self.in_place = sum(self._mesh_revision(mesh) != serial
                                for mesh, serial in serials.items())

    def undo(self, scene):
        super().undo(scene)
        self.restore(scene, self.tree_before)

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Document History and the editor and extension command API."""
from __future__ import annotations

import logging
import os as _os
import time as _time_mod
from pathlib import Path as _Path
from typing import Optional

from ingetrazo_core import DocumentHistory

# ---- The editor and extension command API ------------------------------------
# The undoable commands are native ingetrazo_core steps (docs/commands-port.md).
# What stays here is the plugin command API (docs/plugins.md), the snapshot
# and compound wrappers, and the helpers other modules share.

from ingetrazo_core import planar_edits as _planar
from ingetrazo_core import object_replay as _replay, object_drawing as _drawing
from ingetrazo_core import (definition_edits as _definitions, placement_edits as _placements,
                            placement_transform as _transform)

from abc import ABC, abstractmethod
from collections import Counter
from typing import Iterable, Optional

from core.mesh import element_identity
from core.element_set import ElementSet
from ingetrazo_core import Vec3 as QVector3D
# Typed native steps (docs/commands-port.md): placement renames, locks and
# whole-placement paint are Rust deltas, created through the protocol verbs.
from ingetrazo_core import NativeCommand
# Drawing steps (rust/ingetrazo_core/src/doc/draw_steps.rs).
from ingetrazo_core import (AddEdgeCommand, AddFaceCommand, DeleteEdgesCommand, DeleteFaceCommand,
                            EraseSelectionCommand)
# Plane rebuilds (rust/ingetrazo_core/src/doc/plane_steps.rs).
from ingetrazo_core import HealOverlapsCommand, RebuildPlanarFacesCommand, RebuildPlaneFacesCommand
# Explode (rust/ingetrazo_core/src/doc/explode_steps.rs).
from ingetrazo_core import ExplodeGroupCommand, UnlinkCommand
# Curve tags, push/pull clean-ups, Simplify, Reverse Faces, tag as you draw
# (rust/ingetrazo_core/src/doc/mesh_steps.rs).
from ingetrazo_core import (AutoTagDrawnFacesCommand, CoplanarMergeCommand, ExplodeCurvesCommand,
                            FlipFacesCommand, PruneOrphanEdgesCommand, SimplifyMeshCommand,
                            StitchSolidCommand, TagCurveCommand)
# Material registry edits (rust/ingetrazo_core/src/doc/material_steps.rs).
from ingetrazo_core import DeleteMaterialCommand, EditTexturedMaterialCommand, RestampMaterialCommand
# Transform matrices (rust/ingetrazo_core/src/doc/transform_math.rs).
from ingetrazo_core import invert_scale_factor, mirror_matrix, rotation_matrix, scale_matrix
# Vertex edits (rust/ingetrazo_core/src/doc/vertex_steps.rs).
from ingetrazo_core import (FlipVerticesCommand, MoveVerticesCommand, RotateVerticesCommand,
                            ScaleVerticesCommand)
# Steps with host state (rust/ingetrazo_core/src/doc/host_steps.rs).
from ingetrazo_core import AddGeoPointsCommand, SetNorthCommand, StraightenModelCommand
# Group transform steps (rust/ingetrazo_core/src/doc/transform_steps.rs).
from ingetrazo_core import (ChangeAxesCommand, FlipGroupsCommand, MoveGroupCommand, RotateGroupCommand,
                            ScaleGroupCommand)
# Tag-list steps (rust/ingetrazo_core/src/doc/layer_steps.rs).
from ingetrazo_core import DeleteLayerCommand, EnsureLayersCommand, PurgeUnusedCommand
# Hide and Tag steps (rust/ingetrazo_core/src/doc/tag_steps.rs).
from ingetrazo_core import AssignLayerCommand, HideCommand, HideEdgesCommand
# Face paint steps (rust/ingetrazo_core/src/doc/paint_steps.rs).
from ingetrazo_core import (SetFaceBackCommand, SetFaceColorCommand, SetFaceMaterialTagCommand,
                            SetFaceOpacityCommand, SetFaceTextureCommand)
# Group and component steps (rust/ingetrazo_core/src/doc/group_steps.rs).
from ingetrazo_core import (ReplaceComponentCommand, KeepComponentCommand, AddKeptComponentsCommand, GroupToComponentCommand, EditText3DCommand, ReparentGroupsCommand, MergeGroupsCommand, MakeUniqueCommand, InsertGroupCommand, DeleteGroupCommand, SetStructureNodeCommand, SplitIntoPiecesCommand,
                            SetMaterialFinishCommand, MakeGroupCommand, MakeNestedGroupCommand,
                            MakeComponentOfCommand, GroupInContextCommand, ExplodeViewCommand)
# Annotation, section, guide, image, survey, saved-scene and sheet steps
# (rust/ingetrazo_core/src/doc/steps.rs): the retired classes' names.
from ingetrazo_core import (RecordStep, AddDimensionCommand, DeleteDimensionsCommand, AddTextLabelCommand, MoveTextLabelsCommand, MoveGuidesCommand, RotateGuidesCommand, MoveDimensionsCommand, EditTextLabelCommand, EditDimensionTextCommand, DeleteTextLabelsCommand, AddGuideCommand, PlaceSectionPlaneCommand, DeleteSectionPlanesCommand, ReverseSectionPlaneCommand, SetActiveSectionCommand, SetSectionModeCommand, SetSectionFillCommand, RenameSectionPlaneCommand, AddSheetCommand, PlaceViewCommand, SheetCommand, InsertSheetCommand, RemoveSheetCommand, MoveSheetCommand, CreatePlanCommand, AddSceneCommand, DeleteScenesCommand, UpdateSceneCommand, SceneTreeCommand, MoveSectionPlanesCommand, RotateSectionPlanesCommand, ChangeGuideCommand, DeleteGuidesCommand, AddImagePlaneCommand, DeleteImagePlanesCommand, TransformImagePlaneCommand, SetImagePlaneOpacityCommand, MoveImagePlanesCommand, RotateImagePlanesCommand, ScaleImagePlanesCommand, AddGeoPathCommand, DeleteGeoPointsCommand, DeleteGeoPathsCommand, ToggleGeoPathClosedCommand, SetGeoPathSurfaceCommand, MoveGeoPathNodeCommand)

from core.group import Group
from core import doc_model as _doc
from core.materials import has_own_material
from core.mesh import (Edge, Face, Mesh, Vertex, edge_flags,
                       edge_is_plain, stamp_edge_flags)
from core.topology import (
    _key,
    _loop_edges,
    carve_loop_by_chords,
    defects_near,
    find_containing_face,
    fold_nonplanar_faces,
    heal_overlapping_faces,
    hole_merged_with_loop,
    loop_inside_face,
    new_defects,
    orient_coplanar_faces,
    subtract_loop_from_face,
    winding_footprint,
)


class CommandRejected(Exception):
    """A command declining to happen, on purpose: it has put the scene back
    the way it found it and says why in plain words. :class:`History` takes
    it as a no-op — nothing on the undo stack, nothing in the error log —
    and keeps the reason in ``last_rejected`` for the tool to show."""


class Command(ABC):
    """Abstract reversible operation against a :class:`Scene`."""

    #: Whether doing or undoing it leaves every material where it was —
    #: which ones the model wears.
    keeps_paint = False
    #: Whether doing or undoing it only puts groups in the model or takes
    #: them out, whole, and leaves every group that stays as it was — its
    #: Browser row, the paint of its faces.
    keeps_groups = False
    #: Whether it only changes the geometry of the mesh it edits: no group
    #: comes or goes, and nothing a Browser row shows (name, tag, material,
    #: copies, IFC class, hidden, locked) changes.
    #: A push/pull on the ride no longer reads its 85 000 rows again.
    keeps_rows = False
    #: Undone and redone by its document transaction (core/doc_model.py,
    #: docs/rust-document-model.md §4) instead of by ``undo`` / ``do``:
    #: everything it changes lives in the document. A method
    #: ``doc_undo(scene) -> bool`` for commands that qualify for some
    #: inputs only (asked right after ``do``). ``undo`` stays: it is the
    #: direct and mixed-command replay hook.
    doc_undo = False
    _journal_geometry = False

    def _direct_snapshot(self, mesh):
        if not self._journal_geometry:
            return mesh.capture_state()

    def bind_transaction(self, txn):
        self._doc_txn = txn

    def doc_after(self, scene, phase) -> None:
        """What an undo or redo by transaction still does beside the
        document (``phase`` "undo" / "redo"): bump the version. Commands
        that also drop things from the selection or dirty caches extend it."""
        scene.version += 1

    @abstractmethod
    def do(self, scene) -> None:
        """Apply the operation."""

    @abstractmethod
    def undo(self, scene) -> None:
        """Reverse the operation."""


# The native steps are History commands for every ``isinstance`` check.
Command.register(NativeCommand)
Command.register(RecordStep)


class MeshCommand(Command):
    """Commands whose persistent changes are covered by these geometry stores
    and the document field journal. Object meshes retain the command inverse."""
    def geometry_meshes(self, scene):
        return (scene.mesh,)


# Native mesh steps: History enlists their stores (``geometry_meshes``).
for _mesh_step in (MakeGroupCommand, HideCommand, HideEdgesCommand, AssignLayerCommand,
                   MoveGroupCommand, RotateGroupCommand, ScaleGroupCommand, FlipGroupsCommand,
                   ChangeAxesCommand, AddEdgeCommand, AddFaceCommand, DeleteEdgesCommand,
                   DeleteFaceCommand, EraseSelectionCommand, MoveVerticesCommand,
                   FlipVerticesCommand, RotateVerticesCommand, ScaleVerticesCommand,
                   HealOverlapsCommand, RebuildPlanarFacesCommand, RebuildPlaneFacesCommand,
                   ExplodeGroupCommand, TagCurveCommand, ExplodeCurvesCommand,
                   AutoTagDrawnFacesCommand, PruneOrphanEdgesCommand, CoplanarMergeCommand,
                   StitchSolidCommand, SimplifyMeshCommand, RestampMaterialCommand,
                   EditTexturedMaterialCommand, DeleteMaterialCommand):
    MeshCommand.register(_mesh_step)


class FaceAttributeCommand(MeshCommand):
    def geometry_meshes(self, scene):
        return {getattr(face, "_m", None) for face in self._faces}


for _paint in (SetFaceColorCommand, SetFaceOpacityCommand, SetFaceTextureCommand,
               SetFaceBackCommand, SetFaceMaterialTagCommand, FlipFacesCommand):
    FaceAttributeCommand.register(_paint)


class SetPluginDataCommand(Command):
    """Replace one extension's document data (``scene.plugin_data[key]``),
    undoably. ``value`` None removes the key. Values are copied through JSON
    both ways, so neither the caller nor the history can alias them.

    With the Rust document ``scene.plugin_data`` is a document value: the
    step's transaction undoes it."""

    doc_undo = True

    def __init__(self, key: str, value) -> None:
        import json
        self.key = str(key)
        self.value = None if value is None else json.loads(json.dumps(value))
        self._had = False
        self._before = None

    def do(self, scene) -> None:
        import json
        data = scene.plugin_data
        self._had = self.key in data
        self._before = (json.loads(json.dumps(data[self.key]))
                        if self._had else None)
        if self.value is None:
            data.pop(self.key, None)
        else:
            data[self.key] = json.loads(json.dumps(self.value))
        scene.version += 1

    def undo(self, scene) -> None:
        import json
        if self._had:
            scene.plugin_data[self.key] = json.loads(json.dumps(self._before))
        else:
            scene.plugin_data.pop(self.key, None)
        scene.version += 1


def _geometry_plan(cmd, scene, targets):
    """Enlist all known stores for rollback. Use transaction replay only
    when it also covers the other commands in the step."""
    if isinstance(cmd, CompoundCommand):
        coverage = [_geometry_plan(c, scene, targets) for c in cmd.commands]
        return all(coverage) and (type(cmd) is CompoundCommand or cmd.journal_parts)
    if isinstance(cmd, GroupInContextCommand):
        return _geometry_plan(cmd.inner, scene, targets)
    if isinstance(cmd, MakeComponentOfCommand):
        return _geometry_plan(cmd._nest, scene, targets)
    if isinstance(cmd, MakeNestedGroupCommand) and cmd._inner is not None:
        return _geometry_plan(cmd._inner, scene, targets)
    if isinstance(cmd, MeshCommand):
        from core.store_mesh import StoreMesh
        meshes = cmd.geometry_meshes(scene)
        stores = [m for m in meshes if type(m) is StoreMesh]
        if stores:
            targets.append((cmd, stores))
        return len(stores) == len(meshes)
    return _uses_doc_undo(cmd, scene)


def _uses_doc_undo(cmd, scene) -> bool:
    if getattr(cmd, "_journal_geometry", False):
        return True
    flag = getattr(cmd, "doc_undo", False)
    if callable(flag):
        try:
            return bool(flag(scene))
        except Exception:  # noqa: BLE001 — undecided: its own undo
            return False
    return bool(flag)


# ---- Concrete commands ------------------------------------------------------

def _dirty_group_chunks(scene) -> None:
    """Attr edits bypass the mesh mutation primitives — flag every group
    mesh so the viewport's cached chunks re-validate their materials.

    Two flags, because they answer different questions. ``_chunk_dirty`` is
    "something touched this mesh"; ``_attrs_dirty`` is "a MATERIAL changed",
    and only the second can be told apart from a move. The viewport's
    translation fast path reuses a cached chunk's texture buckets after
    checking geometry alone, so without this a repaint that happened in the
    same breath as a drag stayed invisible outside the group — the flagstone
    corner of Plaza Yanque kept its neighbour's grey (Marco, 2026-09-10).

    Nested placements are flagged too: their meshes render as part of the
    parent, and walking scene.groups as a flat list never reached them.
    """
    from core.group import iter_placements
    # The mesh being edited changed too (its faces' paint or tags), so its
    # revision moves; not mark_modified, which also bumps the mutation
    # serial and the winding count the viewport keys its geometry caches
    # on, and a paint would have re-walked every coordinate.
    scene.mesh.bump_revision()
    for g in getattr(scene, "groups", []):
        for pg, _m in iter_placements(g):
            pg.mesh._chunk_dirty = True
            pg.mesh._attrs_dirty = True


def _dirty_placements(groups) -> None:
    """:func:`_dirty_group_chunks` for the placements ``groups`` and their
    subtrees only: what History does after a step from the placements its
    transaction changed (the tag, Hide, paint: what a chunk bakes), instead
    of every command flagging every mesh — Hide on 200 placements of a
    20,000-placement file walked all of them (≈11 ms)."""
    from core.purge import iter_groups
    for g in iter_groups(groups):
        mesh = getattr(g, "mesh", None)
        if mesh is not None:
            mesh.invalidate_placement_chunks()


#: The placement fields a viewport chunk depends on (see _dirty_placements).
_RENDER_FIELDS = ["layer", "hidden", "material"]


def _unselect(scene, items) -> None:
    """``scene.selection.discard(e)`` for every item: from the selection's
    side when it is the smaller (Lock or Hide of 20,000 placements with a
    handful selected need not look each one up)."""
    sel = scene.selection
    if not sel:
        return
    if len(sel) * 4 < len(items):
        from core.mesh import element_identity
        drop = {element_identity(e) for e in items}
        gone = [e for e in sel if element_identity(e) in drop]
    else:
        gone = items
    for e in gone:
        sel.discard(e)


def _reverse_faces(faces):
    batches = {}
    for face in faces:
        batches.setdefault(face._m, []).append(face.key)
    for mesh, keys in batches.items():
        mesh._store.reverse_faces(keys)
    return batches


def _is_hidden(entity) -> bool:
    attrs = getattr(entity, "attrs", None)
    if attrs is not None:                          # Face
        return bool(attrs.get("hidden"))
    return bool(getattr(entity, "hidden", False))


def _set_hidden(entity, hidden: bool) -> None:
    attrs = getattr(entity, "attrs", None)
    if attrs is not None:                          # Face
        if hidden:
            attrs["hidden"] = True
        else:
            attrs.pop("hidden", None)
    else:
        entity.hidden = hidden


def _member_placement(scene, group):
    """A classic member initially draws at identity in its parent's definition."""
    if (group.xform is None and getattr(scene.mesh, "_instance_definition", None) is not None
            and scene.edit_group is not None and group in scene.edit_group.children):
        from ingetrazo_core import Matrix4
        group.component = False
        return Matrix4()
    return group.xform


def _edit_local_transform(scene, group, transform):
    if scene.edit_group is not None and group in scene.edit_group.children:
        view = scene.mesh.__dict__.get("_instance_definition")
        if view is not None:
            parent = scene.mesh._store.tool_placement()
            inverse, ok = parent.inverted()
            if ok:
                return inverse * transform * parent
    return transform


def run_stitch(mesh, seedkeys: set, new_faces: Optional[set] = None,
               coplanar_merge: bool = True, dedupe: bool = True) -> None:
    """Three-phase watertight cleanup (no undo bookkeeping — the caller snapshots).
    See :class:`StitchSolidCommand` for the rationale of each phase.

    ``seedkeys`` are the operation's own positions (``core.topology._key``
    tuples). They bound phase 1 — only an edge reaching that box can be split,
    which is every T-junction the op can have created and none of the ones it
    found lying around — and seed phase 3.

    ``new_faces`` (when given) are the faces this operation created; phase 3 only
    fuses a coplanar component that contains one of them, so a seam a push just
    made is merged while a pre-existing coplanar split (a user's diagonal) is
    left intact. ``None`` means merge any seeded component (manual stitch).

    ``coplanar_merge=False`` runs only phases 0–2 (connectivity repair). Solid
    push/pull uses this: its seams are dissolved by the deterministic per-plane
    rebuild (``Store::rebuild_push_planes``) instead of the winding-tolerant merge,
    which only remains for raw/open geometry where outwardness is undefined.

    ``dedupe=False`` skips the identical-cycle face dedupe of phase 0. The
    solid path's *first* stitch needs that: a flush-collapse sweep quad lands
    identical to the face it must annihilate *with*, and the per-plane rebuild
    is what decides whether the pair means "keep one" (a shared wall built
    twice — material on both sides) or "drop both" (an emptied region) — by
    parity, where the pair's two crossings cancel. Deduping it early restores
    the material reading and the collapse never classifies."""
    mesh.stitch_connect(seedkeys, dedupe)
    if not coplanar_merge:
        return
    # Phase 3 — coplanar-merge, seeded to the operation. Fuses the whole coplanar
    # component (every shared edge, any number of faces) at once, but only when it
    # includes a face this operation created (so user diagonals survive).
    from core.store_mesh import FaceView, _hypot
    start = 0
    store = mesh._store
    while True:
        fresh = None if new_faces is None else {f.key for f in new_faces}
        match = store.stitch_region(list(seedkeys or ()), fresh, start, _hypot())
        if match is None:
            return
        start, keys = match
        component = {FaceView(mesh, k) for k in keys}
        region = mesh.dissolve_coplanar_region(component)
        if region is not None:
            if new_faces is not None:
                new_faces.difference_update(component)
                new_faces.add(region)
            start = 0


class SnapshotMutation(MeshCommand):
    """Wrap an arbitrary mesh mutation with snapshot undo. The push/pull live
    preview applies the *same* mutation each drag frame (then reverts via its own
    snapshot), so the forming solid renders exactly as it will commit — clean,
    already stitched — instead of flashing the pre-stitch seams.

    Redo restores the captured *result* rather than re-running the mutation: the
    closure usually closes over tool state (``base_face`` etc.) that is reset
    right after commit, so re-running it on redo would crash — and re-running the
    deterministic plane rebuild on stale state would be wasteful besides.

    ``mesh`` (when given) is the mesh to snapshot instead of the scene's loose
    one — a push/pull aimed at a Group edits that group's isolated mesh."""

    keeps_rows = True

    def __init__(self, mutate, mesh: Optional[Mesh] = None) -> None:
        self.mutate = mutate
        self._mesh = mesh
        self.before: Optional[dict] = None
        self.after: Optional[dict] = None

    @classmethod
    def from_states(cls, before: dict, after: dict,
                    mesh: Optional[Mesh] = None) -> "SnapshotMutation":
        """A mutation already applied to the model, given its two snapshots:
        ``do`` takes the redo branch (restores ``after``) and nothing is
        re-run. Push/Pull commits its staged preview this way — the model
        already shows the result."""
        cmd = cls(None, mesh=mesh)
        cmd.before, cmd.after = before, after
        return cmd

    def geometry_meshes(self, scene):
        return (self._target(scene),)

    def _target(self, scene) -> Mesh:
        return self._mesh if self._mesh is not None else scene.mesh

    def do(self, scene) -> None:
        mesh = self._target(scene)
        if self.after is None:
            self.before = self._direct_snapshot(mesh)
            self.mutate(scene)
            self.after = self._direct_snapshot(mesh)
        else:
            mesh.restore_state(self.after)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.before is not None:
            self._target(scene).restore_state(self.before)
            scene.version += 1


class SnapshotImport(Command):
    """Wrap a file import that may add loose geometry AND/OR reference groups
    (big DAE/OBJ models land as a Group). SnapshotMutation only snapshots the
    mesh, so a group added by the loader would survive undo — this also
    remembers and reverts the groups the import appended.

    Sheets too: an agent asked for "a new sheet" appends a composition
    through ``run_python``, and untracked it was no step at all — the
    bridge said nothing changed, and its undo took the user's previous
    edit instead (#694).

    And what the mutation removed: an extension that replaces its own
    container group, or a console line that deletes one, lost that group
    for good on undo, which only took the new one away."""

    # Scene list -> (what the mutation added, what it removed). The removed
    # entries are (index before the mutation, item), so undo puts each one
    # back where it was.
    _TRACKED = (
        ("groups", "added_groups", "removed_groups"),
        ("layers", "added_layers", "removed_layers"),
        ("saved_views", "added_views", "removed_views"),
        ("dimensions", "added_dims", "removed_dims"),
        ("text_labels", "added_texts", "removed_texts"),
        ("compositions", "added_sheets", "removed_sheets"),
    )

    def __init__(self, mutate) -> None:
        self.mutate = mutate
        self.before: Optional[dict] = None
        self.after: Optional[dict] = None
        for _, added, removed in self._TRACKED:
            setattr(self, added, [])
            setattr(self, removed, [])

    @property
    def changed(self) -> bool:
        """Whether the run did anything undo would have to revert."""
        return (self.before != self.after
                or any(getattr(self, added) or getattr(self, removed)
                       for _, added, removed in self._TRACKED))

    def do(self, scene) -> None:
        m = scene.mesh
        if self.after is None:
            lists_before = {name: list(getattr(scene, name))
                            for name, _, _ in self._TRACKED}
            self.before = m.capture_state()
            try:
                self.mutate(scene)
            except BaseException:
                # Restore membership and order, not only newly appended items:
                # failed recipes may also remove or reorder existing objects.
                # Slice assignment preserves references held by scripting scopes.
                for name, items in lists_before.items():
                    getattr(scene, name)[:] = items
                raise
            self.after = m.capture_state()
            for name, added, removed in self._TRACKED:
                before, now = lists_before[name], getattr(scene, name)
                setattr(self, added, [x for x in now if x not in before])
                setattr(self, removed, [(i, x) for i, x in enumerate(before)
                                        if x not in now])
        else:
            m.restore_state(self.after)
            for name, added, removed in self._TRACKED:
                items = getattr(scene, name)
                for _, x in getattr(self, removed):
                    if x in items:
                        items.remove(x)
                    if name == "groups":
                        scene.selection.discard(x)
                for x in getattr(self, added):
                    if x not in items:
                        items.append(x)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.before is not None:
            scene.mesh.restore_state(self.before)
        for name, added, removed in self._TRACKED:
            items = getattr(scene, name)
            for x in getattr(self, added):
                if x in items:
                    items.remove(x)
                if name == "groups":
                    scene.selection.discard(x)
            # Ascending order of the old index, so each insert lands where
            # the item was once the ones before it are back.
            for i, x in getattr(self, removed):
                if x not in items:
                    items.insert(min(i, len(items)), x)
        scene.version += 1


def _run_snapshot_parts(parent, scene):
    # These drawing operations edit only the mesh already enlisted by the
    # enclosing snapshot. Other parts can have additional geometry owners.
    covered = (AddEdgeCommand, DeleteEdgesCommand, DeleteFaceCommand, TagCurveCommand,
               ExplodeCurvesCommand, PruneOrphanEdgesCommand, MoveVerticesCommand)
    for part in parent.inner:
        # Direct face undo also uses snapshots. The enclosing
        # command already owns them in either document mode.
        if type(part) is not AddFaceCommand and (
                not parent._journal_geometry or type(part) not in covered):
            part.do(scene)
            continue
        previous = getattr(part, "_journal_geometry", False)
        part._journal_geometry = True
        try:
            part.do(scene)
        finally:
            part._journal_geometry = previous


class SnapshotCompound(MeshCommand):
    """Run a list of sub-commands under one identity-preserving snapshot, undone
    by restoring it.

    The line-draw plan splits and welds edges (and punches holes in coplanar
    faces); the per-command inverses don't compose into a clean whole — undoing
    them piecemeal leaves orphan split edges and stray vertices behind. One
    snapshot of the entire edit reverses it exactly, and because identity is
    preserved, earlier history entries keep working across this undo.
    ``heal=False`` preserves supplied copy topology instead of applying
    drawing-specific global face cleanup.
    """

    keeps_rows = True

    def __init__(self, inner: Iterable[Command], *, heal: bool = True) -> None:
        self.inner = list(inner)
        self._heal = heal
        self.before: Optional[dict] = None
        self.after: Optional[dict] = None

    def do(self, scene, *, transaction_captured_before=False) -> None:
        if self.after is None:
            # First run: snapshot before, apply the plan, clean up any coplanar
            # overlap it created (redundant nested holes / spurious mother), then
            # snapshot the result so undo/redo restore exactly.
            if not transaction_captured_before:
                self.before = self._direct_snapshot(scene.mesh)
            # Which way every face looked before the draw: the heal may
            # align the faces the draw MADE, never the ones already there
            # (a Reverse Faces undone by the next rectangle).
            footprint = winding_footprint(scene.mesh) if self._heal else None
            _run_snapshot_parts(self, scene)
            if self._heal:
                for f in heal_overlapping_faces(scene.mesh, footprint=footprint):
                    scene.selection.discard(f)
            # A draw that split a curve leaves it in separate contours — break
            # the curve ids there, before the snapshot so redo keeps it.
            scene.mesh.resplit_curves()
            self.after = self._direct_snapshot(scene.mesh)
        else:
            # Redo: re-running the delta plan wouldn't reproduce the splits, so
            # restore the captured result directly.
            scene.mesh.restore_state(self.after)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.before is not None:
            scene.mesh.restore_state(self.before)
            scene.version += 1


class MeshSnapshotCommand(MeshCommand):
    """Run a list of sub-commands plus the stitch pass under a single
    identity-preserving snapshot, undone by restoring that snapshot.

    Push/pull builds its edit as delta commands and then stitches the result
    watertight; the stitch's splits/merges restructure the very edges those
    commands own, so composing their individual undos leaves orphan edges. One
    snapshot of the whole push is exact and robust — and because it preserves
    object identity, *other* history entries (a drawn line, an earlier push) keep
    working across this undo."""

    def __init__(self, inner: Iterable[Command], stitch_seed: Iterable[QVector3D]) -> None:
        self.inner = list(inner)
        self.seed = [QVector3D(p) for p in stitch_seed]
        self.snapshot: Optional[dict] = None

    def do(self, scene) -> None:
        self.snapshot = self._direct_snapshot(scene.mesh)
        before = set(scene.mesh.faces)
        _run_snapshot_parts(self, scene)
        new_faces = set(scene.mesh.faces) - before
        run_stitch(scene.mesh, {_key(p) for p in self.seed}, new_faces)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.snapshot is not None:
            scene.mesh.restore_state(self.snapshot)
            scene.version += 1


def _move_holds(scene, placements, moved) -> None:
    """The dimensions held on a vertex inside ``placements`` hold the
    same vertex in its copy now: ``moved`` maps ``id`` of each part and
    mesh replaced to its replacement (UnlinkCommand, either way). Their
    holds would otherwise stay on the parts no longer placed — still
    reading as alive through the placement at the top of their chain."""
    dims = getattr(scene, "dimensions", None) or ()
    if not dims or not moved:
        return
    from core.dimension import VertexAnchor
    roots = {id(g) for g in placements}

    def remap(h):
        if h is None or not any(id(g) in roots or id(g) in moved
                                for g in h.chain):
            return h
        mesh = moved.get(id(h.mesh), h.mesh)
        chain = tuple(moved.get(id(g), g) for g in h.chain)
        if mesh is h.mesh:
            return VertexAnchor(h.vertex, mesh, chain)
        try:
            v = mesh.vertex_at(QVector3D(h.vertex.position))
        except Exception:  # noqa: BLE001 — a mesh that cannot answer
            v = None
        return VertexAnchor(v, mesh, chain) if v is not None else None

    for dim in dims:
        for which in ("a", "b"):
            name = "anchor_" + which
            if getattr(dim, name, None) is not None:
                setattr(dim, name, remap(getattr(dim, name)))
            holds = getattr(dim, "_holds", None)
            if holds and holds.get(which):
                holds[which] = [h for h in map(remap, holds[which])
                                if h is not None]


class _TagState:
    """What the tags tree, the link registry and the materials were at one
    moment (UnlinkCommand): undo and redo go from one such state to the
    other by changing only what differs between them, so a tag made or
    renamed in the Layers panel in between (not on the undo stack) is
    left alone."""

    _MAPS = ("folder_parents", "folder_visibility", "layer_folder_names",
             "layer_folder_uids")

    def __init__(self, scene) -> None:
        from copy import deepcopy
        from core.layers import folders_of, layer_values
        self.layers = list(scene.layers)
        self.attrs = {id(ly): layer_values(ly) for ly in self.layers}
        self.folders = list(folders_of(scene))
        self.maps = {name: dict(getattr(scene, name, None) or {})
                     for name in self._MAPS}
        self.registry = deepcopy(scene.link_registry)
        self.cache = dict(scene.__dict__.get("_link_tag_tables") or {})
        self.materials = dict(scene.materials)
        self.active = getattr(scene, "active_layer", None)

    @staticmethod
    def _sync(now: dict, old: dict, new: dict, copy=lambda v: v) -> None:
        for key in old.keys() | new.keys():
            a, b = old.get(key, _MISSING), new.get(key, _MISSING)
            if a is b or (a is not _MISSING and b is not _MISSING and a == b):
                continue
            if b is _MISSING:
                now.pop(key, None)
            else:
                now[key] = copy(b)

    def go(self, scene, to: "_TagState") -> None:
        """Change ``scene`` from this state to ``to``."""
        from copy import deepcopy
        old_ids = {id(ly) for ly in self.layers}
        new_ids = {id(ly) for ly in to.layers}
        scene.layers[:] = [ly for ly in scene.layers
                           if id(ly) in new_ids or id(ly) not in old_ids]
        have = {id(ly) for ly in scene.layers}
        for i, ly in enumerate(to.layers):
            if id(ly) not in have:
                scene.layers.insert(min(i, len(scene.layers)), ly)
        for ly in to.layers:
            a, b = self.attrs.get(id(ly), {}), to.attrs[id(ly)]
            for k, v in b.items():
                if a.get(k, _MISSING) != v:
                    setattr(ly, k, v)
        folders = [f for f in (scene.layer_folders or ())
                   if f in to.folders or f not in self.folders]
        for i, f in enumerate(to.folders):
            if f not in folders:
                folders.insert(min(i, len(folders)), f)
        maps = {}
        for name in self._MAPS:
            now = dict(getattr(scene, name, None) or {})
            self._sync(now, self.maps[name], to.maps[name])
            maps[name] = now
        for name, now in maps.items():
            setattr(scene, name, now)
        scene.layer_folders = folders
        self._sync(scene.link_registry, self.registry, to.registry, deepcopy)
        self._sync(scene.__dict__.setdefault("_link_tag_tables", {}),
                   self.cache, to.cache)
        self._sync(scene.materials, self.materials, to.materials)
        if self.active != to.active:
            scene.active_layer = to.active


_MISSING = object()


def _discard_removed_selection(scene):
    for element in tuple(scene.selection):
        owner = getattr(element, "_m", None)
        store = getattr(owner, "_store", None)
        if store is not None and not store.is_live(element.key):
            scene.selection.discard(element)


def _drop_dead_elements(scene, mesh) -> None:
    """Keep in the selection only the faces and edges still in ``mesh``
    (answered by the store for a store-backed mesh: no view per face)."""
    scene.selection = {s for s in ElementSet(scene.selection)
                       if not isinstance(s, (Face, Edge)) or mesh._store.is_live(s.key)}


def _merge_overlapping_planes(scene, before, added, *, on_rebuild=None,
                              others=False) -> int:
    """Rebuild, plane by plane, where faces just ``added`` to the mesh
    overlap faces it already had (``before``): each region of the plane
    ends up covered by ONE face, as when a face is drawn or pasted there
    (:class:`RebuildPlaneFacesCommand`, scoped to the faces involved).
    Faces that only share an edge or a corner are left alone, and so is
    every plane no older face overlaps. Runs inside a command that already
    holds a snapshot of the mesh, so it takes none. Returns how many planes
    were rebuilt. ``on_rebuild`` lets a caller take its snapshot lazily,
    immediately before the first mutation, only if a rebuild is needed.

    ``others=True``: ``before`` is every face of the mesh but ``added``
    (pass None). A store-backed mesh then finds the face pairs whose boxes
    overlap natively (``MeshStore.plane_merge_candidates``) and the plane
    tests below run on those pairs only, in the same order.
    """
    if not added:
        return 0
    mesh = scene.mesh
    tol = RebuildPlaneFacesCommand._TOL
    found = None
    if others:
        from core.store_parts import merge_candidates
        found = merge_candidates(mesh, added, tol)
        if found is None:
            ids = set(map(element_identity, added))
            before = [f for f in mesh.faces if f.key not in ids]
    if found is None:
        if not before:
            return 0
        live = set(map(element_identity, mesh.faces))
        added = [f for f in added if f.key in live]
        if not added:
            return 0

    def box(f):
        ps = f.vertices
        xs = [p.x() for p in ps]
        ys = [p.y() for p in ps]
        zs = [p.z() for p in ps]
        return (min(xs), min(ys), min(zs)), (max(xs), max(ys), max(zs))

    def unit_normal(f):
        n = f._newell()
        length = n.length()
        return n / length if length > 1e-9 else None

    def overlap(alo, ahi, blo, bhi) -> bool:
        # The _plane_merges test of Paste: the boxes overlap with some area
        # along each axis, or are both flat along it.
        return all(alo[i] < bhi[i] - tol and blo[i] < ahi[i] - tol
                   or abs(ahi[i] - alo[i]) < tol and abs(bhi[i] - blo[i]) < tol
                   for i in range(3))

    if found is not None:
        new_faces, pairs = found
        entries: dict = {}

        def entry(j):
            e = entries.get(j)
            if e is None:
                nf = new_faces[j]
                e = entries[j] = (nf, unit_normal(nf), *box(nf))
            return e

        def candidates():
            for f, hits in pairs:
                yield f, (entry(j) for j in hits)
    else:
        new = []
        glo = [float("inf")] * 3
        ghi = [float("-inf")] * 3
        for f in added:
            n = unit_normal(f)
            if n is None:
                continue
            lo, hi = box(f)
            new.append((f, n, lo, hi))
            glo = [min(a, b) for a, b in zip(glo, lo)]
            ghi = [max(a, b) for a, b in zip(ghi, hi)]
        if not new:
            return 0
        added_ids = set(map(element_identity, added))

        def candidates():
            for f in before:
                if f.key not in live or f.key in added_ids:
                    continue
                lo, hi = box(f)
                if not all(lo[i] <= ghi[i] + tol and glo[i] <= hi[i] + tol
                           for i in range(3)):
                    continue
                yield f, (e for e in new if overlap(lo, hi, e[2], e[3]))

    planes: list = []          # [origin, normal, [old faces], [new faces]]
    for f, near in candidates():
        n = None
        for nf, nn, nlo, nhi in near:
            if nn is None:
                continue
            if n is None:
                n = unit_normal(f)
                if n is None:
                    break
            if abs(QVector3D.dotProduct(n, nn)) < 1.0 - 1e-6:
                continue
            o = nf.vertices[0]
            if any(abs(QVector3D.dotProduct(p - o, nn)) > tol
                   for p in f.vertices):
                continue
            for entry_ in planes:
                if (abs(QVector3D.dotProduct(entry_[1], nn)) > 1.0 - 1e-6
                        and abs(QVector3D.dotProduct(o - entry_[0],
                                                     entry_[1])) < tol):
                    break
            else:
                entry_ = [QVector3D(o), QVector3D(nn), [], []]
                planes.append(entry_)
            if f not in entry_[2]:
                entry_[2].append(f)
            if nf not in entry_[3]:
                entry_[3].append(nf)
    if not planes:
        return 0
    if on_rebuild is not None:
        on_rebuild()
    keep = ElementSet(scene.selection)
    for origin, n, old_faces, new_faces in planes:
        # The new faces on that plane that overlap NO older face stay as
        # they are; the rest rebuild with the older faces they cover.
        cmd = RebuildPlaneFacesCommand(origin, n,
                                       faces=old_faces + new_faces)
        cmd.take_snapshot = False
        cmd.do(scene)
    scene.selection = set(keep)
    _drop_dead_elements(scene, mesh)
    return len(planes)


def placement_is_identity(xform, tol: float = 1e-6) -> bool:
    """True when a placement neither turns nor moves anything (within
    ``tol``) — straightening it would be a silent no-op."""
    if xform is None:
        return True
    from ingetrazo_core import Matrix4 as QMatrix4x4
    ident = QMatrix4x4()
    return all(abs(a - b) <= tol for a, b in zip(xform.data(), ident.data()))


def placement_rotation_deg(xform) -> float:
    """Turn about Z (degrees, counter-clockwise seen from above) of a rigid
    placement; raises ``ValueError`` when the placement tilts or scales —
    there is no north angle for those."""
    import math
    c0, c1, c2 = xform.column(0), xform.column(1), xform.column(2)
    if (abs(c2.x()) > 1e-6 or abs(c2.y()) > 1e-6 or abs(c2.z() - 1.0) > 1e-6
            or abs(c0.z()) > 1e-6 or abs(c1.z()) > 1e-6):
        raise ValueError("the model is tilted (its Z axis is not vertical)")
    sx = math.hypot(c0.x(), c0.y())
    sy = math.hypot(c1.x(), c1.y())
    if abs(sx - 1.0) > 1e-4 or abs(sy - 1.0) > 1e-4:
        raise ValueError("the model is scaled")
    return math.degrees(math.atan2(c0.y(), c0.x()))


def group_owner_list(scene, group):
    """The list ``group`` lives in: ``scene.groups`` at the root, or the
    ``children`` of the container that owns it, however deep. ``None`` when
    the group is not in the scene at all."""
    return _definitions.group_owner(scene, group)


_point_in_tri = _planar.point_in_tri
_segment_grid = _planar.SegmentIndex


class CompoundCommand(Command):
    """A list of commands executed and reverted as one atomic step."""

    journal_parts = False

    def __init__(self, commands: Iterable[Command]) -> None:
        self.commands: list[Command] = list(commands)

    @property
    def keeps_paint(self) -> bool:
        return all(getattr(c, "keeps_paint", False) for c in self.commands)

    @property
    def keeps_groups(self) -> bool:
        return all(getattr(c, "keeps_paint", False)
                   or getattr(c, "keeps_groups", False)
                   for c in self.commands)

    @property
    def keeps_rows(self) -> bool:
        return all(getattr(c, "keeps_paint", False)
                   or getattr(c, "keeps_rows", False)
                   for c in self.commands)

    def do(self, scene) -> None:
        done = []
        try:
            for cmd in self.commands:
                cmd.do(scene)
                done.append(cmd)
        except CommandRejected:
            # One part declined: the rest goes back too (a group moved
            # with the vertices lives outside the mesh snapshot).
            if not self._journal_geometry:
                for cmd in reversed(done):
                    cmd.undo(scene)
            raise

    def undo(self, scene) -> None:
        for cmd in reversed(self.commands):
            cmd.undo(scene)

    def doc_undo(self, scene) -> bool:
        """By its transaction when every part would be."""
        return bool(self.commands) and all(
            _uses_doc_undo(c, scene) for c in self.commands)

    def doc_after(self, scene, phase) -> None:
        parts = reversed(self.commands) if phase == "undo" else self.commands
        for cmd in parts:
            cmd.doc_after(scene, phase)


class OccurrenceCommands(CompoundCommand):
    """Replay committed commands in their original occurrence coordinate frames."""

    @staticmethod
    def group_start(history, session):
        return history._native.group_start(session)

    @staticmethod
    def fold(history, session):
        return history._native.fold_occurrence(session, OccurrenceCommands)

    @staticmethod
    def _in_frame(scene, cmd, action, *args):
        from contextlib import nullcontext
        from core.coordinate_frame import mesh_frame
        active = scene.mesh
        try:
            scene.mesh = getattr(cmd, "_history_mesh", active)
            frame = getattr(cmd, "_tool_frame", None)
            with mesh_frame(scene.mesh, frame[0], matrix=frame[1]) if frame else nullcontext():
                action(*args)
        finally:
            scene.mesh = active

    def doc_after(self, scene, phase):
        _doc.NATIVE.DocumentHistory.replay_group(self, scene, phase == "undo", True)

    def _replay(self, scene, undo):
        return _doc.NATIVE.DocumentHistory.replay_group(self, scene, undo, False)

    def do(self, scene):
        self._replay(scene, False)

    def undo(self, scene):
        self._replay(scene, True)


class SetRecordCommand(Command):
    """Top-level patch of group records; None patch removes the record.

    Nested values replace whole fields. GUIDs cannot be changed by a patch.
    Snapshots are detached and retained across redo, including new identities.
    """
    # History currently treats keeps_paint as also keeping BIM tags.
    keeps_paint = False
    #: Records are document values (``Group.ifc``, slice 2).
    doc_undo = True

    def doc_after(self, scene, phase) -> None:
        meshes = {id(scene.mesh): scene.mesh}
        for group in self.targets:
            meshes[id(group.mesh)] = group.mesh
        for mesh in meshes.values():
            mesh.bump_revision()
        scene.version += 1

    def __init__(self, targets, patch, *, normalize_values=True):
        self.normalize_values = normalize_values
        from copy import deepcopy
        self.targets = list(targets)
        self.patch = deepcopy(patch)
        self._before = None
        self._after = None

    def _write(self, scene, records):
        from copy import deepcopy
        meshes = {id(scene.mesh): scene.mesh}
        for group, record in zip(self.targets, records):
            group.ifc = deepcopy(record)
            meshes[id(group.mesh)] = group.mesh
        for mesh in meshes.values():
            mesh.bump_revision()
        scene.version += 1

    def do(self, scene):
        from copy import deepcopy
        from core.ifc_identity import ensure_guid
        from core.record import normalize
        if self._before is None:
            self._before = [deepcopy(g.ifc) for g in self.targets]
            self._after = []
            for old in self._before:
                if self.patch is None:
                    self._after.append(None)
                    continue
                new = {**deepcopy(old or {}), **deepcopy(self.patch)}
                if old and old.get('guid'):
                    new['guid'] = old['guid']
                else:
                    new.pop('guid', None)
                    ensure_guid(new)
                self._after.append(normalize(new) if self.normalize_values else new)
        self._write(scene, self._after)

    def undo(self, scene):
        self._write(scene, self._before)


class TagGroupsCommand(SetRecordCommand):
    def __init__(self, targets, ifc_class, name=''):
        super().__init__(targets, {'class': ifc_class, 'name': name})


class UntagGroupsCommand(SetRecordCommand):
    def __init__(self, targets):
        super().__init__(targets, None)


class SetRegistryCommand(SetPluginDataCommand):
    def __init__(self, value, *, refiled_uids=()):
        self.refiled_uids = frozenset(refiled_uids)
        from core.bim_registry import validate
        errors = validate(value)
        if errors:
            raise ValueError('; '.join(errors))
        super().__init__('bim_registry', value)

    def do(self, scene):
        from core import project, bim_registry
        stamp = getattr(scene, '_project_mtime_ns', None)
        # What the edit was derived from: the merge base of the shared write.
        base = getattr(scene, '_project_registry_base', None)
        current = bim_registry.get(scene)
        removed = (current.get('structure', {}).get('nodes', {}).keys()
                   - self.value.get('structure', {}).get('nodes', {}).keys())
        if removed:
            from core.structure import _placements, node_of
            for group, source, _ in _placements(scene):
                if not source and node_of(group) in removed and group.uid not in self.refiled_uids:
                    raise CommandRejected('Cannot delete occupied node: ' + node_of(group))
        if not hasattr(self, '_project_before'):
            self._project_before = current
        if stamp is not None:
            scene._project_mtime_ns = stamp
        if base is not None:
            scene._project_registry_base = base
        if project.write_registry(scene, self.value):
            scene.version += 1
        else:
            super().do(scene)
        scene.mesh.bump_revision()

    def undo(self, scene):
        from core import project
        # Revert only what this command changed (base: its after-value).
        if project.write_registry(scene, self._project_before, base=self.value):
            scene.version += 1
        else:
            super().undo(scene)
        scene.mesh.bump_revision()

    def doc_undo(self, scene) -> bool:
        # A project's registry lives in its project file, not the document.
        from core import project
        return project.path_for(scene) is None

    def doc_after(self, scene, phase) -> None:
        scene.mesh.bump_revision()
        scene.version += 1


class SetFaceTagsCommand(Command):
    """Legacy loose-face tags, with stable identities across redo."""
    # History currently treats keeps_paint as also keeping BIM tags.
    keeps_paint = False

    def __init__(self, faces, ifc_class=None, name=''):
        self.faces = list(faces)
        self.ifc_class, self.name = ifc_class, name
        self._before = self._after = None

    def _write(self, scene, values):
        from copy import deepcopy
        from core import bim
        for face, value in zip(self.faces, values):
            if value is None:
                face.attrs.pop('ifc', None)
            else:
                face.attrs['ifc'] = deepcopy(value)
        bim._TAG_EPOCH += 1
        scene.mesh.bump_revision()
        scene.version += 1

    def do(self, scene):
        from copy import deepcopy
        from core.bim import next_object_id
        from core.ifc_identity import ensure_guid
        if self._before is None:
            self._before = [deepcopy(f.attrs.get('ifc')) for f in self.faces]
            tag = None
            if self.ifc_class:
                tag = {'id': next_object_id(scene), 'class': self.ifc_class, 'name': self.name}
                ensure_guid(tag)
            self._after = [deepcopy(tag) for f in self.faces]
        self._write(scene, self._after)

    def undo(self, scene):
        self._write(scene, self._before)


# ---- History ------------------------------------------------------------------


_PERF = bool(_os.environ.get("INGETRAZO_PERF"))
_perf_file = None


def _plog(tag: str, ms: float, extra: str = "", floor: float = 100.0) -> None:
    """Same log the viewport telemetry writes (P0): any COMMAND slower than
    the floor lands here with its class name, so a sticky edit names its
    culprit without a profiler round-trip.

    Every line carries the writer's **pid**. One log file collects whatever is
    running — a second window opened to compare a fix, an offscreen repro
    script — and without it two builds' numbers interleave into one
    indistinguishable stream (which has already sent one reading the wrong
    way). ``$INGETRAZO_PERF_LOG`` overrides the path when a run wants its own
    file."""
    global _perf_file
    if not _PERF or ms < floor:
        return
    if _perf_file is None:
        path = _os.environ.get("INGETRAZO_PERF_LOG")
        _perf_file = open(_Path(path) if path
                          else _Path.home() / "ingetrazo-perf.log",
                          "a", buffering=1)
    _perf_file.write(f"{_time_mod.strftime('%H:%M:%S')} [{_os.getpid()}] "
                     f"{tag} {ms:.0f}ms"
                     f"{' ' + extra if extra else ''}\n")

class History:
    """Undo/redo stacks. ``execute`` is TRANSACTIONAL: if a command throws
    mid-mutation, the mesh is restored to its pre-command state, the failure
    is logged (stderr + ``error_log`` file), and nothing lands on the undo
    stack — a failed operation must be a no-op, never a half-committed mess
    (an aas.igz-style aborted draw left a quarter circle, an unsplit face and
    a duplicated edge behind, with the traceback swallowed by the Qt event
    loop). Same fail-safe doctrine as the BIM push guard, one level up."""

    #: Where failed-command tracebacks are appended: the user's log folder
    #: (core.paths.user_log_dir — writable even when the app is installed
    #: under Program Files), so the user can just send the file.
    error_log = "ingetrazo-errors.log"

    @staticmethod
    def default_error_log() -> str:
        from core.paths import user_log_dir
        return str(user_log_dir() / "ingetrazo-errors.log")

    #: How many steps back the history keeps; the oldest fall off the
    #: front. Every command carries a mesh snapshot, so on a big model the
    #: stack is where the memory goes (issue #56, @pacaeiro: «an option to
    #: control number of Undo Steps, start thinking in ways of improving
    #: performance»). Preferences ▸ General sets it; 0 = unlimited.
    max_steps: int = 200

    def __init__(self, scene) -> None:
        self.scene = scene
        self._listeners = []
        self._running = False
        self._native = DocumentHistory()
        self.undo_stack = self._native.undo
        self.redo_stack = self._native.redo
        #: Message describing the last rolled-back failure (UI may flash it).
        self.last_error: Optional[str] = None
        #: Why the last command declined to happen (:class:`CommandRejected`).
        self.last_rejected: Optional[str] = None
        #: The document transaction of the last step that ran (execute,
        #: undo or redo) and whether it is read backwards: ``(txn,
        #: inverse)``, what the feed takes the document half of a step's
        #: record from.
        self.last_txn = None
        #: The change feed (core/protocol, docs/core-protocol.md): one
        #: record per step by stable keys. Subscribed first, so a step's
        #: record is there for every other listener's ``after``.
        from core.protocol.feed import ChangeFeed
        self.feed = ChangeFeed(self)

    def add_listener(self, *, before=None, after=None):
        """Subscribe callback(command, phase, ran); return an unsubscribe function.

        Phase is execute, undo or redo. Before receives ran=False; after
        receives True only on success. Callbacks run in registration order.
        """
        entry = (before, after)
        self._listeners.append(entry)

        def unsubscribe():
            for i, item in enumerate(self._listeners):
                if item is entry:
                    del self._listeners[i]
                    break
        return unsubscribe

    def _notify(self, listeners, index, cmd, phase, ran):
        from core.coordinate_frame import model_frame
        # Feed subscribers may read any placement synchronously during a click.
        with model_frame():
            for entry in listeners:
                fn = entry[index]
                if fn is not None:
                    try:
                        fn(cmd, phase, ran)
                    except Exception:
                        logging.getLogger("ingetrazo.history").exception(
                            "Command listener failed (%s)", phase)

    @property
    def revision(self):
        return self._native.revision

    def changes_since(self, revision=0):
        """Bounded command journal; not a geometric diff or a persisted log."""
        if type(revision) is not int or revision < 0 or revision > self.revision:
            raise ValueError("revision must be between zero and the current revision")
        return self._native.changes_since(revision)

    def _run(self, cmd, phase, operation):
        if getattr(self.scene, 'worksession_read_only', False):
            self.last_rejected = 'This project file is read-only here: make it the file you edit first.'
            return False
        target = getattr(cmd, "_history_mesh", None) or self.scene.mesh
        st = target.__dict__.get("_store")
        if st is not None:
            matrix = st.frame_matrix
            current = (tuple(st.frame_origin), tuple(matrix) if matrix is not None else None)
            if phase == "execute" and not st._model_frame:
                cmd._tool_frame = current
            frame = getattr(cmd, "_tool_frame", None)
            if frame is not None and current != frame:
                from core.coordinate_frame import mesh_frame
                with mesh_frame(target, frame[0], matrix=frame[1]):
                    return self._run(cmd, phase, operation)
        nested = self._running
        if not nested:
            self._moved_signs = []
        listeners = tuple(self._listeners)
        self._notify(listeners, 0, cmd, phase, False)
        self._running = True
        ran = False
        try:
            result = operation(nested)
            ran = (not self.last_error and not self.last_rejected
                   if phase == "execute" else bool(result))
            if ran and not nested:
                name = getattr(cmd, "history_name", type(cmd).__name__)
                self._native.advance(phase, name)
            return result
        finally:
            self._running = nested
            self._notify(listeners, 1, cmd, phase, ran)

    def execute(self, cmd: Command) -> None:
        return self._run(cmd, "execute", lambda nested: self._execute(cmd, nested))

    def undo(self) -> bool:
        if not self.undo_stack:
            return False
        return self._run(self.undo_stack[-1], "undo", self._undo)

    def redo(self) -> bool:
        if not self.redo_stack:
            return False
        return self._run(self.redo_stack[-1], "redo", self._redo)

    def _keep_geometry_caches(self, cmd, before) -> None:
        cv = getattr(self.scene, "content_version", None)
        # Layer counts and dimension anchors can reuse their placement scans
        # after edits confined to the active mesh or construction annotations.
        if isinstance(cmd, (
                SnapshotMutation, SnapshotCompound, AddEdgeCommand, AddFaceCommand,
                AddDimensionCommand, AddTextLabelCommand, AddGuideCommand)):
            rest = getattr(self, "layer_count_rest", None)
            if (rest and rest[0] == (id(self.scene), before, id(self.scene.mesh))
                    and getattr(cmd, "_history_mesh", None) is self.scene.mesh
                    and (not isinstance(cmd, SnapshotMutation) or cmd._target(self.scene) is self.scene.mesh)):
                self.layer_count_rest = (id(self.scene), cv, id(self.scene.mesh)), rest[1]
            # Placement transforms did not change. Prototype bounds in the
            # dimension broad phase are checked by mesh revision on lookup.
            anchors = self.scene.__dict__.get("_anchor_placements")
            if anchors and anchors[0] == (before, len(self.scene.groups),
                                          id(self.scene.edit_group), id(self.scene.mesh)):
                self.scene._anchor_placements = (cv, *anchors[0][1:]), anchors[1]

    def context_changed(self, before) -> None:
        if not isinstance(before, tuple):
            return  # A caller without a pre-transition serial needs a fresh scan.
        version, serial = before
        if version is not None:
            self.feed.readers.context_changed(version, serial, self.scene.content_version)

    def _execute(self, cmd: Command, nested=False) -> None:
        _t0 = _time_mod.perf_counter() if _PERF else 0.0
        self.last_rejected = None
        before = getattr(self.scene, "content_version", None)
        # Geometry-only commands cannot create groups, even on an active tag.
        include_groups = not cmd.keeps_rows
        objs_before = self._objects_before(include_groups=include_groups)
        self.last_txn = None

        def tag_objects():
            self._tag_new_objects(objs_before, include_groups=include_groups)

        try:
            self._native.execute(self, cmd, tag_objects)
        except Exception as exc:
            self.scene.version += 1
            if isinstance(exc, CommandRejected):
                self.last_error = None
                self.last_rejected = str(exc)
            else:
                self.scene.selection.clear()
                self.last_error = f"{type(cmd).__name__}: {exc}"
                self._log_failure(cmd, exc)
            return
        self._dirty_from_txn()
        self.last_error = None
        if _PERF:
            _plog("command", (_time_mod.perf_counter() - _t0) * 1000.0,
                  extra=type(cmd).__name__)
        # Remember which mesh was active (the loose one, or a group being
        # edited): undo/redo re-enter that context so a snapshot restore never
        # lands on the wrong mesh after the user exits the group.
        cmd._history_mesh = self.scene.mesh
        if self.scene.edit_group is not None:
            cmd._occurrences = tuple(level["session"] for level in self.scene._edit_stack)
        self._keep_geometry_caches(cmd, before)
        if nested:
            return
        cap = max(0, int(self.max_steps or 0))
        self._native.push(cmd, cap)
        self._rebind_dimensions()

    def _follow_text_placements(self):
        anchors = getattr(self.scene, "text_placement_anchors", None)
        if anchors:
            from core.coordinate_frame import model_frame
            with model_frame():
                for anchor in anchors:
                    if anchor.follow(self.scene):
                        self._moved_signs.append(anchor.path[-1])

    def _rebind_dimensions(self) -> None:
        """After every command, undo and redo: a dimension whose vertex
        changed meshes (Make Group, Explode, a component conversion) takes
        the vertex now standing where its endpoint was — BEFORE anything
        else moves it. Leaving it to the next read was too late when a
        scale followed in the same breath. Cheap: a hold that is alive is
        a dictionary lookup and an identity test."""
        for dim in getattr(self.scene, "dimensions", None) or ():
            refresh = getattr(dim, "refresh_anchors", None)
            if refresh is not None:
                refresh(self.scene)

    def _log_failure(self, cmd: Command, exc: Exception) -> None:
        import datetime
        import sys
        import traceback

        text = (f"[{datetime.datetime.now().isoformat(timespec='seconds')}] "
                f"command {type(cmd).__name__} failed and was rolled back:\n"
                f"{''.join(traceback.format_exception(exc))}\n")
        if sys.stderr is not None:
            print(text, file=sys.stderr)
        path = self.error_log
        if path == History.error_log:            # the default: user log dir
            try:
                path = self.default_error_log()
            except Exception:  # noqa: BLE001 — logging never breaks a command
                path = self.error_log
        try:
            with open(path, "a", encoding="utf-8") as fh:
                fh.write(text)
        except OSError:
            pass

    def _undo(self, nested=False):
        return self._replay(True)

    def _redo(self, nested=False):
        return self._replay(False)

    def _replay(self, undo):
        source = self.undo_stack if undo else self.redo_stack
        before = getattr(self.scene, "content_version", None)
        cmd = source[-1]
        self._native.replay(self, undo, False)
        self._dirty_from_txn()
        self._keep_geometry_caches(cmd, before)
        self._rebind_dimensions()
        return True

    def _dirty_from_txn(self) -> None:
        """The renderer's dirtiness from the step's transaction: the meshes
        of the placements whose tag, Hide or paint it changed, and of their
        subtrees, re-validate their chunks (docs/rust-document-model.md
        §5). Commands that touch only placements no longer flag every mesh
        themselves."""
        rec = self.last_txn
        if rec is None or rec[0] is None:
            return
        touched = rec[0].placements(_RENDER_FIELDS)
        if touched:
            _dirty_placements(touched)

    def _objects_before(self, *, include_groups=True):
        """The objects in the document before a command runs. With no
        layer active only the guides, dimensions and texts (they go to
        their automatic layers), so the default case never walks the
        groups."""
        from core.layers import (DEFAULT_LAYER, active_layer, auto_ids,
                                 object_ids)
        if active_layer(self.scene) == DEFAULT_LAYER:
            return auto_ids(self.scene)
        return object_ids(self.scene, include_groups=include_groups)

    def _tag_new_objects(self, before, *, include_groups=True) -> None:
        """The objects a command made go onto the active layer
        (core.layers) — groups, components, dimensions and texts; loose
        lines and faces stay on the default layer, so hiding a layer never
        leaves half a solid. Done here, after the command, so every tool
        gets it without knowing about layers."""
        if before is None:
            return
        from core.layers import tag_new_objects
        from core.purge import iter_groups
        tagged = []
        if tag_new_objects(self.scene, before, include_groups=include_groups, tagged_groups=tagged):
            # Only these new placements now inherit a different tag. Dirtying
            # all meshes turns adding one part into a full Metal scene bake.
            for group in iter_groups(tagged):
                group.mesh._chunk_dirty = True
                group.mesh._attrs_dirty = True
            self.scene.version += 1

    def _in_command_mesh(self, cmd, fn, *args) -> None:
        """Run ``fn`` with ``scene.mesh`` pointing at the mesh the command was
        executed against (group-edit context correctness)."""
        target = getattr(cmd, "_history_mesh", None)
        if target is None or target is self.scene.mesh:
            fn(*args)
            return
        # A closed mesh changes: an attrs swap (a paint undone after leaving
        # its group) bumps no serial by itself, and the viewport's placement
        # caches would keep drawing the old bake.
        target._mut_serial += 1
        current = self.scene.mesh
        self.scene.mesh = target
        try:
            fn(*args)
        finally:
            self.scene.mesh = current

    def clear(self) -> None:
        self._native.clear_stacks()

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Document History and the editor and extension command API."""
from __future__ import annotations

from abc import ABC, abstractmethod
from typing import Iterable, Optional

from ingetrazo_core import placement_is_identity, placement_rotation_deg, run_stitch
from ingetrazo_core import definition_edits
from ingetrazo_core import FaceTagsStep, PluginDataStep, RecordPatchStep, registry_apply
from ingetrazo_core import CompoundStep, DocumentHistory, DrawingSnapshotStep, ImportStep, MutationStep, StitchSnapshotStep

# ---- The editor and extension command API ------------------------------------
# The undoable commands are native ingetrazo_core steps (docs/commands-port.md).
# What stays here is the plugin command API (docs/plugins.md), the snapshot
# and compound constructors, and the History API.

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

from core.mesh import Mesh

group_owner_list = definition_edits.group_owner


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

    def __getstate__(self):
        # Snapshot and compound native bases have no instance state. Plugins
        # keep their state in Python attributes and may also declare slots.
        return super().__getstate__()

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
    and the document field journal."""
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


class SetPluginDataCommand(PluginDataStep, Command):
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


# ---- Concrete commands ------------------------------------------------------

class SnapshotMutation(MutationStep, MeshCommand):
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


class SnapshotImport(ImportStep, Command):
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


class SnapshotCompound(DrawingSnapshotStep, MeshCommand):
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


class MeshSnapshotCommand(StitchSnapshotStep, MeshCommand):
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


class CompoundCommand(CompoundStep, Command):
    """A list of commands executed and reverted as one atomic step."""

    journal_parts = False

    def __init__(self, commands: Iterable[Command]) -> None:
        self.commands: list[Command] = list(commands)


class OccurrenceCommands(CompoundCommand):
    """Replay committed commands in their original occurrence coordinate frames."""

    @staticmethod
    def group_start(history, session):
        return history._native.group_start(session)

    @staticmethod
    def fold(history, session):
        return history._native.fold_occurrence(session, OccurrenceCommands)

    def doc_after(self, scene, phase):
        DocumentHistory.replay_group(self, scene, phase == "undo", True)

    def do(self, scene):
        DocumentHistory.replay_group(self, scene, False, False)

    def undo(self, scene):
        DocumentHistory.replay_group(self, scene, True, False)


class SetRecordCommand(RecordPatchStep, Command):
    """Top-level patch of group records; None patch removes the record.

    Nested values replace whole fields. GUIDs cannot be changed by a patch.
    Snapshots are detached and retained across redo, including new identities.
    """
    # History currently treats keeps_paint as also keeping BIM tags.
    keeps_paint = False
    #: Records are document values (``Group.ifc``, slice 2).
    doc_undo = True


    def __init__(self, targets, patch, *, normalize_values=True):
        self.normalize_values = normalize_values
        from copy import deepcopy
        self.targets = list(targets)
        self.patch = deepcopy(patch)
        self._before = None
        self._after = None


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
        registry_apply(self, scene, False)

    def undo(self, scene):
        registry_apply(self, scene, True)

    def doc_undo(self, scene) -> bool:
        # A project's registry lives in its project file, not the document.
        from core import project
        return project.path_for(scene) is None

    def doc_after(self, scene, phase) -> None:
        scene.mesh.bump_revision()
        scene.version += 1


class SetFaceTagsCommand(FaceTagsStep, Command):
    """Legacy loose-face tags, with stable identities across redo."""
    # History currently treats keeps_paint as also keeping BIM tags.
    keeps_paint = False

    def __init__(self, faces, ifc_class=None, name=''):
        self.faces = list(faces)
        self.ifc_class, self.name = ifc_class, name
        self._before = self._after = None


# ---- History ------------------------------------------------------------------


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

    @property
    def revision(self):
        return self._native.revision

    def changes_since(self, revision=0):
        """Bounded command journal; not a geometric diff or a persisted log."""
        if type(revision) is not int or revision < 0 or revision > self.revision:
            raise ValueError("revision must be between zero and the current revision")
        return self._native.changes_since(revision)

    def execute(self, cmd: Command) -> None:
        return self._native.run(self, cmd, "execute")

    def undo(self) -> bool:
        if not self.undo_stack:
            return False
        return self._native.run(self, self.undo_stack[-1], "undo")

    def redo(self) -> bool:
        if not self.redo_stack:
            return False
        return self._native.run(self, self.redo_stack[-1], "redo")

    def context_changed(self, before) -> None:
        if not isinstance(before, tuple):
            return  # A caller without a pre-transition serial needs a fresh scan.
        version, serial = before
        if version is not None:
            self.feed.readers.context_changed(version, serial, self.scene.content_version)

    def clear(self) -> None:
        self._native.clear_stacks()

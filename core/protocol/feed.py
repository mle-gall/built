# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The change feed (docs/core-protocol.md §4): one record per History step.

``History`` owns a :class:`ChangeFeed` (``history.feed``) that listens to
every execute, undo and redo through ``History.add_listener`` and, after a
step that ran, publishes a :class:`Step`:

- per mesh the step changed, a :class:`MeshChange`: for a store-backed
  mesh the store's own change record between the state before the step
  and after it (``MeshStore.changes_since``: created / removed / modified
  keys per kind, moved vertices); for a Python mesh, ``replaced`` (re-read
  it whole) — plus, for either, the faces a paint command repainted;
- document-level changes by stable key: placements (``Group.uid``), tags
  (``Layer.key``) and materials (registry name), each created / removed /
  modified, with ``complete`` False when the step may have changed
  placements the feed cannot name (the consumer then re-reads them).

Undo and redo are diffs too, so an undo publishes the inverse of the step
it takes back (created <-> removed, the same moved and modified keys).

Steps are numbered by ``History.revision`` (the token ``query_changes``
uses), kept in a bounded ring, and delivered to subscribers synchronously
on the UI thread, before any listener registered after the feed (every
other one: History makes its feed first). A step never holds a mesh
alive: :attr:`MeshChange.mesh` is a weak reference."""
from __future__ import annotations

import weakref

from core.protocol.keys import KINDS, mesh_key, to_json_key


def _empty():
    return {k: [] for k in KINDS}


class MeshChange:
    """What one step did to one mesh, by element key."""

    __slots__ = ("key", "_ref", "backing", "replaced", "created", "removed",
                 "modified", "moved", "revision")

    def __init__(self, mesh, backing, replaced=False, created=None,
                 removed=None, modified=None, moved=(), revision=None):
        self.key = mesh_key(mesh)
        self._ref = weakref.ref(mesh)
        #: ``"store"`` (element records below are the store's) or
        #: ``"python"`` (coarse: ``replaced`` says the mesh changed).
        self.backing = backing
        #: Re-read the whole mesh: a Python mesh changed, or the mesh moved
        #: between the store and Python objects during the step.
        self.replaced = bool(replaced)
        self.created = created or _empty()
        self.removed = removed or _empty()
        self.modified = modified or _empty()
        self.moved = list(moved)
        #: The mesh's store revision after the step (None for Python).
        self.revision = revision

    @property
    def mesh(self):
        """The mesh, while it lives (None after it was collected)."""
        return self._ref()

    def empty(self) -> bool:
        return not (self.replaced or self.moved
                    or any(self.created.values()) or any(self.removed.values())
                    or any(self.modified.values()))

    def faces_touched(self) -> set:
        """Face keys whose drawing may differ: created and modified ones
        (a renderer adds the faces around ``moved`` itself, from the
        store's ``faces_around``)."""
        return set(self.created["face"]) | set(self.modified["face"])

    def to_json(self) -> dict:
        ks = lambda d: {k: [to_json_key(x) for x in v] for k, v in d.items()}
        return {"mesh": self.key, "backing": self.backing,
                "replaced": self.replaced, "revision": self.revision,
                "created": ks(self.created), "removed": ks(self.removed),
                "modified": ks(self.modified),
                "moved": [to_json_key(x) for x in self.moved]}


def _sets():
    return {"created": [], "removed": [], "modified": []}


class Step:
    """One History step as the feed reports it."""

    __slots__ = ("revision", "phase", "command", "meshes", "placements",
                 "tags", "materials", "complete", "copies", "scene_fields", "records", "_reader_context")

    def __init__(self, revision, phase, command):
        self.revision = revision
        self.scene_fields = []
        self.records = {}
        self.phase = phase              # "execute" | "undo" | "redo"
        self.command = command          # the command's class name
        self.meshes: list[MeshChange] = []
        self.placements = _sets()       # Group.uid
        self.tags = _sets()             # Layer.key
        self.materials = _sets()        # scene.materials name
        #: False when placements may have changed beyond those named.
        self.complete = True
        #: Elements the step copied from one mesh into another (Explode,
        #: Split into Pieces): ``[{"source", "target", "keys"}]``, the mesh
        #: keys and, per kind, ``(source key, key in target)`` pairs. Not
        #: on undo (the copies are gone: ``removed`` names them).
        self.copies: list = []

    def mesh_change(self, mesh) -> MeshChange | None:
        for c in self.meshes:
            if c.mesh is mesh:
                return c
        return None

    def to_json(self) -> dict:
        return {"revision": self.revision, "phase": self.phase,
                "command": self.command, "complete": self.complete,
                "meshes": [c.to_json() for c in self.meshes],
                "placements": {k: list(v) for k, v in self.placements.items()},
                "tags": {k: list(v) for k, v in self.tags.items()},
                "materials": {k: list(v) for k, v in self.materials.items()},
                "copies": [{"source": c["source"], "target": c["target"],
                            "keys": {k: [[to_json_key(a), to_json_key(b)]
                                         for a, b in pairs]
                                     for k, pairs in c["keys"].items()}}
                           for c in self.copies]}


# ---------------------------------------------------------------- capture

def _placements_showing(scene, mesh) -> list:
    """Uids of every placement, nested and hidden ones included, that
    draws ``mesh``."""
    out, stack = [], list(scene.groups)
    while stack:
        g = stack.pop()
        if getattr(g, "mesh", None) is mesh:
            out.append(g.uid)
        stack.extend(getattr(g, "children", None) or ())
    return out


#: Above this many faces with attrs dicts of their own, the step's
#: before-token keeps only the store snapshot: copying every dict before
#: each step would cost O(painted faces). The faces a paint command names
#: are still reported; other attrs-only edits of existing faces are not.
ATTRS_COPY_MAX = 50_000


def _geometry_only():
    """Commands that change only the mesh they run on (their own target
    for a snapshot mutation), whatever their ``keeps_rows``."""
    from core import history as H
    return (H.SnapshotMutation, H.SnapshotCompound, H.MoveVerticesCommand,
            H.AddEdgeCommand, H.AddFaceCommand, H.EraseSelectionCommand,
            H.DeleteEdgesCommand, H.DeleteFaceCommand)


def _journal_covers(cmd) -> bool:
    """Commands that change placement FIELDS only: with the Rust document
    the step's journal names every placement they wrote, so listing them
    again (20,000 uids for Lock all) is only cost."""
    from core import history as H
    if getattr(cmd, "journal_covers", False):
        return True                 # a native step (docs/commands-port.md)
    if isinstance(cmd, H.MoveGroupCommand):
        return getattr(cmd.group, "xform", None) is not None
    if isinstance(cmd, (H.HideCommand, H.AssignLayerCommand)):
        from core.group import Group
        return all(isinstance(e, Group) for e in cmd._entities)
    return False


def _known_placements(cmd, phase, step, scene=None, journalled=False) -> bool:
    """Name the placements a known command changed; False when the
    command may change placements the feed cannot name. ``journalled``:
    the step's record already holds what its document transaction wrote."""
    from core import history as H
    if isinstance(cmd, H.CompoundCommand):
        return all(_known_placements(c, phase, step, scene, journalled)
                   for c in cmd.commands)
    target = getattr(cmd, "_history_mesh", None)
    definition = getattr(target, "_instance_definition", None)
    if definition is not None and step.mesh_change(definition) is not None:
        step.placements["modified"].extend(scene._root._definition_uids(definition))
    if journalled and _journal_covers(cmd):
        return True
    if isinstance(cmd, H.NativeCommand):
        if not journalled:
            # Run outside a document transaction: name what it writes.
            step.placements["modified"].extend(g.uid for g in cmd.targets)
        return cmd.journal_covers
    if getattr(cmd, "keeps_rows", False) or isinstance(cmd, _geometry_only()):
        return True
    from core.solids import SolidOperationCommand
    if isinstance(cmd, SolidOperationCommand):
        added, gone = (cmd.removed, cmd.created) if phase == "undo" else (cmd.created, cmd.removed)
        step.placements["created"].extend(g.uid for g in added or ())
        step.placements["removed"].extend(g.uid for g in gone or ())
        return True
    group = getattr(cmd, "group", None)
    uid = getattr(group, "uid", None)
    if isinstance(cmd, (H.InsertGroupCommand, H.DeleteGroupCommand)) and uid:
        adding = isinstance(cmd, H.InsertGroupCommand) != (phase == "undo")
        step.placements["created" if adding else "removed"].append(uid)
        return True
    if isinstance(cmd, H.MoveGroupCommand) and uid:
        step.placements["modified"].append(uid)
        return True
    undo = phase == "undo"
    if isinstance(cmd, H.ExplodeGroupCommand) and uid:
        # The group goes, what it held comes out (undo: the reverse).
        step.placements["created" if undo else "removed"].append(uid)
        step.placements["removed" if undo else "created"].extend(
            c[0].uid for c in cmd._lifted)
        return True
    if isinstance(cmd, H.SplitIntoPiecesCommand) and uid:
        step.placements["modified"].append(uid)
        step.placements["removed" if undo else "created"].extend(
            p.uid for p in cmd.pieces)
        return True
    if getattr(cmd, "reference_instance", False) and scene is not None:
        # The edited instance goes back on the definition (undo: the
        # definition goes back as it was): every placement showing it.
        modified = step.placements["modified"]
        modified.extend(_placements_showing(scene, cmd.proto))
        # The journal may already name the edited instance ahead of its siblings.
        modified.sort()
        return True
    if isinstance(cmd, H.ExplodeViewCommand):
        step.placements["modified"].extend(
            k.uid for k in getattr(cmd.container, "children", None) or ())
        return True
    if isinstance(cmd, H.PurgeUnusedCommand):
        return True                 # tags and materials only
    if isinstance(cmd, (H.HideCommand, H.AssignLayerCommand)):
        from core.group import Group
        step.placements["modified"].extend(
            e.uid for e in cmd._entities if isinstance(e, Group))
        return True
    if type(cmd).__name__.startswith("SetFace") or isinstance(
            cmd, (H.SetRecordCommand, H.SetRegistryCommand)):
        if isinstance(cmd, H.SetRecordCommand):
            step.complete = False   # records live on placements
        return True
    return False


# ---------------------------------------------------------------- the feed

class ChangeFeed:
    """The per-step change records of one History (see the module doc)."""

    #: Steps kept for :meth:`since` (History keeps 256 journal entries).
    maxlen = 256

    def __init__(self, history):
        self._history = weakref.ref(history)
        self._native = history._native
        self._subscribers = history._native.subscribers
        from ingetrazo_core import FeedReaders
        self.readers = FeedReaders(getattr(getattr(history.scene, "_root", None), "_doc_key", None))
        self.subscribe(self.readers)
        self._pending = None
        self._depth = 0
        self.unsubscribe = history.add_listener(before=self._before,
                                                after=self._after)

    # -- reading
    @property
    def revision(self) -> int:
        h = self._history()
        return h.revision if h is not None else 0

    def current(self, reader, version):
        return self.readers.revision == self.revision and self.readers.current(reader, version)

    @property
    def last(self) -> Step | None:
        return self._native.feed_step(None)

    def step(self, revision) -> Step | None:
        return self._native.feed_step(revision) if 0 <= revision <= self.revision else None

    def since(self, revision=0) -> dict:
        """``{revision, complete, steps}``: the steps after ``revision``
        (``complete`` False when older ones fell out of the ring; the
        consumer re-reads what it follows)."""
        now = self.revision
        if type(revision) is not int or revision < 0 or revision > now:
            raise ValueError("revision must be between zero and the current revision")
        return self._native.feed_since(revision)

    def subscribe(self, fn):
        """Call ``fn(step)`` after every published step; returns an
        unsubscribe function."""
        entry = [fn]
        self._subscribers.append(entry)

        def unsubscribe():
            if entry in self._subscribers:
                self._subscribers.remove(entry)
        return unsubscribe

    # -- History's listener callbacks
    def _before(self, cmd, phase, ran):
        self._depth += 1
        if self._depth > 1:
            return          # a command run inside another: the outer step covers it
        h = self._history()
        scene = h.scene if h is not None else None
        if scene is None:
            self._pending = None
            return
        serial = self.readers.stamp()
        tokens = self._native.capture_meshes(cmd, scene, phase, ATTRS_COPY_MAX)
        self._pending = (cmd, phase, tokens,
                         getattr(scene, "content_version", None), serial)

    def _after(self, cmd, phase, ran):
        self._depth = max(0, self._depth - 1)
        if self._depth:
            return
        pending, self._pending = self._pending, None
        if not ran or pending is None or pending[0] is not cmd:
            return
        h = self._history()
        if h is None:
            return
        step = self._native.build_step(h, cmd, phase, pending[2])
        step._reader_context = (pending[3], getattr(h.scene, "content_version", None), pending[4],
                                getattr(cmd, "keeps_paint", False), getattr(cmd, "keeps_rows", False),
                                getattr(cmd, "keeps_groups", False))
        return self._native.publish(step)

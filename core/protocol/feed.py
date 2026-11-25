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
from ingetrazo_core import FeedMeshRecord, FeedStepRecord

from core.protocol.keys import KINDS, mesh_key


def _empty():
    return {k: [] for k in KINDS}


class MeshChange(FeedMeshRecord):
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

def _sets():
    return {"created": [], "removed": [], "modified": []}


class Step(FeedStepRecord):
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

#: Above this many faces with attrs dicts of their own, the step's
#: before-token keeps only the store snapshot: copying every dict before
#: each step would cost O(painted faces). The faces a paint command names
#: are still reported; other attrs-only edits of existing faces are not.
ATTRS_COPY_MAX = 50_000


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

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

import logging
import weakref
from collections import deque

from core.protocol.keys import KINDS, mesh_key, store_of, to_json_key

_log = logging.getLogger("ingetrazo.protocol")


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
                 "tags", "materials", "complete", "copies")

    def __init__(self, revision, phase, command):
        self.revision = revision
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

def _command_meshes(cmd, scene, phase) -> list:
    """The meshes a step runs on, known before it runs: the mesh the
    command was executed against (undo / redo go back to it) or the open
    one, a snapshot mutation's own target, a soften's meshes; a compound's
    parts' too."""
    from core.history import CompoundCommand, SnapshotMutation
    out = []
    if phase == "execute":
        out.append(scene.mesh)
    else:
        out.append(getattr(cmd, "_history_mesh", None) or scene.mesh)
    stack = [cmd]
    while stack:
        c = stack.pop()
        if isinstance(c, CompoundCommand):
            stack.extend(c.commands)
        elif isinstance(c, SnapshotMutation):
            out.append(c._target(scene))
        meshes = getattr(c, "meshes", None)
        if callable(meshes) and type(c).__name__ in (
                "SoftenByAngleCommand", "FlipFacesCommand", "FlipGroupsCommand"):
            out.extend(meshes())
    seen, uniq = set(), []
    for m in out:
        if m is not None and id(m) not in seen:
            seen.add(id(m))
            uniq.append(m)
    return uniq


#: Above this many faces with attrs dicts of their own, the step's
#: before-token keeps only the store snapshot: copying every dict before
#: each step would cost O(painted faces). The faces a paint command names
#: are still reported; other attrs-only edits of existing faces are not.
ATTRS_COPY_MAX = 50_000


def _token(mesh):
    """What the mesh looked like before the step: its ``capture_state()``
    when store-backed (the store's snapshot, O(1), plus the faces' own
    attrs dicts, copied), else its serials."""
    st = store_of(mesh)
    serials = (mesh._mut_serial, getattr(mesh, "revision", None))
    if st is None:
        return (None, serials)
    if len(mesh.__dict__.get("_attr_own") or ()) > ATTRS_COPY_MAX:
        return (st.snapshot(), serials)
    return (mesh.capture_state(), serials)


def _store_change(mesh, st, before):
    """The store's net record since ``before`` (store/changes.rs), face
    attrs compared when ``before`` holds them; None across namespaces."""
    from core.store_mesh import StoreState
    try:
        if isinstance(before, StoreState):
            d = mesh.__dict__
            # Read at once, so the live dicts need no copy.
            now = StoreState(st.snapshot(), d["_attr_own"], d["_attr_table"], mesh)
            return now.changes_since(before)
        return st.snapshot().changes_since(before, [])
    except ValueError:
        return None         # another namespace: a foreign snapshot adopted


def _mesh_change(mesh, token, painted=()) -> MeshChange:
    """``painted``: face keys the step's paint commands name in this mesh
    (a Python mesh's attrs change shows in no counter that says which)."""
    before, serials = token
    st = store_of(mesh)
    if before is not None and st is not None:
        ch = _store_change(mesh, st, before)
        if ch is not None:
            return MeshChange(mesh, "store", created=ch.created,
                              removed=ch.removed, modified=ch.modified,
                              moved=ch.moved, revision=st.revision)
        return MeshChange(mesh, "store", replaced=True, revision=st.revision)
    now = (mesh._mut_serial, getattr(mesh, "revision", None))
    backing = "store" if st is not None else "python"
    # Into or out of the store during the step (a materialisation, an
    # undo restoring a store snapshot), or a Python mesh's geometry
    # changed: no element record, re-read it. A revision moved alone is
    # an attrs change (paint, tags): named when a paint command says
    # which faces, else re-read too.
    replaced = ((before is None) != (st is None) or now[0] != serials[0]
                or (now[1] != serials[1] and not painted))
    return MeshChange(mesh, backing, replaced=replaced,
                      revision=st.revision if st is not None else None)


def _painted(cmd) -> list:
    """Face keys a paint command (``SetFace*Command``) wrote attrs on:
    attrs are Python dicts beside the store (design decision 6), so the
    store's record does not see them."""
    from core.history import CompoundCommand
    out, stack = [], [cmd]
    while stack:
        c = stack.pop()
        if isinstance(c, CompoundCommand):
            stack.extend(c.commands)
        elif type(c).__name__.startswith("SetFace"):
            faces = getattr(c, "_faces", None)
            if isinstance(faces, list):
                out.extend(f.key for f in faces if hasattr(f, "key"))
    return out


def _copies(cmd) -> list:
    """What a command says it copied between meshes (``cmd.copies``:
    ``[(source mesh, target mesh, {kind: [(key, key)]})]``)."""
    from core.history import CompoundCommand
    out, stack = [], [cmd]
    while stack:
        c = stack.pop()
        if isinstance(c, CompoundCommand):
            stack.extend(c.commands)
            continue
        for src, dst, keys in getattr(c, "copies", None) or ():
            out.append({"source": mesh_key(src), "target": mesh_key(dst),
                        "keys": {k: list(keys[k]) for k in keys}})
    return out


def _doc_state(scene):
    roots = {id(g): g.uid for g in scene.groups}
    # By value: tags and materials are edited in place too.
    # (A tag's fields are plain values: a shallow copy sees their change.)
    tags = {ly.key: tuple(vars(ly).items())
            for ly in getattr(scene, "layers", ()) or ()}
    mats = {k: repr(m) for k, m in (getattr(scene, "materials", {}) or {}).items()}
    return roots, tags, mats


def _geometry_only():
    """Commands that change only the mesh they run on (their own target
    for a snapshot mutation), whatever their ``keeps_rows``."""
    from core import history as H
    return (H.SnapshotMutation, H.SnapshotCompound, H.MoveVerticesCommand,
            H.AddEdgeCommand, H.AddFaceCommand, H.EraseSelectionCommand,
            H.DeleteEdgesCommand, H.DeleteFaceCommand)


def _known_placements(cmd, phase, step) -> bool:
    """Name the placements a known command changed; False when the
    command may change placements the feed cannot name."""
    from core import history as H
    if isinstance(cmd, H.CompoundCommand):
        return all(_known_placements(c, phase, step) for c in cmd.commands)
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


def _dedupe(d):
    for k, v in d.items():
        d[k] = list(dict.fromkeys(v))
    # A uid both removed and created (a group replaced by its copy, which
    # keeps the uid): modified.
    both = set(d["created"]) & set(d["removed"])
    if both:
        d["created"] = [u for u in d["created"] if u not in both]
        d["removed"] = [u for u in d["removed"] if u not in both]
        d["modified"] = list(dict.fromkeys(d["modified"] + sorted(both)))


# ---------------------------------------------------------------- the feed

class ChangeFeed:
    """The per-step change records of one History (see the module doc)."""

    #: Steps kept for :meth:`since` (History keeps 256 journal entries).
    maxlen = 256

    def __init__(self, history):
        self._history = weakref.ref(history)
        self._steps: deque[Step] = deque(maxlen=self.maxlen)
        self._subscribers = []
        self._pending = None
        self._depth = 0
        self.unsubscribe = history.add_listener(before=self._before,
                                                after=self._after)

    # -- reading
    @property
    def revision(self) -> int:
        h = self._history()
        return h.revision if h is not None else 0

    @property
    def last(self) -> Step | None:
        return self._steps[-1] if self._steps else None

    def step(self, revision) -> Step | None:
        for s in reversed(self._steps):
            if s.revision == revision:
                return s
            if s.revision < revision:
                break
        return None

    def since(self, revision=0) -> dict:
        """``{revision, complete, steps}``: the steps after ``revision``
        (``complete`` False when older ones fell out of the ring; the
        consumer re-reads what it follows)."""
        now = self.revision
        if type(revision) is not int or revision < 0 or revision > now:
            raise ValueError("revision must be between zero and the current revision")
        floor = self._steps[0].revision - 1 if self._steps else now
        return {"revision": now, "complete": revision >= floor,
                "steps": [s for s in self._steps if s.revision > revision]}

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
        meshes = _command_meshes(cmd, scene, phase)
        self._pending = (cmd, phase, [(m, _token(m)) for m in meshes],
                         _doc_state(scene))

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
        step = self._build(h, cmd, phase, pending)
        self._steps.append(step)
        for entry in list(self._subscribers):
            try:
                entry[0](step)
            except Exception:  # noqa: BLE001 — a consumer never breaks a command
                _log.exception("Change feed subscriber failed")

    def _build(self, h, cmd, phase, pending) -> Step:
        _cmd, _phase, tokens, (roots0, tags0, mats0) = pending
        scene = h.scene
        step = Step(h.revision, phase, type(cmd).__name__)
        painted = _painted(cmd)
        for i, (mesh, token) in enumerate(tokens):
            change = _mesh_change(mesh, token, painted if i == 0 else ())
            if not change.empty():
                step.meshes.append(change)
        if painted:
            mesh = tokens[0][0] if tokens else scene.mesh
            change = step.mesh_change(mesh)
            if change is None:
                change = MeshChange(mesh, "store" if store_of(mesh) is not None
                                    else "python")
                step.meshes.append(change)
            mod = change.modified["face"]
            mod.extend(k for k in painted if k not in set(mod))
        if phase != "undo":
            step.copies = _copies(cmd)
        # Document level: roots by identity, tags and materials by value.
        roots1, tags1, mats1 = _doc_state(scene)
        step.placements["created"].extend(u for i, u in roots1.items() if i not in roots0)
        step.placements["removed"].extend(u for i, u in roots0.items() if i not in roots1)
        step.complete = _known_placements(cmd, phase, step) and step.complete
        _dedupe(step.placements)
        for k in tags1.keys() - tags0.keys():
            step.tags["created"].append(k)
        for k in tags0.keys() - tags1.keys():
            step.tags["removed"].append(k)
        step.tags["modified"].extend(k for k in tags1.keys() & tags0.keys()
                                     if tags1[k] != tags0[k])
        for k in mats1.keys() - mats0.keys():
            step.materials["created"].append(k)
        for k in mats0.keys() - mats1.keys():
            step.materials["removed"].append(k)
        step.materials["modified"].extend(k for k in mats1.keys() & mats0.keys()
                                          if mats1[k] != mats0[k])
        for d in (step.tags, step.materials):
            for k in d:
                d[k].sort()
        return step

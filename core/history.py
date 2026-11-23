# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Editor binding for document History and public command imports."""
from __future__ import annotations

import logging
import os as _os
import time as _time_mod
from collections import deque
from pathlib import Path as _Path
from typing import Optional

from core import doc_model as _doc
from core import history_commands
from core.history_commands import *  # Existing editor and extension API.
from core.history_commands import _dirty_placements, _RENDER_FIELDS


def __getattr__(name):
    # Legacy tools also import geometry helpers from core.history.
    return getattr(history_commands, name)


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
        self._native = _doc.NATIVE.DocumentHistory() if _doc.ENABLED else None
        if self._native is None:
            self._revision = 0
            self._changes = deque(maxlen=256)
            self.undo_stack = []
            self.redo_stack = []
        else:
            self.undo_stack = self._native.undo
            self.redo_stack = self._native.redo
        #: Message describing the last rolled-back failure (UI may flash it).
        self.last_error: Optional[str] = None
        #: Why the last command declined to happen (:class:`CommandRejected`).
        self.last_rejected: Optional[str] = None
        #: The document transaction of the last step that ran (execute,
        #: undo or redo) and whether it is read backwards: ``(txn,
        #: inverse)``, what the feed takes the document half of a step's
        #: record from. ``None`` with the Python document.
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
        return self._native.revision if self._native is not None else self._revision

    def changes_since(self, revision=0):
        """Bounded command journal; not a geometric diff or a persisted log."""
        if type(revision) is not int or revision < 0 or revision > self.revision:
            raise ValueError("revision must be between zero and the current revision")
        if self._native is not None:
            return self._native.changes_since(revision)
        floor = self._changes[0]["revision"] - 1 if self._changes else self.revision
        return {"revision": self.revision, "complete": revision >= floor,
                "changes": [dict(c) for c in self._changes if c["revision"] > revision]}

    def _run(self, cmd, phase, operation):
        if getattr(self.scene, 'worksession_read_only', False):
            self.last_rejected = 'Choose this file in Worksession before editing it.'
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
                if self._native is not None:
                    self._native.advance(phase, name)
                else:
                    self._revision += 1
                    self._changes.append({"revision": self.revision, "phase": phase,
                                          "command": name})
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
        snapshot = None
        self.last_rejected = None
        before = getattr(self.scene, "content_version", None)
        # Geometry-only commands cannot create groups, even on an active tag.
        include_groups = not cmd.keeps_rows
        objs_before = self._objects_before(include_groups=include_groups)
        self.last_txn = None

        def tag_objects():
            self._tag_new_objects(objs_before, include_groups=include_groups)

        try:
            if self._native is not None:
                self._native.execute(self, cmd, tag_objects)
            else:
                snapshot = self.scene.mesh.capture_state()
                cmd.do(self.scene)
                self._follow_text_placements()
                tag_objects()
                cmd._doc_txn = None
        except Exception as exc:
            if snapshot is not None:
                self.scene.mesh.restore_state(snapshot)
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
        if self._native is not None:
            self._native.push(cmd, cap)
        else:
            self.undo_stack.append(cmd)
            if cap and len(self.undo_stack) > cap:
                path = getattr(cmd, "_occurrences", ())
                steps = (OccurrenceCommands.group_start(self, path[0]) + 1
                         if path else len(self.undo_stack))
                del self.undo_stack[:max(0, steps - cap)]
            self.redo_stack.clear()
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
        source, dest = ((self.undo_stack, self.redo_stack) if undo
                        else (self.redo_stack, self.undo_stack))
        before = getattr(self.scene, "content_version", None)
        cmd = source[-1]
        if self._native is not None:
            self._native.replay(self, undo, False)
        else:
            self._reference_replay(cmd, undo)
            dest.append(source.pop())
        self._dirty_from_txn()
        self._keep_geometry_caches(cmd, before)
        self._rebind_dimensions()
        return True

    def _reference_replay(self, cmd, undo):
        include_groups = not cmd.keeps_rows
        objects = None if undo else self._objects_before(include_groups=include_groups)
        action = cmd.undo if undo else cmd.do
        self._journalled(lambda: self._in_command_mesh(cmd, action, self.scene))
        if not undo:
            self._tag_new_objects(objects, include_groups=include_groups)

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

    def _journalled(self, fn) -> None:
        """Run a command's own undo or redo inside a transaction kept only
        for the feed's record (``last_txn``)."""
        depth = _doc.begin()
        self.last_txn = None
        try:
            fn()
            self._follow_text_placements()
            txn = _doc.commit() if depth else None
        except Exception:
            if depth:
                _doc.rollback()
            raise
        self.last_txn = (txn, False) if txn is not None else None

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
        self.undo_stack.clear()
        self.redo_stack.clear()

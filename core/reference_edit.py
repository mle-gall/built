# SPDX-License-Identifier: GPL-3.0-or-later
"""World-copy edit sessions for object meshes and singular store placements."""
from ingetrazo_core import Matrix4, command_protocol
from ingetrazo_core.command_protocol import restore_sharing, share_back

from core.history import Command, MeshCommand, CompoundCommand, OccurrenceCommands


class ReshareInstanceCommand(command_protocol.ReshareStep, Command):
    """Leaving a component instance edited in place: the session's edits go
    into the shared definition, so every copy shows them.

    The commands run inside the session are absorbed into this one — the
    whole edit undoes as one step, back to the definition as it was, with
    the instance shared again. Redo replays it from the edited copy."""

    reference_instance = True

    def __init__(self, group, proto, xform, edited, inner=()) -> None:
        self.group = group
        self.proto = proto
        self.xform = xform
        self.edited = edited
        self.inner = list(inner)
        self._before = None
        self._after = None

class ReshareAssemblyCommand(command_protocol.ReshareStep, Command):
    """Leaving an assembly edited in place: a component holding other
    components, placed more than once. Its own geometry goes back into the
    definition its copies share, and its matrix comes back up out of its
    parts.

    The commands run inside are folded in, one step either way, and stay
    what they are: they were recorded with the matrix down in the parts,
    where the tools work. Undo puts it back down, undoes them there, and
    returns the parts to the frame they entered with, bit for bit. Redo
    replays them the same way. A snapshot of the definition alone cannot
    do it: deleting, moving or exploding a part changes the parts, not the
    definition."""

    reference_assembly = True

    def __init__(self, group, proto, xform, edited, state0, parts,
                 inner=()) -> None:
        self.group = group
        self.proto = proto
        self.xform = Matrix4(xform)
        self.edited = edited
        self.state0 = state0
        self.parts = parts            # on entry, matrix up: parts_record
        self.inner = list(inner)
        self._framed = None           # on leaving, matrix down
        self._edited = None           # the world copy on leaving
        self._left = None             # on leaving, matrix up
        self._entered = None          # on entry, matrix down (set by undo)
        self._before = None           # the definition before / after
        self._after = None
        self.shared = False

class _StoreSession(MeshCommand):
    """Close a singular placement's world copy. History journals the writes
    to the canonical store and placements. Replay uses the entry and exit
    transactions around the commands that actually ran inside."""
    def __init__(self, share, parts, inner, shared, entry):
        self.entry = entry
        self.group, self.proto, self.xform = share[:3]
        self.edited = self.group.mesh
        self.parts = parts
        self.inner = inner
        self.shared = shared
        self.reference_assembly = parts is not None
        self.reference_instance = parts is None
        self.history_name = ("ReshareAssemblyCommand" if self.reference_assembly
                             else "ReshareInstanceCommand")
        self.replay = None

    def geometry_meshes(self, scene):
        return (self.proto,)

    def do(self, scene):
        if self.replay is not None:
            self.replay.do(scene)
        elif self.shared:
            share_back(self.group, self.proto, self.xform, self.edited, self.parts)
        else:
            restore_sharing(self.group, self.proto, self.xform, self.parts)
        scene.version += 1

    def undo(self, scene):
        self.replay.undo(scene)
        scene.version += 1

    def doc_after(self, scene, phase):
        OccurrenceCommands(self.inner).doc_after(scene, phase)
        scene.version += 1

    def bind_transaction(self, txn):
        edits = [getattr(c, "_doc_txn", None) for c in self.inner]
        if all(edit is not None for edit in edits):
            self._doc_txn = txn.compose([self.entry, *edits, txn])
            return
        # A compatibility tool changed Python-owned state, e.g. annotations.
        # Keep its own inverse between the native entry and exit transactions.
        opening, closing = CompoundCommand([]), CompoundCommand([])
        opening._doc_txn = self.entry
        closing._doc_txn = txn
        self.replay = OccurrenceCommands([opening, *self.inner, closing])
        self._doc_txn = None


def record_leave(history, level, share):
    """Close the compatibility session as one undoable step."""
    parts = level.get("parts")
    group, proto, xform, state0 = share
    edited = group.mesh
    mark = level.get("undo_mark", len(history.undo_stack))
    mark = min(mark, len(history.undo_stack))
    if "session" in level:
        mark = OccurrenceCommands.group_start(history, level["session"])
    shared = edited.capture_state() != state0
    entry = level.get("entry_transaction")
    from core import doc_model
    changed_parts = parts is not None and len(history.undo_stack) > mark
    if not (shared or changed_parts):
        restore_sharing(group, proto, xform, parts=parts)
        return shared
    inner = history.undo_stack[mark:]
    if entry is not None and not doc_model.NATIVE.doc_depth():
        step = _StoreSession(share, parts, inner, shared, entry)
    elif parts is not None:
        step = ReshareAssemblyCommand(group, proto, xform, edited, state0, parts, inner)
    else:
        step = ReshareInstanceCommand(group, proto, xform, edited, inner)
    history._native.close_group(history, mark, step)
    return shared

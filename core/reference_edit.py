# SPDX-License-Identifier: GPL-3.0-or-later
"""History command constructors for singular placement edit sessions."""
from ingetrazo_core import Matrix4, command_protocol
from ingetrazo_core.command_protocol import record_leave

from core.history import Command, MeshCommand


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

class _StoreSession(command_protocol.StoreSessionStep, MeshCommand):
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

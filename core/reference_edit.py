# SPDX-License-Identifier: GPL-3.0-or-later
"""Compatibility edit sessions for the Python reference document and object meshes.

The native occurrence store and its history never enter these sessions.
"""
from ingetrazo_core import Matrix4, Vec3

from core.history import Command


def enter(scene, group, frame):
    share = parts = None
    is_comp = getattr(group, "is_component", None)
    if getattr(group, "children", None):
        # A container: its children stay children. What CANNOT stay is a
        # transform on it, because the tools work in world coordinates —
        # so the matrix is pushed down into the children and into its own
        # mesh, which leaves every world position exactly where it was.
        instance = (callable(is_comp) and is_comp()
                    and _mesh_placed_elsewhere(scene, group))
        proto, xform = group.mesh, group.xform
        if instance:
            parts = parts_record(group)
        _bake_container_xform(group)
        if instance:
            # An assembly is a component too: its own geometry is the
            # definition its copies share. The session edits a world
            # copy, and leaving shares it back and returns the matrix
            # up out of the parts, as for a component with no parts.
            if group.mesh is proto:
                from core.group import transformed_mesh
                group.mesh = transformed_mesh(proto, Matrix4())
            share = (group, proto, Matrix4(xform), group.mesh.capture_state())
        if frame is not None:
            # Its matrix went down into the children and its mesh is in
            # world coordinates now: the axes stay, as world axes.
            group.axes = frame
    elif getattr(group, "xform", None) is not None:
        # A component instance: the tools work in world coordinates, so
        # the session edits a world-space COPY of the definition. On
        # leaving, the copy goes back into the shared prototype (local
        # coordinates) and every sibling shows the edit — classic
        # component editing. Make Unique first to edit one copy only.
        from core.group import transformed_mesh
        proto, xform = group.mesh, group.xform
        group.mesh = transformed_mesh(proto, xform)
        group.xform = None
        share = (group, proto, xform, group.mesh.capture_state())
    return share, parts


def _bake_container_xform(group) -> None:
    """Push a container's own matrix down into its children and its mesh.

    A group that owns placements is always an instance (``Group.adopt``),
    so moving it composes into ``xform``. Inside it the tools speak world
    coordinates, so the matrix has to come down one level: every child
    matrix takes it on the left, the group's own geometry moves to world,
    and the group is left at identity. Nothing moves in the world.

    Its own geometry moves as a COPY, never in place: a container's mesh
    can be the very prototype its children share (an imported component
    tree is exactly that), and walking those vertices would drag every
    placement with it.
    """
    xform = getattr(group, "xform", None)
    if xform is None or xform == Matrix4():
        return
    from core.explode import rotate_offsets
    from core.group import transformed_mesh
    for child in group.children:
        child.xform = xform * (child.xform if child.xform is not None
                               else Matrix4())
    # An exploded view's offsets live in the container's frame, which
    # has just become the world's.
    rotate_offsets(group, xform)
    if group.mesh.vertices:
        group.mesh = transformed_mesh(group.mesh, xform)
    group.xform = Matrix4()


def _mesh_placed_elsewhere(scene, group) -> bool:
    """Whether another placement, at any depth, draws ``group``'s mesh:
    an assembly placed once has no copies to share an edit with."""
    from core.purge import iter_groups
    mesh = group.mesh
    return any(g is not group and g.mesh is mesh
               for g in iter_groups(scene.groups))

def parts_record(group) -> tuple:
    """An assembly's axes and its parts' matrices and offsets, copied,
    to put back bit for bit: a round trip through the inverse matrix
    drifts in the last digits."""
    axes = getattr(group, "axes", None)
    return (None if axes is None else Matrix4(axes),
            [(c, None if c.xform is None else Matrix4(c.xform),
              c.explode_offset) for c in group.children])

def put_parts(group, record) -> None:
    """Put back what :meth:`parts_record` copied. The list of parts is
    not touched: the commands that changed it undo that themselves."""
    axes, parts = record
    for child, xform, offset in parts:
        child.xform = None if xform is None else Matrix4(xform)
        child.explode_offset = offset
    group.axes = None if axes is None else Matrix4(axes)

def _unbake_container_xform(group, xform, parts=None) -> None:
    """Undo :meth:`_bake_container_xform` on leaving an assembly: its
    matrix comes back up out of the parts, which return to the frame
    the copies share, and its entry axes are its axes again. Nothing
    moves in the world. A redo arrives with the matrix already back,
    and leaves it alone.

    ``parts`` is :meth:`parts_record` on entry: a part the session left
    where it was gets its entry matrix back bit for bit, instead of a
    round trip through the inverse that drifts in the last digits."""
    if (not getattr(group, "children", None) or xform is None
            or group.xform == xform):
        return
    inverse, ok = xform.inverted()
    if not ok:
        return
    axes, entry = parts if parts is not None else (None, ())
    entry = {id(c): (m, o) for c, m, o in entry}
    for child in group.children:
        was = entry.get(id(child))
        now = child.xform if child.xform is not None else Matrix4()
        if was is not None:
            m0 = was[0] if was[0] is not None else Matrix4()
            if xform * m0 == now:
                child.xform = None if was[0] is None else Matrix4(m0)
            else:
                child.xform = inverse * now
            o0, o = was[1], child.explode_offset
            if o0 and o:
                v = xform.mapVector(Vec3(*o0))
                turned = (v.x(), v.y(), v.z())
                child.explode_offset = (o0 if turned == tuple(o) else
                                        _turned(inverse, o))
            elif o:
                child.explode_offset = _turned(inverse, o)
            else:
                child.explode_offset = o
        else:
            child.xform = inverse * now
            if child.explode_offset:
                child.explode_offset = _turned(
                    inverse, child.explode_offset)
    group.axes = None if axes is None else Matrix4(axes)

def _turned(m, offset) -> tuple:
    v = m.mapVector(Vec3(*offset))
    return (v.x(), v.y(), v.z())

def restore_sharing(group, proto, xform, parts=None) -> None:
    """The instance left untouched: back on the shared definition (an
    assembly's matrix back up out of its ``parts``)."""
    _unbake_container_xform(group, xform, parts)
    group.mesh = proto
    group.xform = xform

def share_back(group, proto, xform, edited, parts=None) -> None:
    """Write the edited world-space mesh into the shared prototype (local
    coordinates) and put the instance back on it: every sibling shows
    the edit. An assembly's matrix also comes back up out of its parts
    (``parts``: :meth:`parts_record` on entry)."""
    from core.group import transformed_mesh
    inverse, ok = xform.inverted()
    local = transformed_mesh(edited, inverse if ok else xform)
    proto.restore_state(local.capture_state())
    # A face reversed in the session was counted on the world copy, and
    # a reversed loop is all the definition's chunk could not see.
    proto.note_rewound()
    _unbake_container_xform(group, xform, parts)
    group.mesh = proto
    group.xform = xform


def finish(share, parts):
    group, proto, xform, state = share
    if group.mesh.capture_state() == state:
        restore_sharing(group, proto, xform, parts)
        return True
    share_back(group, proto, xform, group.mesh, parts)
    return False


class ReshareInstanceCommand(Command):
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

    def do(self, scene) -> None:
        if self._before is None:
            self._before = self.proto.capture_state()
            share_back(self.group, self.proto, self.xform, self.edited)
            self._after = self.proto.capture_state()
        else:
            # Redo: the definition as the first run left it, so its
            # elements keep the keys that run gave them (the change feed
            # publishes the same step again). share_back would copy the
            # edited instance once more, under fresh keys.
            self.proto.restore_state(self._after)
            self.proto.note_rewound()
            self.group.mesh = self.proto
            self.group.xform = self.xform
        scene.version += 1

    def undo(self, scene) -> None:
        self.proto.restore_state(self._before)
        self.proto.note_rewound()
        self.group.mesh = self.proto
        self.group.xform = self.xform
        scene.version += 1


class ReshareAssemblyCommand(Command):
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

    @staticmethod
    def _in_mesh(scene, cmd, fn) -> None:
        """As History does: each command against the mesh it ran on."""
        target = getattr(cmd, "_history_mesh", None)
        if target is None or target is scene.mesh:
            fn()
            return
        target._mut_serial += 1
        current = scene.mesh
        scene.mesh = target
        try:
            fn()
        finally:
            scene.mesh = current

    def _frame(self, record, state) -> None:
        """The matrix down in the parts and the world copy in place: how
        the session stood."""
        self.edited.restore_state(state)
        self.group.mesh = self.edited
        self.group.xform = Matrix4()
        put_parts(self.group, record)

    def _unframe(self, record) -> None:
        self.group.mesh = self.proto
        self.group.xform = Matrix4(self.xform)
        put_parts(self.group, record)

    def do(self, scene) -> None:
        refresh = getattr(scene, "_refresh_dimension_holds", None)
        if refresh is not None:
            refresh()
        if self._framed is None:                 # leaving, the first time
            self._framed = parts_record(self.group)
            self._edited = self.edited.capture_state()
            self._before = self.proto.capture_state()
            self.shared = self._edited != self.state0
            if self.shared:
                share_back(self.group, self.proto, self.xform,
                           self.edited, parts=self.parts)
            else:
                restore_sharing(self.group, self.proto, self.xform,
                                parts=self.parts)
            self._after = self.proto.capture_state()
            self._left = parts_record(self.group)
        else:
            self._frame(self._entered, self.state0)
            for cmd in self.inner:
                self._in_mesh(scene, cmd, lambda cmd=cmd: cmd.do(scene))
            self.proto.restore_state(self._after)
            self.proto.note_rewound()
            self._unframe(self._left)
        scene.version += 1

    def undo(self, scene) -> None:
        refresh = getattr(scene, "_refresh_dimension_holds", None)
        if refresh is not None:
            refresh()
        self._frame(self._framed, self._edited)
        self.proto.restore_state(self._before)
        self.proto.note_rewound()
        for cmd in reversed(self.inner):
            self._in_mesh(scene, cmd, lambda cmd=cmd: cmd.undo(scene))
        self._entered = parts_record(self.group)
        self._unframe(self.parts)
        scene.version += 1


def record_leave(history, level, share):
    """Close the compatibility session as one undoable step."""
    parts = level.get("parts")
    group, proto, xform, state0 = share
    edited = group.mesh
    mark = level.get("undo_mark", len(history.undo_stack))
    mark = min(mark, len(history.undo_stack))
    shared = edited.capture_state() != state0
    if parts is not None and (shared
                              or len(history.undo_stack) > mark):
        # An assembly: what ran inside was recorded with its matrix
        # down in the parts, so it folds into the step that brings
        # the matrix back — a part deleted or moved undoes in place.
        inner = history.undo_stack[mark:]
        del history.undo_stack[mark:]
        history.execute(ReshareAssemblyCommand(
            group, proto, xform, edited, state0, parts, inner))
    elif not shared:
        # Nothing ran: an assembly's parts go back bit for bit.
        restore_sharing(group, proto, xform, parts=parts)
    else:
        inner = history.undo_stack[mark:]
        del history.undo_stack[mark:]
        history.execute(ReshareInstanceCommand(
            group, proto, xform, edited, inner))
    return shared

"""The scale of a group or component, read and set the way Blender does:
three numbers (X, Y, Z) along the object's own axes, 1.0 meaning unscaled.

An instance keeps them in the columns of its ``xform``; a classic group
keeps its mesh in world coordinates and carries them in its ``axes``. The
object's frame is ``xform · axes`` (``core.group.group_frame``), the same
one its selection box follows: a group rotated and then made a component
has an identity ``xform`` and all its turn in ``axes``."""
from __future__ import annotations

from ingetrazo_core import Vec3 as QVector3D

from core.group import frame_axes, group_frame

_EPS = 1e-9


def _frame(group):
    return group_frame(group)


def group_scale(group) -> tuple[float, float, float]:
    """The object's scale along its own X, Y and Z."""
    frame = _frame(group)
    if frame is None:
        return (1.0, 1.0, 1.0)
    return tuple(frame.mapVector(QVector3D(*e)).length()
                 for e in ((1, 0, 0), (0, 1, 0), (0, 0, 1)))


def _pivot(group) -> QVector3D:
    """The point that stays put: an instance's origin, a classic group's
    bounding-box centre (its mesh is in world coordinates)."""
    if group.xform is not None:
        return frame_axes(_frame(group))[0]
    pts = [v.position for v in group.mesh.vertices]
    if not pts:
        return QVector3D()
    lo = [min(c) for c in zip(*((p.x(), p.y(), p.z()) for p in pts))]
    hi = [max(c) for c in zip(*((p.x(), p.y(), p.z()) for p in pts))]
    return QVector3D(*((a + b) / 2.0 for a, b in zip(lo, hi)))


def _exact_command(group, center, factor, axes):
    """``ScaleGroupCommand``: an instance is undone and redone by its
    transaction, so the saved matrices come back exactly (no float32
    residue piling up over repeated edits)."""
    from core.history import ScaleGroupCommand
    return ScaleGroupCommand(group, center, factor, axes)


def scale_command(group, target):
    """A command taking ``group`` to the scale ``target`` (X, Y, Z), or
    ``None`` when nothing would change or a value is not usable."""
    if any(t <= _EPS for t in target):
        return None
    now = group_scale(group)
    factor = tuple(t / n if n > _EPS else 1.0 for t, n in zip(target, now))
    if all(abs(f - 1.0) < 1e-6 for f in factor):
        return None
    axes = frame_axes(_frame(group))[1:]
    return _exact_command(group, _pivot(group), factor, axes)

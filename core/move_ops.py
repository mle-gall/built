# SPDX-License-Identifier: GPL-3.0-or-later
"""Moving and copying a selection: the one implementation behind the Move
tool's commit and the protocol's ``move_selection`` (docs/core-protocol.md
§1, docs/rust-tools.md §6)."""
from __future__ import annotations

from ingetrazo_core import CopySelection, MoveCopies as copy_builder, Matrix4 as QMatrix4x4

from ingetrazo_core.history import (Command, CompoundCommand, MoveDimensionsCommand,
                          MoveGroupCommand, MoveGuidesCommand,
                          MoveTextLabelsCommand, MoveVerticesCommand)


def copied_guide(guide, m: QMatrix4x4):
    """A new guide: ``guide`` carried by the transform ``m``. A circle stays
    a circle — centre mapped, plane normal rotated, radius kept — instead of
    collapsing into a guide line (Ctrl+copy of a guide circle, #238)."""
    from core.guide import Guide, GuideCircle
    if getattr(guide, "is_circle", False):
        return GuideCircle(m.map(guide.center),
                           m.mapVector(guide.normal),
                           guide.radius,
                           layer=getattr(guide, "layer", None))
    return Guide(m.map(guide.point),
                 None if guide.direction is None
                 else m.mapVector(guide.direction),
                 None if guide.origin is None else m.map(guide.origin))



def dimension_modes(dims, groups, verts) -> dict:
    """How each of ``dims`` moves with a selection: ``"line"`` — both ends
    anchored to geometry: the dimension line slides and the extension
    lines stretch from their vertices; ``"rigid"`` — a free end travels
    with the delta. A dimension whose two anchors are themselves moving
    (their vertices, or a group holding them) is left out: it rides along
    with its vertices, and shifting its line too would move it twice."""
    moving_verts = {v.key for v in verts}
    moving_groups = {id(g) for g in groups}
    out: dict = {}
    for d in dims:
        anchors = [an for an in (d.anchor_a, d.anchor_b) if an is not None]
        riding = [an for an in anchors
                  if an.vertex.key in moving_verts
                  or any(id(g) in moving_groups for g in an.chain)]
        if len(anchors) == 2 and len(riding) == 2:
            continue                          # follows its vertices
        out[d] = "line" if len(anchors) == 2 else "rigid"
    return out


def move_commands(delta, *, groups=(), positions=(), labels=(), guides=(),
                  dims=None, splanes=(), images=()) -> list:
    """The commands that move these things by ``delta``: each group as a
    unit, the loose positions (coincident vertices along, warped faces
    fold, textures carried), labels, guides, dimensions, section planes and
    reference images."""
    commands = [MoveGroupCommand(g, delta) for g in groups]
    if positions:
        commands.append(MoveVerticesCommand(list(positions), delta, carry_textures=True))
    if labels:
        commands.append(MoveTextLabelsCommand(list(labels), delta))
    if guides:
        commands.append(MoveGuidesCommand(list(guides), delta))
    if dims:
        commands.append(MoveDimensionsCommand(dims, delta))
    if splanes:
        from ingetrazo_core.history import MoveSectionPlanesCommand
        commands.append(MoveSectionPlanesCommand(list(splanes), delta))
    if images:
        from ingetrazo_core.history import MoveImagePlanesCommand
        commands.append(MoveImagePlanesCommand(list(images), delta))
    return commands


class _PlannedWhenRun(Command):
    """Drawn edges planned when the command RUNS, not when it is built:
    by then the copied faces before it in the same compound are in the
    model, so the Line tool's planner splits the faces the edges land on
    without laying another face over the copied ones (#379). Redo
    replays the same plan. Supplied faces are kept verbatim: empty cycles,
    including holes and open internal rims, must stay empty (#1087)."""

    def __init__(self, segments) -> None:
        self._segments = list(segments)
        self._cmd = None

    def do(self, scene) -> None:
        if self._cmd is None:
            from core.edits import build_add_edges
            self._cmd = build_add_edges(scene, self._segments,
                                        preserve_faces=True, plane_rebuild=True)
        self._cmd.do(scene)

    def undo(self, scene) -> None:
        if self._cmd is not None:
            self._cmd.undo(scene)


class _MoveCopiesCommand(CompoundCommand):
    """Select the entire copy/array; undo restores the source selection."""

    def __init__(self, commands, groups, guides, faces, segments) -> None:
        super().__init__(commands)
        self._selection = CopySelection(groups, guides, faces, segments)

    # Selection lives outside the document journal.
    doc_undo = False

    def do(self, scene):
        self._selection.execute(scene, super().do)

    def undo(self, scene):
        self._selection.undo(scene, super().undo)

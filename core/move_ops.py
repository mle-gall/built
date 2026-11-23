# SPDX-License-Identifier: GPL-3.0-or-later
"""Moving and copying a selection: the one implementation behind the Move
tool's commit and the protocol's ``move_selection`` (docs/core-protocol.md
§1, docs/rust-tools.md §6)."""
from __future__ import annotations

from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from core.group import copy_group, transformed_attrs
from core.history import (AddEdgeCommand, AddFaceCommand, AddGuideCommand,
                          Command, CommandRejected, CompoundCommand,
                          InsertGroupCommand, MoveDimensionsCommand,
                          MoveGroupCommand, MoveGuidesCommand,
                          MoveTextLabelsCommand, MoveVerticesCommand)
from core.mesh import Mesh
from core.topology import _key


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
        from core.history import MoveSectionPlanesCommand
        commands.append(MoveSectionPlanesCommand(list(splanes), delta))
    if images:
        from core.history import MoveImagePlanesCommand
        commands.append(MoveImagePlanesCommand(list(images), delta))
    return commands


def copy_builder(scene, groups, faces, edges, guides):
    """A closure that builds the copy command for a list of offsets — one
    copy per offset (an array is several).

    Stamp the supplied topology verbatim. Only edges touching geometry
    outside their copy need the intersection planner (#177, #379).
    Replanning the copy against itself split imported faces and invented
    floors across empty rims, depending on selection order (#1087)."""
    groups, faces, edges, guides = list(groups), list(faces), list(edges), list(guides)

    def build(deltas):
        from core.edits import touching_segments
        from core.geometry import Edge as GeomEdge, Face as GeomFace
        deltas = list(deltas)
        cmds: list = []
        copied_groups, copied_guides, copied_faces, copied_segments = [], [], [], []
        # Other array members are outside this copy too. Build their
        # geometric views before execution, so array order is irrelevant.
        copies = []
        for d in deltas:
            ef = [GeomEdge(e.a + d, e.b + d) for e in edges]
            ff = [GeomFace([v + d for v in f.vertices],
                           [[v + d for v in h] for h in f.holes]) for f in faces]
            copies.append((ef, ff))
        for index, d in enumerate(deltas):
            planned = []
            if scene is not None:
                outside_edges, outside_faces = list(scene.edges), list(scene.faces)
                for j, (ef, ff) in enumerate(copies):
                    if j != index:
                        outside_edges.extend(ef)
                        outside_faces.extend(ff)
                hard = [(e.a + d, e.b + d) for e in edges
                        if e.curve is None and not e.soft]
                planned = touching_segments(hard, outside_edges, outside_faces)
            planned_keys = {(_key(a), _key(b)) for a, b in planned}
            from core.ifc_identity import copied_face_attrs
            copy_attrs = copied_face_attrs()
            m = QMatrix4x4()
            m.translate(d)
            for group in groups:
                clone = copy_group(group, d)
                copied_groups.append(clone)
                cmds.append(InsertGroupCommand(clone))
            for g in guides:
                clone = copied_guide(g, m)
                copied_guides.append(clone)
                cmds.append(AddGuideCommand(clone))
            for f in faces:
                cmd = AddFaceCommand(
                    [v + d for v in f.vertices],
                    holes=[[v + d for v in h] for h in f.holes] or None,
                    auto=False,
                    attrs=copy_attrs(transformed_attrs(f.attrs, m)),
                    interior=f.interior,
                )
                copied_faces.append(cmd)
                cmds.append(cmd)
            id_map: dict[int, int] = {}
            for e in edges:
                copied_segments.append((e.a + d, e.b + d))
                curve = getattr(e, "curve", None)
                soft = getattr(e, "soft", False)
                if scene is not None and curve is None and not soft:
                    if (_key(e.a + d), _key(e.b + d)) in planned_keys:
                        continue
                # Soft edges and curves keep their flags on the copy.
                if curve is not None and curve not in id_map:
                    id_map[curve] = Mesh.next_curve_id()
                cmds.append(AddEdgeCommand(
                    e.a + d, e.b + d,
                    soft=soft or None,
                    curve=id_map.get(curve)))
            if planned:
                cmds.append(_PlannedWhenRun(planned))
        if not cmds:
            return None
        return _MoveCopiesCommand(cmds, copied_groups, copied_guides,
                                  copied_faces, copied_segments)

    return build


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


def _copied_edges(mesh, segments):
    """Resolve copied strokes after welding and splitting, using incidence.

    A copied edge may now be several edges, or reuse an existing one. Walk
    only its collinear chain rather than scanning the whole model per edge.
    """
    picked = set()
    for a, b in segments:
        start, end = mesh.vertex_at(a), mesh.vertex_at(b)
        if start is None or end is None or start is end:
            continue
        ab = b - a
        length2 = ab.lengthSquared()
        if length2 < 1e-18:
            continue
        todo, seen = [start], {start}
        while todo:
            vertex = todo.pop()
            for edge in vertex.edges:
                other = edge.v1 if edge.v0 == vertex else edge.v0
                if other != start and other != end:
                    p = other.position
                    t = QVector3D.dotProduct(p - a, ab) / length2
                    if not 0 <= t <= 1 or (p - (a + ab * t)).length() > 1e-4:
                        continue
                picked.add(edge)
                if other not in seen:
                    seen.add(other)
                    todo.append(other)
    return picked


class _MoveCopiesCommand(CompoundCommand):
    """Select the entire copy/array; undo restores the source selection."""

    def __init__(self, commands, groups, guides, faces, segments) -> None:
        super().__init__(commands)
        self._groups = groups
        self._guides = guides
        self._faces = faces
        self._segments = segments
        self._before_selection = None
        self._context = None

    def doc_undo(self, scene) -> bool:
        # Selection is outside the document journal. Replay this wrapper.
        return False

    def do(self, scene) -> None:
        before = set(scene.selection)
        scene.selection = set(before)
        try:
            super().do(scene)
        except CommandRejected:
            # A late refusal may already have inserted (and undone) groups,
            # whose commands each replace the selection.
            scene.selection = before
            scene.bump_view()
            raise
        if self._before_selection is None:
            self._before_selection = before
            self._context = scene.edit_group
        if scene.edit_group is not self._context:
            # History can replay this mesh after its group was closed.
            # Its contents must not become a selection in the outer model.
            scene.selection = before
            scene.bump_view()
            return
        picked = set(self._groups) | set(self._guides)
        if self._faces:
            live = set(scene.mesh.faces)
            picked.update(cmd.face for cmd in self._faces if cmd.face in live)
        picked.update(_copied_edges(scene.mesh, self._segments))
        scene.selection = picked
        scene.bump_view()

    def undo(self, scene) -> None:
        picked = (self._before_selection if scene.edit_group is self._context
                  else set(scene.selection))
        super().undo(scene)
        scene.selection = set(picked or ())
        scene.bump_view()



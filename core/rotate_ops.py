# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Rotate History commands for loose geometry, placements and model annotations."""
from core.group import copy_group, transformed_attrs
from core.history import (AddEdgeCommand, AddFaceCommand, AddGuideCommand, Command,
                          CompoundCommand, InsertGroupCommand, RotateGroupCommand,
                          RotateGuidesCommand, RotateVerticesCommand, rotation_matrix)
from core.mesh import Mesh
from core.move_ops import copied_guide


class _MergeCopiedFaces(Command):
    """Merge only overlaps between a rotated copy and geometry outside it.

    All copies are stamped first. One lazy snapshot protects their combined
    mesh edit; undo restores the stamped faces before their add commands
    remove them. Disjoint copies need no snapshot and retain their topology.
    """

    keeps_groups = True

    def __init__(self, batches) -> None:
        self._batches = batches
        self._before = self._after = None
        self._selection_before = self._selection_after = None
        self._context = None

    def do(self, scene) -> None:
        if self._after is not None:
            scene.mesh.restore_state(self._after)
            if scene.edit_group is self._context:
                scene.selection = set(self._selection_after)
            scene.version += 1
            return
        from core.history import _merge_overlapping_planes

        def checkpoint():
            if self._before is None:
                self._before = scene.mesh.capture_state()
                self._selection_before = set(scene.selection)
                self._context = scene.edit_group

        pending = {c.face for batch in self._batches for c in batch if c.face is not None}
        for batch in self._batches:
            added = {c.face for c in batch if c.face is not None}
            pending.difference_update(added)
            # Include earlier array members (and their rebuilt regions), but
            # never merge the supplied faces of one copy against each other.
            before = [f for f in scene.mesh.faces if f not in pending and f not in added]
            _merge_overlapping_planes(scene, before, [c.face for c in batch],
                                       on_rebuild=checkpoint)
        if self._before is not None:
            self._after = scene.mesh.capture_state()
            self._selection_after = set(scene.selection)

    def undo(self, scene) -> None:
        if self._before is not None:
            scene.mesh.restore_state(self._before)
            if scene.edit_group is self._context:
                scene.selection = set(self._selection_before)
            scene.version += 1


def rotation_command(start, axis, copy, groups, positions, faces, edges,
                     guides, splanes, images, degs):
    """A single rotation or a fan of copies is one History command."""
    degs = [float(d) for d in degs]
    cmds: list = []
    if copy:
        id_map: dict[int, int] = {}
        face_batches = []
        for deg in degs:
            batch = []
            from core.ifc_identity import copied_face_attrs
            copy_attrs = copied_face_attrs()
            m = rotation_matrix(start, axis, deg)
            for group in groups:
                g = copy_group(group)  # instance → sibling instance
                cmds.append(InsertGroupCommand(g))
                cmds.append(RotateGroupCommand(g, start, axis, deg))
            for gd in guides:
                cmds.append(AddGuideCommand(copied_guide(gd, m)))
            for f in faces:
                face_cmd = AddFaceCommand(
                    [m.map(v) for v in f.vertices],
                    holes=[[m.map(v) for v in h] for h in f.holes]
                    or None,
                    auto=False,
                    attrs=copy_attrs(transformed_attrs(f.attrs, m)),
                )
                cmds.append(face_cmd)
                batch.append(face_cmd)
            if batch:
                face_batches.append(batch)
            for e in edges:
                curve = getattr(e, "curve", None)
                # One fresh curve id per (curve, copy): the arms of
                # an array are separate curves, not one long one.
                key = None if curve is None else (curve, deg)
                if key is not None and key not in id_map:
                    id_map[key] = Mesh.next_curve_id()
                cmds.append(AddEdgeCommand(
                    m.map(e.a), m.map(e.b),
                    soft=getattr(e, "soft", False) or None,
                    curve=id_map.get(key)))
        if face_batches:
            cmds.append(_MergeCopiedFaces(face_batches))
    else:
        # Nothing is copied, so there is only ever one angle.
        deg = degs[-1]
        cmds.extend(RotateGroupCommand(g, start, axis, deg)
                    for g in groups)
        if positions:
            cmds.append(RotateVerticesCommand(
                positions, start, axis, deg, carry_textures=True))
        if splanes:
            from core.history import RotateSectionPlanesCommand
            cmds.append(RotateSectionPlanesCommand(
                splanes, start, axis, deg))
        if images:
            from core.history import RotateImagePlanesCommand
            cmds.append(RotateImagePlanesCommand(
                images, start, axis, deg))
        if guides:
            cmds.append(RotateGuidesCommand(
                guides, start, axis, deg))
    if not cmds:
        return None
    return cmds[0] if len(cmds) == 1 else CompoundCommand(cmds)

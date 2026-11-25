# SPDX-License-Identifier: GPL-3.0-or-later
"""Soften / Smooth Edges: the angle-based smoothing an import runs
(:func:`formats.fuse.soften_smooth_edges`) as an undoable user command.

As in the modeller's Soften Edges, softening hides the line and «Smooth
normals» (on by default) also shades the faces across it smoothly, in the
GL viewport and in the Metal view alike (the shared corner-fan kernel).
With it off the edge is soft only (``Edge.flat``): no line, but each face
keeps its own flat shade. The angle is read as the import
reads it, between the two faces whatever their winding, so a face drawn the
other way round does not leave a seam. By default an edge between two
materials keeps its line, as an import keeps it.

Only edges between exactly two faces change: a border or a wire keeps what
it has. No topology changes, and hidden edges stay hidden."""
from __future__ import annotations

import math

import ingetrazo_core

from core.group import Group
from core.history import MeshCommand
from core.mesh import Edge, Face


def _outside_the_host(group) -> bool:
    """A linked file's parts are edited in their own document, and a
    locked group is not edited at all."""
    return (getattr(group, "locked", False)
            or ingetrazo_core.link_linked_ancestor(group) is not None)


def selected_edges(scene) -> dict:
    """``{mesh: {edge keys}}`` the selection asks to smooth, one entry per
    mesh: a definition shared by several copies is processed once (and its
    copies share the change).

    Selected edges count as they are; selected faces bring the seams
    between two of them, not their outline; a selected group or component
    brings every edge of its geometry and of what is nested in it."""
    targets: dict = {}
    mesh = scene.mesh
    faces = {f for f in scene.selection if isinstance(f, Face)}
    keys = {e.key for e in scene.selection if isinstance(e, Edge)}
    if faces:
        for e in mesh.edges:
            if len(e.faces) == 2 and all(f in faces for f in e.faces):
                keys.add(e.key)
    if keys:
        targets[mesh] = keys
    from core import links
    pending = [g for g in scene.selection
               if isinstance(g, Group) and not _outside_the_host(g)]
    seen = set()
    while pending:
        group = pending.pop()
        if id(group) in seen:
            continue
        seen.add(id(group))
        if getattr(group, "locked", False) or links.is_linked(group):
            continue
        pending.extend(group.children or ())
        m = group.mesh
        if m is None:
            continue
        targets.setdefault(m, set()).update(e.key for e in m.edges)
    return targets


def _smooth_signature(face):
    from formats.fuse import _smooth_sig
    return _smooth_sig(face.attrs)


class SoftenByAngleCommand(MeshCommand):
    """Soften the edges of ``targets`` (``{mesh: {edge keys}}``) whose two
    faces meet at ``angle`` degrees or less; harden the sharper ones.

    ``coplanar``: soften the edges between faces in one plane too.
    ``materials``: soften across two materials too (an import keeps that
    line). ``smooth``: the softened edges shade smooth too (the modeller's
    «Smooth normals»); off, they are soft only (``Edge.flat``).
    ``harden``: unsoften every edge instead.

    The change is worked out once, as ``(mesh, key, before, after)`` per
    edge that changes, each a ``(soft, flat)`` pair; undo and redo replay
    it by key, which survives the other commands' undo."""

    keeps_bounds = True
    keeps_rows = True

    def __init__(self, targets, angle: float = 30.0, coplanar: bool = True,
                 harden: bool = False, materials: bool = False,
                 smooth: bool = True) -> None:
        self.targets = targets
        self.angle = float(angle)
        if not math.isfinite(self.angle) or not 0.0 <= self.angle <= 180.0:
            raise ValueError("Angle must be between 0 and 180 degrees")
        self.coplanar = bool(coplanar)
        self.harden = bool(harden)
        self.materials = bool(materials)
        self.smooth = bool(smooth)
        self.changes = None

    def geometry_meshes(self, scene):
        return self.targets

    def meshes(self) -> list:
        """The meshes this command changes."""
        out, seen = [], set()
        for mesh, _key, _old, _new in self.changes or ():
            if id(mesh) not in seen:
                seen.add(id(mesh))
                out.append(mesh)
        return out

    def prepare(self) -> list:
        from core.store_mesh import FaceView, _hypot, _store_of
        from ingetrazo_core import MeshStore
        changes = []
        for mesh, keys in self.targets.items():
            store = _store_of(mesh)
            options = (self.angle, self.coplanar, self.harden, self.smooth)
            if store is not None:
                def matches(a, b):
                    return _smooth_signature(FaceView(mesh, a)) == _smooth_signature(FaceView(mesh, b))

                planned = store.plan_softening(set(keys), *options, None if self.materials else matches, _hypot())
            else:
                faces = {}
                rows = []
                for e in mesh.edges:
                    if e.key in keys and len(e.faces) == 2:
                        a, b = e.faces
                        faces[a.key], faces[b.key] = a, b
                        rows.append((e.key, (bool(e.soft), bool(getattr(e, "flat", False))), (a.key, b.key)))

                def matches(a, b):
                    return _smooth_signature(faces[a]) == _smooth_signature(faces[b])

                planned = MeshStore.plan_object_softening(rows, *options, None if self.materials else matches,
                                                          lambda k: faces[k].normal().toTuple())
            changes.extend((mesh, key, old, new) for key, old, new in planned)
        return changes

    def _apply(self, scene, after: bool) -> None:
        from core.store_mesh import _store_of
        from ingetrazo_core import MeshStore
        by_mesh = {}
        for mesh, key, old, new in self.changes:
            by_mesh.setdefault(mesh, []).append((key, old, new))
        for mesh, rows in by_mesh.items():
            store = _store_of(mesh)
            if store is None:
                MeshStore.apply_object_softening(mesh.edges, rows, after)
            else:
                store.apply_softening(rows, after)
            mesh.mark_modified()
        if by_mesh:
            scene.version += 1

    def do(self, scene) -> None:
        if self.changes is None:
            self.changes = self.prepare()
        self._apply(scene, True)

    def undo(self, scene) -> None:
        self._apply(scene, False)

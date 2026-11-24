# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Clipboard History operations shared by both viewports and the agent protocol."""
from ingetrazo_core import Vec3 as QVector3D
from core.mesh import element_identity
from core.group import copy_group, translated_attrs
from core.history import (AddEdgeCommand, AddFaceCommand, Command,
                          CompoundCommand, InsertGroupCommand)


class _PruneOnUndo(Command):
    """First in a stamp's compound, so its UNDO runs last: after the stamp's
    faces and edges are gone, the vertices nothing uses any more go too.
    ``remove_edge`` only detaches, and the leftovers kept the edit box of a
    group (which wraps its vertices) at the pasted size after Ctrl+Z. Redo
    puts them back before the geometry is relinked onto them. The pasted
    faces leave the selection here too: undoing a face does not take it out,
    and the next Move would carry the corners of one no longer there."""

    keeps_groups = True        # only vertices nothing uses

    def __init__(self) -> None:
        self._gone: list = []

    def do(self, scene) -> None:
        if self._gone:
            scene.mesh.relink_vertices(self._gone)
        self._gone = []

    def undo(self, scene) -> None:
        m = scene.mesh
        before = {v.key: v for v in m.vertices}
        m.prune_orphan_vertices()
        kept = {v.key for v in m.vertices}
        self._gone = [v for k, v in before.items() if k not in kept]
        from core.mesh import Face
        live = {f.key for f in m.faces}
        for ent in list(scene.selection):
            if isinstance(ent, Face) and ent.key not in live:
                scene.selection.discard(ent)
        scene.version += 1


def stamp_clipboard(history, clip, off):
    """Insert the clipboard's contents and select the surviving copies."""
    scene = history.scene
    from core.ifc_identity import copied_face_attrs
    copy_attrs = copied_face_attrs(scene)
    from core.layers import template_name_map
    names = template_name_map(scene, clip.get("layers") or {})

    from core.links import _renamed_paint, restamp_materials
    from core.materials import register
    from core.history import SetFaceMaterialTagCommand
    commands: list = [_PruneOnUndo()]
    renamed = {}
    if clip.get("materials"):
        registry = dict(scene.materials)
        for old_name, mat in clip["materials"].items():
            name = register(registry, mat)
            if name != old_name:
                renamed[old_name] = name
            if name not in scene.materials:
                commands.append(SetFaceMaterialTagCommand([], name, registry[name]))

    def remap_attrs(attrs):
        out = copy_attrs(attrs)
        if out.get("layer") in names:
            out["layer"] = names[out["layer"]]
        return _renamed_paint(out, renamed)

    # The tags the copy carries that this document lacks (with their
    # folders) come first, undone with the paste; tags it already has
    # keep their own look and visibility.
    if clip.get("layers"):
        from core.history import EnsureLayersCommand
        commands.append(EnsureLayersCommand(clip["layers"]))
    guards = len(commands)
    # A face's boundary edges are created BY the face; undoing the face
    # alone leaves them behind as loose edges (inside a group they read
    # as debris). Adding them first, as commands of their own, makes the
    # stamp own them: undo removes the faces, then exactly the edges it
    # created — an edge that already existed is never touched.
    for loop, holes, *_r in clip["faces"]:
        for lp in (loop, *holes):
            n = len(lp)
            for i in range(n):
                commands.append(AddEdgeCommand(
                    lp[i] + off, lp[(i + 1) % n] + off))
    for loop, holes, *rest in clip["faces"]:
        # Copied attrs travel onto the pasted face; a positioned texture's
        # world-anchored UV map is re-fitted to the paste offset.
        attrs = translated_attrs(rest[0], off) if rest and rest[0] else None
        commands.append(AddFaceCommand(
            [p + off for p in loop],
            holes=[[p + off for p in h] for h in holes] or None,
            auto=False,
            attrs=remap_attrs(attrs),
        ))
    # A face pasted onto a plane that already carries faces merges with
    # them, as a rectangle drawn there does: overlapping ones split into
    # their regions instead of lying one on top of the other (issue #73,
    # @pacaeiro: «if you copy a rectangle on top of the previous one,
    # the engine is not activated»).
    commands.extend(_plane_merges(scene, [
        [p + off for p in loop] for loop, _holes, *_r in clip["faces"]]))
    # Soft/curve flags travel with the copy; curve ids are remapped to
    # FRESH ones so each pasted circle/arc is its own selectable contour
    # (never entangled with the original's).
    from core.mesh import Mesh
    id_map: dict[int, int] = {}
    edge_layers = list(clip.get("edge_layers") or ())
    for i, (a, b, soft, curve) in enumerate(clip["edges"]):
        if curve is not None and curve not in id_map:
            id_map[curve] = Mesh.next_curve_id()
        commands.append(AddEdgeCommand(
            a + off, b + off, soft=soft or None,
            curve=id_map.get(curve),
            layer=names.get(edge_layers[i], edge_layers[i]) if i < len(edge_layers) else None))
    # Each stamp builds FRESH group copies from the clipboard templates
    # (instances stay siblings of the same prototype).
    pasted_groups = [copy_group(g, off)
                     for g in clip.get("groups", ())]
    if renamed or any(a != b for a, b in names.items()):
        from core.group import iter_placements, transformed_mesh
        from ingetrazo_core import Matrix4 as QMatrix4x4
        for root in pasted_groups:
            for g, _ in iter_placements(root):
                g.layer = names.get(g.layer, g.layer)
                # A renamed target tag must not rename the source definition.
                g.mesh = transformed_mesh(g.mesh, QMatrix4x4())
                if renamed:
                    restamp_materials(g.mesh, renamed)
                    if g.material:
                        g.material = _renamed_paint(g.material, renamed)
                for f in g.mesh.faces:
                    if f.attrs.get("layer") in names:
                        f.attrs["layer"] = names[f.attrs["layer"]]
                for e in g.mesh.edges:
                    e.layer = names.get(e.layer, e.layer)
    commands.extend(InsertGroupCommand(g) for g in pasted_groups)
    if len(commands) == guards:
        return False        # only the guards: nothing to stamp
    before = history.revision
    history.execute(CompoundCommand(commands))
    if history.revision == before:
        return False
    picked = list(pasted_groups)
    live = set(map(element_identity, scene.mesh.faces))
    picked += [c.face for c in commands
               if isinstance(c, AddFaceCommand)
               and c.face is not None and c.face.key in live]
    # The copied edges themselves (looked up by position: a copy
    # pasted in place welds onto the original's edges, so the
    # command may own none of them).
    mesh = scene.mesh
    for a, b, _s, _c in clip["edges"]:
        v0, v1 = mesh.vertex_at(a + off), mesh.vertex_at(b + off)
        e = mesh.find_edge(v0, v1) if v0 and v1 else None
        if e is not None:
            picked.append(e)
    # InsertGroupCommand selects only the last one — select them all.
    scene.select(picked)
    return True

#: More distinct planes than this in one paste is 3D geometry being moved
#: about, not a flat drawing laid over another: no merge is attempted.
_MERGE_PLANES_CAP = 24


def _plane_merges(scene, loops) -> list:
    """Scoped plane rebuilds for the planes of the pasted ``loops`` that
    already carry faces of the target mesh (the drawing tools' own
    ``RebuildPlaneFacesCommand``). Runs after the faces are added, so the
    pasted faces cover their own regions."""
    from core.history import RebuildPlaneFacesCommand
    from core.triangulate import _newell
    planes: dict = {}
    for loop in loops:
        if len(loop) < 3:
            continue
        n = _newell(loop)
        length = n.length() if n is not None else 0.0
        if length < 1e-9:
            continue
        n = n / length
        if n.z() < 0 or (n.z() == 0 and (n.y() < 0 or (n.y() == 0 and n.x() < 0))):
            n = -n
        d = QVector3D.dotProduct(n, loop[0])
        key = (round(n.x(), 4), round(n.y(), 4), round(n.z(), 4), round(d, 4))
        entry = planes.setdefault(key, [QVector3D(loop[0]), n, None, None])
        for p in loop:
            xyz = (p.x(), p.y(), p.z())
            entry[2] = xyz if entry[2] is None else tuple(map(min, entry[2], xyz))
            entry[3] = xyz if entry[3] is None else tuple(map(max, entry[3], xyz))
        if len(planes) > _MERGE_PLANES_CAP:
            return []
    if not planes or not scene.mesh.faces:
        return []
    # Which planes already hold geometry: one vectorised pass over the
    # mesh's vertices (a paste beside a big model must stay instant); only
    # then the faces, and only those on that plane whose box meets the
    # pasted one — a face pasted BESIDE another leaves it alone.
    import numpy as np
    pts = np.array([[v.position.x(), v.position.y(), v.position.z()]
                    for v in scene.mesh.vertices], dtype=np.float64)
    out = []
    tol = 1e-4
    for origin, n, lo, hi in planes.values():
        nv = np.array([n.x(), n.y(), n.z()])
        d = float(np.dot(nv, [origin.x(), origin.y(), origin.z()]))
        if np.count_nonzero(np.abs(pts @ nv - d) < tol) < 3:
            continue
        for f in scene.mesh.faces:
            vs = f.vertices
            if any(abs(QVector3D.dotProduct(v - origin, n)) > tol for v in vs):
                continue
            flo = [min(v.x() for v in vs), min(v.y() for v in vs), min(v.z() for v in vs)]
            fhi = [max(v.x() for v in vs), max(v.y() for v in vs), max(v.z() for v in vs)]
            if all(flo[i] < hi[i] - tol and lo[i] < fhi[i] - tol
                   or abs(fhi[i] - flo[i]) < tol and abs(hi[i] - lo[i]) < tol
                   for i in range(3)):
                out.append(RebuildPlaneFacesCommand(origin, n))
                break
    return out

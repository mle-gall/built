# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Reference execution for drawing commands with a native store implementation.

The object mesh, Python document and mixed transactions use these inverses.
"""
from core.topology import _key, _loop_edges


def stamp_edge(self, edge) -> None:
    if self.soft is not None:
        edge.soft = self.soft
    if self.curve is not None:
        edge.curve = self.curve
    if self.layer is not None:
        from core.layers import assign_layer
        assign_layer(edge, self.layer)


def draw_edge(self, scene) -> None:
    m = scene.mesh
    if self._owned and self.edge is not None:
        if getattr(self, "_made_vertices", ()):
            m.relink_vertices(self._made_vertices)
        m.relink_edge(self.edge)  # redo of an edge this command owns
        self._stamp(self.edge)
        scene.version += 1
        return
    v0 = m.vertex_at(self.a)
    v1 = m.vertex_at(self.b)
    pre = m.find_edge(v0, v1) if (v0 is not None and v1 is not None) else None
    if pre is not None:
        # The mesh already has this edge — merged no-op. Own nothing, so
        # ``self.edge`` stays None and undo leaves the pre-existing edge.
        self._stamp(pre)
        if self.soft is not None or self.curve is not None or self.layer is not None:
            m.mark_modified()
        scene.version += 1
        return
    self.edge = m.add_edge(self.a, self.b)
    self._owned = True
    if not self._journal_geometry:
        self._made_vertices = [v for prior, v in ((v0, self.edge.v0), (v1, self.edge.v1))
                               if prior is None]
    self._stamp(self.edge)
    scene.version += 1


def undo_draw_edge(self, scene) -> None:
    if not self._owned or self.edge is None:
        return
    scene.mesh.remove_edge(self.edge)
    if getattr(self, "_made_vertices", ()):
        scene.mesh.drop_orphan_vertices(self._made_vertices)
    scene.selection.discard(self.edge)
    scene.version += 1


def delete_edges(self, scene) -> None:
    from core.history_commands import _faces_by_segments
    self._before = self._reference_snapshot(scene.mesh)
    m = scene.mesh
    self.removed_edges = []
    for a, b in self._endpoints:
        v0 = m.vertex_at(a)
        v1 = m.vertex_at(b)
        edge = m.find_edge(v0, v1) if (v0 is not None and v1 is not None) else None
        if edge is not None:
            self.removed_edges.append(edge)

    if self.cascade_faces:
        gone = {frozenset((_key(a), _key(b))) for a, b in self._endpoints}
        self.removed_faces = [
            f for f in _faces_by_segments(m, self._endpoints)
            if set(_loop_edges(f.vertices)) & gone
        ]
        for f in self.removed_faces:
            m.remove_face(f)
            scene.selection.discard(f)

    for edge in self.removed_edges:
        m.remove_edge(edge)
        scene.selection.discard(edge)
    scene.version += 1


def delete_face(self, scene) -> None:
    from core.history_commands import _find_face_by_loop
    self._before = self._reference_snapshot(scene.mesh)
    self.face = _find_face_by_loop(scene.mesh, self._loop)
    if self.face is not None:
        scene.mesh.remove_face(self.face)
        scene.selection.discard(self.face)
    scene.version += 1


def explode_curves(self, scene) -> None:
    ids = {e.curve for e in self.edges
           if getattr(e, "curve", None) is not None}
    self.curves = len(ids)
    self._ids = [(e, e.curve) for e in scene.mesh.edges
                 if e.curve in ids]
    for e, _cid in self._ids:
        e.curve = None
    scene.mesh._chunk_dirty = True
    scene.mesh._mut_serial += 1
    scene.version += 1


def undo_explode_curves(self, scene) -> None:
    for e, cid in self._ids:
        e.curve = cid
    scene.mesh._chunk_dirty = True
    scene.mesh._mut_serial += 1
    scene.version += 1


def divide_drawn_face(m, face):
    from ingetrazo_core import Vec3 as QVector3D
    from core.mesh import Face, Vertex
    from core.history_commands import _faces_holding, _faces_inside, _inherit_paint
    from core.topology import (carve_loop_by_chords, find_containing_face, hole_merged_with_loop,
                               loop_inside_face, subtract_loop_from_face)
    punched = swapped = False
    # Direction A: the new face falls inside an existing mother → the mother
    # gains the new loop as a hole.
    mother = find_containing_face(_faces_holding(m, face.vertices),
                                  face.vertices, exclude=face)
    if mother is not None:
        grown = hole_merged_with_loop(mother, face.vertices)
        if grown is not None:
            idx, outline = grown
            old_loop = mother.hole_loops[idx]
            m.remove_hole(mother, old_loop)
            m.add_hole(mother, outline)
            swapped = True
        else:
            m.add_hole(mother, face.vertices)
            punched = True
        _inherit_paint(face, mother)

    # Direction B: the new face encloses existing smaller faces → it gains
    # each of them as a hole.
    # A face with a corner off the new face's plane is not inside it
    # (the first thing loop_inside_face checks): said here with the
    # plane worked out once, and not once per face of the model.
    from core.topology import _PLANAR_TOLERANCE
    plane_n = face.normal()
    nx, ny, nz = plane_n.x(), plane_n.y(), plane_n.z()
    plane_d = (QVector3D.dotProduct(plane_n, face.loop[0].position)
               if face.loop else None)
    for other in _faces_inside(m, face):
        if other == face:
            continue
        if plane_d is not None:
            off = False
            for v in other.loop:
                x, y, z = v.position.toTuple()
                if abs(nx * x + ny * y + nz * z - plane_d) \
                        > 2.0 * _PLANAR_TOLERANCE:
                    off = True
                    break
            if off:
                continue
        if loop_inside_face(face, other.vertices):
            m.add_hole(face, other.vertices)
            punched = True

    # Direction C: drawn against an existing face's boundary (corner / edge
    # rectangle) → carve a connected sub-region. Only when no hole applied.
    if not punched and not swapped:
        for other in _faces_holding(m, face.vertices):
            if other == face:
                continue
            remainder = subtract_loop_from_face(other, face.vertices)
            pieces = ([remainder] if remainder is not None
                      else carve_loop_by_chords(other, face.vertices))
            if not pieces:
                continue
            # Each hole of the mother goes to the piece that holds it; a
            # hole straddling a cut leaves the mother alone.
            piece_holes: list[list[list[QVector3D]]] = [[] for _ in pieces]
            straddle = False
            for hole in other.holes:
                for k, piece in enumerate(pieces):
                    if loop_inside_face(Face([Vertex(v) for v in piece]), hole):
                        piece_holes[k].append([QVector3D(v) for v in hole])
                        break
                else:
                    straddle = True
                    break
            if straddle:
                continue
            m.remove_face(other)
            for piece, holes in zip(pieces, piece_holes):
                rem_face = m.add_face(piece, holes or None)
                rem_face.attrs = dict(other.attrs)  # carved mother continues
            _inherit_paint(face, other)    # ...and so does the cut-out
            break

    return swapped

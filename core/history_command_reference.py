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

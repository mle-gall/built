# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Reference execution for drawing commands with a native store implementation.

The object mesh, Python document and mixed transactions use these inverses.
"""
from typing import Optional
from ingetrazo_core import Vec3 as QVector3D

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


def _single_coplanar_pair(faces) -> Optional[tuple]:
    """The one pair of ``faces`` that face the same way (they share an edge,
    so that makes them coplanar), or ``None`` when there is no such pair,
    more than one, or another face lies in their plane too."""
    pairs = [(a, b) for i, a in enumerate(faces) for b in faces[i + 1:]
             if QVector3D.dotProduct(a.normal(), b.normal()) > 0.999]
    if len(pairs) != 1:
        return None
    n = pairs[0][0].normal()
    if any(abs(QVector3D.dotProduct(n, f.normal())) > 0.999
           for f in faces if f not in pairs[0]):
        return None
    return pairs[0]


class _LoopIndex:
    """``{corner keys: [faces with them, in list order]}`` of the mesh as
    it is now — what the erase used to build over every face — filled only
    for the keys of ``loops`` (the faces being erased), from the mesh scan.
    Without a scan (or for many loops) it is that full index."""

    def __init__(self, mesh, loops) -> None:
        self._d: dict = {}
        from core import scan_index
        from core.history_commands import _faces_keyed_like
        loops = [list(lp) for lp in loops]
        sc = scan_index.scan(mesh) if 0 < len(loops) <= 256 else None
        if sc is None:
            if loops:
                for f in mesh.faces:
                    self._d.setdefault(
                        frozenset(_key(p) for p in f.vertices), []).append(f)
            return
        for loop in loops:
            target = frozenset(_key(p) for p in loop)
            if target in self._d:
                continue
            self._d[target] = [
                f for f in _faces_keyed_like(mesh, loop, sc)
                if frozenset(_key(p) for p in f.vertices) == target]

    def get(self, target):
        return self._d.get(target)


class _SegmentIndex:
    """``seg_faces.get(segment keys)`` of the erase: the faces, in list
    order AT CONSTRUCTION, whose outer loop runs along that segment (once
    per run), then the faces :meth:`add` filed since. Built over every face
    without a scan; with one, only the segments asked for are looked up —
    candidates near the erased edges' ends at construction, keyed by THAT
    revision's positions (the scan's), as the full index was."""

    def __init__(self, mesh, endpoints) -> None:
        from core import scan_index
        self._full: Optional[dict] = None
        self._extra: dict = {}
        from core.history_commands import _KEY_REACH
        ends = list(endpoints)
        sc = scan_index.scan(mesh) if len(ends) <= 256 else None
        self._sc = sc
        if sc is None:
            self._full = {}
            for f in mesh.faces:
                for ek in _loop_edges(f.vertices):
                    self._full.setdefault(ek, []).append(f)
            return
        # The edge found at an end may be a weld away from the clicked
        # point; one more key cell covers it.
        hit: set = set()
        for a, b in ends:
            hit.update(sc.faces_meeting(scan_index.box_of((a, b), 2 * _KEY_REACH)))
        self._cand = sorted(hit)
        self._loops = dict(zip(self._cand, sc.native.face_loops(self._cand)))
        self._faces = dict(zip(self._cand, sc.faces(self._cand)))
        self._ends = [(QVector3D(a), QVector3D(b)) for a, b in ends]
        self._memo: dict = {}

    def add(self, f) -> None:
        for ek in _loop_edges(f.vertices):
            self._extra.setdefault(ek, []).append(f)

    def get(self, ek, default=()):
        if self._full is not None:
            base = self._full.get(ek, [])
        else:
            base = self._memo.get(ek)
            if base is None:
                base = self._memo[ek] = self._lookup(ek)
        more = self._extra.get(ek, [])
        if not base and not more:
            return default
        return base + more

    def _lookup(self, ek) -> list:
        sc = self._sc
        if self._covers(list(ek)):
            cand, loops, faces = self._cand, self._loops, self._faces
        else:
            # A segment away from every erased end (the cascade only asks
            # for erased edges): every face of the scan.
            cand = list(range(sc.native.counts()[2]))
            loops = dict(zip(cand, sc.native.face_loops(cand)))
            faces = None
        out = []
        for i in cand:
            loop = loops[i]
            n = len(loop)
            ks = [sc.tkey(v) for v in loop]
            for j in range(n):
                if frozenset((ks[j], ks[(j + 1) % n])) == ek:
                    out.append(i)
        if faces is None:
            return sc.faces(out)
        return [faces[i] for i in out]

    def _covers(self, keys) -> bool:
        """Whether every face with a segment of these keys is among the
        candidates: each key lies within a cell of one erased edge's end
        (two of the same edge for a two-key segment)."""
        from core.history_commands import _KEY_REACH
        def near(k, p) -> bool:
            return all(abs(k[i] - c) <= _KEY_REACH
                       for i, c in enumerate((p.x(), p.y(), p.z())))
        for a, b in self._ends:
            if all(near(k, a) or near(k, b) for k in keys):
                return True
        return False


def _soft_faceless_edges(mesh) -> list:
    """The soft edges bounding no face, in list order (a superset from the
    scan; every edge without it): what the erase sweeps up."""
    from core import scan_index
    sc = scan_index.scan(mesh)
    if sc is None:
        return list(mesh.edges)
    return sc.edges(sc.native.edges_faceless(True))


def _loop_has_segment(loop, v0, v1) -> bool:
    """Whether the vertex loop walks the segment v0–v1 (either way)."""
    n = len(loop)
    return any(
        (loop[i] == v0 and loop[(i + 1) % n] == v1)
        or (loop[i] == v1 and loop[(i + 1) % n] == v0)
        for i in range(n))


def erase_selected_geometry(cmd, scene):
    m = scene.mesh
    # Loop->face lookups through ONE index: _find_face_by_loop scans the
    # whole mesh per call, and deleting thousands of box-selected faces
    # froze for ~26 s on a 28k-face mesh (piscina.igz report, round 2).
    by_loop = _LoopIndex(m, cmd._face_loops)
    for loop in cmd._face_loops:
        bucket = by_loop.get(frozenset(_key(p) for p in loop))
        f = bucket.pop(0) if bucket else None
        if f is not None:
            m.remove_face(f)
            scene.selection.discard(f)
    # One segment->faces index for the cascade below: the per-edge scan
    # over every face was O(edges x faces) — deleting a curved tiled
    # wall (hundreds of curve segments) took 30+ s on an exploded
    # 28k-face mesh (piscina.igz report). Kept live as faces merge/die.
    seg_faces = _SegmentIndex(m, cmd._edge_endpoints)

    def _index_face(f) -> None:
        seg_faces.add(f)
    dead_faces: set = set()
    slit_planes: list = []
    across: list = []
    for a, b in cmd._edge_endpoints:
        v0 = m.vertex_at(a)
        v1 = m.vertex_at(b)
        edge = m.find_edge(v0, v1) if (v0 is not None and v1 is not None) else None
        if edge is None:
            continue
        faces = list(edge.faces)
        merged = None
        if len(faces) == 2 and faces[0] == faces[1]:
            # Slit edge: ONE face walks it twice — a fence line joining an
            # inner boundary to the outline turns the ring into a cut-open
            # walk. Dissolving "the face with itself" deleted the face and
            # kept the line (ss.igz report). Erasing the line must keep
            # the face: drop only the edge and re-derive the plane's
            # regions below (the ring comes back as outer + hole; other
            # fence pieces survive as free edges).
            f = faces[0]
            slit_planes.append((QVector3D(f.vertices[0]), f.normal()))
            m.remove_edge(edge)
            scene.selection.discard(edge)
            continue
        if len(faces) == 2 and QVector3D.dotProduct(
            faces[0].normal(), faces[1].normal()
        ) > 0.999:
            merged = m.dissolve_coplanar_region(faces)  # reunite the split
            if merged is not None:
                dead_faces.update(f.key for f in faces)
                _index_face(merged)
        elif len(faces) > 2:
            # A wall raised against a taller neighbour leaves its inner
            # side faces on the line too, so the front's split line had
            # four faces and the cascade took the whole front (#356).
            # Reunite the one coplanar pair as above; the faces across
            # the line are re-derived on their plane below, so what
            # still closes (the jamb above the sill) stays and the
            # inner partitions, open now, go.
            pair = _single_coplanar_pair(faces)
            if pair is not None:
                merged = m.dissolve_coplanar_region(pair)
            if merged is not None:
                dead_faces.update(f.key for f in pair)
                _index_face(merged)
                across.extend(f for f in faces if f not in pair)
                m.remove_edge(edge)
        if merged is None:
            # Cascade: the edge and every face bounding it (a face can't
            # outlive a bounding edge) go away.
            ekey = frozenset((_key(edge.v0.position), _key(edge.v1.position)))
            dying = [f for f in seg_faces.get(ekey, ())
                     if f.key not in dead_faces]
            # A hole is not a boundary the face dies with — the face
            # CAN outlive a hole edge, as the same face with the
            # opening filled in. Erasing one edge of a hole (an offset
            # ring's, #1191) used to leave the face claiming a hole
            # whose boundary no longer exists: the segment gone, the
            # hole still covering — and still saved. Drop the hole
            # from every surviving face that wears this segment only
            # as a hole edge; the hole's other edges stay as lines.
            for f in faces:
                if f.key in dead_faces or f in dying:
                    continue
                for hole in list(f.hole_loops):
                    if _loop_has_segment(hole, edge.v0, edge.v1) \
                            and not _loop_has_segment(
                                f.loop, edge.v0, edge.v1):
                        m.remove_hole(f, hole)
            for f in dying:
                dead_faces.add(f.key)
                m.remove_face(f)
                scene.selection.discard(f)
            m.remove_edge(edge)
        scene.selection.discard(edge)
    return slit_planes, across


def weld_landed(mesh, moving: set) -> bool:
    """Merge a moved vertex into the one it landed on.

    Lowering a box's top edge onto its bottom edge turns the box into a
    ramp — the endpoint snap puts each moved corner exactly on another
    vertex. Left as two vertices at one point, the side face became a
    zero-area sliver and the vertical edges zero-length; nothing showed it
    until Make Group copied the geometry, welded on the way, and refused the
    edge whose ends were now one vertex (#341, @pacaeiro). Only a move that
    actually lands pays for the weld: the check is one pass over the
    vertices, the weld runs when two of them share a point.

    The weld drops a wall left with under three corners. A wall flattened
    along a curve keeps more (#1294): push/pull merges an arc's nearly flat
    neighbouring walls, and the end one into the side wall it meets
    tangentially. Its loop then goes out along the curve and back: a spur,
    or the whole loop for a wall that was all curve. Spurs reaching a
    landed vertex are cut off (:func:`_without_spurs`); a wall left with
    under three corners goes, the rest is rebuilt on what remains."""
    from core.history_commands import _cells_of, _vertices_at
    moved_keys = _cells_of(mesh, moving)
    if not moved_keys:
        return False
    if not any(v.key not in moving for v in _vertices_at(mesh, moved_keys)):
        return False
    mesh.weld_coincident()
    for f in list(mesh.faces):
        if f.hole_loops or not any(_key(v.position) in moved_keys
                                   for v in f.loop):
            continue
        kept = _without_spurs(f.loop, moved_keys)
        if len(kept) == len(f.loop):
            continue
        attrs = dict(f.attrs)
        mesh.remove_face(f)
        if len(kept) >= 3:
            g = mesh.add_face([QVector3D(v.position) for v in kept])
            g.attrs = attrs
    return True


def _without_spurs(loop, landed: set) -> list:
    """``loop`` with every spur cut off: a run out to a landed vertex and
    straight back (``x, tip, x``), as often as one is left. A loop that was
    only spurs ends with under three corners: it bounded no area."""
    out = list(loop)
    changed = True
    while changed and len(out) >= 3:
        changed = False
        n = len(out)
        for i in range(n):
            prev, tip, nxt = out[i - 1], out[i], out[(i + 1) % n]
            if tip == prev:                  # a corner listed twice
                del out[i]
                changed = True
                break
            if prev == nxt and _key(tip.position) in landed:
                for j in sorted((i, (i + 1) % n), reverse=True):
                    del out[j]
                changed = True
                break
    return out


def carried_faces(mesh, moving: set) -> list:
    """The textured faces a move or turn carries WHOLE — every vertex of
    every loop among ``moving`` (vertex ids). Their texture map rides along
    (:func:`core.group._remap_uvws`); a face the move only stretches keeps
    the map it had."""
    out = []
    fast = getattr(mesh, "textured_faces", None)
    for f in (fast() if fast is not None else mesh.faces):
        if not (f.attrs and f.attrs.get("texture")):
            continue
        loops = [f.loop] + list(getattr(f, "hole_loops", []) or [])
        if all(v.key in moving for lp in loops for v in lp):
            out.append(f)
    return out

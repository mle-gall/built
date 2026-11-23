# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Model editing commands and their Python reference inverses."""
from __future__ import annotations

from abc import ABC, abstractmethod
from collections import Counter
from typing import Iterable, Optional

from core.mesh import element_identity
from core.element_set import ElementSet
from ingetrazo_core import Vec3 as QVector3D

from core.group import Group
from core import doc_model as _doc
from core import history_command_reference as _reference
from core.materials import has_own_material
from core.mesh import (PAINT_KEYS, Edge, Face, Mesh, Vertex, edge_flags,
                       edge_is_plain, stamp_edge_flags)
from core.topology import (
    _key,
    _loop_edges,
    carve_loop_by_chords,
    defects_near,
    find_containing_face,
    fold_nonplanar_faces,
    heal_overlapping_faces,
    hole_merged_with_loop,
    loop_inside_face,
    new_defects,
    orient_coplanar_faces,
    orphaned_edges_at,
    subtract_loop_from_face,
    winding_footprint,
)


def _find_face_by_loop(mesh, loop_positions) -> Optional[Face]:
    """The mesh face whose outer loop matches ``loop_positions`` (by key), or
    ``None``. Used to resolve a command's target face against the live mesh."""
    target = frozenset(_key(p) for p in loop_positions)
    for f in _faces_keyed_like(mesh, loop_positions):
        if frozenset(_key(p) for p in f.vertices) == target:
            return f
    return None


#: A point whose ``_key`` is one of a loop's lies within one key cell
#: (1e-4) of that loop's corner; the scans look this far and a bit more.
_KEY_REACH = 2e-4


def _faces_keyed_like(mesh, loop_positions, sc=None) -> list:
    """The faces, in list order, that can have the same corner keys as
    ``loop_positions``: every corner of such a face lies within a key cell
    of the loop's, so its box lies within the loop's grown by that. From
    the mesh scan when there is one (``core.scan_index``), else every face
    — the caller's key test decides either way. ``sc``: a scan taken
    earlier (the faces of THAT revision)."""
    pts = list(loop_positions)
    from core import scan_index
    if sc is None:
        sc = scan_index.scan(mesh) if pts else None
    if sc is None or not pts:
        return mesh.faces
    return sc.faces(sc.faces_within(scan_index.box_of(pts, _KEY_REACH)))


class CommandRejected(Exception):
    """A command declining to happen, on purpose: it has put the scene back
    the way it found it and says why in plain words. :class:`History` takes
    it as a no-op — nothing on the undo stack, nothing in the error log —
    and keeps the reason in ``last_rejected`` for the tool to show."""


class Command(ABC):
    """Abstract reversible operation against a :class:`Scene`."""

    #: Whether doing or undoing it leaves every material where it was —
    #: which ones the model wears.
    keeps_paint = False
    #: Whether doing or undoing it only puts groups in the model or takes
    #: them out, whole, and leaves every group that stays as it was — its
    #: Browser row, the paint of its faces.
    keeps_groups = False
    #: Whether it only changes the geometry of the mesh it edits: no group
    #: comes or goes, and nothing a Browser row shows (name, tag, material,
    #: copies, IFC class, hidden, locked) changes.
    #: A push/pull on the ride no longer reads its 85 000 rows again.
    keeps_rows = False
    #: Undone and redone by its document transaction (core/doc_model.py,
    #: docs/rust-document-model.md §4) instead of by ``undo`` / ``do``:
    #: everything it changes lives in the document. A method
    #: ``doc_undo(scene) -> bool`` for commands that qualify for some
    #: inputs only (asked right after ``do``). ``undo`` stays: it is the
    #: Python document's path and the fallback for a nested step.
    doc_undo = False
    _journal_geometry = False

    def _reference_snapshot(self, mesh):
        if not self._journal_geometry:
            return mesh.capture_state()

    def bind_transaction(self, txn):
        self._doc_txn = txn

    def doc_after(self, scene, phase) -> None:
        """What an undo or redo by transaction still does beside the
        document (``phase`` "undo" / "redo"): bump the version. Commands
        that also drop things from the selection or dirty caches extend it."""
        scene.version += 1

    @abstractmethod
    def do(self, scene) -> None:
        """Apply the operation."""

    @abstractmethod
    def undo(self, scene) -> None:
        """Reverse the operation."""


class MeshCommand(Command):
    """Commands whose persistent changes are covered by these geometry stores
    and the document field journal. Object meshes retain the command inverse."""
    def geometry_meshes(self, scene):
        return (scene.mesh,)


class FaceAttributeCommand(MeshCommand):
    def geometry_meshes(self, scene):
        return {getattr(face, "_m", None) for face in self._faces}


class SetPluginDataCommand(Command):
    """Replace one extension's document data (``scene.plugin_data[key]``),
    undoably. ``value`` None removes the key. Values are copied through JSON
    both ways, so neither the caller nor the history can alias them.

    With the Rust document ``scene.plugin_data`` is a document value: the
    step's transaction undoes it."""

    doc_undo = True

    def __init__(self, key: str, value) -> None:
        import json
        self.key = str(key)
        self.value = None if value is None else json.loads(json.dumps(value))
        self._had = False
        self._before = None

    def do(self, scene) -> None:
        import json
        data = scene.plugin_data
        self._had = self.key in data
        self._before = (json.loads(json.dumps(data[self.key]))
                        if self._had else None)
        if self.value is None:
            data.pop(self.key, None)
        else:
            data[self.key] = json.loads(json.dumps(self.value))
        scene.version += 1

    def undo(self, scene) -> None:
        import json
        if self._had:
            scene.plugin_data[self.key] = json.loads(json.dumps(self._before))
        else:
            scene.plugin_data.pop(self.key, None)
        scene.version += 1


def _geometry_plan(cmd, scene, targets):
    """Enlist all known stores for rollback. Use transaction replay only
    when it also covers the other commands in the step."""
    if isinstance(cmd, CompoundCommand):
        coverage = [_geometry_plan(c, scene, targets) for c in cmd.commands]
        return all(coverage) and (type(cmd) is CompoundCommand or cmd.journal_parts)
    if isinstance(cmd, GroupInContextCommand):
        return _geometry_plan(cmd.inner, scene, targets)
    if isinstance(cmd, MakeComponentOfCommand):
        return _geometry_plan(cmd._nest, scene, targets)
    if isinstance(cmd, MakeNestedGroupCommand) and cmd._inner is not None:
        return _geometry_plan(cmd._inner, scene, targets)
    if isinstance(cmd, MeshCommand):
        from core.store_mesh import StoreMesh
        meshes = cmd.geometry_meshes(scene)
        stores = [m for m in meshes if type(m) is StoreMesh]
        if stores:
            targets.append((cmd, stores))
        return len(stores) == len(meshes)
    return _uses_doc_undo(cmd, scene)


def _uses_doc_undo(cmd, scene) -> bool:
    if getattr(cmd, "_journal_geometry", False):
        return True
    flag = getattr(cmd, "doc_undo", False)
    if callable(flag):
        try:
            return bool(flag(scene))
        except Exception:  # noqa: BLE001 — undecided: its own undo
            return False
    return bool(flag)


# ---- Concrete commands ------------------------------------------------------

class AddEdgeCommand(MeshCommand):
    """Add a single edge. The mesh always welds and dedups, so a coincident
    edge is reused rather than duplicated; ``_owned`` records whether this
    command actually created the edge, so undo only removes an edge it owns
    (never the pre-existing one it merged into)."""

    keeps_rows = True

    def __init__(self, a: QVector3D, b: QVector3D, merge: bool = True,
                 soft: bool | None = None, curve: int | None = None,
                 layer: str | None = None) -> None:
        self.a = QVector3D(a)
        self.b = QVector3D(b)
        self.merge = merge  # kept for API parity; the mesh never duplicates
        # Optional flags stamped on the resulting edge — used when a split
        # replaces a curve/soft edge with sub-edges, so the pieces inherit.
        self.soft = soft
        self.curve = curve
        # A pasted edge keeps its tag (None: the edge's own, untouched).
        self.layer = layer
        self.edge: Optional[Edge] = None
        self._owned = False

    _stamp = _reference.stamp_edge

    def do(self, scene) -> None:
        m = scene.mesh
        if self._journal_geometry and type(self.soft) in (bool, type(None)) and (
                self.curve is None or type(self.curve) is int) and (
                self.layer is None or isinstance(self.layer, str)):
            from core.layers import DEFAULT_LAYER
            from core.store_mesh import EdgeView, StoreMesh
            if type(m) is StoreMesh:
                key = m._store.draw_command_edge(m._input_point(self.a), m._input_point(self.b),
                                                 self.soft, self.curve, self.layer, DEFAULT_LAYER)
                self._owned = key is not None
                self.edge = EdgeView(m, key) if self._owned else None
                if self._owned:
                    m._dirty(1)
                elif any(v is not None for v in (self.soft, self.curve, self.layer)):
                    m.mark_modified()
                scene.version += 1
                return
        _reference.draw_edge(self, scene)

    def doc_after(self, scene, phase):
        if phase == "undo" and self._owned:
            scene.selection.discard(self.edge)
        scene.version += 1

    undo = _reference.undo_draw_edge


class RemoveGeometryCommand(MeshCommand):
    """Deleting an element changes ordered incidence lists. The reference
    mesh restores their original order with its identity-preserving snapshot."""
    keeps_rows = True
    _before = None

    undo = _reference.restore_mesh_before

    def doc_after(self, scene, phase):
        _discard_removed_selection(scene)
        scene.version += 1


class DeleteEdgesCommand(RemoveGeometryCommand):
    """Erase edges (resolved by endpoint position). A face can't outlive a
    bounding edge — erasing an edge takes its faces with it — so every
    face that used a deleted edge on its *outer* boundary is removed too (its
    other edges stay, now free). Hole edges are left alone."""

    def __init__(self, edges: Iterable, cascade_faces: bool = True) -> None:
        self._endpoints = [(QVector3D(e.a), QVector3D(e.b)) for e in edges]
        self.cascade_faces = cascade_faces
        self.removed_edges: list[Edge] = []
        self.removed_faces: list[Face] = []

    def do(self, scene) -> None:
        m = scene.mesh
        from core.store_mesh import StoreMesh, EdgeView, FaceView
        if self._journal_geometry and type(m) is StoreMesh and not m.__dict__.get("_planned_points"):
            candidates = _faces_by_segments(m, self._endpoints) if self.cascade_faces else ()
            removed = m._store.delete_edge_segments(
                [(a.toTuple(), b.toTuple()) for a, b in self._endpoints],
                [f.key for f in candidates])
            if removed is not None:
                edges, faces = removed
                self.removed_edges = [EdgeView(m, key) for key in edges]
                self.removed_faces = [FaceView(m, key) for key in faces]
                m._dirty(len(edges) + len(faces))
                for face in self.removed_faces:
                    scene.selection.discard(face)
                for edge in self.removed_edges:
                    scene.selection.discard(edge)
                scene.version += 1
                return
        _reference.delete_edges(self, scene)


def _faces_by_segments(mesh, segments) -> list:
    """The faces, in list order, whose outer loop can run along one of
    ``segments`` (``(a, b)`` points) by key: such a face has a corner
    within a key cell of each end, so its box meets the segment's grown by
    that. Every face when there is no scan (or too many segments for
    single queries to beat one pass)."""
    from core import scan_index
    segs = list(segments)
    sc = scan_index.scan(mesh) if 0 < len(segs) <= 256 else None
    if sc is None:
        return mesh.faces
    hit: set = set()
    for a, b in segs:
        hit.update(sc.faces_meeting(scan_index.box_of((a, b), _KEY_REACH)))
    return sc.faces(sorted(hit))


class EraseSelectionCommand(RemoveGeometryCommand):

    """Erase selected edges and faces, the usual way.

    An edge that divides two *coplanar* faces is dissolved and the faces merge
    back into one (rubbing out a face's split line reunites it). Any other erased
    edge takes the faces it bounds with it (a non-planar pair can't become one
    face) — except through a *hole*: an edge that only bounds a face as one of
    its hole's edges takes the hole with it and the face fills in. Undo restores
    an identity-preserving snapshot — the merge restructures
    connectivity too much for a clean per-edge inverse."""

    keeps_rows = False

    def __init__(self, edges: Iterable, faces: Iterable = ()) -> None:
        self._edge_endpoints = [(QVector3D(e.a), QVector3D(e.b)) for e in edges]
        self._face_loops = [[QVector3D(v) for v in f.vertices] for f in faces]
        self._before = None

    def do(self, scene) -> None:
        m = scene.mesh
        self._before = self._reference_snapshot(m)
        footprint = winding_footprint(m)
        from core.store_mesh import EdgeView, FaceView, StoreMesh, _hypot
        result = None
        if _doc.ENABLED and type(m) is StoreMesh and (hypot := _hypot()) is not None:
            result = m._store.erase_selection(
                [[p.toTuple() for p in loop] for loop in self._face_loops],
                [(m._input_point(a), m._input_point(b)) for a,b in self._edge_endpoints], hypot)
        if result is None:
            slit_planes, across = _reference.erase_selected_geometry(self, scene)
        else:
            dirty, edges, faces, merges, slits, across_keys = result
            if dirty:
                m._dirty(dirty)
            from core.mesh import _dominant_attrs
            for target, sources, stacked in merges:
                merged = FaceView(m, target)
                if not stacked or not merged.attrs:
                    candidates = sources[1:] if stacked else sources
                    merged.attrs = dict(_dominant_attrs([FaceView(m, k) for k in candidates]))
            for key in faces:
                scene.selection.discard(FaceView(m, key))
            for key in edges:
                scene.selection.discard(EdgeView(m, key))
            slit_planes = [(QVector3D(*o), QVector3D(*n)) for o,n in slits]
            across = [FaceView(m, key) for key in across_keys]
        # Slit removals re-derive their plane from the arrangement while the
        # cut-ring face is still present to provide coverage (it re-emits as
        # outer + hole, attrs intact; uncovered regions stay empty).
        done_planes: list = []
        for origin, normal in slit_planes:
            nn = normal.normalized()
            if any(abs(QVector3D.dotProduct(nn, n)) > 0.999 and
                   abs(QVector3D.dotProduct(origin - o, n)) < 1e-4
                   for o, n in done_planes):
                continue
            done_planes.append((origin, nn))
            RebuildPlaneFacesCommand(origin, nn).do(scene)
        # The faces across a reunited line, each plane from its own faces'
        # remaining edges: the line's other faces may span past it (the
        # jamb above the sill) and only what still closes comes back.
        while across:
            f = across[0]
            o, n = QVector3D(f.vertices[0]), f.normal()
            same = [g for g in across if abs(QVector3D.dotProduct(
                n, g.normal())) > 0.999 and abs(QVector3D.dotProduct(
                QVector3D(g.vertices[0]) - o, n)) < 1e-4]
            across = [g for g in across if g not in same]
            RebuildPlaneFacesCommand(o, n, faces=same).do(scene)
        # Deleting a curved surface takes its hidden seams with it: a soft edge
        # left bordering no face is a dangling curve segment (a cylinder side's
        # vertical seam) — prune it so no stray vertical lines remain. Hard
        # orphan edges stay (an erased flat face keeps its outline).
        if result is not None and type(m) is StoreMesh:
            n = m._store.erase_soft_seams()
            if n:
                m._dirty(n)
        else:
            for e in _reference._soft_faceless_edges(m):
                if e.soft and not e.faces:
                    m.remove_edge(e)
        # A merge can leave the big enclosing face overlapping its subdivisions;
        # drop any such redundant mother (covered by the snapshot undo above).
        for f in heal_overlapping_faces(m, footprint=footprint):
            scene.selection.discard(f)
        # And the endpoints go with them. remove_edge only detaches, so an
        # erase used to leave its vertices in the mesh: invisible, ignored by
        # bounds(), and written into the .igz. Nine of them 16 km out were
        # all Marco's document had left after deleting a stray line, which is
        # why zoom-to-extents could not bring him back (2026-09-18).
        m.prune_orphan_vertices()
        scene.version += 1


class TagCurveCommand(MeshCommand):
    """Mark the edges of a drawn circle/arc loop as one curve (shared id), so
    selecting a segment selects the whole curve. Runs as the last step of the
    draw so the id is captured in the enclosing snapshot (redo keeps it); undo is
    a no-op because that snapshot restores the pre-draw state."""

    def __init__(self, loop_points, closed: bool = True) -> None:
        self.pts = [QVector3D(p) for p in loop_points]
        self.closed = closed

    def do(self, scene) -> None:
        from core import mesh as mesh_api
        from core.store_mesh import StoreMesh
        mesh = scene.mesh
        if self._journal_geometry and type(mesh) is StoreMesh:
            mesh._chunk_dirty = True
            mesh._mut_serial += 1
            if mesh._store.tag_curve_path([p.toTuple() for p in self.pts], self.closed,
                                          mesh_api._CURVE_COUNTER):
                mesh.next_curve_id()
        else:
            mesh.tag_curve(self.pts, self.closed)

    def undo(self, scene) -> None:
        pass


def _inherit_paint(face, mother) -> None:
    """Give ``face`` the paint of the ``mother`` it was drawn on, for the keys
    it does not already carry itself.

    A face drawn on a painted one is part of that surface: a door outlined on
    a textured wall comes out textured, and lines up with the wall
    because both keep the same world→UV map. Only the remainder of a carved
    mother inherited here, so the cut-out itself came back bare (Marco,
    2026-08-27). Verbatim, map included — the two are coplanar, so the image
    has to run straight across the outline.

    Whatever the face already declares wins: a rectangle drawn with a material
    active, or one the caller stamped (the chord split hands its halves the
    mother's attrs directly), keeps what it was given.
    """
    src = getattr(mother, "attrs", None)
    if not src:
        return
    for k in PAINT_KEYS:
        if k in src and k not in face.attrs:
            v = src[k]
            face.attrs[k] = dict(v) if isinstance(v, dict) else v


#: How far off a face's plane its containment tests let a point sit
#: (``topology._PLANAR_TOLERANCE``, 1e-3), with room to spare.
_PLANE_REACH = 2e-3


def _faces_holding(mesh, loop) -> list:
    """The faces, in list order, that can hold ``loop`` the way the
    containment tests ask (``loop_inside_face``, ``find_subdividing_chain``,
    ``carve_loop_by_chords``): every corner of the loop on the face's plane
    within the planar tolerance and projected inside it, or on its
    boundary. Such a face's box, grown by that tolerance and by how far its
    own corners stray from its plane, holds the loop's box. A list taken
    now, as ``list(m.faces)`` was: the caller may edit the mesh as it goes.
    Every face when there is no scan."""
    from core import scan_index
    pts = list(loop)
    sc = scan_index.scan(mesh) if pts else None
    if sc is None:
        return list(mesh.faces)
    # The loop's own box, not grown: the FACE's grows (a bigger loop box
    # would hold fewer faces).
    lo, hi = scan_index.raw_box(pts)
    pad = _PLANE_REACH + 1e-6 * max(abs(c) for c in lo + hi)
    return sc.faces(sc.native.faces_holding(lo, hi, pad))


def _faces_inside(mesh, face) -> list:
    """The faces, in list order, that can lie strictly inside ``face``
    (``loop_inside_face(face, other.vertices)``): every corner of theirs
    within the planar tolerance of its plane and projected inside its
    outer loop, so within its box grown by that tolerance and by how far
    its corners stray from its plane. A list taken now; every face without
    a scan (or for a face whose plane Python may read differently)."""
    from core import scan_index
    pts = face.vertices
    sc = scan_index.scan(mesh) if len(pts) >= 3 else None
    reach = _plane_reach(face) if sc is not None else None
    if reach is None:
        return list(mesh.faces)
    return sc.faces(sc.faces_within(scan_index.box_of(pts, reach)))


def _plane_reach(face) -> Optional[float]:
    """How far from ``face``'s box a point projected inside it can lie:
    the planar tolerance plus the farthest corner off the face's plane,
    with slack for float32. ``None`` when the Newell vector is too short
    to be sure which normal Python picks."""
    pts = face.vertices
    nx = ny = nz = 0.0
    k = len(pts)
    for i in range(k):
        p, q = pts[i], pts[(i + 1) % k]
        nx += (p.y() - q.y()) * (p.z() + q.z())
        ny += (p.z() - q.z()) * (p.x() + q.x())
        nz += (p.x() - q.x()) * (p.y() + q.y())
    length = (nx * nx + ny * ny + nz * nz) ** 0.5
    if length < 1e-8:
        return None
    nx, ny, nz = nx / length, ny / length, nz / length
    o = pts[0]
    off = span = 0.0
    for p in pts:
        dx, dy, dz = p.x() - o.x(), p.y() - o.y(), p.z() - o.z()
        off = max(off, abs(dx * nx + dy * ny + dz * nz))
        span = max(span, (dx * dx + dy * dy + dz * dz) ** 0.5)
    return _PLANE_REACH + off + 1e-5 * span + 1e-7


class AddFaceCommand(MeshCommand):
    """Add a face, dividing any coplanar face it lands strictly inside.

    When the new loop falls wholly within an existing face, that mother face
    gains a hole so it no longer overlaps the new face. The hole is recorded so
    undo removes exactly the loop this command punched.
    """

    keeps_rows = True

    def __init__(
        self,
        vertices: Iterable[QVector3D],
        auto: bool = True,
        holes: Optional[Iterable[Iterable[QVector3D]]] = None,
        attrs: Optional[dict] = None,
        *,
        interior: bool = False,
    ) -> None:
        self.vertices = [QVector3D(v) for v in vertices]
        self.preset_holes = (
            [[QVector3D(v) for v in loop] for loop in holes] if holes else None
        )
        self.auto = auto
        self.interior = interior
        # Attrs stamped onto the face at creation (paste carries the copied
        # colour/texture); they stay on the object across undo/redo.
        self.attrs = dict(attrs) if attrs else None
        self.face: Optional[Face] = None
        self._hole_swaps = False
        self._before = self._after = None

    def do(self, scene) -> None:
        mesh = scene.mesh
        if self._after is not None:
            mesh.restore_state(self._after)
        else:
            self._before = self._reference_snapshot(mesh)
            self.face = mesh.add_face(self.vertices, self.preset_holes)
            self.face.interior = self.interior
            if self.attrs:
                self.face.attrs.update(self.attrs)
            if self.auto:
                self._divide(mesh)
            self._after = self._reference_snapshot(mesh)
        scene.version += 1

    def _divide(self, mesh):
        from core.store_mesh import FaceView, StoreMesh, _hypot
        if (_doc.ENABLED and type(mesh) is StoreMesh and not mesh.__dict__.get("_planned_points")
                and (hypot := _hypot()) is not None):
            result = mesh._store.divide_drawn_face(
                self.face.key, [f.key for f in _faces_holding(mesh, self.face.vertices)],
                [f.key for f in _faces_inside(mesh, self.face)], hypot)
            if result is not None:
                dirty, source, remainder, self._hole_swaps = result
                if dirty:
                    mesh._dirty(dirty)
                if source is not None:
                    mother = FaceView(mesh, source)
                    for key in remainder:
                        FaceView(mesh, key).attrs = dict(mother.attrs)
                    _inherit_paint(self.face, mother)
                return
        self._hole_swaps = _reference.divide_drawn_face(mesh, self.face)

    def doc_after(self, scene, phase):
        _discard_removed_selection(scene)
        scene.version += 1

    def undo(self, scene) -> None:
        if self._before is not None:
            selected = [e for e in scene.selection if isinstance(e, Edge) and e in scene.mesh.edges]
            scene.mesh.restore_state(self._before)
            for edge in selected:
                if edge not in scene.mesh.edges:
                    scene.selection.discard(edge)
            _discard_removed_selection(scene)
            scene.version += 1


class DeleteFaceCommand(RemoveGeometryCommand):
    """Remove a face (resolved by its outer loop). Holes travel with it, so undo
    restores them via relink."""

    def __init__(self, face) -> None:
        self._loop = [QVector3D(v) for v in face.vertices]
        self.face: Optional[Face] = None

    def do(self, scene) -> None:
        from core.store_mesh import StoreMesh, FaceView
        mesh = scene.mesh
        if self._journal_geometry and type(mesh) is StoreMesh:
            handled, key = mesh._store.delete_face_loop(
                [p.toTuple() for p in self._loop],
                [f.key for f in _faces_keyed_like(mesh, self._loop)])
            if handled:
                self.face = FaceView(mesh, key) if key is not None else None
                if self.face is not None:
                    mesh._dirty()
                    scene.selection.discard(self.face)
                scene.version += 1
                return
        _reference.delete_face(self, scene)


def _dirty_group_chunks(scene) -> None:
    """Attr edits bypass the mesh mutation primitives — flag every group
    mesh so the viewport's cached chunks re-validate their materials.

    Two flags, because they answer different questions. ``_chunk_dirty`` is
    "something touched this mesh"; ``_attrs_dirty`` is "a MATERIAL changed",
    and only the second can be told apart from a move. The viewport's
    translation fast path reuses a cached chunk's texture buckets after
    checking geometry alone, so without this a repaint that happened in the
    same breath as a drag stayed invisible outside the group — the flagstone
    corner of Plaza Yanque kept its neighbour's grey (Marco, 2026-09-10).

    Nested placements are flagged too: their meshes render as part of the
    parent, and walking scene.groups as a flat list never reached them.
    """
    from core.group import iter_placements
    # The mesh being edited changed too (its faces' paint or tags), so its
    # revision moves; not mark_modified, which also bumps the mutation
    # serial and the winding count the viewport keys its geometry caches
    # on, and a paint would have re-walked every coordinate.
    scene.mesh.bump_revision()
    for g in getattr(scene, "groups", []):
        for pg, _m in iter_placements(g):
            pg.mesh._chunk_dirty = True
            pg.mesh._attrs_dirty = True


def _dirty_placements(groups) -> None:
    """:func:`_dirty_group_chunks` for the placements ``groups`` and their
    subtrees only: what History does after a step from the placements its
    transaction changed (the tag, Hide, paint: what a chunk bakes), instead
    of every command flagging every mesh — Hide on 200 placements of a
    20,000-placement file walked all of them (≈11 ms)."""
    from core.purge import iter_groups
    for g in iter_groups(groups):
        mesh = getattr(g, "mesh", None)
        if mesh is not None:
            mesh.invalidate_placement_chunks()


#: The placement fields a viewport chunk depends on (see _dirty_placements).
_RENDER_FIELDS = ["layer", "hidden", "material"]


def _all_placements(items) -> bool:
    """Every item a placement whose fields live in the document: its
    renderer dirtiness then comes from the step's transaction."""
    return _doc.ENABLED and all(isinstance(e, Group) for e in items)


def _bulk_set(items, field, value):
    """Write ``value`` into ``field`` of every placement of ``items`` as one
    document op (``doc_set_many``: Lock, Hide or Tag of a whole selection),
    returning the previous values; ``None`` when not every item is a
    document placement (the caller writes them one by one)."""
    if not _doc.ENABLED or not items:
        return None
    try:
        prev = _doc.NATIVE.doc_get_many(items, field)
        _doc.NATIVE.doc_set_many(items, field, value)
    except (TypeError, RuntimeError):
        return None
    return prev


def _unselect(scene, items) -> None:
    """``scene.selection.discard(e)`` for every item: from the selection's
    side when it is the smaller (Lock or Hide of 20,000 placements with a
    handful selected need not look each one up)."""
    sel = scene.selection
    if not sel:
        return
    if len(sel) * 4 < len(items):
        from core.mesh import element_identity
        drop = {element_identity(e) for e in items}
        gone = [e for e in sel if element_identity(e) in drop]
    else:
        gone = items
    for e in gone:
        sel.discard(e)


def _reverse_faces(faces):
    from core.store_mesh import FaceView, StoreMesh
    batches = {}
    object_keys = set()
    for face in faces:
        if isinstance(face, FaceView) and isinstance(face._m, StoreMesh):
            batches.setdefault(face._m, []).append(face.key)
        else:
            face.reverse_rings()
            object_keys.add(face.key)
    for mesh, keys in batches.items():
        mesh._store.reverse_faces(keys)
    return batches, object_keys


class FlipFacesCommand(FaceAttributeCommand):
    """Reverse the winding of a set of faces — "Reverse Faces".

    Reversing the loops flips the geometric normal while keeping the *same*
    ``Face`` objects and their shared edges/incidence (identity preserved, so
    selection and snapshots stay valid). The operation is an involution:
    undo simply flips again. Faces may live in the loose mesh or inside a
    group's mesh — the flip is per-object, so both work."""

    keeps_rows = True

    def __init__(self, faces) -> None:
        self._faces = [f for f in faces if isinstance(f, Face) or hasattr(f, "loop")]

    def meshes(self):
        from core.store_mesh import FaceView
        return {f._m for f in self._faces if isinstance(f, FaceView)}

    def doc_after(self, scene, phase):
        for mesh in self.meshes():
            mesh.note_rewound()
        scene.version += 1

    def _flip(self, scene) -> None:
        batches, ids = _reverse_faces(self._faces)
        # A reversed loop changes no count, coordinate or paint — all a
        # cached group chunk is checked against — so the group went on
        # drawing the old winding until some edit changed a count (#197).
        # Each mesh holding a flipped face says so; the others keep theirs.
        from core.group import iter_placements
        owners = set(batches)
        if ids:
            meshes = [getattr(scene, "mesh", None), getattr(scene, "loose_mesh", None)]
            meshes.extend(pg.mesh for g in getattr(scene, "groups", [])
                          for pg, _m in iter_placements(g))
            owners.update(m for m in set(meshes) if m is not None
                          and any(f.key in ids for f in m.faces))
        for mesh in owners:
            mesh._winding_rev = getattr(mesh, "_winding_rev", 0) + 1
        _dirty_group_chunks(scene)
        scene.version += 1

    def do(self, scene) -> None:
        self._flip(scene)

    def undo(self, scene) -> None:
        self._flip(scene)


class SetLayerVisibilityCommand(Command):
    """Change own visibility flags without modifying geometry or child flags."""
    keeps_paint = True
    keeps_rows = True

    def __init__(self, names, on, folders=()):
        self.names, self.folders, self.on = list(names), list(folders), bool(on)

    #: Tag flags, folder visibility and the active tag are all in the
    #: document (slice 2).
    doc_undo = True

    def doc_after(self, scene, phase) -> None:
        if phase == "redo":
            for entity in list(scene.selection):
                if not scene.entity_visible(entity):
                    scene.selection.discard(entity)
        scene.version += 1

    def do(self, scene):
        from core import layers as L
        self.before = L.layer_state(scene)
        for key in self.folders:
            scene.folder_visibility[key] = self.on
        L.set_visible(scene, self.names, self.on)
        for entity in list(scene.selection):
            if not scene.entity_visible(entity):
                scene.selection.discard(entity)
        scene.version += 1

    def undo(self, scene):
        from core import layers as L
        L.apply_layer_state(scene, self.before)
        scene.version += 1


class EnsureLayersCommand(Command):
    """Give a document the tags (and tag folders) that pasted geometry
    carries and it lacks — a step of the paste, undone with it. Tags the
    document already has are left exactly as they are
    (``core.layers.add_missing``)."""

    #: Tags, tag folders and their tables are in the document (slice 2).
    doc_undo = True

    def __init__(self, template) -> None:
        self._template = template
        self._added: tuple[list, list] = ([], [])

    def doc_after(self, scene, phase) -> None:
        if any(self._added):
            scene.version += 1

    def do(self, scene) -> None:
        from core.layers import add_missing
        layers, folders = add_missing(scene, self._template)
        self._added = ([scene.layer_by_key(n).uid for n in layers],
                       [scene.layer_folder_uids[n] for n in folders])
        if any(self._added):
            scene.version += 1

    def undo(self, scene) -> None:
        from core.layers import remove_added
        if any(self._added):
            remove_added(scene, *self._added)
            scene.version += 1


def _carrier_meshes(entities, field):
    # A Python-only carrier contributes None, making the plan use its inverse.
    return {getattr(e, "_m", None) for e in entities if not isinstance(e, Group)
            and field not in getattr(e, "_record_fields", ())}


class AssignLayerCommand(MeshCommand):
    """Move entities onto a layer — the Tag field in Entity Info.

    Faces, edges, groups and annotations alike (``core.layers.assign_layer``
    knows where each one keeps its label). Previous labels are captured at
    ``do`` time, so a mixed selection undoes exactly. A layer that does not
    exist yet is created (Rafael's «no sé cómo cambiar el objeto de capa»,
    2026-09-16 — the panel's button was the only road and it was not
    understood; this is what Entity Info and the right-click go through)."""

    def __init__(self, entities, layer: str) -> None:
        self._entities = list(entities)
        self._layer = layer
        self._prev: list = []
        self._created = False

    def geometry_meshes(self, scene):
        return _carrier_meshes(self._entities, "layer")

    def doc_undo(self, scene):
        return not self.geometry_meshes(scene)

    def doc_after(self, scene, phase) -> None:
        # The meshes to re-validate come from the transaction (History).
        scene.version += 1

    def do(self, scene) -> None:
        from core.layers import DEFAULT_LAYER, Layer, assign_layer, layer_of
        ly = scene.layer(self._layer)
        if ly is None:
            ly = Layer(self._layer)
            scene.layers.append(ly)
            self._created = True
        # The tag's key is what the entities carry; ``layer`` may be a label.
        self._layer = ly.key
        prev = _bulk_set(self._entities, "layer",
                         None if self._layer == DEFAULT_LAYER else self._layer)
        if prev is not None:
            self._prev = [p or DEFAULT_LAYER for p in prev]
        else:
            self._prev = [layer_of(e) for e in self._entities]
            for e in self._entities:
                assign_layer(e, self._layer)
        if not _all_placements(self._entities):
            _dirty_group_chunks(scene)
        scene.version += 1

    def undo(self, scene) -> None:
        from core.layers import assign_layer
        for e, prev in zip(self._entities, self._prev):
            assign_layer(e, prev)
        if self._created:
            ly = scene.layer_by_key(self._layer)
            if ly is not None:
                scene.layers.remove(ly)
        _dirty_group_chunks(scene)
        scene.version += 1


class DeleteLayerCommand(Command):
    """Delete a layer: what it carried goes to the default layer, and the
    scenes forget it. Undo puts the layer back at its place in the list,
    with its visibility and lock, and every entity back on it.

    The only Layers-panel action on the undo stack: creating and renaming
    are easy to redo by hand and would only get between Ctrl+Z and the
    drawing, while a deleted layer takes its assignments with it.

    ``into`` merges instead: what the layer carried goes onto that layer
    (Layers ▸ Merge into…), and the active layer, if it was this one,
    becomes that one."""

    def __init__(self, name: str, into: str | None = None) -> None:
        self.name = name
        self.into = into
        self._layer = None
        self._index = 0
        self._carriers: list = []
        self._views: list = []

    def doc_undo(self, scene) -> bool:
        # The tag list, the active tag and placements' tags are in the
        # document; faces carry theirs in their meshes, saved views in
        # Python objects.
        return not _carrier_meshes(self._carriers, "layer")

    def doc_after(self, scene, phase) -> None:
        scene.version += 1

    def do(self, scene) -> None:
        from core.layers import DEFAULT_LAYER, assign_layer, layer_of
        from core.purge import layer_carriers
        ly = scene.layer(self.name)
        if ly is None or ly.key == DEFAULT_LAYER:
            raise CommandRejected("layer cannot be deleted")
        # Entities carry the tag's KEY; ``name`` may be its label.
        self.name = ly.key
        into = scene.layer(self.into or DEFAULT_LAYER)
        if into is None or into is ly:
            raise CommandRejected("no layer to merge into")
        target = into.key
        self._layer = ly
        self._index = scene.layers.index(ly)
        self._carriers = [e for e in layer_carriers(scene)
                          if layer_of(e) == self.name]
        self._views = []
        for view in getattr(scene, "saved_views", None) or ():
            hidden = getattr(view, "hidden_layers", None)
            if hidden and self.name in hidden:
                self._views.append((view, list(hidden)))
                view.hidden_layers = [n for n in hidden if n != self.name]
        for e in self._carriers:
            assign_layer(e, target)
        self._was_active = getattr(scene, "active_layer", None) == self.name
        if self._was_active:
            scene.active_layer = None if target == DEFAULT_LAYER else target
        scene.layers.remove(ly)
        if not _all_placements(self._carriers):
            _dirty_group_chunks(scene)
        scene.version += 1

    def undo(self, scene) -> None:
        from core.layers import assign_layer
        # A layer made and named the same after the delete (renaming is not
        # on the stack) takes the entities back instead of a twin.
        if self._layer not in scene.layers and scene.layer_by_key(self.name) is None:
            scene.layers.insert(min(self._index, len(scene.layers)),
                                self._layer)
        for e in self._carriers:
            assign_layer(e, self.name)
        for view, hidden in self._views:
            view.hidden_layers = list(hidden)
        if getattr(self, "_was_active", False):
            scene.active_layer = self.name
        _dirty_group_chunks(scene)
        scene.version += 1


def _is_hidden(entity) -> bool:
    attrs = getattr(entity, "attrs", None)
    if attrs is not None:                          # Face
        return bool(attrs.get("hidden"))
    return bool(getattr(entity, "hidden", False))


def _set_hidden(entity, hidden: bool) -> None:
    attrs = getattr(entity, "attrs", None)
    if attrs is not None:                          # Face
        if hidden:
            attrs["hidden"] = True
        else:
            attrs.pop("hidden", None)
    else:
        entity.hidden = hidden


class HideCommand(MeshCommand):
    """Hide (or unhide) objects, faces and edges — Edit ▸ Hide
    and Edit ▸ Unhide.

    An OBJECT (group or component, ``Group.hidden``) and a FACE
    (``attrs["hidden"]``) disappear from every consumer that asks
    ``Scene.entity_visible``: render, pick, snap, bounds, export; an
    object's nested placements go with it, and a group's chunk leaves its
    hidden faces out. A hidden EDGE stays in the topology (its faces keep
    their boundary) but draws neither as a line nor as a profile /
    silhouette. Previous per-entity flags are captured at ``do`` time, so
    a mixed selection (some already hidden) undoes exactly."""

    def __init__(self, entities, hidden: bool = True) -> None:
        self._entities = [e for e in entities
                          if hasattr(e, "hidden") or hasattr(e, "attrs")]
        self._hidden = hidden
        self._prev: list[bool] = []

    @property
    def entities(self) -> list:
        return list(self._entities)

    @property
    def hides(self) -> bool:
        return self._hidden

    def geometry_meshes(self, scene):
        return _carrier_meshes(self._entities, "hidden")

    def doc_undo(self, scene):
        return not self.geometry_meshes(scene)

    def doc_after(self, scene, phase) -> None:
        if phase == "redo" and self._hidden:
            _unselect(scene, self._entities)
        # The meshes to re-validate come from the transaction (History).
        scene.version += 1

    def do(self, scene) -> None:
        prev = (_bulk_set(self._entities, "hidden", self._hidden)
                if _all_placements(self._entities) else None)
        if prev is not None:
            self._prev = prev
        else:
            self._prev = [_is_hidden(e) for e in self._entities]
            for e in self._entities:
                _set_hidden(e, self._hidden)
        if self._hidden:
            # An invisible entity must not linger in the selection: every
            # selection consumer (move, delete, paint) assumes it can see
            # what it operates on.
            _unselect(scene, self._entities)
        if prev is None:
            _dirty_group_chunks(scene)
        scene.version += 1

    def undo(self, scene) -> None:
        for e, prev in zip(self._entities, self._prev):
            _set_hidden(e, prev)
        _dirty_group_chunks(scene)
        scene.version += 1


class HideEdgesCommand(HideCommand):
    """The edges-only spelling, kept for callers that grew up with it."""

    def __init__(self, edges, hidden: bool = True) -> None:
        super().__init__(edges, hidden=hidden)


class _FacePaintCommand(FaceAttributeCommand):
    """Apply owned paint fields inside the enlisted geometry transaction."""
    def __init__(self, faces, value):
        self._faces = list(faces)
        self._value = value
        self._old = None

    def do(self, scene):
        self._paint([(self.field, self._value)])
        _dirty_group_chunks(scene)
        scene.version += 1

    def undo(self, scene):
        self._restore_paint([self.field])
        _dirty_group_chunks(scene)
        scene.version += 1

    def _paint(self, fields):
        from ingetrazo_core import apply_face_paint
        capture = self._old is None and not self._journal_geometry
        saved = apply_face_paint(self._faces, fields, capture)
        if capture:
            self._old = saved

    def _restore_paint(self, fields):
        from ingetrazo_core import restore_face_paint
        restore_face_paint(self._faces, fields, self._old or [])


class SetFaceColorCommand(_FacePaintCommand):
    """Set or clear the front colour of each selected face."""

    field = "color"

    def __init__(self, faces, color) -> None:
        super().__init__(faces, list(color) if color is not None else None)


class SetFaceOpacityCommand(_FacePaintCommand):
    """Set or clear the opacity of coloured or textured faces."""

    field = "opacity"

    def __init__(self, faces, opacity) -> None:
        super().__init__(faces, float(opacity) if opacity is not None else None)


class SetFaceMaterialTagCommand(_FacePaintCommand):
    """Set (or clear with ``None``) the registry identity ``attrs["mat"]``
    on a set of faces — the companion of the colour/texture stamp, so a
    paint stroke carries the material's NAME (core.materials) and the
    per-material takeoff stays truthful. Painting anonymously clears the
    tag: a red face is no longer "Concreto visto".

    When ``material`` is given, ``do`` also ensures it exists in the scene
    registry (a library swatch registers itself on first use); ``undo``
    removes it again only if this very command added it."""

    def __init__(self, faces, name, material=None) -> None:
        self._faces = list(faces)
        self._name = name
        self._material = material
        self._old: Optional[list] = None
        self._registered = False

    def do(self, scene) -> None:
        if (self._material is not None and self._name
                and self._name not in getattr(scene, "materials", {})):
            scene.materials[self._name] = self._material
            self._registered = True
        self._paint([("pbr", self._material),
                     ("mat", self._name)])
        scene.version += 1

    def undo(self, scene) -> None:
        self._restore_paint(["pbr", "mat"])
        if self._registered:
            scene.materials.pop(self._name, None)
            self._registered = False
        scene.version += 1


class SetMaterialFinishCommand(Command):
    """Choose how a named material answers light in a render (core.finish;
    ``None`` = guess it from the name again). Only the registry entry
    changes — the viewport draws the same — so undo is exact and cheap."""

    doc_undo = True

    def __init__(self, name, finish) -> None:
        self._name = name
        self._finish = finish
        self._old = None

    def do(self, scene) -> None:
        mat = scene.materials.get(self._name)
        if mat is None:
            return
        self._old = mat.finish
        mat.finish = self._finish
        scene.version += 1

    def undo(self, scene) -> None:
        mat = scene.materials.get(self._name)
        if mat is None:
            return
        mat.finish = self._old
        scene.version += 1


class RestampMaterialCommand(MeshCommand):
    """Edit a registry material and RESTAMP every face that wears it.

    "Change the concrete everywhere": the registry entry is replaced by
    ``new_material`` (same name) and each face across the loose mesh and
    every group whose ``attrs["mat"]`` carries that name receives the new
    recipe — colour/texture/opacity — in one undoable step. Keys absent
    from the new recipe are removed (a material edited from textured to
    plain colour drops its texture). Groups and components painted WHOLE
    wear it too, on the container (issue #155, @fafecm: the loose face
    changed colour, the painted box beside it stayed the old one). Undo
    restores the registry entry, each face's previous values and each
    container's previous paint exactly.

    A ``new_material`` with ANOTHER name renames as it restamps: the old
    entry leaves the registry, the new one takes its place and every face
    and container now says the new name. Only the model changes — a
    library colour renamed here stays in the library as it was."""

    _kind = "stamp"

    def __init__(self, name, new_material) -> None:
        self._name = name
        self._new = new_material
        self._edit = None
        self._meshes = ()

    def geometry_meshes(self, scene):
        from core.purge import iter_meshes
        return list(iter_meshes(scene))

    def doc_after(self, scene, phase):
        for mesh in self._meshes:
            mesh._mut_serial += 1
        _dirty_group_chunks(scene)
        scene.version += 1

    def do(self, scene) -> None:
        from ingetrazo_core import MaterialRestamp
        from core.purge import iter_groups
        if self._edit is None:
            self._edit = MaterialRestamp(
                scene.loose_mesh, iter_groups(scene.groups), scene.materials,
                self._name, self._new.name, self._kind, not self._journal_geometry)
            self._meshes = self._edit.meshes
        self._edit.apply(scene.materials, self._new, getattr(self, "edited", None))
        self.doc_after(scene, "do")
        if self._journal_geometry:
            self._edit = None

    def undo(self, scene) -> None:
        if self._edit is not None:
            self._edit.restore(scene.materials)
            self.doc_after(scene, "undo")


class EditTexturedMaterialCommand(RestampMaterialCommand):
    """Edit a TEXTURED registry material — its colour, its tile size —
    and carry the edit to every face, group and component that wears it.

    A YouTube review of 0.6.1 (#1308): «when you edit an already applied
    material, say change the colour, it does not affect the applied
    materials in the model, almost as if it creates another material
    definition». It did: the Materials panel's Apply changed the texture
    to paint with next and the faces SELECTED, so the model kept the old
    look, and with one face selected the same name came out in two
    colours. Unlike a restamp, the edit is applied to each face's own
    texture: a texture placed by hand (``uvw``), continued round a curved
    surface or turned on another wall keeps its place; only the colour —
    and the tile size and turn, when ``size`` is given — change.

    ``tint`` is an RGB (0-1) or ``None`` to remove the colour; ``size`` is
    ``(sw, sh, rot)`` or ``None`` to leave each face's own. Undo is the
    restamp's: the registry entry, each face's and each container's paint
    exactly as they were."""

    _kind = "texture"

    def __init__(self, name, tint, tint_mode: int = 0, size=None) -> None:
        super().__init__(name, None)
        self._tint = tuple(tint) if tint is not None else None
        self._tint_mode = int(tint_mode or 0)
        self._size = tuple(size) if size is not None else None

    def edited(self, tex):
        """*tex* with this edit: the colour (from its untinted image) and,
        when given, the size and turn."""
        if not tex:
            return tex
        from core.texture import tinted_texture, untinted_texture
        out = dict(tex)
        if self._size is not None:
            out["sw"], out["sh"], out["rot"] = (float(v) for v in self._size)
        if self._tint is None:
            return untinted_texture(out)
        return tinted_texture(out, self._tint, self._tint_mode)

    def do(self, scene) -> None:
        from core.materials import Material
        old = scene.materials.get(self._name)
        if old is None or not old.texture:
            return
        if self._edit is None:
            self._new = Material(self._name, color=old.color, texture=self.edited(old.texture),
                                 opacity=old.opacity, finish=old.finish, pbr=old.pbr)
        super().do(scene)


class PurgeUnusedCommand(Command):
    """"Purge Unused", for layers and/or materials.

    Layers are labels, not owners (core.layers), so emptying one cannot
    delete it — an explicit sweep is the only honest way to clear what an
    import left behind. What goes is decided ONCE, on the first ``do``:
    redo must remove exactly what the original run removed, not re-survey a
    model the user has since drawn on.

    Layers come back at their original indices so the panel order survives
    a round trip, and materials return to the registry with their recipe
    intact — the faces never changed, since a purged material is by
    definition one no face wears.
    """

    #: Tags and the materials registry are in the document (slice 2).
    doc_undo = True

    def __init__(self, layers: bool = True, materials: bool = True) -> None:
        self._do_layers = bool(layers)
        self._do_materials = bool(materials)
        #: (index, Layer) and (name, Material), captured on the first do.
        self._layers: Optional[list] = None
        self._materials: Optional[list] = None

    def _survey(self, scene) -> None:
        from core.purge import unused_layers, unused_materials
        current = list(getattr(scene, "layers", None) or ())
        self._layers = ([(current.index(ly), ly)
                         for ly in unused_layers(scene)]
                        if self._do_layers else [])
        self._materials = ([(n, scene.materials[n])
                            for n in unused_materials(scene)]
                           if self._do_materials else [])

    def counts(self, scene) -> tuple[int, int]:
        """What a run would remove — for the confirmation the panel shows."""
        if self._layers is None:
            self._survey(scene)
        return len(self._layers or []), len(self._materials or [])

    def do(self, scene) -> None:
        if self._layers is None:
            self._survey(scene)
        for _index, ly in self._layers or []:
            if ly in scene.layers:
                scene.layers.remove(ly)
        for name, _material in self._materials or []:
            scene.materials.pop(name, None)
        scene.version += 1

    def undo(self, scene) -> None:
        for index, ly in sorted(self._layers or [], key=lambda p: p[0]):
            if ly not in scene.layers:
                scene.layers.insert(min(index, len(scene.layers)), ly)
        for name, material in self._materials or []:
            scene.materials[name] = material
        scene.version += 1


class AddDimensionCommand(Command):
    """Add a dimension annotation to ``scene.dimensions``, anchored to the
    vertices under its endpoints (if any) so it follows the geometry."""

    keeps_rows = True

    doc_undo = True

    def __init__(self, dimension) -> None:
        self.dimension = dimension

    def do(self, scene) -> None:
        if hasattr(self.dimension, "bind"):
            self.dimension.bind(scene)
        scene.dimensions.append(self.dimension)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.dimension in scene.dimensions:
            scene.dimensions.remove(self.dimension)
        scene.version += 1


class DeleteDimensionsCommand(Command):
    """Remove a set of dimensions from ``scene.dimensions``."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "redo":
            scene.selection.difference_update(self._dims)
        scene.version += 1

    def __init__(self, dimensions) -> None:
        self._dims = list(dimensions)
        self._restore: list[tuple[int, object]] = []

    def do(self, scene) -> None:
        self._restore = [(scene.dimensions.index(d), d)
                         for d in self._dims if d in scene.dimensions]
        for d in self._dims:
            if d in scene.dimensions:
                scene.dimensions.remove(d)
            scene.selection.discard(d)
        scene.version += 1

    def undo(self, scene) -> None:
        for i, d in sorted(self._restore):
            scene.dimensions.insert(i, d)
        scene.version += 1


class AddTextLabelCommand(Command):
    """Add a leader-text annotation to ``scene.text_labels``."""

    keeps_rows = True

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "undo":
            scene.selection.discard(self.label)
        scene.version += 1

    def __init__(self, label) -> None:
        self.label = label

    def do(self, scene) -> None:
        scene.text_labels.append(self.label)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.label in scene.text_labels:
            scene.text_labels.remove(self.label)
        scene.selection.discard(self.label)
        scene.version += 1


class MoveTextLabelsCommand(Command):
    """Translate the floating label of leader texts: the label end moves,
    the anchor stays pinned and the leader stretches (the classic Move-on-
    text behaviour)."""

    keeps_paint = True

    doc_undo = True

    def __init__(self, labels, delta) -> None:
        self._labels = list(labels)
        self._delta = QVector3D(delta)

    def do(self, scene) -> None:
        for t in self._labels:
            t.offset = t.offset + self._delta
        scene.version += 1

    def undo(self, scene) -> None:
        for t in self._labels:
            t.offset = t.offset - self._delta
        scene.version += 1


class MoveGuidesCommand(Command):
    """Translate construction guides (Tape Measure) with the Move tool
    (#627). A guide line slides parallel to itself; a guide point carries
    its ``origin`` along, so the dashed segment it draws keeps its shape."""

    keeps_paint = True
    keeps_rows = True

    doc_undo = True

    def __init__(self, guides, delta) -> None:
        self._guides = list(guides)
        self._delta = QVector3D(delta)

    @staticmethod
    def shift(guide, step: QVector3D) -> None:
        """Apply one step — the tool's live preview uses it too."""
        guide.point = guide.point + step
        if guide.origin is not None:
            guide.origin = guide.origin + step

    def do(self, scene) -> None:
        for g in self._guides:
            self.shift(g, self._delta)
        scene.version += 1

    def undo(self, scene) -> None:
        for g in self._guides:
            self.shift(g, -self._delta)
        scene.version += 1


class RotateGuidesCommand(Command):
    """Rotate construction guides about ``centre``/``axis`` (the Rotate
    tool, #627): the point orbits, a line's direction turns, a guide
    point's ``origin`` orbits with it. Undo puts back the exact values from
    before, so a turn and its undo never drift."""

    keeps_paint = True
    keeps_rows = True

    doc_undo = True

    def __init__(self, guides, centre: QVector3D, axis: QVector3D,
                 deg: float) -> None:
        self._guides = list(guides)
        self.centre = QVector3D(centre)
        self.axis = QVector3D(axis)
        self.deg = float(deg)
        self._before: list = []

    @staticmethod
    def state(guide) -> tuple:
        """What a rotation changes, copied — the tool's preview keeps it."""
        return (guide, QVector3D(guide.point),
                None if guide.direction is None else QVector3D(guide.direction),
                None if guide.origin is None else QVector3D(guide.origin))

    @staticmethod
    def place(state, m) -> None:
        """Put a guide at ``m`` applied to its ``state`` (``None``: as it was)."""
        guide, point, direction, origin = state
        guide.point = m.map(point) if m is not None else QVector3D(point)
        if direction is not None:
            d = m.mapVector(direction) if m is not None else direction
            guide.direction = (d.normalized() if d.length() > 1e-12
                               else QVector3D(direction))
        if origin is not None:
            guide.origin = m.map(origin) if m is not None else QVector3D(origin)

    def do(self, scene) -> None:
        self._before = [self.state(g) for g in self._guides]
        m = rotation_matrix(self.centre, self.axis, self.deg)
        for st in self._before:
            self.place(st, m)
        scene.version += 1

    def undo(self, scene) -> None:
        for st in self._before:
            self.place(st, None)
        scene.version += 1


class MoveDimensionsCommand(Command):
    """Move dimensions with the Move tool. A dimension ANCHORED at both
    ends slides its LINE by the delta (minus any component along the
    measured segment, so the line stays parallel and the extension lines
    stay square) while the endpoints keep holding their vertices —
    the classic Move-on-dimension, «mover la cota conservando la línea guía»
    (Marco, 2026-09-20). A dimension with a free endpoint moves that
    endpoint rigidly instead. ``modes`` maps each dimension to ``"line"``
    or ``"rigid"``, decided by the tool at grab time."""

    keeps_paint = True

    doc_undo = True

    def __init__(self, modes: dict, delta) -> None:
        self._modes = dict(modes)
        self._delta = QVector3D(delta)

    @staticmethod
    def shift(dim, mode: str, step: QVector3D) -> None:
        """Apply one step — the tool's live preview uses it too."""
        if mode == "line":
            ab = dim.b - dim.a
            length = ab.length()
            if length > 1e-9:
                d = ab / length
                step = step - d * QVector3D.dotProduct(step, d)
            dim.offset = dim.offset + step
        else:
            if dim.anchor_a is None and dim.attachment_a is None:
                dim.a = dim.a + step
            if dim.anchor_b is None and dim.attachment_b is None:
                dim.b = dim.b + step

    def do(self, scene) -> None:
        for dim, mode in self._modes.items():
            self.shift(dim, mode, self._delta)
        scene.version += 1

    def undo(self, scene) -> None:
        for dim, mode in self._modes.items():
            self.shift(dim, mode, -self._delta)
        scene.version += 1


class EditTextLabelCommand(Command):
    """Change the text of a leader-text annotation."""

    doc_undo = True

    def __init__(self, label, text: str) -> None:
        self.label = label
        self._new = text
        self._old = label.text

    def do(self, scene) -> None:
        self.label.text = self._new
        scene.version += 1

    def undo(self, scene) -> None:
        self.label.text = self._old
        scene.version += 1


class EditDimensionTextCommand(Command):
    """Change a dimension's custom text (``None`` = back to the measured
    value) — a double-click on the dimension text."""

    doc_undo = True

    def __init__(self, dim, text) -> None:
        self.dim = dim
        self._new = text or None
        self._old = getattr(dim, "text", None)

    def do(self, scene) -> None:
        self.dim.text = self._new
        scene.version += 1

    def undo(self, scene) -> None:
        self.dim.text = self._old
        scene.version += 1


class DeleteTextLabelsCommand(Command):
    """Remove a set of text labels from ``scene.text_labels``."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "redo":
            scene.selection.difference_update(self._labels)
        scene.version += 1

    def __init__(self, labels) -> None:
        self._labels = list(labels)
        self._restore: list[tuple[int, object]] = []

    def do(self, scene) -> None:
        self._restore = [(scene.text_labels.index(t), t)
                         for t in self._labels if t in scene.text_labels]
        for t in self._labels:
            if t in scene.text_labels:
                scene.text_labels.remove(t)
            scene.selection.discard(t)
        scene.version += 1

    def undo(self, scene) -> None:
        for i, t in sorted(self._restore):
            scene.text_labels.insert(i, t)
        scene.version += 1


class AddGuideCommand(Command):
    """Add a construction guide (Tape Measure) to ``scene.guides``."""

    keeps_paint = True
    keeps_rows = True

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "undo":
            scene.selection.discard(self.guide)
        scene.version += 1

    def __init__(self, guide) -> None:
        self.guide = guide

    def do(self, scene) -> None:
        scene.guides.append(self.guide)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.guide in scene.guides:
            scene.guides.remove(self.guide)
        scene.version += 1


class PlaceSectionPlaneCommand(Command):
    """Place a section plane. The new plane immediately becomes
    the ACTIVE cut of the context; undo restores the previous one."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "redo":
            scene.selection.clear()
            scene.selection.add(self.plane)
        else:
            scene.selection.discard(self.plane)
        scene.bump_sections()

    def __init__(self, plane) -> None:
        self.plane = plane
        self._prev_active = None

    def do(self, scene) -> None:
        self._prev_active = scene.active_section()
        scene.section_planes.append(self.plane)
        scene.set_active_section(self.plane)
        scene.selection.clear()
        scene.selection.add(self.plane)
        scene.bump_sections()

    def undo(self, scene) -> None:
        if self.plane in scene.section_planes:
            scene.section_planes.remove(self.plane)
        scene.selection.discard(self.plane)
        prev = self._prev_active
        scene.set_active_section(
            prev if prev in scene.section_planes else None)
        scene.bump_sections()


class DeleteSectionPlanesCommand(Command):
    """Remove section planes; undo restores them at their positions with
    their active state."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "redo":
            scene.selection.difference_update(self._planes)
        scene.bump_sections()

    def __init__(self, planes) -> None:
        self._planes = list(planes)
        self._restore: list[tuple[int, object]] = []

    def do(self, scene) -> None:
        self._restore = [(scene.section_planes.index(p), p)
                         for p in self._planes if p in scene.section_planes]
        for p in self._planes:
            if p in scene.section_planes:
                scene.section_planes.remove(p)
            scene.selection.discard(p)
        scene.bump_sections()

    def undo(self, scene) -> None:
        for i, p in sorted(self._restore):
            scene.section_planes.insert(i, p)
        scene.bump_sections()


class ReverseSectionPlaneCommand(Command):
    """Reverse: flip which side the plane hides."""

    doc_undo = True

    def doc_after(self, scene, phase):
        scene.bump_sections()

    def __init__(self, plane) -> None:
        self.plane = plane

    def do(self, scene) -> None:
        self.plane.flip()
        scene.bump_sections()

    def undo(self, scene) -> None:
        self.plane.flip()
        scene.bump_sections()


class SetActiveSectionCommand(Command):
    """Make ``plane`` the active cut (or ``None`` to deactivate) — the
    Active Cut toggle / double-click."""

    doc_undo = True

    def doc_after(self, scene, phase):
        scene.bump_sections()

    def __init__(self, plane) -> None:
        self.plane = plane
        self._prev = None

    def do(self, scene) -> None:
        self._prev = scene.active_section()
        scene.set_active_section(self.plane)
        scene.bump_sections()

    def undo(self, scene) -> None:
        prev = self._prev
        scene.set_active_section(
            prev if prev in scene.section_planes else None)
        scene.bump_sections()


class SetSectionModeCommand(Command):
    """One section plane's mode, from the Browser: whether it CUTS
    (it becomes the active cut — or none is) and whether its FRAME shows.
    Cut and frame, cut only, frame only, or off — one undo step."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "redo" and self.plane.hidden:
            scene.selection.discard(self.plane)
        scene.bump_sections()

    def __init__(self, plane, cut: bool, frame: bool) -> None:
        self.plane = plane
        self.cut = bool(cut)
        self.frame = bool(frame)
        self._prev = None

    def do(self, scene) -> None:
        self._prev = (scene.active_section(),
                      bool(getattr(self.plane, "hidden", False)))
        if self.cut:
            scene.set_active_section(self.plane)
        elif self.plane.active:
            scene.set_active_section(None)
        self.plane.hidden = not self.frame
        if self.plane.hidden:
            scene.selection.discard(self.plane)
        scene.bump_sections()

    def undo(self, scene) -> None:
        active, hidden = self._prev
        scene.set_active_section(
            active if active in scene.section_planes else None)
        self.plane.hidden = hidden
        scene.bump_sections()


class SetSectionFillCommand(Command):
    """One section plane's own fill — pattern, colour, hatch spacing —
    over the style's. ``fill=None`` goes back to the style's fill."""

    doc_undo = True

    def doc_after(self, scene, phase):
        scene.bump_sections()

    def __init__(self, plane, fill) -> None:
        self.plane = plane
        self.fill = dict(fill) if fill else None
        self._prev = None

    def do(self, scene) -> None:
        prev = getattr(self.plane, "fill", None)
        self._prev = dict(prev) if prev else None
        self.plane.fill = dict(self.fill) if self.fill else None
        scene.bump_sections()

    def undo(self, scene) -> None:
        self.plane.fill = dict(self._prev) if self._prev else None
        scene.bump_sections()


class RenameSectionPlaneCommand(Command):
    """A section plane's name and symbol (the symbol reads on its frame and
    in the sheets' section marks)."""

    doc_undo = True

    def doc_after(self, scene, phase):
        scene.bump_sections()

    def __init__(self, plane, name: str, symbol: str) -> None:
        self.plane = plane
        self.name = name
        self.symbol = symbol
        self._prev = None

    def do(self, scene) -> None:
        self._prev = (self.plane.name, self.plane.symbol)
        self.plane.name, self.plane.symbol = self.name, self.symbol
        scene.bump_sections()

    def undo(self, scene) -> None:
        self.plane.name, self.plane.symbol = self._prev
        scene.bump_sections()


class AddSheetCommand(Command):
    """A new sheet at the end of the set."""

    doc_undo = True

    def __init__(self, sheet) -> None:
        self.sheet = sheet

    def do(self, scene) -> None:
        scene.compositions.append(self.sheet)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.sheet in scene.compositions:
            scene.compositions.remove(self.sheet)
        scene.version += 1


class PlaceViewCommand(Command):
    """A view frame placed on a sheet — a scene dropped on it in the
    Browser. One undo step."""

    doc_undo = True

    def __init__(self, sheet, frame) -> None:
        self.sheet = sheet
        self.frame = frame

    def do(self, scene) -> None:
        self.sheet.frames.append(self.frame)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.frame in self.sheet.frames:
            self.sheet.frames.remove(self.frame)
        scene.version += 1


class MoveSheetCommand(Command):
    """A sheet to another place in the set: the order the strip shows,
    the atlas prints and {hoja} counts."""

    doc_undo = True

    def __init__(self, sheet, index: int) -> None:
        self.sheet = sheet
        self.index = index
        self._old = None

    def do(self, scene) -> None:
        comps = scene.compositions
        self._old = comps.index(self.sheet)
        comps.remove(self.sheet)
        comps.insert(max(0, min(self.index, len(comps))), self.sheet)
        scene.version += 1

    def undo(self, scene) -> None:
        comps = scene.compositions
        comps.remove(self.sheet)
        comps.insert(self._old, self.sheet)
        scene.version += 1


class CreatePlanCommand(Command):
    """A floor plan from a group, in one undo step: a horizontal section
    plane at the cut height (made the active cut), a scene looking down on
    it in parallel projection, and — optionally — a sheet with that scene in
    a view frame."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "undo":
            scene.selection.discard(self.plane)
        scene.version += 1

    def __init__(self, plane, view, sheet=None,
                 add_plane: bool = True) -> None:
        self.plane = plane
        self.view = view
        self.sheet = sheet
        #: False: the plane is already in the model (a view of an existing
        #: cut) — it is made the active cut, not added.
        self.add_plane = add_plane
        self._prev_active = None

    def do(self, scene) -> None:
        self._prev_active = scene.active_section()
        if self.add_plane:
            scene.section_planes.append(self.plane)
        scene.set_active_section(self.plane)
        scene.saved_views.append(self.view)
        if self.sheet is not None:
            scene.compositions.append(self.sheet)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.sheet is not None and self.sheet in scene.compositions:
            scene.compositions.remove(self.sheet)
        if self.view in scene.saved_views:
            scene.saved_views.remove(self.view)
        if self.add_plane and self.plane in scene.section_planes:
            scene.section_planes.remove(self.plane)
        prev = self._prev_active
        scene.set_active_section(
            prev if prev in scene.section_planes else None)
        scene.selection.discard(self.plane)
        scene.version += 1


class ExplodeCurvesCommand(MeshCommand):
    """Explode Curve: the curves the given edges belong to become plain
    segments, each one picked on its own. A circle or an arc selects whole
    because its edges share a curve id; this drops the id, the geometry
    stays exactly as it is. Undo gives every edge its id back."""

    def __init__(self, edges) -> None:
        self.edges = list(edges)
        self._ids: list = []
        self.curves = 0

    def do(self, scene) -> None:
        from core.store_mesh import StoreMesh
        if self._journal_geometry and type(scene.mesh) is StoreMesh:
            ids = {e.curve for e in self.edges if getattr(e, "curve", None) is not None}
            if not all(type(i) is int and 0 <= i < 2**63 for i in ids):
                return _reference.explode_curves(self, scene)
            self.curves = scene.mesh._store.explode_curves(ids)
            scene.mesh._chunk_dirty = True
            scene.mesh._mut_serial += 1
            scene.version += 1
            return
        _reference.explode_curves(self, scene)

    undo = _reference.undo_explode_curves


class AddSceneCommand(Command):
    """A saved scene into the list — a new one, or a copy (Duplicate) —
    at ``index`` (``None``: at the end), with the rest of its folder;
    undo takes it away again. Renamed in
    :class:`core.saved_views.RenameSceneCommand`."""

    doc_undo = True

    def __init__(self, view, index: Optional[int] = None) -> None:
        self.view = view
        self.index = index

    def do(self, scene) -> None:
        views = scene.saved_views
        at = len(views) if self.index is None else self.index
        views.insert(max(0, min(at, len(views))), self.view)
        # Filed in a folder, it goes with the rest of that folder.
        from core.scene_folders import sync_order
        sync_order(scene)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.view in scene.saved_views:
            scene.saved_views.remove(self.view)
        scene.version += 1


class DeleteScenesCommand(Command):
    """Delete saved scenes, several at once (#280); undo puts each back in
    its place in the list."""

    doc_undo = True

    def __init__(self, views) -> None:
        self.views = list(views)
        self._at: list = []

    def do(self, scene) -> None:
        self._at = [(scene.saved_views.index(v), v) for v in self.views
                    if v in scene.saved_views]
        for _i, v in self._at:
            scene.saved_views.remove(v)
        scene.version += 1

    def undo(self, scene) -> None:
        for i, v in sorted(self._at, key=lambda t: t[0]):
            scene.saved_views.insert(i, v)
        scene.version += 1


class UpdateSceneCommand(Command):
    """Update a scene from the live view, keeping its name; undo gives it
    back what it remembered before (#1158)."""

    doc_undo = True

    def __init__(self, view, camera, style=None) -> None:
        self.view = view
        self.camera = camera
        #: The style of the view it is taken from, when that view has its
        #: own (one of several views, #1306); else the scene's.
        self.style = style
        self._before: dict = {}
        self._after: Optional[dict] = None

    def do(self, scene) -> None:
        self._before = {k: getattr(self.view, k) for k in self.view._record_fields}
        if self._after is None:          # redo puts back the same capture
            self.view.recapture(scene, self.camera, self.style)
            self._after = {k: getattr(self.view, k) for k in self.view._record_fields}
        else:
            for k, value in self._after.items():
                setattr(self.view, k, value)
        scene.version += 1

    def undo(self, scene) -> None:
        for k, value in self._before.items():
            setattr(self.view, k, value)
        scene.version += 1


class SceneTreeCommand(Command):
    """A change to the Scenes tab's tree — a folder made, renamed or taken
    away, rows dragged into a folder or to another place (#1158). ``change``
    does it with the helpers of :mod:`core.scene_folders` and returns
    whether anything changed (False: rejected, nothing to undo). Undo puts
    back the order of the scenes, their folders and the folder tree; redo
    puts back what the change left, without running it again."""

    doc_undo = True

    def __init__(self, change) -> None:
        self.change = change
        self._before: Optional[tuple] = None
        self._after: Optional[tuple] = None

    def do(self, scene) -> None:
        from core import scene_folders as SF
        if self._after is not None:
            SF.restore(scene, self._after)
        else:
            self._before = SF.state(scene)
            if not self.change(scene):
                SF.restore(scene, self._before)
                raise CommandRejected("Nothing to change")
            self._after = SF.state(scene)
        scene.version += 1

    def undo(self, scene) -> None:
        from core import scene_folders as SF
        SF.restore(scene, self._before)
        scene.version += 1


class MoveSectionPlanesCommand(Command):
    """Translate section planes (the Move tool)."""

    keeps_paint = True

    doc_undo = True

    def doc_after(self, scene, phase):
        scene.bump_sections()

    def __init__(self, planes, delta: QVector3D) -> None:
        self._planes = list(planes)
        self.delta = QVector3D(delta)

    def do(self, scene) -> None:
        for p in self._planes:
            p.point = p.point + self.delta
        scene.bump_sections()

    def undo(self, scene) -> None:
        for p in self._planes:
            p.point = p.point - self.delta
        scene.bump_sections()


class RotateSectionPlanesCommand(Command):
    """Rotate section planes about ``centre``/``axis`` (the Rotate tool):
    the origin orbits and the normal turns."""

    doc_undo = True

    def doc_after(self, scene, phase):
        scene.bump_sections()

    def __init__(self, planes, centre: QVector3D, axis: QVector3D,
                 deg: float) -> None:
        self._planes = list(planes)
        self.centre = QVector3D(centre)
        self.axis = QVector3D(axis)
        self.deg = float(deg)

    def _apply(self, scene, deg: float) -> None:
        m = rotation_matrix(self.centre, self.axis, deg)
        for p in self._planes:
            p.point = m.map(p.point)
            n = m.mapVector(p.normal)
            if n.length() > 1e-12:
                p.normal = n.normalized()
        scene.bump_sections()

    def do(self, scene) -> None:
        self._apply(scene, self.deg)

    def undo(self, scene) -> None:
        self._apply(scene, -self.deg)


class ChangeGuideCommand(Command):
    """Re-aim an existing guide line (Protractor hot retype: after the guide
    is created, a typed angle updates it until the next click/command)."""

    keeps_paint = True
    keeps_rows = True

    doc_undo = True

    def __init__(self, guide, direction) -> None:
        self.guide = guide
        self.new = QVector3D(direction).normalized()
        self.old: Optional[QVector3D] = None

    def do(self, scene) -> None:
        self.old = self.guide.direction
        self.guide.direction = QVector3D(self.new)
        scene.version += 1

    def undo(self, scene) -> None:
        self.guide.direction = self.old
        scene.version += 1


class DeleteGuidesCommand(Command):
    """Remove construction guides (the Eraser, or Edit ▸ Delete Guides)."""

    keeps_paint = True
    keeps_rows = True

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "redo":
            scene.selection.difference_update(self._guides)
        scene.version += 1

    def __init__(self, guides) -> None:
        self._guides = list(guides)
        self._restore: list[tuple[int, object]] = []

    def do(self, scene) -> None:
        self._restore = [(scene.guides.index(g), g)
                         for g in self._guides if g in scene.guides]
        for g in self._guides:
            if g in scene.guides:
                scene.guides.remove(g)
            scene.selection.discard(g)
        scene.version += 1

    def undo(self, scene) -> None:
        for i, g in sorted(self._restore):
            scene.guides.insert(i, g)
        scene.version += 1


class AddImagePlaneCommand(Command):
    """Place an imported reference image (``File ▸ Import ▸ Image``).

    The image lands on its own layer so it can be switched off without taking
    the drawing with it (the reasoning behind ``layers.SURVEY_LAYER``); the
    command creates that layer when missing and takes it back on undo, so
    importing and undoing leaves the document exactly as it was.
    """

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "redo":
            scene.selection.clear()
            scene.selection.add(self.image)
        else:
            scene.selection.discard(self.image)
        scene.version += 1

    def __init__(self, image) -> None:
        self.image = image
        self._made_layer = False

    def do(self, scene) -> None:
        from core.layers import Layer
        name = getattr(self.image, "layer", None)
        self._made_layer = False
        if name and scene.layer(name) is None:
            scene.layers.append(Layer(name))
            self._made_layer = True
        scene.image_planes.append(self.image)
        scene.selection.clear()
        scene.selection.add(self.image)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.image in scene.image_planes:
            scene.image_planes.remove(self.image)
        scene.selection.discard(self.image)
        if self._made_layer:
            name = getattr(self.image, "layer", None)
            ly = scene.layer(name) if name else None
            if ly is not None:
                scene.layers.remove(ly)
            self._made_layer = False
        scene.version += 1


class DeleteImagePlanesCommand(Command):
    """Remove reference images (Delete / the Eraser). Positions are restored
    so undo puts each one back where it sat in the stacking order."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "redo":
            scene.selection.difference_update(self._images)
        scene.version += 1

    def __init__(self, images) -> None:
        self._images = list(images)
        self._restore: list[tuple[int, object]] = []

    def do(self, scene) -> None:
        self._restore = [(scene.image_planes.index(im), im)
                         for im in self._images if im in scene.image_planes]
        for im in self._images:
            if im in scene.image_planes:
                scene.image_planes.remove(im)
            scene.selection.discard(im)
        scene.version += 1

    def undo(self, scene) -> None:
        for i, im in sorted(self._restore, key=lambda t: t[0]):
            scene.image_planes.insert(i, im)
        scene.version += 1


class TransformImagePlaneCommand(Command):
    """Set an image's corner and edge vectors — the one undoable step behind
    scaling, moving and rotating it. Callers compute the new frame (e.g.
    ``ImagePlane.scaled``) and hand it over; the command only swaps it in, so
    every way of reshaping an image shares one reversible operation."""

    doc_undo = True

    def __init__(self, image, origin=None, u=None, v=None) -> None:
        from ingetrazo_core import Vec3 as QVector3D
        self.image = image
        self._new = (
            QVector3D(origin) if origin is not None else None,
            QVector3D(u) if u is not None else None,
            QVector3D(v) if v is not None else None,
        )
        self._old: tuple | None = None

    def do(self, scene) -> None:
        from ingetrazo_core import Vec3 as QVector3D
        im = self.image
        self._old = (QVector3D(im.origin), QVector3D(im.u), QVector3D(im.v))
        origin, u, v = self._new
        if origin is not None:
            im.origin = QVector3D(origin)
        if u is not None:
            im.u = QVector3D(u)
        if v is not None:
            im.v = QVector3D(v)
        scene.version += 1

    def undo(self, scene) -> None:
        from ingetrazo_core import Vec3 as QVector3D
        if self._old is None:
            return
        origin, u, v = self._old
        self.image.origin = QVector3D(origin)
        self.image.u = QVector3D(u)
        self.image.v = QVector3D(v)
        scene.version += 1


class SetImagePlaneOpacityCommand(Command):
    """Fade a reference image (a scan you trace over, an orthomosaic under
    the model) — one undoable step."""

    doc_undo = True

    def __init__(self, image, opacity: float) -> None:
        self.image = image
        self.opacity = max(0.0, min(1.0, float(opacity)))
        self._old = float(getattr(image, "opacity", 1.0))

    def do(self, scene) -> None:
        self.image.opacity = self.opacity
        scene.version += 1

    def undo(self, scene) -> None:
        self.image.opacity = self._old
        scene.version += 1


class MoveImagePlanesCommand(Command):
    """Translate reference images (the Move tool). Only the origin corner
    moves — the edge vectors carry size and orientation, so a move can never
    resize or turn the picture by accident."""

    keeps_paint = True

    doc_undo = True

    def __init__(self, images, delta: QVector3D) -> None:
        self._images = list(images)
        self.delta = QVector3D(delta)

    def _apply(self, scene, delta: QVector3D) -> None:
        for im in self._images:
            im.origin = im.origin + delta
        scene.version += 1

    def do(self, scene) -> None:
        self._apply(scene, self.delta)

    def undo(self, scene) -> None:
        self._apply(scene, -self.delta)


class RotateImagePlanesCommand(Command):
    """Rotate reference images about ``centre``/``axis`` (the Rotate tool):
    the origin orbits and both edge vectors turn with it, so the picture keeps
    its size and its proportions and simply faces a new way. Rotating about an
    axis off the image's own plane tilts it, which is how a facade photo gets
    stood up against a wall."""

    doc_undo = True

    def __init__(self, images, centre: QVector3D, axis: QVector3D,
                 deg: float) -> None:
        self._images = list(images)
        self.centre = QVector3D(centre)
        self.axis = QVector3D(axis)
        self.deg = float(deg)

    def _apply(self, scene, deg: float) -> None:
        m = rotation_matrix(self.centre, self.axis, deg)
        for im in self._images:
            im.origin = m.map(im.origin)
            im.u = m.mapVector(im.u)
            im.v = m.mapVector(im.v)
        scene.version += 1

    def do(self, scene) -> None:
        self._apply(scene, self.deg)

    def undo(self, scene) -> None:
        self._apply(scene, -self.deg)


class ScaleImagePlanesCommand(Command):
    """Uniformly scale reference images about ``anchor`` (the Scale tool).

    Both edge vectors take the same factor, so the picture is never distorted
    — which is the whole reason a scan is worth scaling: you set it against a
    distance you know and everything else on it becomes measurable. A negative
    factor mirrors the image through the anchor, as it does for geometry.
    """

    doc_undo = True

    def __init__(self, images, anchor: QVector3D, factor, axes=None) -> None:
        self._images = list(images)
        self.axes = axes
        self.anchor = QVector3D(anchor)
        # A float scales uniformly; a 3-tuple per axis (the grip box).
        self.factor = (tuple(float(f) for f in factor)
                       if isinstance(factor, (tuple, list)) else float(factor))

    def _apply(self, scene, factor: float) -> None:
        m = scale_matrix(self.anchor, factor, self.axes)
        for im in self._images:
            im.origin = m.map(im.origin)
            im.u = m.mapVector(im.u)
            im.v = m.mapVector(im.v)
        scene.version += 1

    def do(self, scene) -> None:
        self._apply(scene, self.factor)

    def undo(self, scene) -> None:
        self._apply(scene, invert_scale_factor(self.factor))


class AddGeoPathCommand(Command):
    """Add a traced georef path to ``scene.geo_paths`` (Track G)."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "undo":
            scene.selection.discard(self.path)
        scene.version += 1

    def __init__(self, path) -> None:
        self.path = path

    def do(self, scene) -> None:
        scene.geo_paths.append(self.path)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.path in scene.geo_paths:
            scene.geo_paths.remove(self.path)
        scene.selection.discard(self.path)
        scene.version += 1


class AutoTagDrawnFacesCommand(MeshCommand):
    """Tag-as-you-draw (BIM active class): the final step of a draw plan when
    ``scene.active_ifc`` is set. Stamps the active tag on the faces the plan
    itself created — the drawn polygon, auto-closed cycles, chord halves —
    identified through their ``AddFaceCommand``s, so churn re-emissions
    (``auto=False``) and neighbouring rebuilt faces are never touched. A face
    that inherited a tag from a tagged mother keeps it.

    Each draw commit becomes its OWN BIM object (a fresh id is allocated at
    execution time; the activation only fixes class + name): a wall drawn per
    trace = one object per wall, so the largest-face metrado stays honest —
    one shared id across many walls would under-report (the panel/IFC read
    the biggest face of the whole set).

    Runs inside the draw's :class:`SnapshotCompound`, before the after-state
    snapshot, so undo/redo restore the tags exactly; its own undo is a no-op
    (the snapshot restore reverts the attrs)."""

    def __init__(self, face_commands, tag: dict) -> None:
        self._face_cmds = list(face_commands)
        self._tag = dict(tag)

    def do(self, scene) -> None:
        from core.bim import next_object_id
        tag = dict(self._tag)
        tag["id"] = next_object_id(scene)
        live = scene.mesh.faces
        changed = False
        for fc in self._face_cmds:
            f = getattr(fc, "face", None)
            if f is not None and f in live and not f.attrs.get("ifc"):
                f.attrs["ifc"] = dict(tag)
                changed = True
        if changed:
            scene.mesh.mark_modified()

    def undo(self, scene) -> None:
        pass


class AddGeoPointsCommand(Command):
    """Import a batch of survey points into ``scene.geo_points`` (Track G).
    When the import also anchored the scene datum (first georef action on the
    document), the datum travels with the command so undo restores the
    previous one."""

    def doc_undo(self, scene):
        return self._datum is None

    def __init__(self, points, datum=None) -> None:
        self._points = list(points)
        self._datum = datum
        self._prev_datum = None

    def do(self, scene) -> None:
        if self._datum is not None:
            self._prev_datum = scene.georef
            scene.georef = self._datum
        scene.geo_points.extend(self._points)
        scene.version += 1

    def undo(self, scene) -> None:
        for p in self._points:
            if p in scene.geo_points:
                scene.geo_points.remove(p)
        if self._datum is not None:
            scene.georef = self._prev_datum
        scene.version += 1


class DeleteGeoPointsCommand(Command):
    """Remove survey points from ``scene.geo_points``."""

    doc_undo = True

    def __init__(self, points) -> None:
        self._points = list(points)
        self._restore: list[tuple[int, object]] = []

    def do(self, scene) -> None:
        self._restore = [(scene.geo_points.index(p), p)
                         for p in self._points if p in scene.geo_points]
        for p in self._points:
            if p in scene.geo_points:
                scene.geo_points.remove(p)
        scene.version += 1

    def undo(self, scene) -> None:
        for i, p in sorted(self._restore):
            scene.geo_points.insert(i, p)
        scene.version += 1


class DeleteGeoPathsCommand(Command):
    """Remove georef paths from ``scene.geo_paths``."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "redo":
            scene.selection.difference_update(self._paths)
        scene.version += 1

    def __init__(self, paths) -> None:
        self._paths = list(paths)
        self._restore: list[tuple[int, object]] = []

    def do(self, scene) -> None:
        self._restore = [(scene.geo_paths.index(p), p)
                         for p in self._paths if p in scene.geo_paths]
        for p in self._paths:
            if p in scene.geo_paths:
                scene.geo_paths.remove(p)
            scene.selection.discard(p)
        scene.version += 1

    def undo(self, scene) -> None:
        for i, p in sorted(self._restore):
            scene.geo_paths.insert(i, p)
        scene.version += 1


class ToggleGeoPathClosedCommand(Command):
    """Flip open ↔ closed (loop) on a set of georef paths."""

    doc_undo = True

    def __init__(self, paths) -> None:
        self._paths = list(paths)

    def _flip(self, scene) -> None:
        for p in self._paths:
            p.closed = not p.closed
        scene.version += 1

    do = _flip
    undo = _flip


class SetGeoPathSurfaceCommand(Command):
    """Set the terrain-surface mode (None/"flat"/"draped") on georef paths.

    A surface implies a closed polygon, so this also closes the path; undo
    restores the prior mode and closed flag. Built triangles are recomputed
    outside (they depend on the DEM), so they're just cleared here.
    """

    doc_undo = True

    def doc_after(self, scene, phase):
        for p in self._paths:
            p._surface_tris = None
        scene.version += 1

    def __init__(self, paths, mode) -> None:
        self.mode = mode
        self._paths = list(paths)
        self._prev: list = []

    def do(self, scene) -> None:
        self._prev = [(p, p.surface, p.closed) for p in self._paths]
        for p in self._paths:
            p.surface = self.mode
            if self.mode:
                p.closed = True
            p._surface_tris = None
        scene.version += 1

    def undo(self, scene) -> None:
        for p, surface, closed in self._prev:
            p.surface = surface
            p.closed = closed
            p._surface_tris = None
        scene.version += 1


class MoveGeoPathNodeCommand(Command):
    """Move one node of a georef path to a new position (undoable)."""

    doc_undo = True

    def __init__(self, path, index, new_point) -> None:
        from ingetrazo_core import Vec3 as QVector3D
        self.path = path
        self.index = index
        self._new = QVector3D(new_point)
        self._old = None

    def do(self, scene) -> None:
        from ingetrazo_core import Vec3 as QVector3D
        self._old = QVector3D(self.path.points[self.index])
        self.path.points[self.index] = QVector3D(self._new)
        scene.version += 1

    def undo(self, scene) -> None:
        from ingetrazo_core import Vec3 as QVector3D
        self.path.points[self.index] = QVector3D(self._old)
        scene.version += 1


class SetFaceTextureCommand(_FacePaintCommand):
    """Set or clear a texture with an independent mapping per face."""

    field = "texture"

    def __init__(self, faces, texture) -> None:
        super().__init__(faces, dict(texture) if texture is not None else None)


class SetFaceBackCommand(_FacePaintCommand):
    """Apply per-face back paint and preserve legacy two-sided flags on replay."""

    field = "back"

    def __init__(self, faces, backs) -> None:
        super().__init__(faces, None)
        if backs is None or isinstance(backs, dict):
            backs = [backs] * len(self._faces)
        self._value = [dict(b) if isinstance(b, dict) else None for b in backs]


def translate_points(scene, keys: set, delta: QVector3D) -> None:
    """Move every shared vertex whose position key is in ``keys`` by ``delta``.

    Because vertices are shared, every edge and face referencing a moved vertex
    follows for free — the mechanic behind raising a ridge into a gable roof.
    Shared by :class:`MoveVerticesCommand` and the Push/Pull live preview.
    """
    moving = _vertices_at(scene.mesh, keys)
    for v in moving:
        scene.mesh.move_vertex(v, delta)
    scene.version += 1


def _vertices_at(mesh, cells) -> list:
    """The mesh's vertices whose position key (``topology._key``) is in
    ``cells``, in mesh order. A store-backed mesh answers from its position array
    instead of a view per vertex (core/store_mesh.py)."""
    fast = getattr(mesh, "vertices_in_cells", None)
    if fast is not None:
        return fast(cells, _key)
    return [v for v in mesh.vertices if _key(v.position) in cells]


def _cells_of(mesh, vertex_keys) -> set:
    """The position keys (``topology._key``) of the vertices with these
    element keys."""
    fast = getattr(mesh, "cells_of", None)
    if fast is not None:
        return fast(vertex_keys, _key)
    return {_key(v.position) for v in mesh.vertices if v.key in vertex_keys}


def defect_names() -> dict:
    """The words a refused weld uses for each defect kind."""
    from core.i18n import tr
    return {
        "duplicate_face": tr("a face doubled over another"),
        "overlapping_face": tr("two faces overlapping in one plane"),
        "pinched_face": tr("a face that touches itself"),
        "crossed_face": tr("a face whose sides cross"),
        "flat_face": tr("a face with no area"),
        "zero_edge": tr("an edge of zero length"),
    }


def _weld_landed(mesh, moving: set) -> bool:
    from core.store_mesh import StoreMesh
    if type(mesh) is StoreMesh and not mesh.__dict__.get("_planned_points"):
        result = mesh._store.weld_landing(list(moving))
        if result is not None:
            welded, bumps, copies = result
            mesh._inherit_face_attrs(bumps, ((old, [new]) for old, new in copies))
            return welded
    from ingetrazo_core import MeshStore
    cells = _cells_of(mesh, moving)
    if not any(v.key not in moving for v in _vertices_at(mesh, cells)):
        return False
    mesh.weld_coincident()
    for face in list(mesh.faces):
        if face.hole_loops:
            continue
        loop = face.loop
        kept = MeshStore.landing_ring([(v.key, _key(v.position) in cells) for v in loop])
        if len(kept) != len(loop):
            points = [QVector3D(loop[i].position) for i in kept]
            attrs = dict(face.attrs)
            mesh.remove_face(face)
            if len(points) >= 3:
                mesh.add_face(points).attrs = attrs
    return True


def _carried_faces(mesh, moving: set) -> list:
    """Textured faces whose outer and inner rings move as a unit."""
    from core.store_mesh import FaceView, StoreMesh
    if type(mesh) is StoreMesh:
        keys = [f.key for f in mesh.textured_faces() if f.attrs.get("texture")]
        return [FaceView(mesh, k) for k in mesh._store.carried_faces(keys, list(moving))]
    from ingetrazo_core import MeshStore
    faces = [f for f in mesh.faces if f.attrs and f.attrs.get("texture")]
    rings = [[v.key for ring in (f.loop, *f.hole_loops) for v in ring] for f in faces]
    return [faces[i] for i in MeshStore.carried_object_faces(rings, set(moving))]


class _VertexEditCommand(MeshCommand):
    """Replay standalone and object-mesh edits from the shared mesh snapshot."""
    _before = _after = None
    undo = _reference.restore_mesh_before

    def do(self, scene):
        if self._after is None:
            self._before = self._reference_snapshot(scene.mesh)
            self._edit(scene)
            self._after = self._reference_snapshot(scene.mesh)
        else:
            scene.mesh.restore_state(self._after)
            scene.version += 1

    @abstractmethod
    def _edit(self, scene):
        """Apply the vertex edit to the active mesh."""


class MoveVerticesCommand(_VertexEditCommand):
    """Move shared vertices, carry whole-face textures and fold warped faces.
    A snap that introduces a local topology defect rejects the transaction.
    """

    keeps_rows = True

    def __init__(self, positions: Iterable[QVector3D], delta: QVector3D,
                 carry_textures: bool = False) -> None:
        if hasattr(delta, "toTuple"):
            self.src = [QVector3D(p) for p in positions]
            self.delta = QVector3D(delta)
        else:
            self.src = [tuple(p) for p in positions]
            self.delta = tuple(delta)
        #: The Move tool's moves: a textured face moved whole takes its
        #: image along instead of being re-projected where it lands (#1034).
        self.carry_textures = carry_textures
        #: Whether the move joined moved vertices into ones it landed on.
        self.welded = False

    def _edit(self, scene):
        keys = {_key(p) for p in self.src}
        if isinstance(self.delta, tuple):
            landing = {_key(tuple(x + d for x, d in zip(p, self.delta))) for p in self.src}
        else:
            landing = {_key(p + self.delta) for p in self.src}
        from core.store_mesh import StoreMesh, _hypot, _xyz
        from core.topology import DEFECT_KINDS, _PLANAR_TOLERANCE
        mesh = scene.mesh
        destinations = list(landing)
        started = None
        # The facade treats integer cell coordinates as already scaled.
        if (type(mesh) is StoreMesh and not mesh.__dict__.get("_planned_points")
                and all(type(x) is float for point in keys | landing for x in point)):
            started = mesh._store.begin_vertex_move(list(keys), destinations, _xyz(self.delta),
                                                    self.carry_textures, _hypot())
        if started is None:
            vertices = _vertices_at(mesh, keys)
            moving = {v.key for v in vertices}
            before = (defects_near(_vertices_at(mesh, keys | landing))
                      if any(v.key not in moving for v in _vertices_at(mesh, landing)) else None)
            delta = self.delta if type(mesh) is StoreMesh else QVector3D(*_xyz(self.delta))
            for v in vertices:
                mesh.move_vertex(v, delta)
        else:
            count, moving, before = started
            if count:
                mesh._dirty(count)
        scene.version += 1
        if self.carry_textures and moving:
            carried = _carried_faces(mesh, moving)
            if carried:
                from core.group import _remap_uvws
                from ingetrazo_core import Matrix4
                transform = Matrix4()
                transform.translate(self.delta)
                _remap_uvws(mesh, transform, carried)
        result = None
        if started is not None:
            result = mesh._store.finish_vertex_move(moving, destinations, before, _PLANAR_TOLERANCE, _hypot())
            if result is None and before is not None:
                before = dict(zip(DEFECT_KINDS, before))
        if result is None:
            self.welded = _weld_landed(mesh, set(moving))
            fold_nonplanar_faces(mesh, moved=landing)
            if self.welded and before is not None:
                worse = new_defects(before, defects_near(_vertices_at(mesh, landing)))
                if worse:
                    self._reject(scene, worse[0])
        else:
            self.welded, dirty, copies, rejection = result
            if rejection is not None:
                self._reject(scene, rejection)
            mesh._inherit_face_attrs(dirty, copies)

    def _reject(self, scene, reason):
        from core.i18n import tr
        if self._before is not None:
            scene.mesh.restore_state(self._before)
        scene.version += 1
        self.welded = False
        raise CommandRejected(tr("Not joined: it would leave {defect}. The move was undone.",
                                 defect=defect_names()[reason]))


def rotation_matrix(center: QVector3D, axis: QVector3D, degrees: float):
    """Rigid rotation of ``degrees`` around ``axis`` through ``center``."""
    from ingetrazo_core import Matrix4 as QMatrix4x4
    m = QMatrix4x4()
    m.translate(center)
    m.rotate(degrees, axis.normalized())
    m.translate(-center)
    return m


def rotate_points(scene, keys: set, matrix) -> None:
    """Rotate every shared vertex whose position key is in ``keys`` by the
    rigid ``matrix`` (the rotation twin of :func:`translate_points`). Shared
    by :class:`RotateVerticesCommand` and the Rotate tool's live preview."""
    _map_selected_vertices(scene, keys, matrix, carry_textures=False)


def _map_selected_vertices(scene, keys, matrix, carry_textures):
    from core.store_mesh import StoreMesh
    from ingetrazo_core import Matrix4
    mesh = scene.mesh
    if type(mesh) is StoreMesh and type(matrix) is Matrix4:
        count, ids = mesh._store.transform_vertices(list(keys), matrix.data(), carry_textures)
        if count:
            mesh._dirty(count)
        carried = _carried_faces(mesh, ids) if carry_textures else []
    else:
        moving = [v for v in mesh.vertices if _key(v.position) in keys]
        carried = _carried_faces(mesh, {v.key for v in moving}) if carry_textures else []
        for v in moving:
            mesh.move_vertex(v, matrix.map(v.position) - v.position)
    if carried:
        from core.group import _remap_uvws
        _remap_uvws(mesh, matrix, carried)
    scene.version += 1


def mirror_matrix(centre: QVector3D, axis: QVector3D):
    """Reflection about the plane through ``centre`` with normal ``axis``
    (Householder), as a QMatrix4x4 — the Flip tool."""
    from ingetrazo_core import Matrix4 as QMatrix4x4
    n = QVector3D(axis).normalized()
    r = QMatrix4x4(
        1 - 2 * n.x() * n.x(), -2 * n.x() * n.y(), -2 * n.x() * n.z(), 0,
        -2 * n.y() * n.x(), 1 - 2 * n.y() * n.y(), -2 * n.y() * n.z(), 0,
        -2 * n.z() * n.x(), -2 * n.z() * n.y(), 1 - 2 * n.z() * n.z(), 0,
        0, 0, 0, 1)
    t_in = QMatrix4x4()
    t_in.translate(-centre)
    t_out = QMatrix4x4()
    t_out.translate(centre)
    return t_out * r * t_in


def _member_placement(scene, group):
    """A classic member initially draws at identity in its parent's definition."""
    if (group.xform is None and getattr(scene.mesh, "_instance_definition", None) is not None
            and scene.edit_group is not None and group in scene.edit_group.children):
        from ingetrazo_core import Matrix4
        group.component = False
        return Matrix4()
    return group.xform


def _edit_local_transform(scene, group, transform):
    if scene.edit_group is not None and group in scene.edit_group.children:
        view = scene.mesh.__dict__.get("_instance_definition")
        if view is not None:
            parent = scene.mesh._store.tool_placement()
            inverse, ok = parent.inverted()
            if ok:
                return inverse * transform * parent
    return transform


class FlipGroupsCommand(MeshCommand):
    """Flip whole groups about an axis plane (the Flip tool). Instances
    compose the reflection into their transform (O(1)); classic groups map
    their vertices and re-reverse every face loop so the mirrored solid
    keeps its faces pointing OUT. A reflection is an involution: undo flips
    again."""

    keeps_paint = True

    def __init__(self, groups, centre: QVector3D, axis: QVector3D) -> None:
        self._groups = list(groups)
        self.centre = QVector3D(centre)
        self.axis = QVector3D(axis)

    def doc_undo(self, scene) -> bool:
        # Instances flip by their matrices (document fields); classic groups
        # map their vertices.
        return all(getattr(g, "xform", None) is not None for g in self._groups)

    def doc_after(self, scene, phase) -> None:
        _dirty_placements(self._groups)
        scene.version += 1

    def meshes(self):
        return [g.mesh for g in self._groups if getattr(g, "xform", None) is None]

    def geometry_meshes(self, scene):
        return self.meshes()

    def _flip(self, scene) -> None:
        m = mirror_matrix(self.centre, self.axis)
        for g in self._groups:
            pose = _member_placement(scene, g)
            if pose is not None:
                g.xform = _edit_local_transform(scene, g, m) * pose
                continue
            for v in list(g.mesh.vertices):
                g.mesh.move_vertex(v, m.map(v.position) - v.position)
            from core.group import carry_axes
            carry_axes(g, m)                  # an involution, like the rest
            from core.store_mesh import StoreMesh
            if isinstance(g.mesh, StoreMesh):
                g.mesh._store.reverse_faces(g.mesh._store.keys("face"))
            else:
                _reverse_faces(g.mesh.faces)
        if _doc.ENABLED and self.doc_undo(scene):
            _dirty_placements(self._groups)
        else:
            _dirty_group_chunks(scene)
        scene.version += 1

    def do(self, scene) -> None:
        self._flip(scene)

    def undo(self, scene) -> None:
        self._flip(scene)


class VertexTransformCommand(_VertexEditCommand):
    """Apply a matrix to loose vertices, then fold the faces they warped."""
    carry_textures = False

    @abstractmethod
    def matrix(self):
        """Transform about the tool's chosen axes and origin."""

    def _edit(self, scene):
        _map_selected_vertices(scene, {_key(p) for p in self.src}, self.matrix(), self.carry_textures)
        if faces := getattr(self, "_faces", ()):
            _reverse_faces(f for f in faces if isinstance(f, Face) or hasattr(f, "loop"))
        fold_nonplanar_faces(scene.mesh)


class FlipVerticesCommand(VertexTransformCommand):
    """Mirror loose positions about an axis plane and re-reverse the fully
    selected faces' windings (their normals would flip inward otherwise).
    Snapshot undo, the exact mirror of RotateVerticesCommand."""

    def __init__(self, positions: Iterable[QVector3D], centre: QVector3D,
                 axis: QVector3D, faces=()) -> None:
        self.src = [QVector3D(p) for p in positions]
        self.centre = QVector3D(centre)
        self.axis = QVector3D(axis)
        self._faces = list(faces)

    def matrix(self):
        return mirror_matrix(self.centre, self.axis)


class RotateVerticesCommand(VertexTransformCommand):
    """Rotate every shared vertex at a set of positions around ``axis``
    through ``center`` by ``degrees``, then **autofold** (a partial rotation
    can warp attached faces out of plane, same as Move). Undo/redo restore
    identity-preserving snapshots — the exact mirror of
    :class:`MoveVerticesCommand`."""

    def __init__(self, positions: Iterable[QVector3D], center: QVector3D,
                 axis: QVector3D, degrees: float,
                 carry_textures: bool = False) -> None:
        self.src = [QVector3D(p) for p in positions]
        self.center = QVector3D(center)
        self.axis = QVector3D(axis)
        self.degrees = degrees
        #: As in MoveVerticesCommand: a face turned whole turns its image.
        self.carry_textures = carry_textures

    def matrix(self):
        return rotation_matrix(self.center, self.axis, self.degrees)


class GroupTransformCommand(MeshCommand):
    """A classic group edits its mesh, an occurrence edits its placement."""
    def geometry_meshes(self, scene):
        return () if self.group.xform is not None else (self.group.mesh,)


class RotateGroupCommand(GroupTransformCommand):
    """Rotate a whole group's isolated mesh (rigid — nothing folds). Snapshot
    undo/redo on the group's own mesh."""

    keeps_paint = True

    def doc_undo(self, scene) -> bool:
        # An instance turns by its matrix, and undo puts back that exact
        # matrix (not the inverse turn composed in float32).
        return getattr(self.group, "xform", None) is not None

    def __init__(self, group, center: QVector3D, axis: QVector3D,
                 degrees: float) -> None:
        self.group = group
        self.center = QVector3D(center)
        self.axis = QVector3D(axis)
        self.degrees = degrees
        self._before: Optional[dict] = None
        self._after: Optional[dict] = None

    def do(self, scene) -> None:
        pose = _member_placement(scene, self.group)
        if pose is not None:
            m = rotation_matrix(self.center, self.axis, self.degrees)
            self.group.xform = _edit_local_transform(scene, self.group, m) * pose
            scene.version += 1
            return
        gmesh = self.group.mesh
        if self._after is not None:  # redo
            gmesh.restore_state(self._after)
            self.group.axes = self._axes_after
            scene.version += 1
            return
        self._before = self._reference_snapshot(gmesh)
        self._axes_before = self.group.axes
        m = rotation_matrix(self.center, self.axis, self.degrees)
        for v in list(gmesh.vertices):
            gmesh.move_vertex(v, m.map(v.position) - v.position)
        from core.group import _remap_uvws, carry_axes
        _remap_uvws(gmesh, m)                 # the texture turns with it
        carry_axes(self.group, m)             # and so do its axes (#44)
        self._axes_after = self.group.axes
        self._after = self._reference_snapshot(gmesh)
        scene.version += 1

    def undo(self, scene) -> None:
        if getattr(self.group, "xform", None) is not None:
            m = rotation_matrix(self.center, self.axis, -self.degrees)
            self.group.xform = _edit_local_transform(scene, self.group, m) * self.group.xform
            scene.version += 1
            return
        if self._before is not None:
            self.group.mesh.restore_state(self._before)
            self.group.axes = self._axes_before
            scene.version += 1


def scale_matrix(center: QVector3D, factor, axes=None):
    """Scale about ``center``: a float scales uniformly, a 3-tuple scales each
    axis by its own factor (the edge/face grips). A negative factor
    mirrors through the centre along that axis (allowed — dragging
    a grip past its anchor, or typing -1).

    ``axes`` = ``(red, green, blue)`` unit vectors: the three factors act
    along THOSE instead of the world's — the grip box of a turned group, on
    its own axes (issue #44). ``None`` is the world, exactly as before."""
    from ingetrazo_core import Matrix4 as QMatrix4x4
    if axes is not None and isinstance(factor, (tuple, list)):
        from core.axes import frame_matrix
        rot = frame_matrix(QVector3D(0.0, 0.0, 0.0), *axes)
        inv = rot.transposed()
        s = QMatrix4x4()
        s.scale(float(factor[0]), float(factor[1]), float(factor[2]))
        m = QMatrix4x4()
        m.translate(center)
        m = m * rot * s * inv
        back = QMatrix4x4()
        back.translate(-center)
        return m * back
    m = QMatrix4x4()
    m.translate(center)
    if isinstance(factor, (tuple, list)):
        m.scale(float(factor[0]), float(factor[1]), float(factor[2]))
    else:
        m.scale(float(factor))
    m.translate(-center)
    return m


def invert_scale_factor(factor):
    """The factor that undoes ``factor`` — per-axis for tuples. Zero factors
    (already rejected upstream) invert to 1.0 instead of dividing by zero."""
    def inv(f):
        return 1.0 / f if abs(f) > 1e-12 else 1.0
    if isinstance(factor, (tuple, list)):
        return tuple(inv(float(f)) for f in factor)
    return inv(float(factor))


class ScaleVerticesCommand(VertexTransformCommand):
    """Uniformly scale every shared vertex at a set of positions about
    ``center`` by ``factor``, then autofold (scaling a subset of connected
    geometry can warp attached faces). Snapshot undo/redo — the mirror of
    Move/RotateVerticesCommand."""

    def __init__(self, positions: Iterable[QVector3D], center: QVector3D,
                 factor: float, axes=None) -> None:
        self.src = [QVector3D(p) for p in positions]
        self.center = QVector3D(center)
        self.factor = factor
        self.axes = axes

    def matrix(self):
        return scale_matrix(self.center, self.factor, self.axes)


class ScaleGroupCommand(GroupTransformCommand):
    """Uniformly scale a whole group's isolated mesh about ``center``.
    Snapshot undo/redo on the group's own mesh."""

    keeps_paint = True

    def doc_undo(self, scene) -> bool:
        return getattr(self.group, "xform", None) is not None

    def __init__(self, group, center: QVector3D, factor: float,
                 axes=None) -> None:
        self.group = group
        self.center = QVector3D(center)
        self.factor = factor
        self.axes = axes
        self._before: Optional[dict] = None
        self._after: Optional[dict] = None

    def do(self, scene) -> None:
        pose = _member_placement(scene, self.group)
        if pose is not None:
            m = scale_matrix(self.center, self.factor, self.axes)
            self.group.xform = _edit_local_transform(scene, self.group, m) * pose
            scene.version += 1
            return
        gmesh = self.group.mesh
        if self._after is not None:  # redo
            gmesh.restore_state(self._after)
            self.group.axes = self._axes_after
            scene.version += 1
            return
        self._before = self._reference_snapshot(gmesh)
        self._axes_before = self.group.axes
        m = scale_matrix(self.center, self.factor, self.axes)
        for v in list(gmesh.vertices):
            gmesh.move_vertex(v, m.map(v.position) - v.position)
        from core.group import _remap_uvws, carry_axes
        _remap_uvws(gmesh, m)                 # the texture scales with it
        carry_axes(self.group, m)             # the origin moves with it (#44)
        self._axes_after = self.group.axes
        self._after = self._reference_snapshot(gmesh)
        scene.version += 1

    def undo(self, scene) -> None:
        if getattr(self.group, "xform", None) is not None:
            m = scale_matrix(self.center, invert_scale_factor(self.factor), self.axes)
            self.group.xform = _edit_local_transform(scene, self.group, m) * self.group.xform
            scene.version += 1
            return
        if self._before is not None:
            self.group.mesh.restore_state(self._before)
            self.group.axes = self._axes_before
            scene.version += 1


class PruneOrphanEdgesCommand(MeshCommand):
    """Remove edges incident to ``vertices`` that, once the rest of a compound
    has run, border no face — the dangling lines left where push/pull carved
    geometry away. Computed at ``do`` time so it reflects the real post-carve
    scene; ``undo`` re-links the swept edges."""

    def __init__(self, vertices: Iterable[QVector3D]) -> None:
        self.vertices = [QVector3D(v) for v in vertices]
        self.removed: list[Edge] = []

    def do(self, scene) -> None:
        from core import push_scans
        from core.topology import _key
        # Only an edge with an end near the vertices can be swept, and only
        # a face with a corner there can still hold it: those, in list
        # order (natively), stand in for every edge and face.
        keys = [_key(v) for v in self.vertices]
        edges = push_scans.edges_near_points(scene.mesh, keys)
        faces = push_scans.faces_near_points_rings(scene.mesh, keys)
        from core.store_mesh import StoreMesh, EdgeView
        if type(scene.mesh) is StoreMesh:
            store = scene.mesh._store
            prune = store.prune_orphan_edges if self._journal_geometry else store.orphaned_edges_at
            keys = prune(
                [v.toTuple() for v in self.vertices],
                None if edges is None else [e.key for e in edges],
                None if faces is None else [f.key for f in faces])
            self.removed = [EdgeView(scene.mesh, k) for k in keys]
            if self._journal_geometry:
                if keys:
                    scene.mesh._dirty(len(keys))
                    for edge in self.removed:
                        scene.selection.discard(edge)
                    scene.version += 1
                return
        else:
            self.removed = orphaned_edges_at(
                scene.edges if edges is None else edges,
                scene.faces if faces is None else faces, self.vertices)
        for edge in self.removed:
            scene.mesh.remove_edge(edge)
            scene.selection.discard(edge)
        if self.removed:
            scene.version += 1

    def undo(self, scene) -> None:
        for edge in self.removed:
            scene.mesh.relink_edge(edge)
        if self.removed:
            scene.version += 1
        self.removed = []


class CoplanarMergeCommand(MeshCommand):
    """Dissolve coplanar seams left by push/pull.

    After a wall is pushed flush against an adjacent one, the shared edge borders
    two faces in the same plane and carries no silhouette — a phantom line. This
    command sweeps the edges incident to the operation's vertices and merges any
    such redundant pair into one face (the "L"), so the result reads as a clean
    solid. Seeded with the operation's vertices (not the whole model) so a
    *deliberately* drawn coplanar edge elsewhere is left alone.

    Each merge is recorded as ``(face_a, face_b, edge, merged_face)`` so undo
    restores the exact objects other commands may reference.
    """

    def __init__(self, seed_positions: Iterable[QVector3D]) -> None:
        self.seed = [QVector3D(p) for p in seed_positions]
        self.merges: list[tuple[Face, Face, Edge, Face]] = []

    def do(self, scene) -> None:
        mesh = scene.mesh
        seedkeys = {_key(p) for p in self.seed}
        from core.store_mesh import EdgeView, _store_of
        from ingetrazo_core import merge_seeded_edges
        if merge_seeded_edges(mesh, list(seedkeys), None if self._journal_geometry else self.merges,
                              _store_of, EdgeView):
            scene.version += 1

    def undo(self, scene) -> None:
        mesh = scene.mesh
        from ingetrazo_core import revert_seeded_merges
        revert_seeded_merges(mesh, self.merges)
        if self.merges:
            scene.version += 1
        self.merges = []


class StitchSolidCommand(MeshCommand):
    """Make a solid watertight again after push/pull.

    Repeated pushes leave three kinds of connectivity debris: edges that run past
    a vertex belonging to a neighbour (a *T-junction* — the two sides share a
    line but no edge, so the seam reads as a naked crack), redundant valence-2
    collinear vertices left by mismatched subdivision, and coplanar faces that
    should be one. This runs in three phases:

    1. **Resolve T-junctions** (seeded): split every edge that reaches the
       operation's own bounds at any vertex on its interior, so mismatched
       subdivisions share edges → no naked cracks.
    2. **Collapse collinear vertices** (global): drop spurious valence-2 points.
    3. **Coplanar-merge** (seeded): fuse coplanar faces around the operation
       into one — seeded so a deliberately drawn coplanar line elsewhere stays.

    Phases 1–2 only repair connectivity (no shape change). The splits,
    collapses and merges interact too tightly for a clean per-op inverse, so
    undo restores an identity-preserving snapshot taken before the pass —
    robust, and it keeps the surrounding delta commands' object references
    valid (this command runs last in the push/pull compound).
    """

    def __init__(self, seed_positions: Iterable[QVector3D]) -> None:
        self.seed = [QVector3D(p) for p in seed_positions]
        self.snapshot: Optional[dict] = None

    def do(self, scene) -> None:
        self.snapshot = self._reference_snapshot(scene.mesh)
        run_stitch(scene.mesh, {_key(p) for p in self.seed})
        scene.version += 1

    def undo(self, scene) -> None:
        if self.snapshot is not None:
            scene.mesh.restore_state(self.snapshot)
            scene.version += 1


def run_stitch(mesh, seedkeys: set, new_faces: Optional[set] = None,
               coplanar_merge: bool = True, dedupe: bool = True) -> None:
    """Three-phase watertight cleanup (no undo bookkeeping — the caller snapshots).
    See :class:`StitchSolidCommand` for the rationale of each phase.

    ``seedkeys`` are the operation's own positions (``core.topology._key``
    tuples). They bound phase 1 — only an edge reaching that box can be split,
    which is every T-junction the op can have created and none of the ones it
    found lying around — and seed phase 3.

    ``new_faces`` (when given) are the faces this operation created; phase 3 only
    fuses a coplanar component that contains one of them, so a seam a push just
    made is merged while a pre-existing coplanar split (a user's diagonal) is
    left intact. ``None`` means merge any seeded component (manual stitch).

    ``coplanar_merge=False`` runs only phases 0–2 (connectivity repair). Solid
    push/pull uses this: its seams are dissolved by the deterministic per-plane
    rebuild (:mod:`core.cap_rebuild`) instead of the winding-tolerant merge,
    which only remains for raw/open geometry where outwardness is undefined.

    ``dedupe=False`` skips the identical-cycle face dedupe of phase 0. The
    solid path's *first* stitch needs that: a flush-collapse sweep quad lands
    identical to the face it must annihilate *with*, and the per-plane rebuild
    is what decides whether the pair means "keep one" (a shared wall built
    twice — material on both sides) or "drop both" (an emptied region) — by
    parity, where the pair's two crossings cancel. Deduping it early restores
    the material reading and the collapse never classifies."""
    # Phase 0 — weld coincident vertices (a translated cap landing flush on the
    # ring it came from); merges duplicate edges, drops degenerated faces. Then
    # drop faces stacked on an identical cycle (a shared wall built twice).
    mesh.weld_coincident()
    if dedupe:
        mesh.dedupe_faces()
    # Phase 1 — resolve T-junctions (global). Batch form: the per-edge
    # ``interior_vertex_on`` walked EVERY vertex in Python and the loop
    # restarted from edge zero after each split — O(splits × E × V). An
    # imported brick barbecue (3k faces, T-junctions everywhere) hung the
    # push preview for minutes per drag frame (SIGUSR1 autopsy: two stacks
    # pinned at interior_vertex_on).
    #
    # Now the sweep is ONE vectorised pass, and it is **scoped to the
    # operation**: only edges whose box meets the seed's box can be split.
    #
    # That scope is a correctness fix as much as a speed one. Sweeping every
    # edge made a push *silently re-topologise geometry it never touched* —
    # on an imported barbecue, pushing one brick split ten edges, eight of
    # them a metre away on an unrelated plane, and since a drag preview
    # reverts each frame it redid that same distant repair on every mouse
    # move. A stitch repairs what its operation disturbed; a T-junction the
    # op did not create is not its business.
    #
    # The scope is exact for what the op *can* create: a split needs the
    # splitting vertex to lie on the edge, and either the vertex is the op's
    # (so it is in the seed box, and the edge's box must reach it) or the
    # edge is the op's (so its box is inside the seed box). Either way the
    # edge's box meets the seed box.
    from core.topology import sweep_tjunctions
    sweep_tjunctions(mesh, seedkeys)
    # Phase 2 — collapse redundant valence-2 collinear vertices (global): the
    # first collapsible vertex in list order, until none is left
    # (``Mesh.collapse_collinear``; a store-backed mesh does it natively).
    mesh.collapse_collinear()
    if not coplanar_merge:
        return
    # Phase 3 — coplanar-merge, seeded to the operation. Fuses the whole coplanar
    # component (every shared edge, any number of faces) at once, but only when it
    # includes a face this operation created (so user diagonals survive).
    from core.store_mesh import FaceView, _hypot, _store_of
    start = 0
    while (store := _store_of(mesh)) is not None:
        fresh = None if new_faces is None else {f.key for f in new_faces}
        match = store.stitch_region(list(seedkeys or ()), fresh, start, _hypot())
        if match is None:
            return
        start, keys = match
        component = {FaceView(mesh, k) for k in keys}
        region = mesh.dissolve_coplanar_region(component)
        if region is not None:
            if new_faces is not None:
                new_faces.difference_update(component)
                new_faces.add(region)
            start = 0
    _merge_object_regions(mesh, seedkeys, new_faces, start)


def _merge_object_regions(mesh, seedkeys, new_faces, start):
    from ingetrazo_core import merge_materialised_regions
    return merge_materialised_regions(mesh, list(seedkeys), new_faces, start)


class SnapshotMutation(MeshCommand):
    """Wrap an arbitrary mesh mutation with snapshot undo. The push/pull live
    preview applies the *same* mutation each drag frame (then reverts via its own
    snapshot), so the forming solid renders exactly as it will commit — clean,
    already stitched — instead of flashing the pre-stitch seams.

    Redo restores the captured *result* rather than re-running the mutation: the
    closure usually closes over tool state (``base_face`` etc.) that is reset
    right after commit, so re-running it on redo would crash — and re-running the
    deterministic plane rebuild on stale state would be wasteful besides.

    ``mesh`` (when given) is the mesh to snapshot instead of the scene's loose
    one — a push/pull aimed at a Group edits that group's isolated mesh."""

    keeps_rows = True

    def __init__(self, mutate, mesh: Optional[Mesh] = None) -> None:
        self.mutate = mutate
        self._mesh = mesh
        self.before: Optional[dict] = None
        self.after: Optional[dict] = None

    @classmethod
    def from_states(cls, before: dict, after: dict,
                    mesh: Optional[Mesh] = None) -> "SnapshotMutation":
        """A mutation already applied to the model, given its two snapshots:
        ``do`` takes the redo branch (restores ``after``) and nothing is
        re-run. Push/Pull commits its staged preview this way — the model
        already shows the result."""
        cmd = cls(None, mesh=mesh)
        cmd.before, cmd.after = before, after
        return cmd

    def geometry_meshes(self, scene):
        return (self._target(scene),)

    def _target(self, scene) -> Mesh:
        return self._mesh if self._mesh is not None else scene.mesh

    def do(self, scene) -> None:
        mesh = self._target(scene)
        if self.after is None:
            self.before = self._reference_snapshot(mesh)
            self.mutate(scene)
            self.after = self._reference_snapshot(mesh)
        else:
            mesh.restore_state(self.after)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.before is not None:
            self._target(scene).restore_state(self.before)
            scene.version += 1


class SimplifyMeshCommand(MeshCommand):
    """Merge near-coplanar faces of the active mesh with snapshot undo."""

    def __init__(self, max_angle_degrees: float = 0.0) -> None:
        self.max_angle_degrees = float(max_angle_degrees)
        self.before: Optional[dict] = None
        self.after: Optional[dict] = None
        self.before_selection: set = set()
        self.faces_removed = 0

    def do(self, scene) -> None:
        if self.after is None:
            from formats.fuse import simplify_mesh
            self.before = self._reference_snapshot(scene.mesh)
            self.before_selection = ElementSet(scene.selection)
            self.faces_removed = simplify_mesh(
                scene.mesh, self.max_angle_degrees)
            self.after = self._reference_snapshot(scene.mesh)
        else:
            scene.mesh.restore_state(self.after)
        scene.selection.clear()
        scene.version += 1

    def doc_after(self, scene, phase):
        scene.selection = set(self.before_selection) if phase == "undo" else set()
        scene.version += 1

    def undo(self, scene) -> None:
        if self.before is not None:
            scene.mesh.restore_state(self.before)
            scene.selection = set(self.before_selection)
            scene.version += 1


class SnapshotImport(Command):
    """Wrap a file import that may add loose geometry AND/OR reference groups
    (big DAE/OBJ models land as a Group). SnapshotMutation only snapshots the
    mesh, so a group added by the loader would survive undo — this also
    remembers and reverts the groups the import appended.

    Sheets too: an agent asked for "a new sheet" appends a composition
    through ``run_python``, and untracked it was no step at all — the
    bridge said nothing changed, and its undo took the user's previous
    edit instead (#694).

    And what the mutation removed: an extension that replaces its own
    container group, or a console line that deletes one, lost that group
    for good on undo, which only took the new one away."""

    # Scene list -> (what the mutation added, what it removed). The removed
    # entries are (index before the mutation, item), so undo puts each one
    # back where it was.
    _TRACKED = (
        ("groups", "added_groups", "removed_groups"),
        ("layers", "added_layers", "removed_layers"),
        ("saved_views", "added_views", "removed_views"),
        ("dimensions", "added_dims", "removed_dims"),
        ("text_labels", "added_texts", "removed_texts"),
        ("compositions", "added_sheets", "removed_sheets"),
    )

    def __init__(self, mutate) -> None:
        self.mutate = mutate
        self.before: Optional[dict] = None
        self.after: Optional[dict] = None
        for _, added, removed in self._TRACKED:
            setattr(self, added, [])
            setattr(self, removed, [])

    @property
    def changed(self) -> bool:
        """Whether the run did anything undo would have to revert."""
        return (self.before != self.after
                or any(getattr(self, added) or getattr(self, removed)
                       for _, added, removed in self._TRACKED))

    def do(self, scene) -> None:
        m = scene.mesh
        if self.after is None:
            lists_before = {name: list(getattr(scene, name))
                            for name, _, _ in self._TRACKED}
            self.before = m.capture_state()
            try:
                self.mutate(scene)
            except BaseException:
                # Restore membership and order, not only newly appended items:
                # failed recipes may also remove or reorder existing objects.
                # Slice assignment preserves references held by scripting scopes.
                for name, items in lists_before.items():
                    getattr(scene, name)[:] = items
                raise
            self.after = m.capture_state()
            for name, added, removed in self._TRACKED:
                before, now = lists_before[name], getattr(scene, name)
                setattr(self, added, [x for x in now if x not in before])
                setattr(self, removed, [(i, x) for i, x in enumerate(before)
                                        if x not in now])
        else:
            m.restore_state(self.after)
            for name, added, removed in self._TRACKED:
                items = getattr(scene, name)
                for _, x in getattr(self, removed):
                    if x in items:
                        items.remove(x)
                    if name == "groups":
                        scene.selection.discard(x)
                for x in getattr(self, added):
                    if x not in items:
                        items.append(x)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.before is not None:
            scene.mesh.restore_state(self.before)
        for name, added, removed in self._TRACKED:
            items = getattr(scene, name)
            for x in getattr(self, added):
                if x in items:
                    items.remove(x)
                if name == "groups":
                    scene.selection.discard(x)
            # Ascending order of the old index, so each insert lands where
            # the item was once the ones before it are back.
            for i, x in getattr(self, removed):
                if x not in items:
                    items.insert(min(i, len(items)), x)
        scene.version += 1


def _run_snapshot_parts(parent, scene):
    # These drawing operations edit only the mesh already enlisted by the
    # enclosing snapshot. Other parts can have additional geometry owners.
    covered = (AddEdgeCommand, DeleteEdgesCommand, DeleteFaceCommand, TagCurveCommand,
               ExplodeCurvesCommand, PruneOrphanEdgesCommand, MoveVerticesCommand)
    for part in parent.inner:
        # A face's reference inverse also uses snapshots. The enclosing
        # command already owns them in either document mode.
        if type(part) is not AddFaceCommand and (
                not parent._journal_geometry or type(part) not in covered):
            part.do(scene)
            continue
        previous = part._journal_geometry
        part._journal_geometry = True
        try:
            part.do(scene)
        finally:
            part._journal_geometry = previous


class SnapshotCompound(MeshCommand):
    """Run a list of sub-commands under one identity-preserving snapshot, undone
    by restoring it.

    The line-draw plan splits and welds edges (and punches holes in coplanar
    faces); the per-command inverses don't compose into a clean whole — undoing
    them piecemeal leaves orphan split edges and stray vertices behind. One
    snapshot of the entire edit reverses it exactly, and because identity is
    preserved, earlier history entries keep working across this undo.
    ``heal=False`` preserves supplied copy topology instead of applying
    drawing-specific global face cleanup.
    """

    keeps_rows = True

    def __init__(self, inner: Iterable[Command], *, heal: bool = True) -> None:
        self.inner = list(inner)
        self._heal = heal
        self.before: Optional[dict] = None
        self.after: Optional[dict] = None

    def do(self, scene) -> None:
        if self.after is None:
            # First run: snapshot before, apply the plan, clean up any coplanar
            # overlap it created (redundant nested holes / spurious mother), then
            # snapshot the result so undo/redo restore exactly.
            self.before = self._reference_snapshot(scene.mesh)
            # Which way every face looked before the draw: the heal may
            # align the faces the draw MADE, never the ones already there
            # (a Reverse Faces undone by the next rectangle).
            footprint = winding_footprint(scene.mesh) if self._heal else None
            _run_snapshot_parts(self, scene)
            if self._heal:
                for f in heal_overlapping_faces(scene.mesh, footprint=footprint):
                    scene.selection.discard(f)
            # A draw that split a curve leaves it in separate contours — break
            # the curve ids there, before the snapshot so redo keeps it.
            scene.mesh.resplit_curves()
            self.after = self._reference_snapshot(scene.mesh)
        else:
            # Redo: re-running the delta plan wouldn't reproduce the splits, so
            # restore the captured result directly.
            scene.mesh.restore_state(self.after)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.before is not None:
            scene.mesh.restore_state(self.before)
            scene.version += 1


class MeshSnapshotCommand(MeshCommand):
    """Run a list of sub-commands plus the stitch pass under a single
    identity-preserving snapshot, undone by restoring that snapshot.

    Push/pull builds its edit as delta commands and then stitches the result
    watertight; the stitch's splits/merges restructure the very edges those
    commands own, so composing their individual undos leaves orphan edges. One
    snapshot of the whole push is exact and robust — and because it preserves
    object identity, *other* history entries (a drawn line, an earlier push) keep
    working across this undo."""

    def __init__(self, inner: Iterable[Command], stitch_seed: Iterable[QVector3D]) -> None:
        self.inner = list(inner)
        self.seed = [QVector3D(p) for p in stitch_seed]
        self.snapshot: Optional[dict] = None

    def do(self, scene) -> None:
        self.snapshot = self._reference_snapshot(scene.mesh)
        before = set(scene.mesh.faces)
        _run_snapshot_parts(self, scene)
        new_faces = set(scene.mesh.faces) - before
        run_stitch(scene.mesh, {_key(p) for p in self.seed}, new_faces)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.snapshot is not None:
            scene.mesh.restore_state(self.snapshot)
            scene.version += 1


class MakeGroupCommand(MeshCommand):
    """Encapsulate the selected faces and edges into a new Group with its own
    mesh, removing them from the loose mesh so they no longer weld to the rest.
    Snapshot undo (geometry crosses meshes — too tangled for a per-op inverse)."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "undo":
            if not self._journal_geometry:
                scene.mesh.restore_state(self.snapshot)
            scene.selection.discard(self.group)
        else:
            if not self._journal_geometry:
                scene.mesh.restore_state(self._after)
            scene.selection.clear()
            scene.selection.add(self.group)
        scene.version += 1

    def __init__(self, faces: Iterable[Face], edges: Iterable[Edge],
                 component: bool = False, name: str | None = None) -> None:
        # ``component=True`` builds the fresh mesh in LOCAL coordinates
        # (origin at the selection's min corner) and hands back an INSTANCE
        # (Group with an xform) — Make Component: copies of it
        # share the definition through the existing prototype machinery.
        self._component = component
        self._name = name
        faces = list(faces)
        # Attrs (colour, texture, layer, IFC tag) must travel into the group's
        # fresh mesh with each face — grouping a painted set used to strip it.
        self._face_loops = [
            ([QVector3D(v) for v in f.vertices],
             [[QVector3D(v) for v in h] for h in f.holes],
             dict(f.attrs))
            for f in faces
        ]
        self._edge_ends = [(QVector3D(e.a), QVector3D(e.b)) for e in edges]
        # Edge flags must travel into the group's fresh mesh: without them,
        # grouping a smooth cylinder suddenly shows every facet seam (soft
        # edges hidden in the loose mesh, visible in the group), its rims stop
        # selecting as whole curves, and a HIDDEN edge comes back from the
        # dead — «oculté una arista, agrupo el dibujo y la arista vuelve a
        # aparecer» (Marco, 2026-09-10). They travel as one tuple now, so the
        # next flag added to an edge cannot be forgotten here.
        self._flagged: list = []

        def note(e):
            if e is not None and not edge_is_plain(e):
                self._flagged.append(
                    (QVector3D(e.a), QVector3D(e.b), edge_flags(e)))

        for f in faces:
            for lp in (f.loop, *f.hole_loops):
                n = len(lp)
                for i in range(n):
                    va, vb = lp[i], lp[(i + 1) % n]
                    note(next((k for k in va.edges if k.other(va) == vb),
                              None))
        for e in edges:
            note(e)
        self.snapshot: Optional[dict] = None
        self.group: Optional[Group] = None

    def do(self, scene) -> None:
        m = scene.mesh
        self.snapshot = self._reference_snapshot(m)
        # Built once: a redo puts back the SAME group, the one the next
        # commands (Explode, Move, paint…) hold — a fresh one each time
        # left them pointing at a group no longer in the model, and
        # their redo was lost (#737).
        if self.group is None:
            self.group = self._build(scene)
        self._take_off_loose(scene)
        if _doc.ENABLED and not self._journal_geometry:
            self._after = m.capture_state()
        scene.groups.append(self.group)
        scene.selection.clear()
        scene.selection.add(self.group)
        scene.version += 1

    def _build(self, scene) -> Group:
        # The group is a fresh copy built from the captured positions. A
        # COMPONENT shifts them into local coordinates around the origin.
        origin = QVector3D(0.0, 0.0, 0.0)
        # Made inside a turned group, the new one lines up with THAT
        # context's axes (issue #44): its origin is the lowest
        # corner measured along them, not along the world's.
        frame = getattr(scene, "drawing_frame", None)
        pts = [p for loop, holes, _a in self._face_loops
               for lst in (loop, *holes) for p in lst]
        pts += [p for pair in self._edge_ends for p in pair]
        rot = None
        if frame is not None and pts:
            from core.group import frame_axes
            fo, fx, fy, fz = frame_axes(frame)

            def loc(p):
                d = QVector3D(p) - fo
                return (QVector3D.dotProduct(d, fx), QVector3D.dotProduct(d, fy),
                        QVector3D.dotProduct(d, fz))
            ls = [loc(p) for p in pts]
            lo = (min(q[0] for q in ls), min(q[1] for q in ls),
                  min(q[2] for q in ls))
            corner = fo + fx * lo[0] + fy * lo[1] + fz * lo[2]
            rot = (fx, fy, fz)
        if self._component and pts:
            if rot is not None:
                origin = corner
            else:
                origin = QVector3D(min(p.x() for p in pts),
                                   min(p.y() for p in pts),
                                   min(p.z() for p in pts))

        def L(p):
            if self._component and rot is not None:
                d = QVector3D(p) - origin       # into the context's axes
                return QVector3D(QVector3D.dotProduct(d, rot[0]),
                                 QVector3D.dotProduct(d, rot[1]),
                                 QVector3D.dotProduct(d, rot[2]))
            return QVector3D(p) - origin

        gmesh = Mesh()
        for loop, holes, attrs in self._face_loops:
            gf = gmesh.add_face([L(p) for p in loop],
                                [[L(p) for p in h] for h in holes] or None)
            if attrs:
                gf.attrs.update(attrs)
        for a, b in self._edge_ends:
            la, lb = L(a), L(b)
            if _key(la) == _key(lb):
                continue        # a zero-length edge is no edge (#341)
            gmesh.add_edge(la, lb)
        for a, b, flags in self._flagged:
            va, vb = gmesh.vertex_at(L(a)), gmesh.vertex_at(L(b))
            e = (gmesh.find_edge(va, vb)
                 if va is not None and vb is not None else None)
            if e is not None:
                stamp_edge_flags(e, flags)
        group = Group(gmesh, name=self._name)
        if self._component:
            from ingetrazo_core import Matrix4 as QMatrix4x4
            if rot is not None:
                from core.axes import frame_matrix
                group.xform = frame_matrix(origin, *rot)
            else:
                t = QMatrix4x4()
                t.translate(origin)
                group.xform = t
        elif rot is not None:
            from core.axes import frame_matrix
            group.axes = frame_matrix(corner, *rot)
        if scene.__dict__.get("_groups_owner") is not None and getattr(scene.mesh, "_instance_definition", None) is not None:
            inverse, _ = scene.mesh._store.tool_placement().inverted()
            if group.xform is None:
                from core.group import carry_axes, transformed_mesh
                group.mesh = transformed_mesh(group.mesh, inverse)
                carry_axes(group, inverse)
            else:
                group.xform = inverse * group.xform
        return group

    def _take_off_loose(self, scene) -> None:
        m = scene.mesh
        # Remove the grouped geometry from the loose mesh.
        face_keysets = [frozenset(_key(p) for p in loop)
                        for loop, _h, _a in self._face_loops]
        for f in list(m.faces):
            if frozenset(_key(p) for p in f.vertices) in face_keysets:
                m.remove_face(f)
                scene.selection.discard(f)
        grouped = {_key(p) for loop, holes, _a in self._face_loops
                   for lst in (loop, *holes) for p in lst}
        sel_edges = {frozenset((_key(a), _key(b))) for a, b in self._edge_ends}
        for e in list(m.edges):
            ek = frozenset((_key(e.a), _key(e.b)))
            if not e.faces and (ek in sel_edges or
                                (_key(e.a) in grouped and _key(e.b) in grouped)):
                m.remove_edge(e)
                scene.selection.discard(e)
        # The corners only the grouped geometry used are left behind by the
        # removals above — they do not cascade on purpose — so a box handed
        # to a component left eight invisible vertices in the loose mesh,
        # still in the position registry, ready to catch the next thing
        # drawn there. Sweep them in the same step: the snapshot below takes
        # them back on undo.
        m.purge_debris()

    def undo(self, scene) -> None:
        if self.group in scene.groups:
            scene.groups.remove(self.group)
        scene.selection.discard(self.group)
        scene.mesh.restore_state(self.snapshot)
        scene.version += 1


class MakeUniqueCommand(Command):
    """Make Unique: bake a component instance into its OWN mesh
    so edits stop touching the siblings' shared definition.

    ``rename``: the command Make Unique also gives it a name of its own
    when another object in the model wears the same one — «Silla» becomes
    «Silla #2» (#620). Opening a copied group, which makes it unique on the
    way in, leaves the name alone."""

    doc_undo = True

    def __init__(self, group, rename: bool = False) -> None:
        self.group = group
        self._rename = rename
        self._name = None
        self._proto = None
        self._xform = None
        self._children = None
        self._axes = None
        self._component = True

    def do(self, scene) -> None:
        if self.group.ext and self.group.ext.get("link"):
            return          # a linked file stays its source's (make_unique)
        self._proto = self.group.mesh
        self._xform = self.group.xform
        self._children = self.group.children
        self._axes = self.group.axes
        self._component = self.group.component
        self._name = self.group.name
        if self._rename and self.group.name:
            from core.kept_components import names_in_use, numbered_name
            taken = names_in_use(scene, skip=self.group)
            if self.group.name in taken:
                self.group.name = numbered_name(self.group.name, taken)
        self.group.make_unique()
        scene.version += 1

    def undo(self, scene) -> None:
        if self._proto is None:
            return
        self.group.mesh = self._proto
        self.group.xform = self._xform
        self.group.children = self._children or []
        self.group.axes = self._axes
        self.group.component = self._component
        self.group.name = self._name
        scene.version += 1


def _move_holds(scene, placements, moved) -> None:
    """The dimensions held on a vertex inside ``placements`` hold the
    same vertex in its copy now: ``moved`` maps ``id`` of each part and
    mesh replaced to its replacement (UnlinkCommand, either way). Their
    holds would otherwise stay on the parts no longer placed — still
    reading as alive through the placement at the top of their chain."""
    dims = getattr(scene, "dimensions", None) or ()
    if not dims or not moved:
        return
    from core.dimension import VertexAnchor
    roots = {id(g) for g in placements}

    def remap(h):
        if h is None or not any(id(g) in roots or id(g) in moved
                                for g in h.chain):
            return h
        mesh = moved.get(id(h.mesh), h.mesh)
        chain = tuple(moved.get(id(g), g) for g in h.chain)
        if mesh is h.mesh:
            return VertexAnchor(h.vertex, mesh, chain)
        try:
            v = mesh.vertex_at(QVector3D(h.vertex.position))
        except Exception:  # noqa: BLE001 — a mesh that cannot answer
            v = None
        return VertexAnchor(v, mesh, chain) if v is not None else None

    for dim in dims:
        for which in ("a", "b"):
            name = "anchor_" + which
            if getattr(dim, name, None) is not None:
                setattr(dim, name, remap(getattr(dim, name)))
            holds = getattr(dim, "_holds", None)
            if holds and holds.get(which):
                holds[which] = [h for h in map(remap, holds[which])
                                if h is not None]


class _TagState:
    """What the tags tree, the link registry and the materials were at one
    moment (UnlinkCommand): undo and redo go from one such state to the
    other by changing only what differs between them, so a tag made or
    renamed in the Layers panel in between (not on the undo stack) is
    left alone."""

    _MAPS = ("folder_parents", "folder_visibility", "layer_folder_names",
             "layer_folder_uids")

    def __init__(self, scene) -> None:
        from copy import deepcopy
        from core import links
        from core.layers import folders_of, layer_values
        self.layers = list(scene.layers)
        self.attrs = {id(ly): layer_values(ly) for ly in self.layers}
        self.folders = list(folders_of(scene))
        self.maps = {name: dict(getattr(scene, name, None) or {})
                     for name in self._MAPS}
        self.registry = deepcopy(links.registry(scene))
        self.cache = dict(scene.__dict__.get("_link_tag_tables") or {})
        self.materials = dict(scene.materials)
        self.active = getattr(scene, "active_layer", None)

    @staticmethod
    def _sync(now: dict, old: dict, new: dict, copy=lambda v: v) -> None:
        for key in old.keys() | new.keys():
            a, b = old.get(key, _MISSING), new.get(key, _MISSING)
            if a is b or (a is not _MISSING and b is not _MISSING and a == b):
                continue
            if b is _MISSING:
                now.pop(key, None)
            else:
                now[key] = copy(b)

    def go(self, scene, to: "_TagState") -> None:
        """Change ``scene`` from this state to ``to``."""
        from copy import deepcopy
        from core import links
        old_ids = {id(ly) for ly in self.layers}
        new_ids = {id(ly) for ly in to.layers}
        scene.layers[:] = [ly for ly in scene.layers
                           if id(ly) in new_ids or id(ly) not in old_ids]
        have = {id(ly) for ly in scene.layers}
        for i, ly in enumerate(to.layers):
            if id(ly) not in have:
                scene.layers.insert(min(i, len(scene.layers)), ly)
        for ly in to.layers:
            a, b = self.attrs.get(id(ly), {}), to.attrs[id(ly)]
            for k, v in b.items():
                if a.get(k, _MISSING) != v:
                    setattr(ly, k, v)
        folders = [f for f in (scene.layer_folders or ())
                   if f in to.folders or f not in self.folders]
        for i, f in enumerate(to.folders):
            if f not in folders:
                folders.insert(min(i, len(folders)), f)
        maps = {}
        for name in self._MAPS:
            now = dict(getattr(scene, name, None) or {})
            self._sync(now, self.maps[name], to.maps[name])
            maps[name] = now
        for name, now in maps.items():
            setattr(scene, name, now)
        scene.layer_folders = folders
        self._sync(links.registry(scene), self.registry, to.registry, deepcopy)
        self._sync(scene.__dict__.setdefault("_link_tag_tables", {}),
                   self.cache, to.cache)
        self._sync(scene.materials, self.materials, to.materials)
        if self.active != to.active:
            scene.active_layer = to.active


_MISSING = object()


class UnlinkCommand(Command):
    """Make Unique of a linked file's placement: the link breaks and the
    geometry stays. Each placement in ``groups`` becomes the host's own
    component — its own meshes, its parts editable, saved in the host and
    no longer read from the source (``links.unlinked_copy``) — placed,
    painted and tagged as it was, while the other placements of the same
    file stay live links.

    A file linked INSIDE the source stays a live link: the host's own
    link of that file now, read-only, reloaded when that file changes,
    watched and listed in the Links tab — filled as a fresh link of it is
    (``links.fill_link``: its own tags, under its own folder in «Linked
    files»). Its source missing, it keeps the parts it showed.

    A source no placement links any more leaves the host's link registry
    (nothing watches or reloads it), and its tags become the host's own:
    out of «Linked files / <file>» to the top of the tags tree, with the
    same keys, colours and visibility, the file's folders coming along
    (``links.release_tags``: one of the same label already there is
    merged into the host's). One undo step brings the live links back
    exactly — the same part objects, meshes, registry entries and tag
    folders — and redo puts the very same copies back."""

    doc_undo = True

    def doc_after(self, scene, phase):
        undo = phase == "undo"
        moved = ({id(b): a for a, b in self._pairs} if undo else
                 {id(a): b for a, b in self._pairs})
        _move_holds(scene, self.groups, moved)
        before, after = self._tags_before.cache, self._tags_after.cache
        cache = scene.__dict__.setdefault("_link_tag_tables", {})
        _TagState._sync(cache, after if undo else before, before if undo else after)
        errors = getattr(scene, "link_errors", None)
        if isinstance(errors, dict):
            if undo:
                errors.update(self._errors)
            else:
                for path in self._errors:
                    errors.pop(path, None)
        scene.version += 1

    def __init__(self, groups) -> None:
        from core import links
        self.groups = [g for g in groups if links.is_linked(g)
                       and links.linked_ancestor(getattr(g, "owner", None)) is None]
        #: The source files of the placements, each once.
        self.paths = list(dict.fromkeys(links.link_of(g)["path"]
                                        for g in self.groups))
        self._before = None
        self._after = None
        self._tags_before = None
        self._tags_after = None
        self._errors: dict = {}
        #: ``(old, new)`` for every part and mesh copied.
        self._pairs: list = []
        #: The top-level groups holding the placements (the Metal view
        #: redraws only those), found on the first run.
        self.tops = None

    _FIELDS = ("mesh", "children", "ext", "name", "component")

    def _first(self, scene) -> None:
        """The first run: copy, then fill the files linked inside."""
        from core import links
        from core.kept_components import names_in_use, numbered_name
        self._before = [{k: getattr(g, k) for k in self._FIELDS}
                        for g in self.groups]
        self._after = []
        nested = []
        for g in self.groups:
            inner: list = []
            mesh, children, ext = links.unlinked_copy(g, self._pairs, inner)
            nested.append((links.link_of(g)["path"], inner))
            name = g.name
            if name:
                taken = names_in_use(scene, skip=g)
                taken.update(a["name"] for a in self._after)
                if name in taken:
                    name = numbered_name(name, taken)
            self._after.append(dict(mesh=mesh, children=children,
                                    ext=ext, name=name, component=True))
        for g, after in zip(self.groups, self._after):
            for k, v in after.items():
                setattr(g, k, v)
        host = getattr(scene, "_project_host_path", None)
        for outer, inner in nested:
            for old, new, part_pairs in inner:
                if links.fill_link(scene, new, mirror=outer, host_path=host):
                    self._pairs.extend(part_pairs)      # kept as it showed
                else:
                    self._pairs.extend(_nested_pairs(old, new))

    def _copied(self):
        """The groups and meshes the copies hold, the files linked inside
        them aside (theirs are their links')."""
        from core import links
        groups, meshes = [], [a["mesh"] for a in self._after]
        stack = [c for a in self._after for c in a["children"] or ()]
        while stack:
            g = stack.pop()
            if links.is_linked(g):
                continue
            groups.append(g)
            meshes.append(g.mesh)
            stack.extend(g.children or ())
        return groups, meshes

    def _release(self, scene, gone) -> None:
        """No placement links the files ``gone`` any more: their tags are
        the host's own now, out at the top of the tree
        (``links.release_tags``), the copies relabelled where one merged
        into a host tag of the same label."""
        from types import SimpleNamespace
        from core import links
        from core.purge import layer_counts
        used = layer_counts(scene)
        copies = layer_counts(SimpleNamespace(loose_mesh=None, groups=[
            SimpleNamespace(mesh=a["mesh"], children=a["children"], layer=None)
            for a in self._after]))
        outside = {k: n - copies.get(k, 0) for k, n in used.items()}
        renames: dict = {}
        for path in gone:
            renames.update(links.release_tags(scene, path, used, outside))
        if renames:
            links.relabel(*self._copied(), renames)

    def do(self, scene) -> None:
        from core import links
        if not self.groups:
            raise CommandRejected("Nothing linked is selected")
        if self.tops is None:
            from core.purge import iter_groups
            want = {id(g) for g in self.groups}
            self.tops = [root for root in scene.groups
                         if any(id(g) in want for g in iter_groups([root]))]
        live = set(links.sources(scene))
        if self._before is None:
            self._tags_before = _TagState(scene)
            self._first(scene)
        else:
            for g, after in zip(self.groups, self._after):
                for k, v in after.items():
                    setattr(g, k, v)
            self._tags_before.go(scene, self._tags_after)
        _move_holds(scene, self.groups, {id(a): b for a, b in self._pairs})
        gone = live - set(links.sources(scene))
        if self._tags_after is None:
            if gone:
                self._release(scene, gone)
            reg = links.registry(scene)
            cache = scene.__dict__.get("_link_tag_tables") or {}
            for p in gone:
                reg.pop(p, None)
                cache.pop(p, None)
            self._tags_after = _TagState(scene)
        errors = getattr(scene, "link_errors", None)
        if isinstance(errors, dict):
            self._errors = {p: errors.pop(p) for p in list(errors)
                            if links._norm(p) in gone}
        scene.version += 1

    def undo(self, scene) -> None:
        if self._before is None:
            return
        for g, before in zip(self.groups, self._before):
            for k, v in before.items():
                setattr(g, k, v)
        _move_holds(scene, self.groups, {id(b): a for a, b in self._pairs})
        self._tags_after.go(scene, self._tags_before)
        if self._errors and isinstance(getattr(scene, "link_errors", None), dict):
            scene.link_errors.update(self._errors)
        scene.version += 1


def _nested_pairs(old, new) -> list:
    """``(old, new)`` for the parts and meshes of a file linked inside a
    link just unlinked, now read again as the host's own link (``new``):
    the part read again for each one ``old`` held. ``old``'s parts came
    through the outer file, under the uids it gave them — its copy of
    this file named by ``old.source_uid`` (``links.stable_uid``)."""
    from core import links
    from core.purge import iter_groups
    origin = getattr(old, "source_uid", None)
    out = []
    if old.mesh is not None and new.mesh is not None:
        out.append((old.mesh, new.mesh))
    if not origin:
        return out
    fresh = {}
    for part in iter_groups(new.children or ()):
        src = getattr(part, "source_uid", None)
        if src:
            fresh[links.stable_uid(origin, src)] = part
    meshes = {id(old.mesh)}
    for part in iter_groups(old.children or ()):
        twin = fresh.get(getattr(part, "source_uid", None))
        if twin is None:
            continue
        out.append((part, twin))
        if id(part.mesh) not in meshes:
            meshes.add(id(part.mesh))
            out.append((part.mesh, twin.mesh))
    return out


class ReplaceComponentCommand(Command):
    """Replace Component: each target instance takes ``source``'s
    definition and keeps its own placement.

    What a copy shares — the prototype mesh, the subgroups inside it, the
    name and what was generated or tagged with it (3D text, BIM tag,
    extensions' data, face-me) — comes from ``source``; what belongs to
    this one copy stays: its matrix, layer, paint, visibility and uid, so a
    scene that hides it still finds it. The source's origin lands where the
    target's sat, turned and scaled the way the target was.

    Targets that already share ``source``'s definition, and anything that is
    not a component instance, are left alone."""

    _SHARED = ("mesh", "children", "name", "axes", "component", "text3d",
               "ifc", "ext", "billboard", "exploded")

    doc_undo = True

    def __init__(self, targets, source) -> None:
        self.source = source
        self.targets = [g for g in targets
                        if g is not source and g.is_component()
                        and g.mesh is not source.mesh]
        self._before: list = []

    def do(self, scene) -> None:
        import copy
        from core.group import copy_group
        from ingetrazo_core import Matrix4 as QMatrix4x4
        src = self.source
        self._before = [{k: getattr(g, k) for k in self._SHARED}
                        for g in self.targets]
        for g in self.targets:
            if _doc.ENABLED:
                g.definition = src.definition
            else:
                g.mesh = src.mesh
            # Shared like a copy's subgroups are (copy_group): an edit
            # inside any instance reaches all of them.
            if src.ext and src.ext.get("link"):
                # A linked file: the parts stay its own, read-only.
                from core.links import copy_parts
                g.children = copy_parts(src, g)
            else:
                g.children = [copy_group(c, _in_definition=True)
                              for c in (src.children or ())]
            g.name = src.name
            g.axes = QMatrix4x4(src.axes) if src.axes is not None else None
            g.component = True
            g.text3d = dict(src.text3d) if src.text3d else None
            g.ifc = copy.deepcopy({k: v for k, v in src.ifc.items() if k != "guid"}) if src.ifc else None
            g.ext = copy.deepcopy(src.ext) if src.ext else None
            g.billboard = src.billboard
            g.exploded = dict(src.exploded) if src.exploded else None
        scene.version += 1

    def undo(self, scene) -> None:
        for g, before in zip(self.targets, self._before):
            for k, v in before.items():
                setattr(g, k, v)
        scene.version += 1


class KeepComponentCommand(Command):
    """Keep in File, on or off, for the definition ``instance`` uses
    (``instance`` may be a kept template itself). On: a placement-free
    template joins ``scene.kept_components``, so the definition stays in
    the document with no copy placed. Off: it leaves, and a definition with
    no copy left is gone when the file is saved."""

    doc_undo = True     # ``scene.kept_components`` is a document list

    def __init__(self, instance, keep: bool = True) -> None:
        self.instance = instance
        self.keep = bool(keep)
        self._template = None
        self._index = None

    def do(self, scene) -> None:
        from core.kept_components import kept_template, make_template
        current = kept_template(scene, self.instance.mesh)
        if self.keep and current is None:
            self._template = self._template or make_template(self.instance)
            scene.kept_components.append(self._template)
            self._index = None
        elif not self.keep and current is not None:
            self._template = current
            self._index = scene.kept_components.index(current)
            scene.kept_components.remove(current)
        else:
            self._template = None           # already as asked: a no-op
        scene.version += 1

    def undo(self, scene) -> None:
        if self._template is not None:
            if self.keep:
                scene.kept_components.remove(self._template)
            else:
                scene.kept_components.insert(self._index, self._template)
        scene.version += 1


class AddKeptComponentsCommand(Command):
    """Bring ready-made definitions into the document as kept components
    (no copy placed) — «Components from File…». Undo takes them out."""

    doc_undo = True     # ``scene.kept_components`` is a document list

    def __init__(self, templates) -> None:
        self.templates = list(templates)

    def do(self, scene) -> None:
        scene.kept_components.extend(self.templates)
        scene.version += 1

    def undo(self, scene) -> None:
        for t in self.templates:
            if t in scene.kept_components:
                scene.kept_components.remove(t)
        scene.version += 1


class GroupToComponentCommand(Command):
    """Convert a CLASSIC group into a component instance in place: its mesh
    becomes the shared definition (world frame == local frame) under an
    identity transform. Free — no geometry is copied — and from then on
    Move/Rotate compose the matrix (O(1)) and copies share the definition.
    The reverse road exists too (Make Unique). Exploding a 230k-face group
    to feed Make Component ground the loose-mesh machinery for minutes
    (piscina report); this is the door that was missing."""

    doc_undo = True

    def __init__(self, group, name=None) -> None:
        self.group = group
        self._name = name
        self._old_name = None

    def do(self, scene) -> None:
        from ingetrazo_core import Matrix4 as QMatrix4x4
        self._old_name = self.group.name
        self._old_xform = self.group.xform
        self._old_component = self.group.component
        if self.group.xform is None:
            self.group.xform = QMatrix4x4()
        # A group of groups already carries a matrix: it only changes what
        # it IS — a component from now on (issue #90).
        self.group.component = True
        if self._name:
            self.group.name = self._name
        scene.version += 1

    def undo(self, scene) -> None:
        self.group.xform = self._old_xform
        self.group.component = self._old_component
        self.group.name = self._old_name
        scene.version += 1


class InsertGroupCommand(Command):
    """Insert a ready-made Group (a bundled component, a paste, a future
    collection item) into the scene, selected so the user can Move it into
    place.

    INSIDE an open container it goes into the container — the usual rule
    that whatever you create while editing a group belongs to it. A roof
    pasted while editing the pergola used to land at the top level, beside
    the plaza («se supone que todo lo que edite debería estar dentro del
    grupo», Marco, 2026-09-14). The group arrives in WORLD coordinates and
    is expressed in the container's own space when that one carries a
    matrix (entering usually pushes it down into the children already)."""

    keeps_groups = True

    def __init__(self, group, attachment=None) -> None:
        self.attachment = attachment
        self._sign_anchor = None
        self.group = group
        self._owner: Optional[list] = None
        self._baked = False

    doc_undo = True  # A baked mesh replaces the journalled definition handle.

    def doc_after(self, scene, phase) -> None:
        if phase == "undo":
            scene.selection.discard(self.group)
        else:
            scene.selection.clear()
            scene.selection.add(self.group)
        scene.version += 1

    @staticmethod
    def _target(scene):
        ctx = getattr(scene, "edit_group", None)
        if ctx is not None and getattr(ctx, "children", None) is not None:
            return ctx
        return None

    def do(self, scene) -> None:
        ctx = self._target(scene)
        if ctx is not None:
            owner = ctx.children
            frame = ctx.xform
            if getattr(scene.mesh, "_instance_definition", None) is not None:
                frame = scene.mesh._store.tool_placement()
            if frame is not None:
                inv, ok = frame.inverted()
                if ok:
                    from core.group import transformed_mesh
                    if self.group.xform is not None:
                        self.group.xform = inv * self.group.xform
                    elif getattr(scene.mesh, "_instance_definition", None) is not None:
                        self.group.component = False
                        self.group.xform = inv
                    else:
                        self.group.mesh = transformed_mesh(self.group.mesh, inv)
                        self._baked = True
                        from core.group import carry_axes
                        carry_axes(self.group, inv)
        else:
            owner = scene.groups
        owner.append(self.group)
        self._owner = owner
        if self.attachment is not None:
            from core.annotation_anchor import TextPlacementAnchor
            self._sign_anchor = TextPlacementAnchor.create(
                scene, scene.occurrence_path(self.group), self.attachment)
            if self._sign_anchor is not None:
                scene.text_placement_anchors.append(self._sign_anchor)
        scene.selection.clear()
        scene.selection.add(self.group)
        scene.version += 1

    def undo(self, scene) -> None:
        owner = self._owner if self._owner is not None else scene.groups
        if self._sign_anchor is not None and self._sign_anchor in scene.text_placement_anchors:
            scene.text_placement_anchors.remove(self._sign_anchor)
        if self.group in owner:
            owner.remove(self.group)
        scene.selection.discard(self.group)
        scene.version += 1


class EditText3DCommand(Command):
    """Re-edit a 3D text (core/text3d.py): the container keeps its identity,
    pose and own mesh; its letter children are laid out again from the new
    parameters where the old ones stood, and the name follows the text.
    Undo puts the old letters and parameters back."""

    #: Its letters (children), ``text3d`` and name are document state;
    #: redo puts back the very letters the first run laid out.
    doc_undo = True

    def __init__(self, group, params: dict) -> None:
        self.group = group
        self.params = dict(params)
        self._old: Optional[tuple] = None

    def do(self, scene) -> None:
        from core.text3d import rebuild_text_group, text_state
        g = self.group
        if self._old is None:
            self._old = (g.children, g.text3d, g.name)
        g.children = rebuild_text_group(g, self.params)
        g.text3d = text_state(self.params, g.children)
        g.name = self.params["text"].strip()[:24] or g.name
        scene.version += 1

    def undo(self, scene) -> None:
        g = self.group
        g.children, g.text3d, g.name = self._old
        scene.version += 1


class MakeNestedGroupCommand(Command):
    """Make Group when the selection already holds groups.

    Until 2026-09-11 this was refused: a container baked its children the
    moment you opened it, so nesting would have dissolved the bench and the
    pergola you just grouped. With the edit stack in place (Scene) that is
    gone, and grouping groups is what it says.

    The loose part of the selection, if any, becomes the container's OWN
    mesh — the classic behaviour: inside the new group you find the
    loose faces AND the groups, each still a group. Reusing
    :class:`MakeGroupCommand` for that half keeps one code path for taking
    geometry off the loose mesh.
    """

    def __init__(self, faces: Iterable[Face], edges: Iterable[Edge],
                 groups: Iterable[Group], name: str | None = None) -> None:
        faces, edges, groups = list(faces), list(edges), list(groups)
        self._inner = (MakeGroupCommand(faces, edges, name=name)
                       if (faces or edges) else None)
        self.groups = groups
        self.name = name
        self.container: Optional[Group] = None
        self._indices: list = []
        self._xform0 = None

    doc_undo = True

    def doc_after(self, scene, phase) -> None:
        if self._inner is not None:
            self._inner.doc_after(scene, phase)
        scene.selection.clear()
        if phase == "redo":
            scene.selection.add(self.container)
        scene.version += 1

    def do(self, scene) -> None:
        if self._inner is not None:
            self._inner.do(scene)
            self.container = self._inner.group
        elif self.container is None:
            self.container = Group(Mesh(), name=self.name)
        if self.container not in scene.groups:
            scene.groups.append(self.container)
        self._xform0 = getattr(self.container, "xform", None)
        self._indices = sorted(
            ((scene.groups.index(g), g) for g in self.groups),
            key=lambda pair: pair[0])
        for _i, g in self._indices:
            scene.groups.remove(g)
            scene.selection.discard(g)
        # In document order, not selection order: a selection is a set, and
        # the children of a group are a list somebody will read.
        self.container.adopt([g for _i, g in self._indices])
        # A GROUP of groups: ``adopt`` gives it a matrix, which does not
        # make it a component (issue #90, @fafecm).
        self.container.component = False
        scene.selection.clear()
        scene.selection.add(self.container)
        scene.version += 1

    def undo(self, scene) -> None:
        self.container.children = []
        self.container.xform = self._xform0
        for idx, g in self._indices:
            scene.groups.insert(min(idx, len(scene.groups)), g)
        if self._inner is not None:
            self._inner.undo(scene)        # loose geometry back, group gone
        elif self.container in scene.groups:
            scene.groups.remove(self.container)
        scene.selection.clear()
        scene.version += 1


class GroupInContextCommand(Command):
    """Make Group, Make Component or Explode while a group is open: what
    it makes goes INTO the open group, as whatever you create while editing
    a group belongs to it (#490, #561 — «making a new group directly while
    you are inside another one is the part still missing»), and what
    Explode lets out stays in it.

    ``inner`` is the usual command (:class:`MakeGroupCommand`,
    :class:`MakeNestedGroupCommand`, :class:`MakeComponentOfCommand`,
    :class:`ExplodeGroupCommand`), run
    with the open group's children standing in for the model's top level:
    those commands take groups from and add groups to ``scene.groups``, and
    inside a group the siblings are its children. Entering a container
    already pushed its matrix down into them, so their coordinates are the
    world's, the same the tools draw in. The open group gets the identity
    matrix a group holding groups always has (``Group.adopt``), as a group.

    Callers refuse it inside a component (``outliner.editing_inside_component``):
    components keep their parts, as the Browser's drag and drop says."""

    def __init__(self, inner, context) -> None:
        self.inner = inner
        self.context = context
        self._state = None

    def doc_undo(self, scene) -> bool:
        """By its transaction when its inner command would be: the open
        group's children are document lists, swapped in by key."""
        return _uses_doc_undo(self.inner, scene)

    def doc_after(self, scene, phase) -> None:
        self.inner.doc_after(scene, phase)
        scene.version += 1

    @property
    def group(self):
        return getattr(self.inner, "group", None) or getattr(
            self.inner, "container", None)

    def _run(self, scene, step) -> None:
        ctx = self.context
        if ctx.children is None:
            ctx.children = []
        if _doc.ENABLED:
            # The document's lists cannot be swapped like Python lists:
            # the scene reads its top level from ``ctx`` meanwhile.
            d = scene.__dict__
            prev = d.get("_groups_owner")
            d["_groups_owner"] = ctx
            try:
                step(scene)
            finally:
                d["_groups_owner"] = prev
            return
        top = scene.groups
        scene.groups = ctx.children
        try:
            step(scene)
        finally:
            scene.groups = top

    def do(self, scene) -> None:
        from ingetrazo_core import Matrix4 as QMatrix4x4
        ctx = self.context
        self._state = (ctx.xform, ctx.component)
        self._run(scene, self.inner.do)
        if ctx.children and ctx.xform is None:
            ctx.xform = QMatrix4x4()
            ctx.component = False
        scene.version += 1

    def undo(self, scene) -> None:
        self._run(scene, self.inner.undo)
        self.context.xform, self.context.component = self._state
        scene.version += 1


class ReparentGroupsCommand(Command):
    """Move groups into another group — or out to the model with
    ``target=None`` — without moving them in the world: the Browser's
    drag and drop (#219).

    A group's placement is relative to its parent, so changing parent
    changes the frame it is read in. The old parent's frame and the new
    one are both known, and the difference goes into the group: composed
    into its matrix when it has one, baked into its mesh when it is a
    classic group (and only when the two frames differ at all). Its own
    children ride along untouched, being relative to it.

    The target gets a matrix if it had none — a group that holds groups is
    always an instance (``Group.adopt``) — and stays a GROUP, not a
    component. Callers check :func:`core.outliner.reparent_problem` first:
    components keep their definition, so nothing goes in or out of one.
    """

    def __init__(self, groups, target=None) -> None:
        self.groups = list(groups)
        self.target = target
        self._moves: list = []
        self._target_state = None

    doc_undo = True  # A baked mesh replaces the journalled definition handle.

    def doc_after(self, scene, phase) -> None:
        if phase == "undo":
            scene.selection.clear()
        else:
            _unselect(scene, [m[0] for m in self._moves])
        _dirty_placements([m[0] for m in self._moves])
        scene.version += 1

    def do(self, scene) -> None:
        from ingetrazo_core import Matrix4 as QMatrix4x4
        from core import outliner
        from core.group import transformed_mesh
        up = outliner.parents(scene.groups)
        moving = outliner.topmost(scene.groups, self.groups)
        target = self.target
        self._target_state = None
        if target is not None:
            self._target_state = (target.xform, target.component)
            if target.xform is None:
                target.xform = QMatrix4x4()     # same place: identity
                target.component = False
        to_new = outliner.world_matrix(scene.groups, target)
        inv_new = to_new.inverted()[0] if to_new is not None else None
        self._moves = []
        for g in moving:
            old = up.get(id(g))
            siblings = old.children if old is not None else scene.groups
            index = siblings.index(g)
            from_old = outliner.world_matrix(scene.groups, old)
            self._moves.append((g, old, index, g.xform, g.mesh))
            delta = outliner.compose(inv_new, from_old)
            if delta is not None and not delta.isIdentity():
                if g.xform is not None:
                    g.xform = delta * g.xform
                else:
                    g.mesh = transformed_mesh(g.mesh, delta)
            siblings.remove(g)
            (target.children if target is not None
             else scene.groups).append(g)
            scene.selection.discard(g)
        if _doc.ENABLED:
            _dirty_placements([m[0] for m in self._moves])
        else:
            _dirty_group_chunks(scene)
        scene.version += 1

    def undo(self, scene) -> None:
        target = self.target
        for g, old, index, xform, mesh in reversed(self._moves):
            (target.children if target is not None
             else scene.groups).remove(g)
            g.xform, g.mesh = xform, mesh
            (old.children if old is not None
             else scene.groups).insert(index, g)
        if target is not None and self._target_state is not None:
            target.xform, target.component = self._target_state
        scene.selection.clear()
        _dirty_group_chunks(scene)
        scene.version += 1


class LockCommand(Command):
    """Lock (or unlock) groups and components: a locked object stays drawn
    but cannot be picked, selected or opened. Previous flags are kept, so a
    mixed set undoes exactly."""

    doc_undo = True

    def __init__(self, groups, locked: bool = True) -> None:
        self._groups = list(groups)
        self._locked = bool(locked)
        self._prev: list = []

    def doc_after(self, scene, phase) -> None:
        if phase == "redo" and self._locked:
            _unselect(scene, self._groups)
        scene.version += 1

    def do(self, scene) -> None:
        prev = _bulk_set(self._groups, "locked", self._locked)
        if prev is not None:
            self._prev = prev
        else:
            self._prev = [bool(getattr(g, "locked", False)) for g in self._groups]
            for g in self._groups:
                g.locked = self._locked
        if self._locked:
            _unselect(scene, self._groups)
        scene.version += 1

    def undo(self, scene) -> None:
        for g, prev in zip(self._groups, self._prev):
            g.locked = prev
        scene.version += 1


class MakeComponentOfCommand(Command):
    """Make Component over a selection that holds groups (or groups and
    loose geometry): ONE component containing them, each still a group
    inside — not one component per group (Marco, 24-09, with issue #90).
    It is Make Group's container, made a component."""

    doc_undo = True

    def doc_after(self, scene, phase):
        self._nest.doc_after(scene, phase)

    def __init__(self, faces, edges, groups, name=None) -> None:
        self._nest = MakeNestedGroupCommand(faces, edges, groups, name=name)

    @property
    def container(self):
        return self._nest.container

    def do(self, scene) -> None:
        self._nest.do(scene)
        self._nest.container.component = True
        scene.version += 1

    def undo(self, scene) -> None:
        self._nest.undo(scene)


class MergeGroupsCommand(Command):
    """Fuse several groups (component instances included) into ONE new
    group holding their combined world-space geometry — the fix-my-
    grouping path. Explode + regroup routes everything through the loose
    mesh, whose per-face machinery froze on a 100k-leaf import (piscina
    report); this merge goes group-to-group through the bulk-weld API and
    the loose mesh never hears about it. Undo restores the original
    groups at their positions."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "undo":
            scene.selection.discard(self._merged)
        else:
            scene.selection.clear()
            scene.selection.add(self._merged)
        scene.version += 1

    def __init__(self, groups) -> None:
        self._groups = list(groups)
        self._indices: Optional[list] = None
        self._merged = None

    def do(self, scene) -> None:
        import numpy as np
        from core.group import Group, world_mesh
        from core.mesh import Mesh
        if self._merged is None:
            flat: list = []
            e_flat: list = []
            e_flags: list = []
            ring_sizes: list = []
            ring_counts: list = []
            attrs_list: list = []
            for g in self._groups:
                src = world_mesh(g)      # instances: transformed, UVs refit
                for f in src.faces:
                    holes = f.holes or []
                    ring_counts.append(1 + len(holes))
                    ring_sizes.append(len(f.vertices))
                    flat.extend((v.x(), v.y(), v.z()) for v in f.vertices)
                    for h in holes:
                        ring_sizes.append(len(h))
                        flat.extend((v.x(), v.y(), v.z()) for v in h)
                    attrs_list.append(dict(f.attrs) if f.attrs else None)
                for e in src.edges:
                    e_flat.append((e.a.x(), e.a.y(), e.a.z()))
                    e_flat.append((e.b.x(), e.b.y(), e.b.z()))
                    e_flags.append((bool(getattr(e, "soft", False)),
                                    getattr(e, "curve", None),
                                    getattr(e, "layer", None),
                                    bool(getattr(e, "hidden", False))))
            mesh = Mesh()
            n_edge_pts = len(e_flat)
            if e_flat or flat:
                vobjs, inverse = mesh.bulk_weld(
                    np.array(e_flat + flat, dtype=np.float64))
                emap = None
                if e_flat:
                    emap = mesh.add_edges_welded(
                        vobjs, inverse[0:n_edge_pts:2],
                        inverse[1:n_edge_pts:2], e_flags)
                if ring_counts:
                    mesh.add_faces_welded(
                        vobjs, inverse[n_edge_pts:],
                        np.asarray(ring_sizes, dtype=np.int64),
                        np.asarray(ring_counts, dtype=np.int64),
                        attrs_list, edge_map=emap)
            merged = Group(mesh, name=self._groups[0].name)
            merged.layer = getattr(self._groups[0], "layer", None)
            self._merged = merged
        self._indices = [scene.groups.index(g) for g in self._groups]
        for g in self._groups:
            scene.groups.remove(g)
            scene.selection.discard(g)
        scene.groups.append(self._merged)
        scene.selection.clear()
        scene.selection.add(self._merged)
        scene.version += 1

    def undo(self, scene) -> None:
        if self._merged in scene.groups:
            scene.groups.remove(self._merged)
        scene.selection.discard(self._merged)
        for idx, g in sorted(zip(self._indices or [], self._groups)):
            scene.groups.insert(min(idx, len(scene.groups)), g)
        scene.version += 1


class SetGroupMaterialCommand(Command):
    """Paint a group or component instance as a whole (issue #47): the
    container's ``material`` is set — ``None`` unpaints — and every face
    inside wearing the default material shows it; faces painted themselves
    keep their own. Nothing in the mesh changes, so undo is a plain swap."""

    doc_undo = True

    def __init__(self, group, material) -> None:
        self.group = group
        self.material = dict(material) if material else None
        self._old = None

    def do(self, scene) -> None:
        self._old = getattr(self.group, "material", None)
        self.group.material = self.material
        scene.version += 1

    def undo(self, scene) -> None:
        self.group.material = self._old
        scene.version += 1


class ExplodeGroupCommand(MeshCommand):
    """Dissolve ONE level of a group: its own geometry merges back into the
    loose mesh (welding to whatever it touches) and the groups and
    components nested in it come out whole, as top-level objects in its
    place — Explode (@pacaeiro, issue #72: «the inside groups
    explode as well»). Snapshot undo restores the loose mesh, the group and
    each lifted child's placement.

    Both meshes store-backed: the copy is one store op
    (``MeshStore.paste_from``, ``core.store_parts.explode_geometry``), the
    coplanar merge's broad phase is native, and redo puts back the state
    the first run made, keys included. ``copies`` names, after a run,
    where the group's elements went: ``[(group mesh, mesh, {kind:
    [(source key, key)]})]`` (store runs only; the change feed reports it)."""

    doc_undo = True

    def doc_after(self, scene, phase):
        if phase == "undo":
            if not self._journal_geometry:
                scene.mesh.restore_state(self.snapshot)
            scene.selection = set(self._selection)
        else:
            if not self._journal_geometry:
                scene.mesh.restore_state(self._after)
            scene.selection.discard(self.group)
            if self._lifted:
                scene.selection = {row[0] for row in self._lifted}
            _drop_dead_elements(scene, scene.mesh)
        scene.version += 1

    def __init__(self, group: Group) -> None:
        self.group = group
        self.snapshot: Optional[dict] = None
        self.index: Optional[int] = None
        self._lifted: list = []   # (child, xform, mesh, material, axes, offset)
        self._selection = None
        self._before0 = None      # the store state the first run started from
        self._after = None        # ... and the one it made
        self.copies: list = []

    def do(self, scene) -> None:
        m = scene.mesh
        g = self.group
        self.snapshot = self._reference_snapshot(m)
        self.index = scene.groups.index(g)
        self._selection = ElementSet(scene.selection)
        P = getattr(g, "xform", None)
        paint = getattr(g, "material", None)
        if not self._replay(scene, m):
            transform = P
            if m.__dict__.get("_instance_definition") is not None:
                parent = m._store.tool_placement()
                transform = parent * P if P is not None else parent
            self._geometry(scene, m, g, transform, paint)
            if _doc.ENABLED and not self._journal_geometry and self._after is None:
                self._after = m.capture_state()

        # The nested objects come out one level up, whole: their placement
        # composes with the parent's (a shared prototype is never touched),
        # and an unpainted child takes the parent's paint, as its default
        # faces were already drawn with it.
        kids = list(getattr(g, "children", None) or [])
        self._lifted = [(c, c.xform, c.mesh, c.material, c.axes,
                         c.explode_offset) for c in kids]
        for c in kids:
            # Free of its component, a part keeps where an exploded view put
            # it; there is no longer anything to reassemble it into.
            c.explode_offset = None
            if P is not None:
                if c.xform is not None:
                    c.xform = P * c.xform
                else:
                    from core.group import carry_axes, transformed_mesh
                    c.mesh = transformed_mesh(c.mesh, P)
                    carry_axes(c, P)          # its axes come out with it
            if c.material is None and paint:
                c.material = {k: (dict(v) if isinstance(v, dict) else v)
                              for k, v in paint.items()}
        scene.groups.remove(g)
        # One parent per placement: what comes out leaves the group (undo
        # gives it back).
        g.children = []
        scene.groups[self.index:self.index] = kids
        scene.selection.discard(g)
        if kids:
            scene.selection = set(kids)   # select what came out
        scene.version += 1

    def _replay(self, scene, m) -> bool:
        """Redo on the store: the mesh goes to the state the first run made
        (O(chunks), the same keys) when it stands where that run started."""
        from core.store_mesh import StoreState
        if (self._after is None or not isinstance(self.snapshot, StoreState)
                or not isinstance(self._before0, StoreState)
                or self.snapshot != self._before0):
            return False
        m.restore_state(self._after)
        _drop_dead_elements(scene, m)
        return True

    def _geometry(self, scene, m, g, P, paint) -> None:
        from core.group import placed_face_paint
        from core.materials import effective_attrs
        from core.store_mesh import StoreState
        from core import store_parts
        inverse, invertible = P.inverted() if P is not None else (None, False)
        native = store_parts.explode_geometry(m, g.mesh, P, inverse,
                                              invertible, paint)
        if native is not None:
            added, _change, keys = native
            m.resplit_curves()
            if _merge_overlapping_planes(scene, None, added, others=True):
                keys = keys.only_live(m._store)   # rebuilt planes: drop what went
            self.copies = [(g.mesh, m, keys)]
            if isinstance(self.snapshot, StoreState):
                self._before0 = self.snapshot
                self._after = m.capture_state()
            return

        def W(p):
            # Instance prototypes hold LOCAL coords — explode in world.
            return P.map(p) if P is not None else QVector3D(p)

        before = list(m.faces) if m.faces else []
        added: list = []
        for f in g.mesh.faces:
            nf = m.add_face([W(v) for v in f.vertices],
                            [[W(v) for v in h] for h in f.holes] or None)
            added.append(nf)
            # What the face showed inside is what it keeps outside: its own
            # paint, or — a default face — the group's on BOTH sides
            # (issue #47: the back used to fall back to default).
            drawn = effective_attrs(dict(f.attrs or {}), paint)
            if invertible:
                drawn = placed_face_paint(drawn, f, inverse)
            for key, val in drawn.items():
                nf.attrs[key] = dict(val) if isinstance(val, dict) else val
        for e in g.mesh.edges:
            v0, v1 = m.vertex_at(W(e.a)), m.vertex_at(W(e.b))
            if v0 is None or v1 is None or m.find_edge(v0, v1) is None:
                m.add_edge(W(e.a), W(e.b))
        # Edge flags travel back out of the group (the mirror of
        # MakeGroupCommand): an exploded cylinder must stay smooth, its rims
        # keep selecting as whole curves, and a hidden edge stay hidden.
        for e in g.mesh.edges:
            if edge_is_plain(e):
                continue
            v0, v1 = m.vertex_at(W(e.a)), m.vertex_at(W(e.b))
            k = (m.find_edge(v0, v1)
                 if v0 is not None and v1 is not None else None)
            if k is not None:
                stamp_edge_flags(k, edge_flags(e))
        m.resplit_curves()
        # A face that lands on faces already there merges with them, as a
        # pasted one does (#73): two boxes that touch share ONE wall instead
        # of two coincident faces (issue #721).
        _merge_overlapping_planes(scene, before, added)
        self.copies = []

    def undo(self, scene) -> None:
        for c, xform, mesh, material, axes, offset in self._lifted:
            if c in scene.groups:
                scene.groups.remove(c)
            c.xform, c.mesh, c.material, c.axes = xform, mesh, material, axes
            c.explode_offset = offset
        scene.mesh.restore_state(self.snapshot)
        self.group.children = [c for c, *_rest in self._lifted]
        scene.groups.insert(self.index, self.group)
        if self._selection is not None:
            scene.selection = set(self._selection)
        scene.version += 1



def _discard_removed_selection(scene):
    for element in tuple(scene.selection):
        owner = getattr(element, "_m", None)
        store = getattr(owner, "_store", None)
        if store is not None and not store.is_live(element.key):
            scene.selection.discard(element)


def _drop_dead_elements(scene, mesh) -> None:
    """Keep in the selection only the faces and edges still in ``mesh``
    (answered by the store for a store-backed mesh: no view per face)."""
    from core.store_mesh import StoreMesh
    st = mesh._store if type(mesh) is StoreMesh else None
    if st is not None:
        scene.selection = {s for s in ElementSet(scene.selection)
                           if not isinstance(s, (Face, Edge))
                           or st.is_live(s.key)}
        return
    live = set(map(element_identity, mesh.faces))
    live_e = set(map(element_identity, mesh.edges))
    scene.selection = {s for s in ElementSet(scene.selection)
                       if not isinstance(s, (Face, Edge))
                       or s.key in (live if isinstance(s, Face) else live_e)}


def _merge_overlapping_planes(scene, before, added, *, on_rebuild=None,
                              others=False) -> int:
    """Rebuild, plane by plane, where faces just ``added`` to the mesh
    overlap faces it already had (``before``): each region of the plane
    ends up covered by ONE face, as when a face is drawn or pasted there
    (:class:`RebuildPlaneFacesCommand`, scoped to the faces involved).
    Faces that only share an edge or a corner are left alone, and so is
    every plane no older face overlaps. Runs inside a command that already
    holds a snapshot of the mesh, so it takes none. Returns how many planes
    were rebuilt. ``on_rebuild`` lets a caller take its snapshot lazily,
    immediately before the first mutation, only if a rebuild is needed.

    ``others=True``: ``before`` is every face of the mesh but ``added``
    (pass None). A store-backed mesh then finds the face pairs whose boxes
    overlap natively (``MeshStore.plane_merge_candidates``) and the plane
    tests below run on those pairs only, in the same order.
    """
    if not added:
        return 0
    mesh = scene.mesh
    tol = RebuildPlaneFacesCommand._TOL
    found = None
    if others:
        from core.store_parts import merge_candidates
        found = merge_candidates(mesh, added, tol)
        if found is None:
            ids = set(map(element_identity, added))
            before = [f for f in mesh.faces if f.key not in ids]
    if found is None:
        if not before:
            return 0
        live = set(map(element_identity, mesh.faces))
        added = [f for f in added if f.key in live]
        if not added:
            return 0

    def box(f):
        ps = f.vertices
        xs = [p.x() for p in ps]
        ys = [p.y() for p in ps]
        zs = [p.z() for p in ps]
        return (min(xs), min(ys), min(zs)), (max(xs), max(ys), max(zs))

    def unit_normal(f):
        n = f._newell()
        length = n.length()
        return n / length if length > 1e-9 else None

    def overlap(alo, ahi, blo, bhi) -> bool:
        # The _plane_merges test of Paste: the boxes overlap with some area
        # along each axis, or are both flat along it.
        return all(alo[i] < bhi[i] - tol and blo[i] < ahi[i] - tol
                   or abs(ahi[i] - alo[i]) < tol and abs(bhi[i] - blo[i]) < tol
                   for i in range(3))

    if found is not None:
        new_faces, pairs = found
        entries: dict = {}

        def entry(j):
            e = entries.get(j)
            if e is None:
                nf = new_faces[j]
                e = entries[j] = (nf, unit_normal(nf), *box(nf))
            return e

        def candidates():
            for f, hits in pairs:
                yield f, (entry(j) for j in hits)
    else:
        new = []
        glo = [float("inf")] * 3
        ghi = [float("-inf")] * 3
        for f in added:
            n = unit_normal(f)
            if n is None:
                continue
            lo, hi = box(f)
            new.append((f, n, lo, hi))
            glo = [min(a, b) for a, b in zip(glo, lo)]
            ghi = [max(a, b) for a, b in zip(ghi, hi)]
        if not new:
            return 0
        added_ids = set(map(element_identity, added))

        def candidates():
            for f in before:
                if f.key not in live or f.key in added_ids:
                    continue
                lo, hi = box(f)
                if not all(lo[i] <= ghi[i] + tol and glo[i] <= hi[i] + tol
                           for i in range(3)):
                    continue
                yield f, (e for e in new if overlap(lo, hi, e[2], e[3]))

    planes: list = []          # [origin, normal, [old faces], [new faces]]
    for f, near in candidates():
        n = None
        for nf, nn, nlo, nhi in near:
            if nn is None:
                continue
            if n is None:
                n = unit_normal(f)
                if n is None:
                    break
            if abs(QVector3D.dotProduct(n, nn)) < 1.0 - 1e-6:
                continue
            o = nf.vertices[0]
            if any(abs(QVector3D.dotProduct(p - o, nn)) > tol
                   for p in f.vertices):
                continue
            for entry_ in planes:
                if (abs(QVector3D.dotProduct(entry_[1], nn)) > 1.0 - 1e-6
                        and abs(QVector3D.dotProduct(o - entry_[0],
                                                     entry_[1])) < tol):
                    break
            else:
                entry_ = [QVector3D(o), QVector3D(nn), [], []]
                planes.append(entry_)
            if f not in entry_[2]:
                entry_[2].append(f)
            if nf not in entry_[3]:
                entry_[3].append(nf)
    if not planes:
        return 0
    if on_rebuild is not None:
        on_rebuild()
    keep = ElementSet(scene.selection)
    for origin, n, old_faces, new_faces in planes:
        # The new faces on that plane that overlap NO older face stay as
        # they are; the rest rebuild with the older faces they cover.
        cmd = RebuildPlaneFacesCommand(origin, n,
                                       faces=old_faces + new_faces)
        cmd.take_snapshot = False
        cmd.do(scene)
    scene.selection = set(keep)
    _drop_dead_elements(scene, mesh)
    return len(planes)


class RenameGroupCommand(Command):
    """Give a group, component or part a new name (the Parts tray's edit)."""

    doc_undo = True

    def __init__(self, group: Group, name: str) -> None:
        self.group = group
        self.name = name
        self.old: Optional[str] = None

    def do(self, scene) -> None:
        self.old = self.group.name
        self.group.name = self.name
        scene.version += 1

    def undo(self, scene) -> None:
        self.group.name = self.old
        scene.version += 1


class ExplodeViewCommand(Command):
    """Pull a component's parts apart to ``factor`` along ``mode`` — or put
    them back with ``factor=0`` (see :mod:`core.explode`)."""

    #: Matrices, ``exploded`` and ``explode_offset`` are document fields.
    doc_undo = True

    def __init__(self, container: Group, factor: float,
                 mode: str = "outward") -> None:
        self.container = container
        self.factor = factor
        self.mode = mode
        self.before = None

    def do(self, scene) -> None:
        from core import explode
        self.before = explode.snapshot(self.container)
        explode.apply_explode(self.container, self.factor, self.mode)
        scene.version += 1

    def undo(self, scene) -> None:
        from core import explode
        explode.restore(self.container, self.before)
        scene.version += 1


class SplitIntoPiecesCommand(Command):
    """Replace what ``group`` holds with ``pieces`` (from
    :func:`core.pieces.split_into_pieces`): the same geometry, now one child
    group per physical piece. The group stays the object the user placed —
    same matrix, name, layer and paint — so it moves, copies and exports as
    before, and Explode sets the pieces free. Undo puts the old contents
    back untouched (the split only ever built new meshes)."""

    doc_undo = True

    def __init__(self, group: Group, pieces: list) -> None:
        self.group = group
        self.pieces = list(pieces)
        self.before: Optional[tuple] = None
        self._emptied = None
        #: Where the group's faces went, for the change feed: ``[(source
        #: mesh, piece mesh, {"face": [(key, key)]})]`` for the pieces the
        #: store cut (``core.store_parts.copy_part``).
        from core.store_parts import origin_of
        self.copies = [(src, p.mesh, keys) for p in self.pieces
                       for src, keys in [origin_of(p.mesh)] if src is not None]

    def do(self, scene) -> None:
        g = self.group
        self.before = (g.mesh, list(g.children), g.xform, g.exploded)
        if self._emptied is None:
            # The group keeps no geometry of its own: an empty mesh (in the
            # store when the engine is on, like every Mesh()), made once so
            # a redo shows the same mesh and mesh key.
            self._emptied = Mesh()
        g.mesh = self._emptied
        g.adopt(self.pieces)
        # The pieces are cut from the geometry as it stands, so they ARE
        # assembled in their new arrangement: nothing left to take back.
        g.exploded = None
        scene.version += 1

    def undo(self, scene) -> None:
        g = self.group
        g.mesh, g.children, g.xform, g.exploded = self.before
        scene.version += 1


class MoveGroupCommand(GroupTransformCommand):
    """Translate a whole group by ``delta`` (every vertex of its mesh). Because
    the group is isolated, this never drags the rest of the model."""

    keeps_paint = True

    def __init__(self, group: Group, delta: QVector3D) -> None:
        self.group = group
        self.delta = QVector3D(delta)

    def doc_undo(self, scene) -> bool:
        # An instance moves by its matrix (in the document), and undo puts
        # back that exact matrix rather than composing -delta in float32.
        # A classic group moves its vertices: its own undo.
        return getattr(self.group, "xform", None) is not None

    def _shift(self, scene, delta) -> None:
        pose = _member_placement(scene, self.group)
        if pose is not None:
            # Component instance: compose into the transform — O(1), and the
            # shared prototype mesh (siblings!) is never touched.
            from ingetrazo_core import Matrix4 as QMatrix4x4
            t = QMatrix4x4()
            t.translate(delta)
            self.group.xform = _edit_local_transform(scene, self.group, t) * pose
        else:
            from core.group import _remap_uvws
            from ingetrazo_core import Matrix4 as QMatrix4x4
            for v in list(self.group.mesh.vertices):
                self.group.mesh.move_vertex(v, delta)
            t = QMatrix4x4()
            t.translate(delta)
            _remap_uvws(self.group.mesh, t)   # the texture travels along
            from core.group import carry_axes
            carry_axes(self.group, t)         # and its axes (#44)
        scene.version += 1

    def do(self, scene) -> None:
        self._old_axes = self.group.axes
        self._shift(scene, self.delta)

    def undo(self, scene) -> None:
        self._shift(scene, -self.delta)
        if self.group.xform is None:
            self.group.axes = self._old_axes


def placement_is_identity(xform, tol: float = 1e-6) -> bool:
    """True when a placement neither turns nor moves anything (within
    ``tol``) — straightening it would be a silent no-op."""
    if xform is None:
        return True
    from ingetrazo_core import Matrix4 as QMatrix4x4
    ident = QMatrix4x4()
    return all(abs(a - b) <= tol for a, b in zip(xform.data(), ident.data()))


def placement_rotation_deg(xform) -> float:
    """Turn about Z (degrees, counter-clockwise seen from above) of a rigid
    placement; raises ``ValueError`` when the placement tilts or scales —
    there is no north angle for those."""
    import math
    c0, c1, c2 = xform.column(0), xform.column(1), xform.column(2)
    if (abs(c2.x()) > 1e-6 or abs(c2.y()) > 1e-6 or abs(c2.z() - 1.0) > 1e-6
            or abs(c0.z()) > 1e-6 or abs(c1.z()) > 1e-6):
        raise ValueError("the model is tilted (its Z axis is not vertical)")
    sx = math.hypot(c0.x(), c0.y())
    sy = math.hypot(c1.x(), c1.y())
    if abs(sx - 1.0) > 1e-4 or abs(sy - 1.0) > 1e-4:
        raise ValueError("the model is scaled")
    return math.degrees(math.atan2(c0.y(), c0.x()))


class StraightenModelCommand(Command):
    """Put a model that was turned and dragged onto its site back on its
    own axes, and turn the MAP under it instead (a north angle).

    ``group`` is the placed model: a top-level instance whose ``xform`` is a
    turn about Z plus a translation. The command applies the inverse of that
    placement to EVERYTHING in the scene — every group, loose geometry,
    annotations, guides, section planes, geo data, saved views, sheet
    frames and anchored cotas — so the group's placement becomes identity
    and nothing moves relative to anything else; then it re-anchors the
    datum so the base map, terrain and imports stay where they were under
    the model: the anchor becomes the group's origin and the north angle
    grows by the group's turn. Front/right/top views, axis locks and the
    rectangle tool are square to the model again.

    The vertical part of the placement is dropped: the model returns to
    the heights it was drawn at and the map keeps z = 0 (the reference
    plane) — the only place a flat map can be.
    """

    def __init__(self, group: Group) -> None:
        self.group = group
        self.degrees: float = 0.0
        self._placement = None          # the group's xform before do()
        self._old_datum = None
        self._new_datum = None
        self._snaps: dict = {}          # id(mesh) → (before, after)
        self._terrain = None

    def do(self, scene) -> None:
        if self.group not in scene.groups or self.group.xform is None:
            raise ValueError("pick a placed component at the top level")
        datum = getattr(scene, "georef", None)
        if datum is None:
            raise ValueError("the scene has no location yet")
        if self._placement is None:
            from ingetrazo_core import Matrix4 as QMatrix4x4
            from georef.datum import SceneDatum
            self.degrees = placement_rotation_deg(self.group.xform)
            self._placement = QMatrix4x4(self.group.xform)
            self._old_datum = datum
            t = self._placement.column(3)
            lat, lon, _ = datum.local_to_geodetic(QVector3D(t.x(), t.y(), 0.0))
            self._new_datum = SceneDatum(lat, lon, alt=datum.alt,
                                         north=datum.north + self.degrees)
        w, ok = self._placement.inverted()
        if not ok:
            raise ValueError("the placement is not invertible")
        self._apply(scene, w, -self.degrees, redo=True)
        from ingetrazo_core import Matrix4 as QMatrix4x4
        self.group.xform = QMatrix4x4()     # exactly on its axes, no float dust
        scene.georef = self._new_datum
        self._terrain = getattr(scene, "terrain", None)
        scene.terrain = None            # display-only; rebuilt from the datum

    def undo(self, scene) -> None:
        self._apply(scene, self._placement, self.degrees, redo=False)
        from ingetrazo_core import Matrix4 as QMatrix4x4
        self.group.xform = QMatrix4x4(self._placement)
        scene.georef = self._old_datum
        scene.terrain = self._terrain

    # ---- The rigid transform -----------------------------------------------
    def _apply(self, scene, m, turn_deg: float, redo: bool) -> None:
        """Move everything by the rigid ``m`` (``turn_deg`` is its turn about
        Z, for the cameras)."""
        import math

        def pt(v):
            return m.map(QVector3D(*v) if isinstance(v, (list, tuple))
                         else QVector3D(v))

        def vec(v):
            return m.mapVector(QVector3D(v))

        for g in scene.groups:
            if g.xform is not None:          # instances compose — O(1)
                g.xform = m * g.xform
            else:
                self._transform_mesh(g.mesh, m, redo)
        self._transform_mesh(scene.mesh, m, redo)
        for d in scene.dimensions:
            d.a, d.b, d.offset = pt(d.a), pt(d.b), vec(d.offset)
        for t in scene.text_labels:
            t.anchor, t.offset = pt(t.anchor), vec(t.offset)
        for gd in scene.guides:
            gd.point = pt(gd.point)
            if getattr(gd, "direction", None) is not None:
                gd.direction = vec(gd.direction)
        for sp in scene.section_planes:
            sp.point, sp.normal = pt(sp.point), vec(sp.normal)
        for im in scene.image_planes:
            im.origin, im.u, im.v = pt(im.origin), vec(im.u), vec(im.v)
        for gp in scene.geo_paths:
            gp.points = [pt(q) for q in gp.points]
            if getattr(gp, "_surface_tris", None) is not None:
                gp._surface_tris = None
        for gpt in scene.geo_points:
            gpt.position = pt(gpt.position)
        turn = math.radians(turn_deg)
        for sv in scene.saved_views:
            q = pt(sv.target)
            sv.target = (q.x(), q.y(), q.z())
            sv.yaw = sv.yaw + turn
        for comp in scene.compositions:
            for it in comp.all_items():
                for attr in ("a_world", "b_world"):
                    w = getattr(it, attr, None)
                    if w:
                        q = pt(w)
                        setattr(it, attr, [q.x(), q.y(), q.z()])
                ct = getattr(it, "cam_target", None)
                if ct:
                    q = pt(ct)
                    it.cam_target = [q.x(), q.y(), q.z()]
                    if getattr(it, "cam_yaw", None) is not None:
                        it.cam_yaw = it.cam_yaw + turn
        scene.version += 1

    def _transform_mesh(self, mesh, m, redo: bool) -> None:
        """Classic (world-coordinate) meshes: move the vertices the first
        time, snapshot both states, and replay the snapshots after that so
        undo/redo are exact."""
        from core.group import _remap_uvws
        if not mesh.vertices:
            return
        snap = self._snaps.get(id(mesh))
        if snap is not None:
            mesh.restore_state(snap[1] if redo else snap[0])
            return
        before = mesh.capture_state()
        for v in list(mesh.vertices):
            mesh.move_vertex(v, m.map(v.position) - v.position)
        _remap_uvws(mesh, m)
        self._snaps[id(mesh)] = (before, mesh.capture_state())


def group_owner_list(scene, group):
    """The list ``group`` lives in: ``scene.groups`` at the root, or the
    ``children`` of the container that owns it, however deep. ``None`` when
    the group is not in the scene at all."""
    if group in scene.groups:
        return scene.groups
    from core.group import iter_placements
    for top in scene.groups:
        for pg, _m in iter_placements(top):
            kids = getattr(pg, "children", None)
            if kids and group in kids:
                return kids
    return None


class DeleteGroupCommand(Command):
    """Remove a whole group (and its geometry) from the scene; undo restores it
    at its original position in the list.

    The list may be a container's ``children``: inside a component, Delete
    on one of its parts used to look for it in ``scene.groups`` and fail —
    rolled back, nothing on screen, «quiero eliminar, tampoco puedo»
    (Marco, 2026-09-11, a fountain inside an imported component)."""

    keeps_groups = True

    doc_undo = True

    def __init__(self, group: Group) -> None:
        self.group = group
        self.index: Optional[int] = None
        self._owner: Optional[list] = None

    def doc_after(self, scene, phase) -> None:
        if phase == "redo":
            scene.selection.discard(self.group)
            scene.mesh.purge_debris()
        scene.version += 1

    def do(self, scene) -> None:
        owner = group_owner_list(scene, self.group)
        if owner is None:
            raise ValueError("group is not in the scene")
        self._owner = owner
        self.index = owner.index(self.group)
        owner.remove(self.group)
        scene.selection.discard(self.group)
        # Deleting a group is the moment the user expects the loose mesh to
        # be the loose mesh again: whatever left a vertex with nothing on it
        # — a grouping, an operation that removed geometry without sweeping —
        # no edge draws it, no face holds it, and the position registry still
        # hands it to whatever is drawn there next. It is not part of the
        # group being removed (its own mesh travels with it, undo restores it
        # whole), so sweeping it here cannot lose anything the user can see
        # or undo back to.
        scene.mesh.purge_debris()
        scene.version += 1

    def undo(self, scene) -> None:
        owner = self._owner if self._owner is not None else scene.groups
        owner.insert(min(self.index or 0, len(owner)), self.group)
        scene.version += 1


class HealOverlapsCommand(MeshCommand):
    """Remove redundant 'mother' faces left overlapping their own subdivisions
    (a draw/delete can leave the big enclosing face on top). Snapshot undo."""

    def __init__(self) -> None:
        self.snapshot: Optional[dict] = None
        self.healed = 0

    def do(self, scene) -> None:
        self.snapshot = self._reference_snapshot(scene.mesh)
        # partial defaults to auto: the aggressive pass runs only on a flat plan.
        # Orientation is not this command's business (Orient Faces is).
        removed = heal_overlapping_faces(
            scene.mesh, footprint=winding_footprint(scene.mesh))
        self.healed = len(removed)
        for f in removed:
            scene.selection.discard(f)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.snapshot is not None:
            scene.mesh.restore_state(self.snapshot)
            scene.version += 1


_IN_TRI_SLACK = 1e-7


def _point_in_tri(p: QVector3D, t0: QVector3D, t1: QVector3D,
                  t2: QVector3D) -> bool:
    """Coplanar point-in-triangle via consistent cross-product orientation.

    Measured as distances (``_IN_TRI_SLACK`` metres outside an edge still
    counts), and a sliver -- three nearly collinear corners, as a face whose
    side was split on a straight border triangulates -- contains nothing.
    The raw ``c·n > -1e-9`` test scaled with the triangle's area squared: a
    sliver's tiny normal let every point through, and a 3 mm disc on the
    border of a courtyard "covered" the 548 m² courtyard 8 m away.
    """
    n = QVector3D.crossProduct(t1 - t0, t2 - t0)
    nl = n.length()
    longest = max((t1 - t0).lengthSquared(), (t2 - t1).lengthSquared(),
                  (t0 - t2).lengthSquared())
    if nl < 1e-12 or nl < 1e-9 * longest:
        return False
    for a, b in ((t0, t1), (t1, t2), (t2, t0)):
        c = QVector3D.crossProduct(b - a, p - a)
        side = (b - a).length()
        if side > 0 and QVector3D.dotProduct(c, n) / (nl * side) \
                < -_IN_TRI_SLACK:
            return False
    return True


def _segment_grid(segments):
    """A lookup ``near(p)`` giving, in ascending order, the indices of the
    ``segments`` (``(QVector3D, QVector3D)`` pairs) that ``p`` can lie on
    under ``_point_on_seg_incl``.

    The rebuild re-stamped curve flags by testing every new edge against
    every flagged one: 167 M tests, 218 s, on an exploded DWG plan (#491).
    Each segment goes in the cells of its box padded past the test's reach
    (its ``t`` slack scales with length); the cell size grows until the boxes
    cover at most a few cells each, so a long wall cannot flood the grid."""
    import math
    if not segments:
        return lambda p: ()
    boxes = []
    for a, b in segments:
        pad = 4e-4 * (1.0 + (b - a).length()) + 4e-4
        boxes.append((min(a.x(), b.x()) - pad, min(a.y(), b.y()) - pad,
                      min(a.z(), b.z()) - pad, max(a.x(), b.x()) + pad,
                      max(a.y(), b.y()) + pad, max(a.z(), b.z()) + pad))
    ext = sorted(max(bx[3] - bx[0], bx[4] - bx[1], bx[5] - bx[2])
                 for bx in boxes)
    size = ext[len(ext) // 2]

    def span(lo, hi):
        return math.floor(hi / size) - math.floor(lo / size) + 1

    while sum(span(bx[0], bx[3]) * span(bx[1], bx[4]) * span(bx[2], bx[5])
              for bx in boxes) > 8 * len(boxes):
        size *= 2.0
    cells: dict = {}
    for i, (x0, y0, z0, x1, y1, z1) in enumerate(boxes):
        for cx in range(math.floor(x0 / size), math.floor(x1 / size) + 1):
            for cy in range(math.floor(y0 / size), math.floor(y1 / size) + 1):
                for cz in range(math.floor(z0 / size),
                                math.floor(z1 / size) + 1):
                    cells.setdefault((cx, cy, cz), []).append(i)

    def near(p):
        return cells.get((math.floor(p.x() / size), math.floor(p.y() / size),
                          math.floor(p.z() / size)), ())
    return near


def _welded_loop(mesh, loop):
    """``loop`` with the corners the mesh welds into one merged, or ``None``
    if fewer than three are left -- see the weld note in
    :meth:`RebuildPlanarFacesCommand.do`."""
    out = []
    for p in loop:
        v = mesh.vertex(p)
        if not out or v != out[-1]:
            out.append(v)
    if len(out) > 1 and out[0] == out[-1]:
        out.pop()
    if len(out) < 3:
        return None
    return [v.position for v in out]


class RebuildPlanarFacesCommand(MeshCommand):
    """Rebuild the minimal faces of a flat drawing from its edge graph (a planar
    arrangement) — the deterministic, from-scratch replacement for the heuristic
    heal. Splits every crossing/overlap, drops dangling spurs, and recomputes the
    rooms and their holes exactly. Only runs on a single-plane mesh; 3D is left
    untouched (coplanar nesting there is legitimate). Snapshot undo."""

    def __init__(self) -> None:
        self.snapshot: Optional[dict] = None
        self.rebuilt = 0
        self.flat = True

    def do(self, scene) -> None:
        from core.arrangement import coplanar_plane, planar_rebuild
        from core.topology import _point_on_seg_incl

        self.snapshot = scene.mesh.capture_state()  # source paint for rebuilt faces
        mesh = scene.mesh
        footprint = winding_footprint(mesh)
        if not mesh.edges:
            self.flat = False
            return
        # Every edge endpoint must share one plane; 3D models are left alone.
        from core.store_mesh import StoreMesh, FaceView, _hypot
        if type(mesh) is StoreMesh and (hypot := _hypot()) is not None:
            values = mesh._store.drawing_plane([], hypot)
            plane = None if values is None else tuple(QVector3D(*p) for p in values)
        else:
            plane = coplanar_plane(v.position for v in mesh.vertices)
        if plane is None:
            self.flat = False
            return
        origin, normal = plane
        if mesh.faces:  # prefer a real face's normal for a stable orientation
            normal = mesh.faces[0].normal()
            origin = mesh.faces[0].vertices[0]
        native = type(mesh) is StoreMesh and hypot is not None
        if native:
            from core.store_mesh import EdgeView
            segments, flagged_keys = mesh._store.drawing_plane_edges(
                origin.toTuple(), normal.toTuple(), hypot, all=True)
            segments = [(QVector3D(*a), QVector3D(*b)) for a, b in segments]
            flagged_edges = [EdgeView(mesh, k) for k in flagged_keys]
            flagged = [(e.a, e.b, edge_flags(e)) for e in flagged_edges]
            old_attrs = []
        else:
            segments = [(e.v0.position, e.v1.position) for e in mesh.edges]
            flagged = [(QVector3D(e.a), QVector3D(e.b), edge_flags(e))
                       for e in mesh.edges if not edge_is_plain(e)]
            old_attrs = [([tuple(t) for t in f.triangulate()], dict(f.attrs))
                         for f in mesh.faces if f.attrs]
        edges, faces = planar_rebuild(segments, origin, normal)

        near_flagged = _segment_grid([(fa, fb) for fa, fb, _ in flagged])

        scene.selection.clear()
        if type(mesh) is StoreMesh:
            mesh._store.drawing_clear()
            mesh._attr_own.clear()
            mesh._bump()
        else:
            mesh.clear()
        for a, b in edges:
            if mesh.vertex(a) == mesh.vertex(b):
                # Shorter than the weld: the arrangement keys points on a
                # 0.1 mm grid, so two ends a hair apart across a cell line
                # stay two -- the mesh welds them into one. Raising here
                # rolled the whole draw back on an exploded DWG plan (#491).
                continue
            e = mesh.add_edge(a, b)
            for i in near_flagged(e.a):
                fa, fb, flags = flagged[i]
                if _point_on_seg_incl(e.a, fa, fb) and _point_on_seg_incl(e.b, fa, fb):
                    stamp_edge_flags(e, flags)
                    break
        for outer, holes in faces:
            outer = _welded_loop(mesh, outer)
            if outer is None:
                continue
            holes = [h for h in (_welded_loop(mesh, h) for h in holes) if h]
            f = mesh.add_face(outer, holes or None)
            if f is None or not old_attrs:
                continue
            tris = f.triangulate()
            if not tris:
                continue
            probe = (tris[0][0] + tris[0][1] + tris[0][2]) / 3.0
            for old_tris, attrs in old_attrs:
                if any(_point_in_tri(probe, t0, t1, t2)
                       for t0, t1, t2 in old_tris):
                    f.attrs.update(attrs)
                    break
        if native and (self.snapshot.own or self.snapshot.table):
            own = {k: bool(attrs) for k, attrs in self.snapshot.own.items()}
            rows = [i + 1 for i, attrs in enumerate(self.snapshot.table) if attrs]
            owners = mesh._store.drawing_paint_owners(self.snapshot.snap, own, rows, hypot)
            for face, owner, row in owners:
                FaceView(mesh, face).attrs.update(self.snapshot._face_attrs(owner, row))
        # Every region comes out facing ``normal`` (the first face's); each
        # takes back the way the face it lies in was facing — a lone arc
        # drawn on the plane turned every reversed face back otherwise.
        if footprint is not None:
            orient_coplanar_faces(mesh, footprint)
        mesh.resplit_curves()
        if self._journal_geometry:
            self.snapshot = None
        self.rebuilt = len(faces)
        scene.version += 1

    def undo(self, scene) -> None:
        if self.snapshot is not None:
            scene.mesh.restore_state(self.snapshot)
            scene.version += 1


def _cyclic(ring, both_ways=False) -> tuple:
    """``ring``'s position keys from its smallest one: equal for two rings
    that are the same cycle (``both_ways``: in either direction)."""
    keys = [_key(p) for p in ring]
    if not keys:
        return ()
    def from_min(ks):
        i = ks.index(min(ks))
        return tuple(ks[i:] + ks[:i])
    if not both_ways:
        return from_min(keys)
    return min(from_min(keys), from_min(keys[::-1]))


def _region_is_face(face, outer, holes) -> bool:
    """Whether a rebuilt region (``outer`` wound like ``face``, ``holes``)
    is ``face`` itself: the same outer ring, corner for corner in the same
    order, and the same openings. The rebuild then keeps the face (its
    key, attrs and edges) instead of removing it and adding an equal one:
    a stroke drawn beside a plate leaves the plate's face alone."""
    if getattr(face, "interior", False):
        return False                    # a re-added face would lose the flag
    if _cyclic(face.vertices) != _cyclic(outer):
        return False
    have = sorted(_cyclic(h, True) for h in (face.holes or ()))
    return have == sorted(_cyclic(h, True) for h in (holes or ()))


class RebuildPlaneFacesCommand(MeshCommand):
    """Rebuild the faces of ONE plane of a (possibly 3D) mesh from that plane's
    edge subgraph — the per-plane cousin of :class:`RebuildPlanarFacesCommand`.

    Needed because the whole-mesh flat gate goes dark as soon as ANY 3D
    geometry exists in the scene: two circles drawn on the ground next to a
    solid stacked as full overlapping discs instead of splitting into three
    areas. This command recomputes the arrangement of just the drawing plane
    and leaves the rest of the model untouched.

    Semantics: a minimal region keeps a face only if its interior was covered
    by an existing on-plane face (the freshly drawn loop's face, added by the
    tool before this command, provides coverage for the new regions). Uncovered
    regions stay empty — no resurrecting faces the user deleted. Winding and
    attrs inherit from the covering face. Reused edges and newly split pieces
    retain the source edges' flags, including soft/curve contours. Snapshot undo."""

    _TOL = 1e-4
    #: False when run inside a command that already snapshotted the mesh
    #: (Explode's merge of overlapping faces, #721).
    take_snapshot = True

    def __init__(self, origin: QVector3D, normal: QVector3D,
                 faces: Optional[Iterable] = None) -> None:
        self.origin = QVector3D(origin)
        # Normalised by hand: QVector3D.normalized() nulls anything shorter
        # than 1e-5, and a zero normal here means "rebuild the plane of
        # everything", which empties the mesh. See core/mesh.py Face.normal.
        n = QVector3D(normal)
        length = n.length()
        self.normal = n / length if length > 1e-9 else QVector3D()
        # Given faces: rebuild only them, from their own remaining edges --
        # the rest of the plane is not this edit's business (#356).
        self.faces = None if faces is None else list(faces)
        self.snapshot: Optional[dict] = None
        self.rebuilt = 0

    def _on_plane(self, p: QVector3D) -> bool:
        return abs(QVector3D.dotProduct(p - self.origin, self.normal)) < self._TOL

    def _near_plane(self, mesh, kind: str) -> list:
        """The edges (both ends) or faces (outer corners) of ``mesh`` near
        enough the plane for :meth:`_on_plane` to accept, in list order —
        a superset from the mesh scan, every one without it. The caller's
        own test decides."""
        from core import scan_index
        sc = scan_index.scan(mesh)
        if sc is None:
            return mesh.edges if kind == "edge" else mesh.faces
        o = (self.origin.x(), self.origin.y(), self.origin.z())
        n = (self.normal.x(), self.normal.y(), self.normal.z())
        if kind == "edge":
            return sc.edges(sc.native.edges_on_plane(o, n, self._TOL, False))
        return sc.faces(sc.native.faces_on_plane(o, n, self._TOL))

    def do(self, scene) -> None:
        from core.arrangement import planar_rebuild
        from core.triangulate import triangulate
        from core.topology import _point_on_seg_incl

        self.snapshot = (scene.mesh.capture_state() if self.take_snapshot
                         else None)
        if self.normal.lengthSquared() < 0.5:
            # No plane, nothing to rebuild. _on_plane would otherwise say
            # yes to EVERY point and the arrangement would run over the
            # whole mesh — which is how a 3 mm² sliver erased 447 faces.
            return
        mesh = scene.mesh
        from core.store_mesh import StoreMesh, FaceView, EdgeView, _hypot
        if self.faces is None and type(mesh) is StoreMesh and (hypot := _hypot()) is not None:
            o, n = self.origin.toTuple(), self.normal.toTuple()
            segments, flags = mesh._store.drawing_plane_edges(o, n, hypot)
            if not segments:
                return
            flagged_edges = [EdgeView(mesh, k) for k in flags]
            flagged = [(e.a, e.b, edge_flags(e)) for e in flagged_edges]
            _, regions = planar_rebuild([(QVector3D(*a), QVector3D(*b)) for a, b in segments],
                                        self.origin, self.normal)
            rings = [[[p.toTuple() for p in ring] for ring in (outer, *(holes or ()))]
                     for outer, holes in regions]
            keys, owners = mesh._store.drawing_coverage(o, n, rings, hypot)
            from core.triangulate import _newell
            keep, unchanged = [], set()
            for i, owner, normal in owners:
                outer, holes = regions[i]
                if QVector3D.dotProduct(_newell(outer), QVector3D(*normal)) < 0:
                    outer = list(reversed(outer))
                    holes = [list(reversed(h)) for h in (holes or [])]
                face = FaceView(mesh, owner)
                if owner not in unchanged and _region_is_face(face, outer, holes):
                    unchanged.add(owner)        # comes out as it was: kept
                    continue
                keep.append((outer, holes or None, dict(face.attrs)))
            keys = [k for k in keys if k not in unchanged]
            mesh._store.remove_drawing_faces(keys)
            mesh._dirty(len(keys))
            self._restore_regions(scene, keep, flagged, len(unchanged))
            return
        if self.faces is not None:
            live = set(map(element_identity, mesh.faces))
            plane_faces = [f for f in self.faces if f.key in live]
            plane_edges = []
            for f in plane_faces:
                for loop in (f.loop, *f.hole_loops):
                    for a, b in zip(loop, loop[1:] + loop[:1]):
                        e = mesh.find_edge(a, b)
                        if e is not None and e not in plane_edges:
                            plane_edges.append(e)
        else:
            plane_edges = [e for e in self._near_plane(mesh, "edge")
                           if self._on_plane(e.a) and self._on_plane(e.b)]
        if not plane_edges:
            if self.faces is not None:
                for f in plane_faces:
                    mesh.remove_face(f)
            return
        if self.faces is None:
            plane_faces = [
                f for f in self._near_plane(mesh, "face")
                if all(self._on_plane(v.position) for v in f.loop)
                and all(self._on_plane(v.position)
                        for h in f.hole_loops for v in h)
            ]
        old = [([tuple(t) for t in f.triangulate()], f.normal(), dict(f.attrs))
               for f in plane_faces]
        segments = [(QVector3D(e.a), QVector3D(e.b)) for e in plane_edges]
        # Copy/Explode can arrive without the drawing planner pre-splitting
        # crossings. Carry flags onto the arrangement's new sub-edges too.
        flagged = [(QVector3D(e.a), QVector3D(e.b), edge_flags(e))
                   for e in plane_edges if not edge_is_plain(e)]
        _, regions = planar_rebuild(segments, self.origin, self.normal)

        def ring(loop):
            return frozenset(_key(p) for p in loop)

        # The openings the plane's faces already have, and the outlines a
        # face was drawn exactly along. A region that is one of those
        # openings stays open unless a face traces it (capping a hole on
        # purpose) or traces the whole face around it (redrawing that face
        # fills it, #538): a rectangle or an offset drawn AROUND a through
        # hole covered it and the fresh face filled the hole (tutorial hunt
        # 2, A15).
        outlines = Counter(ring(f.vertices) for f in plane_faces)
        openings: dict = {}
        for f in plane_faces:
            for h in f.holes:
                openings.setdefault(ring(h), set()).add(ring(f.vertices))

        def stays_open(outer) -> bool:
            owners = openings.get(ring(outer))
            if owners is None or outlines[ring(outer)]:
                return False
            return not any(outlines[o] > 1 for o in owners)

        def covering(outer, holes):
            tris = triangulate(outer, holes, self.normal)
            if not tris:
                return None
            # The centroid of the largest triangle, not the first: the first
            # of a circle's fan is a sliver at the rim, and on a centimetre
            # part its centroid sat within rounding of the face around the
            # hole -- the hole read as covered and was capped (A15).
            big = max(tris, key=lambda t: QVector3D.crossProduct(
                t[1] - t[0], t[2] - t[0]).lengthSquared())
            probe = (big[0] + big[1] + big[2]) / 3.0
            for face, (old_tris, nrm, attrs) in zip(plane_faces, old):
                if any(_point_in_tri(probe, t0, t1, t2)
                       for t0, t1, t2 in old_tris):
                    return nrm, attrs, face
            return None

        keep, unchanged = [], set()
        for outer, holes in regions:
            if not holes and stays_open(outer):
                continue                      # an existing opening
            cov = covering(outer, holes or [])
            if cov is None:
                continue                      # uncovered region: stays empty
            nrm, attrs, owner = cov
            # Arrangement emits CCW around self.normal; match the old face.
            from core.triangulate import _newell
            if QVector3D.dotProduct(_newell(outer), nrm) < 0:
                outer = list(reversed(outer))
                holes = [list(reversed(h)) for h in (holes or [])]
            if owner.key not in unchanged and _region_is_face(owner, outer, holes):
                unchanged.add(owner.key)        # comes out as it was: kept
                continue
            keep.append((outer, holes or None, attrs))
        for f in plane_faces:
            if f.key not in unchanged:
                mesh.remove_face(f)
        self._restore_regions(scene, keep, flagged, len(unchanged))

    def _restore_regions(self, scene, keep, flagged, unchanged=0):
        from core.topology import _point_on_seg_incl
        mesh = scene.mesh
        near_flagged = _segment_grid([(a, b) for a, b, _flags in flagged])
        restored = set()
        for outer, holes, attrs in keep:
            f = mesh.add_face(outer, holes)
            if attrs:
                f.attrs.update(attrs)
            if flagged:
                for loop in (f.loop, *f.hole_loops):
                    for a, b in zip(loop, loop[1:] + loop[:1]):
                        edge = mesh.find_edge(a, b)
                        if edge is None or edge in restored:
                            continue
                        restored.add(edge)
                        for i in near_flagged(edge.a):
                            fa, fb, flags = flagged[i]
                            if (_point_on_seg_incl(edge.a, fa, fb)
                                    and _point_on_seg_incl(edge.b, fa, fb)):
                                stamp_edge_flags(edge, flags)
                                break
        # AddFaceCommand re-creates a polygon's full-length side even when the
        # planner already split it at a crossing — a border-0 collinear
        # duplicate that fragments the curve contours at resplit (degree-3
        # vertices). The regions above were re-added from the fully noded
        # arrangement, so the duplicates now bound nothing: prune them.
        from core.topology import prune_collinear_orphan_edges

        prune_collinear_orphan_edges(mesh)
        mesh.resplit_curves()
        self.rebuilt = len(keep) + unchanged
        scene.selection.clear()
        scene.version += 1

    def undo(self, scene) -> None:
        if self.snapshot is not None:
            scene.mesh.restore_state(self.snapshot)
            scene.version += 1


class CompoundCommand(Command):
    """A list of commands executed and reverted as one atomic step."""

    journal_parts = False

    def __init__(self, commands: Iterable[Command]) -> None:
        self.commands: list[Command] = list(commands)

    @property
    def keeps_paint(self) -> bool:
        return all(getattr(c, "keeps_paint", False) for c in self.commands)

    @property
    def keeps_groups(self) -> bool:
        return all(getattr(c, "keeps_paint", False)
                   or getattr(c, "keeps_groups", False)
                   for c in self.commands)

    @property
    def keeps_rows(self) -> bool:
        return all(getattr(c, "keeps_paint", False)
                   or getattr(c, "keeps_rows", False)
                   for c in self.commands)

    def do(self, scene) -> None:
        done = []
        try:
            for cmd in self.commands:
                cmd.do(scene)
                done.append(cmd)
        except CommandRejected:
            # One part declined: the rest goes back too (a group moved
            # with the vertices lives outside the mesh snapshot).
            if not self._journal_geometry:
                for cmd in reversed(done):
                    cmd.undo(scene)
            raise

    def undo(self, scene) -> None:
        for cmd in reversed(self.commands):
            cmd.undo(scene)

    def doc_undo(self, scene) -> bool:
        """By its transaction when every part would be."""
        return bool(self.commands) and all(
            _uses_doc_undo(c, scene) for c in self.commands)

    def doc_after(self, scene, phase) -> None:
        parts = reversed(self.commands) if phase == "undo" else self.commands
        for cmd in parts:
            cmd.doc_after(scene, phase)


class OccurrenceCommands(CompoundCommand):
    """Replay committed commands in their original occurrence coordinate frames."""

    @staticmethod
    def group_start(history, session):
        if history._native is not None:
            return history._native.group_start(session)
        stack = history.undo_stack
        start = len(stack)
        while start and session in getattr(stack[start - 1], "_occurrences", ()):
            start -= 1
        return start

    @staticmethod
    def fold(history, session):
        if history._native is not None:
            return history._native.fold_occurrence(session, OccurrenceCommands)
        stack = history.undo_stack
        start = OccurrenceCommands.group_start(history, session)
        if len(stack) - start > 1:
            step = OccurrenceCommands(stack[start:])
            step._occurrences = stack[start]._occurrences
            stack[start:] = [step]

    @staticmethod
    def _in_frame(scene, cmd, action, *args):
        from contextlib import nullcontext
        from core.coordinate_frame import mesh_frame
        active = scene.mesh
        try:
            scene.mesh = getattr(cmd, "_history_mesh", active)
            frame = getattr(cmd, "_tool_frame", None)
            with mesh_frame(scene.mesh, frame[0], matrix=frame[1]) if frame else nullcontext():
                action(*args)
        finally:
            scene.mesh = active

    def doc_after(self, scene, phase):
        _doc.NATIVE.DocumentHistory.replay_group(self, scene, phase == "undo", True)

    def _replay(self, scene, undo):
        if _doc.ENABLED:
            return _doc.NATIVE.DocumentHistory.replay_group(self, scene, undo, False)
        for cmd in reversed(self.commands) if undo else self.commands:
            self._in_frame(scene, cmd, cmd.undo if undo else cmd.do, scene)

    def do(self, scene):
        self._replay(scene, False)

    def undo(self, scene):
        self._replay(scene, True)


class ChangeAxesCommand(MeshCommand):
    """Change Axes (issue #44): give ``group`` a new frame —
    origin, red, green, blue as the world matrix ``frame`` — while nothing
    moves in the world.

    A classic group only swaps its ``axes`` (its mesh is in world
    coordinates already). A component instance re-expresses its SHARED
    definition in the new axes: the definition takes ``D = frame⁻¹ · xform``,
    this instance's placement becomes ``frame``, and every other instance of
    the definition takes ``D⁻¹`` on the right — so all of them now carry the
    new axes and none of them moves («changing one updates the
    others»). Undo runs the inverse."""

    def doc_undo(self, scene) -> bool:
        # A classic group swaps its axes (a document field); an instance
        # re-expresses its definition's mesh: its own undo.
        return self._d is None

    def __init__(self, group, frame) -> None:
        from ingetrazo_core import Matrix4 as QMatrix4x4
        self.group = group
        self.frame = QMatrix4x4(frame)
        self._old_axes = None
        self._d = None
        self._instances: list = []
        self._inst_axes: list = []
        self._inst_xforms: list = []

    def geometry_meshes(self, scene):
        return (self.group.mesh,) if self.group.xform is not None else ()

    def _siblings(self, scene) -> list:
        from core.group import iter_placements
        proto = self.group.mesh
        out, seen = [], set()
        for top in scene.groups:
            for g, _m in iter_placements(top):
                if (g.mesh is proto and getattr(g, "xform", None) is not None
                        and id(g) not in seen):
                    seen.add(id(g))
                    out.append(g)
        if id(self.group) not in seen:
            out.append(self.group)
        return out

    @staticmethod
    def _reexpress(group, d) -> None:
        """Map the definition's geometry (and nested placements) by ``d``."""
        from core.group import transformed_mesh
        proto = group.mesh
        moved = transformed_mesh(proto, d)
        proto.restore_state(moved.capture_state())
        for child in getattr(group, "children", None) or []:
            if child.xform is not None:
                child.xform = d * child.xform

    def do(self, scene) -> None:
        g = self.group
        if getattr(g, "xform", None) is None:
            self._old_axes = g.axes
            g.axes = self._copy(self.frame)
            scene.version += 1
            return
        inv, ok = self.frame.inverted()
        if not ok:
            return
        self._d = inv * g.xform
        d_inv, ok2 = self._d.inverted()
        if not ok2:
            self._d = None
            return
        self._instances = self._siblings(scene)
        self._reexpress(g, self._d)
        # An instance's frame is ``xform · axes``: axes it carried from its
        # classic-group days would turn the new frame again (#484), so the
        # re-expressed definition's own axes are the instances' now.
        self._inst_axes = [inst.axes for inst in self._instances]
        self._inst_xforms = [self._copy(inst.xform) for inst in self._instances]
        for inst in self._instances:
            inst.xform = inst.xform * d_inv
            inst.axes = None
        scene.version += 1

    def undo(self, scene) -> None:
        g = self.group
        if getattr(g, "xform", None) is None and self._d is None:
            g.axes = self._old_axes
            scene.version += 1
            return
        if self._d is None:
            return
        d_inv, _ok = self._d.inverted()
        self._reexpress(g, d_inv)
        # The placements as they were, not ``xform · d`` again: that product
        # came back 6e-8 off, and undo should leave exactly what was there.
        for inst, axes, xform in zip(self._instances, self._inst_axes,
                                     self._inst_xforms):
            inst.xform = self._copy(xform)
            inst.axes = axes
        scene.version += 1

    @staticmethod
    def _copy(m):
        from ingetrazo_core import Matrix4 as QMatrix4x4
        return QMatrix4x4(m)


class SetStructureNodeCommand(Command):
    """Membership belongs to a placement, including ones without an IFC record."""
    doc_undo = True     # ``Group.ext`` is a document value (slice 2)

    def __init__(self, targets, key):
        self.targets = list(targets)
        self.key = key

    def do(self, scene):
        from copy import deepcopy
        self.before = []
        for group in self.targets:
            ext = dict(group.ext or {})
            self.before.append((group.ext is None, 'structure' in ext, deepcopy(ext.get('structure'))))
            if self.key is None:
                ext.pop('structure', None)
            else:
                ext['structure'] = {'node': self.key}
            group.ext = ext or None
        scene.version += 1

    def undo(self, scene):
        from copy import deepcopy
        for group, (was_none, had_key, value) in zip(self.targets, self.before):
            ext = dict(group.ext or {})
            if had_key:
                ext['structure'] = deepcopy(value)
            else:
                ext.pop('structure', None)
            group.ext = None if was_none and not ext else ext
        scene.version += 1


class SetRecordCommand(Command):
    """Top-level patch of group records; None patch removes the record.

    Nested values replace whole fields. GUIDs cannot be changed by a patch.
    Snapshots are detached and retained across redo, including new identities.
    """
    # History currently treats keeps_paint as also keeping BIM tags.
    keeps_paint = False
    #: Records are document values (``Group.ifc``, slice 2).
    doc_undo = True

    def doc_after(self, scene, phase) -> None:
        meshes = {id(scene.mesh): scene.mesh}
        for group in self.targets:
            meshes[id(group.mesh)] = group.mesh
        for mesh in meshes.values():
            mesh.bump_revision()
        scene.version += 1

    def __init__(self, targets, patch, *, normalize_values=True):
        self.normalize_values = normalize_values
        from copy import deepcopy
        self.targets = list(targets)
        self.patch = deepcopy(patch)
        self._before = None
        self._after = None

    def _write(self, scene, records):
        from copy import deepcopy
        meshes = {id(scene.mesh): scene.mesh}
        for group, record in zip(self.targets, records):
            group.ifc = deepcopy(record)
            meshes[id(group.mesh)] = group.mesh
        for mesh in meshes.values():
            mesh.bump_revision()
        scene.version += 1

    def do(self, scene):
        from copy import deepcopy
        from core.ifc_identity import ensure_guid
        from core.record import normalize
        if self._before is None:
            self._before = [deepcopy(g.ifc) for g in self.targets]
            self._after = []
            for old in self._before:
                if self.patch is None:
                    self._after.append(None)
                    continue
                new = {**deepcopy(old or {}), **deepcopy(self.patch)}
                if old and old.get('guid'):
                    new['guid'] = old['guid']
                else:
                    new.pop('guid', None)
                    ensure_guid(new)
                self._after.append(normalize(new) if self.normalize_values else new)
        self._write(scene, self._after)

    def undo(self, scene):
        self._write(scene, self._before)


class TagGroupsCommand(SetRecordCommand):
    def __init__(self, targets, ifc_class, name=''):
        super().__init__(targets, {'class': ifc_class, 'name': name})


class UntagGroupsCommand(SetRecordCommand):
    def __init__(self, targets):
        super().__init__(targets, None)


class SetRegistryCommand(SetPluginDataCommand):
    def __init__(self, value, *, refiled_uids=()):
        self.refiled_uids = frozenset(refiled_uids)
        from core.bim_registry import validate
        errors = validate(value)
        if errors:
            raise ValueError('; '.join(errors))
        super().__init__('bim_registry', value)

    def do(self, scene):
        from core import project, bim_registry
        stamp = getattr(scene, '_project_mtime_ns', None)
        current = bim_registry.get(scene)
        removed = (current.get('structure', {}).get('nodes', {}).keys()
                   - self.value.get('structure', {}).get('nodes', {}).keys())
        if removed:
            from core.structure import _placements, node_of
            for group, source, _ in _placements(scene):
                if not source and node_of(group) in removed and group.uid not in self.refiled_uids:
                    raise CommandRejected('Cannot delete occupied node: ' + node_of(group))
        if not hasattr(self, '_project_before'):
            self._project_before = current
        if stamp is not None:
            scene._project_mtime_ns = stamp
        if project.write_registry(scene, self.value):
            scene.version += 1
        else:
            super().do(scene)
        scene.mesh.bump_revision()

    def undo(self, scene):
        from core import project
        if project.write_registry(scene, self._project_before):
            scene.version += 1
        else:
            super().undo(scene)
        scene.mesh.bump_revision()

    def doc_undo(self, scene) -> bool:
        # A project's registry lives in its project file, not the document.
        from core import project
        return project.path_for(scene) is None

    def doc_after(self, scene, phase) -> None:
        scene.mesh.bump_revision()
        scene.version += 1


class SetFaceTagsCommand(Command):
    """Legacy loose-face tags, with stable identities across redo."""
    # History currently treats keeps_paint as also keeping BIM tags.
    keeps_paint = False

    def __init__(self, faces, ifc_class=None, name=''):
        self.faces = list(faces)
        self.ifc_class, self.name = ifc_class, name
        self._before = self._after = None

    def _write(self, scene, values):
        from copy import deepcopy
        from core import bim
        for face, value in zip(self.faces, values):
            if value is None:
                face.attrs.pop('ifc', None)
            else:
                face.attrs['ifc'] = deepcopy(value)
        bim._TAG_EPOCH += 1
        scene.mesh.bump_revision()
        scene.version += 1

    def do(self, scene):
        from copy import deepcopy
        from core.bim import next_object_id
        from core.ifc_identity import ensure_guid
        if self._before is None:
            self._before = [deepcopy(f.attrs.get('ifc')) for f in self.faces]
            tag = None
            if self.ifc_class:
                tag = {'id': next_object_id(scene), 'class': self.ifc_class, 'name': self.name}
                ensure_guid(tag)
            self._after = [deepcopy(tag) for f in self.faces]
        self._write(scene, self._after)

    def undo(self, scene):
        self._write(scene, self._before)


class SetNorthCommand(Command):
    """Tools ▸ North: where true north lies, in degrees clockwise from the
    green (Y) axis — one undo step.

    With a location the datum gets the new north angle (a NEW datum object:
    the map geometry is cached by datum identity, and the terrain is
    rebuilt from it — dropped here like :class:`StraightenModelCommand`
    does, the window builds it again). Without one, ``scene.north_deg``
    holds it and only the sun turns. The model never moves."""

    def __init__(self, degrees: float) -> None:
        from core.sun import wrap_north_deg
        self.degrees = wrap_north_deg(degrees)
        self._old_datum = None
        self._new_datum = None
        self._old_deg = 0.0
        self._terrain = None

    def do(self, scene) -> None:
        datum = getattr(scene, "georef", None)
        if datum is None:
            self._old_deg = float(getattr(scene, "north_deg", 0.0) or 0.0)
            scene.north_deg = self.degrees
        else:
            from georef.datum import SceneDatum
            self._old_datum = datum
            if self._new_datum is None:
                self._new_datum = SceneDatum(datum.lat, datum.lon,
                                             alt=datum.alt,
                                             north=self.degrees)
            scene.georef = self._new_datum
            self._terrain = getattr(scene, "terrain", None)
            if self._terrain is not None:
                scene.terrain = None     # display-only; rebuilt from the datum
        scene.version += 1

    def undo(self, scene) -> None:
        if self._old_datum is None:
            scene.north_deg = self._old_deg
        else:
            scene.georef = self._old_datum
            if self._terrain is not None:
                scene.terrain = self._terrain
        scene.version += 1

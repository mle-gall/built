# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Editor command adapters for document transactions and host-owned changes."""
from __future__ import annotations

from ingetrazo_core import planar_edits as _planar
from ingetrazo_core import object_replay as _replay, object_drawing as _drawing
from ingetrazo_core import (definition_edits as _definitions, placement_edits as _placements,
                            placement_transform as _transform)

from abc import ABC, abstractmethod
from collections import Counter
from typing import Iterable, Optional

from core.mesh import element_identity
from core.element_set import ElementSet
from ingetrazo_core import Vec3 as QVector3D
# Typed native steps (docs/commands-port.md): placement renames, locks and
# whole-placement paint are Rust deltas, created through the protocol verbs.
from ingetrazo_core import NativeCommand
# Group and component steps (rust/ingetrazo_core/src/doc/group_steps.rs).
from ingetrazo_core import (ReplaceComponentCommand, KeepComponentCommand, AddKeptComponentsCommand, GroupToComponentCommand, EditText3DCommand, ReparentGroupsCommand, MergeGroupsCommand, MakeUniqueCommand, InsertGroupCommand, DeleteGroupCommand, SetStructureNodeCommand, SplitIntoPiecesCommand,
                            SetMaterialFinishCommand)
# Annotation, section, guide, image, survey, saved-scene and sheet steps
# (rust/ingetrazo_core/src/doc/steps.rs): the retired classes' names.
from ingetrazo_core import (RecordStep, AddDimensionCommand, DeleteDimensionsCommand, AddTextLabelCommand, MoveTextLabelsCommand, MoveGuidesCommand, RotateGuidesCommand, MoveDimensionsCommand, EditTextLabelCommand, EditDimensionTextCommand, DeleteTextLabelsCommand, AddGuideCommand, PlaceSectionPlaneCommand, DeleteSectionPlanesCommand, ReverseSectionPlaneCommand, SetActiveSectionCommand, SetSectionModeCommand, SetSectionFillCommand, RenameSectionPlaneCommand, AddSheetCommand, PlaceViewCommand, SheetCommand, InsertSheetCommand, RemoveSheetCommand, MoveSheetCommand, CreatePlanCommand, AddSceneCommand, DeleteScenesCommand, UpdateSceneCommand, SceneTreeCommand, MoveSectionPlanesCommand, RotateSectionPlanesCommand, ChangeGuideCommand, DeleteGuidesCommand, AddImagePlaneCommand, DeleteImagePlanesCommand, TransformImagePlaneCommand, SetImagePlaneOpacityCommand, MoveImagePlanesCommand, RotateImagePlanesCommand, ScaleImagePlanesCommand, AddGeoPathCommand, DeleteGeoPointsCommand, DeleteGeoPathsCommand, ToggleGeoPathClosedCommand, SetGeoPathSurfaceCommand, MoveGeoPathNodeCommand)

from core.group import Group
from core import doc_model as _doc
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
    #: direct and mixed-command replay hook.
    doc_undo = False
    _journal_geometry = False

    def _direct_snapshot(self, mesh):
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


# The native steps are History commands for every ``isinstance`` check.
Command.register(NativeCommand)
Command.register(RecordStep)


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
        _replay.draw_edge(self, m)
        scene.version += 1

    def doc_after(self, scene, phase):
        if phase == "undo" and self._owned:
            scene.selection.discard(self.edge)
        scene.version += 1

    def undo(self, scene):
        if _replay.undo_edge(self, scene.mesh):
            scene.selection.discard(self.edge)
            scene.version += 1


class RemoveGeometryCommand(MeshCommand):
    """Deleting an element changes ordered incidence lists. Direct replay
    restores their order and the original element handles."""
    keeps_rows = True
    _before = None

    def undo(self, scene):
        if self._before is not None:
            scene.mesh.restore_state(self._before)
            scene.version += 1

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
        self._before = self._direct_snapshot(m)
        self.removed_edges, self.removed_faces = _drawing.delete_edges(m, self._endpoints, self.cascade_faces, scene.selection)
        scene.version += 1


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
        self._before = self._direct_snapshot(m)
        footprint = winding_footprint(m)
        from core.store_mesh import EdgeView, FaceView, StoreMesh, _hypot
        result = None
        if type(m) is StoreMesh and (hypot := _hypot()) is not None:
            result = m._store.erase_selection(
                [[p.toTuple() for p in loop] for loop in self._face_loops],
                [(m._input_point(a), m._input_point(b)) for a,b in self._edge_endpoints], hypot)
        if result is None:
            slit_planes, across = _drawing.erase_selection(m, self._face_loops, self._edge_endpoints, scene.selection)
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
            _drawing.prune_soft_edges(m)
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
            self._before = self._direct_snapshot(mesh)
            self.face = mesh.add_face(self.vertices, self.preset_holes)
            self.face.interior = self.interior
            if self.attrs:
                self.face.attrs.update(self.attrs)
            if self.auto:
                self._divide(mesh)
            self._after = self._direct_snapshot(mesh)
        scene.version += 1

    def _divide(self, mesh):
        from core.store_mesh import FaceView, StoreMesh, _hypot
        if (type(mesh) is StoreMesh and not mesh.__dict__.get("_planned_points")
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
        self._hole_swaps = _drawing.divide_face(mesh, self.face)

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
        self._before = self._direct_snapshot(mesh)
        self.face = _drawing.delete_face(mesh, self._loop, scene.selection)
        scene.version += 1


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
    return all(isinstance(e, Group) for e in items)


def _bulk_set(items, field, value):
    """Write ``value`` into ``field`` of every placement of ``items`` as one
    document op (``doc_set_many``: Lock, Hide or Tag of a whole selection),
    returning the previous values; ``None`` when not every item is a
    document placement (the caller writes them one by one)."""
    if not items:
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
        from ingetrazo_core import add_template_layers
        self._added = add_template_layers(scene, self._template)
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
                return self._explode(scene)
            self.curves = scene.mesh._store.explode_curves(ids)
            scene.mesh._chunk_dirty = True
            scene.mesh._mut_serial += 1
            scene.version += 1
            return
        self._explode(scene)

    def _explode(self, scene):
        self.curves, self._ids = _replay.explode_curves(scene.mesh, self.edges)
        scene.version += 1

    def undo(self, scene):
        _replay.restore_curves(scene.mesh, self._ids)
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
        _definitions.append_members(scene.geo_points, self._points)
        scene.version += 1

    def undo(self, scene) -> None:
        _definitions.remove_members(scene.geo_points, self._points)
        if self._datum is not None:
            scene.georef = self._prev_datum
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
    undo = RemoveGeometryCommand.undo

    def do(self, scene):
        if self._after is None:
            self._before = self._direct_snapshot(scene.mesh)
            self._edit(scene)
            self._after = self._direct_snapshot(scene.mesh)
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
        _transform.flip_groups(scene, self._groups, mirror_matrix(self.centre, self.axis))
        if self.doc_undo(scene):
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
        self._transform = _transform.GroupTransform()

    def _matrix(self, reverse):
        return rotation_matrix(self.center, self.axis, -self.degrees if reverse else self.degrees)

    def do(self, scene) -> None:
        if self._transform.apply(scene, self.group, self._matrix, False, self._journal_geometry):
            scene.version += 1

    def undo(self, scene) -> None:
        if self._transform.apply(scene, self.group, self._matrix, True, self._journal_geometry):
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
        self._transform = _transform.GroupTransform()

    def _matrix(self, reverse):
        factor = invert_scale_factor(self.factor) if reverse else self.factor
        return scale_matrix(self.center, factor, self.axes)

    def do(self, scene) -> None:
        if self._transform.apply(scene, self.group, self._matrix, False, self._journal_geometry):
            scene.version += 1

    def undo(self, scene) -> None:
        if self._transform.apply(scene, self.group, self._matrix, True, self._journal_geometry):
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
        self.snapshot = self._direct_snapshot(scene.mesh)
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
    rebuild (``Store::rebuild_push_planes``) instead of the winding-tolerant merge,
    which only remains for raw/open geometry where outwardness is undefined.

    ``dedupe=False`` skips the identical-cycle face dedupe of phase 0. The
    solid path's *first* stitch needs that: a flush-collapse sweep quad lands
    identical to the face it must annihilate *with*, and the per-plane rebuild
    is what decides whether the pair means "keep one" (a shared wall built
    twice — material on both sides) or "drop both" (an emptied region) — by
    parity, where the pair's two crossings cancel. Deduping it early restores
    the material reading and the collapse never classifies."""
    # A store does phases 0–2 in one native call (Store::stitch_connect).
    if type(mesh).__name__ == "StoreMesh" and mesh.stitch_connect(seedkeys, dedupe):
        if coplanar_merge:
            _coplanar_phase(mesh, seedkeys, new_faces)
        return
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
    if coplanar_merge:
        _coplanar_phase(mesh, seedkeys, new_faces)


def _coplanar_phase(mesh, seedkeys, new_faces) -> None:
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
            self.before = self._direct_snapshot(mesh)
            self.mutate(scene)
            self.after = self._direct_snapshot(mesh)
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
            self.before = self._direct_snapshot(scene.mesh)
            self.before_selection = ElementSet(scene.selection)
            self.faces_removed = simplify_mesh(
                scene.mesh, self.max_angle_degrees)
            self.after = self._direct_snapshot(scene.mesh)
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
        # Direct face undo also uses snapshots. The enclosing
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

    def do(self, scene, *, transaction_captured_before=False) -> None:
        if self.after is None:
            # First run: snapshot before, apply the plan, clean up any coplanar
            # overlap it created (redundant nested holes / spurious mother), then
            # snapshot the result so undo/redo restore exactly.
            if not transaction_captured_before:
                self.before = self._direct_snapshot(scene.mesh)
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
            self.after = self._direct_snapshot(scene.mesh)
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
        self.snapshot = self._direct_snapshot(scene.mesh)
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
        from ingetrazo_core import GroupGeometry
        self._geometry = GroupGeometry(faces, edges)
        self.snapshot = self.group = None

    def do(self, scene) -> None:
        m = scene.mesh
        self.snapshot = self._direct_snapshot(m)
        # Built once: a redo puts back the SAME group, the one the next
        # commands (Explode, Move, paint…) hold — a fresh one each time
        # left them pointing at a group no longer in the model, and
        # their redo was lost (#737).
        if self.group is None:
            self.group = self._build(scene)
        self._take_off_loose(scene)
        if not self._journal_geometry:
            self._after = m.capture_state()
        scene.groups.append(self.group)
        from ingetrazo_core import file_created_placement
        self._filing = file_created_placement(scene, self.group)
        scene.selection.clear()
        scene.selection.add(self.group)
        scene.version += 1

    def _build(self, scene) -> Group:
        return self._geometry.build(scene, self._component, self._name)

    def _take_off_loose(self, scene) -> None:
        m = scene.mesh
        return self._geometry.take_off(m, scene.selection.discard)

    def undo(self, scene) -> None:
        if getattr(self, "_filing", None) is not None:
            self._filing.undo(scene)
        if self.group in scene.groups:
            scene.groups.remove(self.group)
        scene.selection.discard(self.group)
        scene.mesh.restore_state(self.snapshot)
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
        self._before, self._after, nested = _definitions.unlink_placements(
            scene, self.groups, self._FIELDS, self._pairs)
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
            _definitions.restore_definitions(self.groups, self._after)
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
        _definitions.restore_definitions(self.groups, self._before)
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
        self._xform0, self._indices = _placements.nest(
            scene.groups, self.container, self.groups, scene.selection.discard)
        if self._inner is None:
            from ingetrazo_core import file_created_placement
            self._filing = file_created_placement(scene, self.container)
        scene.selection.clear()
        scene.selection.add(self.container)
        scene.version += 1

    def undo(self, scene) -> None:
        if getattr(self, "_filing", None) is not None:
            self._filing.undo(scene)
        _placements.unnest(scene.groups, self.container, self._xform0, self._indices)
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
        self.snapshot = self._direct_snapshot(m)
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
            if not self._journal_geometry and self._after is None:
                self._after = m.capture_state()

        # The nested objects come out one level up, whole: their placement
        # composes with the parent's (a shared prototype is never touched),
        # and an unpainted child takes the parent's paint, as its default
        # faces were already drawn with it.
        from ingetrazo_core import lift_placements
        self._lifted = lift_placements(scene.groups, g, self.index)
        kids = [row[0] for row in self._lifted]
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

        from ingetrazo_core import explode_object_geometry
        before = list(m.faces)
        added = explode_object_geometry(m, g.mesh, P, paint, inverse, invertible)
        m.resplit_curves()
        # A face that lands on faces already there merges with them, as a
        # pasted one does (#73): two boxes that touch share ONE wall instead
        # of two coincident faces (issue #721).
        _merge_overlapping_planes(scene, before, added)
        self.copies = []

    def undo(self, scene) -> None:
        from ingetrazo_core import lower_placements
        lower_placements(scene.groups, self._lifted)
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
        _transform.shift_group(scene, self.group, delta)
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
        _transform.reframe_world(scene, m, turn_deg, redo, self._snaps)
        scene.version += 1



def group_owner_list(scene, group):
    """The list ``group`` lives in: ``scene.groups`` at the root, or the
    ``children`` of the container that owns it, however deep. ``None`` when
    the group is not in the scene at all."""
    return _definitions.group_owner(scene, group)


class HealOverlapsCommand(MeshCommand):
    """Remove redundant 'mother' faces left overlapping their own subdivisions
    (a draw/delete can leave the big enclosing face on top). Snapshot undo."""

    def __init__(self) -> None:
        self.snapshot: Optional[dict] = None
        self.healed = 0

    def do(self, scene) -> None:
        self.snapshot = self._direct_snapshot(scene.mesh)
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


_point_in_tri = _planar.point_in_tri
_segment_grid = _planar.SegmentIndex


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

        scene.selection.clear()
        _planar.install_flat(mesh, type(mesh) is StoreMesh, edges, faces, flagged, old_attrs)
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
            self.rebuilt = _planar.store_regions(mesh, regions, keys, owners, flagged)
            scene.selection.clear()
            scene.version += 1
            return
        count = _planar.rebuild_scoped(self, mesh)
        if count is not None:
            self.rebuilt = count
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
        return history._native.group_start(session)

    @staticmethod
    def fold(history, session):
        return history._native.fold_occurrence(session, OccurrenceCommands)

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
        return _doc.NATIVE.DocumentHistory.replay_group(self, scene, undo, False)

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
        return self._axes_edit.fields_only()

    def __init__(self, group, frame) -> None:
        from ingetrazo_core import Matrix4 as QMatrix4x4
        self.group = group
        self.frame = QMatrix4x4(frame)
        self._axes_edit = _transform.AxesEdit()

    def geometry_meshes(self, scene):
        return (self.group.mesh,) if self.group.xform is not None else ()

    def do(self, scene) -> None:
        if self._axes_edit.apply(scene, self.group, self.frame):
            scene.version += 1

    def undo(self, scene) -> None:
        if self._axes_edit.undo(self.group):
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
        # What the edit was derived from: the merge base of the shared write.
        base = getattr(scene, '_project_registry_base', None)
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
        if base is not None:
            scene._project_registry_base = base
        if project.write_registry(scene, self.value):
            scene.version += 1
        else:
            super().do(scene)
        scene.mesh.bump_revision()

    def undo(self, scene):
        from core import project
        # Revert only what this command changed (base: its after-value).
        if project.write_registry(scene, self._project_before, base=self.value):
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

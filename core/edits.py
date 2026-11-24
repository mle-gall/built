# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Edit operations that turn drawn segments into history commands.

This is the bridge between the pure geometry planner in :mod:`core.topology`
and the undo/redo commands in :mod:`core.history`. Tools hand it the raw
segments the user drew; it returns a single reversible command that:

- splits existing edges the new segments cross (the classic auto-split),
- breaks each new segment at those crossings,
- welds coincident edges (via ``AddEdgeCommand``'s merge), and
- optionally auto-faces any planar cycle the new sub-edges close.

Segments are processed in order against a running simulation of the scene's
edge list, so a batch (e.g. a rectangle's four edges) splits correctly even
when later segments cross edges created by earlier ones.

Kept in its own module to avoid an import cycle: ``history`` imports
``topology``, so the command-building glue can't live in either of them.
"""
from __future__ import annotations
from core.mesh import element_identity

from typing import Iterable, Optional, Sequence

from ingetrazo_core import Vec3 as QVector3D, object_drawing, polygon_newell as _newell

_order_chain = object_drawing.order_chain

from core.geometry import Edge, Face
from core.history import (
    AddEdgeCommand,
    AddFaceCommand,
    Command,
    CompoundCommand,
    DeleteEdgesCommand,
    DeleteFaceCommand,
    SnapshotCompound,
)
from core.topology import (
    _point_in_face_solid,
    face_exists,
    find_chord_split,
    find_cycles_through,
    find_duplicate_edge,
    is_planar,
    plan_edge_split,
    split_edge_in_faces,
)

Segment = tuple[QVector3D, QVector3D]


touching_segments = object_drawing.touching_segments


def plan_edge_commands(
    scene,
    segments: Sequence[Segment],
    detect_faces: bool = True,
    eye=None,
    *,
    close_cycles: bool = True,
    soft=None, tag=None,
) -> list[Command]:
    """Build the ordered command list to add ``segments`` to ``scene``.

    Mirrors what each emitted command will do to the scene in a local
    ``simulated`` edge list, so planning and execution stay in lock-step.
    Returns a flat list (callers wrap it, possibly alongside their own
    commands such as a tool-managed face, in a single ``CompoundCommand``).
    ``close_cycles=False`` keeps chord splits but never infers new faces
    from cycles, for tools that carry existing topology.
    """
    from core import scan_index
    from core.topology import KeyMemo
    sc = scan_index.scan(scene.mesh)
    if sc is not None:
        # The mesh as arrays (core.scan_index): the lists below are the
        # scan's, edited as the plan goes, and their searches ask it.
        simulated = _ScanEdges(sc)
        faces_snapshot = _ScanFaces(sc)
        near = None
    else:
        simulated = _PlanEdges(scene.edges)
        faces_snapshot = list(scene.faces)
        near = _Nearby(simulated, faces_snapshot)
    simulated.key = KeyMemo()       # nothing moves while a plan is made
    if near is not None:
        near.key = simulated.key

    return object_drawing.plan_segments(simulated, faces_snapshot, near, segments,
                                        sc is not None, detect_faces, close_cycles,
                                        eye, soft, tag)


class _PlanEdges(list):
    """A plan's edge list, with the graph the cycle search walks kept
    beside it: built the first time a cycle is asked for, and following
    every ``append`` and ``remove`` after that. A plan only ever changes
    the list through those two."""

    key = None
    _graph = None

    def graph(self):
        if self._graph is None:
            from core.topology import CycleGraph, _key
            self._graph = CycleGraph(self, self.key or _key)
        return self._graph

    def append(self, edge) -> None:
        list.append(self, edge)
        if self._graph is not None:
            self._graph.add(edge)

    def remove(self, edge) -> None:
        list.remove(self, edge)
        if self._graph is not None and not self._graph.remove(edge):
            self._graph = None      # built again at the next question


_Nearby = object_drawing.Nearby


def _ScanEdges(scan):
    return object_drawing.ScanOverlay(scan, True)


def _ScanFaces(scan):
    return object_drawing.ScanOverlay(scan, False)


def _scene_has_curves(scene) -> bool:
    from core import scan_index
    sc = scan_index.scan(scene.mesh)
    if sc is not None:
        return sc.native.any_curve()
    return object_drawing.has_plane_edge(scene.edges)


def _append_flat_curve_rebuild(scene, commands, points, *, close_cycles=True) -> None:
    """Append the deterministic planar-arrangement rebuild after a draw when
    the cycle planner alone cannot produce the expected regions:

    - Whole-flat drawing WITH curves → full rebuild (every minimal region gets
      a face; a square over a circle splits into three areas).
    - Whole-flat drawing, straight edges only, faces present → SCOPED rebuild
      with coverage semantics (two overlapping rectangles split into three
      regions; regions nobody covered stay empty — no resurrecting faces the
      user deleted).
    - 3D scene whose drawing plane carries curve edges → SCOPED rebuild on
      just that plane (straight-only 3D drawing keeps its proven path).

    Dangling edges survive the rebuild (spur pruning only affects face
    tracing), so half-drawn chains are safe."""
    from core.history import RebuildPlanarFacesCommand, RebuildPlaneFacesCommand

    if any(isinstance(c, (RebuildPlanarFacesCommand, RebuildPlaneFacesCommand))
           for c in commands):
        return
    from core.arrangement import coplanar_plane

    has_curves = _scene_has_curves(scene)
    if _whole_drawing_plane(scene, points) is not None:
        if has_curves and close_cycles:
            # Whole-flat drawing with curves: the full rebuild (every minimal
            # region gets a face — how a lone circle yields its disc).
            commands.append(RebuildPlanarFacesCommand())
        elif 0 < len(scene.mesh.faces) <= 400:
            # Apply the populated-plane rebuild limit to flat imports too.
            # Whole-flat, straight edges only, faces present: the SCOPED
            # rebuild with coverage semantics. Two overlapping rectangles must
            # split into three regions (the classic behaviour) — the planner leaves
            # the first rectangle whole over the lens (rect.igz report) — but
            # regions nobody covered stay empty (no resurrecting faces the
            # user deleted).
            normal = scene.mesh.faces[0].normal()
            origin = scene.mesh.faces[0].vertices[0]
            commands.append(RebuildPlaneFacesCommand(origin, normal))
        return
    if not has_curves:
        return
    plane = coplanar_plane(list(points))
    if plane is None:
        return
    origin, normal = plane
    tol = 1e-4

    from core import scan_index
    sc = scan_index.scan(scene.mesh)
    if sc is None:
        edges = scene.mesh.edges
    else:
        edges = sc.edges(sc.native.edges_on_plane(
            (origin.x(), origin.y(), origin.z()),
            (normal.x(), normal.y(), normal.z()), tol, True))
    if object_drawing.has_plane_edge(edges, plane):
        commands.append(RebuildPlaneFacesCommand(origin, normal))


def _whole_drawing_plane(scene, points):
    """``coplanar_plane(every vertex of the mesh + points)``: whether the
    whole drawing is flat. Its answer hangs on the first few points (they
    fix the plane) and then on whether ANY point leaves it, so with a mesh
    scan the first points are walked as the function walks them and the
    rest are judged in one array pass — a point clearly off the plane, by
    more than the float32 error of the test, decides; the doubtful ones go
    through the test itself. Without a scan, the call on the full list."""
    from core import scan_index
    sc = scan_index.scan(scene.mesh)
    if sc is None:
        return object_drawing.coplanar_plane(points, vertices=scene.mesh.vertices)
    from core._native import qt_hypot
    source = scene.mesh._store if sc.store_backed else sc.native
    plane = source.drawing_plane([p.toTuple() for p in points], qt_hypot())
    return None if plane is None else tuple(QVector3D(*p) for p in plane)


def _append_face_plane_rebuild(scene, commands, points) -> None:
    """Append the scoped plane rebuild when straight edges land on a plane
    that already carries faces, in a scene the whole-flat gate can't cover.

    The naive cycle planner mishandles a chord that touches a hole boundary
    (a line across a slab top grazing an inner rectangle's edge): instead of
    subdividing the mother it stacks a duplicate, sometimes flipped, region
    face on top — and erasing any of those edges later cascade-deletes the
    whole plane instead of merging (aa.igz plaza report, 2026-07-12). The
    deterministic arrangement with coverage semantics is the machinery that
    already solves this for curves; a populated plane deserves it too."""
    from core.history import RebuildPlanarFacesCommand, RebuildPlaneFacesCommand

    if any(isinstance(c, (RebuildPlanarFacesCommand, RebuildPlaneFacesCommand))
           for c in commands):
        return
    from core import scan_index
    from core._native import qt_hypot
    sc = scan_index.scan(scene.mesh)
    if sc is None:
        plane = object_drawing.face_rebuild_plane(scene.mesh.faces, points, 400)
    else:
        source = scene.mesh._store if sc.store_backed else sc.native
        plane = source.rebuild_plane([p.toTuple() for p in points], 400, qt_hypot())
        if plane is not None:
            plane = tuple(QVector3D(*p) for p in plane)
    if plane is not None:
        commands.append(RebuildPlaneFacesCommand(*plane))


def _append_active_tag(scene, commands) -> None:
    """Tag-as-you-draw: when the BIM active class is set, append the step that
    stamps it on the plan's own drawn faces. Inserted BEFORE the plane
    rebuilds, so a rebuild that replaces the drawn face inherits the tag
    per-region (A.3) instead of losing the reference."""
    tag = getattr(scene, "active_ifc", None)
    if not tag:
        return
    from core.history import AutoTagDrawnFacesCommand
    face_cmds = [c for c in commands
                 if isinstance(c, AddFaceCommand) and c.auto]
    if face_cmds:
        commands.append(AutoTagDrawnFacesCommand(face_cmds, tag))


def face_up_or_toward(loop, eye=None):
    """The winding a brand-new face should take (the usual rule for a face
    with no neighbour to agree with): a horizontal one shows its FRONT
    upwards, any other one faces the ``eye`` that drew it. A loop closed by
    hand with Line, or by Offset, came out either way — the order the cycle
    was traced in — and Marco read the blue back as «cara invertida»
    (2026-09-21, tests 15 and 19 of the bench). Returns the loop, reversed
    when needed; untouched when nothing decides (no eye and a vertical
    face, or a degenerate loop)."""
    return object_drawing.face_winding(loop, eye)


def build_add_edge(scene, a: QVector3D, b: QVector3D, detect_faces: bool = True,
                   eye=None, *, soft=None, tag=None) -> Command:
    """Single-segment convenience: one drawn edge → one atomic command."""
    commands = plan_edge_commands(scene, [(a, b)], detect_faces=detect_faces,
                                  eye=eye, soft=soft, tag=tag)
    _append_active_tag(scene, commands)
    _append_flat_curve_rebuild(scene, commands, [a, b])
    if detect_faces:
        _append_face_plane_rebuild(scene, commands, [a, b])
    # When curves exist, even a single added edge can break a circle into
    # contours (a tangent line landing on a curve vertex splits it),
    # so it must go through SnapshotCompound, which runs the contour re-split —
    # and whose undo restores the reunited curve.
    if len(commands) == 1 and soft is None and tag is None and not _scene_has_curves(scene):
        return commands[0]
    # Splits/welds/hole-punches don't compose into a clean per-op inverse — undo
    # via one snapshot so it reverses exactly (no orphan edges/vertices left).
    return SnapshotCompound(commands)


def build_add_edges(
    scene,
    segments: Sequence[Segment],
    detect_faces: bool = True,
    extra: Iterable[Command] = (),
    eye=None,
    plane_rebuild: bool = False,
    *,
    preserve_faces: bool = False,
) -> Command:
    """Batch convenience: many drawn edges (+ optional ``extra`` commands such
    as a tool-managed face) → one atomic command. ``eye`` (the camera's
    position) decides which way a new vertical face looks.

    ``plane_rebuild``: a tool that owns its face (the Rectangle) asks for
    the plane it lands on to be rebuilt, as a drawn Line has been since
    July — drawn over a ring on walls it stacked a full face on the ring
    instead of filling the hole (#538).

    ``preserve_faces``: copied geometry supplies its own faces. Split faces
    the edges cross, but do not face empty cycles or globally heal the mesh.
    Rebuild coverage only for cuts or an explicitly requested plane rebuild;
    callers must keep isolated copies out of the planner (#1087).
    """
    commands = plan_edge_commands(scene, segments, detect_faces=detect_faces,
                                  eye=eye, close_cycles=not preserve_faces)
    commands.extend(extra)
    _append_active_tag(scene, commands)
    points = [p for seg in segments for p in seg]
    cuts = any(isinstance(c, (DeleteFaceCommand, DeleteEdgesCommand)) for c in commands)
    if not preserve_faces or cuts or plane_rebuild:
        _append_flat_curve_rebuild(scene, commands, points,
                                   close_cycles=not preserve_faces)
        if plane_rebuild:
            _append_face_plane_rebuild(scene, commands, points)
    if len(commands) == 1 and not _scene_has_curves(scene):
        return commands[0]
    return SnapshotCompound(commands, heal=not preserve_faces)


def divide_edges(mesh, edges, n: int) -> int:
    """Divide (issue #63, @pacaeiro): split each selected edge —
    or, for an edge of a curve, the whole curve — into ``n`` pieces of
    equal length. A straight edge gets n−1 new vertices. A curve (arc,
    circle) is measured along its chain, cut where the k/n marks fall, and
    comes out as ``n`` INDEPENDENT arcs — each piece its own curve, each
    selectable on its own, the classic behaviour (a circle divided in
    four is four quarter arcs; Marco, 2026-09-21).
    A mark that lands on an existing facet vertex cuts nothing there but
    still parts the curve. Faces the edges bound take the new vertices in
    their loops (``Mesh.split_edge``). Returns the number of cuts made."""
    n = int(n)
    if n < 2:
        return 0
    from core import mesh as mesh_module
    from core.store_mesh import StoreMesh, _hypot
    import sys
    if type(mesh) is StoreMesh and (hypot := _hypot()) is not None:
        (cuts, next_id), change = mesh._store.divide_edges(
            [e.key for e in edges], n, mesh_module._CURVE_COUNTER, hypot,
            sys.version_info >= (3, 12))
        mesh_module._CURVE_COUNTER = next_id
        mesh._dirty(3 * cuts)
        if change.modified["edge"]:
            mesh._chunk_dirty = True
        return cuts
    return object_drawing.divide_edges(mesh, edges, n)


#: Why Join Segments leaves a point where it is, for its preview: the
#: points it takes away ("join") and the ones on a straight line that it
#: keeps all the same, each for its reason. A plain corner is not listed:
#: only a kink close enough to straight to be mistaken for none.
JOIN_REASONS = ("join", "kink", "branch", "flags", "curve")

def join_edges(mesh, edges, verdicts: Optional[dict] = None) -> list:
    """Join Segments, the inverse of Divide: every vertex where two of
    ``edges`` meet in a straight line, with nothing else attached to it,
    goes, and the two segments become one line. A long line drawn in
    pieces, or one left cut by a divide or by an erased crossing, comes out
    clean, its faces (if it bounds any) losing the needless corner.

    A vertex stays when a third edge meets it, when the two segments are
    not collinear, bound different faces, carry different flags (soft,
    hidden, layer), or belong to a curve: an arc's facets are not
    segments to merge. Returns the edges the selection is made of
    afterwards, the joined lines in place of their pieces.

    ``verdicts``, when given, is filled with what happened to each point
    on the way, ``{vertex: reason}`` (see :data:`JOIN_REASONS`): what
    :func:`join_preview` shows."""
    live = [e for e in edges if e in mesh.edges]
    from core.store_mesh import StoreMesh, EdgeView, VertexView, _hypot
    import sys
    if type(mesh) is StoreMesh and (hypot := _hypot()) is not None:
        (keys, reasons, joins), _ = mesh._store.join_edges(
            [e.key for e in live], hypot, sys.version_info >= (3, 12))
        mesh._dirty(3 * joins)
        if verdicts is not None:
            verdicts.update((VertexView(mesh, k), why) for k, why in reasons)
        return [EdgeView(mesh, k) for k in keys]
    return object_drawing.join_edges(mesh, live, verdicts)


def join_preview(mesh, edges) -> dict:
    """What Join Segments would do to ``edges``, without doing it:
    ``{vertex: reason}`` for the points it would take away and the ones on
    a straight line it would keep (see :data:`JOIN_REASONS`). A dry run of
    :func:`join_edges` itself, so the preview never disagrees with the
    command; the mesh is put back as it was, the same objects included."""
    verdicts: dict = {}
    snap = mesh.capture_state()
    try:
        join_edges(mesh, edges, verdicts)
    finally:
        mesh.restore_state(snap)
    return verdicts

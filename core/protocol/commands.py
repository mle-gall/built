# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Commands of the core protocol by key (docs/core-protocol.md §1): each
command runs through History and returns the step published by its change feed.

``run(history, name, **args)`` dispatches the names in ``COMMANDS``.
Arguments are keys, placement uids and plain values. An
unknown key raises ``KeyError`` before anything runs. The answer is
``{"ok", "step", "error", "rejected"}`` with ``step`` the feed's record
(a :class:`core.protocol.feed.Step`) or None when nothing happened."""
from __future__ import annotations

from core.protocol.keys import elements, from_json_key


def _need(mesh, kind, keys):
    keys = [from_json_key(k) for k in keys]
    found = elements(mesh, kind, keys)
    gone = [k for k in keys if k not in found]
    if gone:
        raise KeyError(f"no live {kind} with key {gone[0]} in the open mesh")
    return [found[k] for k in keys]


def _answer(history, before, ok=True):
    step = history.feed.last if history.revision != before else None
    return {"ok": bool(ok and step is not None), "step": step,
            "error": history.last_error,
            "rejected": getattr(history, "last_rejected", None)}


def move_vertices(history, vertices, delta):
    """Move the vertices (keys) by ``delta`` ``[x, y, z]``: the Move
    tool's command (``MoveVerticesCommand``: coincident vertices move
    along, warped faces fold)."""
    from PySide6.QtGui import QVector3D
    from core.history import MoveVerticesCommand
    vs = _need(history.scene.mesh, "vertex", vertices)
    before = history.revision
    history.execute(MoveVerticesCommand([QVector3D(v.position) for v in vs],
                                        QVector3D(*map(float, delta))))
    return _answer(history, before)


def erase(history, edges=(), faces=()):
    """Erase edges and faces (keys) the Eraser's way
    (``EraseSelectionCommand``: a split line between coplanar faces
    dissolves, other edges take their faces)."""
    from core.history import EraseSelectionCommand
    mesh = history.scene.mesh
    es, fs = _need(mesh, "edge", edges), _need(mesh, "face", faces)
    before = history.revision
    history.execute(EraseSelectionCommand(es, fs))
    return _answer(history, before)


class _Headless:
    """The viewport surface the Push/Pull tool's commit uses."""

    def __init__(self, history):
        self.scene = history.scene
        self.history = history

    def update(self):
        pass

    def set_hover(self, entity):
        pass

    def set_suppressed_faces(self, faces):
        pass

    def flash_status(self, text, msec=2500):
        pass


def push_pull(history, face, distance, keep_base=False):
    """Push or pull a face (key) by ``distance`` along its normal: the
    Push/Pull tool locked on the face, the distance typed, committed —
    clamped where the tool clamps (a push stopped at an opposite face)."""
    from tools.pushpull import PushPullTool
    (f,) = _need(history.scene.mesh, "face", [face])
    tool = PushPullTool()
    tool.base_face = f
    tool.dragging = True
    tool._group = None
    target = tool._target_scene(history.scene)
    tool._anchor = f.centroid()
    tool._normal = f.normal()
    tool._attached, tool._prism_cap = tool._classify_base(target)
    tool._cap_positions = tool._cap_loop_positions(f)
    tool._keep_base = bool(keep_base)
    tool._compute_inward_limit(target)
    tool.extrusion = float(distance)
    tool._clamp_extrusion()
    before = history.revision
    if abs(tool.extrusion) < 1e-6:
        tool._reset()
        return _answer(history, before, ok=False)
    tool._commit(_Headless(history))
    return _answer(history, before)


def follow_me(history, face, path, closed=False):
    from PySide6.QtGui import QVector3D
    from core.sweep import build_sweep_face
    points = [QVector3D(*p) for p in path]
    cmd = build_sweep_face(history.scene.mesh, from_json_key(face), points, closed)
    before = history.revision
    history.execute(cmd)
    return _answer(history, before)


def intersect_faces(history, mode, faces=None, placements=None):
    from core.intersect import build_intersect_faces, WITH_CONTEXT, WITH_SELECTION, WITH_MODEL
    if mode not in (WITH_CONTEXT, WITH_SELECTION, WITH_MODEL, "placements"):
        raise ValueError(mode)
    if placements is not None and (mode != "placements" or faces is not None):
        raise ValueError("placement uids require placements mode")
    groups = None
    if placements is not None:
        scene = history.scene
        siblings = scene.edit_group.children if scene.edit_group else scene.groups
        by_uid = {g.uid: g for g in siblings}
        groups = [by_uid[uid] for uid in dict.fromkeys(placements)]
    if faces is not None:
        faces = [from_json_key(k) for k in faces]
    before = history.revision
    _, cmd = build_intersect_faces(history.scene, mode, faces, groups)
    if cmd is not None:
        history.execute(cmd)
    return _answer(history, before)


def rebuild_cap(history, face):
    """Rebuild the plane of a seed face with the direct cap planner."""
    from core.cap_rebuild import apply_rebuild
    from core.history import CommandRejected, SnapshotMutation
    seed, = _need(history.scene.mesh, "face", [face])
    origin, normal = seed.centroid(), seed.normal()

    def rebuild(scene):
        if not apply_rebuild(scene.mesh, origin, normal):
            raise CommandRejected("Cap already matches its plane")

    before = history.revision
    history.execute(SnapshotMutation(rebuild))
    return _answer(history, before)


def orient_faces(history, face):
    """Orient the connected sheet to the seed face; all keys survive."""
    from core.orient_faces import faces_to_flip
    from core.history import FlipFacesCommand
    seed, = _need(history.scene.mesh, "face", [face])
    flip = faces_to_flip(seed)
    before = history.revision
    if flip:
        history.execute(FlipFacesCommand(flip))
    return _answer(history, before)


def solid_operation(history, operation, placements):
    """Run a solid tool with placement uids in pick order."""
    from core.solids import OPS, SolidOperationCommand
    if operation not in OPS:
        raise ValueError(operation)
    if len(set(placements)) != len(placements):
        raise ValueError("duplicate placement uid")
    scene = history.scene
    siblings = scene.edit_group.children if scene.edit_group else scene.groups
    groups = {g.uid: g for g in siblings}
    inputs = [groups[uid] for uid in placements]
    before = history.revision
    history.execute(SolidOperationCommand(operation, inputs))
    return _answer(history, before)


def add_edge(history, a, b, soft=None, tag=None):
    from PySide6.QtGui import QVector3D
    from core.edits import build_add_edge
    before = history.revision
    if tag is not None and history.scene.layer_by_key(tag) is None:
        raise KeyError(f"no tag with key {tag!r}")
    history.execute(build_add_edge(history.scene, QVector3D(*a), QVector3D(*b), soft=soft, tag=tag))
    return _answer(history, before)


def add_edges(history, segments):
    from PySide6.QtGui import QVector3D
    from core.edits import build_add_edges
    before = history.revision
    history.execute(build_add_edges(history.scene, [(QVector3D(*a), QVector3D(*b))
                                                    for a, b in segments]))
    return _answer(history, before)


def _edge_edit(history, edges, edit, *args):
    from core.history import SnapshotMutation
    selected = _need(history.scene.mesh, "edge", edges)
    before = history.revision
    history.execute(SnapshotMutation(lambda sc: edit(sc.mesh, selected, *args)))
    return _answer(history, before)


def divide_edges(history, edges, n):
    from core.edits import divide_edges as divide
    return _edge_edit(history, edges, divide, int(n))


def join_edges(history, edges):
    from core.edits import join_edges as join
    return _edge_edit(history, edges, join)


def _placement(history, uid):
    """The top-level placement (``scene.groups``) with ``uid``."""
    for g in history.scene.groups:
        if getattr(g, "uid", None) == uid:
            return g
    raise KeyError(f"no placement with uid {uid!r} in the open context")


def explode_group(history, placement):
    """Explode the placement (uid): its geometry merges into the open mesh
    through its matrix (welding onto what is there, coplanar overlaps
    rebuilt into one face), its nested placements come out whole
    (``ExplodeGroupCommand``). The step's ``copies`` say where each of the
    group's vertices, edges and faces went (store-backed meshes)."""
    from core.history import ExplodeGroupCommand
    g = _placement(history, placement)
    before = history.revision
    history.execute(ExplodeGroupCommand(g))
    return _answer(history, before)


def split_into_pieces(history, placement):
    """Regroup the placement's (uid) contents by physical piece
    (``core.pieces.split_into_pieces``, ``SplitIntoPiecesCommand``): one
    child group per solid that does not touch another. Nothing happens
    (``ok`` False, no step) when it already is its pieces."""
    from core.history import SplitIntoPiecesCommand
    from core.pieces import split_into_pieces as split
    g = _placement(history, placement)
    before = history.revision
    pieces = split(g)
    if not pieces:
        return _answer(history, before, ok=False)
    history.execute(SplitIntoPiecesCommand(g, pieces))
    return _answer(history, before)


def explode_view(history, placement, factor, mode="outward"):
    """Pull the placement's (uid) parts apart to ``factor`` (0
    reassembles) along ``mode``: ``outward``, ``x``, ``y`` or ``z``
    (``ExplodeViewCommand``)."""
    from core.explode import MODES
    from core.history import ExplodeViewCommand
    if mode not in MODES:
        raise ValueError(f"mode must be one of {MODES}")
    g = _placement(history, placement)
    before = history.revision
    history.execute(ExplodeViewCommand(g, float(factor), mode))
    return _answer(history, before)


def purge_unused(history, layers=True, materials=True):
    """Remove the tags and/or materials nothing in the document uses
    (``PurgeUnusedCommand``; tags a saved view hides stay)."""
    from core.history import PurgeUnusedCommand
    before = history.revision
    cmd = PurgeUnusedCommand(layers=bool(layers), materials=bool(materials))
    if cmd.counts(history.scene) == (0, 0):
        return _answer(history, before, ok=False)
    history.execute(cmd)
    return _answer(history, before)


def purge_debris(history):
    """Drop the open mesh's vertices that no edge and no face uses
    (``Mesh.purge_debris``, one snapshot step)."""
    from core.history import SnapshotMutation
    before = history.revision
    history.execute(SnapshotMutation(lambda scene: scene.mesh.purge_debris()))
    return _answer(history, before)


def undo(history):
    before = history.revision
    history.undo()
    return _answer(history, before)


def redo(history):
    before = history.revision
    history.redo()
    return _answer(history, before)


COMMANDS = {"follow_me": follow_me, "intersect_faces": intersect_faces,
            "add_edge": add_edge, "add_edges": add_edges,
            "divide_edges": divide_edges, "join_edges": join_edges,
            "move_vertices": move_vertices, "erase": erase,
            "push_pull": push_pull, "orient_faces": orient_faces,
            "rebuild_cap": rebuild_cap, "solid_operation": solid_operation,
            "explode_group": explode_group,
            "split_into_pieces": split_into_pieces,
            "explode_view": explode_view, "purge_unused": purge_unused,
            "purge_debris": purge_debris, "undo": undo, "redo": redo}


def run(history, name, **args):
    try:
        fn = COMMANDS[name]
    except KeyError:
        raise ValueError(f"unknown command {name!r}; one of {sorted(COMMANDS)}") from None
    return fn(history, **args)

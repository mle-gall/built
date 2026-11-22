# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Core protocol commands by key. Each edit is a History step on the open
mesh. The result contains ok, step, error and rejected. Unknown element
keys raise KeyError before the command runs (docs/core-protocol.md §1)."""
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


def undo(history):
    before = history.revision
    history.undo()
    return _answer(history, before)


def redo(history):
    before = history.revision
    history.redo()
    return _answer(history, before)


COMMANDS = {"add_edge": add_edge, "add_edges": add_edges,
            "divide_edges": divide_edges, "join_edges": join_edges,
            "move_vertices": move_vertices, "erase": erase,
            "push_pull": push_pull, "undo": undo, "redo": redo}


def run(history, name, **args):
    try:
        fn = COMMANDS[name]
    except KeyError:
        raise ValueError(f"unknown command {name!r}; one of {sorted(COMMANDS)}") from None
    return fn(history, **args)

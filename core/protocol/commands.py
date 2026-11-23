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

from core.history import Command
from core.protocol.records import patch_record_field, set_filing_rule
from core.protocol.keys import elements, from_json_key, store_of


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
    from ingetrazo_core import Vec3 as QVector3D
    from core.history import MoveVerticesCommand
    vs = _need(history.scene.mesh, "vertex", vertices)
    before = history.revision
    if store_of(history.scene.mesh) is not None:
        x, y, z = map(float, delta)
        cmd = MoveVerticesCommand([v.position_tuple() for v in vs], (x, y, z))
    else:
        cmd = MoveVerticesCommand([v.position for v in vs], QVector3D(*map(float, delta)))
    history.execute(cmd)
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
    from core.coordinate_frame import ZERO, choose_origin, mesh_frame
    st = store_of(history.scene.mesh)
    origin = choose_origin(st.face_positions(f.key)[0][0]) if st is not None else ZERO
    with mesh_frame(history.scene.mesh, origin):
        return _push_face(history, f, distance, keep_base, PushPullTool())


def _push_face(history, f, distance, keep_base, tool):
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
    from ingetrazo_core import Vec3 as QVector3D
    from core.sweep import build_sweep_face
    points = (list(path) if store_of(history.scene.mesh) is not None else
              [QVector3D(*p) for p in path])
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


def reverse_faces(history, faces):
    from core.history import FlipFacesCommand
    selected = _need(history.scene.mesh, "face", faces)
    before = history.revision
    if selected:
        history.execute(FlipFacesCommand(selected))
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
    if tag is not None and history.scene.layer_by_key(tag) is None:
        raise KeyError(f"no tag with key {tag!r}")
    return _draw_segments(history, [(a, b)], single=True, soft=soft, tag=tag)


def add_edges(history, segments):
    return _draw_segments(history, list(segments))


def _draw_segments(history, segments, single=False, **options):
    from core.coordinate_frame import ZERO, choose_origin, mesh_frame, point_in_frame
    from core.edits import build_add_edge, build_add_edges
    from core.coordinate_frame import planned_points
    mesh = history.scene.mesh
    st = store_of(mesh)
    origin = choose_origin(segments[0][0]) if st is not None and segments else ZERO
    local = [(point_in_frame(a, origin), point_in_frame(b, origin)) for a, b in segments]
    before = history.revision
    with mesh_frame(mesh, origin):
        cmd = (build_add_edge(history.scene, *local[0], **options) if single else
               build_add_edges(history.scene, local))
        exact = {p.toTuple(): tuple(v-o for v, o in zip(raw, origin))
                 for pair, args in zip(local, segments) for p, raw in zip(pair, args)
                 } if st is not None else {}
        with planned_points(mesh, exact):
            history.execute(cmd)
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


def _placements(history, uids):
    """Placements of the open context by uid, nested ones included (the
    project tree names items at any depth)."""
    want = list(dict.fromkeys(tuple(u) if isinstance(u, (list, tuple)) else u for u in uids))
    found = {}
    for path in (u for u in want if isinstance(u, tuple)):
        group = history.scene.resolve_occurrence(path)
        if group is None:
            raise KeyError(f"no placement at occurrence path {path!r}")
        found[path] = group
    wanted = set(want)
    stack = list(reversed(history.scene.groups))
    while stack and len(found) < len(wanted):
        g = stack.pop()
        uid = getattr(g, "uid", None)
        if uid in wanted and uid not in found:
            found[uid] = g
        stack.extend(reversed(getattr(g, "children", None) or ()))
    gone = [u for u in want if u not in found]
    if gone:
        raise KeyError(f"no placement with uid {gone[0]!r} in the open context")
    return list(dict.fromkeys(found[u] for u in want))


def hide(history, placements):
    """Hide placements (uids, nested included): ``HideCommand``."""
    from core.history import HideCommand
    gs = _placements(history, placements)
    before = history.revision
    history.execute(HideCommand(gs, hidden=True))
    return _answer(history, before)


def unhide(history, placements):
    """Show placements again (uids, nested included): ``HideCommand``."""
    from core.history import HideCommand
    gs = _placements(history, placements)
    before = history.revision
    history.execute(HideCommand(gs, hidden=False))
    return _answer(history, before)


def lock(history, placements, locked=True):
    """Lock (or unlock) placements (uids, nested included): ``LockCommand``."""
    from core.history import LockCommand
    gs = _placements(history, placements)
    before = history.revision
    history.execute(LockCommand(gs, locked=bool(locked)))
    return _answer(history, before)


def rename_placement(history, placement, name):
    """Rename one placement (uid, nested included): ``RenameGroupCommand``
    (a component's copies keep their own names)."""
    from core.history import RenameGroupCommand
    g, = _placements(history, [placement])
    before = history.revision
    history.execute(RenameGroupCommand(g, str(name)))
    return _answer(history, before)


def set_tag(history, placements, tag):
    """Put placements (uids, nested included) on a tag (key):
    ``AssignLayerCommand``."""
    from core.history import AssignLayerCommand
    gs = _placements(history, placements)
    layer = history.scene.layer(tag)
    if layer is None:
        raise KeyError(f"no tag with key {tag!r}")
    before = history.revision
    history.execute(AssignLayerCommand(gs, layer.key))
    return _answer(history, before)


class EditTagCommand(Command):
    doc_undo = True
    keeps_rows = True

    def __init__(self, edit):
        self.edit = edit

    def do(self, scene):
        self.edit.apply(True)
        self.doc_after(scene, "execute")

    def undo(self, scene):
        self.edit.apply(False)
        self.doc_after(scene, "undo")

    def doc_after(self, scene, phase):
        from core.history import _dirty_group_chunks
        _dirty_group_chunks(scene)
        super().doc_after(scene, phase)


def unlink_placements(history, placements):
    from core.history import UnlinkCommand
    command = UnlinkCommand(_placements(history, placements))
    before = history.revision
    if command.groups:
        history.execute(command)
    return _answer(history, before)


def edit_loose_geometry(history, field):
    from ingetrazo_core import loose_edit
    before = history.revision
    history.execute(loose_edit(history.scene, field))
    return _answer(history, before)


def file_placements(history, placements, node):
    from core import structure
    before = history.revision
    history.execute(structure.file(history.scene, placements, node))
    return _answer(history, before)


def edit_structure_node(history, node, field, value):
    from ingetrazo_core import plan_node_edit
    before = history.revision
    history.execute(plan_node_edit(history.scene, node, field, value))
    return _answer(history, before)


def edit_tag(history, tags, field, value):
    edit = history.scene._root._tag_edit(tags, field, value)
    before = history.revision
    if edit is not None:
        history.execute(EditTagCommand(edit))
    return _answer(history, before)


class SetTagStateCommand(Command):
    """Show / hide and lock / unlock tags and tag folders as one step. A
    linked file's tag switched here is the host's *override* of it
    (``core.links``: kept by the host, never written to the source, the
    source's later switches no longer reach it), recorded now rather than
    guessed at the next reload; undo puts the overrides back with the
    flags."""
    keeps_paint = True
    keeps_rows = True
    #: Tag flags, folder visibility and the links' overrides (plugin data)
    #: are all document state: the step's transaction undoes it.
    doc_undo = True

    def __init__(self, tags, folders, visible, locked):
        self.tags, self.folders = list(tags), list(folders)
        self.visible, self.locked = visible, locked

    def doc_after(self, scene, phase) -> None:
        if phase == "redo":
            for entity in list(scene.selection):
                if not scene.entity_visible(entity):
                    scene.selection.discard(entity)
        scene.version += 1

    def do(self, scene):
        from copy import deepcopy
        from core import layers as L, links
        self.before = L.layer_state(scene)
        self.links_before = deepcopy({k: e.get("overrides") for k, e in links.registry(scene).items()
                                      if isinstance(e, dict)})
        if self.visible is not None:
            for key in self.folders:
                scene.folder_visibility[key] = bool(self.visible)
            L.set_visible(scene, self.tags, self.visible)
            for key in self.tags:
                links.note_visibility(scene, key, bool(self.visible))
            for entity in list(scene.selection):
                if not scene.entity_visible(entity):
                    scene.selection.discard(entity)
        if self.locked is not None:
            L.set_locked(scene, self.tags, self.locked)
        scene.version += 1

    def undo(self, scene):
        from core import layers as L, links
        L.apply_layer_state(scene, self.before)
        for key, entry in links.registry(scene).items():
            if isinstance(entry, dict) and key in self.links_before:
                entry["overrides"] = dict(self.links_before[key] or {})
        scene.version += 1


def set_tag_state(history, tags=(), folders=(), visible=None, locked=None):
    """Show or hide (``visible``) and lock or unlock (``locked``) tags
    (keys) and tag folders (keys; visibility only) as one undoable step.
    A linked file's tag (``"A / Rails"``) records the host's override in
    ``plugin_data["links"][path]["overrides"]`` (§1). Unknown keys raise
    ``KeyError`` before anything runs."""
    scene = history.scene
    tags, folders = list(dict.fromkeys(tags)), list(dict.fromkeys(folders))
    for key in tags:
        if scene.layer_by_key(key) is None:
            raise KeyError(f"no tag with key {key!r}")
    known = set(getattr(scene, "layer_folders", ()) or ())
    for key in folders:
        if key not in known:
            raise KeyError(f"no tag folder with key {key!r}")
    if visible is None and locked is None:
        raise ValueError("set_tag_state needs visible or locked")
    if folders and locked is not None and visible is None:
        raise ValueError("tag folders have no lock")
    cmd = SetTagStateCommand(tags, folders, None if visible is None else bool(visible),
                             None if locked is None else bool(locked))
    before = history.revision
    history.execute(cmd)
    return _answer(history, before)


def resegment_curve(history, edge, segments):
    """Rebuild the regular curve (circle, polygon or arc) one of whose
    edges (key) is given with ``segments`` straight pieces, in place:
    ``ResegmentCurveCommand``. Only a flat curve; ``rejected`` otherwise."""
    from core.curve_segments import ResegmentCurveCommand
    e, = _need(history.scene.mesh, "edge", [edge])
    before = history.revision
    history.execute(ResegmentCurveCommand(e, int(segments)))
    return _answer(history, before)


def repair_solid(history, placement):
    """Repair what has one right answer in a placement's (uid) mesh, shared
    by its copies: stray edges, duplicated faces, flat holes, partitions,
    reversed faces (the Solid Inspector's ``RepairSolidsCommand``). The
    counts come back as ``repaired``."""
    from plugins.solid_inspector import RepairSolidsCommand
    g, = _placements(history, [placement])
    cmd = RepairSolidsCommand([g.mesh])
    before = history.revision
    history.execute(cmd)
    out = _answer(history, before)
    out["repaired"] = dict(cmd.done)
    return out


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
    g, = _placements(history, [placement])
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
    g, = _placements(history, [placement])
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


def _material(history, name):
    mat = (getattr(history.scene, "materials", None) or {}).get(name)
    if mat is None:
        raise KeyError(f"no material called {name!r}")
    return mat


def rename_material(history, name, new_name):
    """Rename a registry material (its name is its key, §2) on every face
    and container that wears it: ``RestampMaterialCommand``. A name another
    material has is refused (that would merge two materials)."""
    from dataclasses import replace
    from core.history import RestampMaterialCommand
    mat = _material(history, name)
    new_name = str(new_name).strip()
    if not new_name:
        raise ValueError("a material needs a name")
    before = history.revision
    if new_name == name:
        return _answer(history, before, ok=False)
    if new_name in history.scene.materials:
        raise ValueError(f"there is already a material called {new_name!r}")
    history.execute(RestampMaterialCommand(name, replace(mat, name=new_name)))
    return _answer(history, before)


def edit_material(history, name, color=None, opacity=None, tile=None):
    """Change a registry material everywhere it is worn. ``color`` is
    ``[r, g, b]`` 0–1 (a textured material takes it as its tint),
    ``opacity`` 0–1 (1 = opaque), ``tile`` ``[width, height]`` in metres
    (textured only). Plain: ``RestampMaterialCommand``; textured:
    ``EditTexturedMaterialCommand`` (each face keeps its own placement)."""
    from dataclasses import replace
    from core.history import EditTexturedMaterialCommand, RestampMaterialCommand
    mat = _material(history, name)
    before = history.revision
    if mat.texture and (color is not None or tile is not None) and opacity is None:
        tex = mat.texture
        size = None
        if tile is not None:
            size = (float(tile[0]), float(tile[1]), float(tex.get("rot", 0.0)))
        tint = tuple(color) if color is not None else (tuple(mat.color) if mat.color is not None else None)
        history.execute(EditTexturedMaterialCommand(name, tint, size=size))
        return _answer(history, before)
    if tile is not None and not mat.texture:
        raise ValueError("only a textured material has a tile size")
    changes = {}
    if color is not None:
        changes["color"] = tuple(float(c) for c in color[:3])
    if opacity is not None:
        o = float(opacity)
        changes["opacity"] = None if o >= 0.999 else max(0.0, o)
    if not changes:
        return _answer(history, before, ok=False)
    history.execute(RestampMaterialCommand(name, replace(mat, **changes)))
    return _answer(history, before)


def duplicate_material(history, name, new_name=None):
    """A copy of a registry material under a new name (``"<name> copy"``,
    numbered when taken), worn by nothing yet: ``AddMaterialCommand``. The
    answer adds ``name``."""
    from copy import deepcopy
    from dataclasses import replace
    from core.material_commands import AddMaterialCommand
    mat = _material(history, name)
    taken = history.scene.materials
    if new_name is None:
        new_name, n = f"{name} copy", 2
        while new_name in taken:
            new_name, n = f"{name} copy {n}", n + 1
    elif new_name in taken:
        raise ValueError(f"there is already a material called {new_name!r}")
    before = history.revision
    history.execute(AddMaterialCommand(replace(deepcopy(mat), name=new_name)))
    out = _answer(history, before)
    out["name"] = new_name
    return out


def delete_material(history, name):
    """Remove a registry material; what wore it shows the default
    material: ``DeleteMaterialCommand``."""
    from core.material_commands import DeleteMaterialCommand
    _material(history, name)
    before = history.revision
    history.execute(DeleteMaterialCommand(name))
    return _answer(history, before)


def delete_placements(history, placements):
    """Delete placements (uids, nested included) with their geometry: one
    ``DeleteGroupCommand`` each, as one step."""
    from core.history import CompoundCommand, DeleteGroupCommand
    groups = _placements(history, placements)
    before = history.revision
    if groups:
        history.execute(CompoundCommand([DeleteGroupCommand(g) for g in groups]))
    return _answer(history, before)


def create_material(history, name=None, color=None, texture=None, opacity=None, pbr=None, finish=None,
                    reuse=True):
    """Add a material: a colour base (``color`` ``[r,g,b]`` 0–1) or an image
    base (``texture`` ``{"path", "sw", "sh"}``), with optional ``opacity``,
    ``pbr`` and ``finish``. With ``reuse`` and no ``pbr``, a material with
    the same plain recipe is answered instead (``reused`` true, no step).
    Unnamed, it is named after its colour (``Colour #A0703C``) or image.
    The answer adds ``name``."""
    from core.material_commands import AddMaterialCommand, find_material, fresh_name, material_for_paint
    from core.materials import Material
    if (color is None) == (texture is None):
        raise ValueError("a material has one base: a colour or an image")
    materials = history.scene.materials
    rgb = tuple(float(c) for c in color[:3]) if color is not None else None
    before = history.revision
    if reuse and pbr is None:
        found = find_material(materials, color=rgb, texture=texture, opacity=opacity)
        if found is not None and (name is None or name == found):
            return {"ok": True, "step": None, "error": None, "rejected": None, "name": found, "reused": True}
    if name is None:
        name = material_for_paint(history.scene, color=rgb, texture=texture, opacity=opacity).name
        name = fresh_name(materials, name)
    elif name in materials:
        raise ValueError(f"there is already a material called {name!r}")
    mat = Material(name, color=rgb, texture=dict(texture) if texture else None,
                   opacity=None if opacity is None or float(opacity) >= 0.999 else float(opacity),
                   pbr=dict(pbr) if pbr else None, finish=finish)
    history.execute(AddMaterialCommand(mat))
    out = _answer(history, before)
    out.update(name=name, reused=False)
    return out


def set_material_base(history, name, base, color=None, texture=None):
    """Switch a material's base between a colour (``base="color"``,
    ``color`` or its current colour) and an image (``base="image"``,
    ``texture`` ``{"path", "sw", "sh"}``, or the image it had). Its PBR
    values, opacity and finish are kept; what wears it follows
    (``RestampMaterialCommand``)."""
    from dataclasses import replace
    from core.history import RestampMaterialCommand
    mat = _material(history, name)
    before = history.revision
    if base == "color":
        rgb = tuple(float(c) for c in (color if color is not None else (mat.color or (0.8, 0.8, 0.8)))[:3])
        new = replace(mat, color=rgb, texture=None)
    elif base == "image":
        tex = dict(texture) if texture else (dict(mat.texture) if mat.texture else None)
        if not tex or not tex.get("path"):
            raise ValueError("an image base needs an image")
        tex.setdefault("sw", 1.0)
        tex.setdefault("sh", tex["sw"])
        new = replace(mat, texture=tex, color=mat.color if mat.pbr is not None else None)
    else:
        raise ValueError("base is 'color' or 'image'")
    if new == mat:
        return _answer(history, before, ok=False)
    history.execute(RestampMaterialCommand(name, new))
    return _answer(history, before)


def set_material_pbr(history, name, **channels):
    """Set PBR channels of a material, whatever its base: ``metallic``,
    ``roughness``, ``ao_strength`` (0–1), ``normal_scale`` (≥ 0), and maps
    ``metallic_map``, ``roughness_map``, ``normal_map``, ``ao_map`` (a
    path or ``{"path", "flip_y"?}``). ``None`` removes a channel; with none
    left the material has no PBR. ``RestampMaterialCommand``."""
    from dataclasses import replace
    from core.history import RestampMaterialCommand
    from core.material_commands import PBR_KEYS, PBR_MAPS
    mat = _material(history, name)
    unknown = sorted(set(channels) - set(PBR_KEYS))
    if unknown:
        raise ValueError(f"unknown PBR channel {unknown[0]!r}; one of {list(PBR_KEYS)}")
    pbr = dict(mat.pbr or {})
    for key, value in channels.items():
        if value is None:
            pbr.pop(key, None)
        elif key in PBR_MAPS:
            pbr[key] = {"path": value} if isinstance(value, str) else dict(value)
        else:
            v = float(value)
            pbr[key] = max(v, 0.0) if key == "normal_scale" else min(max(v, 0.0), 1.0)
    before = history.revision
    new = replace(mat, pbr=pbr or None)
    if new == mat:
        return _answer(history, before, ok=False)
    history.execute(RestampMaterialCommand(name, new))
    return _answer(history, before)


def name_unnamed_colours(history):
    """"Name these": each distinct colour painted without a material
    (older files) becomes a material, reusing one with the same recipe;
    the faces and containers take its name (``NameColoursCommand``). The
    answer adds ``named``, the number of colours."""
    from core.material_commands import NameColoursCommand
    cmd = NameColoursCommand()
    before = history.revision
    n = cmd.count(history.scene)
    if n:
        history.execute(cmd)
    out = _answer(history, before, ok=bool(n))
    out["named"] = n
    return out


def rename_component(history, placement, name):
    """Rename a component definition: every copy (placed or nested) and the
    template kept in the file take ``name``, as one step. ``placement`` is
    the uid of any copy, or of the kept template."""
    from core.history import CompoundCommand, RenameGroupCommand
    from core.kept_components import copies_of, kept_template
    scene = history.scene
    name = str(name).strip()
    if not name:
        raise ValueError("a component needs a name")
    kept = {getattr(t, "uid", None): t for t in getattr(scene, "kept_components", None) or ()}
    g = kept.get(placement) if isinstance(placement, str) else None
    if g is None:
        g, = _placements(history, [placement])
    targets = copies_of(scene, g.mesh)
    template = kept_template(scene, g.mesh)
    for extra in (template, g):
        if extra is not None and extra not in targets:
            targets.append(extra)
    before = history.revision
    targets = [t for t in targets if t.name != name]
    if targets:
        history.execute(CompoundCommand([RenameGroupCommand(t, name) for t in targets]))
    return _answer(history, before)


def undo(history):
    before = history.revision
    history.undo()
    return _answer(history, before)


def redo(history):
    before = history.revision
    history.redo()
    return _answer(history, before)


from core.deliveries import create_delivery, edit_delivery, delete_delivery, run_delivery


COMMANDS = {"patch_record_field": patch_record_field, "set_filing_rule": set_filing_rule,
            "create_delivery": create_delivery, "edit_delivery": edit_delivery,
            "delete_delivery": delete_delivery, "run_delivery": run_delivery, "follow_me": follow_me, "intersect_faces": intersect_faces,
            "add_edge": add_edge, "add_edges": add_edges,
            "divide_edges": divide_edges, "join_edges": join_edges,
            "move_vertices": move_vertices, "erase": erase,
            "reverse_faces": reverse_faces, "push_pull": push_pull, "orient_faces": orient_faces,
            "rebuild_cap": rebuild_cap, "solid_operation": solid_operation,
            "explode_group": explode_group,
            "split_into_pieces": split_into_pieces,
            "explode_view": explode_view, "purge_unused": purge_unused,
            "purge_debris": purge_debris, "undo": undo, "redo": redo,
            "hide": hide, "unhide": unhide, "lock": lock, "rename_placement": rename_placement,
            "unlink_placements": unlink_placements, "edit_loose_geometry": edit_loose_geometry, "file_placements": file_placements, "edit_structure_node": edit_structure_node, "edit_tag": edit_tag, "set_tag": set_tag, "set_tag_state": set_tag_state, "resegment_curve": resegment_curve, "repair_solid": repair_solid,
            "rename_material": rename_material, "edit_material": edit_material,
            "duplicate_material": duplicate_material, "delete_material": delete_material,
            "delete_placements": delete_placements, "rename_component": rename_component,
            "create_material": create_material, "set_material_base": set_material_base,
            "set_material_pbr": set_material_pbr, "name_unnamed_colours": name_unnamed_colours}


def run(history, name, /, **args):
    try:
        fn = COMMANDS[name]
    except KeyError:
        raise ValueError(f"unknown command {name!r}; one of {sorted(COMMANDS)}") from None
    return fn(history, **args)

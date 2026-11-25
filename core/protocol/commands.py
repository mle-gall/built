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

import ingetrazo_core
from ingetrazo_core import command_protocol
from ingetrazo_core.command_protocol import (hide, lock, make_unique, move_vertices, redo,
    rename_placement, rename_placements, set_tag, undo, unhide)
from core.history import Command
from core.protocol.records import patch_record_field, set_filing_rule
from core.protocol.keys import elements, from_json_key, store_of


def erase(history, edges=(), faces=()):
    """Erase edges and faces (keys) the Eraser's way
    (``EraseSelectionCommand``: a split line between coplanar faces
    dissolves, other edges take their faces)."""
    from core.history import EraseSelectionCommand
    mesh = history.scene.mesh
    es, fs = command_protocol.need(mesh, "edge", edges), command_protocol.need(mesh, "face", faces)
    return command_protocol.execute(history, EraseSelectionCommand(es, fs))


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
    from core.push_pull import PushPullOperation
    (f,) = command_protocol.need(history.scene.mesh, "face", [face])
    op = PushPullOperation()
    op.base_face = f
    from core.coordinate_frame import ZERO, choose_origin, mesh_frame
    st = store_of(history.scene.mesh)
    origin = choose_origin(st.face_positions(f.key)[0][0]) if st is not None else ZERO
    with mesh_frame(history.scene.mesh, origin):
        return _push_face(history, f, distance, keep_base, op)


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
        return command_protocol.answer(history, before, ok=False)
    tool._commit(_Headless(history))
    return command_protocol.answer(history, before)


def set_group_axes(history, placement, origin, red, green):
    """Re-express a placement's axes while keeping its geometry in place."""
    from ingetrazo_core import ToolMachine, Vec3
    from core.axes import frame_matrix
    from core.coordinate_frame import point_in_frame, ZERO
    from core.history import ChangeAxesCommand
    group, = command_protocol.resolve_placements(history, [placement])
    basis = ToolMachine.group_axes_basis(origin, red, green)
    if basis is None:
        raise ValueError("the red and green points must define a nondegenerate frame")
    local = point_in_frame(origin, getattr(store_of(history.scene.mesh), "frame_origin", ZERO))
    frame = frame_matrix(local, *(Vec3(*p) for p in basis))
    return command_protocol.execute(history, ChangeAxesCommand(group, frame))


def flip_selection(history, center, axis, copy=False, placements=(), faces=(), edges=()):
    """Mirror keyed selection across a plane through center with normal axis."""
    from math import isfinite
    from core.coordinate_frame import point_in_frame, ZERO
    from ingetrazo_core import Vec3
    from core.group import copy_group, transformed_attrs
    from core.mesh import Mesh
    from core.history import (AddEdgeCommand, AddFaceCommand, CompoundCommand, FlipGroupsCommand,
                              FlipVerticesCommand, InsertGroupCommand, mirror_matrix)
    if len(center) != 3 or len(axis) != 3 or not all(map(isfinite, (*center, *axis))):
        raise ValueError("flip needs a finite centre and plane normal")
    if Vec3(*axis).length() < 1e-12:
        raise ValueError("flip needs a nonzero plane normal")
    scene = history.scene
    mesh = scene.mesh
    centre = point_in_frame(center, getattr(store_of(mesh), "frame_origin", ZERO))
    n = Vec3(*axis).normalized()
    groups = command_protocol.resolve_placements(history, placements)
    faces, edges = command_protocol.need(mesh, "face", faces), command_protocol.need(mesh, "edge", edges)
    positions = {v.key: v.position for f in faces for v in f.loop}
    positions.update((v.key, v.position) for e in edges for v in (e.v0, e.v1))
    before = history.revision
    cmds: list = []
    if copy:
        from core.ifc_identity import copied_face_attrs
        copy_attrs = copied_face_attrs(scene)
        m = mirror_matrix(centre, n)
        for g in groups:
            clone = copy_group(g)
            cmds.append(InsertGroupCommand(clone))
            cmds.append(FlipGroupsCommand([clone], centre, n))
        id_map: dict[int, int] = {}
        for f in faces:
            # Mirrored loops are re-reversed so the copy faces OUT.
            loop = [m.map(Vec3(v)) for v in f.vertices][::-1]
            holes = [[m.map(Vec3(v)) for v in h][::-1]
                     for h in f.holes] or None
            cmds.append(AddFaceCommand(
                loop, holes=holes, auto=False,
                attrs=copy_attrs(transformed_attrs(f.attrs, m))))
        for e in edges:
            curve = getattr(e, "curve", None)
            if curve is not None and curve not in id_map:
                id_map[curve] = Mesh.next_curve_id()
            cmds.append(AddEdgeCommand(
                m.map(Vec3(e.a)), m.map(Vec3(e.b)),
                soft=getattr(e, "soft", False) or None,
                curve=id_map.get(curve)))
    else:
        if groups:
            cmds.append(FlipGroupsCommand(groups, centre, n))
        if positions:
            cmds.append(FlipVerticesCommand(
                list(positions.values()), centre, n, faces=faces))
    if cmds:
        history.execute(cmds[0] if len(cmds) == 1 else CompoundCommand(cmds))
    return command_protocol.answer(history, before)


def paste_clipboard(history, offset, *, clipboard):
    """Stamp the bound document clipboard with a model-space displacement."""
    import math
    from ingetrazo_core import Vec3
    from core.paste_ops import stamp_clipboard
    if len(offset) != 3 or not all(map(math.isfinite, offset)):
        raise ValueError("paste offset must be a finite vector")
    before = history.revision
    ok = clipboard and stamp_clipboard(history, clipboard, Vec3(*offset))
    return command_protocol.answer(history, before, ok)


def bevel_selection(history, radius, segments=8, profile=.5, measure="radius", corners="round",
                    shape=None, edges=(), vertices=(), _plan=None):
    """Round keyed edges or bevel keyed corners of the open mesh in a single step."""
    from math import isfinite
    from core.fillet import apply_fillet, plan_fillet
    from core.vertex_bevel import plan_vertex_bevel
    from core.history import SnapshotMutation
    from core.store_face_edit import StoreFaceMutation
    from core import bevel_profiles
    if bool(edges) == bool(vertices):
        raise ValueError("supply edges or vertices")
    if not isfinite(radius) or radius <= 0 or type(segments) is not int or segments < 1:
        raise ValueError("bevel needs a positive size and segment count")
    if not 0 < profile < 1 or measure not in ("radius", "offset", "width", "depth") or corners not in ("round", "cut"):
        raise ValueError("invalid bevel profile, measure or corner fill")
    if shape is not None:
        if vertices or (isinstance(shape, str) and shape not in ("steps", "supports", "cornice", "crown")):
            raise ValueError("unknown shape for this bevel")
        if not isinstance(shape, str):
            if not bevel_profiles.valid(shape):
                raise ValueError("invalid drawn bevel profile")
            shape = tuple(tuple(p) for p in shape)
    mesh = history.scene.mesh
    before = history.revision
    plan = _plan
    if plan is None:
        if vertices:
            plan = plan_vertex_bevel(mesh, command_protocol.need(mesh, "vertex", vertices), radius, segments, profile)
        else:
            plan = plan_fillet(mesh, command_protocol.need(mesh, "edge", edges), radius, segments, profile, measure, corners, shape)
    if isinstance(plan, str):
        return {**command_protocol.answer(history, before, ok=False), "error": plan}
    constructor = StoreFaceMutation if plan._store_plan is not None else SnapshotMutation
    history.execute(constructor(lambda scene: apply_fillet(scene.mesh, plan), mesh=mesh))
    return command_protocol.answer(history, before)


def paint_material(history, *, placement=None, mesh=None, faces=(), seed=None, material=None,
                   color=None, texture=None, opacity=None, default=False, back=False, plane=None,
                   _picked_placement=None):
    """Paint one placement or a list of keyed mesh faces in one history step."""
    from core.materials import Material
    from core.material_commands import material_for_paint
    from core.paint_ops import build_paint
    from core.protocol.keys import resolve_mesh
    from ingetrazo_core import Vec3
    if (placement is None) == (not faces):
        raise ValueError("paint_material needs a placement or faces")
    group, target_mesh, targets, clicked = None, None, [], None
    if placement is not None:
        # A viewport may supply a transient placement belonging to its own
        # editing context. Its UID must match the target sent by the machine.
        if _picked_placement is not None and _picked_placement.uid == placement:
            group = _picked_placement
        else:
            group, = command_protocol.resolve_placements(history, [placement])
    else:
        target_mesh = resolve_mesh(history.scene, mesh)
        targets = command_protocol.need(target_mesh, "face", faces)
        clicked = command_protocol.need(target_mesh, "face", [seed])[0] if seed is not None else targets[0]
    mat = None
    if isinstance(material, str):
        mat = _material(history, material)
        if color is None and texture is None:
            color, texture = mat.color, mat.texture
        if opacity is None:
            opacity = mat.opacity
    elif material is not None:
        mat = Material.from_dict(material)
    if color is None and texture is None and mat is not None:
        color, texture = mat.color, mat.texture
        if opacity is None:
            opacity = mat.opacity
    if color is not None and (len(color) != 3 or not all(0 <= c <= 1 for c in color)):
        raise ValueError("paint colour needs three components from zero to one")
    if opacity is not None and not 0 <= opacity <= 1:
        raise ValueError("paint opacity must be between zero and one")
    if color is None and texture is None and not default:
        raise ValueError("paint_material needs a colour, texture, or default")
    if mat is None and not default:
        mat = material_for_paint(history.scene, color=color, texture=texture, opacity=opacity)
    source_plane = (Vec3(*plane[0]), float(plane[1])) if plane is not None else None
    cmd = build_paint(target_mesh, targets, clicked, group, back, default,
                      color, texture, opacity, mat, source_plane)
    return command_protocol.execute(history, cmd)


def offset_outline(history, distance, face=None, points=None, normal=None, closed=False, eye=None):
    """Offset one keyed face or a coplanar edge path, in model coordinates."""
    import math
    from core.coordinate_frame import ZERO, choose_origin, mesh_frame, point_in_frame
    from core.edits import build_add_edges
    from core.offset import build_offset_face, OffsetHoleCollision, offset_chain
    from core.topology import offset_loop
    from ingetrazo_core import Vec3

    if (face is None) == (points is None):
        raise ValueError("offset_outline needs one face or point path")
    if not math.isfinite(distance) or distance == 0:
        raise ValueError("offset distance must be finite and nonzero")
    mesh = history.scene.mesh
    store = store_of(mesh)
    before = history.revision
    if face is not None:
        (target,) = command_protocol.need(mesh, "face", [face])
        origin = choose_origin(store.face_positions(target.key)[0][0]) if store is not None else ZERO
        with mesh_frame(mesh, origin):
            try:
                cmd = build_offset_face(mesh, target, distance)
            except OffsetHoleCollision:
                return {**command_protocol.answer(history, before, False), "error": "hole"}
            if cmd is None:
                return {**command_protocol.answer(history, before, False), "error": "collapsed_face"}
            history.execute(cmd)
    else:
        if len(points) < (3 if closed else 2) or normal is None:
            raise ValueError("an offset path needs points and a plane normal")
        if not all(len(p) == 3 and all(math.isfinite(v) for v in p) for p in [normal, *points]):
            raise ValueError("offset path vectors must contain three finite coordinates")
        if Vec3(*normal).lengthSquared() < 1e-18:
            raise ValueError("offset path normal must be nonzero")
        origin = choose_origin(points[0]) if store is not None else ZERO
        with mesh_frame(mesh, origin):
            path = [point_in_frame(p, origin) for p in points]
            shifted = (offset_loop if closed else offset_chain)(path, Vec3(*normal), distance)
            if shifted is None:
                return {**command_protocol.answer(history, before, False), "error": "collapsed_chain"}
            segments = list(zip(shifted, shifted[1:] + (shifted[:1] if closed else [])))
            history.execute(build_add_edges(history.scene, segments,
                            eye=point_in_frame(eye, origin) if eye is not None else None))
    return command_protocol.answer(history, before)


def follow_me(history, face, path, closed=False):
    from core.sweep import build_sweep_face
    points = list(path)
    from core.i18n import tr
    failed = tr("Follow Me: could not sweep along that path (reversal or degenerate segment)")
    cmd = build_sweep_face(history.scene.mesh, from_json_key(face), points, closed, failed)
    return command_protocol.execute(history, cmd)


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
    return command_protocol.answer(history, before)


def rebuild_cap(history, face):
    """Rebuild the plane of a seed face with the direct cap planner."""
    from core.history import CommandRejected, SnapshotMutation
    seed, = command_protocol.need(history.scene.mesh, "face", [face])
    origin, normal = seed.centroid(), seed.normal()

    def rebuild(scene):
        if not scene.mesh.rebuild_cap_plane(origin, normal):
            raise CommandRejected("Cap already matches its plane")

    return command_protocol.execute(history, SnapshotMutation(rebuild))


def reverse_faces(history, faces):
    from core.history import FlipFacesCommand
    selected = command_protocol.need(history.scene.mesh, "face", faces)
    before = history.revision
    if selected:
        history.execute(FlipFacesCommand(selected))
    return command_protocol.answer(history, before)


def orient_faces(history, face):
    """Orient the connected sheet to the seed face; all keys survive."""
    from core.orient_faces import faces_to_flip
    from core.history import FlipFacesCommand
    seed, = command_protocol.need(history.scene.mesh, "face", [face])
    flip = faces_to_flip(seed)
    before = history.revision
    if flip:
        history.execute(FlipFacesCommand(flip))
    return command_protocol.answer(history, before)


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
    from core.solids import SolidError
    command = SolidOperationCommand(operation, inputs)
    try:
        command.prepare()
    except SolidError as error:
        return {**command_protocol.answer(history, before, False), "error": str(error)}
    history.execute(command)
    return command_protocol.answer(history, before)


def add_edge(history, a, b=None, soft=None, tag=None, eye=None, offset=None):
    """One drawn segment, ``a`` to ``b``. ``offset`` instead of (or with) ``b``:
    the end is the stored position at ``a`` plus ``offset`` exactly (a typed
    length). ``eye`` (the camera position, as the Line tool passes it)
    decides which way a new vertical face looks."""
    if tag is not None and history.scene.layer_by_key(tag) is None:
        raise KeyError(f"no tag with key {tag!r}")
    if b is None:
        if offset is None:
            raise ValueError("add_edge needs b or offset")
        b = [float(x) + float(d) for x, d in zip(a, offset)]
    return _draw_segments(history, [(a, b)], single=True, soft=soft, tag=tag, eye=eye,
                          offset=offset)


def add_edges(history, segments):
    return _draw_segments(history, list(segments))


def _draw_segments(history, segments, single=False, **options):
    from core.coordinate_frame import ZERO, choose_origin, mesh_frame, point_in_frame
    from core.edits import build_add_edge, build_add_edges
    from core.coordinate_frame import drawing_point, planned_points
    offset = options.pop("offset", None)
    mesh = history.scene.mesh
    st = store_of(mesh)
    origin = choose_origin(segments[0][0]) if st is not None and segments else ZERO
    local = [(point_in_frame(a, origin), point_in_frame(b, origin)) for a, b in segments]
    if options.get("eye") is not None:
        options["eye"] = point_in_frame(options["eye"], origin)
    before = history.revision
    with mesh_frame(mesh, origin):
        cmd = (build_add_edge(history.scene, *local[0], **options) if single else
               build_add_edges(history.scene, local))
        exact = {p.toTuple(): tuple(v-o for v, o in zip(raw, origin))
                 for pair, args in zip(local, segments) for p, raw in zip(pair, args)
                 } if st is not None else {}
        if offset is not None and st is not None:
            start = drawing_point(mesh, local[0][0])
            exact = {local[0][0].toTuple(): tuple(start),
                     local[0][1].toTuple(): tuple(start + [float(d) for d in offset])}
        with planned_points(mesh, exact):
            history.execute(cmd)
    return command_protocol.answer(history, before)


def add_face(history, outer=None, holes=(), anchor=None, offsets=None, curve=False):
    """A face over a ring of points, with its edges: the Rectangle's commit.
    ``outer`` the ring (open mesh coordinates), or ``anchor`` plus
    ``offsets``: the ring is the stored position at ``anchor`` plus each
    offset exactly (typed sizes). ``holes`` are inner rings. The plane it
    lands on is rebuilt, so a ring drawn over a face fills its hole (#538).
    ``curve``: the ring is one closed curve (the Circle's and the Polygon's
    commit), tagged so, its plane's faces rebuilt as
    ``core.arrangement.curve_extras`` says. One step."""
    from core.coordinate_frame import (ZERO, choose_origin, drawing_point, mesh_frame,
                                       planned_points, point_in_frame)
    from core.edits import build_add_edges
    from core.history import AddFaceCommand
    if outer is None:
        if anchor is None or offsets is None:
            raise ValueError("add_face needs outer, or anchor and offsets")
        outer = [[float(a) + float(d) for a, d in zip(anchor, off)] for off in offsets]
    outer = [list(map(float, p)) for p in outer]
    if len(outer) < 3:
        raise ValueError("a face needs three points")
    holes = [[list(map(float, p)) for p in ring] for ring in (holes or ())]
    mesh = history.scene.mesh
    st = store_of(mesh)
    origin = choose_origin(outer[0]) if st is not None else ZERO
    ring = [point_in_frame(p, origin) for p in outer]
    inner = [[point_in_frame(p, origin) for p in h] for h in holes]
    segments = [(r[i], r[(i + 1) % len(r)]) for r in [ring, *inner] for i in range(len(r))]
    before = history.revision
    with mesh_frame(mesh, origin):
        exact = {}
        if st is not None:
            if offsets is not None:
                start = drawing_point(mesh, point_in_frame(anchor, origin))
                exact = {p.toTuple(): tuple(start + [float(d) for d in off])
                         for p, off in zip(ring, offsets)}
            else:
                exact = {p.toTuple(): tuple(v - o for v, o in zip(raw, origin))
                         for p, raw in zip(ring, outer)}
        if curve:
            from core.arrangement import curve_extras
            cmd = build_add_edges(history.scene, segments, detect_faces=False,
                                  extra=curve_extras(history.scene, ring))
        else:
            cmd = build_add_edges(history.scene, segments, detect_faces=False,
                                  extra=[AddFaceCommand(list(ring), holes=inner or None)],
                                  plane_rebuild=True)
        with planned_points(mesh, exact):
            history.execute(cmd)
    return command_protocol.answer(history, before)


def add_arc(history, points, center=None, trim=None):
    """An arc polyline (open mesh coordinates): the Arc tools' and
    Freehand's commit (``core.arrangement.arc_command``). ``center`` closes
    the wedge (the Pie); ``trim`` is ``[[a, b], ...]``, edges to erase once
    the arc is in (a fillet's corner stubs). One step."""
    from core.arrangement import arc_command
    from core.coordinate_frame import ZERO, choose_origin, mesh_frame, planned_points, point_in_frame
    pts = [list(map(float, p)) for p in points]
    if len(pts) < 2:
        raise ValueError("an arc needs two points")
    mesh = history.scene.mesh
    st = store_of(mesh)
    origin = choose_origin(pts[0]) if st is not None else ZERO
    local = [point_in_frame(p, origin) for p in pts]
    close = point_in_frame(center, origin) if center is not None else None
    cuts = [(point_in_frame(a, origin), point_in_frame(b, origin)) for a, b in (trim or ())]
    before = history.revision
    with mesh_frame(mesh, origin):
        cmd = arc_command(history.scene, local, close, cuts or None)
        exact = ({p.toTuple(): tuple(v - o for v, o in zip(raw, origin)) for p, raw in zip(local, pts)}
                 if st is not None else {})
        with planned_points(mesh, exact):
            history.execute(cmd)
    return command_protocol.answer(history, before)

def add_guide(history, point, direction=None, origin=None):
    """Construction line or point, in open mesh coordinates."""
    from ingetrazo_core import Vec3
    from core.coordinate_frame import point_in_frame, ZERO
    from core.guide import Guide
    from core.history import AddGuideCommand
    store = store_of(history.scene.mesh)
    frame = store.frame_origin if store is not None else ZERO
    guide = Guide(point_in_frame(point, frame), Vec3(*direction) if direction is not None else None,
                  point_in_frame(origin, frame) if origin is not None else None)
    before = history.revision
    if guide.is_finite() and (direction is None or guide.is_line):
        history.execute(AddGuideCommand(guide))
    return command_protocol.answer(history, before)


def change_guide(history, index, direction):
    """Re-aim an existing construction line, preserving its identity."""
    from core.guide import Guide
    from core.history import ChangeGuideCommand
    from ingetrazo_core import Vec3
    import math
    if type(index) is not int or not 0 <= index < len(history.scene.guides):
        raise ValueError("unknown guide index")
    guide = history.scene.guides[index]
    if not isinstance(guide, Guide) or not guide.is_line:
        raise ValueError("this guide is not a line")
    if len(direction) != 3 or not all(map(math.isfinite, direction)) or Vec3(*direction).length() < 1e-12:
        raise ValueError("guide direction must be a finite nonzero vector")
    return command_protocol.execute(history, ChangeGuideCommand(guide, Vec3(*direction)))


def add_guide_circle(history, center, normal, radius):
    """An analytic guide circle (the Circle tool with Ctrl): ``center``
    (open mesh coordinates), the unit ``normal`` of its plane, ``radius``.
    Nothing when the radius is zero or the circle is not finite. One step."""
    from ingetrazo_core import Vec3
    from core.coordinate_frame import point_in_frame
    from core.guide import GuideCircle
    from core.history import AddGuideCommand
    before = history.revision
    radius = float(radius)
    if radius < 1e-9:
        return command_protocol.answer(history, before)
    st = store_of(history.scene.mesh)
    origin = getattr(st, "frame_origin", None) if st is not None else None
    c = point_in_frame(center, origin) if origin is not None and any(origin) else Vec3(*map(float, center))
    guide = GuideCircle(c, Vec3(*map(float, normal)), radius)
    if guide.is_finite():
        history.execute(AddGuideCommand(guide))
    return command_protocol.answer(history, before)

def move_placements(history, placements, delta):
    """Move placements (uids, nested included) by ``delta`` ``[x, y, z]``:
    one ``MoveGroupCommand`` each, one step."""
    from ingetrazo_core import Vec3
    from core.history import CompoundCommand, MoveGroupCommand
    groups = command_protocol.resolve_placements(history, placements)
    d = Vec3(*map(float, delta))
    before = history.revision
    cmds = [MoveGroupCommand(g, d) for g in groups]
    if cmds:
        history.execute(cmds[0] if len(cmds) == 1 else CompoundCommand(cmds))
    return command_protocol.answer(history, before)


def copy_placements(history, placements, delta, count=1, mode="x"):
    """Copies of placements (uids, nested included): ``count`` copies at
    multiples of ``delta`` (``mode`` ``"x"``, an external array) or
    dividing it (``"/"``, internal), one step. The answer adds
    ``placements``: the copies' uids, copy by copy."""
    from ingetrazo_core import Vec3
    from core.group import copy_group
    from core.history import CompoundCommand, InsertGroupCommand
    groups = command_protocol.resolve_placements(history, placements)
    count = int(count)
    if count < 1 or mode not in ("x", "/"):
        raise ValueError("count is at least 1 and mode is 'x' or '/'")
    d = Vec3(*map(float, delta))
    steps = ([d * (k / float(count)) for k in range(1, count + 1)] if mode == "/"
             else [d * float(k) for k in range(1, count + 1)])
    clones = [copy_group(g, step) for step in steps for g in groups]
    before = history.revision
    cmds = [InsertGroupCommand(c) for c in clones]
    if cmds:
        history.execute(cmds[0] if len(cmds) == 1 else CompoundCommand(cmds))
    answer = command_protocol.answer(history, before)
    answer["placements"] = [c.uid for c in clones] if answer["ok"] else []
    return answer


def rotate_selection(history, center, axis, angles, copy=False, placements=(), vertices=(),
                     edges=(), faces=(), images=(), guides=(), sections=()):
    """Rotate named entities, or stamp copies at the supplied angles, in one step."""
    from math import isfinite
    from core.coordinate_frame import point_in_frame
    from core.rotate_ops import rotation_command
    from ingetrazo_core import Vec3
    scene = history.scene
    if len(center) != 3 or len(axis) != 3 or not all(map(isfinite, (*center, *axis, *angles))):
        raise ValueError("finite centre, axis and angles required")
    if Vec3(*axis).length() < 1e-12 or not angles or (not copy and len(angles) != 1):
        raise ValueError("a nonzero axis and one angle (or one per copy) required")

    def records(items, indices):
        if any(type(i) is not int or not 0 <= i < len(items) for i in indices):
            raise KeyError("unknown record index")
        return [items[i] for i in dict.fromkeys(indices)]

    mesh = scene.mesh
    origin = getattr(store_of(mesh), "frame_origin", None)
    start = point_in_frame(center, origin) if origin is not None else Vec3(*center)
    cmd = rotation_command(start, Vec3(*axis), copy, command_protocol.resolve_placements(history, placements),
                           [v.position for v in command_protocol.need(mesh, "vertex", vertices)],
                           command_protocol.need(mesh, "face", faces), command_protocol.need(mesh, "edge", edges),
                           records(scene.guides, guides), records(scene.section_planes, sections),
                           records(scene.image_planes, images), angles)
    return command_protocol.execute(history, cmd)


def place_group(history, template, basis, offset, point=None, attachment=None, _pending=None):
    """Stamp a named placement template, or the active tool's detached import."""
    from ingetrazo_core import Matrix4, Vec3
    from core.annotation_anchor import Attachment
    from core.group import carry_axes, copy_group, _remap_uvws
    from core.history import InsertGroupCommand
    if _pending is not None and _pending.uid != template:
        raise KeyError("unknown placement template")
    group = _pending if _pending is not None else copy_group(command_protocol.resolve_placements(history, [template])[0])
    x, y, z = basis
    shift = Vec3(*offset)
    matrix = Matrix4(x[0], y[0], z[0], shift.x(), x[1], y[1], z[1], shift.y(),
                     x[2], y[2], z[2], shift.z(), 0., 0., 0., 1.)
    if isinstance(attachment, dict):
        attachment = Attachment.restore(history.scene, attachment, Vec3(*(point or offset)))
    old_xform, old_axes = group.xform, group.axes
    state = group.mesh.capture_state() if group.xform is None else None
    before = history.revision
    try:
        if group.xform is not None:
            group.xform = matrix * group.xform
        else:
            for vertex in list(group.mesh.vertices):
                delta = matrix.map(vertex.position) - vertex.position
                if delta.length() > 1e-9:
                    group.mesh.move_vertex(vertex, delta)
            _remap_uvws(group.mesh, matrix)
            carry_axes(group, matrix)
        history.execute(InsertGroupCommand(group, attachment=attachment))
    finally:
        if history.revision == before:
            group.xform, group.axes = old_xform, old_axes
            if state is not None:
                group.mesh.restore_state(state)
    return command_protocol.answer(history, before)


def erase_stroke(history, hide=False, edges=(), placements=(), guides=(), dimensions=(),
                 geo_paths=(), text_labels=()):
    from core.history import (CompoundCommand, EraseSelectionCommand, HideEdgesCommand, HideCommand,
                              DeleteGroupCommand, DeleteGuidesCommand, DeleteDimensionsCommand,
                              DeleteGeoPathsCommand, DeleteTextLabelsCommand)
    scene = history.scene
    es = command_protocol.need(scene.mesh, "edge", edges)
    groups = command_protocol.resolve_placements(history, placements)
    commands = []
    if hide:
        es = [e for e in es if not e.hidden]
        groups = [g for g in groups if not g.hidden]
        if es:
            commands.append(HideEdgesCommand(es, hidden=True))
        if groups:
            commands.append(HideCommand(groups, hidden=True))
    else:
        if es:
            commands.append(EraseSelectionCommand(es, []))
        commands.extend(DeleteGroupCommand(g) for g in groups)
        for indices, items, delete in ((guides, scene.guides, DeleteGuidesCommand),
                (dimensions, scene.dimensions, DeleteDimensionsCommand),
                (geo_paths, scene.geo_paths, DeleteGeoPathsCommand),
                (text_labels, scene.text_labels, DeleteTextLabelsCommand)):
            if any(type(i) is not int or not 0 <= i < len(items) for i in indices):
                raise KeyError("unknown stroke target")
            if indices:
                commands.append(delete([items[i] for i in dict.fromkeys(indices)]))
    before = history.revision
    if commands:
        history.execute(commands[0] if len(commands) == 1 else CompoundCommand(commands))
    return command_protocol.answer(history, before)


def add_geo_path(history, points, closed=False):
    from ingetrazo_core import Vec3
    from georef.geopath import GeoPath
    from core.history import AddGeoPathCommand
    if len(points) < 2:
        raise ValueError("expected at least two path nodes")
    return command_protocol.execute(history, AddGeoPathCommand(GeoPath([Vec3(*p) for p in points], closed=closed)))


def move_geo_path_node(history, path, node, point):
    from ingetrazo_core import Vec3
    from core.history import MoveGeoPathNodeCommand
    paths = history.scene.geo_paths
    if not 0 <= path < len(paths) or not 0 <= node < len(paths[path].points):
        raise KeyError("unknown path node")
    return command_protocol.execute(history, MoveGeoPathNodeCommand(paths[path], node, Vec3(*point)))


def add_text_label(history, anchor, offset, text, attachment=None):
    from ingetrazo_core import Vec3
    from core.annotation_anchor import Attachment
    from core.history import AddTextLabelCommand
    from core.textlabel import TextLabel
    point = Vec3(*anchor)
    label = TextLabel(point, Vec3(*offset), text, attachment=Attachment.restore(history.scene, attachment, point))
    return command_protocol.execute(history, AddTextLabelCommand(label))


def add_dimension(history, a, b, offset, axis=None, attachments=(None, None)):
    from ingetrazo_core import Vec3
    from core.annotation_anchor import Attachment
    from core.dimension import Dimension
    from core.history import AddDimensionCommand
    dim = Dimension(Vec3(*a), Vec3(*b), Vec3(*offset), axis=axis)
    dim.attachment_a, dim.attachment_b = (Attachment.restore(history.scene, raw, pos)
                                          for raw, pos in zip(attachments, (dim.a, dim.b), strict=True))
    return command_protocol.execute(history, AddDimensionCommand(dim))


def place_section_plane(history, point, normal, name=None, symbol=None):
    from ingetrazo_core import Vec3
    from core.i18n import tr
    from core.history import PlaceSectionPlaneCommand
    from core.section import SectionPlane
    planes = history.scene.section_planes
    plane = SectionPlane(Vec3(*point), Vec3(*normal),
                         name=tr("Section {n}", n=len(planes) + 1) if name is None else name,
                         symbol=ingetrazo_core.section_next_symbol(planes) if symbol is None else symbol)
    return command_protocol.execute(history, PlaceSectionPlaneCommand(plane))


def set_north(history, degrees):
    import math
    from core.history import SetNorthCommand
    from core.sun import wrap_north_deg, scene_north_deg
    if not math.isfinite(degrees):
        raise ValueError("expected a finite north angle")
    deg = wrap_north_deg(degrees)
    before = history.revision
    if abs(deg - scene_north_deg(history.scene)) > 1e-9:
        history.execute(SetNorthCommand(deg))
    return command_protocol.answer(history, before)


def add_image_plane(history, path, origin, u, v, aspect=1.0, name=""):
    from ingetrazo_core import Vec3
    from core.history import AddImagePlaneCommand
    from core.image_plane import ImagePlane
    before = history.revision
    plane = ImagePlane(path, Vec3(*origin), Vec3(*u), Vec3(*v), aspect=aspect, name=name)
    history.execute(AddImagePlaneCommand(plane))
    return command_protocol.answer(history, before)


def set_group_scale(history, placement, factors):
    """Set absolute scale along a placement's local axes."""
    import math
    from core.group_scale import scale_command
    values = tuple(float(v) for v in factors)
    if len(values) != 3 or not all(map(math.isfinite, values)):
        raise ValueError("expected three finite scale factors")
    group, = command_protocol.resolve_placements(history, [placement])
    before = history.revision
    command = scale_command(group, values)
    if command is not None:
        history.execute(command)
    return command_protocol.answer(history, before)


def scale_selection(history, anchor, factors, vertices=(), placements=(), images=(), axes=None):
    """Scale loose vertex keys, placement uids/paths and reference image indices
    about an open-mesh point. Optional axes orient the grip box. One undo step."""
    from math import isfinite
    from core.coordinate_frame import point_in_frame
    from core.history import (CompoundCommand, ScaleGroupCommand, ScaleImagePlanesCommand,
                              ScaleVerticesCommand)
    from ingetrazo_core import Vec3
    factors = tuple(map(float, factors))
    if len(factors) != 3 or any(not isfinite(f) or f == 0.0 for f in factors):
        raise ValueError("three finite, nonzero scale factors required")
    if len(anchor) != 3 or not all(map(isfinite, anchor)):
        raise ValueError("three finite anchor coordinates required")
    if axes is not None:
        if len(axes) != 3 or any(len(a) != 3 or not all(map(isfinite, a)) for a in axes):
            raise ValueError("three finite box axes required")
        axes = tuple(Vec3(*a) for a in axes)
    mesh = history.scene.mesh
    origin = getattr(store_of(mesh), "frame_origin", None)
    center = point_in_frame(anchor, origin) if origin is not None else Vec3(*anchor)
    groups = command_protocol.resolve_placements(history, placements)
    vs = command_protocol.need(mesh, "vertex", dict.fromkeys(vertices))
    planes = history.scene.image_planes
    if any(type(i) is not int or not 0 <= i < len(planes) for i in images):
        raise KeyError("unknown image plane")
    refs = [planes[i] for i in dict.fromkeys(images)]
    packed = factors[0] if factors[0] == factors[1] == factors[2] else factors
    cmds = [ScaleGroupCommand(g, center, packed, axes) for g in groups]
    if vs:
        cmds.append(ScaleVerticesCommand([v.position for v in vs], center, packed, axes))
    if refs:
        cmds.append(ScaleImagePlanesCommand(refs, center, packed, axes))
    before = history.revision
    if cmds:
        history.execute(cmds[0] if len(cmds) == 1 else CompoundCommand(cmds))
    return command_protocol.answer(history, before)


def move_selection(history, delta, placements=None, vertices=None, edges=None,
                   faces=None, copy=False, count=1, mode="x", _plan=None):
    """Move (or copy) what is named — placements by uid, loose vertices,
    edges and faces by key — or, with none named, the scene's selection
    (placements, loose geometry, guides, labels, dimensions, section
    planes, reference images; a dimension anchored to what moves rides
    along): the Move tool's commit (``core.move_ops``). A copy
    lays ``count`` copies at multiples of ``delta`` (``mode`` ``"x"``) or
    dividing it (``"/"``) and selects them. One step."""
    from core.history import Command
    if _plan is not None and not isinstance(_plan, Command):
        raise TypeError("a prepared move must be a history command")
    before = history.revision
    command = _plan if _plan is not None else _move_selection_command(
        history, delta, placements, vertices, edges, faces, copy, count, mode)
    if command is not None:
        history.execute(command)
    return command_protocol.answer(history, before)


def _move_selection_command(history, delta, placements, vertices, edges, faces, copy, count, mode):
    from ingetrazo_core import Vec3
    from core.dimension import Dimension
    from core.group import Group
    from core.guide import is_guide
    from core.history import CompoundCommand
    from core.mesh import Edge, Face
    from core.move_ops import copy_builder, move_commands
    from core.section import SectionPlane
    from core.textlabel import TextLabel
    mesh = history.scene.mesh
    named = any(x is not None for x in (placements, vertices, edges, faces))
    if named:
        groups = command_protocol.resolve_placements(history, placements or [])
        vs = command_protocol.need(mesh, "vertex", vertices or [])
        es = command_protocol.need(mesh, "edge", edges or [])
        fs = command_protocol.need(mesh, "face", faces or [])
        rest = []
    else:
        sel = list(history.scene.selection)
        groups = [e for e in sel if isinstance(e, Group)]
        vs = []
        es = [e for e in sel if isinstance(e, Edge)]
        fs = [e for e in sel if isinstance(e, Face)]
        rest = sel
    labels = [e for e in rest if isinstance(e, TextLabel)]
    guides = [e for e in rest if is_guide(e)]
    splanes = [e for e in rest if isinstance(e, SectionPlane)]
    images = [e for e in rest if type(e).__name__ == "ImagePlane"]
    d = Vec3(*map(float, delta))
    if copy:
        count = int(count)
        if count < 1 or mode not in ("x", "/"):
            raise ValueError("count is at least 1 and mode is 'x' or '/'")
        steps = ([d * (k / float(count)) for k in range(1, count + 1)] if mode == "/"
                 else [d * float(k) for k in range(1, count + 1)])
        return copy_builder(history.scene, groups, fs, es, guides)(steps)
    from core.topology import _key
    seen, positions = set(), []
    for p in [v.position for v in vs] + [p for e in es for p in (e.a, e.b)] + \
            [p for f in fs for p in f.vertices]:
        key = _key(p)
        if key not in seen:
            seen.add(key)
            positions.append(Vec3(p))
    # Selected dimensions slide or move rigidly; those anchored to what
    # moves ride along with their vertices (core.move_ops.dimension_modes).
    from core.move_ops import dimension_modes
    verts = [v for v in (mesh.vertex_at(p) for p in positions) if v is not None]
    dims = dimension_modes([e for e in rest if isinstance(e, Dimension)], groups, verts)
    cmds = move_commands(d, groups=groups, positions=positions, labels=labels,
                         guides=guides, dims=dims, splanes=splanes, images=images)
    if cmds:
        return cmds[0] if len(cmds) == 1 else CompoundCommand(cmds)


def _edge_edit(history, edges, edit, *args):
    from core.history import SnapshotMutation
    selected = command_protocol.need(history.scene.mesh, "edge", edges)
    return command_protocol.execute(history, SnapshotMutation(lambda sc: edit(sc.mesh, selected, *args)))


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


def paint_placements(history, placements, paint, material=None):
    """Paint placements (uids, nested included) as a whole with one paint
    dict, or unpaint them with ``paint`` None, as one step. Faces painted
    themselves keep their own paint. ``material`` (a ``Material`` the
    paint names) joins the registry in the same step when it is not there."""
    from core.history import CompoundCommand, SetFaceMaterialTagCommand
    from ingetrazo_core import NativeCommand
    gs = [g for g in command_protocol.resolve_placements(history, placements) if g.material != (paint or None)]
    parts = []
    if material is not None and history.scene.materials.get(material.name) is None:
        parts.append(SetFaceMaterialTagCommand([], material.name, material))
    if gs:
        parts.append(NativeCommand.paint(gs, paint))
    before = history.revision
    if parts:
        history.execute(parts[0] if len(parts) == 1 else CompoundCommand(parts))
    return command_protocol.answer(history, before)


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


class _ReversibleSceneEdit(Command):
    def __init__(self, edit):
        self.edit = edit

    def do(self, scene):
        self.edit.apply(scene, True)
        scene.version += 1

    def undo(self, scene):
        self.edit.apply(scene, False)
        scene.version += 1


class LinkSubsetCommand(_ReversibleSceneEdit):
    pass


class SetLocationCommand(_ReversibleSceneEdit):
    history_name = "Set project location"
    keeps_rows = True


def set_location(history, lat, lon):
    from ingetrazo_core import LocationEdit
    return command_protocol.execute(history, SetLocationCommand(LocationEdit(history.scene, lat, lon)))


def edit_link_subset(history, placement, text):
    from ingetrazo_core import LinkSubset
    group, = command_protocol.resolve_placements(history, [placement])
    edit = LinkSubset.prepare(group, history.scene, text)
    before = history.revision
    if edit is not None:
        history.execute(LinkSubsetCommand(edit))
    return command_protocol.answer(history, before)


def reset_link_tags(history, path):
    from ingetrazo_core import reset_source_tags
    command = reset_source_tags(history.scene, path)
    return command_protocol.execute(history, command)


def unlink_placements(history, placements):
    from core.history import UnlinkCommand
    command = UnlinkCommand(command_protocol.resolve_placements(history, placements))
    before = history.revision
    if command.groups:
        history.execute(command)
    return command_protocol.answer(history, before)


def edit_section_plane(history, uid, field, value=""):
    from ingetrazo_core import section_edit
    command = section_edit(history.scene, uid, field, value)
    return command_protocol.execute(history, command)


def edit_loose_geometry(history, field):
    from ingetrazo_core import loose_edit
    return command_protocol.execute(history, loose_edit(history.scene, field))


def promote_tag(history, tag, ifc_class):
    from ingetrazo_core import promote_tag as plan
    return command_protocol.execute(history, plan(history.scene, tag, ifc_class))


def file_placements(history, placements, node):
    from core import structure
    return command_protocol.execute(history, structure.file(history.scene, placements, node))


def edit_assembly_part(history, placement, child=None, visible=None, name=None):
    from ingetrazo_core import plan_part_edit
    group, = command_protocol.resolve_placements(history, [placement])
    before = history.revision
    command = plan_part_edit(history.scene, group, child, visible, name)
    if command is not None:
        history.execute(command)
    return command_protocol.answer(history, before)


def edit_visibility_set(history, verb, key="", name=""):
    from ingetrazo_core import visibility_set_command
    before = history.revision
    command = visibility_set_command(history.scene, verb, key, name)
    if command is not None:
        history.execute(command)
    return command_protocol.answer(history, before)


def edit_saved_scene(history, verb, **args):
    from ingetrazo_core import plan_scene_command
    before = history.revision
    command = plan_scene_command(history.scene, verb, **args)
    if command is None:
        return {"ok": False, "step": None, "error": None, "rejected": None}
    history.execute(command)
    result = command_protocol.answer(history, before)
    if result["ok"] and verb in ("add", "duplicate"):
        result["index"] = history.scene.saved_views.index(command.view)
    return result


def edit_structure_node(history, node, field, value):
    from ingetrazo_core import plan_node_edit
    return command_protocol.execute(history, plan_node_edit(history.scene, node, field, value))


def edit_tag(history, tags, field, value):
    edit = history.scene._root._tag_edit(tags, field, value)
    before = history.revision
    if edit is not None:
        history.execute(EditTagCommand(edit))
    return command_protocol.answer(history, before)


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
        self.before = ingetrazo_core.tag_edits.layer_state(scene)
        self.links_before = deepcopy({k: e.get("overrides") for k, e in links.registry_view(scene).items()
                                      if isinstance(e, dict)})
        if self.visible is not None:
            for key in self.folders:
                scene.folder_visibility[key] = bool(self.visible)
            ingetrazo_core.tag_edits.set_visible(scene, self.tags, self.visible)
            for key in self.tags:
                links.note_visibility(scene, key, bool(self.visible))
            for entity in list(scene.selection):
                if not scene.entity_visible(entity):
                    scene.selection.discard(entity)
        if self.locked is not None:
            ingetrazo_core.tag_edits.set_locked(scene, self.tags, self.locked)
        scene.version += 1

    def undo(self, scene):
        from core import layers as L, links
        ingetrazo_core.tag_edits.apply_layer_state(scene, self.before)
        for key, entry in links.registry_view(scene).items():
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
    return command_protocol.execute(history, cmd)


def resegment_curve(history, edge, segments):
    """Rebuild the regular curve (circle, polygon or arc) one of whose
    edges (key) is given with ``segments`` straight pieces, in place:
    ``ResegmentCurveCommand``. Only a flat curve; ``rejected`` otherwise."""
    from core.curve_segments import ResegmentCurveCommand
    e, = command_protocol.need(history.scene.mesh, "edge", [edge])
    return command_protocol.execute(history, ResegmentCurveCommand(e, int(segments)))


def repair_solid(history, placement, fault=None):
    """Repair what has one right answer in a placement's (uid) mesh, shared
    by its copies: stray edges, duplicated faces, flat holes, partitions,
    reversed faces (the Solid Inspector's ``RepairSolidsCommand``). The
    counts come back as ``repaired``."""
    from ingetrazo_core import plan_solid_fault
    group = command_protocol.resolve_placements(history, [placement])[0] if placement is not None else None
    cmd = plan_solid_fault(history.scene, group, fault)
    if cmd is None:
        return {"ok": False, "step": None, "error": None, "rejected": None, "repaired": {}}
    before = history.revision
    history.execute(cmd)
    out = command_protocol.answer(history, before)
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
    return command_protocol.execute(history, ExplodeGroupCommand(g))


def split_into_pieces(history, placement):
    """Regroup the placement's (uid) contents by physical piece
    (``core.pieces.split_into_pieces``, ``SplitIntoPiecesCommand``): one
    child group per solid that does not touch another. Nothing happens
    (``ok`` False, no step) when it already is its pieces."""
    from core.history import SplitIntoPiecesCommand
    from core.pieces import split_into_pieces as split
    g, = command_protocol.resolve_placements(history, [placement])
    before = history.revision
    pieces = split(g)
    if not pieces:
        return command_protocol.answer(history, before, ok=False)
    history.execute(SplitIntoPiecesCommand(g, pieces))
    return command_protocol.answer(history, before)


def explode_view(history, placement, factor, mode="outward"):
    """Pull the placement's (uid) parts apart to ``factor`` (0
    reassembles) along ``mode``: ``outward``, ``x``, ``y`` or ``z``
    (``ExplodeViewCommand``)."""
    from core.explode import MODES
    from core.history import ExplodeViewCommand
    if mode not in MODES:
        raise ValueError(f"mode must be one of {MODES}")
    g, = command_protocol.resolve_placements(history, [placement])
    return command_protocol.execute(history, ExplodeViewCommand(g, float(factor), mode))


def purge_unused(history, layers=True, materials=True):
    """Remove the tags and/or materials nothing in the document uses
    (``PurgeUnusedCommand``; tags a saved view hides stay)."""
    from core.history import PurgeUnusedCommand
    before = history.revision
    cmd = PurgeUnusedCommand(layers=bool(layers), materials=bool(materials))
    if cmd.counts(history.scene) == (0, 0):
        return command_protocol.answer(history, before, ok=False)
    history.execute(cmd)
    return command_protocol.answer(history, before)


def purge_debris(history):
    """Drop the open mesh's vertices that no edge and no face uses
    (``Mesh.purge_debris``, one snapshot step)."""
    from core.history import SnapshotMutation
    return command_protocol.execute(history, SnapshotMutation(lambda scene: scene.mesh.purge_debris()))


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
        return command_protocol.answer(history, before, ok=False)
    if new_name in history.scene.materials:
        raise ValueError(f"there is already a material called {new_name!r}")
    history.execute(RestampMaterialCommand(name, replace(mat, name=new_name)))
    return command_protocol.answer(history, before)


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
        tint = tuple(color) if color is not None else tex.get("tint")
        history.execute(EditTexturedMaterialCommand(name, tint, size=size))
        return command_protocol.answer(history, before)
    if tile is not None and not mat.texture:
        raise ValueError("only a textured material has a tile size")
    changes = {}
    if color is not None:
        changes["color"] = tuple(float(c) for c in color[:3])
    if opacity is not None:
        o = float(opacity)
        changes["opacity"] = None if o >= 0.999 else max(0.0, o)
    if not changes:
        return command_protocol.answer(history, before, ok=False)
    history.execute(RestampMaterialCommand(name, replace(mat, **changes)))
    return command_protocol.answer(history, before)


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
    out = command_protocol.answer(history, before)
    out["name"] = new_name
    return out


def delete_material(history, name):
    """Remove a registry material; what wore it shows the default
    material: ``DeleteMaterialCommand``."""
    from core.material_commands import DeleteMaterialCommand
    _material(history, name)
    return command_protocol.execute(history, DeleteMaterialCommand(name))


def delete_placements(history, placements):
    """Delete placements (uids, nested included) with their geometry: one
    ``DeleteGroupCommand`` each, as one step."""
    from core.history import CompoundCommand, DeleteGroupCommand
    groups = command_protocol.resolve_placements(history, placements)
    before = history.revision
    if groups:
        history.execute(CompoundCommand([DeleteGroupCommand(g) for g in groups]))
    return command_protocol.answer(history, before)


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
    out = command_protocol.answer(history, before)
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
        return command_protocol.answer(history, before, ok=False)
    history.execute(RestampMaterialCommand(name, new))
    return command_protocol.answer(history, before)


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
        return command_protocol.answer(history, before, ok=False)
    history.execute(RestampMaterialCommand(name, new))
    return command_protocol.answer(history, before)


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
    out = command_protocol.answer(history, before, ok=bool(n))
    out["named"] = n
    return out


def rename_component(history, placement, name):
    """Rename a component definition: every copy (placed or nested) and the
    template kept in the file take ``name``, as one step. ``placement`` is
    the uid of any copy, or of the kept template."""
    from core.kept_components import copies_of, kept_template
    scene = history.scene
    name = str(name).strip()
    if not name:
        raise ValueError("a component needs a name")
    kept = {getattr(t, "uid", None): t for t in getattr(scene, "kept_components", None) or ()}
    g = kept.get(placement) if isinstance(placement, str) else None
    if g is None:
        g, = command_protocol.resolve_placements(history, [placement])
    targets = copies_of(scene, g.mesh)
    template = kept_template(scene, g.mesh)
    for extra in (template, g):
        if extra is not None and extra not in targets:
            targets.append(extra)
    from ingetrazo_core import NativeCommand
    before = history.revision
    targets = list(dict.fromkeys(t for t in targets if t.name != name))
    if targets:
        history.execute(NativeCommand.rename_each(targets, [name] * len(targets)))
    return command_protocol.answer(history, before)


from ingetrazo_core.deliveries import create_delivery, edit_delivery, delete_delivery, run_delivery


class _DocumentSettingsCommand(Command):
    doc_undo = True
    keeps_rows = True

    def __init__(self, name, edit):
        self.history_name = name
        self.edit = edit

    def do(self, scene):
        self.edit.apply(scene)
        self.doc_after(scene, "execute")

    def undo(self, scene):
        self._doc_txn.undo()
        self.doc_after(scene, "undo")


def _document_settings(history, name, edit):
    return command_protocol.execute(history, _DocumentSettingsCommand(name, edit))


def set_model_units(history, units):
    from ingetrazo_core import DocumentSettings
    return _document_settings(history, "Set model units", DocumentSettings.model_units(units))


def set_dimension_style(history, key, value):
    from ingetrazo_core import DocumentSettings
    return _document_settings(history, "Set dimension style", DocumentSettings.dimension_style(key, value))


def set_annotation_font(history, family, bold, italic, size):
    from ingetrazo_core import DocumentSettings
    return _document_settings(history, "Set annotation font",
                              DocumentSettings.annotation_font(family, bold, italic, size))


COMMANDS = {"set_location": set_location, "set_model_units": set_model_units, "set_annotation_font": set_annotation_font,
            "set_dimension_style": set_dimension_style, "patch_record_field": patch_record_field, "set_filing_rule": set_filing_rule,
            "create_delivery": create_delivery, "edit_delivery": edit_delivery,
            "delete_delivery": delete_delivery, "run_delivery": run_delivery, "follow_me": follow_me, "intersect_faces": intersect_faces,
            "add_edge": add_edge, "add_edges": add_edges, "add_face": add_face,
            "add_guide": add_guide, "change_guide": change_guide, "add_guide_circle": add_guide_circle, "add_arc": add_arc,
            "move_placements": move_placements, "copy_placements": copy_placements,
            "move_selection": move_selection, "scale_selection": scale_selection,
            "place_group": place_group, "erase_stroke": erase_stroke,
            "add_geo_path": add_geo_path, "move_geo_path_node": move_geo_path_node,
            "add_text_label": add_text_label, "add_dimension": add_dimension,
            "place_section_plane": place_section_plane, "set_north": set_north,
            "add_image_plane": add_image_plane, "set_group_scale": set_group_scale,
            "set_group_axes": set_group_axes, "flip_selection": flip_selection, "paste_clipboard": paste_clipboard, "bevel_selection": bevel_selection, "rotate_selection": rotate_selection, "offset_outline": offset_outline, "paint_material": paint_material,
            "divide_edges": divide_edges, "join_edges": join_edges,
            "move_vertices": move_vertices, "erase": erase,
            "reverse_faces": reverse_faces, "push_pull": push_pull, "orient_faces": orient_faces,
            "rebuild_cap": rebuild_cap, "solid_operation": solid_operation,
            "explode_group": explode_group,
            "split_into_pieces": split_into_pieces,
            "explode_view": explode_view, "purge_unused": purge_unused,
            "purge_debris": purge_debris, "undo": undo, "redo": redo,
            "hide": hide, "unhide": unhide, "lock": lock, "rename_placement": rename_placement,
            "rename_placements": rename_placements, "paint_placements": paint_placements,
            "make_unique": make_unique,
            "edit_assembly_part": edit_assembly_part, "edit_visibility_set": edit_visibility_set, "edit_saved_scene": edit_saved_scene, "edit_link_subset": edit_link_subset, "reset_link_tags": reset_link_tags, "unlink_placements": unlink_placements, "edit_section_plane": edit_section_plane, "edit_loose_geometry": edit_loose_geometry, "file_placements": file_placements, "promote_tag": promote_tag, "edit_structure_node": edit_structure_node, "edit_tag": edit_tag, "set_tag": set_tag, "set_tag_state": set_tag_state, "resegment_curve": resegment_curve, "repair_solid": repair_solid,
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

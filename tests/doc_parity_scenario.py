"""One document scenario, run by tests/test_document_parity.py in two
processes: ``INGETRAZO_DOC=0`` (the Python document) and the default (the
Rust document, core/doc_model.py). Prints one JSON object: the document
after every step, the feed's records, and what the saved files hold.

    python tests/doc_parity_scenario.py <workdir>
"""
from __future__ import annotations

import hashlib
import json
import os
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from core import doc_model  # noqa: E402
from core.group import Group  # noqa: E402
from core.history import (AddKeptComponentsCommand,  # noqa: E402
                          AssignLayerCommand, ChangeAxesCommand,
                          DeleteGroupCommand, DeleteLayerCommand,
                          ExplodeViewCommand,
                          FlipGroupsCommand, HideCommand, History,
                          InsertGroupCommand, KeepComponentCommand,
                          LockCommand, MakeNestedGroupCommand, MoveGroupCommand,
                          PurgeUnusedCommand, RenameGroupCommand,
                          ReparentGroupsCommand, RotateGroupCommand,
                          ScaleGroupCommand, SetGroupMaterialCommand,
                          SetLayerVisibilityCommand, SetMaterialFinishCommand,
                          SetPluginDataCommand, SetRecordCommand,
                          SetStructureNodeCommand, TagGroupsCommand)
from core.material_commands import AddMaterialCommand  # noqa: E402
from core.layers import TAG_FIELDS, Layer  # noqa: E402
from core.materials import Material  # noqa: E402
from core.mesh import Mesh  # noqa: E402
from core.scene import Scene  # noqa: E402
from formats import built, igz  # noqa: E402
import core.group as _group  # noqa: E402
import core.layers as _layers  # noqa: E402
import itertools  # noqa: E402

# Uids (and IFC guids) the two runs agree on.
_uids = itertools.count(1)
_group.new_uid = _layers.new_uid = lambda: f"uid-{next(_uids):05d}"
import uuid as _uuid  # noqa: E402
_guids = itertools.count(1)
_uuid.uuid4 = lambda: _uuid.UUID(int=next(_guids))


def box(mesh, s=1.0, x=0.0):
    import numpy as np
    p = [(0, 0, 0), (s, 0, 0), (s, s, 0), (0, s, 0),
         (0, 0, s), (s, 0, s), (s, s, s), (0, s, s)]
    quads = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4),
             (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
    pos = np.array([p[i] for q in quads for i in q], dtype=np.float64)
    pos[:, 0] += x
    mesh.add_faces_bulk(pos, np.full(6, 4), np.ones(6, dtype=np.int64))


def matrix(m):
    return None if m is None else [float(v) for v in m.data()]


def group_dump(g):
    return {
        "uid": g.uid, "name": g.name, "layer": g.layer,
        "hidden": bool(g.hidden), "locked": bool(g.locked),
        "component": bool(g.component), "material": g.material,
        "xform": matrix(g.xform), "axes": matrix(g.axes),
        "built": (built.placement_values(g.xform)
                  if g.xform is not None else None),
        "faces": len(g.mesh.faces), "billboard": g.billboard,
        "ifc": g.ifc, "ext": g.ext, "text3d": g.text3d,
        "exploded": g.exploded,
        "explode_offset": (None if g.explode_offset is None
                           else [float(v) for v in g.explode_offset]),
        "children": [group_dump(c) for c in g.children],
    }


def dump(scene):
    return json.loads(json.dumps({
        "views": [v.to_dict() for v in scene.saved_views],
        "sheets": [s.to_dict() for s in scene.compositions],
        "annotations": [t.to_dict() for t in scene.text_labels],
        "dimensions": [(d.a.toTuple(), d.b.toTuple(), d.offset.toTuple(), d.text)
                       for d in scene.dimensions],
        "groups": [group_dump(g) for g in scene.groups],
        "layers": [dict({f: getattr(ly, f) for f in TAG_FIELDS},
                        **{"dict": ly.to_dict()}) for ly in scene.layers],
        "materials": {k: m.to_dict() for k, m in sorted(scene.materials.items())},
        "loose_faces": len(scene.mesh.faces),
        "active_layer": scene.active_layer,
        "folders": {"list": list(scene.layer_folders),
                    "parents": dict(scene.folder_parents),
                    "visibility": dict(scene.folder_visibility),
                    "uids": dict(scene.layer_folder_uids),
                    "names": dict(scene.layer_folder_names)},
        "plugin_data": scene.plugin_data,
        "kept": [group_dump(g) for g in scene.kept_components],
    }, default=lambda o: sorted(o) if isinstance(o, set) else repr(o)))


def build():
    scene = Scene()
    for i in range(3):
        scene.layers.append(Layer(f"Tag {i}", uid=f"tag-uid-{i}",
                                  color=(i / 3, 0.5, 0.25)))
    # Slice 2: tag folders and their tables, sheet styles, the active tag.
    scene.layer_folders = ["Site", "Site/Walls"]
    scene.folder_parents["Site/Walls"] = "Site"
    scene.layer_folder_uids.update({"Site": "folder-1", "Site/Walls": "folder-2"})
    scene.layer_folder_names["Site/Walls"] = "Walls"
    scene.folder_visibility["Site"] = True
    styled = Layer("Styled", uid="tag-uid-styled", folder="Site/Walls",
                   linetype="dashed", lineweight=0.35, hatch="brick",
                   hatch_angle=30.0, opacity=0.5, printable=False)
    scene.layers.append(styled)
    scene.active_layer = "Tag 2"
    scene.plugin_data["windowizer"] = {"version": 2, "panes": [1, 2]}
    for i in range(2):
        scene.materials[f"Mat {i}"] = Material(f"Mat {i}", color=(0.1 * i, 0.2, 0.3))
    defs = []
    for d in range(2):
        m = Mesh()
        box(m, 0.5 + d * 0.25)
        defs.append(m)
    kids = []
    for k in range(4):
        g = Group(defs[k % 2], name=f"Part {k}", uid=f"part-{k}")
        t = QMatrix4x4()
        t.translate(QVector3D(k * 1.5, 0.25, 0))
        t.rotate(15.0 * k, QVector3D(0, 0, 1))
        g.xform = t
        g.layer = f"Tag {k % 3}"
        kids.append(g)
    assembly = Group(Mesh(), name="Assembly", uid="assembly")
    assembly.adopt(kids)
    assembly.component = False
    scene.groups.append(assembly)
    # A .built placement: binary64 values beside their float32 projection.
    vals = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 650000.013, 6860000.027, 1.5, 1]
    inst = Group(defs[0], name="Site", uid="site")
    inst.xform = built.placement(vals)
    scene.groups.append(inst)
    classic = Mesh()
    box(classic, 1.0, x=10.0)
    scene.groups.append(Group(classic, name="Classic", uid="classic"))
    kids[0].ifc = {"class": "IfcBeam", "name": "B1", "guid": "guid-b1",
                   "classifications": [{"system": "uniclass", "code": "Ss_20"}]}
    kids[1].ext = {"windowizer": {"panes": 3}}
    kept = Group(defs[1], name="Kept def", uid="kept")
    kept.xform = QMatrix4x4()
    scene.kept_components.append(kept)
    return scene


def step_record(step):
    return {"phase": step.phase, "command": step.command,
            "placements": {k: sorted(v) for k, v in step.placements.items()},
            "tags": step.tags, "materials": step.materials,
            "complete": step.complete}


def members(path):
    """Each member's bytes (a ZIP's own timestamps left out); a small
    document is one plain JSON file."""
    if not zipfile.is_zipfile(path):
        return {"document": hashlib.sha256(Path(path).read_bytes()).hexdigest()}
    with zipfile.ZipFile(path) as z:
        return {n: hashlib.sha256(z.read(n)).hexdigest()
                for n in sorted(z.namelist())}


def saved(scene, work: Path, tag: str) -> dict:
    files = {}
    for name in ("doc.igz", "doc.built"):
        path = work / f"{tag}-{name}"
        igz.save_scene(scene, path, backup=False)
        back = Scene()
        igz.load_into(back, path)
        files[name] = {"members": members(path), "reopened": dump(back),
                       "file_hash": hashlib.sha256(path.read_bytes()).hexdigest()}
        # Saved again from the reopened document: the same bytes.
        again = work / f"{tag}-again-{name}"
        igz.save_scene(back, again, backup=False)
        files[name]["again"] = members(again)
        files[name]["again_hash"] = hashlib.sha256(again.read_bytes()).hexdigest()
    return files


def template():
    m = Mesh()
    box(m, 0.3)
    g = Group(m, name="Template", uid="tpl")
    g.xform = QMatrix4x4()
    return g


def slice_two(scene, h):
    """The commands slice 2 makes transactions (records, structure, plugin
    data, folders, kept components, the registry, placements' structure
    and matrices), each run, all undone, all redone."""
    from core.purge import iter_groups
    out = []
    by_uid = {g.uid: g for g in iter_groups(scene.groups)}
    assembly, site, classic = by_uid["assembly"], by_uid["site"], by_uid["classic"]
    parts = list(assembly.children)
    frame = QMatrix4x4()
    frame.translate(QVector3D(10.0, 0.0, 0.0))
    frame.rotate(30.0, QVector3D(0, 0, 1))
    cmds = [
        SetRecordCommand([parts[0], parts[2]], {"name": "Beam", "fire": "R60"}),
        TagGroupsCommand([parts[3]], "IfcColumn", "C1"),
        SetStructureNodeCommand([parts[1], parts[2]], "level-1"),
        SetPluginDataCommand("windowizer", {"version": 3}),
        SetPluginDataCommand("other", {"a": [1, 2, {"b": None}]}),
        SetLayerVisibilityCommand(["Tag 0"], False, folders=["Site"]),
        KeepComponentCommand(scene.kept_components[0], False),
        AddKeptComponentsCommand([template()]),
        AddMaterialCommand(Material("Fresh", color=(0.9, 0.8, 0.1))),
        PurgeUnusedCommand(layers=False, materials=True),
        ExplodeViewCommand(assembly, 1.5, "outward"),
        RotateGroupCommand(parts[0], QVector3D(0, 0, 0), QVector3D(0, 0, 1), 33.0),
        ScaleGroupCommand(parts[1], QVector3D(1, 1, 0), 1.25),
        FlipGroupsCommand([parts[2]], QVector3D(0, 0, 0), QVector3D(1, 0, 0)),
        ChangeAxesCommand(classic, frame),
        ReparentGroupsCommand([parts[3]], None),
        DeleteLayerCommand("Tag 1"),
    ]
    for c in cmds:
        h.execute(c)
        assert not h.last_error, (type(c).__name__, h.last_error)
        out.append(dump(scene))
    for _ in cmds:
        assert h.undo()
        out.append(dump(scene))
    for _ in cmds:
        assert h.redo()
        out.append(dump(scene))
    # An in-place edit of a record, outside any command, then a step.
    parts[0].ifc["name"] = "Edited in place"
    h.execute(RenameGroupCommand(parts[0], "After the edit"))
    out.append(dump(scene))
    h.undo()
    out.append(dump(scene))
    return out


def structural_steps(steps):
    from core.history import (ExplodeGroupCommand, GroupToComponentCommand,
                              MakeComponentOfCommand, MakeGroupCommand,
                              MakeUniqueCommand, MergeGroupsCommand,
                              ReplaceComponentCommand, SplitIntoPiecesCommand,
                              UnlinkCommand)
    scene = build()
    scene.active_layer = None
    history = History(scene)
    history.feed.subscribe(lambda step: steps.append(step_record(step)))
    assembly, target, classic = scene.groups
    source = assembly.children[1]
    source.billboard = "mesh"
    link = template()
    link.ext = {"link": {"path": "/synthetic/reference.igz"}}
    scene.groups.append(link)
    scene.plugin_data["links"] = {"/synthetic/reference.igz": {}}
    box(scene.mesh)
    commands = [
        MakeGroupCommand(scene.mesh.faces, [], name="Faces"),
        GroupToComponentCommand(classic),
        MakeComponentOfCommand([], [], [classic, target], "Assembly"),
        MakeUniqueCommand(target),
        ReplaceComponentCommand([target], source),
        SplitIntoPiecesCommand(classic, [template()]),
        ExplodeGroupCommand(assembly),
        MergeGroupsCommand([classic, target]),
        UnlinkCommand([link]),
    ]
    states = []
    for cmd in commands:
        history.execute(cmd)
        assert not history.last_error and not history.last_rejected
        if doc_model.ENABLED:
            assert cmd._doc_txn is not None, type(cmd).__name__
        for action in (None, history.undo, history.redo, history.undo):
            if action:
                assert action()
            states.append(dump(scene))
    return states


def shared_assembly(work):
    """Persisted occurrences have their own UIDs, visibility, tags and records."""
    from core.protocol import commands
    scene = Scene()
    scene.layers.append(Layer("Steel", uid="steel"))
    geometry, part = Mesh(), Mesh()
    box(part)
    for label in ("West", "East", "Annex"):
        group = Group(geometry, uid=label)
        children = []
        for name in (("beam",) if label == "Annex" else ("beam", "post")):
            child = Group(part, name=name, uid=name if name == "beam" else f"{label}-{name}")
            child.xform = QMatrix4x4()
            child.ifc = {"class": "IfcBeam", "name": f"{label} {name}"}
            children.append(child)
        group.adopt(children)
        scene.groups.append(group)
    west, east, annex = scene.groups
    target = east.children[0]
    path = [east.uid, target.uid]
    from core.link_structure import Tree
    tree = Tree(scene, {})
    assert tree.address(west.children[0]) == [west.uid, "beam"]
    assert tree.address(target) == path and tree.row_id(west.children[0]) != tree.row_id(target)
    h = History(scene)
    commands.hide(h, [path])
    commands.set_tag(h, [path], "Steel")
    assert scene.occurrence_path(target) == path and scene.resolve_occurrence(path) is target
    assert not west.children[0].hidden and west.children[0].layer is None
    assert h.undo() and h.redo()
    commands.run(h, "patch_record_field", placements=[path], path=["description"], value="East only")
    assert target.ifc["description"] == "East only"
    assert "description" not in west.children[0].ifc
    from core.deliveries import scoped
    scene.select([target])
    projection = scoped(scene, {"scope": "selection"})
    delivered = projection.resolve_occurrence(path)
    assert delivered is not target and delivered.mesh is target.mesh
    assert delivered.ifc["description"] == "East only"
    assert projection.resolve_occurrence([west.uid, target.uid]) is None
    scene.selection.clear()
    result = saved(scene, work, "shared")
    # Independently reopen the old on-disk shape and edit through the second
    # nested placement. Both models preserve the third, divergent child tree.
    restored = Scene()
    igz.load_into(restored, work / "shared-doc.igz")
    west, east, annex = restored.groups
    target = restored.resolve_occurrence(path)
    assert target is east.children[0] and target.hidden and target.layer == "Steel"
    assert target.ifc["name"] == "East beam" and west.children[0].ifc["name"] == "West beam"
    restored.begin_group_edit(east)
    restored.begin_group_edit(target)
    restored.mesh.add_edge(QVector3D(8, 8, 8), QVector3D(10, 8, 8))
    restored.end_group_edit()
    assert len(west.children[0].mesh.edges) == 13 and len(annex.children) == 1
    if doc_model.ENABLED:
        history = History(restored)
        old = east.children[1]
        counterpart = west.children[1]
        history.execute(DeleteGroupCommand(old))
        assert history.last_error is None and list(east.children) == [target]
        assert len(west.children) == len(annex.children) == 1
        assert history.undo() and west.children[1] is counterpart and east.children[1] is old
        assert history.redo() and len(west.children) == 1
        assert history.undo()
        # Reorder the slots and write an override through a path. Both
        # occurrences keep their original handles and the old UID paths resolve.
        east.children[:] = list(reversed(east.children))
        assert west.children[0] is counterpart and restored.resolve_occurrence(path) is target
        restored.begin_group_edit(east)
        new_child = Group(part, uid="East-brace")
        new_child.xform = QMatrix4x4()
        history.execute(InsertGroupCommand(new_child))
        assert history.last_error is None and len(west.children) == len(east.children) == 3
        projected = west.children[-1]
        assert projected.uid != new_child.uid and projected.mesh is new_child.mesh
        assert history.undo() and len(west.children) == 2
        assert history.redo() and west.children[-1] is projected
        restored.end_group_edit()
    return result


def geometry_steps(work):
    """Compare the loose mesh, feed and replay at each command boundary."""
    from core.history import (MakeGroupCommand, ExplodeGroupCommand, SetFaceColorCommand,
                              AddEdgeCommand, DeleteEdgesCommand, DeleteFaceCommand,
                              ExplodeCurvesCommand, TagCurveCommand, PruneOrphanEdgesCommand,
                              AddFaceCommand, EraseSelectionCommand, MoveVerticesCommand, SnapshotCompound)
    from core.protocol import commands

    scene = Scene()
    # Commands in the preceding scenario consume generated UIDs differently
    # on redo. This independent document needs the same initial tag identity.
    scene.layers[0].uid = "geometry-default-tag"
    mesh = scene.mesh
    box(mesh)
    h = History(scene)
    out = []
    def state():
        return {
            "vertices": [v.position_tuple() for v in mesh.vertices],
            "faces": [[list(v.toTuple()) for v in f.vertices] for f in mesh.faces],
            "holes": [[[v.toTuple() for v in ring] for ring in f.holes] for f in mesh.faces],
            "paint": [dict(f.attrs) for f in mesh.faces],
            "edge_flags": [(e.soft, e.flat, e.curve, e.layer) for e in mesh.edges],
            "group_faces": [len(g.mesh.faces) for g in scene.groups],
            "feed": [({k: len(v) for k, v in m.created.items()},
                      {k: len(v) for k, v in m.removed.items()},
                      {k: len(v) for k, v in m.modified.items()}) for m in h.feed.last.meshes],
        }

    for edit in (lambda: h.execute(AddEdgeCommand(QVector3D(4, 0, 0), QVector3D(4, 1, 0),
                                                  curve=17, soft=True, layer="Ink")),
                 lambda: h.execute(ExplodeCurvesCommand([mesh.edges[-1]])),
                 lambda: h.execute(SnapshotCompound([TagCurveCommand(
                     [QVector3D(4, 0, 0), QVector3D(4, 1, 0)], closed=False)], heal=False)),
                 lambda: h.execute(PruneOrphanEdgesCommand([QVector3D(4, 0, 0)])),
                 lambda: h.execute(AddEdgeCommand(QVector3D(8, 0, 0), QVector3D(8, 1, 0))),
                 lambda: h.execute(DeleteEdgesCommand([mesh.edges[-1]])),
                 lambda: h.execute(SetFaceColorCommand([mesh.faces[1]], [0., .5, 1.])),
                 lambda: commands.push_pull(h, mesh.faces[1].key, .25),
                 lambda: commands.move_vertices(h, [mesh.vertices[0].key], [.1, 0., 0.]),
                 lambda: h.execute(MakeGroupCommand(list(mesh.faces), list(mesh.edges))),
                 lambda: h.execute(ExplodeGroupCommand(scene.groups[0]))):
        edit()
        assert not h.last_error and not h.last_rejected
        out.append(state())
        ids = [f.key for f in mesh.faces]
        assert h.undo()
        out.append(state())
        assert h.redo()
        assert ids == [f.key for f in mesh.faces]
        out.append(state())
    # A vertical face crosses the box. Cut only the selected loose context.
    cutter = mesh.add_face([QVector3D(.5, -1, -1), QVector3D(.5, 2, -1),
                            QVector3D(.5, 2, 2), QVector3D(.5, -1, 2)])
    assert commands.intersect_faces(h, "context", [cutter.key])["ok"]
    keys = [edge.key for edge in mesh.edges]
    out.append(state())
    assert h.undo()
    out.append(state())
    assert h.redo() and keys == [edge.key for edge in mesh.edges]
    out.append(state())
    h.execute(DeleteFaceCommand(mesh.faces[-1]))
    assert not h.last_error and not h.last_rejected
    out.append(state())
    assert h.undo()
    out.append(state())
    assert h.redo()
    out.append(state())
    # Draw a painted panel, a hole, an adjacent opening and a corner cut.
    def rectangle(x, y, w, height):
        return [QVector3D(a, b, 0) for a, b in
                ((x,y), (x+w,y), (x+w,y+height), (x,y+height))]
    draw_revisions = []
    for rect, paint in [((10,0,8,8), {"color":[.2,.4,.6]}),
                        ((12,2,1,1), None), ((13,2,1,1), None), ((10,0,1,1), None)]:
        serial = mesh._mut_serial
        h.execute(AddFaceCommand(rectangle(*rect), attrs=paint))
        draw_revisions.append(mesh._mut_serial - serial)
        assert not h.last_error and not h.last_rejected
        out.append(state())
        assert h.undo()
        out.append(state())
        assert h.redo()
        out.append(state())
    left = mesh.add_face(rectangle(100, 0, 2, 2))
    left.attrs["color"] = [0.1, 0.2, 0.3]
    right = mesh.add_face(rectangle(102, 0, 1, 2))
    edge = next(e for e in mesh.edges if left in e.faces and right in e.faces)
    scene.selection.add(edge)
    h.execute(EraseSelectionCommand([edge]))
    assert not h.last_error and not h.last_rejected and edge not in scene.selection
    out.append(state())
    assert h.undo() and edge in mesh.edges
    out.append(state())
    assert h.redo() and edge not in mesh.edges
    out.append(state())
    # Compare welding a box into a ramp and carrying both rings of a textured panel.
    box(mesh, x=200.)
    outer = rectangle(300, 0, 4, 4)
    hole = rectangle(301, 1, 1, 1)
    panel = mesh.add_face(outer, [hole])
    panel.attrs["texture"] = {"path": "tile.png", "sw": .3, "sh": .7, "rot": 12.}
    for cmd in (MoveVerticesCommand([QVector3D(201, 0, 1), QVector3D(201, 1, 1)],
                                    QVector3D(0, 0, -1)),
                MoveVerticesCommand(outer + hole, QVector3D(.3, .7, .2), carry_textures=True)):
        h.execute(cmd)
        assert not h.last_error and not h.last_rejected
        assert cmd.welded == (not cmd.carry_textures)
        out.append(state())
        assert h.undo()
        out.append(state())
        assert h.redo()
        out.append(state())
    outer = rectangle(400, 0, 2, 2)
    warp = mesh.add_face(outer)
    warp.attrs["color"] = [0.1, 0.3, 0.7]
    h.execute(MoveVerticesCommand(outer[:1], QVector3D(0, 0, .7)))
    assert not h.last_rejected and not h.last_error and warp not in mesh.faces
    out.append(state())
    assert h.undo() and warp in mesh.faces
    out.append(state())
    assert h.redo() and warp not in mesh.faces
    out.append(state())
    from core.history import FlipVerticesCommand, RotateVerticesCommand, ScaleVerticesCommand
    panel = mesh.add_face(rectangle(500, 0, 2, 2))
    panel.attrs["texture"] = {"path": "fabric.png", "sw": .7, "sh": .3}
    pivot, axis = QVector3D(500, 0, 0), QVector3D(0, 0, 1)
    for transform in (lambda pts: RotateVerticesCommand(pts, pivot, axis, 30., carry_textures=True),
                      lambda pts: ScaleVerticesCommand(pts, pivot, (1.25, .5, 1.)),
                      lambda pts: FlipVerticesCommand(pts, pivot, QVector3D(1, 0, 0), [panel])):
        h.execute(transform(panel.vertices))
        assert not h.last_error and not h.last_rejected
        out.append(state())
        assert h.undo()
        out.append(state())
        assert h.redo()
        out.append(state())
    # The cap and both wall rings share the outer push/pull transaction.
    sheet = mesh.add_face(rectangle(800, 0, 5, 5), [rectangle(801, 1, 1, 1)])
    sheet.attrs["color"] = [0.2, 0.7, 0.4]
    store = getattr(mesh, "_store", None)
    version, serial, revision = scene.version, mesh._mut_serial, getattr(store, "revision", 0)
    assert commands.push_pull(h, sheet.key, 1.)["ok"]
    band_revisions = [scene.version - version, mesh._mut_serial - serial,
                      getattr(store, "revision", 0) - revision]
    out.append(state())
    face_ids = [f.key for f in mesh.faces]
    assert h.undo()
    out.append(state())
    assert h.redo() and face_ids == [f.key for f in mesh.faces]
    out.append(state())
    # Offset a painted face with an opening, through its normal History entry.
    from core.offset import build_offset_face
    slab = mesh.add_face(rectangle(850, 0, 6, 6), [rectangle(852, 2, 1, 1)])
    slab.attrs["color"] = [.3, .4, .8]
    h.execute(build_offset_face(mesh, slab.key, .5))
    assert not h.last_error and not h.last_rejected
    out.append(state())
    assert h.undo()
    out.append(state())
    assert h.redo()
    out.append(state())
    profile = mesh.add_face(rectangle(900, 0, 2, 2))
    profile.attrs["color"] = [.2, .6, .8]
    assert commands.follow_me(h, profile.key, [(900, 0, 0), (900, 0, 2), (900, 3, 2)])["ok"]
    out.append(state())
    assert h.undo()
    out.append(state())
    assert h.redo()
    out.append(state())
    from core.soften import SoftenByAngleCommand
    p, q = QVector3D(930, 0, 0), QVector3D(932, 0, 0)
    mesh.add_face([p, q, QVector3D(930, 1, 0)])
    mesh.add_face([q, p, QVector3D(930, -1, .5)])
    seam = mesh.find_edge(mesh.vertex_at(p), mesh.vertex_at(q))
    h.execute(SoftenByAngleCommand({mesh: {seam.key}}, 30, smooth=False))
    assert (seam.soft, seam.flat) == (True, True)
    out.append(state())
    assert h.undo() and (seam.soft, seam.flat) == (False, False)
    out.append(state())
    assert h.redo() and (seam.soft, seam.flat) == (True, True)
    out.append(state())
    # Refused joins preserve the last published change and the exact mesh.
    refused = rectangle(600, 0, 2, 2)
    mesh.add_face(refused)
    before = state()
    revision = h.revision
    h.execute(MoveVerticesCommand(refused[:1], refused[2] - refused[0]))
    assert h.last_rejected and not h.last_error
    assert h.revision == revision and state() == before
    rejection = h.last_rejected
    archives = {}
    for ext in ("igz", "built"):
        path = work / f"edited-geometry.{ext}"
        igz.save_scene(scene, path, backup=False)
        archives[ext] = hashlib.sha256(path.read_bytes()).hexdigest()
    return {"states": out, "archive_hashes": archives, "draw_revisions": draw_revisions,
            "band_revisions": band_revisions, "rejection": rejection}


def anchor_steps(work):
    """Three attachment kinds on the second of two nested instances."""
    from core.annotation_anchor import Attachment
    from core.history import MoveGroupCommand
    from core.textlabel import TextLabel

    scene = Scene()
    mesh = Mesh()
    face = mesh.add_face([QVector3D(0, 0, 0), QVector3D(4, 0, 0),
                          QVector3D(0, 4, 0)])
    for name, x in (("one", 0), ("two", 10)):
        parent = Group(Mesh(), uid=name)
        parent.xform = QMatrix4x4()
        parent.xform.translate(x, 0, 0)
        part = Group(mesh, uid="part")
        part.xform = QMatrix4x4()
        parent.adopt([part])
        scene.groups.append(parent)
    edge = mesh.find_edge(face.loop[0], face.loop[1])
    for entity, xyz in ((face.loop[0], (10, 0, 0)), (edge, (12, 0, 0)), (face, (11, 1, 0))):
        point = QVector3D(*xyz)
        ref = Attachment.capture(scene, entity, part, point)
        assert ref.binding.path == ["two", "part"]
        scene.text_labels.append(TextLabel(point, QVector3D(0, 1, 0), ref.binding.kind, attachment=ref))
    def positions(sc):
        return [label.anchor.toTuple() for label in sc.text_labels]
    history = History(scene)
    sign = Group(Mesh(), uid="sign")
    sign.xform = QMatrix4x4()
    sign.text3d = {"text": "Sign"}
    history.execute(InsertGroupCommand(sign, attachment=scene.text_labels[-1].attachment))
    assert not history.last_error, history.last_error
    assert history.undo() and not scene.text_placement_anchors
    assert history.redo() and len(scene.text_placement_anchors) == 1
    from core.composition import Composicion, CotaItem, MarcoVista
    start_point = positions(scene)[0]
    item = CotaItem(anchor_uid="detail", a_world=list(start_point), b_world=[0, 0, 0])
    item.hold_a = (start_point, scene.text_labels[0].attachment)
    scene.compositions.append(Composicion(frames=[MarcoVista(uid="detail")], cotas=[item]))
    original = positions(scene)
    history.execute(MoveGroupCommand(parent, QVector3D(0, 3, 0)))
    assert sign.xform.map(QVector3D()).toTuple() == (0, 3, 0)
    assert "sign" in history.feed.last.placements["modified"]
    moved = positions(scene)
    assert moved == [(x, y + 3, z) for x, y, z in original]
    assert history.undo() and positions(scene) == original
    assert sign.xform.map(QVector3D()).toTuple() == (0, 0, 0)
    assert history.redo() and positions(scene) == moved
    # A translation of the defining vertices keeps all three element keys.
    from core.history import MoveVerticesCommand
    scene.begin_group_edit(part)
    vertices = [v.position for v in scene.mesh.vertices]
    history.execute(MoveVerticesCommand(vertices, QVector3D(0, 0, 2)))
    assert not history.last_error, history.last_error
    raised = [(x, y, z + 2) for x, y, z in moved]
    assert positions(scene) == raised
    assert history.undo() and positions(scene) == moved
    assert history.redo() and positions(scene) == raised
    assert history.undo()
    scene.end_group_edit()
    assert positions(scene) == moved
    for ext in ("igz", "built"):
        file = work / ("anchors." + ext)
        igz.save_scene(scene, file, backup=False)
        back = Scene()
        igz.load_into(back, file)
        assert positions(back) == moved
        again = work / ("anchors-saved-again." + ext)
        igz.save_scene(back, again, backup=False)
        assert again.read_bytes() == file.read_bytes()
        cota = back.compositions[0].cotas[0]
        assert tuple(cota.a_world) == moved[0] and cota.hold_a[1].position().toTuple() == moved[0]
        assert all(label.attachment is not None for label in back.text_labels)
        loaded = History(back)
        loaded.execute(MoveGroupCommand(back.groups[1], QVector3D(0, -3, 0)))
        assert positions(back) == original
        assert tuple(back.compositions[0].to_dict()["cotas"][0]["a_world"]) == original[0]
        assert back.resolve_occurrence(["sign"]).xform.map(QVector3D()).toTuple() == (0, 0, 0)
    from core.history import AddTextLabelCommand
    auto = TextLabel(QVector3D(), QVector3D(), "New")
    history.execute(AddTextLabelCommand(auto))
    tag = scene.layer(auto.layer)
    assert tag is not None and history.undo() and scene.layer(tag.key) is tag
    assert history.feed.last.tags["removed"] == []
    assert history.redo() and scene.text_labels[-1] is auto
    return moved


def presentation_steps(scene, steps):
    from core.annotation_anchor import Attachment
    from core.composition import Composicion, MarcoVista, TextoItem
    from core.dimension import Dimension
    from core.history import (AddDimensionCommand, AddSceneCommand, AddSheetCommand,
                              AddTextLabelCommand, EditDimensionTextCommand,
                              MoveTextLabelsCommand, PlaceViewCommand)
    from core.saved_views import RenameSceneCommand, SavedView
    from core.textlabel import TextLabel

    h = History(scene)
    h.feed.subscribe(lambda step: steps.append(step_record(step)))
    owner = next(g for g in scene.groups if g.uid == "classic")
    face = owner.mesh.faces[0]
    p = sum(face.vertices, QVector3D()) / len(face.vertices)
    label = TextLabel(p, QVector3D(0, 0, 1), "Face", layer="Tag 0",
                      attachment=Attachment.capture(scene, face, owner, p))
    dim = Dimension(face.vertices[0], face.vertices[1], QVector3D(0, 1, 0), layer="Tag 0")
    sheet = Composicion(texts=[TextoItem(text="Plan notes")])
    view = SavedView("Plan", hidden_objects=["classic"], hidden_layers=["Tag 2"],
                     section={"active": None}, style={"profiles": True})
    edits = [AddDimensionCommand(dim), AddTextLabelCommand(label), AddSceneCommand(view),
             AddSheetCommand(sheet), PlaceViewCommand(sheet, MarcoVista(view_key="scene:Plan")),
             RenameSceneCommand(view, "Floor"), MoveTextLabelsCommand([label], QVector3D(1, 1, 0)),
             EditDimensionTextCommand(dim, "Length <>")]
    states = []
    for cmd in edits:
        h.execute(cmd)
        assert not h.last_error, h.last_error
        if doc_model.ENABLED:
            assert cmd._doc_txn is not None
        states.append(dump(scene))
    for action in (h.undo, h.redo):
        for _ in edits:
            assert action()
            states.append(dump(scene))
    assert sheet.frames[0].view_key == "scene:Floor"
    return states


def main(work: Path):
    initial_scene = build()
    # The prototype table in existing files interns geometry. Two entries
    # naming the same prototype can have different nested placement data.
    original = initial_scene.groups[0]
    other = Group(original.mesh, name="Second assembly", uid="other-assembly")
    other.adopt([Group(original.children[0].mesh, name="Only part", uid="only-part")])
    initial_scene.groups.append(other)
    # Two occurrences of the same geometry can carry distinct IFC and tag
    # values. Address the second part through its persisted UID path.
    from core.protocol import commands
    history = History(initial_scene)
    child = other.children[0]
    child.ifc = {"name": "Second record"}
    commands.set_tag(history, [[other.uid, child.uid]], "Tag 2")
    commands.hide(history, [[other.uid, child.uid]])
    assert child.hidden and not original.children[0].hidden
    assert child.layer == "Tag 2" and original.children[0].layer == "Tag 0"
    assert history.undo() and not child.hidden
    assert history.redo() and child.hidden
    anchor_steps(work)
    presentation_feed = []
    presentation = presentation_steps(initial_scene, presentation_feed)
    initial = saved(initial_scene, work, "initial")
    scene = build()
    h = History(scene)
    steps = list(presentation_feed)
    h.feed.subscribe(lambda s: steps.append(step_record(s)))
    states = [dump(scene)]
    a = scene.groups[0]
    part = a.children[1]
    extra = Group(Mesh(), name="Inserted", uid="inserted")
    cmds = [
        RenameGroupCommand(part, "Renamed part"),
        LockCommand([scene.groups[1]], True),
        HideCommand([scene.groups[2], part], True),
        AssignLayerCommand([scene.groups[1]], "Brand new tag"),
        SetGroupMaterialCommand(part, {"color": (1.0, 0.0, 0.0), "mat": "Mat 1"}),
        MoveGroupCommand(part, QVector3D(0.1, 0.2, 0.3)),
        MoveGroupCommand(scene.groups[2], QVector3D(0.0, 1.0, 0.0)),
        InsertGroupCommand(extra),
        DeleteGroupCommand(scene.groups[1]),
        SetLayerVisibilityCommand(["Tag 1"], False),
        SetMaterialFinishCommand("Mat 0", "matte"),
    ]
    for c in cmds:
        h.execute(c)
        assert not h.last_error, h.last_error
        states.append(dump(scene))
    while h.undo():
        states.append(dump(scene))
    for _ in range(len(cmds)):
        h.redo()
        states.append(dump(scene))
    for _ in range(5):
        h.undo()
        states.append(dump(scene))
    # A group of two root placements (a structural command with its own
    # undo: journalled, not undone by its transaction).
    h.execute(MakeNestedGroupCommand([], [], list(scene.groups[:2]), name="Outer"))
    assert not h.last_error, h.last_error
    states.append(dump(scene))
    h.undo()
    states.append(dump(scene))
    h.redo()
    states.append(dump(scene))
    states.extend(presentation)
    states.extend(slice_two(scene, h))
    states.extend(structural_steps(steps))
    # Rotate a mirrored, non-uniformly scaled occurrence and edit through
    # its world-space tool coordinates. The sibling reads definition space.
    framed = Scene()
    first = Group(Mesh(), uid="framed")
    second = Group(first.mesh, uid="sibling")
    transform = QMatrix4x4()
    transform.translate(17, 4, 2)
    transform.rotate(90, 0, 0, 1)
    transform.scale(-2, 3, 4)
    first.xform, second.xform = transform, QMatrix4x4()
    framed.groups.extend([first, second])
    framed.begin_group_edit(first)
    endpoints = [QVector3D(1, 2, 3), QVector3D(3, 1, 2)]
    framed.mesh.add_edge(*(transform.map(p) for p in endpoints))
    if doc_model.ENABLED:
        assert first.mesh is second.mesh and len(second.mesh.edges) == 1
        from core.protocol.keys import mesh_key
        assert mesh_key(framed.mesh) == mesh_key(second.mesh)
    framed.end_group_edit()
    positions = [v.position_tuple() for v in second.mesh.vertices]
    import numpy as np
    np.testing.assert_allclose(sorted(positions), sorted(p.toTuple() for p in endpoints), atol=1e-6)
    shared = shared_assembly(work)
    print(json.dumps({"geometry": geometry_steps(work), "shared": shared, "rotated_edit": positions, "engine": doc_model.engine(), "states": states,
                      "feed": steps, "initial": initial,
                      "final": saved(scene, work, "final")}))


if __name__ == "__main__":
    main(Path(sys.argv[1]))

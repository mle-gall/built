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
    initial = saved(initial_scene, work, "initial")
    scene = build()
    h = History(scene)
    steps = []
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
    print(json.dumps({"shared": shared, "rotated_edit": positions, "engine": doc_model.engine(), "states": states,
                      "feed": steps, "initial": initial,
                      "final": saved(scene, work, "final")}))


if __name__ == "__main__":
    main(Path(sys.argv[1]))

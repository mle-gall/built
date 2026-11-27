# SPDX-License-Identifier: GPL-3.0-or-later
"""Persistent element and record ids (docs/persistent-ids.md).

Element ids (vertices, edges, faces) survive save and load, undo and redo,
and the edits §4 lists; definitions, materials, scenes, annotations and
sheet items keep uids; the change feed, queries and commands speak them;
IFC GlobalIds derive from them and stay the same across sessions."""
from __future__ import annotations

import json
import zipfile
from pathlib import Path

import pytest
from built_core import Matrix4, Vec3 as V

from built_core import element_ids as E
from built_core.group import Group
from built_core.history import History
from built_core.protocol import commands as C
from built_core.protocol import queries as Q
from built_core.scene import Scene
from formats import igz


def ids(mesh, kind="face"):
    return [E.id_text(kind, i) for i in mesh._store.ids(kind)]


def all_ids(mesh):
    return {k: sorted(ids(mesh, k)) for k in ("vertex", "edge", "face")}


def face_id(mesh, face):
    return E.id_text("face", mesh._store.id_of("face", face.key))


def area(face):
    pts = [v.position_tuple() for v in face.loop]
    s = 0.0
    for i in range(len(pts)):
        (x1, y1, _), (x2, y2, _) = pts[i], pts[(i + 1) % len(pts)]
        s += x1 * y2 - x2 * y1
    return abs(s) / 2


def faces_by_id(mesh):
    return {face_id(mesh, f): f for f in mesh.faces}


def manifest(path):
    with zipfile.ZipFile(path) as z:
        return json.loads(z.read("document.json"))


def touched(mesh):
    """Build a deferred mesh (its first access)."""
    mesh.edges
    return mesh


def history_scene():
    sc = Scene()
    h = History(sc)
    C.run(h, "add_face", outer=[[0, 0, 0], [4, 0, 0], [4, 2, 0], [0, 2, 0]])
    return sc, h


# ---------------------------------------------------------------- the scheme

def test_text_ids_round_trip_and_migration_origin():
    assert E.id_text("face", 0x4a2) == "f:0:4a2"
    assert E.parse_id("f:0:4a2") == ("face", 0x4a2)
    first = E.mint_range(3)
    assert E.parse_id(E.id_text("edge", first))[1] == first
    with pytest.raises(ValueError):
        E.parse_id("x:0:1")
    assert E.is_text_id("v:0:0") and not E.is_text_id("123")


def test_new_elements_get_unique_ids():
    sc, h = history_scene()
    C.run(h, "add_edge", a=[5, 0, 0], b=[6, 0, 0])
    for kind in ("vertex", "edge", "face"):
        got = ids(sc.mesh, kind)
        assert len(got) == len(set(got))


# ---------------------------------------------------------------- file format

def test_format_2_round_trip_keeps_every_id(tmp_path):
    sc, h = history_scene()
    C.run(h, "add_edge", a=[5, 0, 0], b=[6, 0, 0])
    group = Group(name="Box")
    group.xform = Matrix4()
    group.mesh.add_face([V(0, 0, 3), V(1, 0, 3), V(1, 1, 3), V(0, 1, 3)])
    sc.groups.append(group)
    before = all_ids(sc.mesh), all_ids(group.mesh)
    path = tmp_path / "a.built"
    igz.save_scene(sc, path)
    data = manifest(path)
    assert data["built_format"] == 2 and data["requires"] == []
    assert len(data["doc"]["id"]) == 32 and data["doc"]["origins"][0] == "0"
    assert data["scene"]["geom"]["uid"] == E.ROOT_DEFINITION
    proto_uid = data["scene"]["protos"][0]["geom"]["uid"]
    assert len(proto_uid) == 16 and proto_uid != E.ROOT_DEFINITION
    back = Scene()
    igz.load_into(back, path)
    assert back.doc_id == data["doc"]["id"]
    assert (all_ids(touched(back.mesh)), all_ids(touched(back.groups[0].mesh))) == before
    assert E.definition_uid(back.groups[0].mesh) == proto_uid
    # Saved again (another session would remap its origins the same way).
    again = tmp_path / "b.built"
    igz.save_scene(back, again)
    third = Scene()
    igz.load_into(third, again)
    assert manifest(again)["doc"]["id"] == data["doc"]["id"]
    assert all_ids(touched(third.mesh)) == before[0]


def test_untouched_meshes_store_ids_implicitly(tmp_path):
    sc = Scene()
    sc.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    path = tmp_path / "a.built"
    igz.save_scene(sc, path)
    first = manifest(path)["scene"]["geom"]
    back = Scene()
    igz.load_into(back, path)
    igz.save_scene(back, tmp_path / "b.built")
    second = manifest(tmp_path / "b.built")["scene"]["geom"]
    # A block whose ids run first, first + 1, … stores only `first`.
    assert "face" in second.get("ids", {})
    assert set(second["arrays"]) - set(first["arrays"]) <= {"vertex_id", "edge_id", "face_id"}


def test_old_files_get_the_same_migration_ids_every_time(tmp_path):
    sc, h = history_scene()
    C.run(h, "add_edge", a=[5, 0, 0], b=[6, 0, 0])
    path = tmp_path / "old.igz"
    igz.save_scene(sc, path)
    a, b = Scene(), Scene()
    igz.load_into(a, path)
    igz.load_into(b, path)
    got = all_ids(touched(a.mesh))
    assert got == all_ids(touched(b.mesh))
    assert all(i.split(":")[1] == "0" for k in got.values() for i in k)
    # Saved as .built 2, they stay.
    built = tmp_path / "new.built"
    igz.save_scene(a, built)
    c = Scene()
    igz.load_into(c, built)
    assert all_ids(touched(c.mesh)) == got


def test_igz_copy_carries_no_ids(tmp_path):
    sc, _ = history_scene()
    path = tmp_path / "copy.igz"
    igz.save_scene(sc, path)
    text = json.dumps(json.loads(zipfile.ZipFile(path).read("document.json"))
                      if zipfile.is_zipfile(path) else json.loads(path.read_text()))
    assert "vertex_id" not in text and '"doc"' not in text and "_doc_id" not in text


def test_unknown_built_version_is_refused(tmp_path):
    sc, _ = history_scene()
    path = tmp_path / "a.built"
    igz.save_scene(sc, path)
    data = manifest(path)
    data["built_format"] = 3
    bad = tmp_path / "future.built"
    with zipfile.ZipFile(path) as src, zipfile.ZipFile(bad, "w") as dst:
        for info in src.infolist():
            dst.writestr(info, json.dumps(data) if info.filename == "document.json" else src.read(info))
    with pytest.raises(ValueError):
        igz.load_into(Scene(), bad)


def _rewritten(path, out, change):
    with zipfile.ZipFile(path) as src, zipfile.ZipFile(out, "w") as dst:
        data = json.loads(src.read("document.json"))
        change(data)
        for info in src.infolist():
            dst.writestr(info, json.dumps(data) if info.filename == "document.json" else src.read(info))


@pytest.mark.parametrize("change", [
    lambda d: d.__setitem__("requires", ["built.exact_geometry.v1"]),
    lambda d: d.__setitem__("ext", {"built.exact_geometry": {"version": 1}}),
    lambda d: d["scene"]["geom"]["arrays"].__setitem__(
        "face_surface_ref", ["<i4", [1], d["scene"]["geom"]["arrays"]["face_attr"][2]]),
], ids=["requires", "exact-namespace", "face-surface-column"])
def test_features_this_build_lacks_are_refused(tmp_path, change):
    sc, _ = history_scene()
    path = tmp_path / "a.built"
    igz.save_scene(sc, path)
    out = tmp_path / "future.built"
    _rewritten(path, out, change)
    with pytest.raises(ValueError, match="lacks"):
        back = Scene()
        igz.load_into(back, out)
        touched(back.mesh)


def test_the_document_ext_map_survives_a_save(tmp_path):
    sc, _ = history_scene()
    path = tmp_path / "a.built"
    igz.save_scene(sc, path)
    out = tmp_path / "ext.built"
    _rewritten(path, out, lambda d: d.__setitem__("ext", {"later.feature": {"x": 1}}))
    back = Scene()
    igz.load_into(back, out)
    again = tmp_path / "again.built"
    igz.save_scene(back, again)
    assert manifest(again)["ext"] == {"later.feature": {"x": 1}}


# ---------------------------------------------------------------- edits

def test_undo_and_redo_restore_ids_exactly():
    sc, h = history_scene()
    face = sc.mesh.faces[0]
    C.run(h, "push_pull", face=face.key, distance=1.0)
    after = all_ids(sc.mesh)
    C.run(h, "undo")
    C.run(h, "redo")
    assert all_ids(sc.mesh) == after


def test_the_larger_half_of_a_split_face_keeps_its_id():
    sc, h = history_scene()
    (original,) = ids(sc.mesh)
    C.run(h, "add_edge", a=[1, 0, 0], b=[1, 2, 0])
    pieces = faces_by_id(sc.mesh)
    assert len(pieces) == 2
    assert area(pieces[original]) == pytest.approx(6.0)
    step = h.feed.last
    derived = step.meshes[0].derived
    assert {(d["kind"], d["parent"]) for d in derived} >= {("face", original)}


def test_a_face_drawn_inside_leaves_the_outer_its_id():
    sc, h = history_scene()
    (original,) = ids(sc.mesh)
    C.run(h, "add_face", outer=[[1, 0.5, 0], [2, 0.5, 0], [2, 1.5, 0], [1, 1.5, 0]])
    pieces = faces_by_id(sc.mesh)
    assert area(pieces[original]) == pytest.approx(8.0) and pieces[original].hole_loops


def test_a_merge_keeps_the_larger_original():
    sc, h = history_scene()
    C.run(h, "add_edge", a=[1, 0, 0], b=[1, 2, 0])
    big = next(i for i, f in faces_by_id(sc.mesh).items() if area(f) > 5)
    seam = next(e for e in sc.mesh.edges
                if all(abs(v.position_tuple()[0] - 1) < 1e-9 for v in (e.v0, e.v1)))
    C.run(h, "erase", edges=[seam.key], faces=[])
    assert ids(sc.mesh) == [big]


def test_a_pushed_lone_face_moves_with_its_id():
    sc, h = history_scene()
    (original,) = ids(sc.mesh)
    C.run(h, "push_pull", face=sc.mesh.faces[0].key, distance=1.0)
    top = faces_by_id(sc.mesh)[original]
    assert all(abs(v.position_tuple()[2] - 1.0) < 1e-9 for v in top.loop)


def test_erased_ids_are_never_reused():
    sc, h = history_scene()
    gone = set(ids(sc.mesh))
    C.run(h, "erase", edges=[], faces=[sc.mesh.faces[0].key])
    C.run(h, "add_face", outer=[[0, 0, 0], [4, 0, 0], [4, 2, 0], [0, 2, 0]])
    assert not gone & set(ids(sc.mesh))


def test_group_copies_and_make_unique_keep_element_ids():
    sc = Scene()
    h = History(sc)
    g = Group(name="Part")
    g.xform = Matrix4()
    g.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    sc.groups.append(g)
    original = all_ids(g.mesh)
    made = C.run(h, "copy_placements", placements=[g.uid], delta=[3, 0, 0])
    (copy_uid,) = made["placements"]
    copy = next(x for x in sc.groups if x.uid == copy_uid)
    if copy.mesh is not g.mesh:  # a group copy is a new definition
        assert all_ids(touched(copy.mesh)) == original
        assert E.definition_uid(copy.mesh) != E.definition_uid(g.mesh)
    import copy as copying
    duplicate = copying.deepcopy(g.mesh)
    assert all_ids(duplicate) == original
    assert E.definition_uid(duplicate) != E.definition_uid(g.mesh)


def test_make_unique_gives_a_new_definition_with_the_same_element_ids():
    sc = Scene()
    h = History(sc)
    g = Group(name="Part")
    g.xform = Matrix4()
    g.component = True
    g.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    sc.groups.append(g)
    made = C.run(h, "copy_placements", placements=[g.uid], delta=[3, 0, 0])
    (copy_uid,) = made["placements"]
    copy = next(x for x in sc.groups if x.uid == copy_uid)
    shared = E.definition_uid(g.mesh)
    before = all_ids(touched(copy.mesh))
    C.run(h, "make_unique", placement=copy_uid)
    copy = next(x for x in sc.groups if x.uid == copy_uid)
    assert E.definition_uid(copy.mesh) != shared
    assert all_ids(touched(copy.mesh)) == before


def test_explode_keeps_ids_where_free():
    sc = Scene()
    h = History(sc)
    g = Group(name="Part")
    g.xform = Matrix4()
    g.mesh.add_face([V(0, 0, 5), V(1, 0, 5), V(1, 1, 5), V(0, 1, 5)])
    sc.groups.append(g)
    inner = set(ids(touched(g.mesh)))
    C.run(h, "explode_group", placement=g.uid)
    assert inner <= set(ids(sc.mesh))


# ---------------------------------------------------------------- protocol

def test_feed_records_carry_ids_definition_and_document():
    sc, h = history_scene()
    C.run(h, "add_edge", a=[5, 0, 0], b=[6, 0, 0])
    step = h.feed.last
    record = step.to_json()
    assert len(record["document"]) == 32
    mesh = record["meshes"][0]
    assert mesh["definition"] == E.definition_uid(sc.mesh)
    created = mesh["created"]["edge"]
    assert created and all(mesh["ids"]["edge"][k].startswith("e:") for k in created)


def test_queries_and_commands_take_persistent_ids():
    sc, h = history_scene()
    (fid,) = ids(sc.mesh)
    got = Q.faces(sc, [fid])
    assert got["faces"][0]["id"] == fid and not got["missing"]
    resolved = Q.resolve(sc, [fid, "f:0:ffff"])
    assert resolved["keys"][fid] == str(sc.mesh.faces[0].key)
    assert resolved["missing"] == ["f:0:ffff"]
    assert C.run(h, "reverse_faces", faces=[fid])["ok"]
    sel = Q.select(sc, faces=[fid])
    assert Q.selection(sc)["ids"]["face"] == [fid] and not sel["missing"]


# ---------------------------------------------------------------- records

def test_material_uid_survives_save_load_and_rename(tmp_path):
    sc, h = history_scene()
    made = C.run(h, "create_material", color=[0.5, 0.2, 0.1], name="Brick")
    uid = E.material_uid(sc.materials["Brick"])
    C.run(h, "rename_material", name="Brick", new_name="Clay")
    assert sc.materials["Clay"].uid == uid
    path = tmp_path / "m.built"
    igz.save_scene(sc, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.materials["Clay"].uid == uid
    assert made["ok"]


def test_scenes_and_new_section_planes_have_uids(tmp_path):
    from built_core.section import SectionPlane
    from built_core.saved_views import SavedView
    sc, _ = history_scene()
    sc.saved_views.append(SavedView.from_dict(
        {"name": "Iso", "target": [0, 0, 0], "distance": 5, "yaw": 0, "pitch": 0}))
    plane = SectionPlane(point=(0, 0, 0), normal=(0, 0, 1))
    assert isinstance(plane.uid, str) and len(plane.uid) == 16
    sc.section_planes.append(plane)
    path = tmp_path / "s.built"
    igz.save_scene(sc, path)
    data = manifest(path)["scene"]
    view_uid = data["saved_views"][0]["uid"]
    back = Scene()
    igz.load_into(back, path)
    assert back.saved_views[0].uid == view_uid
    assert back.section_planes[0].uid == plane.uid


# ---------------------------------------------------------------- exporters

def test_ifc_global_ids_are_stable_across_sessions_without_a_save(tmp_path):
    from formats.ifc import save_ifc
    from core.ifc_identity import component_guid
    sc = Scene()
    wall = Group(name="Wall")
    wall.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 0, 3), V(0, 0, 3)])
    wall.ifc = {"class": "IfcWall"}
    sc.groups.append(wall)
    path = tmp_path / "w.built"
    igz.save_scene(sc, path)

    def export(scene, name):
        out = tmp_path / name
        save_ifc(scene, out)
        text = out.read_text()
        return {line.split("'")[1] for line in text.splitlines() if "IFCWALL(" in line}

    a, b = Scene(), Scene()
    igz.load_into(a, path)
    first = export(a, "a.ifc")
    igz.load_into(b, path)
    assert export(b, "b.ifc") == first
    assert not a.groups[0].ifc.get("guid")  # the export wrote nothing
    assert component_guid(a, a.groups[0].uid) in first


def test_bcf_components_name_their_ifc_guid(tmp_path):
    from core.ifc_identity import component_guid
    sc = Scene()
    g = Group(name="Wall")
    g.ifc = {"class": "IfcWall", "guid": "0YvctVUKr0kugbFTf53O9L"}
    sc.groups.append(g)
    assert component_guid(sc, g.uid) == "0YvctVUKr0kugbFTf53O9L"
    assert component_guid(sc, f"built:{g.uid}#f:0:1") == "0YvctVUKr0kugbFTf53O9L"
    assert component_guid(sc, "nobody") is None

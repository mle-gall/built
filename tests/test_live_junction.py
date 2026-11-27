# SPDX-License-Identifier: GPL-3.0-or-later
from copy import deepcopy

from built_core import command_protocol as live
from built_core.history import History
from built_core.protocol import commands
from built_core.scene import Scene
from core.ifc_identity import occurrence_guid
from formats import igz
from tests.test_live_cut import make_member
from tests.test_live_recipes import apply
from tests.test_persistent_ids import all_ids


def corner():
    return dict(uid="c081000000000001", rev=1, name="Corner mitre", primitive="junction",
                pack="Project", locks={}, record={"class": "IfcElementAssembly"},
                values=dict(meeting="corner", roles={"main": {}, "joining": {}},
                            params=[dict(key="gap", expr="main.width / 45", unit="mm")],
                            contents=[dict(key=f"{role}_trim", on=role, row={
                                "kind": "cut", "operation": "trim", "end": f"@{role}.end",
                                "cutter": {"plane": {}},
                                "anchor": {"ref": "@meet", "offset": "gap"}})
                                for role in ("main", "joining")]))


def setup():
    scene = Scene()
    h = History(scene)
    make_member(h, [[0, 0, 0], [2, 0, 0]])
    make_member(h, [[2, 0, 0], [2, 3, 0]])
    a, b = scene.groups
    detail = corner()
    apply(h, "preset_update", rows=[detail])
    params = dict(main=dict(placement=a.uid, end="end"), joining=dict(placement=b.uid, end="start"))
    uid = apply(h, "junction_apply", detail=detail["uid"], bindings=params)["uid"]
    return scene, h, a, b, uid


def junction(scene):
    return next(r for r in live.live_rows(scene) if r["kind"] == "junction")


def test_corner_follows_members_and_keeps_identity(tmp_path):
    scene, h, a, b, uid = setup()
    j = junction(scene)
    assert j["state"] == "ok" and j["params"] == {"gap": 1}
    assert live.junction_frames(scene, uid)["meet"][12:15] == [2., 0., 0.]
    guid = occurrence_guid(scene, [j["owner"]])
    ids = (all_ids(a.mesh), all_ids(b.mesh), j["owner"], j["contents"])
    apply(h, "move_placements", placements=[b.uid], delta=[0.2, 0, 0])
    assert live.junction_frames(scene, uid)["meet"][12] == 2.1
    row = live.live_sweep(scene, a.uid)
    row["profile"]["params"]["width"] = 90.
    apply(h, "live_set", row=row)
    assert junction(scene)["params"] == {"gap": 2}
    for verb, gap in [("undo", 1), ("redo", 2)]:
        apply(h, verb)
        assert junction(scene)["params"]["gap"] == gap
        assert occurrence_guid(scene, [junction(scene)["owner"]]) == guid
        assert ids == (all_ids(a.mesh), all_ids(b.mesh), junction(scene)["owner"], junction(scene)["contents"])
    path = tmp_path / "corner.built"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert junction(back) == junction(scene)
    apply(History(back), "junction_release", uid=uid)
    cuts = [r for r in live.live_rows(back) if r["kind"] == "cut"]
    assert len(cuts) == 2 and all("junction" not in r and "anchor" not in r for r in cuts)


def test_detail_diff_and_lost_role_keep_last_good():
    scene, h, a, b, uid = setup()
    j = junction(scene)
    apply(h, "junction_set", row=dict(j, overrides=dict(gap=3)))
    detail = deepcopy(corner())
    detail["values"]["contents"].pop()
    apply(h, "preset_update", rows=[detail])
    assert junction(scene)["contents"] == j["contents"][:1]
    assert junction(scene)["params"] == {"gap": 3}
    parts = a.mesh
    apply(h, "delete_placements", placements=[b.uid])
    assert junction(scene)["state"] == "lost"
    assert a.mesh is parts
    apply(h, "undo")
    assert junction(scene)["state"] == "ok"
    result = commands.run(h, "junction_set", row=dict(junction(scene), overrides=dict(gap="gap + 1")))
    assert not result["ok"] and "cycle" in str(result).lower()


def test_failed_cut_is_local_to_its_junction():
    scene, h, a, b, uid = setup()
    bad = corner()
    bad.update(uid="c081000000000002", name="End trim")
    bad["values"]["contents"] = [dict(key="trim", on="main", row={
        "kind": "cut", "operation": "trim", "end": "start", "cutter": {"plane": {}},
        "anchor": {"ref": "@main.contact"}})]
    apply(h, "preset_update", rows=[bad])
    last = a.mesh
    other = apply(h, "junction_apply", detail=bad["uid"], bindings={
        "main": {"placement": a.uid, "end": "start"},
        "joining": {"placement": b.uid, "end": "start"}})["uid"]
    rows = {r["uid"]: r for r in live.live_rows(scene)}
    assert rows[uid]["state"] == "ok"
    assert rows[other]["state"] == "failed"
    assert rows[other]["reason"]["child"] == rows[other]["contents"][0]["row"]
    assert a.mesh is last


def test_size_rule_limits_and_bound_field_edits():
    scene, h, a, _, uid = setup()
    detail = corner()
    detail["values"]["rule"] = {"roles": {"main": {"size": {"width": [40, 60]}}}}
    detail["values"]["params"][0]["limits"] = {"range": [0, 5]}
    apply(h, "preset_update", rows=[detail])
    edited = next(r for r in live.live_rows(scene) if r.get("junction") == uid)
    edited["anchor"]["offset"] = 4
    assert not commands.run(h, "live_set", row=edited)["ok"]
    apply(h, "junction_set", row=dict(junction(scene), overrides={"gap": 8}))
    assert junction(scene)["state"] == "failed"
    assert junction(scene)["reason"] == {"criterion": "limit"}
    assert junction(scene)["params"] == {"gap": 5}
    row = live.live_sweep(scene, a.uid)
    row["profile"]["params"]["width"] = 75
    apply(h, "live_set", row=row)
    assert junction(scene)["state"] == "outside"
    assert junction(scene)["reason"]["value"] == 75
    assert junction(scene)["params"] == {"gap": 5}


def test_deleting_assembly_removes_its_cuts_in_one_step():
    scene, h, a, b, uid = setup()
    owner = junction(scene)["owner"]
    ids = all_ids(a.mesh), all_ids(b.mesh)
    apply(h, "delete_placements", placements=[owner])
    assert all(r["kind"] == "sweep" and not r.get("cuts") for r in live.live_rows(scene))
    apply(h, "undo")
    assert junction(scene)["uid"] == uid
    assert ids == (all_ids(a.mesh), all_ids(b.mesh))


def test_local_child_identity_reexpansion_and_deep_release():
    scene, h, _, _, uid = setup()
    parent = junction(scene)
    detail = corner()
    detail["uid"] = "c081000000000003"
    detail["values"]["contents"] = []
    apply(h, "preset_update", rows=[detail])
    child = apply(h, "junction_set", row=dict(detail=detail["uid"], parent=uid, key="child",
                                               roles=parent["roles"]))["uid"]
    rows = {r["uid"]: r for r in live.live_rows(scene)}
    assert rows[uid]["local"]["added"] == [child]
    assert rows[uid]["contents"][-1]["placement"] == rows[child]["owner"]
    apply(h, "preset_update", rows=[corner()])
    rows = {r["uid"]: r for r in live.live_rows(scene)}
    assert rows[uid]["contents"][-1]["junction"] == child
    revision = h.revision
    assert not commands.run(h, "junction_set", row=dict(rows[uid], parent=child, key="cycle"))["ok"]
    assert h.revision == revision
    apply(h, "junction_release", uid=uid, deep=True)
    assert not live.live_rows(scene)
    apply(h, "undo")
    assert {r["uid"] for r in live.live_rows(scene)} == rows.keys()
    apply(h, "delete_placements", placements=[rows[child]["owner"]])
    assert child not in junction(scene)["local"]["added"]


def test_detached_junction_keeps_frames_cuts_and_values():
    scene, h, a, b, uid = setup()
    before = junction(scene)
    apply(h, "junction_set", row=dict(before, detail=None))
    detail = corner()
    detail["values"]["params"][0]["expr"] = "5"
    apply(h, "preset_update", rows=[detail])
    apply(h, "move_placements", placements=[b.uid], delta=[.2, 0, 0])
    detached = junction(scene)
    assert detached["state"] == "ok" and not detached.get("detail")
    assert detached["contents"] == before["contents"] and detached["params"] == {"gap": 1}
    assert live.junction_frames(scene, uid)["meet"][12] == 2.1


def test_stock_face_and_repeat_station_bindings():
    from tests.test_live_repeat import repeat
    from tests.test_persistent_ids import face_id

    scene, h, a, b, _ = setup()
    detail = corner()
    detail["values"]["contents"] = []
    apply(h, "preset_update", rows=[detail])
    j = junction(scene)
    face = face_id(a.mesh, a.mesh.faces[0])
    roles = deepcopy(j["roles"])
    roles["main"]["face"] = face
    apply(h, "junction_set", row=dict(j, roles=roles))
    assert junction(scene)["state"] == "ok"
    row = repeat(b)
    apply(h, "live_set", row=row)
    roles["joining"] = dict(row=row["uid"], station="2", end="start")
    apply(h, "junction_set", row=dict(junction(scene), roles=roles))
    assert junction(scene)["state"] == "ok"
    assert live.junction_frames(scene, j["uid"])["joining.end"][12:15] == [3., 0., 0.]
    apply(h, "live_set", row=dict(row, skips=[{"from": "start", "at": 1000}]))
    assert junction(scene)["state"] == "lost"


def test_member_tree_links_to_assembly():
    from tests.test_project_drawing_filters import Project

    scene, _, a, _, _ = setup()
    project = Project(scene=scene)
    owner_key = "i:" + a.uid
    rows = project.rows()
    if owner_key not in rows:
        rows = project.action("toggle", "n:")
    rows = project.action("toggle", owner_key)
    links = [key for key, (name, _) in rows.items() if key.startswith("g:") and name == "Corner mitre"]
    assert len(links) == 1
    assert rows[links[0]][1][0] == rows[owner_key][1][0] + 1


def test_owned_part_gets_a_stable_placement_and_releases_in_place(tmp_path):
    import hashlib

    scene, h, a, _, uid = setup()
    parent = junction(scene)
    sweep = live.live_sweep(scene, a.uid)
    apply(h, "live_set", row=dict(kind="sweep", junction=uid, key="plate",
                                  path=sweep["path"], profile=sweep["profile"]))
    derive = lambda key: hashlib.sha256(f"built:junction:v1:{uid}:{key}".encode()).hexdigest()[:16]
    part = next(r for r in live.live_rows(scene) if r.get("key") == "plate")
    assert part["uid"] == derive("plate") and part["owner"] == derive("plate:owner")
    assembly = scene.groups_by_uid()[parent["owner"]]
    assert [g.uid for g in assembly.children] == [part["owner"]]
    path = tmp_path / "owned.built"
    igz.save_scene(scene, path)
    reopened = Scene()
    igz.load_into(reopened, path)
    assert all(r["state"] == "ok" for r in live.live_rows(reopened))
    part_ids = all_ids(assembly.children[0].mesh)
    apply(h, "junction_release", uid=uid)
    released = next(r for r in live.live_rows(scene) if r["uid"] == part["uid"])
    assert "junction" not in released and all_ids(assembly.children[0].mesh) == part_ids
    apply(h, "undo")
    assert junction(scene)["contents"][-1]["row"] == part["uid"]


def test_detail_child_tracks_keys_and_depth_limit():
    scene, history, _, _, parent_uid = setup()
    detail = corner()
    child = deepcopy(detail)
    child["uid"] = "c081000000000031"
    child["values"]["contents"] = []
    detail["values"]["contents"] = [dict(key="nested", junction=dict(
        detail=child["uid"], roles={"main": "@main", "joining": "@joining"}))]
    apply(history, "preset_update", rows=[child, detail])
    rows = {row["uid"]: row for row in live.live_rows(scene)}
    nested = rows[parent_uid]["contents"][0]["junction"]
    assert rows[nested]["parent"] == parent_uid
    assert rows[nested]["roles"] == rows[parent_uid]["roles"]
    parent = nested
    for _ in range(6):
        parent = apply(history, "junction_set", row=dict(
            detail=child["uid"], parent=parent, key="nested", roles=rows[nested]["roles"]))["uid"]
    revision = history.revision
    result = commands.run(history, "junction_set", row=dict(
        detail=child["uid"], parent=parent, key="nested", roles=rows[nested]["roles"]))
    assert not result["ok"] and "8 levels" in str(result)
    assert history.revision == revision
    detail["values"]["contents"] = []
    apply(history, "preset_update", rows=[detail])
    assert [r["uid"] for r in live.live_rows(scene) if r["kind"] == "junction"] == [parent_uid]
    assert not scene.groups_by_uid()[rows[parent_uid]["owner"]].children


def knee_fixture(stage=3, bead=False, portal=False):
    """Build the three knee storyboard states through the same History verbs."""
    from tests.test_sketch_rows import member

    scene = Scene()
    history = History(scene)
    if portal:
        members = [member({"library": "en/he200a"}, [[0, 0, 0], [0, 0, 1.1]]),
                   member({"library": "en/ipe240"}, [[0, 0, .8], [0, 1, 1.068]])]
    else:
        members = [member({"library": "en/he200a"}, [[0, 0, 0], [2, 0, 0]]),
                   member({"library": "basic/timber"}, [[1, 0, 0], [1, 0, 1]])]
    apply(history, "create_sweeps", sweeps=members)
    column, beam = scene.groups
    plate = next(p for p, _ in live.preset_catalog(scene) if p["name"] == "Plate")

    def detail(n, name, roles, contents):
        return dict(uid=f"c081{n:012x}", name=name, primitive="junction", pack="Project",
                    rev=1, locks={}, record={"class": "IfcElementAssembly"},
                    values=dict(meeting="parts", roles=dict.fromkeys(roles, {}), contents=contents))

    # The head and bores start on the plate outer face, clear of the column web.
    origin = [-55, -55, 0]
    bolt = detail(18, "Bolt M16", ["ply_1", "ply_2"], [
        dict(key="bolt", part={"fastener": {"d": 16, "clearance": 2, "grade": "8.8"},
                               "at": {"frame": "@meet", "offset": origin}})])
    weld = detail(19, "Fillet weld", ["a", "b"], [
        dict(key="seam", part={"weld": {"throat": 4, "type": "fillet", "bead": bead}})])
    root = detail(20, "End plate", ["main", "joining"], [
        dict(key="end_cut", on="joining", row={"kind": "cut", "end": "start",
             "operation": "trim", "cutter": {"plane": {}}, "anchor": {"ref": "@meet", "offset": 12}})])
    root["values"]["meeting"] = "end_face"
    root["values"]["params"] = [dict(key="plate_width", expr="joining.width + 150", unit="mm")]
    if stage >= 2:
        root["values"]["contents"].append(dict(key="plate", part={
            "preset": plate["uid"], "fields": {"profile.params.width": "plate_width",
            "profile.params.height": 300 if portal else 190, "thickness": 12}, "at": {"frame": "@meet"}}))
    if stage >= 3:
        root["values"]["contents"].extend([
            dict(key="bolts", junction={"detail": bolt["uid"], "roles": {"ply_1": "@plate", "ply_2": "@main"},
                 "repeat": {"rule": {"count": 2, "step": 110}, "run": "@meet.y",
                            "nest": {"count": 2, "step": 110}}}),
            dict(key="weld", junction={"detail": weld["uid"], "roles": {
                "a": "@joining", "b": {"placement": "@plate", "face": "end"}}})])
    apply(history, "preset_update", rows=[bolt, weld, root])
    uid = apply(history, "junction_apply", detail=root["uid"], bindings={
        "main": dict(placement=column.uid, face="flange_outer"),
        "joining": dict(placement=beam.uid, end="start")})["uid"]
    return scene, history, uid


def test_knee_through_protocol(tmp_path):
    scene, history, uid = knee_fixture()
    rows = {r["uid"]: r for r in live.live_rows(scene)}
    assert all(r["state"] == "ok" for r in rows.values()), [(r.get("key"), r["state"], r.get("reason")) for r in rows.values()]
    groups = scene.groups_by_uid()
    parts = {r["key"]: r for r in rows.values() if "key" in r}
    assert groups[parts["plate"]["owner"]].ifc["class"] == "IfcPlate"
    assert parts["plate"]["profile"]["params"]["width"] == 195
    assert len([r for r in rows.values() if "hole" in r.get("cutter", {})]) == 2
    bolts = groups[parts["bolts"]["owner"]]
    copies = groups[parts["bolts:repeat"]["owner"]].children
    assert len(copies) == 3 and all(c.children[0].mesh is bolts.children[0].mesh for c in copies)
    assert bolts.children[0].ifc["predefined_type"] == "BOLT"
    seam = groups[parts["weld"]["owner"]].children[0]
    assert parts["weld"]["params"]["length"] == seam.ifc["length"]
    assert abs(seam.ifc["length"] - 380) < .01 and len(seam.mesh.edges) == 4
    path = tmp_path / "knee.built"
    igz.save_scene(scene, path)
    reopened = Scene()
    igz.load_into(reopened, path)
    assert all(r["state"] == "ok" for r in live.live_rows(reopened))
    apply(history, "undo")
    assert not any(r["kind"] == "junction" for r in live.live_rows(scene))
    apply(history, "redo")
    assert {r["uid"] for r in live.live_rows(scene)} == rows.keys()


def test_component_glue_and_bead_records():
    scene, history, parent = knee_fixture(bead=True)
    groups = scene.groups_by_uid()
    rows = {r["uid"]: r for r in live.live_rows(scene)}
    weld = next(r for r in rows.values() if r.get("key") == "weld")
    bead = groups[weld["owner"]].children[0]
    assert len(bead.mesh.faces) == 12
    glue = corner()
    glue["uid"] = "c081000000000021"
    glue["values"] = dict(meeting="parts", roles={"a": {}, "b": {}}, contents=[
        dict(key="bond", part={"glue": {}}),
        dict(key="bracket", part={"component": bead.uid, "at": {"frame": "@meet"}})])
    apply(history, "preset_update", rows=[glue])
    child = apply(history, "junction_set", row={"parent": parent, "key": "bond",
                  "detail": glue["uid"], "roles": weld["roles"]})["uid"]
    row = next(r for r in live.live_rows(scene) if r["uid"] == child)
    bond, bracket = scene.groups_by_uid()[row["owner"]].children
    assert bond.ifc["predefined_type"] == "GLUE" and abs(bond.ifc["area"] - 45 * 145) < .01
    assert row["params"]["area"] == bond.ifc["area"]
    assert not bond.mesh.faces
    assert bead.mesh is bracket.mesh and bead.uid != bracket.uid


def test_released_bolt_grid_keeps_holes_at_its_stations():
    scene, history, uid = knee_fixture()
    grid = next(r for r in live.live_rows(scene) if r["kind"] == "repeat")
    apply(history, "live_release", uid=grid["uid"])
    holes = [r["cutter"]["hole"] for r in live.live_rows(scene) if "hole" in r.get("cutter", {})]
    assert len(holes) == 2
    assert all(not h.get("repeat") and len(h["stations"]) == 3 for h in holes)
    assert all(r["state"] == "ok" for r in live.live_rows(scene))
    apply(history, "undo")
    assert len(next(r for r in live.live_rows(scene) if r["uid"] == uid)["contents"]) == 5


def test_removing_a_record_part_survives_detail_update():
    scene, history, _ = knee_fixture()
    weld = next(r for r in live.live_rows(scene) if r.get("key") == "weld")
    part = scene.groups_by_uid()[weld["owner"]].children[0]
    apply(history, "delete_placements", placements=[part.uid])
    current = next(r for r in live.live_rows(scene) if r["uid"] == weld["uid"])
    assert current["local"]["removed"] == ["seam"]
    detail = next(p for p, _ in live.preset_catalog(scene) if p["uid"] == weld["detail"]["uid"])
    apply(history, "preset_update", rows=[detail])
    assert not scene.groups_by_uid()[weld["owner"]].children
    assert all(r["state"] == "ok" for r in live.live_rows(scene))


def test_fastener_resize_preserves_stations_and_refreshes_both_plies():
    scene, history, _ = knee_fixture(portal=True)
    rows = {r.get("key"): r for r in live.live_rows(scene)}
    assembly = scene.groups_by_uid()[rows["bolts"]["owner"]]
    grid = scene.groups_by_uid()[rows["bolts:repeat"]["owner"]]
    bolt = assembly.children[0]
    old_mesh = bolt.mesh
    stations = [(c.uid, c.children[0].uid) for c in grid.children]
    detail = next(p for p, _ in live.preset_catalog(scene) if p["uid"] == rows["bolts"]["detail"]["uid"])
    detail["values"]["contents"][0]["part"]["fastener"]["d"] = 12
    apply(history, "preset_update", rows=[detail])
    assert bolt.ifc["d"] == 12 and bolt.mesh is not old_mesh
    assert [(c.uid, c.children[0].uid) for c in grid.children] == stations
    assert all(c.children[0].mesh is bolt.mesh for c in grid.children)
    assert [r["cutter"]["hole"]["d"] for r in live.live_rows(scene)
            if "hole" in r.get("cutter", {})] == [14, 14]
    assert all(r["state"] == "ok" for r in live.live_rows(scene))
    apply(history, "undo")
    assert bolt.mesh is old_mesh


def test_glue_only_counts_the_overlap_of_the_two_stock_faces():
    scene, history, parent = knee_fixture(stage=2)
    root = next(r for r in live.live_rows(scene) if r["uid"] == parent)
    apply(history, "junction_set", row=dict(root, overrides={"plate_width": 20}))
    rows = {r["uid"]: r for r in live.live_rows(scene)}
    plate = next(r for r in rows.values() if r.get("key") == "plate")
    glue = corner()
    glue["uid"] = "c081000000000022"
    glue["values"] = dict(meeting="parts", roles={"a": {}, "b": {}},
                          contents=[dict(key="bond", part={"glue": {}})])
    apply(history, "preset_update", rows=[glue])
    child = apply(history, "junction_set", row=dict(parent=parent, key="bond", detail=glue["uid"],
                  roles={"a": root["roles"]["joining"], "b": {"placement": plate["owner"], "face": "end"}}))["uid"]
    row = next(r for r in live.live_rows(scene) if r["uid"] == child)
    assert row["state"] == "ok" and abs(row["params"]["area"] - 20 * 145) < .01


def test_moving_the_portal_knee_keeps_the_bolt_grid_and_weld(capfd):
    import os
    import re
    from statistics import median

    scene, history, uid = knee_fixture(portal=True)
    rows = {r["key"]: r for r in live.live_rows(scene) if "key" in r}
    grid = scene.groups_by_uid()[rows["bolts:repeat"]["owner"]]
    stations = [g.uid for g in grid.children]
    definition = grid.children[0].children[0].mesh
    capfd.readouterr()
    for i in range(21):
        apply(history, "move_placements", placements=[scene.groups[1].uid],
              delta=[.001 if i % 2 == 0 else -.001, 0, 0])
    assert [g.uid for g in grid.children] == stations
    assert all(g.children[0].mesh is definition for g in grid.children)
    assert all(r["state"] == "ok" for r in live.live_rows(scene))
    if os.environ.get("BUILT_PERF"):
        samples = re.findall(r"Junction live evaluation ([\d.]+) ms", capfd.readouterr().err)
        assert len(samples) == 21
        print(f"One knee, median live evaluation {median(map(float, samples)):.3f} ms")


def test_wall_tool_applies_corners_only_to_the_new_wall():
    scene = Scene()
    h = History(scene)
    def wall(a, b, tool=False):
        apply(h, "create_sweeps", sweeps=[dict(profile=dict(library="basic/wall"),
              path=dict(points=[a, b]))], wall_junctions=tool)
    wall([0, 0, 0], [2, 0, 0])
    wall([0, 0, 0], [0, 1, 0])
    assert not any(r["kind"] == "junction" for r in live.live_rows(scene))
    wall([2, 0, 0], [2, 1, 0], True)
    j = junction(scene)
    assert j["state"] == "ok" and j["detail"]["name"] == "Wall corner · by priority"
    assert len(j["contents"]) == 2 and len(live.junction_markers(scene)) == 1
    apply(h, "undo")
    assert len(scene.groups) == 2
    wall([1, 0, 0], [1, 1, 0], True)
    assert junction(scene)["detail"]["name"] == "Wall tee"


def test_window_lintel_and_lining_follow_wall_stock(tmp_path):
    from built_core import Matrix4, Vec3
    from built_core.group import Group
    from tests.test_geometry_arrays import CUBE
    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[dict(profile=dict(library="basic/wall",
          params=dict(width=200, height=3000)), attachment="bottom_left",
          path=dict(points=[[0, 0, 0], [6, 0, 0]]))])
    wall, = scene.groups
    fill = Group(name="Window")
    fill.xform = Matrix4()
    fill.ifc = {"class": "IfcWindow"}
    fill.ext = {"library_host": "wall", "junction": "5c9f9bb202d14a8c"}
    for face in CUBE:
        fill.mesh.add_face([Vec3(x, .12*y, z) for x, y, z in face])
    apply(h, "place_group", template=fill.uid, _pending=fill,
          basis=[[1,0,0],[0,1,0],[0,0,1]], offset=[2,0,1],
          wall_host=dict(uid=wall.uid, outline=[[2,0,1],[3,0,1],[3,0,2],[2,0,2]],
                         normal=[0,-1,0], initial_offset=0, offset=0, sill=1, flipped=False))
    j = junction(scene)
    assert j["state"] == "ok", j.get("reason")
    assert j["params"]["frame_depth"] == 200 and j["params"]["lintel_len"] == 1300
    rows = {r["key"]: r for r in live.live_rows(scene) if r.get("junction") == j["uid"]}
    assert rows["lintel"]["path"]["points"][1] == [1.3, 0, 0]
    assert {"frame", "sill", "lining_left", "lining_right", "reveal_left", "reveal_right"} <= rows.keys()
    sweep = live.live_sweep(scene, wall.uid)
    sweep["path"]["points"][1][0] = 8
    sweep["profile"]["params"]["width"] = 300
    apply(h, "live_set", row=sweep)
    j = junction(scene)
    assert j["params"]["frame_depth"] == 300
    assert fill.xform.data()[12:15] == (2., 0., 1.)
    assert wall.ifc["openings"][0]["wall_rect"][0] == .25
    apply(h, "move_placements", placements=[wall.uid], delta=[1, 2, 0])
    assert fill.xform.data()[12:15] == (3., 2., 1.)
    apply(h, "undo")
    assert fill.xform.data()[12:15] == (2., 0., 1.)


def test_wall_on_slab_contact_and_optional_trim():
    from built_core import Matrix4, Vec3
    from built_core.group import Group
    from tests.test_geometry_arrays import CUBE
    from tests.test_live_cut import solid_volume
    import pytest
    scene = Scene()
    h = History(scene)
    slab = Group(name="Slab")
    slab.xform = Matrix4()
    slab.ifc = {"class": "IfcSlab"}
    for face in CUBE:
        slab.mesh.add_face([Vec3(x*4-1, y*4-2, z*.25) for x, y, z in face])
    scene.groups.append(slab)
    apply(h, "create_sweeps", sweeps=[dict(profile=dict(library="basic/wall"),
          attachment="bottom_left", path=dict(points=[[0, 0, 0], [2, 0, 0]]))])
    wall = scene.groups[1]
    stock = solid_volume(wall)
    for name in ["Wall on slab", "Wall on slab · trim"]:
        p = next(p for p, _ in live.preset_catalog(scene) if p["name"] == name)
        apply(h, "junction_apply", detail=p["uid"], bindings={
            "main": {"placement": slab.uid}, "joining": {"placement": wall.uid}})
        j = junction(scene)
        assert j["state"] == "ok" and j["frames"]["meet"][14] == pytest.approx(.25)
        expected = stock - .25*.2*2 if j["contents"] else stock
        assert solid_volume(wall) == pytest.approx(expected)
        apply(h, "undo")

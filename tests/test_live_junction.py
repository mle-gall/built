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

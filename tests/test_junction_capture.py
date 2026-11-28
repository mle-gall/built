# SPDX-License-Identifier: GPL-3.0-or-later
"""Capture and editing (docs/junctions.md §5, §6, lane J2): a knee made by
hand (a cut, a plate, a bolt group) saved as a detail, applied to the other
knees, edited for one use and for all, released."""
from collections import Counter

import pytest

from built_core import Vec3, command_protocol as live
from built_core.history import History  # noqa: F401  (fixtures build through History)
from built_core.protocol import commands
from core.ifc_identity import occurrence_guid
from formats import igz
from tests.test_junction_meetings import first_knee, garage, knee_details
from tests.test_live_recipes import apply
from tests.test_persistent_ids import face_id


def xyz(v):
    return tuple(c() if callable(c) else c for c in (v.x, v.y, v.z))


def world_points(scene, g):
    """World positions of a placement's vertices, rounded to 1 µm."""
    chain, node = [], g
    while node is not None:
        if node.xform is not None:
            chain.append(node.xform)
        node = node.owner
    out = []
    for v in g.mesh.vertices:
        q = Vec3(*v.position_tuple())
        for m in chain:
            q = m.map(q)
        out.append(tuple(round(c, 6) for c in xyz(q)))
    return sorted(out)


def facing_face(column, toward):
    """The outermost column face whose world normal points along `toward`
    (a member's own axes follow its profile, not the world's)."""
    m = column.xform

    def world(p):
        return xyz(m.map(Vec3(*xyz(p)))) if m is not None else xyz(p)

    def direction(v):
        o, e = world(Vec3(0, 0, 0)), world(v)
        return tuple(b - a for a, b in zip(o, e))

    def dot(v):
        return sum(a * b for a, b in zip(v, toward))
    faces = [f for f in column.mesh.faces if dot(direction(f.normal())) > 0.99]
    best = max(faces, key=lambda f: dot(world(f.centroid() if callable(f.centroid) else f.centroid)))
    return face_id(column.mesh, best)


def bolt_group(scene):
    bolt, _, _ = knee_details(scene)
    group = dict(uid="c0a2000000000001", name="Bolt group M16 · 2 × 2", primitive="junction", pack="Project",
                 rev=1, locks={}, record={"class": "IfcElementAssembly"},
                 values=dict(meeting="parts", roles={"ply_1": {}, "ply_2": {}}, contents=[
                     dict(key="bolt", junction={"detail": bolt["uid"], "roles": {"ply_1": "@ply_1", "ply_2": "@ply_2"},
                          "repeat": {"rule": {"count": 2, "step": 110}, "run": "@meet.y",
                                     "nest": {"count": 2, "step": 110}}})]))
    return bolt, group


def handmade_knee(scene, history, bolts=True):
    """What a user makes at the first knee with the Cut, Sweep and Junction
    tools: an end cut on the rafter 15 mm off the column flange, a 15 mm
    plate on that flange, and a bolt group through plate and flange."""
    source = first_knee(scene)
    groups = scene.groups_by_uid()
    column, rafter = groups[source["members"][0]], groups[source["members"][1]]
    flange = facing_face(column, (0, 1, 0))
    ref = {"path": [column.uid], "id": flange}
    cut_uid = "c0a2000000000101"
    apply(history, "live_set", row=dict(kind="cut", uid=cut_uid, owner=rafter.uid, name="End plate",
                                        operation="trim", end=source["roles"]["other"]["end"],
                                        cutter={"plane": {}}, anchor={"ref": ref, "offset": 15}, on=True))
    plate = next(p for p, _ in live.preset_catalog(scene) if p["name"] == "Plate")
    p0 = [0, 0.095, 3.0 + 0.095 * 0.2679491924311227]
    p1 = [0, 0.110, p0[2]]
    apply(history, "create_sweeps", sweeps=[dict(
        preset_uid=plate["uid"], profile={"library": "basic/flat", "params": {"width": 150, "height": 360}},
        attachment="centre", path={"points": [p0, p1], "ends": [{"to": ref, "extend": True}, p1]})])
    pl = scene.groups[-1]
    pl.name = "PL-1"
    group_uid = None
    if bolts:
        bolt, group = bolt_group(scene)
        apply(history, "preset_update", rows=[bolt, group])
        group_uid = apply(history, "junction_apply", detail=group["uid"], bindings={
            "ply_1": {"placement": pl.uid}, "ply_2": {"placement": column.uid, "face": "flange_outer"}})["uid"]
    return source, column, rafter, pl, cut_uid, group_uid


def rows_by_uid(scene):
    return {r["uid"]: r for r in live.live_rows(scene)}


def test_capture_preview_gathers_cut_plate_and_bolts_with_relations():
    scene, history = garage(purlins=False)
    source, column, rafter, pl, cut_uid, group_uid = handmade_knee(scene, history)
    draft = live.junction_capture_preview(scene, cut_uid)
    items = {i["id"]: i for i in draft["items"]}
    assert set(items) == {cut_uid, pl.uid, group_uid}, draft["items"]
    assert all(i["ticked"] for i in items.values()), draft["items"]
    assert items[cut_uid]["how"] == "anchored" and items[cut_uid]["key"] == "end_cut"
    params = {p["key"]: p for p in draft["params"]}
    assert params["plate_width"]["expr"] == "joining.width + 30"
    assert params["plate_height"]["expr"] == "joining.height + 120"
    assert params["plate_t"]["value"] == pytest.approx(15)
    assert params["gap"]["expr"] == "plate_t", params["gap"]
    rule = draft["detail"]["values"]["rule"]
    assert rule["angle"] == {"value": 75.0, "tol": 2.0} and rule["meeting"] == "end_face"
    assert {p["library"] for p in rule["roles"]["joining"]["profile"]} == {"en/ipe220", "en/ipe240", "en/ipe270"}
    assert rule["roles"]["main"]["face"] == ["flange_outer"]
    assert draft["similar"] == 7 and draft["names"][0] == column.name
    # Seeds from the plate, the bolt group and the two members agree.
    for seed in (pl.uid, group_uid, [column.uid, rafter.uid]):
        again = live.junction_capture_preview(scene, seed)
        assert {i["id"] for i in again["items"]} == set(items), seed
    # Unticking the bolts drops them and their parameters; nothing is written.
    revision = history.revision
    less = live.junction_capture_preview(scene, cut_uid, {"include": [cut_uid, pl.uid]})
    assert len(less["detail"]["values"]["contents"]) == 2 and history.revision == revision
    swapped = live.junction_capture_preview(scene, cut_uid, {"swap": True})
    assert swapped["members"] == [rafter.uid, column.uid]


def test_capture_keeps_uids_geometry_and_guids_in_one_step():
    scene, history = garage(purlins=False)
    source, column, rafter, pl, cut_uid, group_uid = handmade_knee(scene, history)
    before = {g.uid: world_points(scene, g) for g in (column, rafter)}
    plate_mesh = [c for p in world_points(scene, pl) for c in p]
    guid = occurrence_guid(scene, scene.occurrence_path(pl))
    revision = history.revision
    reply = apply(history, "junction_capture", seed=cut_uid, options={"name": "Knee end plate"})
    assert history.revision == revision + 1
    rows = rows_by_uid(scene)
    j = rows[reply["uid"]]
    assert j["state"] == "ok", (j["state"], j.get("reason"))
    assert j["detail"]["uid"] == reply["detail"] and j["detail"]["name"] == "Knee end plate"
    assert rows[cut_uid]["junction"] == reply["uid"] and rows[cut_uid]["key"] == "end_cut"
    assert rows[cut_uid]["anchor"]["ref"] == {"junction": reply["uid"], "frame": "meet"}
    assert rows[group_uid]["parent"] == reply["uid"]
    groups = scene.groups_by_uid()
    assembly = groups[j["owner"]]
    assert pl.uid in [c.uid for c in assembly.children]
    assert pl.ifc["guid"] == guid
    # Nothing moved: the members and the plate are where they were.
    for g in (column, rafter):
        assert world_points(scene, g) == before[g.uid]
    assert [c for p in world_points(scene, groups[pl.uid]) for c in p] == pytest.approx(plate_mesh, abs=1e-5)
    assert all(r["state"] == "ok" for r in rows.values()), [(r.get("key"), r["state"], r.get("reason")) for r in rows.values() if r["state"] != "ok"]
    apply(history, "undo")
    rows = rows_by_uid(scene)
    assert reply["uid"] not in rows and "junction" not in rows[cut_uid]
    apply(history, "redo")
    assert rows_by_uid(scene)[cut_uid]["junction"] == reply["uid"]


def captured_garage():
    scene, history = garage(purlins=False)
    knee = handmade_knee(scene, history)
    reply = apply(history, "junction_capture", seed=knee[4], options={"name": "Knee end plate"})
    return scene, history, knee, reply


def test_captured_detail_applies_to_the_other_knees_with_the_same_parts():
    scene, history, knee, reply = captured_garage()
    detail = reply["detail"]
    fits = live.junction_suggest(scene, detail)["suggestions"]
    assert Counter(s["fit"] for s in fits) == {"applied": 1, "match": 7}
    matches = [s for s in fits if s["fit"] == "match"]
    assert sum(s["mirrored"] for s in matches) == 4
    apply(history, "junction_apply", detail=detail, bindings=[s["roles"] for s in matches])
    rows = live.live_rows(scene)
    uses = [r for r in rows if r["kind"] == "junction" and r.get("detail", {}).get("uid") == detail]
    assert len(uses) == 8 and all(r["state"] == "ok" for r in uses), [(r["state"], r.get("reason")) for r in uses]
    plates = [r for r in rows if r["kind"] == "sweep" and r.get("key") == "plate"]
    assert len(plates) == 8
    assert {(r["profile"]["params"]["width"], r["profile"]["params"]["height"]) for r in plates} == {(150, 360)}
    assert len([r for r in rows if "hole" in r.get("cutter", {})]) == 16
    cuts = [r for r in rows if r.get("key") == "end_cut"]
    assert len(cuts) == 8 and {r["anchor"]["offset"] for r in cuts} == {15}
    # Each plate sits the same way on its own knee: centred on the meet
    # frame, 15 mm thick along its Z, mirrored knees included.
    groups = scene.groups_by_uid()
    for use in uses:
        m = live.junction_frames(scene, use["uid"])["meet"]
        o, z = m[12:15], m[8:11]
        plate = next(c for c in use["contents"] if c["key"] == "plate")
        pts = world_points(scene, groups[plate["placement"]])
        c = [sum(p[i] for p in pts) / len(pts) for i in range(3)]
        d = [c[i] - o[i] for i in range(3)]
        along = sum(d[i] * z[i] for i in range(3))
        assert along == pytest.approx(0.0075, abs=1e-4), use["uid"]
        assert sum(x * x for x in d) - along * along == pytest.approx(0, abs=1e-6), use["uid"]


def test_this_one_override_survives_a_detail_revision_and_all_edits_every_use():
    scene, history, knee, reply = captured_garage()
    detail = reply["detail"]
    matches = [s["roles"] for s in live.junction_suggest(scene, detail)["suggestions"] if s["fit"] == "match"]
    other = apply(history, "junction_apply", detail=detail, bindings=matches[0])["uid"]
    row = rows_by_uid(scene)[other]
    apply(history, "junction_set", row=dict(row, overrides={"plate_t": 20}))
    assert rows_by_uid(scene)[other]["params"]["plate_t"] == 20
    preset = next(p for p, _ in live.preset_catalog(scene) if p["uid"] == detail)
    for p in preset["values"]["params"]:
        if p["key"] == "plate_width":
            p["expr"] = "joining.width + 60"
    apply(history, "preset_update", rows=[preset])
    rows = rows_by_uid(scene)
    assert rows[other]["params"]["plate_t"] == 20 and rows[other]["params"]["plate_width"] == 180
    assert rows[reply["uid"]]["params"]["plate_t"] == 15 and rows[reply["uid"]]["params"]["plate_width"] == 180
    plates = {r["junction"]: r for r in rows.values() if r.get("key") == "plate"}
    assert plates[other]["path"]["points"][1][2] == pytest.approx(0.020)


def test_adopt_a_cut_for_this_use_or_for_the_detail():
    scene, history, knee, reply = captured_garage()
    source, column, rafter = knee[0], knee[1], knee[2]
    flange = facing_face(column, (0, 1, 0))
    extra = "c0a2000000000201"
    apply(history, "live_set", row=dict(kind="cut", uid=extra, owner=rafter.uid, operation="trim", end="end",
                                        cutter={"plane": {}}, anchor={"ref": {"path": [column.uid], "id": flange}, "offset": 400}, on=True, name="Notch"))
    apply(history, "junction_adopt", junction=reply["uid"], rows=[extra])
    rows = rows_by_uid(scene)
    assert rows[extra]["junction"] == reply["uid"] and rows[extra]["anchor"]["ref"]["junction"] == reply["uid"]
    assert extra in rows[reply["uid"]]["local"]["added"]
    apply(history, "undo")
    apply(history, "junction_adopt", junction=reply["uid"], rows=[extra], detail=True)
    preset = next(p for p, _ in live.preset_catalog(scene) if p["uid"] == reply["detail"])
    assert preset["rev"] == 2 and len(preset["values"]["contents"]) == 4
    rows = rows_by_uid(scene)
    assert rows[extra]["junction"] == reply["uid"] and "added" not in rows[reply["uid"]].get("local", {})


def test_release_one_level_then_all(tmp_path):
    scene, history, knee, reply = captured_garage()
    rows = rows_by_uid(scene)
    group_uid = knee[5]
    apply(history, "junction_release", uid=reply["uid"])
    rows = rows_by_uid(scene)
    assert reply["uid"] not in rows and group_uid in rows and "parent" not in rows[group_uid]
    assert "junction" not in rows[knee[4]]
    apply(history, "undo")
    apply(history, "junction_release", uid=reply["uid"], deep=True)
    rows = rows_by_uid(scene)
    assert not any(r["kind"] == "junction" for r in rows.values())
    path = tmp_path / "released.built"
    igz.save_scene(scene, path)


def test_capture_refuses_a_meeting_that_already_has_a_junction():
    scene, history, knee, reply = captured_garage()
    result = commands.run(history, "junction_capture", seed=[knee[1].uid, knee[2].uid])
    assert not result["ok"] and "already" in str(result)


def test_offers_name_the_meeting_once_two_items_follow_it():
    scene, history = garage(purlins=False)
    assert live.junction_capture_offers(scene) == []
    source, column, rafter, pl, cut_uid, _ = handmade_knee(scene, history, bolts=False)
    (offer,) = live.junction_capture_offers(scene)
    assert offer["key"] == source["key"] and sorted(offer["items"]) == sorted([cut_uid, pl.uid])
    apply(history, "junction_capture", seed=cut_uid)
    assert live.junction_capture_offers(scene) == []


def test_alt_click_applies_outside_the_rule_as_custom():
    scene, history = garage(purlins=False, lean_to=True)
    knee = handmade_knee(scene, history)
    reply = apply(history, "junction_capture", seed=knee[4])
    near = next(s for s in live.junction_suggest(scene, reply["detail"])["suggestions"] if s["fit"] == "near")
    plain = apply(history, "junction_apply", detail=reply["detail"], bindings=near["roles"])["uid"]
    assert rows_by_uid(scene)[plain]["state"] == "outside"
    apply(history, "undo")
    forced = apply(history, "junction_apply", detail=reply["detail"], bindings=near["roles"], force=True)["uid"]
    record = live.junction_record(scene, forced)
    assert record["state"] == "ok" and record["forced"]


def test_upgrade_pushes_this_use_values_into_the_detail():
    scene, history, knee, reply = captured_garage()
    detail = reply["detail"]
    matches = [s["roles"] for s in live.junction_suggest(scene, detail)["suggestions"] if s["fit"] == "match"]
    uids = apply(history, "junction_apply", detail=detail, bindings=matches)["uids"]
    row = rows_by_uid(scene)[uids[0]]
    apply(history, "junction_set", row=dict(row, overrides={"plate_t": 20}))
    record = live.junction_record(scene, uids[0])
    assert record["overrides"] == ["plate_t"] and record["uses"] == 8
    width = next(p for p in record["params"] if p["key"] == "plate_width")
    assert width["relation"].startswith("= ") and "width + 30" in width["relation"]
    apply(history, "junction_upgrade", uid=uids[0])
    rows = rows_by_uid(scene)
    assert all(rows[u]["params"]["plate_t"] == 20 for u in uids + [reply["uid"]])
    assert not rows[uids[0]].get("overrides")


def test_along_a_repeat_is_one_row_that_follows_its_copies():
    from tests.test_junction_meetings import cleat_detail
    scene, history = garage()
    cleat = cleat_detail(scene)
    apply(history, "preset_update", rows=[cleat])
    crossing = next(s for s in live.junction_suggest(scene, cleat["uid"])["suggestions"] if s["fit"] == "match")
    revision = history.revision
    uid = apply(history, "junction_apply", detail=cleat["uid"], bindings=crossing["roles"], along={"end": "both"})["uid"]
    assert history.revision == revision + 1
    rows = rows_by_uid(scene)
    j = rows[uid]
    assert j["state"] == "ok" and j["along"]["repeat"]
    children = [r for r in rows.values() if r.get("parent") == uid]
    # This slope's Purlins: 4 purlins in each of 3 bays, both ends.
    assert len(children) == 24 and all(r["state"] == "ok" for r in children)
    assert len([r for r in rows.values() if r.get("key") == "cleat"]) == 24
    # One purlin fewer per bay: its cleats go in the same step.
    outer = rows[j["along"]["repeat"]]
    inner = next(r for r in rows.values() if r["kind"] == "repeat" and r["owner"] == outer["source"])
    apply(history, "live_set", row=dict(inner, rule={"divide": 2}))
    children = [r for r in rows_by_uid(scene).values() if r.get("parent") == uid]
    assert len(children) == 18, len(children)
    apply(history, "undo")
    assert len([r for r in rows_by_uid(scene).values() if r.get("parent") == uid]) == 24

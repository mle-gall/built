# SPDX-License-Identifier: GPL-3.0-or-later
"""Meetings and suggestions (docs/junctions.md §4, lane J3) on lived-in fixtures."""
import os
from collections import Counter
from copy import deepcopy
from math import cos, radians, sin, tan

import pytest

from built_core import Vec3, command_protocol as live
from built_core.group import Group
from built_core.history import History
from built_core.protocol import commands
from built_core.scene import Scene
from formats import igz
from tests.test_live_recipes import apply
from tests.test_sketch_rows import member

SLOPE = radians(15)
BAY, SPAN, EAVE = 4.0, 6.0, 3.0
PERF = bool(os.environ.get("BUILT_PERF"))


def box_group(name, lo, hi, cls):
    g = Group(name=name)
    (x0, y0, z0), (x1, y1, z1) = lo, hi
    v = [Vec3(x, y, z) for z in (z0, z1) for y in (y0, y1) for x in (x0, x1)]
    for a, b, c, d in ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4), (2, 6, 7, 3), (0, 4, 6, 2), (1, 3, 7, 5)):
        g.mesh.add_face([v[a], v[b], v[c], v[d]])
    g.ifc = {"class": cls, "name": name}
    return g


def ride(scene, count=400, cells=12):
    """Unclassified imported geometry: many faces the index must never visit."""
    for i in range(count):
        g = Group(name=f"Ride part {i}")
        ox, oy = 40 + (i % 20) * 3, (i // 20) * 3
        for y in range(cells):
            for x in range(cells):
                g.mesh.add_face([Vec3(ox + x * .2, oy + y * .2, 0), Vec3(ox + x * .2 + .2, oy + y * .2, 0),
                                 Vec3(ox + x * .2 + .2, oy + y * .2 + .2, 0), Vec3(ox + x * .2, oy + y * .2 + .2, 0)])
        scene.groups.append(g)


def slope_point(y0, s, side):
    """A point `s` metres up a rafter from its eave, and the rafter's up."""
    d = (side * cos(SLOPE), sin(SLOPE))
    return (y0 + d[0] * s, EAVE + d[1] * s), (-side * sin(SLOPE), cos(SLOPE))


def garage(frames=4, reversed_rafter=False, purlins=True, lean_to=False):
    """Slab, portal frames (HE 200 A columns, IPE 240 rafters) and purlins
    on the rafters' top flanges by nested Repeats: 3 bays x 4 x 2 slopes."""
    scene = Scene()
    scene.groups.append(box_group("Slab", (-.5, -.5, -.2), (BAY * (frames - 1) + .5, SPAN + .5, 0), "IfcSlab"))
    history = History(scene)
    ridge = EAVE + SPAN / 2 * tan(SLOPE)
    sweeps = []
    for f in range(frames):
        x = f * BAY
        sweeps += [member({"library": "en/he200a"}, [[x, 0, 0], [x, 0, EAVE + .3]]),
                   member({"library": "en/he200a"}, [[x, SPAN, 0], [x, SPAN, EAVE + .3]])]
        left = [[x, 0, EAVE], [x, SPAN / 2, ridge]]
        if reversed_rafter and f == 2:
            left.reverse()
        sweeps += [member({"library": "en/ipe240"}, left),
                   member({"library": "en/ipe240"}, [[x, SPAN, EAVE], [x, SPAN / 2, ridge]])]
    if lean_to:
        x, rise = -3., 2.5 * tan(radians(9))
        sweeps += [member({"library": "en/he200a"}, [[x, 0, 0], [x, 0, EAVE + .3]]),
                   member({"library": "en/ipe240"}, [[x, 0, EAVE], [x, -2.5, EAVE + rise]])]
    apply(history, "create_sweeps", sweeps=sweeps)
    groups = scene.groups[1:]
    names = ["C-1", "C-2", "R-1", "R-2"]
    for i, g in enumerate(groups[:frames * 4]):
        g.name = f"{names[i % 4]} · frame {i // 4 + 1}"
    if purlins:
        for n, side in enumerate((1, -1)):
            y0 = 0 if side == 1 else SPAN
            (y, z), up = slope_point(y0, .8, side)
            y, z = y + up[0] * .16, z + up[1] * .16
            apply(history, "create_sweeps", sweeps=[member({"library": "en/shs80x4"}, [[0, y, z], [BAY, y, z]])])
            source = scene.groups[-1]
            source.name = "Purlin"
            run = [[0, y, z], [0, y + side * cos(SLOPE) * 2.1, z + sin(SLOPE) * 2.1]]
            inner = dict(kind="repeat", uid=f"0000000000001{n}01", source=source.uid, run={"points": run},
                         rule={"divide": 3})
            apply(history, "live_set", row=inner)
            outer = dict(kind="repeat", uid=f"0000000000001{n}02", source=scene.groups[-1].uid,
                         run={"points": [[0, 0, 0], [BAY * (frames - 2), 0, 0]]}, rule={"divide": frames - 2})
            apply(history, "live_set", row=outer)
    return scene, history


def knee_details(scene, hand=1, plate_uid=None):
    """J1's knee: end cut, plate, a 2 x 2 bolt grid and a fillet weld."""
    plate = plate_uid or next(p for p, _ in live.preset_catalog(scene) if p["name"] == "Plate")["uid"]

    def detail(n, name, roles, contents, meeting="parts"):
        return dict(uid=f"c093{n:012x}", name=name, primitive="junction", pack="Project", rev=1, locks={},
                    record={"class": "IfcElementAssembly"},
                    values=dict(meeting=meeting, roles=dict.fromkeys(roles, {}), contents=contents))
    bolt = detail(1, "Bolt M16", ["ply_1", "ply_2"], [
        dict(key="bolt", part={"fastener": {"d": 16, "clearance": 2, "grade": "8.8"},
                               "at": {"frame": "@meet", "offset": [-55, -55, 0]}})])
    weld = detail(2, "Fillet weld", ["a", "b"], [dict(key="seam", part={"weld": {"throat": 4, "type": "fillet"}})])
    knee = detail(3, "Knee end plate", ["main", "joining"], [
        dict(key="end_cut", on="joining", row={"kind": "cut", "end": "start", "operation": "trim",
                                               "cutter": {"plane": {}}, "anchor": {"ref": "@meet", "offset": 12}}),
        dict(key="plate", part={"preset": plate, "fields": {"profile.params.width": "plate_width",
             "profile.params.height": 300, "thickness": 12}, "at": {"frame": "@meet"}}),
        dict(key="bolts", junction={"detail": bolt["uid"], "roles": {"ply_1": "@plate", "ply_2": "@main"},
             "repeat": {"rule": {"count": 2, "step": 110}, "run": "@meet.y", "nest": {"count": 2, "step": 110}}}),
        dict(key="weld", junction={"detail": weld["uid"], "roles": {
            "a": "@joining", "b": {"placement": "@plate", "face": "end"}}})], meeting="end_face")
    knee["values"]["params"] = [dict(key="plate_width", expr="joining.width + 150", unit="mm")]
    knee["values"]["rule"] = {
        "meeting": "end_face", "mirror": "allow", "direction": "ignore", "hand": hand,
        "roles": {"main": {"profile": [{"library": "en/he"}], "size": {"height": [152, 240]},
                           "face": ["flange_outer"]},
                  "joining": {"profile": [{"library": "en/ipe"}], "size": {"height": [200, 300]}, "end": "any"}},
        "angle": {"value": 75, "tol": 2}, "offset": {"tol_mm": 5}}
    return bolt, weld, knee


def cleat_detail(scene):
    plate = next(p for p, _ in live.preset_catalog(scene) if p["name"] == "Plate")["uid"]
    return dict(uid="c09300000000000a", name="Purlin cleat", primitive="junction", pack="Project", rev=1,
                locks={}, record={"class": "IfcElementAssembly"},
                values=dict(meeting="crossing", roles={"main": {}, "joining": {}}, contents=[
                    dict(key="cleat", part={"preset": plate, "fields": {
                        "profile.params.width": 80, "profile.params.height": 120, "thickness": 8},
                        "at": {"frame": "@meet"}})],
                    rule={"meeting": "crossing", "roles": {
                        "main": {"profile": [{"library": "en/ipe"}], "face": ["flange_outer"]},
                        "joining": {"profile": [{"library": "en/shs"}]}},
                        "angle": {"value": 90, "tol": 2}}))


def meetings_by_kind(scene):
    return Counter(m["kind"] for m in live.meetings(scene))


def first_knee(scene):
    return next(m for m in live.meetings(scene, {"kind": "end_face"})
                if scene.groups_by_uid()[m["members"][0]].name == "C-1 · frame 1")


def test_garage_meetings_are_found_with_no_false_positives():
    scene, _ = garage()
    ride(scene, count=60)
    kinds = meetings_by_kind(scene)
    assert kinds == {"end_face": 8, "crossing": 48, "end_end": 4, "base": 8}, kinds
    groups = scene.groups_by_uid()
    knees = [m for m in live.meetings(scene, {"kind": "end_face"}) if groups[m["members"][0]].name.startswith("C-")]
    assert len(knees) == 8 and {m["face"] for m in knees} == {"flange_outer"}
    assert {round(m["angle"]) for m in knees} == {75} and {m["hand"] for m in knees} == {1, -1}
    cleats = live.meetings(scene, {"kind": "crossing"})
    assert all(groups[m["members"][0]].name.startswith("R-") for m in cleats)
    assert len(cleats) == 48 and all(m["face"] == "flange_outer" for m in cleats)
    # Keys are stable and unique: the same model gives the same keys.
    keys = [m["key"] for m in live.meetings(scene)]
    assert len(set(keys)) == len(keys)
    live.meeting_index_reset(scene)
    assert sorted(m["key"] for m in live.meetings(scene)) == sorted(keys)
    groups_ = live.junction_arrangements(scene)
    assert [(g["kind"], g["count"], g["status"]) for g in groups_][:2] == [("crossing", 48, "none"), ("end_face", 8, "none")]
    assert "HE 200 A × IPE 240" in groups_[1]["arrangement"] and "75°" in groups_[1]["arrangement"]


def test_knee_detail_with_parts_is_suggested_mirrored_and_applied_in_one_step(tmp_path):
    scene, history = garage(lean_to=True)
    source = first_knee(scene)
    bolt, weld, knee = knee_details(scene, hand=source["hand"])
    apply(history, "preset_update", rows=[bolt, weld, knee])
    roles = {"main": source["roles"]["main"], "joining": source["roles"]["other"]}
    assert roles["main"]["face"] == "flange_outer"
    apply(history, "junction_apply", detail=knee["uid"], bindings=roles)
    reply = live.junction_suggest(scene, knee["uid"])
    fits = Counter(s["fit"] for s in reply["suggestions"])
    assert fits == {"applied": 1, "match": 7, "near": 1}, fits
    matches = [s for s in reply["suggestions"] if s["fit"] == "match"]
    assert sum(s["mirrored"] for s in matches) == 4 and all("mirrored" in s["why"] for s in matches if s["mirrored"])
    near = next(s for s in reply["suggestions"] if s["fit"] == "near")
    assert near["near"]["criterion"] == "angle" and near["near"]["value"] == 81
    assert near["near"]["widen_to"] == {"value": 75, "tol": 6}
    ghosts = live.junction_ghosts(scene, knee["uid"], [s["key"] for s in matches[:2]])
    assert len(ghosts) == 2 and all(len(g["lines"]) >= 7 for g in ghosts)
    revision = history.revision
    result = apply(history, "junction_apply", detail=knee["uid"], bindings=[s["roles"] for s in matches])
    assert history.revision == revision + 1 and len(result["uids"]) == 7
    rows = live.live_rows(scene)
    knees = [r for r in rows if r["kind"] == "junction" and r.get("detail", {}).get("uid") == knee["uid"]]
    assert len(knees) == 8 and all(r["state"] == "ok" for r in knees), [(r["state"], r.get("reason")) for r in knees]
    plates = [r for r in rows if r.get("key") == "plate"]
    assert len(plates) == 8 and {r["profile"]["params"]["width"] for r in plates} == {270}
    assert len([r for r in rows if r.get("key") == "weld"]) == 8
    assert len([r for r in rows if "hole" in r.get("cutter", {})]) == 16
    after = Counter(s["fit"] for s in live.junction_suggest(scene, knee["uid"])["suggestions"])
    assert after == {"applied": 8, "near": 1}
    apply(history, "undo")
    assert Counter(s["fit"] for s in live.junction_suggest(scene, knee["uid"])["suggestions"]) == fits
    groups = live.junction_arrangements(scene)
    knee_group = next(g for g in groups if g["count"] == 8)
    assert (knee_group["status"], knee_group["detailed"]) == ("some", 1)


def test_reversed_rafter_matches_and_binds_its_other_end():
    scene, history = garage(reversed_rafter=True, purlins=False)
    source = first_knee(scene)
    bolt, weld, knee = knee_details(scene, hand=source["hand"])
    knee["values"]["contents"] = knee["values"]["contents"][:2]
    apply(history, "preset_update", rows=[bolt, weld, knee])
    matches = [s for s in live.junction_suggest(scene, knee["uid"])["suggestions"] if s["fit"] == "match"]
    assert len(matches) == 8
    ends = Counter(s["roles"]["joining"]["end"] for s in matches)
    assert ends == {"start": 7, "end": 1}
    apply(history, "junction_apply", detail=knee["uid"], bindings=[s["roles"] for s in matches])
    junctions = [r for r in live.live_rows(scene) if r["kind"] == "junction"]
    assert len(junctions) == 8 and all(r["state"] == "ok" for r in junctions)


def test_skips_are_remembered_undone_and_survive_save_and_reopen(tmp_path):
    scene, history = garage(purlins=False)
    bolt, weld, knee = knee_details(scene, hand=first_knee(scene)["hand"])
    apply(history, "preset_update", rows=[bolt, weld, knee])
    suggestions = live.junction_suggest(scene, knee["uid"])["suggestions"]
    skipped = suggestions[-1]["key"]
    apply(history, "junction_skip", detail=knee["uid"], meeting=skipped)
    fits = {s["key"]: s["fit"] for s in live.junction_suggest(scene, knee["uid"])["suggestions"]}
    assert fits[skipped] == "skipped" and Counter(fits.values()) == {"match": 7, "skipped": 1}
    assert not commands.run(history, "junction_skip", detail=knee["uid"], meeting="not a key")["ok"]
    apply(history, "junction_follow", detail=knee["uid"])
    counts = live.junction_counts(scene)["details"][knee["uid"]]
    assert counts["follow"] and counts["new"] == 7 and counts["counts"]["skipped"] == 1
    path = tmp_path / "garage.built"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    again = {s["key"]: s["fit"] for s in live.junction_suggest(back, knee["uid"])["suggestions"]}
    assert again == fits
    assert live.junction_counts(back)["details"][knee["uid"]]["follow"]
    apply(history, "undo")
    apply(history, "undo")
    assert all(s["fit"] == "match" for s in live.junction_suggest(scene, knee["uid"])["suggestions"])
    apply(History(back), "junction_skip", detail=knee["uid"], meeting=skipped, skip=False)
    assert all(s["fit"] == "match" for s in live.junction_suggest(back, knee["uid"])["suggestions"])


def test_cleats_across_the_purlin_repeats_and_the_index_follows_edits():
    scene, history = garage()
    cleat = cleat_detail(scene)
    apply(history, "preset_update", rows=[cleat])
    suggestions = live.junction_suggest(scene, cleat["uid"])["suggestions"]
    assert Counter(s["fit"] for s in suggestions) == {"match": 48}
    stations = [s for s in suggestions if "station" in s["roles"]["joining"]]
    assert len(stations) == 2 * 3 * 2
    apply(history, "junction_apply", detail=cleat["uid"], bindings=[s["roles"] for s in suggestions])
    junctions = [r for r in live.live_rows(scene) if r["kind"] == "junction"]
    assert len(junctions) == 48 and all(r["state"] == "ok" for r in junctions), \
        Counter((r["state"], str(r.get("reason"))) for r in junctions)
    # Moving a column off its rafter end ends that knee; undo brings it back.
    knees = meetings_by_kind(scene)
    column = next(g for g in scene.groups if g.name.startswith("C-1 · frame 1"))
    apply(history, "move_placements", placements=[column.uid], delta=[0, -1, 0])
    stats = live.meeting_index_stats(scene)
    assert meetings_by_kind(scene)["end_face"] == knees["end_face"] - 1
    assert stats["updated"] >= 1
    apply(history, "undo")
    assert meetings_by_kind(scene) == knees


def hall(frames=101, bay=6.0, span=20.0, purlins=8):
    """A synthetic steel hall: about 2,000 members and 4,000 meetings."""
    scene = Scene()
    scene.groups.append(box_group("Slab", (-1, -1, -.3), (bay * (frames - 1) + 1, span + 1, 0), "IfcSlab"))
    history = History(scene)
    ridge = EAVE + span / 2 * tan(SLOPE)
    sweeps = []
    for f in range(frames):
        x = f * bay
        sweeps += [member({"library": "en/he200a"}, [[x, 0, 0], [x, 0, EAVE + .3]]),
                   member({"library": "en/he200a"}, [[x, span, 0], [x, span, EAVE + .3]]),
                   member({"library": "en/ipe240"}, [[x, 0, EAVE], [x, span / 2, ridge]]),
                   member({"library": "en/ipe240"}, [[x, span, EAVE], [x, span / 2, ridge]])]
    rafter = span / 2 / cos(SLOPE)
    for f in range(frames - 1):
        for side, y0 in ((1, 0), (-1, span)):
            for k in range(purlins):
                (y, z), up = slope_point(y0, .6 + k * (rafter - 1.) / (purlins - 1), side)
                y, z = y + up[0] * .16, z + up[1] * .16
                sweeps.append(member({"library": "en/shs80x4"}, [[f * bay, y, z], [(f + 1) * bay, y, z]]))
    apply(history, "create_sweeps", sweeps=sweeps)
    return scene, history


@pytest.mark.slow
def test_meeting_budgets_on_the_hall_and_the_garage_beside_the_ride():
    scene, history = garage()
    ride(scene)
    garage_builds = []
    for _ in range(3):
        live.meeting_index_reset(scene)
        garage_builds.append(live.meeting_index_stats(scene)["build_ms"])
    garage_ms = min(garage_builds)
    scene, history = hall()
    builds = []
    for _ in range(3):
        live.meeting_index_reset(scene)
        builds.append(live.meeting_index_stats(scene)["build_ms"])
    stats = live.meeting_index_stats(scene)
    assert stats["members"] >= 2000 and stats["meetings"] >= 3600, stats
    updates = []
    column = scene.groups[1]
    for i in range(5):
        apply(history, "move_placements", placements=[column.uid], delta=[0, .5 if i % 2 == 0 else -.5, 0])
        updates.append(live.meeting_index_stats(scene)["update_ms"])
    bolt, weld, knee = knee_details(scene, hand=1)
    details = [deepcopy(knee) for _ in range(5)]
    for i, d in enumerate(details):
        d["uid"] = f"c0930000000001{i:02x}"
        d["values"]["rule"].pop("hand")
    apply(history, "preset_update", rows=[bolt, weld, *details])
    one = min(live.junction_suggest(scene, details[0]["uid"])["ms"] for _ in range(3))
    every = min(live.junction_counts(scene)["ms"] for _ in range(3))
    result = dict(garage_and_ride_build_ms=garage_ms, hall_build_ms=min(builds), update_ms=max(updates[1:]),
                  bytes=stats["bytes"], search_one_ms=one, search_all_ms=every,
                  members=stats["members"], meetings=stats["meetings"])
    print("Meeting index budgets:", result)
    scale = 1 if PERF else 4
    assert garage_ms < 5 * scale and min(builds) < 20 * scale
    assert max(updates[1:]) < .5 * scale and stats["bytes"] < 2_000_000
    assert one < 10 * scale and every < 50 * scale


WALL_CORNER, WALL_MITRE, WALL_TEE, WALL_ON_SLAB, DOOR_OPENING = (
    "f4f093aed0396c6f", "4036051395af3d4c", "a2ff384d2a3b97b3", "9266cd42050e4e90", "30e27bad196ea165")


def walls():
    """Existing walls (no Wall tool corners): an L, a tee, a door in a wall."""
    scene = Scene()
    scene.groups.append(box_group("Floor", (-1, -1, -.2), (7, 6, 0), "IfcSlab"))
    door = box_group("Door", (1, -.15, 0), (1.9, .15, 2.1), "IfcDoor")
    scene.groups.append(door)
    history = History(scene)
    wall = lambda a, b: dict(profile=dict(library="basic/wall"), attachment="bottom", path=dict(points=[a, b]))
    apply(history, "create_sweeps", sweeps=[wall([0, 0, 0], [6, 0, 0]), wall([6, 0, 0], [6, 5, 0]),
                                            wall([3, 0, 0], [3, 4, 0])])
    for g, name in zip(scene.groups[2:], ("W-1", "W-2", "W-3")):
        g.name = name
    return scene, history, door


def test_existing_walls_get_corners_tees_and_slab_details_through_the_review():
    scene, history, door = walls()
    kinds = meetings_by_kind(scene)
    assert kinds == {"corner": 2, "base": 3, "hosted": 1}, kinds
    fits = lambda uid: Counter(s["fit"] for s in live.junction_suggest(scene, uid)["suggestions"])
    assert fits(WALL_CORNER) == fits(WALL_MITRE) == {"match": 1}
    assert fits(WALL_TEE) == {"match": 1} and fits(WALL_ON_SLAB) == {"match": 3}
    door_fit = live.junction_suggest(scene, DOOR_OPENING)["suggestions"]
    assert [(s["fit"], sorted(s["roles"])) for s in door_fit] == [("match", ["host", "hosted"])]
    for uid in (WALL_CORNER, WALL_TEE):
        (s,) = live.junction_suggest(scene, uid)["suggestions"]
        apply(history, "junction_apply", detail=uid, bindings=[s["roles"]])
    junctions = [r for r in live.live_rows(scene) if r["kind"] == "junction"]
    assert len(junctions) == 2 and all(r["state"] == "ok" for r in junctions), \
        [(r["detail"]["name"], r["state"], r.get("reason")) for r in junctions]
    corner = next(r for r in junctions if r["detail"]["uid"] == WALL_CORNER)
    assert len(corner["contents"]) == 2
    assert fits(WALL_CORNER) == {"applied": 1} and fits(WALL_MITRE) == {"detailed": 1}
    assert fits(WALL_TEE) == {"applied": 1}
    groups = {g["arrangement"]: g for g in live.junction_arrangements(scene)}
    assert sum(g["detailed"] for g in groups.values()) == 2
    # Undo takes both back in one step each.
    apply(history, "undo")
    apply(history, "undo")
    assert fits(WALL_CORNER) == {"match": 1}

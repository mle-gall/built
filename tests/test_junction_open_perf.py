# SPDX-License-Identifier: GPL-3.0-or-later
"""Opening a model with junctions (docs/junctions.md §3.4: stamps checked,
nothing evaluated, < 30 ms over the same model without them). The model is
a synthetic portal hall whose knees carry the J1 knee detail (end cut,
plate, 2 x 2 bolts, weld); the reference is the same file with only its
junction rows removed, so groups, geometry, cuts and parts are the same."""
import os
import time

from built_core import command_protocol as live
from built_core.history import History
from built_core.scene import Scene
from formats import igz
from tests.test_junction_meetings import first_knee, garage, knee_details
from tests.test_live_recipes import apply

PERF = bool(os.environ.get("BUILT_PERF"))


def hall_with_knees(frames):
    scene, history = garage(frames=frames, purlins=False)
    source = first_knee(scene)
    bolt, weld, knee = knee_details(scene, hand=source["hand"])
    apply(history, "preset_update", rows=[bolt, weld, knee])
    apply(history, "junction_apply", detail=knee["uid"],
          bindings={"main": source["roles"]["main"], "joining": source["roles"]["other"]})
    matches = [s["roles"] for s in live.junction_suggest(scene, knee["uid"])["suggestions"] if s["fit"] == "match"]
    apply(history, "junction_apply", detail=knee["uid"], bindings=matches)
    return scene, history


def junction_rows(scene):
    return [r for r in live.live_rows(scene) if r["kind"] == "junction"]


def open_ms(path, n=5):
    best = None
    for _ in range(n):
        scene = Scene()
        t = time.perf_counter()
        igz.load_into(scene, path)
        ms = (time.perf_counter() - t) * 1000
        best = ms if best is None else min(best, ms)
    return best, scene


def without_junction_rows(src, dst):
    """The same file with only its junction rows (and owned rows' links to
    them) removed: the same groups, geometry and other recipes."""
    import json
    import zipfile

    def find(o):
        if isinstance(o, dict):
            if isinstance(o.get("built.live"), dict) and "rows" in o["built.live"]:
                return o["built.live"]
            return next((r for r in map(find, o.values()) if r is not None), None)
        if isinstance(o, list):
            return next((r for r in map(find, o) if r is not None), None)
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as zout:
        doc = json.loads(zin.read("document.json"))
        live_ext = find(doc)
        live_ext["rows"] = [r for r in live_ext["rows"] if r.get("kind") != "junction"]
        for r in live_ext["rows"]:
            r.pop("junction", None)
            r.pop("key", None)
        for info in zin.infolist():
            zout.writestr(info, json.dumps(doc).encode() if info.filename == "document.json" else zin.read(info.filename))


def test_opening_junctions_checks_stamps_and_evaluates_nothing(tmp_path):
    frames = 150 if PERF else 6
    scene, history = hall_with_knees(frames)
    rows = junction_rows(scene)
    count = len(rows)
    assert len([r for r in rows if not r.get("parent")]) == 2 * frames
    with_junctions = tmp_path / "with.built"
    igz.save_scene(scene, with_junctions)
    without = tmp_path / "without.built"
    without_junction_rows(with_junctions, without)
    n = 12 if PERF else 3
    base_ms, plain = open_ms(without, n)
    ms, opened = open_ms(with_junctions, n)
    assert not junction_rows(plain)
    reopened = junction_rows(opened)
    assert len(reopened) == count and all(r["state"] == "ok" for r in reopened), \
        sorted({r["state"] for r in reopened})
    print(f"open: {count} junction rows {ms:.1f} ms, without {base_ms:.1f} ms, over {ms - base_ms:.1f} ms")
    if PERF:
        assert ms - base_ms < 30


def test_junction_stamps_are_checked_after_the_open_and_before_anyone_reads_them(tmp_path):
    from built_core import Vec3
    scene, history = hall_with_knees(2)
    knee = next(r for r in junction_rows(scene) if not r.get("parent"))
    assembly = scene.groups_by_uid()[knee["owner"]]
    # An outside reader edits geometry inside the junction's assembly and
    # keeps its metadata: the saved stamp no longer matches.
    assembly.mesh.add_edge(Vec3(0, 0, 9), Vec3(0, 1, 9))
    path = tmp_path / "outside.built"
    igz.save_scene(scene, path)
    opened = Scene()
    igz.load_into(opened, path)
    left, stale = live.live_verify(opened, 0.0)
    assemblies = [r for r in live.live_rows(scene) if r["kind"] in ("junction", "repeat")]
    assert left == len(assemblies) and not stale  # nothing checked during the open
    states = {r["uid"]: r["state"] for r in junction_rows(opened)}  # a reader checks them all first
    assert states[knee["uid"]] == "stale"
    assert all(s == "ok" for u, s in states.items() if u != knee["uid"])
    assert live.live_verify(opened, 50.0) == (0, False)
    # Saved again, the row stays stale; re-evaluated, it is fresh again.
    again = tmp_path / "again.built"
    igz.save_scene(opened, again)
    back = Scene()
    igz.load_into(back, again)
    assert {r["uid"]: r["state"] for r in junction_rows(back)}[knee["uid"]] == "stale"
    h = History(back)
    apply(h, "live_reevaluate", uid=knee["uid"])
    assert {r["uid"]: r["state"] for r in junction_rows(back)}[knee["uid"]] == "ok"


def test_idle_checks_finish_in_slices(tmp_path):
    scene, _ = hall_with_knees(3)
    path = tmp_path / "hall.built"
    igz.save_scene(scene, path)
    opened = Scene()
    igz.load_into(opened, path)
    slices = 0
    while True:
        left, stale = live.live_verify(opened, 0.0001)
        slices += 1
        assert not stale
        if not left:
            break
    assert slices > 1

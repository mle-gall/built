# SPDX-License-Identifier: GPL-3.0-or-later
"""Opening a model with junctions (docs/junctions.md §3.4: stamps checked,
nothing evaluated, < 30 ms over the same model without them). The model is
a synthetic portal hall whose knees carry the J1 knee detail (end cut,
plate, 2 x 2 bolts, weld); the reference is the same file with its junction
rows released one level, so cuts and parts stay live and only the junction
rows differ."""
import os
import time

from built_core import command_protocol as live
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


def test_opening_junctions_checks_stamps_and_evaluates_nothing(tmp_path):
    frames = 50 if PERF else 6
    scene, history = hall_with_knees(frames)
    rows = junction_rows(scene)
    count = len(rows)
    assert len([r for r in rows if not r.get("parent")]) == 2 * frames
    with_junctions = tmp_path / "with.built"
    igz.save_scene(scene, with_junctions)
    while rows := junction_rows(scene):
        apply(history, "junction_release", uid=rows[0]["uid"])
    without = tmp_path / "without.built"
    igz.save_scene(scene, without)
    base_ms, plain = open_ms(without)
    ms, opened = open_ms(with_junctions)
    assert not junction_rows(plain)
    reopened = junction_rows(opened)
    assert len(reopened) == count and all(r["state"] == "ok" for r in reopened), \
        sorted({r["state"] for r in reopened})
    print(f"open: {len(reopened)} junction rows {ms:.1f} ms, without {base_ms:.1f} ms, over {ms - base_ms:.1f} ms")
    if PERF:
        assert ms - base_ms < 30

# SPDX-License-Identifier: GPL-3.0-or-later
"""The junction review in a native window, in lived-in state (lane J3)."""
import inspect
import json
import os
from pathlib import Path

import pytest
from formats import igz
from tests.test_live_recipes import apply


def review_plugin(app):
    import json
    from math import radians
    from pathlib import Path
    import built_render
    from built_core import Vec3, command_protocol as live
    from built_core.protocol import commands
    from built_core.settings import settings

    folder = Path(__file__).resolve().parents[4]
    probe = Path(__file__).with_suffix(".json")
    settings().setValue("home/recent_files", json.dumps([
        {"path": str(path), "pinned": False} for path in sorted(folder.glob("*.built"))]))

    def detail():
        return next(p for p, _ in live.preset_catalog(app.scene) if p["name"] == "Knee end plate")

    def frame_garage():
        camera = app.viewport.camera
        camera.set_view("iso")
        camera.yaw = radians(-38)
        camera.pitch = radians(24)
        camera.fit_to(Vec3(-1, -1, 0), Vec3(13, 7, 4.5))

    def save_detail():
        # Saving the detail again (a revision) offers the review in a toast.
        row = detail()
        commands.run(app.viewport.history, "preset_update", rows=[row])

    def review():
        built_render.junction_review(app.window, detail()["uid"])

    def untick_last():
        state = built_render.junction_review_state(app.scene)
        last = max(i for i, item in enumerate(state["items"]) if item["fit"] == "match")
        built_render.junction_review_action(app.window, f"tick:{last}")

    def record():
        rows = [r for r in live.live_rows(app.scene) if r["kind"] == "junction" and not r.get("parent")]
        log = json.loads(probe.read_text()) if probe.exists() else []
        log.append({"review": built_render.junction_review_state(app.scene),
                    "junctions": sorted(r["state"] for r in rows)})
        probe.write_text(json.dumps(log))

    for name, action in [("Frame garage", frame_garage), ("Save detail", save_detail), ("Review", review),
                         ("Untick last", untick_last), ("Record", record)]:
        app.add_menu_action(name, action)


@pytest.mark.window
@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1", reason="requires a native window")
def test_review_badges_list_enter_and_escape_in_a_native_window(tmp_path):
    from built_core import Vec3, links
    from built_core.scene import Scene
    from math import cos, sin, tau
    from scripts.isolated_launch import run
    from tests.test_junction_meetings import garage, first_knee, knee_details

    scene, history = garage(purlins=False, lean_to=True)
    source = first_knee(scene)
    bolt, weld, knee = knee_details(scene, hand=source["hand"])
    apply(history, "preset_update", rows=[bolt, weld, knee])
    apply(history, "junction_apply", detail=knee["uid"],
          bindings={"main": source["roles"]["main"], "joining": source["roles"]["other"]})
    model = tmp_path / "Garage.built"
    # Lived in: a linked file that has gone missing, recents, the ride beside.
    reference = tmp_path / "Survey.built"
    igz.save_scene(Scene(), reference)
    scene.groups.append(links.make_link(scene, reference, host_path=model))
    igz.save_scene(scene, model)
    reference.unlink()
    ride = Scene()
    rim = [Vec3(cos(tau * i / 24), 0, 1 + sin(tau * i / 24)) for i in range(24)]
    for i, p in enumerate(rim):
        ride.mesh.add_edge(p, rim[(i + 1) % len(rim)])
        ride.mesh.add_edge(p, Vec3(0, 0, 1))
    ride_path = tmp_path / "Observation wheel.built"
    igz.save_scene(ride, ride_path)
    for i in range(10):
        igz.save_scene(Scene(), tmp_path / f"Study {i + 1}.built")
    root = Path(__file__).resolve().parents[1]
    binary = os.environ.get("BUILT_BINARY", root / "rust/built_app/target/release/built")
    profile = tmp_path / "profile"
    plugin = profile / "data/built/plugins/review_probe.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text(inspect.getsource(review_plugin).replace("def review_plugin(", "def setup("))
    menu = lambda i: f"menu extension:review_probe:{i}"
    steps = [f"open {model}", "wait 800", menu(0), "wait 600",
             menu(1), "wait 1200", "capture saved-toast",
             menu(2), "wait 900", menu(3), "wait 300", menu(4), "capture review",
             "key escape", "wait 400", menu(4), "capture left",
             menu(2), "wait 900", menu(3), "wait 200", "key enter", "wait 1500", menu(4), "capture applied",
             "quit", ""]
    script = tmp_path / "review.txt"
    script.write_text("\n".join(steps))
    result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv",
                  "--size", "1280x800", "--script", script, "--shots", tmp_path, ride_path],
                 settings=profile, capture_output=True, text=True, timeout=120)
    assert result.returncode == 0, result.stderr
    log = json.loads(plugin.with_suffix(".json").read_text())
    opened, left, applied = log
    fits = [item["fit"] for item in opened["review"]["items"]]
    assert fits.count("match") == 7 and fits.count("near") == 1, fits
    assert sum(item["ticked"] for item in opened["review"]["items"]) == 6
    assert all(item["ghost"] for item in opened["review"]["items"] if item["fit"] == "match")
    assert left["review"] is None and left["junctions"] == ["ok"]
    assert applied["review"] is None and applied["junctions"] == ["ok"] * 7
    for frame in ("saved-toast", "review", "left", "applied"):
        assert (tmp_path / f"{frame}.png").stat().st_size > 1000

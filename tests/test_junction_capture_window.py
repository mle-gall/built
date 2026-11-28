# SPDX-License-Identifier: GPL-3.0-or-later
"""Capture in a native window, lived-in (lane J2): the chip appears after the
second item at a knee, its popover saves the detail, the review applies it
to the similar knees with Enter, and the record edits one use and all."""
import inspect
import json
import os
from pathlib import Path

import pytest
from formats import igz


def capture_plugin(app):
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

    def frame():
        camera = app.viewport.camera
        camera.set_view("iso")
        camera.yaw = radians(52)
        camera.pitch = radians(14)
        camera.fit_to(Vec3(-0.6, -0.4, 2.4), Vec3(0.6, 0.9, 3.5))

    def plate():
        # What the Sweep tool commits for a Plate started on the flange.
        spec = json.loads((folder / "plate.json").read_text())
        commands.run(app.viewport.history, "create_sweeps", sweeps=[spec])

    def chip():
        built_render.junction_capture_action(app.window, "open")

    def save():
        built_render.junction_capture_action(app.window, "save")

    def review():
        detail = next(p for p, _ in live.preset_catalog(app.scene) if p["primitive"] == "junction" and p["pack"] == "Project")
        built_render.junction_review(app.window, detail["uid"])

    def record():
        rows = [r for r in live.live_rows(app.scene) if r["kind"] == "junction" and not r.get("parent")]
        log = json.loads(probe.read_text()) if probe.exists() else []
        log.append({"capture": built_render.junction_capture_state(app.scene),
                    "junctions": sorted(r["state"] for r in rows)})
        probe.write_text(json.dumps(log))

    for name, action in [("Frame", frame), ("Plate", plate), ("Chip", chip), ("Save", save),
                         ("Review", review), ("Record", record)]:
        app.add_menu_action(name, action)


@pytest.mark.window
@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1", reason="requires a native window")
def test_chip_popover_save_review_and_apply_in_a_native_window(tmp_path):
    from built_core import command_protocol as live, links
    from built_core.scene import Scene
    from scripts.isolated_launch import run
    from tests.test_junction_capture import facing_face, handmade_knee
    from tests.test_junction_meetings import first_knee, garage

    scene, history = garage(purlins=False)
    source, column, rafter, pl, cut_uid, _ = handmade_knee(scene, history, bolts=False)
    spec = live.live_sweep(scene, pl.uid)
    history.undo()  # the plate is added again in the window, as the tool does
    plate = {k: spec[k] for k in ("preset_uid", "profile", "attachment", "path") if k in spec}
    (tmp_path / "plate.json").write_text(json.dumps(plate))
    model = tmp_path / "Garage.built"
    reference = tmp_path / "Survey.built"
    igz.save_scene(Scene(), reference)
    scene.groups.append(links.make_link(scene, reference, host_path=model))
    igz.save_scene(scene, model)
    reference.unlink()
    for i in range(12):
        igz.save_scene(Scene(), tmp_path / f"Study {i + 1}.built")
    root = Path(__file__).resolve().parents[1]
    binary = os.environ.get("BUILT_BINARY", root / "rust/built_app/target/release/built")
    profile = tmp_path / "profile"
    plugin = profile / "data/built/plugins/capture_probe.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text(inspect.getsource(capture_plugin).replace("def capture_plugin(", "def setup("))
    menu = lambda i: f"menu extension:capture_probe:{i}"
    steps = ["wait 800", menu(0), "wait 400", menu(1), "wait 900", menu(5), "capture chip",
             menu(2), "wait 500", menu(5), "capture popover",
             menu(3), "wait 1200", menu(5), "capture saved",
             menu(4), "wait 900", "capture review", "key enter", "wait 1500", menu(5), "capture applied",
             "quit", ""]
    script = tmp_path / "capture.txt"
    script.write_text("\n".join(steps))
    result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv",
                  "--size", "1280x800", "--script", script, "--shots", tmp_path, model],
                 settings=profile, capture_output=True, text=True, timeout=180)
    assert result.returncode == 0, result.stderr
    assert "error in frame" not in result.stdout + result.stderr, result.stdout + result.stderr
    chip, popover, saved, applied = json.loads(plugin.with_suffix(".json").read_text())
    assert chip["capture"]["chip"] and chip["capture"]["chip"]["similar"] == 7, chip
    assert popover["capture"]["open"] and len(popover["capture"]["draft"]["items"]) == 2
    assert saved["junctions"] == ["ok"] and not saved["capture"]["open"]
    assert applied["junctions"] == ["ok"] * 8, applied
    for frame in ("chip", "popover", "saved", "review", "applied"):
        assert (tmp_path / f"{frame}.png").stat().st_size > 1000

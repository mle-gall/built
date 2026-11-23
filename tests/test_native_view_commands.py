"""The native frame at 1600 × 1000 logical points, using winit input."""
import json
import os
from pathlib import Path
import subprocess

import pytest


def test_navigation_cluster(tmp_path):
    root = Path(__file__).resolve().parents[1]
    binary = Path(os.environ.get("BUILT_BINARY", root / "rust/built_app/target/debug/built"))
    if not binary.is_file():
        pytest.skip("build the native built_app binary for the windowed smoke test")
    model = root / "tests/fixtures/shell/slab-and-cube.igz"
    steps = tmp_path / "steps"
    steps.write_text("""wait 600
tool line
state
click-command orbit
wait 100
drag 600 400 730 465
state
key escape
state
click-command look_around
wait 100
state
drag 600 400 730 465
state
click-command walk
wait 100
state
move 600 400
press 600 400
move 650 330
wait 300
release 650 330
state
key escape
state
quit
""")
    home, settings = tmp_path / "home", tmp_path / "settings"
    home.mkdir()
    settings.mkdir()
    result = subprocess.run([binary, "--root", root, "--venv", root / ".venv", "--new-instance",
                             "--script", steps, "--shots", tmp_path, "--size", "1600x1000", model],
                            env=dict(os.environ, HOME=str(home), BUILT_SETTINGS_DIR=str(settings)),
                            capture_output=True, text=True, timeout=90, check=True)
    states = [json.loads(line.removeprefix("[built] state ")) for line in result.stdout.splitlines()
              if line.startswith("[built] state ")]
    assert all(s["errors"] == 0 for s in states), result.stderr
    start, orbit, resumed, looking, turned, walking, walked, done = states[:8]
    assert start["documents"] == [model.name] and start["active"] == 0
    assert len(start["tab_rects"]) == 1
    assert set(start["cluster"]) == {
        "orbit", "pan", "zoom", "zoom_window", "zoom_extents", "zoom_selection",
        "look_around", "walk", "position_camera", "xray", "back_edges",
        "hidden_geometry", "shadows", "section_cuts",
    }
    assert orbit["nav_mode"] == "orbit" and orbit["camera_eye"] != start["camera_eye"]
    assert resumed["nav_mode"] is None and resumed["tool"] == "LineTool"
    assert looking["tool"] == "LookAroundTool" and looking["camera_forward"] != turned["camera_forward"]
    assert walking["tool"] == "WalkTool" and walked["camera_eye"] != pytest.approx(walking["camera_eye"])
    assert done["tool"] == "LineTool"

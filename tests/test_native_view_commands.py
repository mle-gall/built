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
click 1566 20
wait 200
click 1364 178
wait 100
state
drag 600 400 730 465
state
click 1566 20
wait 200
click 1396 178
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
    assert start["cluster"] == [
        "orbit", "pan", "zoom_extents", "xray", "hidden_geometry", "section_cuts",
    ]
    assert orbit["nav_mode"] == "orbit" and orbit["camera_eye"] != start["camera_eye"]
    assert resumed["nav_mode"] is None and resumed["tool"] == "LineTool"
    assert looking["tool"] == "LookAroundTool" and looking["camera_forward"] != turned["camera_forward"]
    assert walking["tool"] == "WalkTool" and walked["camera_eye"] != pytest.approx(walking["camera_eye"])
    assert done["tool"] == "LineTool"


def test_review_exit_and_modelling_keys(tmp_path):
    """The top-bar exit restores the workspace while tool and measurement keys stay live."""
    import sys
    from scripts.isolated_launch import run
    root = Path(__file__).resolve().parents[1]
    binary = Path(os.environ.get("BUILT_BINARY", root / "rust/built_app/target/debug/built"))
    if not binary.is_file():
        pytest.skip("build the native built_app binary for the windowed smoke test")
    profile = tmp_path / "settings"
    plugin = profile / "data/ingetrazo/plugins/review_probe.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text("""def setup(app):
    def measure():
        from app_shell.modal import _HOST
        from pathlib import Path
        Path(__file__).with_suffix(".txt").write_text(_HOST.view.tools.value_buffer)
    app.add_menu_action("Read measurements", measure)
""")
    modifier = "cmd" if sys.platform == "darwin" else "ctrl"
    steps = tmp_path / "steps"
    steps.write_text(f"""wait 500
key 0 {modifier}
wait 200
key r
type 3;4
menu extension:review_probe:0
state
click-command exit_review
wait 200
key l
state
quit
""")
    result = run([binary, "--root", root, "--venv", root / ".venv", "--new-instance",
                  "--script", steps, root / "tests/fixtures/shell/slab-and-cube.igz"],
                 settings=profile, capture_output=True, text=True, timeout=60, check=True)
    before, after, *_ = [json.loads(line.removeprefix("[built] state ")) for line in result.stdout.splitlines()
                     if line.startswith("[built] state ")]
    assert before["input"]["review"] and "exit_review" in before["cluster"]
    assert before["tool"] == "RectangleTool" and plugin.with_suffix(".txt").read_text() == "3;4"
    assert not after["input"]["review"] and "exit_review" not in after["cluster"]
    assert after["tool"] == "LineTool" and before["errors"] == after["errors"] == 0

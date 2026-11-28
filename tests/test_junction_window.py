# SPDX-License-Identifier: GPL-3.0-or-later
import inspect
import json
import os
from pathlib import Path

import pytest
from formats import igz
from tests.test_live_junction import setup as corner_model
from tests.test_live_recipes import apply


def window_plugin(app):
    from pathlib import Path
    import json
    from built_core import Vec3, command_protocol as live

    def inspect_corner():
        member = app.scene.groups[0]
        app.scene.select([member])
        app.viewport.camera.set_view("iso")
        app.viewport.camera.fit_to(Vec3(0, 0, -.2), Vec3(2, 3, .2))
        markers = live.junction_markers(app.scene)
        Path(__file__).with_suffix(".json").write_text(json.dumps(markers))
    app.add_menu_action("Inspect corner", inspect_corner)


@pytest.mark.window
@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1", reason="requires a native window")
def test_corner_marker_in_native_window(tmp_path):
    from scripts.isolated_launch import run

    scene, history, _, _, _ = corner_model()
    model = tmp_path / "corner.built"
    igz.save_scene(scene, model)
    apply(history, "undo")
    before = tmp_path / "members.built"
    igz.save_scene(scene, before)
    root = Path(__file__).resolve().parents[1]
    binary = os.environ.get("BUILT_BINARY", root / "rust/built_app/target/release/built")
    profile = tmp_path / "profile"
    plugin = profile / "data/built/plugins/corner_probe.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text(inspect.getsource(window_plugin).replace("def window_plugin(", "def setup("))
    script = tmp_path / "corner.txt"
    script.write_text(f"wait 500\nmenu extension:corner_probe:0\nwait 200\ncapture members\n"
                      f"open {model}\nwait 600\nmenu extension:corner_probe:0\nwait 300\n"
                      "capture mitred-corner\nclick 1304 214\nwait 200\nclick 1318 263\n"
                      "wait 300\ncapture member-tree\nquit\n")
    result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv",
                  "--size", "1600x1000", "--script", script, "--shots", tmp_path, before],
                 settings=profile, capture_output=True, text=True, timeout=90)
    assert result.returncode == 0, result.stderr
    assert json.loads(plugin.with_suffix(".json").read_text()) == [[[2., 0., 0.], True]]
    for frame in ("members.png", "mitred-corner.png", "member-tree.png"):
        assert (tmp_path / frame).stat().st_size > 1000


def knee_window_plugin(app):
    import json
    from math import radians
    from pathlib import Path
    from built_core import Vec3
    from built_core.settings import settings

    folder = Path(__file__).resolve().parents[4]
    settings().setValue("home/recent_files", json.dumps([
        {"path": str(path), "pinned": False} for path in sorted(folder.glob("*.built"))]))

    def frame_joint():
        camera = app.viewport.camera
        camera.set_view("iso")
        camera.yaw = radians(32)
        camera.pitch = radians(18)
        camera.fit_to(Vec3(-.16, -.05, .62), Vec3(.16, .28, .98))
    app.add_menu_action("Frame knee", frame_joint)


@pytest.mark.window
@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1", reason="requires a native window")
def test_knee_storyboard_in_native_window(tmp_path):
    from scripts.isolated_launch import run
    from tests.test_live_junction import knee_fixture

    root = Path(__file__).resolve().parents[1]
    binary = os.environ.get("BUILT_BINARY", root / "rust/built_app/target/release/built")
    script = tmp_path / "knee.txt"
    from built_core import Vec3, links
    from built_core.scene import Scene
    from math import cos, sin, tau

    profile = tmp_path / "profile"
    plugin = profile / "data/built/plugins/knee_probe.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text(inspect.getsource(knee_window_plugin).replace("def knee_window_plugin(", "def setup("))
    ride = Scene()
    rim = [Vec3(cos(tau * i / 16), 0, 1 + sin(tau * i / 16)) for i in range(16)]
    for i, p in enumerate(rim):
        ride.mesh.add_edge(p, rim[(i + 1) % len(rim)])
        ride.mesh.add_edge(p, Vec3(0, 0, 1))
    ride_path = tmp_path / "Observation wheel.built"
    igz.save_scene(ride, ride_path)
    reference = tmp_path / "Survey.built"
    igz.save_scene(Scene(), reference)
    for i in range(10):
        igz.save_scene(Scene(), tmp_path / f"Study {i + 1}.built")
    steps = []
    for stage, name in enumerate(("end-cut", "plate", "bolts-and-weld"), 1):
        scene, _, _ = knee_fixture(stage, portal=True)
        model = tmp_path / (name + ".built")
        scene.groups.append(links.make_link(scene, reference, host_path=model))
        igz.save_scene(scene, model)
        steps.extend([f"open {model}", "wait 500", "menu extension:knee_probe:0",
                      "wait 3500", f"capture {name}"])
    reference.unlink()
    script.write_text("\n".join([*steps, "quit", ""]))
    result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv",
                  "--size", "1280x800", "--script", script, "--shots", tmp_path, ride_path],
                 settings=tmp_path / "profile", capture_output=True, text=True, timeout=90)
    assert result.returncode == 0, result.stderr
    for name in ("end-cut", "plate", "bolts-and-weld"):
        assert (tmp_path / (name + ".png")).stat().st_size > 1000


def schedule_window_plugin(app):
    from pathlib import Path
    import json
    import built_render
    from built_core import Vec3
    from built_core.settings import settings

    folder = Path(__file__).resolve().parents[4]
    settings().setValue("home/recent_files", json.dumps([
        {"path": str(p), "pinned": False} for p in sorted(folder.glob("*.built"))]))

    def frame():
        camera = app.viewport.camera
        camera.set_view("iso")
        camera.fit_to(Vec3(.63, -.20, -.18), Vec3(1.37, .20, .55))

    def show():
        frame()
        built_render.delivery_action(app.window, app.window._host.view, "schedules")

    def place():
        built_render.delivery_action(app.window, app.window._host.view, "place_schedule:fasteners")
        sheet = app.scene.compositions[-1]
        Path(__file__).with_suffix(".json").write_text(json.dumps([t.text for t in sheet.texts]))

    app.add_menu_action("Show schedules", show)
    app.add_menu_action("Place bolt schedule", place)
    app.add_menu_action("Frame connection", frame)


@pytest.mark.window
@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1", reason="requires a native window")
def test_place_junction_schedule_window(tmp_path):
    from scripts.isolated_launch import run
    from tests.test_ifc_records_export import garage_junctions
    from tests.test_live_junction import Scene

    root = Path(__file__).resolve().parents[1]
    model = tmp_path / "Garage.built"
    from built_core import links
    reference = tmp_path / "Survey.built"
    igz.save_scene(Scene(), reference)
    scene = garage_junctions()
    scene.groups.append(links.make_link(scene, reference, host_path=model))
    igz.save_scene(scene, model)
    reference.unlink()
    for i in range(10):
        igz.save_scene(Scene(), tmp_path / f"Study {i+1}.built")
    ride = tmp_path / "Ride.built"
    igz.save_scene(Scene(), ride)
    profile = tmp_path / "profile"
    plugin = profile / "data/built/plugins/schedule_probe.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text(inspect.getsource(schedule_window_plugin).replace("def schedule_window_plugin(", "def setup("))
    script = tmp_path / "schedules.txt"
    script.write_text(f"open {model}\nwait 800\nmenu extension:schedule_probe:2\nwait 3400\ncapture before\nmenu extension:schedule_probe:0\nwait 600\n"
                      "capture junction-schedules\nmenu extension:schedule_probe:1\nwait 400\nquit\n")
    binary = os.getenv("BUILT_BINARY", root / "rust/built_app/target/release/built")
    result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv", "--size", "1280x800",
                  "--script", script, "--shots", tmp_path, ride], settings=profile,
                 capture_output=True, text=True, timeout=90)
    assert result.returncode == 0, result.stderr
    cells = json.loads(plugin.with_suffix(".json").read_text())
    assert "32" in cells and "96" in cells and "Purlin cleat" in cells

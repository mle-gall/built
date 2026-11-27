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

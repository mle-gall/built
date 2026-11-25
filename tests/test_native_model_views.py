"""Exercise shared-scene GPU panes and their input through the winit window."""
import json
import os
from pathlib import Path

import pytest

from scripts.isolated_launch import run
from tests.shell_isolation import restore_shell_patches  # noqa: F401 (autouse)

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "rust/built_app/target/release/built"


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BINARY.is_file(),
                    reason="requires the native window")
def test_split_edit_restore_and_release(tmp_path):
    model = ROOT / "tests/fixtures/shell/slab-and-cube.igz"
    script = tmp_path / "views.txt"
    script.write_text("""wait 400
menu layout:two
wait 250
capture split
state
click 230 340
state
tool line
click 250 300
click 310 325
key escape
tool select
state
click 740 340
state
menu undo
state
menu maximize_model_view
wait 200
state
menu maximize_model_view
menu layout:four
wait 200
state
menu new
wait 200
state
menu prev_doc
wait 200
state
menu layout:single
wait 200
key escape
state
quit
""")
    result = run([BINARY, "--new-instance", "--root", ROOT, "--venv", ROOT / ".venv",
                  "--size", "1280x800", "--script", script, "--shots", tmp_path, model],
                 capture_output=True, text=True, timeout=100)
    log = result.stdout + result.stderr
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and len(states) >= 10, log
    assert all(state["errors"] == 0 for state in states), result.stderr
    split, top, edit, front, undo, maximized, four, other, restored, single = states[:10]
    assert (split["model_layout"], split["view_targets"]) == ("two", 3)
    assert top["model_pane"] == 0 and front["model_pane"] == 1
    assert top["camera_forward"] != front["camera_forward"]
    assert edit["edges"] > top["edges"] and front["edges"] == edit["edges"]
    assert undo["edges"] == top["edges"]
    assert maximized["model_pane"] == front["model_pane"] and maximized["view_targets"] == 2
    assert (four["model_layout"], four["view_targets"]) == ("four", 5)
    assert (other["model_layout"], other["view_targets"]) == ("single", 1)
    assert restored["model_layout"] == "four" and restored["view_targets"] == 5
    assert (single["model_layout"], single["view_targets"]) == ("single", 1)
    # Sample the window framebuffer, within each pane and outside the chrome.
    # Both views must contain geometry with a different rendered silhouette.
    from built_render import reference_texture
    import numpy as np
    width, height, pixels = reference_texture(tmp_path / "split.png", 1280)
    assert (width, height) == (1280, 800)
    rgb = pixels.reshape(height, width, 4)[:, :, :3]
    crops = [rgb[210:590, x:x+270] for x in (150, 585)]
    assert all(np.ptp(crop[:, :, 0]) > 30 for crop in crops)
    assert np.abs(crops[0].astype(float) - crops[1]).mean() > 1


@pytest.mark.parametrize(("preset", "ids"), [
    ("single", [1]), ("two", [0, 1]), ("three", [1, 0, 2]),
    ("four", [0, 2, 1, 3]), ("three_mirror", [0, 2, 1]),
    ("two_wide", [1, 0]), ("two_stacked", [1, 3]),
])
def test_model_area_is_partitioned_once(preset, ids):
    from built_render import ModelViews
    views = ModelViews()
    views.choose(preset)
    panes = views.arrange(160, 100, [-0.5, -0.5, 0.75, 0.5])
    assert [p[0] for p in panes] == ids
    assert sum(w*h for _, _, _, w, h in panes) == 100 * 50
    for index, x, y, w, h in panes:
        assert views.pointer("move", x+w/2, y+h/2, False) == (index, w/2, h/2)
    assert views.pointer("move", 4, 40, False) is None


def test_saved_split_fractions_and_invalid_legacy_values():
    from built_render import ModelViews
    views = ModelViews()
    views.restore({"layout": "four", "columns": 0.6, "rows": [0.25, 0.75]})
    assert views.arrange(300, 400, [-1., -1., 1., 1.]) == [
        (0, 0, 0, 180, 100), (2, 0, 100, 180, 300),
        (1, 180, 0, 120, 300), (3, 180, 300, 120, 100)]
    home = views.archive()
    assert home["layout"] == "four" and home["columns"] == pytest.approx(0.6)
    assert home["rows"] == [0.25, 0.75]
    views.restore({"layout": "four", "columns": float("nan"), "rows": [None, 12]})
    assert views.archive() == {"layout": "four", "columns": 0.5, "rows": [0.5, 0.5]}
    views.choose("single")
    assert views.archive() is None


def test_native_open_save_keeps_the_classic_views_home(tmp_path):
    from built_render import Host
    from built_render import shell_files as files
    from formats import igz
    from built_core.scene import Scene
    scene = Scene()
    scene.views_home = {"layout": "two_wide", "columns": 0.75, "rows": [0.5, 0.5]}
    source = tmp_path / "views.built"
    igz.save_scene(scene, source)
    host = Host(0, 900, 600, 1.0, settings=str(tmp_path / "profile"))
    try:
        assert host.open(source)
        assert host.window._doc.view_layout == "two_wide"
        destination = tmp_path / "copy.built"
        assert files.do_save(host.window, destination, copy=True)
        copy = Scene()
        igz.load_into(copy, destination)
        assert copy.views_home == scene.views_home
    finally:
        host.close()

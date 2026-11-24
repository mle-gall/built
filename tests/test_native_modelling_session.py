"""Exercise real window commands across edits, named scenes and file reload."""
import json
import os
import sys
import uuid
from pathlib import Path

import pytest

from scripts.isolated_launch import run

ROOT = Path(__file__).resolve().parents[1]


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1", reason="requires a native window")
def test_modelling_session_uses_native_handlers(tmp_path):
    binary = Path(os.environ.get("BUILT_BINARY", ROOT / "rust/built_app/target/release/built"))
    source = ROOT / "tests/fixtures/shell/slab-and-cube.igz"
    output = tmp_path / "edited.built"
    profile = tmp_path / "settings"
    plugins = profile / "data/ingetrazo/plugins"
    plugins.mkdir(parents=True)
    probe = plugins / "session_probe.py"
    probe.write_text("""import json, sys
from pathlib import Path
def setup(app):
    def snapshot():
        Path(__file__).with_suffix(".json").write_text(json.dumps({
            "classic": sorted(set(sys.modules) & {
                "views.main_window", "views.scene_tabs", "views.side_tree",
                "views.tray", "views.layers_panel", "views.outliner",
            }),
            "skp_exporter": "formats.skp_out" in sys.modules,
            "qt_pasteboard": "formats.clip" in sys.modules,
            "scenes": [v.name for v in app.scene.saved_views],
        }))
    app.add_menu_action("Inspect the session", snapshot)
""")
    script = tmp_path / "session.txt"
    script.write_text(f"""wait 350
state
menu select_all
state
menu copy
menu paste
click 450 280
key escape
tool select
state
menu select_none
menu paste_in_place
state
menu select_all
menu cut
state
menu undo
state
menu select_all
menu make_group
state
menu undo
menu select_all
menu hide
menu unhide_last
menu select_none
menu reverse_faces
menu orient_faces
menu intersect_context
menu delete_guides
menu repeat_last_command
menu invert_selection
menu select_none
state
tool line
move 240 455
wait 150
click 240 455
move 405 490
wait 150
click 405 490
state
key escape
tool select
tool move
state
tool rotate
state
tool pushpull
state
tool select
menu scene_add
menu scene_duplicate
menu scene_next
menu scene_show
menu scene_previous
wait 700
menu save_to:{output}
wait 250
state
menu close_tab
menu open_path:{output}
wait 600
state
menu select_all
menu copy
menu new
state
menu paste_in_place
state
menu undo
state
menu prev_doc
menu extension:session_probe:0
quit
""")
    result = run([binary, "--new-instance", "--root", ROOT, "--venv", ROOT / ".venv",
                  "--size", "1024x700", "--script", script, source], settings=profile,
                 env=dict(os.environ, BUILT_PASTEBOARD="built-tests-" + uuid.uuid4().hex),
                 capture_output=True, text=True, timeout=90)
    assert result.returncode == 0, result.stderr
    states = [json.loads(l.removeprefix("[built] state ")) for l in result.stdout.splitlines()
              if l.startswith("[built] state ")]
    (initial, selected, pasted, in_place, cut, restored, grouped, before_line,
     drawn, move, rotate, pushpull, saved, reopened) = states[:14]
    assert all(s["errors"] == 0 and not s["classic_window_loaded"] for s in states), result.stderr
    assert all(s["qt_modules"] == [] for s in states), [s["qt_modules"] for s in states]
    assert selected["selection"] > 0 and pasted["edges"] > initial["edges"]
    assert in_place["clipboard"] and in_place["edges"] >= pasted["edges"]
    assert cut["edges"] < in_place["edges"] and restored["edges"] == in_place["edges"]
    assert (grouped["groups"], grouped["edges"]) != (restored["groups"], restored["edges"])
    assert drawn["edges"] > before_line["edges"] and drawn["tool"] == "LineTool"
    assert [s["tool"] for s in (move, rotate, pushpull)] == ["MoveTool", "RotateTool", "PushPullTool"]
    empty_tab, adopted, undone = states[14:17]
    assert adopted["edges"] > empty_tab["edges"] and undone["edges"] == empty_tab["edges"]
    assert saved["path"] == reopened["path"] == str(output)
    assert not saved["dirty"] and not reopened["dirty"]
    assert (saved["edges"], saved["faces"], saved["groups"]) == (
        reopened["edges"], reopened["faces"], reopened["groups"])
    snapshot = json.loads(probe.with_suffix(".json").read_text())
    assert not snapshot["classic"]
    assert not snapshot["skp_exporter"]
    if sys.platform == "darwin":
        assert not snapshot["qt_pasteboard"]
    assert len(set(snapshot["scenes"])) >= 2

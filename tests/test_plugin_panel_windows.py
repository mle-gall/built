"""Windowed checks. Enable with BUILT_WINDOW_TESTS=1 on a Metal-capable Mac."""
import json
import os
from scripts import isolated_launch
from pathlib import Path

import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401

ROOT = Path(__file__).resolve().parents[1]
pytestmark = pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1",
                                reason="opens the application window")


def test_shell_console_history_and_render_dock(tmp_path):
    from tests.test_operation_form_smoke import fixture_venv
    venv = fixture_venv(tmp_path, """
from built_render import Host
state = Host.state
def panel_state(self):
    return dict(state(self), utility=self.renderer.ui_area_rect("plugin_utility"))
Host.state = panel_state
""")
    marker = tmp_path / "runs"
    script = tmp_path / "steps.txt"
    # Recalling and running the same line writes twice, through real input.
    script.write_text(f"""wait 800
menu panel:render_blender
wait 250
menu panel:python_console
wait 350
commit with open({str(marker)!r}, 'a') as stream: print('run', file=stream)
wait 150
menu new
wait 400
menu prev_doc
wait 350
menu panel:python_console
wait 300
key enter cmd
wait 300
key up
wait 100
key enter cmd
wait 300
state
quit
""")
    env = dict(os.environ, HOME=str(tmp_path), CFFIXED_USER_HOME=str(tmp_path),
               XDG_DATA_HOME=str(tmp_path / "data"), XDG_CONFIG_HOME=str(tmp_path / "config"),
               BUILT_ANSWERS="escape", BUILT_PASTEBOARD="built-plugin-smoke")
    binary = ROOT / "rust/built_app/target/release/built"
    result = isolated_launch.run([str(binary), "--settings", str(tmp_path / "settings"),
                             "--venv", str(venv), "--root", str(ROOT), "--new-instance", "--size", "1280x800",
                             "--script", str(script), str(ROOT / "tests/fixtures/shell/slab-and-cube.igz")],
                            env=env, home=tmp_path, settings=tmp_path / "settings", text=True, capture_output=True, timeout=60, check=True)
    assert marker.read_text().splitlines() == ["run", "run"]
    state = next(json.loads(line.removeprefix("[built] state ")) for line in result.stdout.splitlines()
                 if line.startswith("[built] state "))
    assert not state["errors"] and not state["blocked_widgets"]
    assert state["utility"] and min(state["utility"][2:]) > 0

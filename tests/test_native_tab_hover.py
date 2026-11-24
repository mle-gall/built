"""Tab pictures come from the background document's last GPU frame."""
import json
import os
from pathlib import Path
import sys

import pytest

from scripts.isolated_launch import run


def test_background_picture_and_shortcuts_in_a_release_window(tmp_path):
    root = Path(__file__).resolve().parents[1]
    binary = root / "rust/built_app/target/release/built"
    if sys.platform != "darwin" or not binary.is_file():
        pytest.skip("requires the macOS release shell")
    profile = tmp_path / "profile"
    plugin = profile / "data/ingetrazo/plugins/tab_probe.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text("""import json
from pathlib import Path

def setup(app):
    from core import units
    from core.settings import settings
    win = app.window
    def hover(index):
        view = win.view
        x, y, w, h = view.renderer.ui_project_tab_rects()[index]
        view.renderer.ui_pointer("move", x + w / 2, y + h / 2, 0, 0)
        view.update()
    def record():
        assert units._SCENE is win.viewport.scene
        rect = win.view.renderer.ui_area_rect("tab_card")
        with Path(__file__).with_suffix(".jsonl").open("a") as out:
            print(json.dumps(rect), file=out)
    def arm():
        def unexpected(*args, **kwargs):
            Path(__file__).with_suffix(".redrew").touch()
            raise AssertionError("rendered the model again for a tab preview")
        type(win.view).render_pixels = unexpected
        from core.shortcuts import action_keys
        from ingetrazo_render import shortcut_domains
        from ingetrazo_render.settings_sheet import SettingsSheet
        sheet = SettingsSheet("keyboard")
        sheet._win = win
        actions = sheet.actions()
        for command in ("close_tab", "next_doc", "prev_doc"):
            owned = sheet._action("sc:menu:" + command)
            assert owned and owned.portable_keys(), command
            for key in owned.portable_keys():
                active = [a for a in actions if shortcut_domains(a.objectName()) & 1]
                assert [a for a in active if key in action_keys(a)] == [owned], key
        Path(__file__).with_suffix(".armed").touch()
    for label, fn in [("Arm", arm), ("First", lambda: hover(0)), ("Second", lambda: hover(1)),
                      ("Record", record),
                      ("Off", lambda: settings().setValue("tabs/hover_picture", "0")),
                      ("On", lambda: settings().setValue("tabs/hover_picture", "1"))]:
        app.add_menu_action(label, fn)
""")
    fixture = root / "tests/fixtures/shell/slab-and-cube.igz"
    other = tmp_path / "second.igz"
    other.write_bytes(fixture.read_bytes())
    script = tmp_path / "hover.txt"
    steps = ["wait 250", "menu extension:tab_probe:0", f"open {other}", "wait 300"]
    for action in (1, 1, 4, 5, 2):
        steps.extend([f"menu extension:tab_probe:{action}", "wait 240", "menu extension:tab_probe:3"])
    script.write_text("\n".join([*steps, "quit", ""]))
    result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv",
                  "--size", "1280x640", "--script", script, fixture], settings=profile,
                 env=dict(os.environ, BUILT_START="0"), text=True, capture_output=True, timeout=60)
    assert result.returncode == 0 and "[built] error" not in result.stderr, result.stderr
    assert plugin.with_suffix(".armed").is_file(), result.stderr
    assert not plugin.with_suffix(".redrew").exists()
    first, repeated, disabled, enabled, front = [
        json.loads(row) for row in plugin.with_suffix(".jsonl").read_text().splitlines()]
    assert first[3] > 150 and first == repeated == enabled
    assert min(first[3] - disabled[3], first[3] - front[3]) > 100

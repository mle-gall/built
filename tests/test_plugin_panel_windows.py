"""Windowed checks. Enable with BUILT_WINDOW_TESTS=1 on a Metal-capable Mac."""
import json
import os
import subprocess
from pathlib import Path

import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401

ROOT = Path(__file__).resolve().parents[1]
pytestmark = pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1",
                                reason="opens the application window")


def test_shell_console_history_and_render_dock(tmp_path):
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
capture plugins
state
quit
""")
    env = dict(os.environ, HOME=str(tmp_path), CFFIXED_USER_HOME=str(tmp_path),
               XDG_DATA_HOME=str(tmp_path / "data"), XDG_CONFIG_HOME=str(tmp_path / "config"),
               BUILT_ANSWERS="escape", BUILT_PASTEBOARD="built-plugin-smoke")
    binary = ROOT / "rust/built_app/target/release/built"
    result = subprocess.run([str(binary), "--settings", str(tmp_path / "settings"),
                             "--new-instance", "--size", "1280x800", "--shots", str(tmp_path),
                             "--script", str(script), str(ROOT / "examples/arco-yanque.igz")],
                            env=env, text=True, capture_output=True, timeout=60, check=True)
    assert marker.read_text().splitlines() == ["run", "run"]
    state = next(json.loads(line.removeprefix("[built] state ")) for line in result.stdout.splitlines()
                 if line.startswith("[built] state "))
    assert not state["errors"] and not state["blocked_widgets"]
    assert (tmp_path / "plugins.png").stat().st_size > 10_000


def test_qt_metal_uses_native_panels():
    from PySide6.QtTest import QTest
    from PySide6.QtGui import QGuiApplication
    from core.plugin_panels import open_panel
    from views.main_window import MainWindow
    if QGuiApplication.platformName() != "cocoa" or os.environ.get("INGETRAZO_WGPU") != "1":
        pytest.skip("run with QT_QPA_PLATFORM=cocoa INGETRAZO_WGPU=1 BUILT_UI=1")
    win = MainWindow()
    try:
        win.show()
        win.open_path(ROOT / "examples/arco-yanque.igz")
        win._leave_empty_area()
        view = win._doc.stand_in
        win._doc_stack.setCurrentWidget(view)
        QTest.qWait(800)
        assert open_panel(win, "render_blender")
        assert open_panel(win, "python_console")
        QTest.qWait(250)
        assert view.renderer.ui_area_rect("plugin_utility")
    finally:
        win.close()

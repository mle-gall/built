"""Apply edge softening and raster export through a real native window."""
import json
import os
from pathlib import Path

import pytest

from scripts.isolated_launch import run

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "rust/built_app/target/release/built"


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_soften_and_export(tmp_path):
    from PySide6.QtGui import QImage
    from core.group import Group
    from core.mesh import Mesh
    from core.scene import Scene
    from formats.igz import save_scene
    from tests.test_viewport_smooth_shading import _cylinder

    model = tmp_path / "cylinder.igz"
    save_scene(Scene(groups=[Group(_cylinder(Mesh(), soft=False))]), model)
    picture = tmp_path / "cylinder.png"
    steps = tmp_path / "commands.txt"
    steps.write_text(f"""wait 800
key escape
menu select_all
menu soften
wait 400
key enter
wait 300
state
menu export_image_to:{picture}
wait 400
key enter
wait 600
state
quit
""")
    (tmp_path / "home").mkdir()
    result = run([str(BIN), "--root", str(ROOT), "--new-instance", "--size", "1280x800",
                  "--script", str(steps), str(model)],
                 env=dict(os.environ, BUILT_ANSWERS="escape"),
                 home=tmp_path / "home", settings=tmp_path / "profile",
                 capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    assert result.returncode == 0, log
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert len(states) >= 2 and all(s["errors"] == 0 for s in states), log
    assert states[0]["status"] == "Smoothed 12 edge(s)", log
    image = QImage(str(picture))
    assert image.width() == 3840 and image.height() > 0, log

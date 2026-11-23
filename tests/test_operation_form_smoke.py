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
key tab
wait 100
key down
wait 100
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
    assert image.width() == 3520 and image.height() > 0, log


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_release_review(tmp_path):
    """Install a release backend fixture in the embedded interpreter only."""
    import site
    import sys
    from core.scene import Scene
    from formats.igz import save_scene

    packages = tmp_path / "venv" / "lib" / f"python{sys.version_info.major}.{sys.version_info.minor}" / "site-packages"
    packages.mkdir(parents=True)
    opened = tmp_path / "download-url"
    release_url = "https://example.invalid/releases/test"
    (packages / "release_fixture.py").write_text(
        "from core import desktop_open, update_check\n"
        "from pathlib import Path\n"
        f"update_check.fetch_latest = lambda: update_check.Release('999', '2026-01-01', '## Fixed', {release_url!r})\n"
        f"desktop_open.open_url = lambda url: Path({str(opened)!r}).write_text(url)\n")
    (packages / "fixture.pth").write_text("\n".join(site.getsitepackages()) +
        f"\nimport site; site.addsitedir({site.getsitepackages()[0]!r}); import release_fixture\n")
    model = tmp_path / "blank.igz"
    save_scene(Scene(), model)
    steps = tmp_path / "steps.txt"
    steps.write_text("wait 500\nkey escape\nmenu check_updates\nwait 700\nstate\nkey enter\n"
                     "wait 200\nmenu select_all\nstate\nquit\n")
    home = tmp_path / "home"
    home.mkdir()
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(tmp_path / "venv"),
                  "--new-instance", "--script", str(steps), str(model)],
                 env=dict(os.environ, BUILT_ANSWERS="escape"), home=home, settings=tmp_path / "profile",
                 capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    assert result.returncode == 0 and opened.exists(), log
    assert opened.read_text() == release_url
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert [s["large_sheet"] for s in states[:2]] == ["update", ""], log
    assert all(s["errors"] == 0 and not s["blocked_widgets"] for s in states), log

"""Open, apply and close migrated chrome operations in a native window."""
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


def fixture_venv(tmp_path, code):
    import site
    import sys
    venv = tmp_path / "venv"
    packages = venv / "lib" / f"python{sys.version_info.major}.{sys.version_info.minor}" / "site-packages"
    packages.mkdir(parents=True)
    (packages / "window_fixture.py").write_text(code)
    (packages / "fixture.pth").write_text("\n".join(site.getsitepackages()) +
        f"\nimport site; site.addsitedir({site.getsitepackages()[0]!r}); import window_fixture\n")
    return venv


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_release_review(tmp_path):
    """Install a release backend fixture in the embedded interpreter only."""
    from core.scene import Scene
    from formats.igz import save_scene
    opened = tmp_path / "download-url"
    release_url = "https://example.invalid/releases/test"
    venv = fixture_venv(tmp_path,
        "from core import desktop_open, update_check\n"
        "from pathlib import Path\n"
        f"update_check.fetch_latest = lambda: update_check.Release('999', '2026-01-01', '## Fixed', {release_url!r})\n"
        f"desktop_open.open_url = lambda url: Path({str(opened)!r}).write_text(url)\n")
    model = tmp_path / "blank.igz"
    save_scene(Scene(), model)
    steps = tmp_path / "steps.txt"
    steps.write_text("wait 500\nkey escape\nmenu check_updates\nwait 700\nstate\nkey enter\n"
                     "wait 200\nmenu select_all\nstate\nquit\n")
    home = tmp_path / "home"
    home.mkdir()
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv),
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


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_first_run_custom_units(tmp_path):
    from PySide6.QtCore import QSettings

    profile = tmp_path / "profile"
    folder = profile / "IngeTrazo"
    folder.mkdir(parents=True)
    settings = QSettings(str(folder / "IngeTrazo.ini"), QSettings.IniFormat)
    custom = {"length": "cm", "precision": 3}
    settings.setValue("units/new_document", json.dumps(custom))
    settings.sync()
    steps = tmp_path / "commands"
    # In the 1000 × 920 Units sheet, click “Also use for new documents”.
    steps.write_text("wait 700\nmenu home\nwait 200\nmenu new\nwait 300\nstate\n"
                     "key enter\nwait 300\nstate\nkey enter\nwait 300\nstate\n"
                     "menu units\nwait 300\nstate\nclick 391 655\nwait 250\n"
                     "key escape\nwait 300\nstate\nquit\n")
    home = tmp_path / "home"
    home.mkdir()
    result = run([str(BIN), "--root", str(ROOT), "--new-instance", "--size", "1000x920",
                  "--script", str(steps)],
                 home=home, settings=profile, capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and len(states) >= 3, log
    assert [s["form_open"] for s in states[:3]] == [True, True, False], log
    assert all(s["errors"] == 0 and not s["blocked_widgets"] for s in states), log
    assert [s["large_sheet"] for s in states[3:5]] == ["settings", ""], log
    assert not states[2]["dirty"]
    settings.sync()
    assert json.loads(settings.value("units/new_document")) == custom
    assert settings.value("units/use_for_new_documents") == "0"


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_settings_device_preferences(tmp_path):
    from PySide6.QtCore import QSettings

    profile = tmp_path / "profile"
    home = tmp_path / "home"
    home.mkdir()
    steps = tmp_path / "commands"
    # Navigate from the sidebar to the device pane and toggle navigation.
    steps.write_text("wait 600\nkey escape\nmenu prefs\nwait 400\nstate\n"
                     "click 119 282\nwait 250\nclick 426 234\nwait 250\n"
                     "key escape\nwait 300\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--new-instance", "--size", "1000x720",
                  "--script", str(steps)], home=home, settings=profile,
                 capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and [s["large_sheet"] for s in states[:2]] == ["settings", "start"], log
    assert not any(s["errors"] or s["blocked_widgets"] for s in states), log
    settings = QSettings(str(profile / "IngeTrazo" / "IngeTrazo.ini"), QSettings.IniFormat)
    assert settings.value("ndof/enabled") == "0"


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_about_copies_diagnostics(tmp_path):
    copied = tmp_path / "copied.txt"
    venv = fixture_venv(tmp_path,
        "import built_shell\nfrom pathlib import Path\n"
        f"built_shell.pasteboard_write = lambda items: Path({str(copied)!r}).write_bytes(items[0][1])\n")
    home = tmp_path / "home"
    home.mkdir()
    steps = tmp_path / "steps"
    steps.write_text("wait 600\nmenu about\nwait 350\nstate\nclick 320 264\nwait 250\n"
                     "key escape\nwait 300\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--new-instance", "--venv", str(venv),
                  "--size", "1000x720", "--script", str(steps)],
                 home=home, settings=tmp_path / "profile", capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and [s["large_sheet"] for s in states[:2]] == ["about", "start"], log
    assert not any(s["errors"] or s["blocked_widgets"] for s in states), log
    assert copied.exists() and "Window: 1000x720" in copied.read_text(), log


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_section_name_question(tmp_path):
    from core.scene import Scene
    from formats.igz import save_scene

    model = tmp_path / "empty.igz"
    save_scene(Scene(), model)
    steps = tmp_path / "commands"
    steps.write_text("wait 650\nkey escape\ntool section\nwait 300\n"
                     "click 510 435\nwait 350\nstate\nkey enter\nwait 250\nstate\nquit\n")
    home = tmp_path / "home"
    home.mkdir()
    result = run([str(BIN), "--root", str(ROOT), "--new-instance", "--size", "1000x720",
                  "--script", str(steps), str(model)], home=home, settings=tmp_path / "settings",
                 capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and [s["form_open"] for s in states[:2]] == [True, False], log
    assert not any(s["errors"] or s["blocked_widgets"] for s in states), log

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
    assert copied.exists(), log
    report = copied.read_text()
    assert "Window: 1000x720" in report and "Renderer: wgpu (" in report, report


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


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_extensions_open_painted_and_apply(tmp_path):
    home = tmp_path / "home"
    home.mkdir()
    plugins = tmp_path / "plugins"
    venv = fixture_venv(tmp_path,
        "from pathlib import Path\nfrom core import extension_manager as em\n"
        f"em.user_plugins_dir = lambda: Path({str(plugins)!r})\n"
        "em.load_cached_catalog = lambda: None\n"
        "em.refresh_catalog = lambda: em.CatalogSnapshot([], {}, '', False)\n")
    steps = tmp_path / "commands"
    steps.write_text("wait 600\nmenu extensions\nwait 450\nstate\nclick 96 648\nwait 300\n"
                     "key escape\nwait 300\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "1000x720", "--script", str(steps)],
                 home=home, settings=tmp_path / "profile", capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and [s["large_sheet"] for s in states[:2]] == ["extensions", "start"], log
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), log
    assert (plugins / ".safe-mode").is_file(), log


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_location_coordinates_map_apply_and_cancel(tmp_path):
    from core.scene import Scene
    from formats.igz import save_scene, load_into
    from georef.datum import SceneDatum

    model = tmp_path / "site.igz"
    saved = tmp_path / "edited.built"
    scene = Scene()
    scene.north_deg = 31
    save_scene(scene, model)
    home = tmp_path / "home"
    home.mkdir()
    requested = tmp_path / "tile"
    venv = fixture_venv(tmp_path,
        "from georef.tile_fetcher import TileFetcher\nfrom pathlib import Path\n"
        f"TileFetcher.request = lambda self, src, x, y, z: Path({str(requested)!r}).touch()\n")
    steps = tmp_path / "commands"
    steps.write_text("wait 600\nkey escape\nmenu prefs\nwait 350\nclick 116 460\nwait 150\n"
                     "click 552 328\nwait 200\nclick 550 282\nkey a cmd\ntype 48.85\n"
                     "click 550 311\nkey a cmd\ntype 2.35\nclick 340 351\nwait 450\n"
                     "drag 590 585 625 585 left 7\nwait 250\nstate\nclick 492 747\nwait 250\n"
                     "click 552 328\nwait 200\nclick 550 282\nkey a cmd\ntype 33\n"
                     f"click 431 351\nwait 200\nkey escape\nwait 200\nmenu save_to:{saved}\nwait 500\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "1000x920", "--script", str(steps), str(model)],
                 env=dict(os.environ, BUILT_ANSWERS="escape,cmd-d,cmd-d"), home=home, settings=tmp_path / "profile",
                 capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and [s["large_sheet"] for s in states[:2]] == ["settings", ""], log
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), log
    assert requested.is_file(), log
    loaded = Scene()
    load_into(loaded, saved)
    assert isinstance(loaded.georef, SceneDatum), log
    assert loaded.georef.lat == pytest.approx(48.85, abs=1e-5)
    assert loaded.georef.lon == pytest.approx(2.35 - 35 * 360 / (256 * 2**12), abs=.0004)
    assert loaded.georef.north == 31


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_online_catalogue_places_the_credited_component(tmp_path):
    import zipfile
    from core.scene import Scene
    from formats.igz import save_scene, load_into

    src = tmp_path / "catalogue"
    (src / "modelos").mkdir(parents=True)
    entry = {"id": "stool", "nombre": "Taburete", "nombre_en": "Stool",
             "categoria": "Cocina", "autor": "Ana", "licencia": "CC BY", "obj": "stool.obj"}
    (src / "index.json").write_text(json.dumps({"modelos": [entry]}))
    with zipfile.ZipFile(src / "modelos/stool.zip", "w") as package:
        package.writestr("stool.obj", "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")
    initial, saved = tmp_path / "empty.built", tmp_path / "placed.built"
    save_scene(Scene(), initial)
    steps = tmp_path / "commands"
    steps.write_text("wait 600\nkey escape\nmenu library\nwait 450\nclick 174 135\nwait 250\n"
                     "click 118 335\nwait 600\nstate\nkey enter\nwait 600\nstate\n"
                     f"click 525 540\nwait 300\nkey escape\nmenu save_to:{saved}\nwait 450\nquit\n")
    home = tmp_path / "home"
    home.mkdir()
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(ROOT / ".venv"), "--new-instance",
                  "--size", "1000x920", "--script", str(steps), str(initial)],
                 env=dict(os.environ, INGETRAZO_LIBRARY=str(src), INGETRAZO_TEXTURE_CACHE=str(tmp_path / "cache/tex"),
                          BUILT_ANSWERS="escape,cmd-d"), home=home, settings=tmp_path / "profile",
                 capture_output=True, text=True, timeout=60)
    log = result.stdout + result.stderr
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and [s["large_sheet"] for s in states[:2]] == ["library", ""], log
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), log
    scene = Scene()
    load_into(scene, saved)
    assert any((g.ext or {}).get("catalogue", {}).get("autor") == "Ana" for g in scene.groups), log


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_f3_finds_and_runs_a_misspelled_command(tmp_path):
    from core.scene import Scene
    from formats.igz import save_scene
    path = tmp_path / "empty.built"
    save_scene(Scene(), path)
    steps = tmp_path / "f3"
    steps.write_text("wait 600\nkey escape\nkey f3\nwait 250\ntype cut lsit\nwait 300\n"
                     "key enter\nwait 350\nstate\nkey escape\nwait 300\nstate\nquit\n")
    home = tmp_path / "home"
    home.mkdir()
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(ROOT / ".venv"),
                  "--new-instance", "--size", "1000x720", "--script", str(steps), str(path)],
                 env=dict(os.environ, BUILT_ANSWERS="escape"), home=home,
                 settings=tmp_path / "profile", capture_output=True, text=True, timeout=45)
    log = result.stdout + result.stderr
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and [s["large_sheet"] for s in states[:2]] == ["cutlist", ""], log
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), log


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists()
                    or __import__("sys").platform != "darwin", reason="requires the native macOS menu")
def test_native_menu_search_and_persisted_keyboard_binding(tmp_path):
    from PySide6.QtCore import QSettings
    from core.scene import Scene
    from formats.igz import save_scene
    from views.shortcuts import _store_key
    model = tmp_path / "empty.built"
    save_scene(Scene(), model)
    profile, home = tmp_path / "profile", tmp_path / "home"
    config = profile / "IngeTrazo"
    config.mkdir(parents=True)
    home.mkdir()
    settings = QSettings(str(config / "IngeTrazo.ini"), QSettings.IniFormat)
    settings.setValue("shortcuts/" + _store_key("menu:cutlist"), "Ctrl+Shift+F9")
    settings.sync()
    steps = tmp_path / "menu-commands"
    steps.write_text("wait 800\nkey escape\nkey f9 cmd+shift\nwait 350\nstate\nkey escape\n"
                     "key f3\nwait 200\ntype new window\nwait 300\nkey enter\nwait 800\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(ROOT / ".venv"), "--new-instance",
                  "--size", "900x700", "--script", str(steps), str(model)], home=home, settings=profile,
                 env=dict(os.environ, BUILT_ANSWERS="escape"), capture_output=True, text=True, timeout=60)
    log = result.stdout + result.stderr
    states = [json.loads(s.removeprefix("[built] state ")) for s in log.splitlines() if s.startswith("[built] state ")]
    assert result.returncode == 0 and states[0]["large_sheet"] == "cutlist", log
    assert states[1]["windows"] == 2 and all(not s["errors"] for s in states), log


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_ai_connection_edits_plugin_model(tmp_path):
    from PySide6.QtCore import QSettings

    home, profile = tmp_path / "home", tmp_path / "settings"
    home.mkdir()
    steps = tmp_path / "ai-settings"
    steps.write_text("wait 650\nmenu prefs\nwait 350\nclick 115 353\nwait 200\n"
                     "click 620 263\ntype fixture-model\nkey enter\nwait 250\nstate\n"
                     "key escape\nwait 300\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--new-instance", "--size", "1000x720",
                  "--script", str(steps)], home=home, settings=profile, capture_output=True,
                 text=True, timeout=60)
    states = [json.loads(s.removeprefix("[built] state ")) for s in result.stdout.splitlines()
              if s.startswith("[built] state ")]
    assert result.returncode == 0 and not any(s["errors"] for s in states), result.stderr
    assert [s["large_sheet"] for s in states[:2]] == ["settings", "start"]
    store = QSettings(str(profile / "IngeTrazo" / "IngeTrazo.ini"), QSettings.IniFormat)
    assert store.value("ia/modelo") == "fixture-model"


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_svg_preview_and_delivery_edit(tmp_path):
    from PySide6.QtGui import QImage
    from core import deliveries
    from core.history import History
    from core.scene import Scene
    from formats.igz import load_into, save_scene
    from tests.test_ifc_export import _tagged_wall_scene

    initial, saved = tmp_path / "wall.built", tmp_path / "edited.built"
    drawing = tmp_path / "wall.svg"
    scene = _tagged_wall_scene()
    key = deliveries.create_delivery(History(scene), dict(name="Wall drawing", format="svg",
        scope="visible", destination=str(tmp_path / "named.svg"), options={"paper": [297, 210]}))["key"]
    save_scene(scene, initial)
    venv = fixture_venv(tmp_path,
        f"from app_shell import native\nnative.save_file = lambda *args: {str(drawing)!r}\n")
    steps = tmp_path / "steps"
    steps.write_text("wait 700\nkey escape\nmenu delivery_export\nwait 650\nclick 630 208\n"
                     "wait 450\nclick 470 377\nwait 450\nclick 843 269\nwait 250\n"
                     "click 260 329\nwait 900\ncapture one-off\nstate\nclick 392 127\n"
                     "wait 650\nstate\nclick 1160 72\nwait 600\nclick 885 107\n"
                     "wait 120\nclick 890 222\nwait 650\nclick 260 329\nwait 900\n"
                     "capture named\nstate\ndrag 990 370 1030 380 left 8\nwait 600\n"
                     f"key enter\nwait 400\nstate\nmenu save_to:{saved}\nwait 600\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "1280x800", "--shots", str(tmp_path), "--script", str(steps), str(initial)],
                 env=dict(os.environ, BUILT_ANSWERS="escape,cmd-d,cmd-d"),
                 capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    assert result.returncode == 0 and "Traceback" not in log, log
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert [s["large_sheet"] for s in states[:4]] == ["export", "", "delivery", ""], log
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), log
    for name in ("one-off", "named"):
        frame = QImage(str(tmp_path / f"{name}.png"))
        assert not frame.isNull()
        assert frame.pixelColor(int(frame.width() * .79), int(frame.height() * .45)).lightness() > 230
    assert "<path" in drawing.read_text()
    edited = Scene()
    load_into(edited, saved)
    frame = deliveries.records(edited)[key]["options"]["frame"]
    assert frame[0] < 0 and frame[1] > 0 and frame[2] == 1


@pytest.mark.skipif(
    os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
    reason="requires a built window and BUILT_WINDOW_TESTS=1",
)
def test_current_view_svg_modes_stay_centred_and_export(tmp_path):
    from formats.igz import save_scene
    from tests.test_ifc_export import _tagged_wall_scene

    drawing, initial = tmp_path / "current.svg", tmp_path / "wall.built"
    save_scene(_tagged_wall_scene(), initial)
    venv = fixture_venv(
        tmp_path,
        f"""from app_shell import native
from app_shell.host import Host
native.save_file = lambda *args: {str(drawing)!r}
original_state = Host.state
def state(self):
    return dict(original_state(self), rect=self.renderer.ui_large_sheet_rect())
Host.state = state
""",
    )
    steps = tmp_path / "steps"
    steps.write_text(
        "wait 700\nkey escape\nmenu export_view_svg\nwait 600\nstate\n"
        "click 847 178\nwait 160\nclick 228 212\nwait 1200\nstate\n"
        "click 95 212\nwait 400\nstate\nkey enter\nwait 650\nstate\nquit\n"
    )
    result = run(
        [
            str(BIN),
            "--root",
            str(ROOT),
            "--venv",
            str(venv),
            "--new-instance",
            "--size",
            "1280x800",
            "--script",
            str(steps),
            str(initial),
        ],
        capture_output=True,
        text=True,
        timeout=60,
    )
    log = result.stdout + result.stderr
    assert result.returncode == 0 and "Traceback" not in log, log
    states = [
        json.loads(line.removeprefix("[built] state "))
        for line in result.stdout.splitlines()
        if line.startswith("[built] state ")
    ]
    assert [s["large_sheet"] for s in states[:4]] == ["export", "export", "export", ""], log
    assert not any(s["errors"] or s["blocked_widgets"] for s in states), log
    rects = [s["rect"] for s in states[:3]]
    assert rects[0] == rects[2] and rects[1][2] > rects[0][2]
    for x, y, w, h in rects:
        assert x >= 0 and y >= 0 and x + w <= 1280 and y + h <= 800
        assert x + w / 2 == pytest.approx(640, abs=1)
    assert "<image" in drawing.read_text()


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_bevel_inspector_edit_apply_and_leave(tmp_path):
    from formats.igz import save_scene
    from tests.test_ifc_export import _tagged_wall_scene

    initial = tmp_path / "wall.built"
    save_scene(_tagged_wall_scene(), initial)
    venv = fixture_venv(tmp_path, """from tools.fillet import FilletTool
from app_shell.host import Host
FilletTool.shape = "steps"
FilletTool.radius = 0.01
state = Host.state
def bevel_state(self):
    tool = self.viewport.active_tool
    return dict(state(self), curve=getattr(tool, "shape", None), picked=len(getattr(tool,"edges",[])),
                sizing=getattr(tool,"sizing",False), revision=self.viewport.history.revision)
Host.state = bevel_state
""")
    steps = tmp_path / "steps"
    steps.write_text("wait 650\nkey escape\nmenu select_all\ntool fillet\nwait 500\nstate\n"
                     "click 1115 466\nclick 1016 490\nwait 400\nstate\n"
                     "click 1025 466\nclick 1016 490\nwait 400\nstate\n"
                     "click 1130 490\nwait 300\nstate\nclick 700 781\nwait 500\nstate\n"
                     "click 752 781\nwait 300\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "1280x800", "--script", str(steps), str(initial)],
                 env=dict(os.environ, BUILT_ANSWERS="cmd-d"), capture_output=True, text=True, timeout=60)
    log = result.stdout + result.stderr
    assert result.returncode == 0 and "Traceback" not in log, log
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    start, corner, smooth, removed, applied, done = states[:6]
    assert start["curve"] == "steps" and start["picked"] > 0
    assert isinstance(corner["curve"], list) and not any(p[2] for p in corner["curve"])
    assert len(smooth["curve"]) == len(corner["curve"])+1 and sum(p[2] for p in smooth["curve"]) == 1
    assert removed["curve"] == corner["curve"]
    assert all(not s["sizing"] and s["picked"] == start["picked"] and s["revision"] == start["revision"]
               for s in (corner,smooth,removed))
    assert applied["revision"] == start["revision"]+1 and applied["faces"] > start["faces"]
    assert done["tool"] != "FilletTool" and done["curve"] is None
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states)


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_camera_registry_text_and_plugin_utilities(tmp_path):
    """Use renderer verbs for controls without a native menu-script spelling."""
    from PySide6.QtGui import QImage
    from core.scene import Scene
    from formats.igz import save_scene

    initial = tmp_path / "empty.built"
    save_scene(Scene(), initial)
    image = tmp_path / "render.png"
    thumb = QImage(90, 40, QImage.Format_RGB32)
    thumb.fill(0x557788)
    assert thumb.save(str(image))
    marker = tmp_path / "preview-closed"
    venv = fixture_venv(tmp_path, f"""from app_shell.host import Host
from core import bim_registry
from ingetrazo_render import RenderResult
from core.plugin_panels import show_utility
from views import chrome_sheet, wgpu_ui
from pathlib import Path
menu, state = Host.menu, Host.state
def fixture_menu(self, command):
    if command == "fixture-camera":
        wgpu_ui._run(self.window, ["tab:scene:add", "tab:scene:editcamera"])
    elif command == "fixture-shift":
        wgpu_ui._run(self.window, ['panel:["camera.shift_y",0.25]'])
    elif command == "fixture-close-camera":
        wgpu_ui._run(self.window, ['panel:["camera.close",null]'])
    elif command == "fixture-registry":
        chrome_sheet.action(self.window, chrome_sheet.SEP.join(["library","tab","registry"]))
    elif command == "fixture-namespace":
        for args in (["v","registry_key","survey"],["v","registry_name","Survey"],["verb","registry_add"]):
            chrome_sheet.action(self.window, chrome_sheet.SEP.join(["library",*args]))
    elif command == "fixture-apply":
        chrome_sheet.action(self.window, chrome_sheet.SEP.join(["library","verb","registry_apply"]))
    elif command == "fixture-text":
        self.window._act_3dtext.trigger()
    elif command == "fixture-preview":
        show_utility(self.window, "test.result", RenderResult({str(image)!r}, False, lambda: None,
                     lambda: None, lambda on: Path({str(marker)!r}).write_text(str(on))))
    else:
        return menu(self, command)
    self.request_frame()
def fixture_state(self):
    lines = next((b.model.lines for b in getattr(self.view, "_plugin_bindings", {{}}).values()
                  if b.key == "python_console"), [])
    cam = getattr(self.view, "_camera_inspector", None)
    saved = self.window._scene_tabs.marked_view()
    return dict(state(self), bound=bool(cam and cam.observe(self.viewport.camera, self.viewport.scene, saved)),
                camera_open=bool(cam), saved_shift=None if saved is None else saved.lens.get("shift_y"),
                namespace="survey" in bim_registry.get(self.viewport.scene)["classification_systems"],
                font=bool(getattr(self.view, "_font_done", None)), console=lines,
                utility=self.renderer.ui_area_rect("plugin_utility"))
Host.menu, Host.state = fixture_menu, fixture_state
""")
    steps = tmp_path / "steps"
    steps.write_text("""wait 800
key escape
menu fixture-camera
wait 250
state
menu fixture-shift
menu fixture-close-camera
wait 300
state
menu library
wait 350
menu fixture-registry
wait 300
menu fixture-namespace
wait 150
state
menu fixture-apply
wait 400
state
menu fixture-text
wait 400
state
key enter
wait 400
state
key escape
menu panel:python_console
wait 500
type 40+2
key enter cmd
wait 350
state
click 933 591
wait 220
menu fixture-preview
wait 500
state
click 933 591
wait 280
quit
""")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "1280x800", "--script", str(steps), str(initial)],
                 env=dict(os.environ, BUILT_ANSWERS="cmd-d"), capture_output=True, text=True, timeout=60)
    log = result.stdout + result.stderr
    assert result.returncode == 0 and "Traceback" not in log, log
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    camera, edited, draft, applied, picker, placed, console, preview = states[:8]
    assert camera["bound"] and edited["saved_shift"] == .25 and not edited["camera_open"], log
    assert draft["large_sheet"] == "library" and not draft["namespace"], log
    assert applied["namespace"] and not applied["large_sheet"], log
    assert picker["font"] and not placed["font"] and placed["tool"] == "PlaceGroupTool", log
    assert "42" in console["console"] and preview["utility"], log
    assert marker.read_text() == "False", log
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), log

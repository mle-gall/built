"""Open, apply and close migrated chrome operations in a native window."""
from tests.raster_samples import solid_raster
import json
import os
from pathlib import Path

import pytest

from scripts.isolated_launch import run

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "rust/built_app/target/release/built"


def english(profile):
    """The scripted runs below type and click on English text and layout;
    a profile without a language follows the system's."""
    from ingetrazo_core import NativeSettings
    profile.mkdir(parents=True, exist_ok=True)
    st = NativeSettings(str(profile)).fresh()
    st.setValue("language", "en")
    st.sync()
    return profile


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_soften_and_export(tmp_path):
    from ingetrazo_render import SurveyRaster
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
                 home=tmp_path / "home", settings=english(tmp_path / "profile"),
                 capture_output=True, text=True, timeout=90)
    log = result.stdout + result.stderr
    assert result.returncode == 0, log
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert len(states) >= 2 and all(s["errors"] == 0 for s in states), log
    assert states[0]["status"] == "Smoothed 12 edge(s)", log
    image = SurveyRaster.open(picture)
    assert image.width == 3520 and image.height > 0, log


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
        "from ingetrazo_core import desktop_open\n"
        "from ingetrazo_render import release_check\n"
        "from pathlib import Path\n"
        f"release_check.fetch_latest = lambda: release_check.Release('999', '2026-01-01', '## Fixed', {release_url!r})\n"
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
    from ingetrazo_core import NativeSettings

    profile = tmp_path / "profile"
    folder = profile / "IngeTrazo"
    folder.mkdir(parents=True)
    settings = NativeSettings(profile)
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
    from ingetrazo_core import NativeSettings

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
    settings = NativeSettings(profile)
    assert settings.value("ndof/enabled") == "0"


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_about_copies_diagnostics(tmp_path):
    copied = tmp_path / "copied.txt"
    venv = fixture_venv(tmp_path,
        "import built_shell\nfrom pathlib import Path\n"
        f"built_shell.pasteboard_write = lambda items: Path({str(copied)!r}).write_bytes(items[0][1]) == len(items[0][1])\n")
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
        "from pathlib import Path\nfrom ingetrazo_core import extension_manager as em\n"
        f"from core import extensions\nextensions.user_plugins_dir = lambda: Path({str(plugins)!r})\n"
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
    steps.write_text("wait 600\nkey escape\nmenu prefs\nwait 350\nclick 116 510\nwait 150\n"
                     "click 552 328\nwait 200\nclick 550 282\nkey a cmd\ntype 48.85\n"
                     "click 550 311\nkey a cmd\ntype 2.35\nclick 340 351\nwait 450\n"
                     "drag 590 585 625 585 left 7\nwait 250\nstate\nclick 492 747\nwait 250\n"
                     "click 552 328\nwait 200\nclick 550 282\nkey a cmd\ntype 33\n"
                     f"click 431 351\nwait 200\nkey escape\nwait 200\nmenu save_to:{saved}\nwait 500\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "1000x920", "--script", str(steps), str(model)],
                 env=dict(os.environ, BUILT_ANSWERS="escape,cmd-d,cmd-d"), home=home, settings=english(tmp_path / "profile"),
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
    from ingetrazo_core import NativeSettings
    from core.scene import Scene
    from formats.igz import save_scene
    from ingetrazo_render.keyboard_bindings import settings_key
    model = tmp_path / "empty.built"
    save_scene(Scene(), model)
    profile, home = tmp_path / "profile", tmp_path / "home"
    config = profile / "IngeTrazo"
    config.mkdir(parents=True)
    home.mkdir()
    settings = NativeSettings(profile)
    settings.setValue("shortcuts/" + settings_key("menu:cutlist"), "Ctrl+Shift+F9")
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
    from ingetrazo_core import NativeSettings

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
    store = NativeSettings(profile)
    assert store.value("ia/modelo") == "fixture-model"


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_svg_preview_and_delivery_edit(tmp_path):
    from ingetrazo_render import SurveyRaster
    from ingetrazo_core import deliveries
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
        f"from ingetrazo_render import shell_services as native\nnative.save_file = lambda *args: {str(drawing)!r}\n")
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
        frame = SurveyRaster.open(tmp_path / f"{name}.png")
        assert frame is not None
        from tests.raster_samples import pixels
        rgb = pixels(frame)[int(frame.height * .45), int(frame.width * .79), :3].astype(int)
        assert (rgb.min() + rgb.max()) / 2 > 230
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
        f"""from ingetrazo_render import shell_services as native
from ingetrazo_render import Host
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
from ingetrazo_render import Host
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
    from ingetrazo_render import SurveyRaster
    from core.scene import Scene
    from formats.igz import save_scene

    initial = tmp_path / "empty.built"
    save_scene(Scene(), initial)
    image = tmp_path / "render.png"
    thumb = solid_raster(90, 40, (85, 119, 136))
    assert thumb.save(str(image))
    marker = tmp_path / "preview-closed"
    venv = fixture_venv(tmp_path, f"""from ingetrazo_render import Host
from core import bim_registry
from ingetrazo_render import RenderResult
from core.plugin_panels import show_utility
from ingetrazo_render import chrome_sheet
from views import wgpu_ui
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
    group = getattr(self.viewport.active_tool, "_group", None)
    selected_font = (getattr(group, "text3d", None) or {{}}).get("font")
    return dict(state(self), selected_font=selected_font, bound=bool(cam and cam.observe(self.viewport.camera, self.viewport.scene, saved)),
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
    assert placed["selected_font"], log
    assert "42" in console["console"] and preview["utility"], log
    assert marker.read_text() == "False", log
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), log


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_drawing_catalog_and_scene_folders_in_native_window(tmp_path):
    from tests.composite_fixture import _file

    model = _file(tmp_path / "drawing.igz", {"Site": None}, {"Paths": "Site"}, 2)
    output = tmp_path / "drawing.pat"
    venv = fixture_venv(tmp_path, f"""
import json
from ingetrazo_render import Host
from views import wgpu_ui
from ingetrazo_render.shell_files import ShellFileDialog
menu, state = Host.menu, Host.state
def dispatch(self, command):
    if not command.startswith("fixture-"):
        return menu(self, command)
    scene = self.viewport.scene
    def act(verb, op, data):
        wgpu_ui._tree_action(self.window,self.view,wgpu_ui.SEP.join((verb,op,json.dumps(data))))
    if command=="fixture-open":
        wgpu_ui.open_line_type(self.window,"",["Paths"])
    elif command=="fixture-line":
        act("editline","save",dict(name="Boundary",pieces=[8,-2],tags=["Paths"]))
    elif command=="fixture-hatch":
        wgpu_ui._tree_action(self.window,self.view,"drawingpatch"+wgpu_ui.SEP+json.dumps([["Paths"],dict(hatch="brick",mortar="1",hatch_ground="layer")]))
        act("documentpiece","brick",[250,115,60,8])
        from core.settings import settings
        pref=self.view.renderer.ui_hatch_preference
        for key in ("glass","brick","glass","glass"):
            pref("pin",key)
        for key in ("brick","glass","brick"):
            pref("recent",key)
        own=json.dumps(["brick",[300,150,80,12,8]])
        pref("piece",own)
        self.fixture_hatch_prefs=dict(pinned=settings().value("layers/favourite_hatches"),
            recent=settings().value("layers/recent_hatches"),
            saved=json.loads(settings().value("layers/own_pieces")))
        pref("piece",own)
        self.fixture_hatch_prefs["removed"]=json.loads(settings().value("layers/own_pieces"))
        ShellFileDialog.getSaveFileName=lambda *args: ({str(output)!r},"")
        wgpu_ui._tree_action(self.window,self.view,"exportpat"+wgpu_ui.SEP)
    elif command=="fixture-folder":
        act("tagfolder","new",dict(name="Survey",parent="Site"))
        folder=next(key for key in scene.layer_folders if scene.layer_folder_names.get(key,key)=="Survey")
        act("tagfolder","new",dict(name="Details",parent="Site"))
        detail=next(key for key in scene.layer_folders if scene.layer_folder_names.get(key,key)=="Details")
        act("tagfolder","batch",dict(owner=id(scene),parent=folder,folders=[detail],tags=["Paths"]))
        self.fixture_mixed_folder=scene.folder_parents.get(detail)==folder
    elif command=="fixture-tag-controls":
        from ingetrazo_core import tag_edits
        wgpu_ui._tree_action(self.window,self.view,wgpu_ui.SEP.join(("tagcolours","1")))
        wgpu_ui._tree_action(self.window,self.view,wgpu_ui.SEP.join(("modellines","0")))
        template={str(tmp_path / "drawing.json")!r}
        ShellFileDialog.getSaveFileName=lambda *args: (template,"")
        ShellFileDialog.getOpenFileName=lambda *args: (template,"")
        for verb in ("exporttags","importtags"):
            wgpu_ui._tree_action(self.window,self.view,verb+wgpu_ui.SEP)
        folder=tag_edits.add_folder(scene,"Temporary tags")
        wgpu_ui._tree_action(self.window,self.view,wgpu_ui.SEP.join(("newtag","preview-folder:"+folder)))
        tag=scene.layers[-1]
        self.fixture_tag=tag.key
        self.fixture_filed_tag=tag.folder==folder
        wgpu_ui._tree_action(self.window,self.view,wgpu_ui.SEP.join(("rename","preview-tag:"+tag.key,"Unused survey tag")))
        wgpu_ui._tree_action(self.window,self.view,wgpu_ui.SEP.join(("purgetags","")))
    elif command=="fixture-selection-tag":
        from ingetrazo_render import context_menu_build, context_menu_run
        from core.group import Group
        group=Group(name="Loose tag target")
        scene.groups.append(group)
        scene.select([group])
        menu_state=context_menu_build(self.window,self.view)
        context_menu_run(self.window,self.view,menu_state.kind,"selectiontag",menu_state.sel)
    elif command=="fixture-rename":
        from core.group import Group
        if not hasattr(self,"fixture_rename"):
            self.fixture_rename=[Group(name="Panel A"), Group(name="Panel B")]
            scene.groups.extend(self.fixture_rename)
        scene.select(self.fixture_rename)
        order=[[g.uid,False] for g in self.fixture_rename]
        wgpu_ui._tree_action(self.window,self.view,"batchrename"+wgpu_ui.SEP+json.dumps([id(scene),True,"",order]))
    elif command=="fixture-merge-tag":
        from core.layers import Layer
        if scene.layer_by_key("Merge me") is None:
            scene.layers.append(Layer("Merge me"))
            scene.groups[0].layer="Merge me"
            scene.version+=1
        wgpu_ui._tree_action(self.window,self.view,wgpu_ui.SEP.join(("mergetag","preview-tag:Merge me")))
    elif command=="fixture-finish-merge":
        from ingetrazo_render import chrome_forms
        form=chrome_forms.current(self.window)
        options=form.field("item").keys()
        form.receive("item",str(next(i for i, label in enumerate(options) if label.endswith("Paths"))))
        form.press("ok")
    elif command=="fixture-delete-tag":
        from core.layers import Layer
        if scene.layer_by_key("Disposable") is None:
            scene.layers.append(Layer("Disposable"))
            scene.groups[0].layer="Disposable"
            scene.version+=1
        wgpu_ui._tree_action(self.window,self.view,wgpu_ui.SEP.join(("deletetag","preview-tag:Disposable")))
    elif command=="fixture-confirm-delete":
        wgpu_ui._run(self.window,["sheet:delete"])
    elif command=="fixture-scenes":
        wgpu_ui._run(self.window,["tab:scene:add","tab:scenefolders:"+json.dumps(["add",str(id(scene)),"","","Field views"])])
        saved=self.window._scene_tabs.marked_view()
        wgpu_ui._run(self.window,["tab:scenefolders:"+json.dumps(["file",str(id(scene)),"","Field views",saved.name])])
    elif command=="fixture-preview":
        from ingetrazo_render import render_scene_photo
        cam=self.viewport.camera
        cam.shift_y=0.18
        row=self.window._scene_tabs.marked_view()
        pic=render_scene_photo(self.window,json.dumps([str(id(scene)),str(scene.content_version),row.name]))
        self.fixture_photo=bool(pic and len(pic[2])==pic[0]*pic[1]*4 and max(pic[2])>min(pic[2]))
        self.fixture_restored=(cam.shift_y==0.18)
        wgpu_ui._paint_with(self.window,"ral:RAL 1000")
        from ingetrazo_render import wgpu_library
        from tools.paint import PaintTool
        wgpu_library.edit_field(self.window,self.view,"brush_name:"+PaintTool.current_material.name,"Site paint")
        self.fixture_swatch=(PaintTool.current_material.name=="Site paint" and "Site paint" not in scene.materials)
    elif command=="fixture-paint-rows":
        from ingetrazo_render import chrome_sheet
        group=scene.groups[0]
        wgpu_ui._tree_action(self.window,self.view,wgpu_ui.SEP.join(("paintrows","i:"+group.uid)))
    elif command=="fixture-apply-paint":
        from ingetrazo_render import chrome_sheet
        st=self.window._chrome_sheet
        self.fixture_paint_sheet=st.built.id
        chrome_sheet.action(self.window,chrome_sheet.SEP.join(("tree-material","open","ral:RAL 1000")))
        self.fixture_row_paint=scene.groups[0].material["mat"]
    self.request_frame()
def facts(self):
    scene=self.viewport.scene
    from ingetrazo_render.chrome_dialogs import current_sheet
    from ingetrazo_render import chrome_forms
    choice=chrome_forms.current(self.window)
    question=current_sheet(self.window)
    paths=scene.layer("Paths")
    row=self.window._scene_tabs.marked_view()
    return dict(state(self), rename_names=[g.name for g in getattr(self,"fixture_rename",[])],
        choice=choice.fields[0].value if choice and choice.fields else None, group_tag=scene.groups[0].layer,
        photo=getattr(self,"fixture_photo",False), restored=getattr(self,"fixture_restored",False),
        hatch_prefs=getattr(self,"fixture_hatch_prefs",False),
        paint_sheet=getattr(self,"fixture_paint_sheet",None), row_paint=getattr(self,"fixture_row_paint",None),
        swatch=getattr(self,"fixture_swatch",False), editor=self.renderer.ui_input_state()["drawing_line"],
        line=getattr(paths,"linetype",None), hatch=getattr(paths,"hatch",None), mortar=getattr(paths,"mortar",None), size=scene.bricks.get("brick"),
        created_tag=scene.layer_by_key(getattr(self,"fixture_tag","")) is not None,
        tag_in_folder=getattr(self,"fixture_filed_tag",False), tag_colours=scene.display_style.color_by_layer,
        model_lines=scene.display_style.model_linetypes,
        disposable=scene.layer_by_key("Disposable") is not None, question=question.text if question else "",
        mixed_folder=getattr(self,"fixture_mixed_folder",False),folder=scene.layer_folder_names.get(paths.folder,paths.folder) if paths else None,scene_folder=getattr(row,"folder",None))
Host.menu,Host.state=dispatch,facts
""")
    steps = tmp_path / "commands.txt"
    steps.write_text("""wait 850
key escape
menu fixture-open
wait 300
state
menu fixture-line
wait 250
state
menu fixture-hatch
menu fixture-folder
menu fixture-scenes
wait 350
menu fixture-preview
wait 500
state
menu fixture-paint-rows
wait 450
menu fixture-apply-paint
wait 350
state
menu fixture-delete-tag
wait 250
state
key escape
wait 150
state
menu fixture-delete-tag
wait 200
menu fixture-confirm-delete
wait 250
state
menu fixture-tag-controls
wait 200
state
menu fixture-confirm-delete
wait 200
state
menu fixture-selection-tag
wait 200
state
key escape
wait 150
state
menu fixture-merge-tag
wait 200
state
menu fixture-finish-merge
wait 200
state
menu fixture-rename
wait 250
state
key escape
wait 150
state
menu fixture-rename
wait 250
key enter
wait 200
state
quit
""")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "1280x800", "--script", str(steps), str(model)],
                 env=dict(os.environ, BUILT_ANSWERS="cmd-d"), capture_output=True, text=True, timeout=60)
    log = result.stdout + result.stderr
    assert result.returncode == 0 and "Traceback" not in log, log
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    assert states[3]["paint_sheet"] == "tree-material" and states[3]["row_paint"].startswith("RAL 1000"), log
    asked, cancelled, deleted = states[4:7]
    assert "Disposable" in asked["question"] and asked["disposable"], log
    assert not cancelled["question"] and cancelled["disposable"], log
    assert not deleted["question"] and not deleted["disposable"], log
    proposed, purged = states[7:9]
    assert proposed["tag_colours"] and not proposed["model_lines"], log
    assert "Paths" in [tag["name"] for tag in json.loads((tmp_path / "drawing.json").read_text())["layers"]]
    assert proposed["created_tag"] and proposed["tag_in_folder"] and "Unused survey tag" in proposed["question"], log
    assert not purged["created_tag"] and not purged["question"], log
    selection, cancelled, merge, moved = states[9:13]
    assert selection["choice"] and selection["choice"].startswith("Assign to ") and cancelled["choice"] is None, log
    assert merge["choice"] and merge["group_tag"] == "Merge me", log
    assert moved["choice"] is None and moved["group_tag"] == "Paths", log
    opened_rename, cancelled_rename, applied_rename = states[13:16]
    assert opened_rename["form_open"] and not cancelled_rename["form_open"], log
    assert cancelled_rename["rename_names"] == ["Panel A", "Panel B"], log
    assert not applied_rename["form_open"] and applied_rename["rename_names"] == ["Panel01", "Panel02"], log
    opened, saved, edited = states[:3]
    assert edited["hatch_prefs"] == {"pinned": ["brick", "glass"], "recent": ["brick", "glass"],
                                     "saved": {"brick": [[300, 150, 80, 12, 8]]}, "removed": {}}
    assert opened["editor"] and not saved["editor"] and saved["line"] == "lin:Boundary", log
    assert edited["hatch"] == "brick" and edited["mortar"] and edited["size"] == [250, 115, 60, 8], log
    assert edited["mixed_folder"], log
    assert edited["folder"] == "Survey" and edited["scene_folder"] == "Field views", log
    assert edited["photo"] and edited["restored"] and edited["swatch"], log
    assert output.is_file() and "*" in output.read_text(), log
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), log


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires the native window")
@pytest.mark.parametrize("suffix", ["obj", "stl"])
def test_native_mesh_units_and_model_menu(tmp_path, suffix):
    from tests.test_import_failure_reported import _CUBE_OBJ, _GOOD_STL

    path = tmp_path / ("triangle." + suffix)
    path.write_bytes(_CUBE_OBJ if suffix == "obj" else _GOOD_STL)
    steps = tmp_path / "steps"
    steps.write_text(
        f"wait 600\nmenu open_path:{path}\nwait 300\nstate\nkey escape\nwait 200\nstate\n"
        f"menu open_path:{path}\nwait 250\nkey enter\nwait 700\nstate\n"
        "menu scene_add\nmenu scene_duplicate\nmenu export:obj\nwait 250\nstate\n"
        "key escape\nwait 120\nkey escape\nwait 120\nkey escape\nwait 300\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(ROOT / ".venv"), "--new-instance",
                  "--size", "960x700", "--script", str(steps),
                  str(ROOT / "tests/fixtures/shell/slab-and-cube.igz")],
                 env=dict(os.environ, BUILT_ANSWERS="discard"), capture_output=True,
                 text=True, timeout=60)
    states = [json.loads(line.removeprefix("[built] state ")) for line in result.stdout.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and len(states) >= 5, result.stderr
    opened, cancelled, accepted, exporting, closed = states[:5]
    assert opened["form_open"] and not cancelled["form_open"]
    assert opened["documents"] == cancelled["documents"] == ["slab-and-cube.igz"]
    assert not accepted["form_open"] and len(accepted["documents"]) == 2
    assert accepted["groups"] or accepted["faces"]
    assert exporting["large_sheet"] == "export" and not closed["large_sheet"]
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), result.stderr


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires the native window")
def test_native_animation_forms_and_video_file(tmp_path):
    from core.camera import OrbitCamera
    from core.saved_views import SavedView
    from core.scene import Scene
    from ingetrazo_core import SceneAnimation
    from formats.igz import save_scene

    from ingetrazo_core import video_format
    suffix, _ = video_format()
    path, movie = tmp_path / "tour.built", tmp_path / ("tour" + suffix)
    scene, camera = Scene(), OrbitCamera()
    for name in ("top", "front"):
        camera.set_view(name)
        scene.saved_views.append(SavedView.capture(name, scene, camera))
    scene.animation = SceneAnimation(transition=0, pause=0)
    save_scene(scene, path)
    venv = fixture_venv(tmp_path, "from ingetrazo_render.shell_files import ShellFileDialog\n"
                       f"ShellFileDialog.getSaveFileName = lambda *a, **k: ({str(movie)!r}, '')\n")
    script = tmp_path / "steps"
    script.write_text("wait 700\nmenu animation_settings\nwait 300\nstate\nkey enter\nwait 200\n"
                      "menu scene_play\nwait 300\nkey escape\nmenu export:video\nwait 300\nstate\n"
                      "key enter\nwait 1300\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "900x700", "--script", str(script), str(path)],
                 capture_output=True, text=True, timeout=60)
    states = [json.loads(x.removeprefix("[built] state ")) for x in result.stdout.splitlines()
              if x.startswith("[built] state ")]
    assert result.returncode == 0 and len(states) >= 3, result.stderr
    assert [s["form_open"] for s in states[:3]] == [True, True, False], states
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), result.stderr
    movie_bytes = movie.read_bytes()
    if suffix == ".mp4":
        assert movie_bytes[4:8] == b"ftyp" and b"avc1" in movie_bytes
    else:
        assert movie_bytes.startswith(b"\x1a\x45\xdf\xa3")


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_orthomosaic_import_and_undo(tmp_path):
    from tests.geotiff_fixture import write_geotiff
    from core.scene import Scene
    from formats.igz import save_scene
    import numpy as np

    model, tile = tmp_path / "empty.igz", tmp_path / "site.tif"
    save_scene(Scene(), model)
    write_geotiff(tile, np.full((8, 16, 4), 255, dtype=np.uint8))
    venv = fixture_venv(tmp_path,
        "from ingetrazo_render.shell_files import ShellFileDialog\n"
        "from georef.tile_fetcher import TileFetcher\n"
        "TileFetcher.request = lambda *args: None\n"
        f"ShellFileDialog.getOpenFileName = lambda *a, **k: ({str(tile)!r}, '')\n")
    steps = tmp_path / "actions"
    steps.write_text("wait 700\nmenu import:orthomosaic\nwait 400\nstate\n"
                     "menu undo\nwait 400\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "900x700", "--script", str(steps), str(model)],
                 capture_output=True, text=True, timeout=60)
    states = [json.loads(line.removeprefix("[built] state ")) for line in result.stdout.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and len(states) >= 2, result.stderr
    assert states[0]["dirty"] and all(not st["errors"] and not st["blocked_widgets"] for st in states)


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires the native window")
def test_native_survey_imports_share_datum_and_save(tmp_path):
    from core.scene import Scene
    from formats.igz import load_into, save_scene
    from tests.test_photomesh import _write_odm_tree

    path = tmp_path / "site.built"
    save_scene(Scene(), path)
    csv = tmp_path / "control.csv"
    csv.write_text("1,8278698,717623,1750,origin\n")
    alignment = tmp_path / "line.geojson"
    alignment.write_text(json.dumps(dict(type="LineString", coordinates=[[-73., -15.], [-73.001, -15.]])))
    obj = _write_odm_tree(tmp_path)
    venv = fixture_venv(tmp_path,
        "from ingetrazo_render.shell_files import ShellFileDialog\n"
        "from georef.tile_fetcher import TileFetcher\n"
        "TileFetcher.request = lambda *a, **k: None\n"
        f"paths = {{'*.csv': {str(csv)!r}, '*.obj': {str(obj)!r}, '*.kml': {str(alignment)!r}}}\n"
        "ShellFileDialog.getOpenFileName = lambda *a: (next(path for ext, path in paths.items() if ext in a[3]), '')\n")
    script = tmp_path / "survey-steps"
    script.write_text("wait 800\nmenu import:survey_points\nwait 300\nstate\nkey escape\nwait 300\nstate\n"
                      "menu import:survey_points\nwait 300\nkey enter\nwait 300\nstate\n"
                      "menu import:alignment\nwait 300\nmenu import:photomesh\nwait 700\nstate\n"
                      "menu save\nwait 900\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "900x700", "--script", str(script), str(path)],
                 capture_output=True, text=True, timeout=60)
    states = [json.loads(x.removeprefix("[built] state ")) for x in result.stdout.splitlines()
              if x.startswith("[built] state ")]
    assert result.returncode == 0 and len(states) >= 4, result.stderr
    assert [s["form_open"] for s in states[:4]] == [True, False, False, False]
    assert all(not s["errors"] and not s["blocked_widgets"] for s in states), result.stderr
    scene = Scene()
    load_into(scene, path)
    assert len(scene.geo_paths) == len(scene.geo_points) == 1
    assert scene.photo_mesh.triangle_count == 2 and scene.georef.alt == pytest.approx(1750.)


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                    reason="requires a built window and BUILT_WINDOW_TESTS=1")
def test_native_takeoff_picks_tagged_faces_in_their_edit_context(tmp_path):
    from core.bim import tag_faces
    from core.history import History
    from core.scene import Scene
    from formats.igz import save_scene
    from tests.test_bim_grouped_tags import _box, _group

    scene = Scene()
    faces = _box(scene)
    tag_faces(faces, "IfcWall", "West wall", 1)
    _group(scene, History(scene), faces).name = "Wall assembly"
    model = tmp_path / "wall.built"
    save_scene(scene, model)
    venv = fixture_venv(tmp_path, """from ingetrazo_render import Host
from views import wgpu_ui
menu, state = Host.menu, Host.state
def drive(self, command):
    scene = self.viewport.scene
    if command == "fixture-measure":
        wgpu_ui._tree_action(self.window, self.view, "insp:measure")
    elif command == "fixture-select-faces":
        group, = scene.selection
        (key, label), = scene._inspector_takeoff.face_actions(group)
        assert label == "Select West wall faces"
        wgpu_ui._tree_action(self.window, self.view, "insp:" + key)
    else:
        return menu(self, command)
def inspect(self):
    scene = self.viewport.scene
    return dict(state(self), context=scene.edit_group.name if scene.edit_group else "",
                tagged_faces=sum((getattr(f, "attrs", {}).get("ifc") or {}).get("name") == "West wall"
                                 for f in scene.selection))
Host.menu, Host.state = drive, inspect
""")
    steps = tmp_path / "steps"
    steps.write_text("wait 600\nkey escape\nmenu select_all\nmenu fixture-measure\nwait 250\n"
                     "menu fixture-select-faces\nwait 250\nstate\nkey escape\nkey escape\nwait 250\nstate\nquit\n")
    result = run([str(BIN), "--root", str(ROOT), "--venv", str(venv), "--new-instance",
                  "--size", "1000x720", "--script", str(steps), str(model)],
                 env=dict(os.environ, BUILT_ANSWERS="cmd-d"), capture_output=True, text=True, timeout=45)
    log = result.stdout + result.stderr
    assert result.returncode == 0 and "Traceback" not in log, log
    states = [json.loads(line.removeprefix("[built] state ")) for line in log.splitlines()
              if line.startswith("[built] state ")]
    selected, closed = states[:2]
    assert (selected["context"], selected["tagged_faces"]) == ("Wall assembly", 6), log
    assert closed["context"] == "" and not closed["tagged_faces"], log
    assert all(not s["errors"] and not s["blocked_widgets"] for s in (selected, closed)), log

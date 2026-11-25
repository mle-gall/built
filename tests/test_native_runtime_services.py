"""Exercise first-party library boundaries in the embedded interpreter."""
import json
import os
from pathlib import Path

import pytest

from scripts.isolated_launch import run


ROOT = Path(__file__).resolve().parents[1]


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1", reason="requires a native window")
def test_document_services_run_without_qt(tmp_path):
    plugins = tmp_path / "profile/data/built/plugins"
    plugins.mkdir(parents=True)
    probe = plugins / "service_probe.py"
    probe.write_text('''from pathlib import Path

def setup(app):
    def exercise():
        import threading
        import numpy as np
        from built_core.protocol.commands import add_image_plane
        from built_render import background_autosave
        from built_core.scale_figure import store_picture
        from built_core.text3d import build_text_letters
        from built_core.paths import cache_directory
        from built_core.texture import texture_cache_root
        from georef import transport
        from georef.tiles import PRESETS, TileCache
        from georef.tile_fetcher import TileFetcher
        from built_render import SurveyRaster, notice_text
        from built_render import sheet_workspace
        from built_render import about_review
        from views.ndof_input import shared_input
        from built_render import ask_scale

        folder = Path(__file__).parent
        assert cache_directory().is_relative_to(folder.parents[2])
        assert texture_cache_root().is_relative_to(folder.parents[2])
        app.window._on_select_all()
        assert app.viewport.copy_selection()
        assert app.window._clipboard_available()
        app.window._on_select_none()
        view = app.window._host.view
        view._pick_frame_dirty = True
        view.hover(view.width() // 2, view.height() // 2)
        assert sheet_workspace._cajetin_template()[0]
        sheet_workspace.to_sheets(app.window)
        assert app.window._composer.isVisible()
        sheet_workspace.to_model(app.window)
        assert not app.window._composer.isVisible()
        assert about_review("smoke", host=app.window._host).sheet(str)
        assert notice_text("<b>A</b><br>B") == "A\\nB"
        shared_input(app.window)
        manager = background_autosave(app.window)
        manager.request([])
        manager.cancel()
        assert len(build_text_letters("AV")) == 2
        labels = app.viewport._model_line_words(
            np.array([[(0., 0., 0.), (5., 0., 0.)]]), ("GAS", 2., 5., 12.))
        assert len(labels) and np.isfinite(labels).all()
        app.viewport.camera.frame_aspect = 1.5
        image = app.viewport.render_raster(300, crop_to_frame=True)
        assert (image.width, image.height) == (300, 200)
        app.window._save_export_image(app.viewport, folder / "export.png", 300, False)
        assert SurveyRaster.open(folder / "export.png").height == 200

        tile = SurveyRaster(2, 1, bytes([200, 50, 10, 255, 0, 0, 0, 0])).png()
        sample = folder / "figure.png"
        sample.write_bytes(tile)
        assert store_picture(sample).is_file()
        add_image_plane(app.viewport.history, str(sample), [0, 0, 0], [1, 0, 0], [0, 1, 0])
        assert app.viewport.render_raster(300, crop_to_frame=True).height == 200
        # Use an offline transport; real workers, window timers, decoding,
        # cache publication and UI-thread callbacks still run.
        main = threading.get_ident()
        transport._download = lambda url, agent: tile
        fetcher = TileFetcher(TileCache(folder / "tiles"), app.window)
        def ready(source, x, y, z, raster):
            assert threading.get_ident() == main
            assert raster.rgba() == SurveyRaster.decode(tile).rgba()
            assert fetcher.request(PRESETS["osm"], x, y, z).rgba() == raster.rgba()
            (folder / "finished").touch()
        fetcher.tileReady.connect(ready)
        assert fetcher.request(PRESETS["osm"], 0, 0, 0) is None
    app.add_menu_action("Exercise document services", exercise)
''')
    capture = tmp_path / "model.png"
    script = tmp_path / "services.script"
    script.write_text(f"wait 350\nstate\nmenu extension:service_probe:0\nwait 800\n"
                      "capture model\nstate\nquit\n")
    binary = os.environ.get("BUILT_BINARY", ROOT / "rust/built_app/target/release/built")
    result = run([binary, "--new-instance", "--root", ROOT, "--venv", ROOT / ".venv",
                  "--shots", tmp_path, "--script", script, ROOT / "tests/fixtures/shell/slab-and-cube.igz"],
                 settings=tmp_path / "profile", env=dict(os.environ, BUILT_PASTEBOARD=tmp_path.name),
                 capture_output=True, text=True, timeout=90)
    states = [json.loads(line[14:]) for line in result.stdout.splitlines()
              if line.startswith("[built] state ")]
    assert result.returncode == 0 and len(states) >= 2, result.stderr
    assert (plugins / "finished").is_file(), result.stderr
    assert all(s["errors"] == 0 and s["qt_modules"] == [] for s in states), result.stderr
    assert capture.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")

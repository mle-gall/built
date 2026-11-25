# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The extension API beyond tools — ``setup(app)`` (views/extension_api.py).

Marco, 25-09, on José Castro's Levels: «hacer muchas funciones que cada
usuario me pida cargaría mucho al sistema… ¿no sería bueno hacerlo como una
extensión?». So a plugin can now keep data in the document, add a side
panel, draw over the viewport and offer an inference — and Levels is the
worked example, living in examples/extensions/, not in the core."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from pathlib import Path

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core import Vec3 as V

from core.history import History, SetPluginDataCommand
from core.scene import Scene

EXAMPLES = Path(__file__).resolve().parents[1] / "examples" / "extensions"


def test_extension_data_travels_in_the_igz_and_new_clears_it(tmp_path):
    from formats import igz
    scene = Scene()
    scene.plugin_data["niveles"] = {"levels": [{"name": "PA", "z": 2.6}]}
    scene.plugin_data["roto"] = {1, 2}          # not JSON: dropped, not fatal
    path = tmp_path / "m.igz"
    igz.save_scene(scene, path)
    other = Scene()
    igz.load_into(other, path)
    assert other.plugin_data == {"niveles": {"levels": [{"name": "PA",
                                                          "z": 2.6}]}}
    other.clear()
    assert other.plugin_data == {}


def test_setting_extension_data_is_one_undo_step():
    scene = Scene()
    hist = History(scene)
    hist.execute(SetPluginDataCommand("niveles", {"guides": True}))
    hist.execute(SetPluginDataCommand("niveles", {"guides": False}))
    assert scene.plugin_data["niveles"] == {"guides": False}
    hist.undo()
    assert scene.plugin_data["niveles"] == {"guides": True}
    hist.undo()
    assert "niveles" not in scene.plugin_data
    hist.redo()
    assert scene.plugin_data["niveles"] == {"guides": True}


def test_a_setup_only_plugin_is_discovered(tmp_path):
    from core.extensions import discover_plugins
    (tmp_path / "solo_panel.py").write_text(
        "def setup(app):\n    app.called = True\n")
    plugins, errors = discover_plugins([tmp_path])
    assert not errors
    assert [p.stem for p in plugins] == ["solo_panel"]
    assert plugins[0].tools == [] and callable(plugins[0].setup)


@pytest.fixture
def levels(monkeypatch, request):
    from core import extensions
    monkeypatch.setattr(extensions, "plugin_dirs", lambda: [EXAMPLES])
    return request.getfixturevalue("native_host").window


def _panel(win):
    return win._panel_factories["niveles"](_app(win), win.viewport.update)


def _app(win):
    from views.extension_api import ExtensionApp
    return ExtensionApp(win, "niveles")


def test_levels_adds_its_tab_and_keeps_levels_in_the_document(levels):
    win = levels
    panel = _panel(win)
    assert panel.slot == "dock"
    app = _app(win)
    app.set_document_data({"levels": [{"name": "PB", "z": 0.0},
                                      {"name": "PA", "z": 2.6}],
                           "guides": True})
    def rows():
        return next(c["rows"] for c in panel.sections()[0]["items"] if c["kind"] == "list")
    assert [row["cells"][0] for row in rows()] == ["PA", "PB"]
    assert win.viewport.history.undo()
    assert rows() == []


def _front_parallel(vp):
    vp.camera.set_view("front")
    vp.camera.perspective = False
    vp.camera.target = V(0, 0, 1.5)
    vp.camera.distance = 20.0
    vp.window()._host.resize(800, 600, 1.0)
    vp.camera.set_aspect(800, 600)


def test_levels_snap_in_a_parallel_elevation_and_say_which(levels):
    from core.snap import SnapResult
    win = levels
    vp = win.viewport
    _app(win).set_document_data({"levels": [{"name": "PA", "z": 2.6}],
                                 "guides": True})
    _front_parallel(vp)
    near = vp._world_to_pixel(V(1.0, 0.0, 2.6))
    free = SnapResult(V(1.0, 0.0, 2.58), "on_face")
    got = vp._extension_snap(free, near[0], near[1])
    assert got is not free and got.label == "PA"
    assert got.point.z() == pytest.approx(2.6)
    # a named point is never taken away
    corner = SnapResult(V(1.0, 0.0, 2.58), "endpoint")
    assert vp._extension_snap(corner, near[0], near[1]) is corner
    # far from the level: the engine's answer stands
    far = SnapResult(V(1.0, 0.0, 0.5), "on_face")
    assert vp._extension_snap(far, *vp._world_to_pixel(far.point)) is far
    # in perspective no level snaps (heights do not read)
    vp.camera.perspective = True
    assert vp._extension_snap(free, near[0], near[1]) is free


def test_levels_guides_turn_off_with_their_checkbox(levels):
    from core.snap import SnapResult
    win = levels
    vp = win.viewport
    _app(win).set_document_data({"levels": [{"name": "PA", "z": 2.6}],
                                 "guides": True})
    _panel(win).event("guides", "commit", False)
    _front_parallel(vp)
    free = SnapResult(V(1.0, 0.0, 2.58), "on_face")
    near = vp._world_to_pixel(V(1.0, 0.0, 2.6))
    assert vp._extension_snap(free, near[0], near[1]) is free


def test_a_raising_provider_cannot_take_the_cursor(native_host):
    from core.snap import SnapResult
    vp = native_host.window.viewport

    def broken(*_a):
        raise ValueError("plugin bug")
    _app(native_host.window).add_snap_provider(broken)
    s = SnapResult(V(0, 0, 0), "on_face")
    assert vp._extension_snap(s, 10, 10) is s


def test_a_provider_is_handed_the_viewport_not_the_engine(native_host):
    """The provider signature stayed ``fn(viewport, snap, px, py)`` across
    the extraction (the rule now lives in ``InferenceEngine``): a plugin
    never learns that there is an engine."""
    from core.snap import SnapResult
    vp = native_host.window.viewport
    seen = []

    def spy(viewport, snap, x, y):
        seen.append((viewport, snap, x, y))
        return None

    _app(native_host.window).add_snap_provider(spy)
    s = SnapResult(V(0, 0, 0), "on_face")
    assert vp._extension_snap(s, 10, 10) is s
    assert seen and seen[0][0] is vp and seen[0][2:] == (10, 10)


def test_levels_panel_has_a_one_click_way_to_where_the_guides_show(levels):
    """«Hice niveles pero no veo nada en el dibujo» — the camera was in
    perspective. The panel's button goes to a parallel front elevation."""
    win = levels
    vp = win.viewport
    vp.camera.perspective = True
    _panel(win).event("elevation", "invoke", None)
    assert not vp.camera.perspective
    assert abs(vp.camera.pitch) < 1e-6


def test_the_example_extensions_ship_but_install_only_on_request(tmp_path,
                                                                 monkeypatch):
    """Marco: «sería bueno tener esa extensión como ejemplo en Built para
    que otros usuarios vean» — shipped with the app and loaded only once
    someone installs it. Since 0.5.8 they install from Manage extensions…
    (the Extensions menu no longer has a submenu of its own for them), as
    managed extensions: disable and uninstall work on them like on any."""
    from built_core import extension_manager as manager
    from core import extensions
    examples = manager.example_extension_files()
    assert any(p.name == "niveles.py" and title == "Niveles"
               for p, title, _blurb in examples)
    user = tmp_path / "plugins"
    monkeypatch.setattr(extensions, "user_plugins_dir", lambda: user)
    monkeypatch.setattr(extensions, "plugin_dirs", lambda: [user])
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: user)
    assert not extensions.discover_plugins([user])[0]
    niveles = next(e for e in manager.bundled_extensions()
                   if e.id == "niveles")
    manager.install_extension(niveles)             # from the local copy
    assert (user / "niveles" / "__init__.py").read_bytes() == \
        (EXAMPLES / "niveles.py").read_bytes()
    assert manager.installed_info("niveles")["sha256"] == niveles.sha256
    manager.set_extension_enabled("niveles", False)
    assert not manager.is_extension_enabled("niveles")
    manager.uninstall_extension("niveles")
    assert not (user / "niveles").exists()


def test_an_example_extension_can_be_a_package(tmp_path, monkeypatch):
    """A bigger example extension (CAM, PR #132) is a folder with an
    ``__init__.py``: it is listed from its package docstring, installed as
    the whole folder (no ``__pycache__``) and removed as a whole."""
    from built_core import extension_manager as manager
    from built_core import paths
    root = tmp_path / "app"
    pkg = root / "examples" / "extensions" / "demo"
    (pkg / "engine").mkdir(parents=True)
    (pkg / "__init__.py").write_text(
        '"""Demo — a packaged example\n\nTwo files and a subfolder."""\n'
        "def setup(app):\n    pass\n", encoding="utf-8")
    (pkg / "engine" / "core.py").write_text("X = 1\n", encoding="utf-8")
    (pkg / "__pycache__").mkdir()
    (pkg / "__pycache__" / "junk.pyc").write_bytes(b"x")
    monkeypatch.setattr(paths, "app_root", lambda: root)
    user = tmp_path / "plugins"
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: user)
    assert [(p.name, t, b) for p, t, b in manager.example_extension_files()] \
        == [("demo", "Demo", "Two files and a subfolder.")]
    demo, = manager.bundled_extensions()
    assert demo.bundled and demo.name == {"en": "Demo"}
    manager.install_extension(demo)
    assert (user / "demo" / "engine" / "core.py").read_text() == "X = 1\n"
    assert not (user / "demo" / "__pycache__").exists()
    assert manager.installed_info("demo")["sha256"] == demo.sha256
    manager.uninstall_extension("demo")
    assert not (user / "demo").exists()

"""Extension callbacks, shortcuts and checked state follow the owning window."""
import json
import os
from pathlib import Path

import pytest

from scripts.isolated_launch import run
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_make_component_shows_once_with_loose_geometry_and_a_group(native_host):
    from core.group import Group
    from core.mesh import Mesh
    from ingetrazo_core import Vec3
    from ingetrazo_render import context_menu_build

    win = native_host.window
    scene = win.viewport.scene
    group = Group(Mesh(), name="G")
    scene.groups.append(group)
    face = scene.mesh.add_face([Vec3(0, 0, 0), Vec3(2, 0, 0), Vec3(1, 1, 0)])
    scene.select([group, face])
    menu = context_menu_build(win, win.view)
    assert menu.kind == "mixed" and menu.ids().count("makecomp") == 1


def test_multiple_plugins_keep_their_declared_order_and_paths(native_host):
    from views.extension_api import ExtensionApp

    win = native_host.window
    calls = []
    display = ExtensionApp(win, "display_thing").add_menu("Display")
    for name in ("Bigger", "Smaller", "Reset"):
        display.addAction(name, lambda checked=False, name=name: calls.append(name))
    ExtensionApp(win, "solo_tool").add_menu_action("Solo…", lambda: calls.append("Solo"))
    menus = win._extension_menus
    rows = [row for row in menus.rows() if row[2] in ("Bigger", "Smaller", "Reset", "Solo…")]
    assert [(row[1], row[2]) for row in rows] == [
        (["Display"], "Bigger"), (["Display"], "Smaller"), (["Display"], "Reset"), ([], "Solo…")]
    for command, *_ in rows:
        assert menus.trigger(command)
    assert calls == ["Bigger", "Smaller", "Reset", "Solo"]


def test_nested_actions_checked_state_and_ancestor_visibility():
    from ingetrazo_render import ExtensionMenus
    menus = ExtensionMenus()
    menu = menus.root("wood").addMenu("Wood")
    sub = menu.addMenu("Edges")
    called = []
    flag = sub.addAction("Highlight", lambda checked: called.append(("run", checked)))
    flag.setCheckable(True)
    flag.toggled.connect(lambda checked: called.append(("change", checked)))
    command, path, label, keys, mark, enabled = menus.rows()[0]
    assert (path, label, keys, mark, enabled) == (["Wood", "Edges"], "Highlight", "", 0, True)
    assert menus.trigger(command)
    assert called == [("change", True), ("run", True)]
    sub.menuAction().setEnabled(False)
    assert menus.rows()[0][-2:] == (1, False)
    menus.trigger(command)
    assert len(called) == 2
    sub.menuAction().setVisible(False)
    assert menus.rows() == []
    sub.menuAction().setVisible(True)
    sub.removeAction(flag)
    flag.setVisible(True)
    assert menus.rows() == []


def test_raw_plugin_action_tracks_live_changes():
    from PySide6.QtGui import QAction
    from ingetrazo_render import ExtensionMenus
    import shiboken6
    raw = QAction("Marks")
    raw.setCheckable(True)
    tree = ExtensionMenus()
    tree.root("raw").addAction(raw)
    command = tree.rows()[0][0]
    on = []
    raw.triggered.connect(lambda checked: on.append(checked))
    tree.trigger(command)
    assert on == [True] and tree.rows()[0][-2] == 1
    raw.setText("Highlights")
    raw.setEnabled(False)
    assert tree.rows()[0][2:] == ("Highlights", "", 1, False)
    parent = tree.root("extra")
    parent.addAction(raw)
    parent.removeAction(raw)
    assert len(tree.rows()) == 1
    shiboken6.delete(raw)
    assert not tree.rows()


def test_window_registry_isolation_and_setup_rollback(tmp_path, monkeypatch):
    from app_shell.host import Host
    from core import extensions
    from core.extensions import LoadedPlugin

    events = []
    def setup(app):
        assert app.scene is app.window.viewport.scene
        flag = app.add_menu("Tools").addAction("Marks", events.append)
        flag.setCheckable(True)
        app.add_menu_action("Other", lambda: None, "Space", tip="Run another command")
        app.window.fixture_flag = flag

    def broken(app):
        app.add_menu_action("Partial", lambda: None)
        raise ValueError("broken fixture")

    plugins = [LoadedPlugin(name, tmp_path / (name + ".py"), setup=hook)
               for name, hook in [("good", setup), ("bad", broken)]]
    monkeypatch.setattr(extensions, "discover_plugins", lambda **kw: (plugins, []))
    hosts = []
    try:
        for _ in range(2):
            hosts.append(Host(0, 1024, 700, 1., settings=str(tmp_path / "settings")))
        first, second = (host.window for host in hosts)
        menus = first._extension_menus
        assert "broken fixture" in first._extension_load_errors["bad"]["error"]
        assert [row[2] for row in menus.rows()] == ["Marks", "Other"]
        command = menus.rows()[0][0]
        action = first._registry_actions["menu:" + command]
        other = first._registry_actions["menu:" + menus.rows()[1][0]]
        assert other.portable_keys() == [] and other.statusTip() == "Run another command"
        action.setShortcuts(["Ctrl+Alt+J"])
        assert menus.rows()[0][3] == "Ctrl+Alt+J"
        assert menus.commands()[0][3] == ""
        action.trigger()
        assert events == [True]
        assert second._extension_menus.rows()[0][-2] == 0
        first.fixture_flag.setText("Show marks")
        first.fixture_flag.setEnabled(False)
        first._extension_menus.refresh(first)
        assert action.text() == "Show marks" and not action.isEnabled()
        action.trigger()
        assert events == [True]
    finally:
        for host in reversed(hosts):
            host.close()


@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1", reason="requires the native window")
def test_installed_plugin_toggles_in_a_modelling_window(tmp_path):
    root = Path(__file__).resolve().parents[1]
    binary = root / "rust/built_app/target/release/built"
    settings = tmp_path / "settings"
    plugins = settings / "data/ingetrazo/plugins"
    plugins.mkdir(parents=True)
    plugin = plugins / "menu_fixture.py"
    plugin.write_text("""from pathlib import Path
import sys
def setup(app):
    action = app.add_menu("Fixture").addAction("Toggle guides")
    action.setCheckable(True)
    app.add_menu_action("Inspect host", lambda: Path(__file__).with_suffix(".host").write_text(
        "classic" if "views.main_window" in sys.modules else "native"))
    action.triggered.connect(lambda on: Path(__file__).with_suffix(".state").open("a").write(str(on) + "\\n"))
""")
    script = tmp_path / "steps.txt"
    script.write_text("""wait 300
menu extension:menu_fixture:1
wait 120
menu extension:menu_fixture:1
tool line
click 280 300
click 340 330
key escape
wait 120
menu extension:menu_fixture:2
state
quit
""")
    result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv",
                  "--size", "1024x700", "--script", script,
                  root / "tests/fixtures/shell/slab-and-cube.igz"],
                 settings=settings, capture_output=True, text=True, timeout=60)
    assert result.returncode == 0, result.stderr
    assert plugin.with_suffix(".state").read_text().splitlines() == ["True", "False"]
    states = [json.loads(line.removeprefix("[built] state ")) for line in
              result.stdout.splitlines() if line.startswith("[built] state ")]
    assert states and all(s["errors"] == 0 and s["edges"] > 0 for s in states), result.stderr
    assert all(not s["classic_window_loaded"] for s in states)
    assert plugin.with_suffix(".host").read_text() == "native"

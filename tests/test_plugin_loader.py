# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The plugin discovery engine (core.extensions) and its Extensions menu.

Each test below pins one clause of the engine's contract — most of them are
regression tests for the failure modes the original contribution (PR #1)
shipped with: a plugin raising in its constructor took the whole app down at
startup, a plugin that merely *imported* a built-in tool re-registered it
(duplicating the menu entry and cloning its shortcut), and plugins were
imported by package name, which breaks inside a PyInstaller bundle.
"""
from __future__ import annotations

import sys
import textwrap
from pathlib import Path

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401

import core.extensions as extensions                              # noqa: E402
from core.extensions import (                                     # noqa: E402
    discover_plugins, plugin_dirs, user_plugins_dir)


def _write(dirpath: Path, name: str, source: str) -> Path:
    path = dirpath / name
    path.write_text(textwrap.dedent(source))
    return path


GOOD = """
    from tools.base import Tool

    class HelloTool(Tool):
        name = "Hello"

        def on_activate(self, viewport):
            pass

        def on_deactivate(self, viewport):
            pass
"""


# ---- Discovery ------------------------------------------------------------

def test_good_plugin_discovered(tmp_path):
    _write(tmp_path, "hello.py", GOOD)
    plugins, errors = discover_plugins([tmp_path])
    assert errors == []
    assert [t.name for p in plugins for t in p.tools] == ["Hello"]
    assert plugins[0].stem == "hello"


def test_package_plugin_discovered(tmp_path):
    pkg = tmp_path / "hola"
    pkg.mkdir()
    _write(pkg, "__init__.py", GOOD)
    plugins, errors = discover_plugins([tmp_path])
    assert errors == []
    assert len(plugins) == 1 and plugins[0].stem == "hola"


def test_import_leaves_sys_path_alone(tmp_path):
    """By-path loading must not put the plugin dir on sys.path, or a user
    file named json.py would shadow the stdlib for the whole app."""
    _write(tmp_path, "hello.py", GOOD)
    before = list(sys.path)
    discover_plugins([tmp_path])
    assert sys.path == before


def test_missing_directory_is_fine(tmp_path):
    plugins, errors = discover_plugins([tmp_path / "no-existe"])
    assert plugins == [] and errors == []


def test_default_dirs_shape():
    dirs = plugin_dirs()
    assert dirs[0].name == "plugins"
    assert user_plugins_dir().parts[-2:] == ("built", "plugins")


# ---- Failure isolation (the app must always start) ------------------------

def test_broken_import_reported_not_raised(tmp_path):
    _write(tmp_path, "roto.py", "import modulo_que_no_existe\n")
    plugins, errors = discover_plugins([tmp_path])
    assert plugins == []
    assert len(errors) == 1 and errors[0].stem == "roto"
    assert "ModuleNotFoundError" in errors[0].error


def test_broken_constructor_reported_not_raised(tmp_path):
    """The PR #1 regression: its try wrapped only the import, so a tool
    raising in __init__ crashed the whole application at startup."""
    _write(tmp_path, "explota.py", """
        from tools.base import Tool

        class Boom(Tool):
            name = "Boom"

            def __init__(self):
                raise RuntimeError("missing optional dependency")

            def on_activate(self, viewport):
                pass

            def on_deactivate(self, viewport):
                pass
    """)
    plugins, errors = discover_plugins([tmp_path])
    assert plugins == []
    assert len(errors) == 1 and "RuntimeError" in errors[0].error


def test_one_bad_plugin_does_not_sink_the_good_one(tmp_path):
    _write(tmp_path, "aaa_roto.py", "import modulo_que_no_existe\n")
    _write(tmp_path, "hello.py", GOOD)
    plugins, errors = discover_plugins([tmp_path])
    assert [p.stem for p in plugins] == ["hello"]
    assert [e.stem for e in errors] == ["aaa_roto"]


# ---- Registration filter --------------------------------------------------

def test_imported_builtin_tool_not_reregistered(tmp_path):
    """A plugin importing LineTool (to reuse or subclass it) must not
    duplicate the built-in Line entry — or clone its 'L' shortcut."""
    _write(tmp_path, "reusa.py", """
        from tools.rust_tools import LineTool
        from tools.base import Tool

        class Mine(Tool):
            name = "Mine"

            def on_activate(self, viewport):
                pass

            def on_deactivate(self, viewport):
                pass
    """)
    plugins, errors = discover_plugins([tmp_path])
    assert errors == []
    assert [t.name for p in plugins for t in p.tools] == ["Mine"]


def test_abstract_subclass_skipped(tmp_path):
    """A Tool subclass that never implements the abstract hooks is a base
    class, not a tool — instantiating it would raise TypeError."""
    _write(tmp_path, "base_extra.py", """
        from tools.base import Tool

        class StillAbstract(Tool):
            name = "Nope"
    """)
    plugins, errors = discover_plugins([tmp_path])
    assert plugins == [] and errors == []


def test_duplicate_stem_first_dir_wins(tmp_path):
    d1 = tmp_path / "app"
    d2 = tmp_path / "user"
    d1.mkdir(), d2.mkdir()
    _write(d1, "dup.py", GOOD)
    _write(d2, "dup.py", GOOD.replace('"Hello"', '"Second"'))
    plugins, _errors = discover_plugins([d1, d2])
    assert [t.name for p in plugins for t in p.tools] == ["Hello"]


# ---- The Extensions menu itself -------------------------------------------

@pytest.fixture
def open_window(tmp_path, monkeypatch, request):
    # Discover the fixture plugins only after the test has written them.
    monkeypatch.setattr(extensions, "plugin_dirs", lambda: [tmp_path])
    return lambda: request.getfixturevalue("native_host").window


def test_menu_lists_working_tools_and_reports_load_errors(tmp_path, open_window):
    _write(tmp_path, "hello.py", GOOD)
    _write(tmp_path, "roto.py", "import modulo_que_no_existe\n")
    win = open_window()
    row, = win._extension_menus.rows()
    assert row[2] == "Hello" and row[-1]
    assert "ModuleNotFoundError" in win._extension_load_errors["roto"]["error"]


def test_empty_plugins_leave_management_and_author_commands_available(
        open_window, tmp_path, monkeypatch):
    win = open_window()
    assert win._extension_menus.rows() == []
    assert {"menu:develop_plugin", "menu:extensions", "menu:plugins_folder"} <= win._registry_actions.keys()
    opened = []
    folder = tmp_path / "profile" / "plugins"
    monkeypatch.setattr(extensions, "user_plugins_dir", lambda: folder)
    monkeypatch.setattr("built_core.desktop_open.open_url", opened.append)
    win._registry_actions["menu:plugins_folder"].trigger()
    assert folder.is_dir() and opened == [folder]


def test_plugin_cannot_steal_a_builtin_shortcut(tmp_path, open_window):
    _write(tmp_path, "ladron.py", GOOD.replace(
        'name = "Hello"', 'name = "Ladron"\n        shortcut = "L"'))
    win = open_window()
    row, = win._extension_menus.rows()
    assert row[2:4] == ("Ladron", "")
    assert win._tool_actions["line"].portable_keys() == ["L"]


def test_fresh_shortcut_is_honoured(tmp_path, open_window):
    _write(tmp_path, "consola.py", GOOD.replace(
        'name = "Hello"', 'name = "Consola"\n        shortcut = "Ctrl+Alt+J"'))
    row, = open_window()._extension_menus.rows()
    assert row[2:4] == ("Consola", "Ctrl+Alt+J")


def test_activating_a_plugin_keeps_the_real_tool_active(tmp_path, open_window):
    _write(tmp_path, "hello.py", GOOD.replace(
        "def on_activate(self, viewport):\n            pass",
        "def on_activate(self, viewport):\n            viewport._plugin_ran = True"))
    win = open_window()
    active = win.viewport.active_tool
    label = win.tool_label
    row, = win._extension_menus.rows()
    win._registry_actions["menu:" + row[0]].trigger()
    assert win.viewport._plugin_ran and win.viewport.active_tool is active
    assert win.tool_label == label


SETUP_BUILT_TOOL = """
    from tools.base import Tool

    class WallTool(Tool):
        name = "Wall"

        def __init__(self, app, mode):        # the extension builds it
            super().__init__()
            self.app, self.mode = app, mode

        def on_activate(self, viewport):
            pass

        def on_deactivate(self, viewport):
            pass

    BUILT = []

    def setup(app):
        BUILT.append(WallTool(app, "draw"))
"""


def test_a_tool_that_needs_arguments_is_left_to_setup(tmp_path):
    # planArq (José Castro) builds its tools in setup(app) with the app and
    # a mode; the loader tried WallTool() itself, it raised, and the whole
    # extension — setup and panel — was thrown away.
    _write(tmp_path, "walls.py", SETUP_BUILT_TOOL)
    plugins, errors = discover_plugins([tmp_path])
    assert errors == []
    (plug,) = plugins
    assert plug.stem == "walls" and plug.setup is not None
    assert plug.tools == []                  # not made by the loader
    plug.setup(object())
    assert len(sys.modules[plug.setup.__module__].BUILT) == 1


def test_a_tool_with_optional_arguments_is_still_listed(tmp_path):
    _write(tmp_path, "opt.py", """
        from tools.base import Tool

        class OptTool(Tool):
            name = "Opt"

            def __init__(self, mode="draw"):
                super().__init__()
                self.mode = mode

            def on_activate(self, viewport):
                pass

            def on_deactivate(self, viewport):
                pass
    """)
    plugins, errors = discover_plugins([tmp_path])
    assert errors == [] and [type(t).__name__ for t in plugins[0].tools] == [
        "OptTool"]

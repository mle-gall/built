# SPDX-License-Identifier: GPL-3.0-or-later
"""Native bundled panel order and shortcut precedence."""
import sys
from types import SimpleNamespace
from unittest.mock import Mock

from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_the_python_console_comes_after_the_model_tools(native_host):
    panels = list(native_host.window._panel_factories)
    assert panels.index("model_info") < panels.index("solid_inspector") < panels.index("python_console")


def test_an_extension_cannot_take_a_bundled_tools_shortcut(native_host, monkeypatch):
    from plugins.python_console import PythonConsoleTool
    from views.extension_api import ExtensionApp

    # The host fixture has no OS menu bar. Supply the native console item at
    # the same boundary the binary uses to publish its accelerators.
    tool = PythonConsoleTool()
    console = SimpleNamespace(id="panel:python_console", name=tool.name, keys=tool.shortcut,
                              path="Plugins", isEnabled=lambda: True, set_shortcut=Mock())
    monkeypatch.setitem(sys.modules, "built_shell", SimpleNamespace(menu_commands=lambda: [console]))
    win = native_host.window
    ExtensionApp(win, "grabby").add_menu_action("Grabby", lambda: None, tool.shortcut)
    win._extension_menus.refresh(win)
    action = win._registry_actions["menu:panel:python_console"]
    assert action.portable_keys() == ["Ctrl+Shift+P"]
    assert next(row[3] for row in win._extension_menus.rows() if row[2] == "Grabby") == ""

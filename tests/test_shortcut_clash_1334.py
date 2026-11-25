# SPDX-License-Identifier: GPL-3.0-or-later
"""Plugins may publish Save commands while the model keeps its own shortcuts."""
from unittest.mock import Mock

import pytest

from built_core import Input as Qt
from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.mark.parametrize("visible", [True, False])
def test_plugin_save_actions_leave_the_model_shortcuts_available(native_host, monkeypatch, visible):
    from built_render import shell_files as files

    win = native_host.window
    root = win._extension_menus.root("graph")
    plugin_save = Mock()
    for label, key in [("Save graph", "Ctrl+S"), ("Save graph as…", "Ctrl+Shift+S")]:
        action = root.add_command(label, plugin_save, key, "")
        action.setVisible(visible)
    win._extension_menus.refresh(win)
    assert all(not row[3] for row in win._extension_menus.rows())
    calls = []
    monkeypatch.setattr(files, "save", lambda w: calls.append(("save", w)))
    monkeypatch.setattr(files, "save_as", lambda w: calls.append(("save as", w)))
    for mask in [Qt.ControlModifier, Qt.ControlModifier | Qt.ShiftModifier]:
        native_host.key(True, Qt.Key_S, mask.value)
        native_host.key(False, Qt.Key_S, mask.value)
    assert calls == [("save", win), ("save as", win)]
    plugin_save.assert_not_called()

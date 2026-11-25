# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Extensions menu's own entries all work. «Develop an extension…» lost
the guide's address when the example-extensions submenu went away and did
nothing but raise (app bench, 10-10)."""
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_develop_an_extension_opens_the_guide(native_host, monkeypatch):
    opened = []
    monkeypatch.setattr("ingetrazo_core.desktop_open.open_url", opened.append)
    action = native_host.window._registry_actions["menu:develop_plugin"]
    action.trigger()
    assert opened == ["https://github.com/ingelibre/ingetrazo/blob/main/docs/plugins.md"]

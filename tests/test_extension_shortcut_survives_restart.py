# SPDX-License-Identifier: GPL-3.0-or-later
"""A plugin's saved binding is restored after discovery in a fresh native window."""
from contextlib import contextmanager

import pytest

from core import extensions
from ingetrazo_core import settings
from ingetrazo_core import NativeSettings
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_render import keyboard_bindings as shortcuts


@pytest.mark.parametrize("factory,rebound", [(None, "Ctrl+Alt+U"), ("Ctrl+Alt+J", "Ctrl+Alt+K")])
def test_extension_keeps_its_user_and_factory_bindings_across_restart(tmp_path, monkeypatch, factory, rebound):
    from ingetrazo_render import Host
    (tmp_path / "unfold.py").write_text(
        f"def setup(app):\n    app.add_menu_action('Unfold Faces', lambda: None, {factory!r})\n")
    monkeypatch.setattr(extensions, "plugin_dirs", lambda: [tmp_path])
    profile = str(tmp_path / "profile")
    @contextmanager
    def session():
        monkeypatch.setattr(settings, "_store", NativeSettings(profile))
        host = Host(0, 900, 600, 1., settings=profile)
        try:
            row, = host.window._extension_menus.rows()
            assert row[2] == "Unfold Faces"
            yield host.window._registry_actions["menu:" + row[0]]
        finally:
            host.close()
    expected = [factory] if factory else []
    with session() as action:
        assert action.portable_keys() == expected
        shortcuts.save_shortcut(action, [rebound])
    with session() as action:
        assert action.portable_keys() == [rebound]
        assert shortcuts.default_keys(action) == expected

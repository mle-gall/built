# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The native renderer installs the bundled chrome on its first attachment."""
from unittest.mock import Mock

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import wgpu_ui


def test_startup_installs_the_chrome(native_host, monkeypatch):
    view = native_host.view
    renderer = Mock()
    monkeypatch.setattr(view, "renderer", renderer)
    wgpu_ui.attach(view)
    renderer.ui_enable.assert_called_once_with(wgpu_ui.FONT)
    assert view._ui_attached
    assert view.source_viewport._chrome_owns_labels


def test_startup_is_idempotent(native_host):
    view = native_host.view
    renderer = Mock()
    native_host.hand_renderer(view, renderer)
    timer = view._ui_timer
    wgpu_ui.attach(view)
    renderer.ui_enable.assert_not_called()
    assert view._ui_timer is timer

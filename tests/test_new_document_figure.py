# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #221: someone who models parts for a 3D printer wants each new
document to start empty, without the 1.70 m scale figure. Preferences ▸
General ▸ «Put the scale figure in new documents»."""
from __future__ import annotations

from built_core import scale_figure
from built_core.settings import settings
from tests.preferences_chrome import Controls
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_by_default_a_new_document_has_the_figure(native_host):
    native_host.cold_start()
    assert len(native_host.window.viewport.scene.groups) == 1


def test_switched_off_new_documents_start_empty(native_host):
    settings().setValue(scale_figure.KEY_ON, "0")
    native_host.cold_start()
    win = native_host.window
    assert win.viewport.scene.groups == []
    assert not win._is_dirty()
    win._on_new()
    assert win.viewport.scene.groups == []
    assert not win._is_dirty()


def test_the_preference_is_stored(native_host):
    ui = Controls(native_host.window, "general")
    assert ui.value("scale_figure")
    ui.set("scale_figure", False)
    assert settings().value(scale_figure.KEY_ON) == "0"

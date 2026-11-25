# SPDX-License-Identifier: GPL-3.0-or-later
"""The native Settings sheet exposes only supported appearance choices."""
from tests.preferences_chrome import Controls


def test_palette_names_are_absent_from_settings(native_host):
    fields = str(Controls(native_host.window, "general").wire())
    assert "Solarized" not in fields and "Campbell" not in fields

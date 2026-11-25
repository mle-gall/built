# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Old preference values cannot override the native design-system accent."""
import pytest
from ingetrazo_core import general_settings
from ingetrazo_render import chrome_token


@pytest.mark.parametrize("legacy", ["orange", "system", "light_grey"])
def test_a_legacy_accent_cannot_change_the_chrome(private_settings, legacy):
    general_settings.set_value("accent", legacy)
    assert chrome_token("ACCENT") == (106, 165, 255, 255)

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Location editor synchronizes the model origin in UTM and geographic coordinates."""
from __future__ import annotations

import pytest

from georef.datum import utm_forward, utm_inverse
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_settings_sheet import win, act, fields, frame
from views.settings_sheet import edit_location

# Plaza de Yanque, Caylloma — UTM zone 19, southern hemisphere.
LAT, LON = -15.8402, -70.0219


def open_location(win):
    edit_location(win, center=(LAT, LON))
    return fields(frame(win))


def test_lat_lon_fills_the_utm_boxes(win):
    open_location(win)
    utm = fields(act(win, "v", "site_mode", "1"))
    east, north = utm_forward(LAT, LON, 19)
    assert int(utm["site_zone"][3]) == 19 and utm["site_hemisphere"][4] == 1
    assert float(utm["site_easting"][3]) == pytest.approx(east, abs=.01)
    assert float(utm["site_northing"][3]) == pytest.approx(north, abs=.01)


def test_typing_utm_moves_lat_lon(win):
    open_location(win)
    act(win, "v", "site_mode", "1")
    east, north = utm_forward(LAT, LON, 19)
    act(win, "v", "site_easting", str(east + 100))
    shown = fields(act(win, "v", "site_mode", "0"))
    expected = utm_inverse(east + 100, north, 19, False)
    assert (float(shown["site_lat"][3]), float(shown["site_lon"][3])) == pytest.approx(expected, abs=1e-6)


def test_utm_survives_a_round_trip(win):
    open_location(win)
    act(win, "v", "site_mode", "1")
    shown = fields(act(win, "v", "site_mode", "0"))
    assert float(shown["site_lat"][3]) == pytest.approx(LAT, abs=1e-6)
    assert float(shown["site_lon"][3]) == pytest.approx(LON, abs=1e-6)


def test_geo_mode_hides_the_utm_rows(win):
    shown = open_location(win)
    assert "site_easting" not in shown and "site_lat" in shown


def test_utm_mode_hides_the_geo_rows(win):
    open_location(win)
    shown = fields(act(win, "v", "site_mode", "1"))
    assert "site_easting" in shown and "site_lat" not in shown


def test_choice_is_remembered_everywhere(win):
    from core.settings import settings
    from ingetrazo_render import chrome_sheet
    open_location(win)
    act(win, "v", "site_mode", "1")
    assert settings().value("georef/coord_mode") == "utm"
    chrome_sheet.close(win)
    shown = open_location(win)
    assert shown["site_mode"][4] == 1 and "site_zone" in shown
    assert "site_lat" not in shown


def test_center_fills_both_coordinate_rows(win):
    edit_location(win, center=(LAT, LON))
    shown = fields(frame(win))
    assert float(shown["site_lat"][3]) == pytest.approx(LAT)
    assert float(shown["site_lon"][3]) == pytest.approx(LON)
    east, north = utm_forward(LAT, LON, 19)
    shown = fields(act(win, "v", "site_mode", "1"))
    assert shown["site_zone"][3] == "19" and shown["site_hemisphere"][4] == 1
    assert float(shown["site_easting"][3]) == pytest.approx(east, abs=.01)
    assert float(shown["site_northing"][3]) == pytest.approx(north, abs=.01)


def test_typed_utm_moves_the_pin(win):
    from ingetrazo_render.chrome_sheet import current
    edit_location(win, center=(LAT, LON))
    frame(win)
    act(win, "v", "site_mode", "1")
    east, north = utm_forward(LAT, LON, 19)
    act(win, "v", "site_easting", str(east + 250))
    act(win, "v", "site_northing", str(north - 250))
    expected = utm_inverse(east + 250, north - 250, 19, False)
    assert current(win).site.position() == pytest.approx(expected, abs=1e-6)

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""TileFetcher (Track G, G1): cache-first behaviour and the zoom guard,
exercised without touching the network."""
from __future__ import annotations

import pytest

from built_render import SurveyRaster

from georef.tiles import PRESETS, TileCache
from georef.tile_fetcher import TileFetcher, default_cache_dir


@pytest.fixture(autouse=True)
def offline_tiles(monkeypatch):
    from georef import transport
    monkeypatch.setattr(transport, "_download", lambda *_: _png_bytes())


def _fetcher(tmp_path) -> TileFetcher:
    return TileFetcher(TileCache(tmp_path))


def _png_bytes() -> bytes:
    return SurveyRaster(4, 4, bytes((255, 0, 0, 255)) * 16).png()


def test_cache_hit_returns_image_synchronously(tmp_path):
    cache = TileCache(tmp_path)
    cache.put("osm", 5, 6, 7, _png_bytes())
    fetcher = TileFetcher(cache)

    got = fetcher.request(PRESETS["osm"], 5, 6, 7)
    assert got is not None
    assert got.rgba()[:4] == bytes((255, 0, 0, 255))
    assert got.width == 4 and got.height == 4


def test_zoom_out_of_range_refused(tmp_path):
    fetcher = TileFetcher(TileCache(tmp_path))
    osm = PRESETS["osm"]
    assert fetcher.request(osm, 0, 0, osm.max_zoom + 1) is None
    assert fetcher.request(osm, 0, 0, -1) is None


def test_cache_miss_returns_none_and_starts_inflight(tmp_path):
    # Completion is dispatched only when the window pumps its timers.
    fetcher = _fetcher(tmp_path)
    got = fetcher.request(PRESETS["osm"], 1, 1, 5)
    assert got is None
    assert len(fetcher._inflight) == 1
    fetcher.cancel_all()
    assert fetcher._inflight == {}


def test_duplicate_request_coalesces(tmp_path):
    fetcher = _fetcher(tmp_path)
    fetcher.request(PRESETS["osm"], 2, 2, 5)
    fetcher.request(PRESETS["osm"], 2, 2, 5)
    assert len(fetcher._inflight) == 1


def test_default_cache_dir_uses_the_native_profile_directory(tmp_path, monkeypatch):
    from pathlib import Path
    from unittest.mock import Mock
    import built_render

    profile = tmp_path / "settings"
    base = tmp_path / "local"
    resolve = Mock(return_value=base)
    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(profile))
    monkeypatch.setattr(built_render, "local_data_directory", resolve, raising=False)
    assert default_cache_dir() == base / "Built" / "Built" / "tiles"
    resolve.assert_called_once()
    assert Path(resolve.call_args.args[0]) == profile

    monkeypatch.delenv("BUILT_SETTINGS_DIR")
    assert default_cache_dir() == base / "Built" / "Built" / "tiles"
    resolve.assert_called_with(None)


def test_downloads_are_capped_and_queued(tmp_path):
    # Asking for many tiles at once must not start them all — only up to the
    # concurrency limit run; the rest queue (prevents the network flood/hang).
    fetcher = _fetcher(tmp_path)
    osm = PRESETS["osm"]
    for x in range(30):
        fetcher.request(osm, x, 0, 5)
    assert len(fetcher._inflight) == fetcher._MAX_INFLIGHT
    assert len(fetcher._queue) == 30 - fetcher._MAX_INFLIGHT
    fetcher.cancel_all()
    assert fetcher._inflight == {} and fetcher._queue == []


def test_survey_pixels_preserve_unpremultiplied_channels_and_bounds():
    image = SurveyRaster(2, 1, bytes((10, 20, 30, 0, 40, 50, 60, 128)))
    assert image.pixelColor(0, 0).getRgb() == (10, 20, 30, 0)
    sample = image.pixelColor(1, 0)
    assert (sample.red(), sample.green(), sample.blue(), sample.alpha()) == (40, 50, 60, 128)
    assert sample.name() == "#28323c" and sample.lightness() == 50
    for x, y in [(-1, 0), (2, 0), (0, 1)]:
        with pytest.raises(ValueError):
            image.pixelColor(x, y)

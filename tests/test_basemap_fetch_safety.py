# SPDX-License-Identifier: GPL-3.0-or-later
"""Regression checks for the Windows base-map crash (#327)."""
from __future__ import annotations

from concurrent.futures import Future

from georef.datum import SceneDatum
from georef.tile_fetcher import TileFetcher
from georef.tiles import PRESETS, TileCache, TileLayer
from ingetrazo_render import SurveyRaster
from tests.controller_surface import ControllerSurface as Viewport


class _Fetcher:
    def __init__(self):
        self.requests = []

    def request(self, source, x, y, z):
        self.requests.append((source.id, x, y, z))
        return None


def test_missing_tiles_leave_paint_before_starting_network(monkeypatch):
    """Two paints coalesce their missing tiles on the window timer queue."""
    viewport = Viewport()
    deliveries = []
    dispatch = viewport._request_missing_tiles

    def deliver():
        deliveries.append(None)
        dispatch()

    monkeypatch.setattr(viewport, "_request_missing_tiles", deliver)
    layer = TileLayer(PRESETS["osm"], zoom=16)
    datum = SceneDatum(-12.0464, -77.0428)
    viewport.scene.tile_layer = layer
    viewport.scene.georef = datum
    fetcher = _Fetcher()
    monkeypatch.setattr(viewport, "_ensure_tile_fetcher", lambda: fetcher)

    from types import SimpleNamespace
    from views.wgpu_references import sync
    display = SimpleNamespace(source_viewport=viewport, renderer=None,
                              packed=SimpleNamespace(origin=(0., 0., 0.)),
                              _shared=SimpleNamespace())
    sync(display, {})
    assert fetcher.requests == []
    sync(display, {})
    assert viewport._tile_requests_scheduled
    assert deliveries == []
    viewport._host.timers.run_due()
    assert len(deliveries) == 1
    assert fetcher.requests
    assert viewport._tile_requests_scheduled is False
    viewport.close()


def test_tile_download_submits_the_tile_url_to_the_worker(tmp_path, monkeypatch):
    from georef import transport

    received = Future()

    def download(url, agent):
        received.set_result(url)
        return b""

    monkeypatch.setattr(transport, "_download", download)
    viewport = Viewport()
    fetcher = TileFetcher(TileCache(tmp_path), parent=viewport)
    try:
        fetcher._start_download(PRESETS["osm"], 1, 2, 3)
        assert received.result(timeout=5) == "https://tile.openstreetmap.org/3/1/2.png"
    finally:
        fetcher.cancel_all()
        viewport.close()


def test_a_tile_on_disk_is_read_at_once_without_the_network(tmp_path):
    """A sheet frame or an export is drawn ONCE: a tile already in the
    disk cache must come back in that same paint (deferring it left the
    base map blank in the composer), and reading it must not build the
    fetcher, whose network manager is what may not start inside paintGL."""
    viewport = Viewport()
    cache = TileCache(tmp_path)
    data = SurveyRaster(4, 4, bytes((10, 120, 30, 255)) * 16).png()
    cache.put("osm", 1, 2, 3, data)
    viewport._tile_disk_cache = cache
    got = viewport._disk_cached_tile(PRESETS["osm"], 1, 2, 3)
    assert got is not None and got.rgba()[:4] == bytes((10, 120, 30, 255))
    assert viewport._disk_cached_tile(PRESETS["osm"], 9, 9, 3) is None
    assert viewport._tile_fetcher is None, "the network manager was built"
    viewport.close()


def test_the_tile_cache_location_needs_no_network_module():
    """Asking for the disk cache path must not initialize the HTTP workers."""
    import subprocess
    import sys
    subprocess.run([sys.executable, "-c", """
import sys
from georef import tiles, transport
assert tiles.default_cache_dir().name == "tiles"
assert not hasattr(transport, "_workers")
assert "urllib.request" not in sys.modules
"""], check=True, timeout=30)

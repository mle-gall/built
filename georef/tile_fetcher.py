# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Cache-first survey requests on the requesting host's event loop."""
from functools import partialmethod
from operator import attrgetter
from ingetrazo_core import (
    fetch_init, tile_request, tile_pump, tile_start, tile_finished, tile_completed, tile_cancel,
    tile_failed as _tile_failed,
)
from ingetrazo_render import SurveyRaster
from georef.transport import NATIVE, Service, Signal
if not NATIVE:
    from PySide6.QtCore import QUrl
    from PySide6.QtNetwork import QNetworkAccessManager, QNetworkReply, QNetworkRequest


from georef.tiles import TileCache, TileSource, default_cache_dir  # noqa: F401 — re-exported


# Slippy-map etiquette: identify the client and don't hammer servers. OSM's
# usage policy in particular requires a real User-Agent.
from core.version import __version__

_USER_AGENT = (f"IngeTrazo/{__version__} (https://github.com/ingelibre/ingetrazo)").encode()


class TileFetcher(Service):
    tileReady = Signal(str, int, int, int, object)
    tileFailed = Signal(str, int, int, int, str)
    _MAX_INFLIGHT = 8

    __init__ = partialmethod(fetch_init)
    cache = property(attrgetter("_cache"))
    request = partialmethod(tile_request)
    _pump = partialmethod(tile_pump)
    _start_download = partialmethod(tile_start)
    _on_finished = partialmethod(tile_finished)
    _completed = partialmethod(tile_completed)
    cancel_all = partialmethod(tile_cancel)

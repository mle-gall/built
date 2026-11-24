# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Async tile fetching (Track G, G1) — cache-first, over the network.

The Qt-net half of the tile layer: :class:`TileFetcher` wraps a
``QNetworkAccessManager``, serves cached tiles instantly and downloads the rest
in the background, emitting :attr:`TileFetcher.tileReady` when an image lands.
The pure slippy/source/cache logic lives in :mod:`georef.tiles`; this module is
the only tile piece that needs Qt-net + a running event loop.

QtNetwork transports tiles. The native raster decoder supplies their pixels.
"""
from __future__ import annotations

from pathlib import Path

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
    """Cache-first async tile provider.

    Call :meth:`request` with a source + tile index; the tile arrives via the
    :attr:`tileReady` signal (immediately for cache hits, later for downloads).
    Duplicate in-flight requests for the same tile are coalesced.
    """

    #: ``(source_id, x, y, z, image)`` — a decoded tile is available.
    tileReady = Signal(str, int, int, int, object)
    #: ``(source_id, x, y, z, reason)`` — the tile could not be fetched.
    tileFailed = Signal(str, int, int, int, str)

    #: Max simultaneous downloads. A large capture asks for hundreds of tiles at
    #: once; firing them all floods the network stack and freezes the UI, so
    #: they queue and drain a few at a time.
    _MAX_INFLIGHT = 8

    def __init__(self, cache: TileCache | None = None,
                 parent: Service | None = None) -> None:
        super().__init__(parent)
        self._cache = cache if cache is not None else TileCache(default_cache_dir())
        if not NATIVE:
            self._nam = QNetworkAccessManager(self)
        # (source_id, x, y, z) -> QNetworkReply, so repeated asks coalesce.
        self._inflight: dict[tuple[str, int, int, int], QNetworkReply] = {}
        # Pending downloads waiting for an in-flight slot.
        self._queue: list[tuple[TileSource, int, int, int]] = []
        self._queued: set[tuple[str, int, int, int]] = set()

    @property
    def cache(self) -> TileCache:
        return self._cache

    def request(self, source: TileSource, x: int, y: int, z: int) -> SurveyRaster | None:
        """Ask for tile ``(x, y, z)`` of ``source``.

        Returns the :class:`SurveyRaster` synchronously on a cache hit (and does not
        emit); otherwise returns ``None`` and emits :attr:`tileReady` once the
        download finishes. Out-of-range zooms are refused.
        """
        if z < 0 or z > source.max_zoom:
            return None
        key = (source.id, x, y, z)
        cached = self._cache.get(source.id, x, y, z)
        if cached is not None:
            img = SurveyRaster.decode(cached)
            if img is not None:
                return img
            # Corrupt cache entry — fall through and re-download.
        if key in self._inflight or key in self._queued:
            return None  # already downloading / queued; the reply will emit
        if len(self._inflight) < self._MAX_INFLIGHT:
            self._start_download(source, x, y, z)
        else:
            self._queue.append((source, x, y, z))
            self._queued.add(key)
        return None

    def _pump(self) -> None:
        """Start queued downloads up to the concurrency limit."""
        while self._queue and len(self._inflight) < self._MAX_INFLIGHT:
            source, x, y, z = self._queue.pop(0)
            self._queued.discard((source.id, x, y, z))
            self._start_download(source, x, y, z)

    def _start_download(self, source: TileSource, x: int, y: int, z: int) -> None:
        if NATIVE:
            from georef.transport import request
            key = (source.id, x, y, z)
            self._inflight[key] = request(self, source.url(x, y, z), _USER_AGENT,
                                          lambda data: self._completed(key, data),
                                          lambda reason: self._completed(key, None, reason))
            return
        # QNetworkRequest's documented constructor takes QUrl.  Relying on
        # PySide's implicit str conversion enters binding code just as the
        # native network backend is being created; keep this path identical
        # to geocode.py and explicit on every Qt/Windows build.
        req = QNetworkRequest(QUrl(source.url(x, y, z)))
        req.setRawHeader(b"User-Agent", _USER_AGENT)
        req.setAttribute(QNetworkRequest.Attribute.RedirectPolicyAttribute,
                         QNetworkRequest.RedirectPolicy.NoLessSafeRedirectPolicy)
        reply = self._nam.get(req)
        key = (source.id, x, y, z)
        self._inflight[key] = reply
        reply.finished.connect(
            lambda: self._on_finished(source.id, x, y, z, reply))

    def _on_finished(self, source_id: str, x: int, y: int, z: int,
                     reply: QNetworkReply) -> None:
        key = source_id, x, y, z
        try:
            if reply.error() != QNetworkReply.NetworkError.NoError:
                self._completed(key, None, reply.errorString())
            else:
                self._completed(key, bytes(reply.readAll()))
        finally:
            reply.deleteLater()

    def _completed(self, key, data, error=""):
        self._inflight.pop(key, None)
        self._pump()
        img = SurveyRaster.decode(data) if data is not None else None
        if img is None:
            self.tileFailed.emit(*key, error or "undecodable tile data")
        else:
            self._cache.put(*key, data)
            self.tileReady.emit(*key, img)

    def cancel_all(self) -> None:
        """Abort every in-flight download and drop the queue (source changed)."""
        for reply in list(self._inflight.values()):
            reply.abort()
        self._inflight.clear()
        self._queue.clear()
        self._queued.clear()

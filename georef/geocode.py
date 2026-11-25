# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Host signals for asynchronous place and IP lookups."""
from dataclasses import dataclass
from functools import partialmethod
from georef.transport import Service, Signal
from ingetrazo_core import (
    geocode_url, parse_places, parse_ip, geocoder_init, geo_get,
    geo_search, geo_locate, geocode_received, ip_received,
)


NOMINATIM = "https://nominatim.openstreetmap.org/search"
IPAPI = "https://ipapi.co/json/"
from core.version import __version__

_USER_AGENT = (f"IngeTrazo/{__version__} (https://github.com/ingelibre/ingetrazo)").encode()


@dataclass(frozen=True)
class Place:
    """A geocoding candidate."""
    name: str
    lat: float
    lon: float


class _Fetcher(Service):
    __init__ = partialmethod(geocoder_init)
    _get = partialmethod(geo_get)


class Geocoder(_Fetcher):
    resultsReady = Signal(list)
    failed = Signal(str)
    search = partialmethod(geo_search)
    _received = partialmethod(geocode_received)


class IpLocator(_Fetcher):
    located = Signal(float, float, str)
    failed = Signal(str)
    locate = partialmethod(geo_locate)
    _received = partialmethod(ip_received)

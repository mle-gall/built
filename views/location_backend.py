"""Bind site lookup and tile requests from the Settings sheet to georef services."""
from PySide6.QtCore import QObject

from core.i18n import tr
from georef.geocode import Geocoder, IpLocator
from georef.tile_fetcher import TileFetcher


class SiteRequests(QObject):
    def __init__(self, draft, source, refresh):
        super().__init__()
        self.draft, self.source, self.refresh = draft, source, refresh
        self.tiles = TileFetcher(parent=self)
        self.tiles.tileReady.connect(self.tile_ready)
        self.searcher = Geocoder(self)
        self.searcher.resultsReady.connect(self.places)
        self.searcher.failed.connect(self.error)
        self.locator = IpLocator(self)
        self.locator.located.connect(self.located)
        self.locator.failed.connect(self.error)

    def tile_ready(self, *_):
        self.draft.tile_ready()
        self.refresh()

    def request(self, tiles):
        for index in tiles:
            x, y, z = map(int, index.split(","))
            if self.tiles.request(self.source, x, y, z) is not None:
                self.tile_ready()

    def places(self, items):
        self.draft.places([(p.name, p.lat, p.lon) for p in items])
        self.refresh()

    def located(self, lat, lon, label):
        self.draft.move_pin(lat, lon, 11)
        self.draft.note = tr("Near {label} (approximate — refine on the map).", label=label)
        self.refresh()

    def error(self, message):
        self.draft.note = message
        self.refresh()

    def close(self):
        self.refresh = lambda: None
        self.tiles.cancel_all()
        self.deleteLater()

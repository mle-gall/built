"""Qt delayed MIME rendering for the model clipboard."""
from __future__ import annotations

from PySide6.QtCore import QByteArray, QMimeData

from formats.clip_payload import decode, encode

MIME = "application/x-ingetrazo-clipboard"


class ClipMime(QMimeData):
    """The system-clipboard entry of one Copy: :data:`MIME` rendered on
    demand from the clipboard dict it was made for."""

    def __init__(self, clip: dict):
        super().__init__()
        self.clip = clip
        self._bytes: QByteArray | None = None

    def formats(self):
        return [MIME]

    def hasFormat(self, mime: str) -> bool:
        return mime == MIME

    def retrieveData(self, mime, _type):
        return clip_retrieve(self, mime)


from ingetrazo_core import (
    clip_retrieve, clip_available as available, clip_current_offer as current_offer, clip_flush as flush,
    clip_foreign as foreign, clip_own as own, clip_publish as publish,
)

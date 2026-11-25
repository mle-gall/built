"""Native model clipboard offers and platform pasteboard data."""
from formats.clip_payload import decode, encode
from ingetrazo_core import (
    ClipMime, clip_available as available, clip_current_offer as current_offer,
    clip_flush as flush, clip_foreign as foreign, clip_own as own, clip_publish as publish,
)

MIME = "application/x-ingetrazo-clipboard"
_offer = None
_change = None

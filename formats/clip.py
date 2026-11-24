"""Copy and paste between IngeTrazo windows (issue #76).

The model clipboard lives on the viewport (``viewport.clipboard``), so it
never left the process: with two IngeTrazo windows open side by side,
Ctrl+C in one and Ctrl+V in the other pasted nothing. Copy now also offers
the clipboard on the SYSTEM clipboard under :data:`MIME`, and Paste adopts
it when it came from another window.

The payload is a small zip: ``clip.json`` (reference point and the loose
edges) plus ``scene.igz``, a throwaway document holding the copied faces and
groups — written by the regular ``.igz`` writer, so materials, textures,
component definitions and group axes travel exactly as they do in a file.

Encoding is LAZY: :class:`ClipMime` builds the bytes only when another
application asks for them (Qt's delayed rendering), so Ctrl+C on a big
hedge costs nothing extra in the window that copies. A paste in the same
window sees its own :class:`ClipMime` and keeps the in-memory clipboard.
"""
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
        if mime != MIME:
            return None
        if self._bytes is None:
            try:
                self._bytes = QByteArray(encode(self.clip))
            except Exception:  # noqa: BLE001 - never crash the app from here
                self._bytes = QByteArray()
        return self._bytes


def publish(clip: dict) -> None:
    """Offer ``clip`` on the system clipboard (after a Copy/Cut)."""
    from PySide6.QtWidgets import QApplication
    app = QApplication.instance()
    if app is None:
        return
    if not getattr(app, "_clip_flush_hooked", False):
        app._clip_flush_hooked = True
        app.aboutToQuit.connect(flush)
    QApplication.clipboard().setMimeData(ClipMime(clip))


def flush() -> None:
    """On quit, trade our lazy offer for plain data: the copy then survives
    this window closing (another window can still paste it), and no Python
    subclass is left inside Qt's clipboard while the interpreter tears down
    — the CI run of 9393ef5 passed every test and still exited with a
    segfault."""
    from PySide6.QtWidgets import QApplication
    if QApplication.instance() is None:
        return
    cb = QApplication.clipboard()
    md = cb.mimeData()
    if not isinstance(md, ClipMime):
        return
    try:
        data = md.retrieveData(MIME, None)
    except Exception:  # noqa: BLE001 - quitting: never raise from here
        data = None
    plain = QMimeData()
    if data is not None and len(data):
        plain.setData(MIME, data)
    cb.setMimeData(plain)


def foreign() -> dict | None:
    """The clipboard another IngeTrazo window copied, decoded — or ``None``
    when the system clipboard holds this window's own copy, or no model
    clipboard at all."""
    from PySide6.QtWidgets import QApplication
    if QApplication.instance() is None:
        return None
    md = QApplication.clipboard().mimeData()
    if md is None or isinstance(md, ClipMime) or not md.hasFormat(MIME):
        return None
    return decode(md.data(MIME).data())


def own(current: dict | None, adopted_from=None) -> dict | None:
    """The copy made in ANOTHER project of this window (projects in tabs,
    #1066), as a clipboard of the project pasting it — or ``None`` when
    the system clipboard holds ``current`` itself, the copy
    ``adopted_from`` was made from, or no copy of this window.

    The copy crosses through the same encoding as between windows, so
    its materials, textures and component definitions become the pasting
    project's own instead of objects shared with the other document."""
    from PySide6.QtWidgets import QApplication
    if QApplication.instance() is None:
        return None
    md = QApplication.clipboard().mimeData()
    if not isinstance(md, ClipMime):
        return None
    if md.clip is current or md.clip is adopted_from or md.clip is None:
        return None
    data = md.retrieveData(MIME, None)
    if data is None or not len(data):
        return None
    return decode(bytes(data.data()))


def current_offer():
    from PySide6.QtWidgets import QApplication
    return QApplication.clipboard().mimeData() if QApplication.instance() is not None else None


def available() -> bool:
    """Whether the current system clipboard offers model geometry."""
    md = current_offer()
    return md is not None and md.hasFormat(MIME)

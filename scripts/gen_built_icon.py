# SPDX-License-Identifier: GPL-3.0-or-later
"""Rasterize Built's application icon from ``resources/icons/ingetrazo.svg``
with Qt's own SVG renderer (no Inkscape or ImageMagick needed).

    QT_QPA_PLATFORM=offscreen python scripts/gen_built_icon.py

Writes ``ingetrazo_<size>.png`` (16..512), ``ingetrazo_master.png`` (1024) and
``ingetrazo.ico`` (PNG-compressed entries). The macOS ``.icns`` is built from
the PNGs by ``packaging/build-macos-app.sh``. File names keep the old prefix so
packaging and the window icon find them unchanged.
"""
from __future__ import annotations

import struct
import sys
from pathlib import Path

from PySide6.QtCore import QBuffer, QByteArray, QIODevice, Qt
from PySide6.QtGui import QGuiApplication, QImage, QPainter
from PySide6.QtSvg import QSvgRenderer

ICONS = Path(__file__).resolve().parent.parent / "resources" / "icons"
SIZES = (16, 32, 48, 64, 128, 256, 512)


def render(svg: QSvgRenderer, size: int) -> QImage:
    img = QImage(size, size, QImage.Format_ARGB32_Premultiplied)
    img.fill(Qt.transparent)
    p = QPainter(img)
    p.setRenderHint(QPainter.Antialiasing)
    p.setRenderHint(QPainter.SmoothPixmapTransform)
    svg.render(p)
    p.end()
    return img


def png_bytes(img: QImage) -> bytes:
    data = QByteArray()
    buf = QBuffer(data)
    buf.open(QIODevice.WriteOnly)
    img.save(buf, "PNG")
    return bytes(data)


def write_ico(path: Path, images: dict[int, bytes]) -> None:
    entries = sorted(images.items())
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    table, blobs = b"", b""
    for size, blob in entries:
        dim = 0 if size >= 256 else size
        table += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset + len(blobs))
        blobs += blob
    path.write_bytes(header + table + blobs)


def main() -> int:
    app = QGuiApplication.instance() or QGuiApplication(sys.argv[:1])  # noqa: F841
    svg = QSvgRenderer(str(ICONS / "ingetrazo.svg"))
    if not svg.isValid():
        print("invalid SVG", file=sys.stderr)
        return 1
    for size in SIZES:
        render(svg, size).save(str(ICONS / f"ingetrazo_{size}.png"))
    render(svg, 1024).save(str(ICONS / "ingetrazo_master.png"))
    write_ico(ICONS / "ingetrazo.ico",
              {s: png_bytes(render(svg, s)) for s in (16, 32, 48, 64, 128, 256)})
    print("wrote", ", ".join(f"{s}px" for s in SIZES), "+ master + ico")
    return 0


if __name__ == "__main__":
    sys.exit(main())

# SPDX-License-Identifier: GPL-3.0-or-later
"""Rasterize Built's application icon from ``resources/icons/ingetrazo.svg``
with the renderer's resvg (no Inkscape or ImageMagick needed).

    python scripts/gen_built_icon.py

Writes ``ingetrazo_<size>.png`` (16..512), ``ingetrazo_master.png`` (1024) and
``ingetrazo.ico`` (PNG-compressed entries). The macOS ``.icns`` is built from
the PNGs by ``packaging/build-macos-app.sh``. File names keep the old prefix so
packaging and the window icon find them unchanged.
"""
from __future__ import annotations

import struct
import sys
from pathlib import Path

from ingetrazo_render import SurveyRaster, svg_rgba

ICONS = Path(__file__).resolve().parent.parent / "resources" / "icons"
SIZES = (16, 32, 48, 64, 128, 256, 512)


def render(size: int) -> SurveyRaster:
    return SurveyRaster(*svg_rgba(ICONS / "ingetrazo.svg", size))


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
    try:
        render(16)
    except ValueError as exc:
        print(exc, file=sys.stderr)
        return 1
    for size in SIZES:
        render(size).save(str(ICONS / f"ingetrazo_{size}.png"))
    render(1024).save(str(ICONS / "ingetrazo_master.png"))
    write_ico(ICONS / "ingetrazo.ico",
              {s: bytes(render(s).png()) for s in (16, 32, 48, 64, 128, 256)})
    print("wrote", ", ".join(f"{s}px" for s in SIZES), "+ master + ico")
    return 0


if __name__ == "__main__":
    sys.exit(main())

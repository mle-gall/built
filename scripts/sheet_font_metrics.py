# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Write the sheet font's metrics for ingetrazo_core sheets::metrics: the
vertical metrics, every glyph's advance and the kerning pairs of Qt's
"Sans Serif" at a pixel size of 100, regular and bold — what the Python
painters measured text with (views/composer.py, _text_width_mm and kin).

    .venv/bin/python scripts/sheet_font_metrics.py rust/ingetrazo_core/src/sheets/font_metrics.json
"""
from __future__ import annotations

import json
import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

CHARS = ("".join(chr(c) for c in range(32, 127)) + "".join(chr(c) for c in range(160, 256))
         + "—–±²³°·×•…’‘“”€√≈≤≥ΔΣΩπ")


def measure(bold: bool) -> dict:
    from PySide6.QtGui import QFont, QFontMetricsF
    font = QFont("Sans Serif")
    font.setPixelSize(100)
    font.setBold(bold)
    fm = QFontMetricsF(font)
    adv = {c: fm.horizontalAdvance(c) for c in CHARS}
    kern = {}
    for a in CHARS:
        for b in CHARS:
            k = fm.horizontalAdvance(a + b) - adv[a] - adv[b]
            if abs(k) > 1e-9:
                kern[a + b] = round(k, 6)
    return {"ascent": fm.ascent(), "descent": fm.descent(), "cap_height": fm.capHeight(),
            "height": fm.height(), "leading": fm.leading(), "line_spacing": fm.lineSpacing(),
            "average": fm.averageCharWidth(),
            "advance": {c: round(v, 6) for c, v in adv.items()}, "kern": kern}


def main(path: str) -> None:
    from PySide6.QtWidgets import QApplication
    QApplication.instance() or QApplication([])
    from PySide6.QtGui import QFont, QFontInfo
    data = {"family": QFontInfo(QFont("Sans Serif")).family(), "pixel_size": 100,
            "regular": measure(False), "bold": measure(True)}
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


if __name__ == "__main__":
    main(sys.argv[1])

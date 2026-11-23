# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The sheet font: Inter (resources/fonts/inter, SIL OFL), regular and
SemiBold for bold, the font the PDF writer embeds (ingetrazo_core
sheets::pdf). For the scripts that measure it (sheet_font_metrics.py) and
record the painter goldens against it (sheet_paint_goldens.py): Qt's
"Sans Serif", which the Python painters ask for, resolves to it."""
from __future__ import annotations

from pathlib import Path

FONTS = Path(__file__).resolve().parents[1] / "resources" / "fonts" / "inter"


def use_inter() -> str:
    """Register the bundled Inter with Qt and make "Sans Serif" it."""
    from PySide6.QtGui import QFont, QFontDatabase
    for weight in ("Regular", "SemiBold"):
        if QFontDatabase.addApplicationFont(str(FONTS / f"Inter-{weight}.ttf")) < 0:
            raise RuntimeError(f"Inter-{weight}.ttf did not load")
    QFont.insertSubstitution("Sans Serif", "Inter")
    return "Inter"

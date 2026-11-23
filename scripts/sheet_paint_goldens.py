# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Write the painter parity goldens from the Python painters of
views/composer.py (tests/sheet_paint_cases.py, tests/sheet_paint_recorder.py),
with the sheet font (scripts/sheet_font.py). Run before a painter's Python
is deleted; the goldens stay.

    .venv/bin/python scripts/sheet_paint_goldens.py tests/fixtures/sheet_paint/goldens_a.json.gz a
"""
from __future__ import annotations

import gzip
import json
import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def main(path: str, group: str) -> None:
    from PySide6.QtWidgets import QApplication
    QApplication.instance() or QApplication([])
    from scripts.sheet_font import use_inter
    use_inter()                     # the sheet font: what the Rust painters measure with
    from tests.sheet_paint_cases import cases, python_ops
    out = {name: python_ops(make(), name) for name, make in cases(group)}
    with gzip.open(path, "wt") as f:
        json.dump(out, f)
    print(len(out), "cases", sum(len(v) for v in out.values()), "primitives")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])

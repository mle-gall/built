# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The items the Sheets page's tools make (ingetrazo_core sheets::create):
the title block from a template, as the composer's load_cajetin_template
read it, docked in the page's corner."""
from __future__ import annotations

import json
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtWidgets import QApplication  # noqa: E402

import ingetrazo_core as NATIVE  # noqa: E402

_app = QApplication.instance() or QApplication([])


def test_a_title_block_starts_from_its_template(tmp_path):
    from core.composition import Composicion, MarcoVista
    tpl = {"name": "Estudio", "layout": "banded", "columns": 2, "fill_color": "#dfe6ee", "w_mm": 150.0,
           "h_mm": 40.0, "x_mm": 3.0, "locked": True, "nonsense": 1,
           "campos": [["PROYECTO", "Casa"], ["CLIENTE", 7], ["bad"], ["FECHA", ""], ["ESCALA", ""]]}
    (tmp_path / "Estudio.json").write_text(json.dumps(tpl), encoding="utf-8")
    comp = Composicion(name="A", paper="A3", landscape=True)
    comp.frames.append(MarcoVista(scale_n=50.0))
    item = NATIVE.sheet_new_item(comp, "cajetin", 0, 0, 0, 0, None, str(tmp_path), "Estudio")
    pw, ph = comp.page_size_mm()
    assert (item.layout, item.columns, item.fill_color, item.w_mm, item.h_mm) == ("banded", 2, "#dfe6ee", 150.0, 40.0)
    assert (item.x_mm, item.y_mm) == (pw - comp.margin_mm - 150.0, ph - comp.margin_mm - 40.0)
    assert item.locked is False
    rows = dict((r[0], r[1]) for r in item.campos)
    assert rows["PROYECTO"] == "Casa" and rows["CLIENTE"] == "7" and rows["ESCALA"] == "1:50" and rows["FECHA"]
    plain = NATIVE.sheet_new_item(comp, "cajetin", 0, 0, 0, 0, None, str(tmp_path), "Missing")
    assert plain.layout == "grid"

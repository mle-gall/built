# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The items the Sheets page's tools make (built_core sheets::create):
the title block from a template, as the composer's load_cajetin_template
read it, docked in the page's corner."""
from __future__ import annotations

import json
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import built_core as NATIVE  # noqa: E402


def test_a_title_block_starts_from_its_template(tmp_path):
    from built_core.composition import Composicion, MarcoVista
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


def test_a_line_type_writes_its_words_along_its_lines():
    """The words of a line type (GAS…) along a vector frame's lines of
    that layer: one per place built_core.layers.text_marks finds, in the line's
    colour, reading upright; none on the cut."""
    import json
    import numpy as np
    from built_core.composition import MarcoVista
    from built_core.layers import text_marks
    frame = MarcoVista(w_mm=100, h_mm=80, style="vectorial")
    segs = np.array([[10.0, 10.0, 90.0, 10.0], [10.0, 20.0, 10.0, 70.0], [20.0, 30.0, 80.0, 30.0]])
    spec = ("GAS", 1.5, 4.0, 12.0)
    content = {"segs": segs, "kinds": np.array([0, 1, 2], np.int8), "tags": ["Gas", "Gas", "Gas"],
               "styles": {"Gas": ((0.9, 0.6, 0.1), None, None, [6.0, 3.0], spec)}}
    ops = json.loads(NATIVE.sheet_frame_display(frame, content, True))
    words = [o for o in ops if o["op"] == "text"]
    want = len(text_marks((10.0, 10.0), (90.0, 10.0), spec)) + len(text_marks((10.0, 20.0), (10.0, 70.0), spec))
    assert want > 3 and len(words) == want
    assert all(w["text"] == "GAS" and w["bold"] for w in words)
    lines = {o.get("pen", {}).get("color") for o in ops if o["op"] in ("line", "lines", "polyline")}
    assert {w["color"] for w in words} <= lines, (words[0]["color"], lines)

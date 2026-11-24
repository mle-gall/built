# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Title-block designs (corner shapes, layouts, colours), the built-in
presets, and user title-block templates."""
from __future__ import annotations

from PySide6.QtGui import QImage, QPainter

from core.composition import (CAJETIN_DESIGN_BASE, CAJETIN_DESIGNS, Cajetin,
                              Composicion)


def _rgb(img: QImage, x: int, y: int) -> int:
    return img.pixel(x, y) & 0xFFFFFF


def test_look_fields_round_trip_and_old_documents_default():
    c = Composicion()
    c.cajetin = Cajetin(w_mm=120.0, h_mm=30.0, corner="rounded", radius_mm=4.0,
                        layout="banded", double_border=True, fill_color="#eeeeee",
                        label_color="#333333", text_color="#111111",
                        line_color="#222222", label_mm=35.0)
    back = Composicion.from_dict(c.to_dict()).cajetin
    assert back.look() == c.cajetin.look()
    d = c.to_dict()
    for k in Cajetin.LOOK_FIELDS:
        d["cajetin"].pop(k, None)               # a pre-designs document
    old = Composicion.from_dict(d).cajetin
    assert old.look() == dict(CAJETIN_DESIGN_BASE)
    assert old.design_key() == "classic"


def test_template_dict_keeps_rows_size_and_look_only():
    c = Cajetin(x_mm=50.0, y_mm=60.0, z=7.0, w_mm=140.0, h_mm=40.0,
                corner="rounded", campos=[["OBRA", "Pileta"]], locked=True)
    d = c.template_dict()
    assert d["w_mm"] == 140.0 and d["campos"] == [["OBRA", "Pileta"]]
    assert d["corner"] == "rounded"
    assert not {"x_mm", "y_mm", "z", "locked", "group_id"} & set(d)
    fresh = Cajetin(x_mm=1.0, y_mm=2.0)
    for k, v in d.items():
        setattr(fresh, k, v)
    assert (fresh.x_mm, fresh.y_mm) == (1.0, 2.0) and fresh.look() == c.look()


def _close(win, comp):
    comp.close()
    win._saved_version = win.viewport.scene.version
    win.close()


def _select(comp, model):
    comp._rebuild_canvas()
    it = comp._item_for(model)
    it.setSelected(True)
    comp.on_selection_changed()
    return it



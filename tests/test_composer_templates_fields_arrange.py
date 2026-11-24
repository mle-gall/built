# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Sheet templates, dynamic fields and the arrange commands."""
from __future__ import annotations

import datetime

import pytest

from core.composition import (Cajetin, Composicion, CotaItem, MarcoVista,
                              TextoItem, expand_fields, set_field_context)


def test_dynamic_fields_read_the_sheet():
    comp = Composicion(name="Lámina 3")
    comp.frames.append(MarcoVista(w_mm=100, h_mm=80, scale_n=40.0,
                                  view_key="scene:Planta"))
    comp.frames.append(MarcoVista(w_mm=200, h_mm=120, scale_n=75.0,
                                  view_key="std:front"))
    comp.cajetin = Cajetin(proyecto="Plaza Yanque", autor="MS", lamina="L-03")
    set_field_context(comp=comp, scene=None, path="/x/pileta fuente.igz",
                      index=2, total=5)
    text = ("{proyecto} · {autor} · {lamina} · {escala} · {archivo} · "
            "{hoja}/{total} · {nombre} · {escena} · ESC. 1:{n}")
    out = expand_fields(text)
    assert out.startswith("Plaza Yanque · MS · L-03 · 1:75 · pileta fuente · 3/5")
    assert "· Lámina 3 · Planta · " in out              # main frame has no
    assert out.endswith("ESC. 1:{n}")                  # scene: any bound one;
                                                       # unknown field kept
    assert expand_fields("{fecha}") == datetime.date.today().strftime("%d/%m/%Y")
    assert expand_fields("sin campos") == "sin campos"
    set_field_context()
    assert expand_fields("{escala}|{lamina}") == "|"


def test_fields_read_the_rows_the_user_edited():
    """{proyecto}/{autor}/{lamina} follow the title block's table, not the
    values it was created with; a row labelled in English or German, or
    without the accent, counts too."""
    comp = Composicion(name="Planta baja")
    comp.cajetin = Cajetin(proyecto="Casa", autor="MS", lamina="L-01")
    comp.cajetin.set_field("PROYECTO", "Plaza nueva")
    comp.cajetin.set_field("LÁMINA", "A-03")
    comp.cajetin.campos = [r for r in comp.cajetin.campos if r[0] != "AUTOR"]
    comp.cajetin.campos.append(["Verfasser", "Ana"])
    set_field_context(comp=comp)
    try:
        assert expand_fields("{proyecto}|{autor}|{lamina}") == \
            "Plaza nueva|Ana|A-03"
        comp.cajetin.campos = [r for r in comp.cajetin.campos
                               if r[0] != "LÁMINA"]
        comp.cajetin.campos.append(["Lamina", ""])
        assert expand_fields("{lamina}") == "Planta baja"   # empty → name
    finally:
        set_field_context()


def test_sheet_name_is_always_the_sheets_name():
    """{sheet} is the title block's LÁMINA row (the drawing number);
    {sheet_name} is the sheet's own name, whatever that row says."""
    comp = Composicion(name="Planta baja")
    comp.cajetin = Cajetin()
    comp.cajetin.set_field("LÁMINA", "A-101")
    set_field_context(comp=comp)
    try:
        assert expand_fields("{sheet} · {sheet_name} · {SHEET_NAME}") == \
            "A-101 · Planta baja · Planta baja"
    finally:
        set_field_context()


def _close(win, comp):
    comp.close()
    win._saved_version = win.viewport.scene.version
    win.close()



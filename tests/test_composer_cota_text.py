# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Sheet cotas: double-click edits the text, <> stands for the value."""
from __future__ import annotations

from core.composition import CotaItem


def test_cota_text_placeholder_expands_to_the_value():
    ct = CotaItem(dx_mm=80.0, dy_mm=0.0, scale_n=100.0)
    assert ct.label() == "8.00 m"
    ct.text = "H = <> (verificar)"
    assert ct.label() == "H = 8.00 m (verificar)"
    ct.text = "VARIABLE"
    assert ct.label() == "VARIABLE"



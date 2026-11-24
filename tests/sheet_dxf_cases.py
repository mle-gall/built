# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""DXF writer cases (tests/test_sheet_dxf_parity.py): line groups per class
and per layer with their line types, colours and own patterns, solids,
texts, odd layer names. The goldens are the Python writer's bytes
(formats/dxf_out.py before it moved to ingetrazo_core sheets::dxf)."""
from __future__ import annotations

import numpy as np


def cases():
    rng = np.random.default_rng(11)
    segs = lambda n: rng.uniform(-50.0, 250.0, (n, 4))  # noqa: E731
    out = {}
    out["classes"] = ([("Planta", segs(12)), ("Planta-PERFIL", segs(5)), ("Planta-CORTE", segs(3)),
                       ("Planta-OCULTAS", segs(4), "DASHED")], [], [])
    out["empty-groups"] = ([("Vista", np.empty((0, 4))), ("", segs(2))], [], [])
    out["layers"] = ([("Muros", segs(6), None, (0.8, 0.2, 0.1)),
                      ("Ejes", segs(4), "DASHDOT", (0.2, 0.4, 0.9)),
                      ("Gas", segs(3), "GASLINE", (0.9, 0.7, 0.0), (0.4, -0.1, 0.0, -0.1)),
                      ("señal ñ", segs(2), "DOT", None),
                      ("Muros", segs(2), "HIDDEN", (0.0, 0.0, 0.0))],
                     [("Muros", [((0, 0), (1, 0), (0, 1), (1, 1)), ((2.5, 1e-7), (3, 0), (2, 1e6), (3, 1))],
                       (0.5, 0.5, 0.5)),
                      ("Relleno", [((10, 10), (12, 10), (10, 12), (12, 12))], None)],
                     [("Gas", 1.25, 2.5, 0.18, 33.333333, "GAS"), ("Ejes", -4.0, 7.0, 0.25, 0.0, "A\nB")])
    out["tiny-and-huge"] = ([("x", np.array([[1e-12, -3.14159265358979, 123456789.123, 0.1 + 0.2]]))], [], [])
    return out

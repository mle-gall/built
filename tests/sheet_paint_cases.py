# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Synthetic sheet items for the painter parity goldens
(tests/test_sheet_paint_parity.py). ``python_ops(item)`` paints one with
the Python painter of views/composer.py through the recording painter."""
from __future__ import annotations

from core.composition import (BarraEscala, FlechaNorte, FormaItem, ImagenItem,
                              Leyenda, LlamadaItem, NivelItem)


def cases(group: str = "a"):
    """``(name, factory)``: a fresh record per call."""
    out = []
    for n, segs in ((100.0, 4), (50.0, 5), (250.0, 3), (1000.0, 4), (20.0, 6)):
        out.append((f"scalebar-{n:g}-{segs}", lambda n=n, s=segs: BarraEscala(scale_n=n, segments=s)))
    for size, ang in ((18.0, 0.0), (24.0, 30.0), (12.0, -95.0), (30.0, 180.0)):
        out.append((f"north-{size:g}-{ang:g}", lambda s=size, a=ang: FlechaNorte(size_mm=s, angle_deg=a)))
    out.append(("legend-empty", lambda: Leyenda(title="LEYENDA")))
    out.append(("legend-rows", lambda: Leyenda(
        title="Capas", w_mm=60.0, rows=["Muros", "Mobiliario", "Ejes"],
        swatches={"Muros": {"color": [0.8, 0.2, 0.1]},
                  "Mobiliario": {"color": [0.2, 0.4, 0.9], "linetype": "DASHED"},
                  "Ejes": {"pattern": [0.5, -0.25, 0.0, -0.25]}})))
    for kind in ("rect", "elipse", "poligono", "linea", "flecha", "terreno"):
        for variant in range(2):
            def make(kind=kind, v=variant):
                return FormaItem(kind=kind, w_mm=40.0 + 7 * v, h_mm=25.0 - 9 * v,
                                 stroke_mm=0.35 + 0.2 * v, fill=bool(v), invert=bool(v),
                                 radius_mm=3.0 * v, sides=6 + 2 * v,
                                 color="#1e242c" if v == 0 else "#a03020",
                                 ground=("ticks", "hatch")[v])
            out.append((f"shape-{kind}-{variant}", make))
    out.append(("shape-terreno-band", lambda: FormaItem(kind="terreno", w_mm=50.0, h_mm=-8.0,
                                                        ground="band", band_mm=5.0)))
    for shape in ("rect", "circle"):
        for k, (bx, by, sheet) in enumerate(((36.0, -6.0, "L-05"), (10.0, 10.0, ""), (-12.0, 30.0, "A2"))):
            out.append((f"callout-{shape}-{k}", lambda s=shape, bx=bx, by=by, sh=sheet, k=k: LlamadaItem(
                shape=s, bx_mm=bx, by_mm=by, number=str(k + 1), sheet=sh, size_mm=3.0 + k)))
    for symbol in ("triangle", "circle"):
        for k, (mirror, ax, ay, z) in enumerate(((False, 0.0, 0.0, 0.15), (True, 4.0, 3.0, -1.2),
                                                 (False, -2.0, 6.0, 0.0))):
            out.append((f"level-{symbol}-{k}", lambda s=symbol, m=mirror, ax=ax, ay=ay, z=z, k=k: NivelItem(
                symbol=s, mirror=m, ax_mm=ax, ay_mm=ay, z_m=z, size_mm=2.5 + k * 0.5)))
    out.append(("image-missing", lambda: ImagenItem(w_mm=60.0, h_mm=40.0, path="")))
    return out


def python_ops(item):
    """The Python painter's primitives for ``item`` (item-local mm)."""
    from tests.sheet_paint_recorder import RecordingPainter
    from views import composer as c
    p = RecordingPainter()
    kind = type(item).__name__
    if kind == "BarraEscala":
        c.paint_scalebar_mm(p, item)
    elif kind == "FlechaNorte":
        c.paint_norte_mm(p, item)
    elif kind == "Leyenda":
        c.paint_leyenda_mm(p, item)
    elif kind == "FormaItem":
        c.paint_forma_mm(p, item)
    elif kind == "LlamadaItem":
        c.paint_llamada_mm(p, item)
    elif kind == "NivelItem":
        c.paint_nivel_mm(p, item)
    elif kind == "ImagenItem":
        c.paint_image_mm(p, item, None)
    else:
        raise ValueError(kind)
    return p.ops

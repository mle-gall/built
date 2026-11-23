# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Synthetic sheet items for the painter parity goldens
(tests/test_sheet_paint_parity.py). ``python_ops(item)`` paints one with
the Python painter of views/composer.py through the recording painter."""
from __future__ import annotations

import math

from core.composition import (BarraEscala, CotaAngularItem, CotaItem, CotaRadialItem,
                              EtiquetaItem, FlechaNorte, FormaItem, ImagenItem, Leyenda,
                              LlamadaItem, NivelItem, PerfilTerreno)


def cases(group: str = "a"):
    """``(name, factory)``: a fresh record per call."""
    if group == "b":
        return cases_b()
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


def cases_b():
    out = []
    dims = [
        dict(), dict(ends="arrow", dx_mm=30.0, dy_mm=-18.0, sep_mm=6.0),
        dict(ends="none", text_pos="below", dx_mm=0.0, dy_mm=40.0, sep_mm=-5.0),
        dict(text_pos="centered", dx_mm=55.0, dy_mm=3.0, text="<> libre", units="cm", decimals=1),
        dict(text_pos="centered", dx_mm=6.0, dy_mm=0.0),
        dict(text_pos="aside", dx_mm=0.0, dy_mm=-35.0, text_align="horizontal"),
        dict(text_pos="aside_below", dx_mm=25.0, dy_mm=25.0, text_flip=True),
        dict(text_align="horizontal", dx_mm=20.0, dy_mm=30.0, text_along="end"),
        dict(text_along="start", dx_mm=-30.0, dy_mm=5.0, text_bg="#ffffff", text_bg_opacity=0.7),
        dict(axis="h", dx_mm=40.0, dy_mm=12.0, sep_mm=-8.0, text_color="#a03020"),
        dict(axis="v", dx_mm=14.0, dy_mm=-30.0, sep_mm=9.0, text_flip=True, ends="arrow"),
        dict(text_dx_mm=3.0, text_dy_mm=-2.0, text_along="end", dx_mm=18.0, dy_mm=0.0),
        dict(scale_n=50.0, dx_mm=120.0, dy_mm=0.0, text_mm=3.5, stroke_mm=0.35, offset_mm=2.0),
    ]
    for i, kw in enumerate(dims):
        out.append((f"dimension-{i}", lambda kw=kw: CotaItem(**kw)))
    angs = [dict(), dict(ax_mm=20.0, ay_mm=10.0, bx_mm=-15.0, by_mm=12.0, ends="tick"),
            dict(ax_mm=-10.0, ay_mm=-25.0, bx_mm=25.0, by_mm=-5.0, radius_mm=8.0, ends="none",
                 text_bg="#eef0f2", decimals=2, text="∠ <>"),
            dict(ax_mm=40.0, ay_mm=0.0, bx_mm=-40.0, by_mm=-1.0, radius_mm=30.0, text_color="#3060a0")]
    for i, kw in enumerate(angs):
        out.append((f"angular-{i}", lambda kw=kw: CotaAngularItem(**kw)))
    rads = [dict(), dict(kind="diameter"), dict(radius_mm=4.0, angle_deg=120.0),
            dict(kind="diameter", radius_mm=3.0, angle_deg=-100.0, ends="tick", centre_mark=False),
            dict(placement="out", angle_deg=200.0, text_bg="#ffffff", units="mm", decimals=0),
            dict(placement="in", kind="diameter", radius_mm=25.0, angle_deg=45.0, ends="none")]
    for i, kw in enumerate(rads):
        out.append((f"radial-{i}", lambda kw=kw: CotaRadialItem(**kw)))
    labels = [dict(text="Pozo de registro"), dict(text="Muro norte levantado en 2026 con piedra",
                                                  w_mm=30.0, ax_mm=40.0, ay_mm=5.0),
              dict(text="BUZÓN\nTipo B", ax_mm=10.0, ay_mm=-12.0, arrow=False, bold=True,
                   leaders=[{"ax_mm": 30.0, "ay_mm": -10.0}, {"ax_mm": -5.0, "ay_mm": 20.0}]),
              dict(text="Escultura de campesino con pedestal", w_mm=25.0, ax_mm=10.0, ay_mm=30.0,
                   bg_color="#ffffff", bg_opacity=0.8, dot=False, italic=True, size_pt=9.0),
              dict(text="Farola ornamental", w_mm=50.0, ax_mm=60.0, ay_mm=10.0, underline=True,
                   color="#304050", stroke_mm=0.35)]
    for i, kw in enumerate(labels):
        out.append((f"label-{i}", lambda kw=kw: EtiquetaItem(**kw)))
    for i, kw in enumerate([dict(), dict(scale_n=500.0, exag=10.0, grid=False, title="Eje 1"),
                            dict(w_mm=120.0, h_mm=50.0, fill=False, grid_h_m=25.0, grid_v_m=2.0,
                                 scale_n=1000.0, exag=4.0),
                            dict(w_mm=60.0, h_mm=30.0)]):
        out.append((f"profile-{i}", lambda kw=kw: PerfilTerreno(**kw)))
    out.append(("profile-empty", lambda: PerfilTerreno(w_mm=100.0, h_mm=40.0)))
    return out


class _Sample:
    def __init__(self, station, elevation):
        self.station, self.elevation = station, elevation


class FakeProfile:
    """A sampled terrain profile (georef.profile.Profile's reading side)."""

    def __init__(self, seed: int):
        n = 40 + 7 * seed
        self.length = 320.0 + 90.0 * seed
        self.samples = [_Sample(self.length * k / (n - 1),
                                None if 17 <= k <= 19 and seed % 2 == 0 else
                                2400.0 + 12.0 * math.sin(k / 5.0 + seed) + 0.3 * k)
                        for k in range(n)]

    def elevations(self):
        return [s.elevation for s in self.samples if s.elevation is not None]

    def min_elevation(self):
        es = self.elevations()
        return min(es) if es else None

    def max_elevation(self):
        es = self.elevations()
        return max(es) if es else None


def profile_of(name: str):
    """``(profile, path_name, message)`` the composer would give the case."""
    if name == "profile-empty":
        return None, "", "Trace a path with the Path tool (T) first."
    seed = int(name.split("-")[1])
    return FakeProfile(seed), f"Canal {seed}", ("DEM partly loaded" if seed == 2 else None)


def python_ops(item, name: str = ""):
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
    elif kind == "CotaItem":
        c.paint_cota_mm(p, item)
    elif kind == "CotaAngularItem":
        c.paint_cota_angular_mm(p, item)
    elif kind == "CotaRadialItem":
        c.paint_cota_radial_mm(p, item)
    elif kind == "EtiquetaItem":
        c.paint_etiqueta_mm(p, item)
    elif kind == "PerfilTerreno":
        profile, path_name, message = profile_of(name)
        c.paint_perfil_mm(p, item, profile, path_name, message)
    else:
        raise ValueError(kind)
    return p.ops

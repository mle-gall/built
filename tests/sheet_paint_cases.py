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
    if group == "c":
        return cases_c()
    if group == "d":
        return cases_d()
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


def cases_c():
    """Text blocks, title blocks and the sheet's border (the kinds the page
    drew its own way until rust/sheets-6)."""
    from core.composition import Cajetin, Composicion, TextoItem
    out = []
    texts = [
        dict(text="Planta baja"),
        dict(text="Nota: las cotas en metros salvo indicación contraria, verificar en obra.",
             w_mm=50.0, size_pt=9.0),
        dict(text="Centro\ndos líneas", align="center", bold=True, size_pt=18.0),
        dict(text="Derecha subrayada", align="right", underline=True, italic=True, color="#a03020"),
        dict(text="Con fondo y un texto que se parte en varias líneas", w_mm=40.0,
             bg_color="#fff3c4", bg_opacity=0.6),
        dict(text="", bg_color="#ffffff"),
    ]
    for i, kw in enumerate(texts):
        out.append((f"text-{i}", lambda kw=kw: TextoItem(**kw)))
    blocks = [
        dict(proyecto="Vivienda", autor="M. S.", fecha="2026-11-24", lamina="L-01"),
        dict(proyecto="Ampliación de la vivienda unifamiliar en la calle Mercaderes, Arequipa, "
                      "segunda etapa", autor="", lamina="A-102"),
        dict(campos=[["PROYECTO", "Canal de riego"], ["CLIENTE", "Junta de usuarios"],
                     ["FECHA", "nov 2026"], ["ESCALA", "1:500"], ["LÁMINA", "P-03"]],
             columns=2, fill_color="#eef1f4"),
        dict(layout="banded", fill_color="#dfe6ee", proyecto="Plaza mayor", lamina="U-01",
             corner="rounded", radius_mm=4.0),
        dict(layout="minimal", proyecto="Detalle de escalera", autor="R. P.", fecha="",
             corner="chamfer", radius_mm=3.0, double_border=True),
        dict(label_mm=40.0, border_mm=0.7, line_mm=0.25, line_color="#203040",
             label_color="#406080", text_color="#102030", w_mm=150.0, h_mm=45.0,
             campos=[["Nº", 7], ["VACÍO", None], ["REVISIÓN", "B"]]),
    ]
    for i, kw in enumerate(blocks):
        out.append((f"titleblock-{i}", lambda kw=kw: Cajetin(**kw)))
    borders = [dict(border=True), dict(border=True, border_style="dashed", border_mm=0.35),
               dict(border=True, border_style="double", border_radius_mm=5.0, border_color="#3a6ea5",
                    paper="A3", landscape=False, margin_mm=15.0)]
    for i, kw in enumerate(borders):
        out.append((f"sheet-border-{i}", lambda kw=kw: Composicion(**kw)))
    return out


def _frame_drawing(seed: int):
    """A vector drawing in frame mm: segments with every line class, and
    two cut rings (one with a hole)."""
    import numpy as np
    rng = np.random.default_rng(seed)
    segs = rng.uniform(-5.0, 95.0, (40, 4))
    kinds = np.array([k % 4 for k in range(40)], np.int8)
    fills = [np.array([[10, 10], [40, 10], [40, 30], [10, 30]], float),
             np.array([[18, 15], [30, 15], [30, 24], [18, 24]], float),
             np.array([[60, 40], [80, 45], [70, 70]], float)]
    return segs, kinds, fills


#: What fills each frame case: ``(missing, (segs, kinds, fills) or None,
#: (w_px, h_px) or None)``.
FRAME_CONTENT = {}
#: The layer styles of a case: ``(tags per segment, {tag: line_styles row})``.
FRAME_STYLES = {}
#: The looks of a case's cut rings: ``[(colour, hatch, ground, ink, form)]``.
FRAME_FILLS = {}


def cases_d():
    """View frames (paint_frame_mm without layer styles or annotations)."""
    from core.composition import MarcoVista
    out = []

    def add(name, content, **kw):
        FRAME_CONTENT[name] = content
        base = dict(x_mm=10.0, y_mm=10.0, w_mm=90.0, h_mm=70.0, scale_n=100.0, view_key="std:top",
                    style="vectorial", show_title=False)
        base.update(kw)
        out.append((name, lambda kw=base: MarcoVista(**kw)))
    add("frame-missing", ("Iso", None, None), view_key="scene:Iso")
    add("frame-vector", (None, _frame_drawing(1), None))
    add("frame-vector-hatch", (None, _frame_drawing(2), None), cut_fill="hatch", cut_hatch_mm=1.2,
        cut_fill_color="#a03020", pen_hatch_mm=0.1)
    add("frame-vector-nofill", (None, _frame_drawing(3), None), cut_fill="none", pen_cut_mm=0.7,
        pen_profile_mm=0.4, pen_edge_mm=0.13)
    add("frame-vector-empty", (None, None, None))
    seg, _k, fills = _frame_drawing(4)
    add("frame-vector-nokinds", (None, (seg, None, fills), None))
    add("frame-image-fit", (None, None, (1063, 827)), style="sombreado")
    add("frame-image-crop", (None, None, (900, 500)), style="style:Architectural", border=True,
        border_mm=0.5, border_color="#3a6ea5")
    add("frame-image-none", (None, None, None), style="sombreado", grid_m=2.0)
    add("frame-grid", (None, _frame_drawing(5), None), grid_m=5.0, scale_n=200.0)
    titles = [
        dict(show_title=True),
        dict(show_title=True, title_number="3", title_sheet="A-102", title_subtitle="Planta alta",
             title_align="right", title_mm=5.0),
        dict(show_title=True, title_number="12", title_align="center", title_pos="above",
             title_text="Sección {escala}"),
        dict(show_title=True, title_style="simple", title_subtitle="Cotas en m", title_align="center"),
        dict(show_title=True, title_style="simple", title_align="left", title_scale=False,
             view_key="scene:Iso", title_pos="above"),
        dict(show_title=True, title_style="bar", title_number="5", title_sheet="L-03",
             title_subtitle="Alzado", title_mm=3.5),
        dict(show_title=True, title_style="bar", perspective=True, view_key="std:iso"),
        dict(show_title=True, perspective=True, view_key="std:front", title_text="Fachada {escala}"),
    ]
    for i, kw in enumerate(titles):
        add(f"frame-title-{i}", (None, _frame_drawing(10 + i), None), **kw)
    for pos in ("under-right", "under-left", "inside-bl", "inside-br"):
        add(f"frame-scale-{pos}", (None, _frame_drawing(20), None), show_scale=True, scale_pos=pos,
            scale_mm=3.5, scale_n=50.0)
    add("frame-scale-text", (None, _frame_drawing(21), None), show_scale=True, scale_text="Escala {n}",
        scale_n=25.0)
    # Layer by layer: each tag with its colour, weight, cut weight and line type.
    styles = {"Muros": ((0.8, 0.2, 0.1), "continuous", 0.5, 0.9, None, None),
              "Ejes": ((0.2, 0.4, 0.9), "center", None, None, None, None),
              "Rejas": (None, "dashed", 0.25, None, None, None),
              "Propio": (None, "custom:gas", None, 0.3, (2.0, 0.5, 0.2, 0.5), None),
              "Layer 0": (None, "continuous", None, None, None, None)}
    for i, pens in enumerate(((0.18, 0.35, 0.5), (0.13, 0.25, 0.7))):
        segs, kinds, fills = _frame_drawing(30 + i)
        tags = [("Muros", "Ejes", "Rejas", "Propio", "Layer 0", "Desconocida")[k % 6] for k in range(len(segs))]
        add(f"frame-layers-{i}", (None, (segs, kinds, fills), None), hidden_lines=True,
            pen_edge_mm=pens[0], pen_profile_mm=pens[1], pen_cut_mm=pens[2])
        FRAME_STYLES[f"frame-layers-{i}"] = (tags, styles)
    import numpy as np
    # The cut filled layer by layer: one look per ring (colour, pattern,
    # ground, ink, form), as compute_hlr hands them to the painter.
    walls = [np.array([[10, 10], [60, 10], [60, 14], [10, 14]], float),          # a wall cut in plan
             np.array([[10, 20], [14, 20], [14, 60], [10, 60]], float),
             np.array([[30, 30], [45, 30], [45, 32], [30, 32]], float),          # a board along
             np.array([[50, 40], [53, 40], [53, 47], [50, 47]], float)]          # a stud across
    looks = {
        "solid-by-layer": [((0.8, 0.3, 0.2), None, None, None, None)] * 2 + [((0.2, 0.5, 0.8), None, None, None, None)] * 2,
        "diagonal": [((0.8, 0.3, 0.2), "diagonal", None, None, {"angle": 0.0, "scale": 1.0})] * 2
        + [(None, "diagonal", (0.9, 0.9, 0.7), (0.1, 0.2, 0.6), {"angle": 30.0, "scale": 2.0})] * 2,
        "concrete": [(None, "concrete", None, None, {"scale": 1.0})] * 4,
        "crosshatch": [((0.5, 0.5, 0.5), "crosshatch", None, None, {"angle": 15.0, "scale": 0.7})] * 4,
        "insulation": [(None, "insulation", (1.0, 1.0, 0.9), None, {"scale": 1.0})] * 4,
        "brick-plan": [((0.8, 0.4, 0.3), "brick", None, None, {"cut": "plan", "scale_n": 20.0, "scale": 1.0,
                                                               "joints": "single"})] * 4,
        "brick-section": [((0.8, 0.4, 0.3), "brick", None, None, {"cut": "vertical", "scale_n": 20.0, "scale": 1.0,
                                                                  "joints": "double", "mortar": True})] * 4,
        "brick-small": [(None, "brick", None, None, {"cut": "plan", "scale_n": 200.0, "scale": 1.0})] * 4,
        "wood-cross": [((0.6, 0.4, 0.2), "wood", None, None, {"cut": "plan", "cross": True, "scale_n": 20.0,
                                                              "scale": 1.0})] * 4,
    }
    for name, look in looks.items():
        segs, kinds, _f = _frame_drawing(40)
        add(f"frame-fills-{name}", (None, (segs[:6], kinds[:6], walls), None), cut_fill="solid", cut_hatch_mm=1.5,
            pen_hatch_mm=0.1, scale_n=20.0)
        FRAME_FILLS[f"frame-fills-{name}"] = look
    return out


def frame_content(name: str):
    """The content of a frame case as sheet_frame_display takes it."""
    from core.layers import LINETYPES
    missing, drawing, image = FRAME_CONTENT[name]
    out = {"missing": missing, "image": image}
    if drawing is not None:
        segs, kinds, fills = drawing
        out.update(segs=segs, kinds=kinds, fills=fills)
    if name in FRAME_FILLS:
        out["fill_looks"] = [(None if c is None else list(c), h, None if g is None else list(g),
                              None if i is None else list(i), None if f is None else dict(f))
                             for c, h, g, i, f in FRAME_FILLS[name]]
    if name in FRAME_STYLES:
        tags, styles = FRAME_STYLES[name]
        out["tags"] = tags
        out["styles"] = {t: (st[0], st[2], st[3], list(st[4] if st[4] is not None else LINETYPES.get(st[1], ())))
                         for t, st in styles.items()}
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


def profile_of(name: str, native: bool = False):
    """``(profile, path_name, message)`` the composer would give the case;
    ``native``: the profile as sheets::profile hands it to the painter."""
    if name == "profile-empty":
        return None, "", "Trace a path with the Path tool (T) first."
    seed = int(name.split("-")[1])
    prof = FakeProfile(seed)
    if native:
        prof = {"samples": [(s.station, 0.0, 0.0, s.elevation) for s in prof.samples],
                "length": prof.length, "complete": all(s.elevation is not None for s in prof.samples)}
    return prof, f"Canal {seed}", ("DEM partly loaded" if seed == 2 else None)


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
    elif kind == "TextoItem":
        c.paint_text_mm(p, item)
    elif kind == "Cajetin":
        c.paint_cajetin_mm(p, item)
    elif kind == "Composicion":
        c.paint_sheet_border_mm(p, item)
    elif kind == "MarcoVista":
        from PySide6.QtGui import QImage
        missing, drawing, image = FRAME_CONTENT[name]
        segs = kinds = fills = None
        if drawing is not None:
            segs, kinds, fills = drawing
        img = QImage(image[0], image[1], QImage.Format_RGB32) if image is not None else None
        fill_styles = None
        if name in FRAME_FILLS:
            from PySide6.QtGui import QColor
            qc = lambda c: None if c is None else QColor.fromRgbF(*c)  # noqa: E731
            fill_styles = [(qc(col), h, qc(g), qc(i), None if f is None else dict(f))
                           for col, h, g, i, f in FRAME_FILLS[name]]
        c.paint_frame_mm(p, item, img, hlr=segs, kinds=kinds, fills=fills, missing=missing,
                         styles=FRAME_STYLES.get(name), fill_styles=fill_styles)
    elif kind == "PerfilTerreno":
        profile, path_name, message = profile_of(name)
        c.paint_perfil_mm(p, item, profile, path_name, message)
    else:
        raise ValueError(kind)
    return p.ops

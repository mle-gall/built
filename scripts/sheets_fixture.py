# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A synthetic document with one sheet, for the Sheets workspace captures
(scripts/sheets_native_shots.sh) and the native page tests.

Three boxes and a slab, a saved scene, and an A3 sheet holding:
- a vector plan at 1:100;
- an Architectural iso view;
- a text;
- the title block.

    .venv/bin/python scripts/sheets_fixture.py out.igz
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


def _box(mesh, x0, y0, z0, x1, y1, z1):
    from ingetrazo_core import Vec3 as V
    p = [V(x, y, z) for z in (z0, z1) for y in (y0, y1) for x in (x0, x1)]
    for loop in ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4), (1, 3, 7, 5), (3, 2, 6, 7), (2, 0, 4, 6)):
        mesh.add_face([p[i] for i in loop])


def _every_kind(comp) -> None:
    """One item of every kind the sheet holds, below the views."""
    from core.composition import (BarraEscala, CotaAngularItem, CotaItem, CotaRadialItem,
                                  EtiquetaItem, FlechaNorte, FormaItem, ImagenItem, Leyenda,
                                  LlamadaItem, NivelItem, PerfilTerreno)
    comp.scalebars.append(BarraEscala(x_mm=215, y_mm=185, scale_n=100.0))
    comp.nortes.append(FlechaNorte(x_mm=380, y_mm=180, size_mm=22, angle_deg=20))
    comp.leyendas.append(Leyenda(x_mm=320, y_mm=185, w_mm=50, title="Capas",
                                 rows=["Muros", "Losa"],
                                 swatches={"Muros": {"color": [0.8, 0.3, 0.2]},
                                           "Losa": {"linetype": "dashed"}}))
    comp.shapes.append(FormaItem(kind="rect", x_mm=20, y_mm=205, w_mm=30, h_mm=18, radius_mm=3,
                                 fill=True))
    comp.shapes.append(FormaItem(kind="terreno", x_mm=60, y_mm=210, w_mm=50, h_mm=6,
                                 ground="hatch"))
    comp.cotas.append(CotaItem(x_mm=40, y_mm=172, dx_mm=120, dy_mm=0, sep_mm=-4, scale_n=100.0))
    comp.cotas_ang.append(CotaAngularItem(x_mm=150, y_mm=245, ax_mm=30, ay_mm=0, bx_mm=10,
                                          by_mm=-20))
    comp.cotas_rad.append(CotaRadialItem(x_mm=130, y_mm=60, radius_mm=12, kind="diameter",
                                         angle_deg=30))
    comp.etiquetas.append(EtiquetaItem(x_mm=120, y_mm=215, w_mm=40, text="Losa de cimentación",
                                       ax_mm=-15, ay_mm=-30))
    comp.niveles.append(NivelItem(x_mm=250, y_mm=150, z_m=3.2, text="N.P.T. {z}"))
    comp.llamadas.append(LlamadaItem(x_mm=260, y_mm=60, w_mm=40, h_mm=30, number="2",
                                     sheet="A-02"))
    comp.perfiles.append(PerfilTerreno(x_mm=20, y_mm=230, w_mm=110, h_mm=45))
    comp.images.append(ImagenItem(x_mm=60, y_mm=258, w_mm=30, h_mm=20, path=""))


def build(path: Path) -> None:
    from PySide6.QtWidgets import QApplication
    QApplication.instance() or QApplication([])
    from core.composition import Composicion, MarcoVista, TextoItem
    from core.saved_views import SavedView
    from core.scene import Scene
    from formats.igz import save_scene
    scene = Scene()
    _box(scene.mesh, 0, 0, 0, 12, 8, 0.3)
    _box(scene.mesh, 1, 1, 0.3, 5, 4, 3.2)
    _box(scene.mesh, 6, 1.5, 0.3, 11, 6, 2.6)
    _box(scene.mesh, 2, 5, 0.3, 4, 7, 4.5)
    scene.saved_views.append(SavedView("Iso", target=(6.0, 4.0, 1.5), distance=24.0,
                                       perspective=False))
    comp = Composicion(name="Plans", paper="A3", landscape=True)
    comp.frames.append(MarcoVista(x_mm=20, y_mm=20, w_mm=190, h_mm=150, scale_n=100.0,
                                  view_key="std:top", style="vectorial", show_title=True))
    comp.frames.append(MarcoVista(x_mm=225, y_mm=20, w_mm=175, h_mm=150, scale_n=150.0,
                                  view_key="scene:Iso", style="style:Architectural"))
    comp.texts.append(TextoItem(x_mm=20, y_mm=190, w_mm=180, text="Ground floor and massing",
                                size_pt=18.0, bold=True))
    if os.environ.get("SHEETS_FIXTURE_ALL_KINDS"):
        _every_kind(comp)
    comp.cajetin = comp.default_cajetin()
    comp.cajetin.set_field("PROYECTO", "Sheets fixture")
    scene.compositions.append(comp)
    save_scene(scene, path, backup=False)


if __name__ == "__main__":
    build(Path(sys.argv[1]))

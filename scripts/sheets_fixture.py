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
    comp.cajetin = comp.default_cajetin()
    comp.cajetin.set_field("PROYECTO", "Sheets fixture")
    scene.compositions.append(comp)
    save_scene(scene, path, backup=False)


if __name__ == "__main__":
    build(Path(sys.argv[1]))

# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Generate the bundled texture library (procedural, tileable, ours).

Commercial modellers ship material libraries whose images are copyrighted, so
Built's are generated here on the renderer's annotation canvas
(``built_render.AnnotationCanvas``; gradients with numpy) — deterministic (fixed seed),
seamlessly tileable (every speckle/line is drawn with toroidal wrap), and
licence-clean. Re-run to regenerate:

    python scripts/gen_textures.py

Writes ``resources/textures/library/<category>/<name>.png`` (256 px) and the
manifest ``resources/textures/library.json`` with per-texture real-world tile
sizes (metres) and category/display names (translated at display time).
"""
from __future__ import annotations

import json
import math
import random
from pathlib import Path

import numpy as np
from built_core import ScreenPoint as QPointF, ScreenRect as QRectF
from built_render import AnnotationCanvas, PaintColor, PaintPen, SurveyRaster

PX = 256
ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "resources" / "textures" / "library"

rng = random.Random(20260711)


def QColor(r, g, b, a=255):
    return PaintColor(int(r), int(g), int(b), int(a))


def QPen(color, width=1.0):
    return PaintPen(color, float(width))


class _Tile:
    """A tile being drawn: an opaque background (H×W×3) under the canvas."""

    def __init__(self, background):
        self.background = np.ascontiguousarray(background, np.uint8)
        self.canvas = AnnotationCanvas(PX, PX, float(PX))

    def save(self, path):
        rgba = np.dstack([self.background, np.full((PX, PX), 255, np.uint8)])
        pixels = self.canvas.composite(rgba.tobytes())
        return SurveyRaster(PX, PX, pixels).save(path)


def _canvas(base) -> tuple[_Tile, AnnotationCanvas]:
    tile = _Tile(np.broadcast_to(np.array(base, np.uint8), (PX, PX, 3)))
    return tile, tile.canvas


def _ramp(t, stops):
    """Colours along ``t`` (0..1 per pixel) through evenly spaced ``stops``."""
    stops = np.array(stops, float)
    at = np.clip(t, 0., 1.) * (len(stops) - 1)
    i = np.minimum(at.astype(int), len(stops) - 2)
    f = (at - i)[..., None]
    return np.rint(stops[i] * (1 - f) + stops[i + 1] * f).astype(np.uint8)


def _rounded(rect_xywh, rx, ry, steps=8):
    """A rounded rectangle's outline as points (the canvas draws polygons)."""
    x, y, w, h = rect_xywh
    rx, ry = min(rx, w / 2), min(ry, h / 2)
    points = []
    for cx, cy, start in ((x + w - rx, y + ry, -90), (x + w - rx, y + h - ry, 0),
                          (x + rx, y + h - ry, 90), (x + rx, y + ry, 180)):
        for k in range(steps + 1):
            a = math.radians(start + 90 * k / steps)
            points.append(QPointF(cx + rx * math.cos(a), cy + ry * math.sin(a)))
    return points


def _wrap_rect(p, x, y, w, h, color):
    """Fill a rect with toroidal wrap so the tile stays seamless."""
    p.setPen(None)
    p.setBrush(color)
    for dx in (-PX, 0, PX):
        for dy in (-PX, 0, PX):
            p.drawRect(QRectF(x + dx, y + dy, w, h))


def _speckle(p, count, size, colors, alpha=255):
    for _ in range(count):
        x, y = rng.uniform(0, PX), rng.uniform(0, PX)
        s = rng.uniform(size * 0.5, size * 1.5)
        _wrap_rect(p, x, y, s, s, QColor(*rng.choice(colors), alpha))


def _tone(base, spread):
    """Luminance-only variation: ONE delta for all channels (independent
    per-channel noise reads as pastel hue shifts on bricks/stone)."""
    d = rng.randint(-spread, spread)
    return [min(255, max(0, v + d)) for v in base]


def _hline(p, y, color, width=1.0):
    p.setPen(QPen(QColor(*color), width))
    p.drawLine(QPointF(0, y), QPointF(PX, y))


def _vline(p, x, color, width=1.0):
    p.setPen(QPen(QColor(*color), width))
    p.drawLine(QPointF(x, 0), QPointF(x, PX))


# ---- Recipes ------------------------------------------------------------------

def brick(base, mortar, dark):
    img, p = _canvas(mortar)
    rows, cols = 8, 4
    bh, bw, gap = PX / rows, PX / cols, 3
    for r in range(rows):
        off = (bw / 2) if r % 2 else 0
        for c in range(-1, cols + 1):
            tone = _tone(base, 14)
            x, y = c * bw + off, r * bh
            _wrap_rect(p, x + gap / 2, y + gap / 2, bw - gap, bh - gap,
                       QColor(*tone))
    _speckle(p, 300, 2, [dark], alpha=40)
    return img


def concrete(base, speck_dark, speck_light, panels=False):
    img, p = _canvas(base)
    _speckle(p, 900, 2, [speck_dark], alpha=28)
    _speckle(p, 600, 2, [speck_light], alpha=30)
    if panels:
        for y in (0, PX / 2):
            _hline(p, y, speck_dark, 2)
        _vline(p, 0, speck_dark, 2)
        for cx, cy in ((PX / 4, PX / 4), (3 * PX / 4, PX / 4),
                       (PX / 4, 3 * PX / 4), (3 * PX / 4, 3 * PX / 4)):
            p.setPen(None)
            p.setBrush(QColor(*speck_dark))
            p.drawEllipse(QPointF(cx, cy), 3.5, 3.5)
    return img


def blocks(base, joint):
    img, p = _canvas(joint)
    rows, cols = 4, 2
    bh, bw, gap = PX / rows, PX / cols, 4
    for r in range(rows):
        off = (bw / 2) if r % 2 else 0
        for c in range(-1, cols + 1):
            tone = _tone(base, 8)
            _wrap_rect(p, c * bw + off + gap / 2, r * bh + gap / 2,
                       bw - gap, bh - gap, QColor(*tone))
    _speckle(p, 500, 2, [(90, 90, 92)], alpha=22)
    return img


def ashlar(base, joint):
    img, p = _canvas(joint)
    rng2 = random.Random(7)
    rows = 4
    bh = PX / rows
    for r in range(rows):
        x = -rng2.uniform(0, 40)
        while x < PX:
            w = rng2.uniform(45, 95)
            d = rng2.randint(-16, 16)
            tone = [min(255, max(0, v + d)) for v in base]
            _wrap_rect(p, x + 2, r * bh + 2, w - 4, bh - 4, QColor(*tone))
            x += w
    _speckle(p, 400, 2, [(70, 66, 60)], alpha=30)
    return img


def wood(base, grain, dark):
    img, p = _canvas(base)
    planks = 6
    pw = PX / planks
    for i in range(planks):
        tone = _tone(base, 12)
        _wrap_rect(p, i * pw + 1, 0, pw - 2, PX, QColor(*tone))
        for _ in range(7):
            x = i * pw + rng.uniform(3, pw - 3)
            p.setPen(QPen(QColor(*grain, 70), rng.uniform(0.6, 1.4)))
            p.drawLine(QPointF(x, -PX), QPointF(x + rng.uniform(-6, 6), 2 * PX))
        _vline(p, i * pw, dark, 1.6)
    return img


def roof_colonial(base, shade):
    img, p = _canvas(shade)
    rows, cols = 4, 4
    th, tw = PX / rows, PX / cols
    for r in range(rows):
        off = (tw / 2) if r % 2 else 0
        for c in range(-1, cols + 1):
            tone = _tone(base, 14)
            for dx in (-PX, 0, PX):
                rect = (c * tw + off + dx, r * th - th * 0.15, tw - 2, th * 1.15)
                p.setPen(QPen(QColor(*shade), 2))
                p.setBrush(QColor(*tone))
                p.drawPolygon(_rounded(rect, tw * 0.45, th * 0.35))
    return img


def corrugated(base, dark, light):
    waves = 8
    ww = PX / waves
    # Across each wave: dark, light at its crest, dark (pixel centres).
    t = ((np.arange(PX) + .5) % ww) / ww
    return _Tile(np.broadcast_to(_ramp(t, [dark, light, dark]), (PX, PX, 3)))


def floor_tiles(base, joint, n):
    img, p = _canvas(joint)
    tw = PX / n
    for r in range(n):
        for c in range(n):
            tone = _tone(base, 6)
            _wrap_rect(p, c * tw + 1.5, r * tw + 1.5, tw - 3, tw - 3,
                       QColor(*tone))
    return img


def checker(a, b, n=4):
    img, p = _canvas(a)
    tw = PX / n
    for r in range(n):
        for c in range(n):
            if (r + c) % 2:
                _wrap_rect(p, c * tw, r * tw, tw, tw, QColor(*b))
    return img


def pavers(base, joint):
    img, p = _canvas(joint)
    rows, cols = 8, 4
    bh, bw = PX / rows, PX / cols
    for r in range(rows):
        off = (bw / 2) if r % 2 else 0
        for c in range(-1, cols + 1):
            tone = _tone(base, 12)
            _wrap_rect(p, c * bw + off + 1.5, r * bh + 1.5,
                       bw - 3, bh - 3, QColor(*tone))
    _speckle(p, 400, 2, [(96, 96, 96)], alpha=26)
    return img


def noise(base, tones, count=1400, size=3):
    img, p = _canvas(base)
    _speckle(p, count, size, tones, alpha=60)
    return img


def metal_brushed(base, line):
    img, p = _canvas(base)
    for _ in range(220):
        y = rng.uniform(0, PX)
        alpha = rng.randint(14, 40)
        p.setPen(QPen(QColor(*line, alpha), rng.uniform(0.5, 1.2)))
        p.drawLine(QPointF(0, y), QPointF(PX, y))
    return img


def glass(base, hi):
    # Along the diagonal: highlight, base at the middle, highlight.
    y, x = np.mgrid[0:PX, 0:PX] + .5
    return _Tile(_ramp((x + y) / (2 * PX), [hi, base, hi]))


# category → [(name, recipe, tile_w, tile_h)]
LIBRARY = {
    "brick": [
        ("brick_red", lambda: brick((150, 62, 46), (196, 188, 178), (60, 24, 18)), 1.0, 0.6),
        ("brick_clay", lambda: brick((186, 120, 78), (204, 196, 184), (92, 52, 30)), 1.0, 0.6),
        ("brick_gray", lambda: brick((136, 134, 130), (190, 188, 184), (66, 64, 62)), 1.0, 0.6),
    ],
    "concrete": [
        ("concrete_smooth", lambda: concrete((178, 178, 176), (120, 120, 120), (210, 210, 208)), 1.5, 1.5),
        ("concrete_exposed", lambda: concrete((168, 168, 166), (110, 110, 110), (205, 205, 203), panels=True), 1.2, 1.2),
        ("concrete_blocks", lambda: blocks((172, 172, 170), (140, 140, 138)), 0.8, 0.8),
    ],
    "stone": [
        ("stone_ashlar", lambda: ashlar((158, 148, 132), (110, 102, 90)), 1.2, 0.9),
        ("stone_cobble", lambda: pavers((142, 136, 126), (98, 94, 86)), 0.8, 0.8),
    ],
    "wood": [
        ("wood_planks_light", lambda: wood((198, 162, 116), (150, 112, 70), (140, 106, 66)), 1.0, 1.0),
        ("wood_planks_dark", lambda: wood((124, 86, 56), (86, 56, 34), (70, 46, 28)), 1.0, 1.0),
    ],
    "roof": [
        ("roof_tiles_colonial", lambda: roof_colonial((176, 96, 66), (110, 56, 38)), 0.9, 0.6),
        ("roof_corrugated", lambda: corrugated((172, 176, 180), (120, 124, 130), (216, 220, 224)), 0.9, 0.9),
    ],
    "floor": [
        ("floor_ceramic_white", lambda: floor_tiles((228, 226, 220), (180, 178, 172), 2), 0.6, 0.6),
        ("floor_ceramic_gray", lambda: floor_tiles((176, 178, 180), (130, 132, 134), 2), 0.6, 0.6),
        ("floor_checker", lambda: checker((232, 230, 226), (52, 52, 56)), 0.8, 0.8),
        ("floor_pavers", lambda: pavers((168, 148, 128), (120, 104, 88)), 0.8, 0.8),
    ],
    "metal": [
        ("metal_brushed", lambda: metal_brushed((176, 180, 186), (120, 124, 130)), 1.0, 1.0),
        ("metal_dark", lambda: metal_brushed((92, 96, 102), (50, 54, 60)), 1.0, 1.0),
    ],
    "ground": [
        ("grass", lambda: noise((96, 138, 74), [(70, 110, 52), (120, 160, 90), (84, 124, 62)]), 1.5, 1.5),
        ("gravel", lambda: noise((166, 160, 150), [(120, 114, 104), (196, 190, 180), (140, 134, 124)], count=1800), 1.0, 1.0),
        ("asphalt", lambda: noise((72, 72, 74), [(50, 50, 52), (98, 98, 100), (60, 60, 62)], count=2000, size=2), 1.5, 1.5),
    ],
    "glass": [
        ("glass_blue", lambda: glass((168, 198, 214), (208, 228, 238)), 1.0, 1.0),
    ],
}


def main(out: Path = OUT) -> None:
    """Write the library under ``out`` and its manifest beside it (a folder
    given on the command line: to look at a run without touching the
    bundled files)."""
    manifest: dict = {"categories": []}
    for cat, entries in LIBRARY.items():
        (out / cat).mkdir(parents=True, exist_ok=True)
        items = []
        for name, recipe, sw, sh in entries:
            img = recipe()
            rel = f"{cat}/{name}.png"
            assert img.save(str(out / rel)), rel
            items.append({"file": rel, "name": name, "sw": sw, "sh": sh})
            print("wrote", rel)
        manifest["categories"].append({"id": cat, "items": items})
    (out.parent / "library.json").write_text(
        json.dumps(manifest, indent=1), encoding="utf-8")
    print("manifest ok:",
          sum(len(c["items"]) for c in manifest["categories"]), "textures")


if __name__ == "__main__":
    import sys
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else OUT)

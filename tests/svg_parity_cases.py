# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The line groups of the SVG parity fixtures (tests/test_svg_parity.py).

The goldens in ``tests/fixtures/svg_parity/`` hold the text
``formats.svg_out.svg_text`` wrote for these groups in Python (integ/built
60e017a8), before the lines path moved to Rust."""
from __future__ import annotations

import math
import random

import numpy as np


def groups(seed: int):
    """Line groups as a frame exports them: per line class or per tag, with
    line types, colours, document dash patterns and own weights; arcs,
    closed rings, junctions and broken edges to chain."""
    rng = random.Random(seed)
    out = []
    names = ["Planta", "Planta-PERFIL", "Planta-CORTE", "Planta-OCULTAS",
             "Muros", "Muros", "Mobiliario & <otros>", "1 nivel", 'Cota "B"', ""]
    for gi in range(rng.randint(2, 6)):
        segs = []
        for _ in range(rng.randint(1, 6)):
            shape = rng.choice(("line", "arc", "ring", "broken", "star"))
            x, y = rng.uniform(-20, 20), rng.uniform(-15, 15)
            if shape == "line":
                segs.append((x, y, x + rng.uniform(-5, 5), y + rng.uniform(-5, 5)))
            elif shape in ("arc", "ring"):
                r = rng.uniform(0.3, 4)
                n = rng.randint(6, 24)
                stop = n if shape == "ring" else rng.randint(2, n - 1)
                pts = [(x + r * math.cos(2 * math.pi * k / n), y + r * math.sin(2 * math.pi * k / n))
                       for k in range(stop + 1)]
                segs += [(*pts[k], *pts[k + 1]) for k in range(stop)]
            elif shape == "broken":
                dx, dy = rng.uniform(-3, 3), rng.uniform(-3, 3)
                for k in range(rng.randint(2, 5)):
                    segs.append((x + k * dx, y + k * dy, x + (k + 1) * dx, y + (k + 1) * dy))
            else:
                for k in range(rng.randint(3, 5)):
                    a = rng.uniform(0, 2 * math.pi)
                    segs.append((x, y, x + 2 * math.cos(a), y + 2 * math.sin(a)))
        rng.shuffle(segs)
        segs = [s if rng.random() < 0.5 else (s[2], s[3], s[0], s[1]) for s in segs]
        g = [names[(seed + gi) % len(names)], np.asarray(segs, dtype=float)]
        extra = rng.randint(0, 4)
        if extra >= 1:
            g.append(rng.choice(["CONTINUOUS", "DASHED", "hidden", "CENTER", "", "MINE"]))
        if extra >= 2:
            g.append(rng.choice([None, (0.2, 0.4, 0.6), (1.0, 0.0, 0.0), (0.123, 0.5, 0.999)]))
        if extra >= 3:
            g.append(rng.choice([None, (0.5, -0.25), (1.0, -0.5, 0.0, -0.5)]))
        if extra >= 4:
            g.append(rng.choice([None, 0.25, 0.7, 0.13]))
        out.append(tuple(g))
    out.append(("Vacío", np.empty((0, 4))))
    return out


def cases():
    """``(name, kwargs)`` for ``svg_text(**kwargs)``."""
    out = []
    for seed in range(10):
        g = groups(seed)
        out.append((f"scale-{seed}", dict(groups=g, scale_n=[50, 100, 0.5, 1000][seed % 4])))
        out.append((f"pens-{seed}", dict(groups=g, scale_n=75.0,
                                         pens={"edge": 0.1, "profile": 0.5}, line_colours=False)))
        out.append((f"paper-{seed}", dict(groups=g, scale_n=100.0, paper=(420.0, 297.0))))
        out.append((f"window-{seed}", dict(groups=g, scale_n=100.0, paper=(297.0, 210.0),
                                           window=(-10.0, -8.0, 12.0, 9.5), margin=0.0)))
    out.append(("empty", dict(groups=[("X", np.empty((0, 4)))], scale_n=100.0)))
    return out

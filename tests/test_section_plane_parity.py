# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A section plane's archive, its next symbol and its effective fill moved
to Rust (``rust/ingetrazo_core/src/model/section_fill.rs``) and must answer
as ``core/section.py``'s Python did at ``9cf53080``: ``from_dict`` of seeded
raw entries (odd fills, layers, flags), ``to_dict`` back, the letters the
next plane takes, and the fill a plane's cut is drawn with under a style.

Regenerate only on purpose, against the reference implementation:
``python tests/test_section_plane_parity.py --write``."""
from __future__ import annotations

import ingetrazo_core
import gzip
import json
import random
import sys
from pathlib import Path
from types import SimpleNamespace

GOLDEN = Path(__file__).parent / "fixtures" / "section_plane_golden.json.gz"


def raws():
    rng = random.Random(20261125)
    out = [{"point": [0, 0, 0], "normal": [0, 0, 0]},
           {"point": (1, 2, 3), "normal": (0, 0, 2), "uid": 7, "fill": {}},
           {"point": [0, 0, 0], "normal": [1, 0, 0], "layer": "", "fill": None},
           {"point": [0, 0, 0], "normal": [1, 0, 0], "layer": 3, "fill": [1]}]
    for k in range(120):
        raw = {"point": [rng.uniform(-9, 9) for _ in range(3)],
               "normal": [rng.uniform(-1, 1) for _ in range(3)]}
        for key, value in (("name", f"Cut {k}"), ("symbol", rng.choice(("A", "B", ""))),
                           ("active", rng.choice((True, 1, 0))), ("hidden", rng.choice((True, False))),
                           ("uid", rng.randint(1, 500)), ("layer", rng.choice(("Walls", "", None)))):
            if rng.random() < 0.5:
                raw[key] = value
        if rng.random() < 0.6:
            fill = {}
            if rng.random() < 0.7:
                fill["pattern"] = rng.choice(("solid", "hatch", "cross", "none", "layers", "dots", 3))
            if rng.random() < 0.6:
                fill["color"] = rng.choice(([0.1, 0.2, 0.3], (1, 0, 0, 1), [1, 2], "red", [1, "a", 0]))
            if rng.random() < 0.6:
                fill["spacing"] = rng.choice((0.1, 0, -1, "x", 2, True))
            raw["fill"] = fill
        out.append(raw)
    return out


def symbol_cases():
    rng = random.Random(5)
    out = [[], ["A"], ["a", " b "], [None, ""], [chr(ord("A") + i) for i in range(26)],
           [chr(ord("A") + i) for i in range(26)] + ["AA", "AB"]]
    for _ in range(40):
        out.append(rng.sample([chr(ord("A") + i) for i in range(26)] + ["AA", "ab", "Ba", 3],
                              rng.randint(0, 20)))
    return out


def answers(section):
    out = {"planes": [], "symbols": [], "fills": []}
    styles = [SimpleNamespace(), SimpleNamespace(section_fill=False),
              SimpleNamespace(section_fill=True, section_fill_color=(0.5, 0.5, 0.5))]
    for raw in raws():
        try:
            sp = section.SectionPlane.from_dict(raw)
        except Exception as e:  # noqa: BLE001
            out["planes"].append(type(e).__name__)
            continue
        uid = sp.uid
        d = sp.to_dict()
        d["uid"] = "same" if d["uid"] == uid else d["uid"]
        if isinstance(raw.get("uid"), int):
            d["given_uid"] = uid == raw["uid"]
        out["planes"].append([d, [sp.point.toTuple(), sp.normal.toTuple(), sp.name, sp.symbol,
                                  sp.active, sp.hidden, sp.layer, sp.fill]])
        out["fills"].append([list(ingetrazo_core.section_effective_fill(sp, st)) for st in styles])
    for taken in symbol_cases():
        planes = [SimpleNamespace(symbol=t) for t in taken] + [SimpleNamespace()]
        out["symbols"].append(ingetrazo_core.section_next_symbol(planes))
    out["symbols"].append(ingetrazo_core.section_next_symbol(None))
    return out


def test_the_section_plane_archive_answers_as_the_python_did():
    from core import section
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers(section)))
    for key in want:
        bad = [i for i, (g, w) in enumerate(zip(got[key], want[key])) if g != w]
        assert len(got[key]) == len(want[key]) and not bad, (key, bad[:10])


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import section
    data = answers(section)
    GOLDEN.parent.mkdir(exist_ok=True)
    GOLDEN.write_bytes(gzip.compress(json.dumps(data).encode(), mtime=0))
    print(f"wrote {GOLDEN}")

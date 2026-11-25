# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A saved view's archive and its eye/target/up import moved to Rust (the
document's own ``doc/file_view.rs`` archive, and
``rust/ingetrazo_core/src/model/saved_view.rs``) and must answer as
``core/saved_views.py``'s Python did at ``9cf53080``: ``to_dict`` of views
built from seeded fields, ``from_dict`` of seeded raw entries (missing
fields, tuples for lists, odd types), and ``from_lookat`` of seeded
cameras (plans looking straight down or up, parallel framing).

Regenerate only on purpose, against the reference implementation:
``python tests/test_saved_view_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "saved_view_golden.json.gz"


def raw_entries():
    rng = random.Random(20261124)
    out = [{}, {"name": "Plan"}, {"name": "T", "target": (1, 2, 3)},
           {"name": "N", "target": None, "fov": 45, "parallel": 0},
           {"name": "Z", "hidden_layers": ("a", "b"), "folder": 3, "lens": [1]},
           {"name": "E", "style": {}, "section": {}, "georef": {}, "lens": {}}]
    for k in range(120):
        raw = {"name": f"Scene {k}"}
        if rng.random() < 0.9:
            raw["target"] = [rng.uniform(-50, 50) for _ in range(3)]
        for key, f in (("distance", lambda: rng.uniform(0.1, 300)),
                       ("yaw", lambda: rng.uniform(-7, 7)),
                       ("pitch", lambda: rng.uniform(-1.6, 1.6)),
                       ("fov", lambda: rng.choice((45.0, 30.0, 60, 90.5)))):
            if rng.random() < 0.8:
                raw[key] = f()
        for key in ("parallel", "two_point"):
            if rng.random() < 0.3:
                raw[key] = rng.choice((True, 1, False))
        if rng.random() < 0.5:
            raw["hidden_layers"] = [f"L{i}" for i in range(rng.randint(0, 4))]
        if rng.random() < 0.3:
            raw["hidden_folders"] = [f"F{i}" for i in range(rng.randint(0, 3))]
        if rng.random() < 0.3:
            raw["hidden_objects"] = [f"u{i}" for i in range(rng.randint(0, 3))]
        for key in ("style", "section", "georef", "shadows", "hidden_shown", "lens"):
            if rng.random() < 0.3:
                raw[key] = {"k": rng.randint(0, 3)} if rng.random() < 0.8 else {}
        if rng.random() < 0.2:
            raw["navigator_view"] = rng.choice(("plans", "", None, "v1"))
        if rng.random() < 0.3:
            raw["folder"] = rng.choice(("Walls", "", None, "A/B"))
        out.append(raw)
    return out


def lookat_cases():
    rng = random.Random(77)
    out = [((0, 0, 10), (0, 0, 0), (0, 1, 0), 45.0, True, None),
           ((0, 0, 10), (0, 0, 0), (1, 0, 0), 45.0, False, 12.0),
           ((0, 0, -10), (0, 0, 0), (0, 0, 0), 30.0, True, None),
           ((0, 0, 0), (0, 0, 0), (0, 0, 1), 45.0, True, None),
           ((3, 4, 0), (0, 0, 0), (0, 0, 1), 60.0, False, 0.0)]
    for _ in range(150):
        eye = tuple(rng.uniform(-40, 40) for _ in range(3))
        tgt = tuple(rng.uniform(-10, 10) for _ in range(3))
        up = tuple(rng.uniform(-1, 1) for _ in range(3))
        out.append((eye, tgt, up, rng.choice((45.0, 35.0, 70.0)), rng.random() < 0.6,
                    rng.choice((None, 5.0, 33.3))))
    return out


def answers(sv):
    out = {"from_dict": [], "lookat": []}
    for raw in raw_entries():
        v = sv.SavedView.from_dict(raw)
        out["from_dict"].append([v.to_dict(), {f: repr(getattr(v, f)) for f in v._record_fields}])
    for eye, tgt, up, fov, persp, oh in lookat_cases():
        v = sv.from_lookat("L", eye, tgt, up, fov, persp, oh, hidden_layers=["x"])
        out["lookat"].append(v.to_dict())
    return out


def test_the_saved_view_archive_answers_as_the_python_one_did():
    from ingetrazo_core import saved_views
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers(saved_views)))
    for key in want:
        bad = [i for i, (g, w) in enumerate(zip(got[key], want[key])) if g != w]
        assert len(got[key]) == len(want[key]) and not bad, (key, bad[:10])


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from ingetrazo_core import saved_views
    data = answers(saved_views)
    GOLDEN.parent.mkdir(exist_ok=True)
    GOLDEN.write_bytes(gzip.compress(json.dumps(data).encode(), mtime=0))
    print(f"wrote {GOLDEN}")

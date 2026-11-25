# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A tag's constructor moved to Rust (``model/tag_styles.rs: tag_init``) and
must clean what it is given as ``core/layers.py``'s Python did at
``2bd1c38d``: every field of ``Layer(...)`` over seeded keywords, odd values
included (non-finite angles, out-of-range scales and opacities, unknown
line types, hatches and joints, grounds that are not colours), and
positional arguments. Uids are random: compared by whether one was given.

Regenerate only on purpose, against the reference implementation:
``python tests/test_tag_init_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "tag_init_golden.json.gz"

CHOICES = {
    "visible": (True, False, 0, 1, None), "locked": (True, False, 0, "x"),
    "color": (None, (0.1, 0.2, 0.3), [1, 0, 0], (0.5, 0.5, 0.5, 1.0)),
    "folder": (None, "", "F", "Steel"),
    "linetype": ("continuous", "dashed", "dot", "lin:mine", "lin:", "nonsense", None, 3),
    "lineweight": (None, 0.25, 7.0), "cut_weight": (None, 0.5),
    "opacity": (1.0, 0.5, 0.0, 2.0, -1, math.nan, "0.3"),
    "printable": (True, False, 0, None),
    "uid": (None, "", "abc123uid"), "key": (None, "", "K"),
    "hatch": (None, "brick", "concrete", "nope", "pat:x", "solid", 3),
    "hatch_ground": (None, "layer", (0.9, 0.9, 0.9), [2, -1, 0.5], (1, 2), "red", ("a", 0, 0)),
    "hatch_color": (None, "layer", (0, 0, 0), [0.2, 0.3, 0.4, 0.5]),
    "hatch_angle": (0.0, 45.0, -30.0, 725.5, math.inf, math.nan, "x", None, "90"),
    "hatch_scale": (1.0, 0.01, 50.0, 2.5, math.inf, None, "3"),
    "hatch_on_paper": (None, True, False, 0, 1),
    "brick": (None, (240, 115, 52, 10), "x", (1, 2)),
    "hatch_cross": (False, True, 1), "joints": (None, "single", "double", "nope"),
    "mortar": (False, True, 0), "hatch_offset": (None, (10.0, 0.0), (0, 0), "x", (1, 2, 3)),
}


def _j(x):
    if isinstance(x, float) and not math.isfinite(x):
        return repr(x)
    if isinstance(x, (list, tuple)):
        return [_j(v) for v in x]
    if isinstance(x, dict):
        return {k: _j(v) for k, v in x.items()}
    return x


def answers(L):
    rng = random.Random(20261125)
    out = []
    for k in range(400):
        kw = {n: rng.choice(v) for n, v in CHOICES.items() if rng.random() < 0.5}
        name = rng.choice(("Walls", "Layer 0", "Ü", "Notes"))
        try:
            ly = L.Layer(name, **kw)
        except Exception as e:  # noqa: BLE001
            out.append(type(e).__name__)
            continue
        vals = L.layer_values(ly)
        given = kw.get("uid")
        vals["uid"] = "given" if given and vals["uid"] == given else bool(vals["uid"])
        out.append(_j(vals))
    for args in (("P",), ("P", False), ("P", True, True, (1, 0, 0), "F", "dashed", 0.5, 0.4, False)):
        vals = L.layer_values(L.Layer(*args))
        vals["uid"] = bool(vals["uid"])
        out.append(_j(vals))
    return out


def test_the_tag_constructor_cleans_as_the_python_did():
    from built_core import layers
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers(layers)))
    bad = [i for i, (g, w) in enumerate(zip(got, want)) if g != w]
    assert len(got) == len(want) and not bad, bad[:10]


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from built_core import layers
    data = answers(layers)
    GOLDEN.parent.mkdir(exist_ok=True)
    GOLDEN.write_bytes(gzip.compress(json.dumps(data).encode(), mtime=0))
    print(f"wrote {GOLDEN} ({len(data)} cases)")

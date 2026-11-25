# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""``Scene.selection_bounds`` and ``Scene.bounds`` moved to Rust
(``model/scene_bounds.rs``) and must answer as ``core/scene.py``'s Python
did at ``fff1954a`` on what ``tests/test_bounds_parity.py`` leaves out:
selections of loose edges and faces, guide points, lines and circles,
dimensions, things with ``corners()`` or ``vertices``, placements and
nothing at all; and the model's box with hidden tags, hidden objects and
faces, loose edges alone, an empty scene, and asked twice (the kept box).
Exact but where a placement's points go through its matrix (4 ulp, as
``test_bounds_parity``).

Regenerate only on purpose, against the reference implementation:
``python tests/test_selection_bounds_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path
from types import SimpleNamespace

GOLDEN = Path(__file__).parent / "fixtures" / "selection_bounds_golden.json.gz"


def _tup(v):
    return None if v is None else [v.x(), v.y(), v.z()]


def answers():
    from built_core import Matrix4 as M, Vec3 as V
    from built_core.dimension import Dimension
    from built_core.group import Group
    from built_core.guide import Guide, GuideCircle
    from built_core.layers import Layer
    from built_core.mesh import Mesh
    from built_core.scene import Scene
    rng = random.Random(20261125)
    p = lambda s=10.0: V(rng.uniform(-s, s), rng.uniform(-s, s), rng.uniform(-s, s))  # noqa: E731
    out = []
    for k in range(60):
        scene = Scene()
        scene.layers.append(Layer("Off", visible=False))
        mesh = scene.loose_mesh
        for _ in range(rng.randint(0, 3)):
            a = p()
            f = mesh.add_face([a, a + V(1, 0, 0), a + V(1, 2, 0.5), a + V(0, 2, 0)])
            if f is not None and rng.random() < 0.3:
                f.attrs["layer"] = "Off"
            elif f is not None and rng.random() < 0.2:
                f.attrs["hidden"] = True
        for _ in range(rng.randint(0, 3)):
            e = mesh.add_edge(p(), p())
            if e is not None and rng.random() < 0.3:
                e.layer = "Off"
        proto = Mesh()
        proto.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 1, 0), V(0, 1, 3)])
        for i in range(rng.randint(0, 4)):
            g = Group(proto, f"g{i}")
            if rng.random() < 0.8:
                x = M()
                x.translate(rng.uniform(-30, 30), rng.uniform(-30, 30), rng.uniform(-3, 3))
                x.rotate(rng.uniform(0, 360), 0.2, 0.3, 1.0)
                g.xform = x
            g.hidden = rng.random() < 0.2
            if rng.random() < 0.2:
                g.layer = "Off"
            if rng.random() < 0.4:
                c = Group(proto, f"c{i}")
                y = M()
                y.translate(rng.uniform(-3, 3), 0.0, 1.0)
                c.xform = y
                c.hidden = rng.random() < 0.3
                g.children.append(c)
            scene.groups.append(g)
        scene.version += 1
        first = scene.bounds()
        again = scene.bounds()
        pool = (list(mesh.edges) + list(mesh.faces) + list(scene.groups)
                + [Guide(p()), Guide(p(), V(0, 0, 1)), GuideCircle(p(), V(0.2, 0.1, 1), rng.uniform(0.5, 4)),
                   Dimension(p(), p(), V(0, 0.5, 0)),
                   SimpleNamespace(corners=lambda a=p(): [a, a + V(1, 1, 0)]),
                   SimpleNamespace(vertices=[p(), p(), p()]),
                   SimpleNamespace(a=p(), b=p()), SimpleNamespace(name="nothing")])
        picks = []
        for frac in (0.0, 0.2, 0.6, 1.0):
            chosen = [e for e in pool if rng.random() < frac]
            scene.selection = list(chosen)
            lo, hi = scene.selection_bounds()
            picks.append([_tup(lo), _tup(hi)])
        out.append({"bounds": [_tup(first[0]), _tup(first[1])],
                    "again": [_tup(again[0]), _tup(again[1])], "selection": picks})
    return out


def _close(a, b):
    if a is None or b is None:
        return a is b
    if isinstance(a, list):
        return len(a) == len(b) and all(_close(x, y) for x, y in zip(a, b))
    return abs(a - b) <= 4 * math.ulp(max(abs(b), 1.0))


def test_boxes_of_the_model_and_the_selection_answer_as_the_python_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers()))
    assert len(got) == len(want)
    exact = 0
    for i, (g, w) in enumerate(zip(got, want)):
        for key in w:
            assert _close(g[key], w[key]), (i, key, g[key], w[key])
        exact += g == w
    assert exact == len(want), f"{len(want) - exact} scenes within 4 ulp but not exact"


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    GOLDEN.parent.mkdir(exist_ok=True)
    data = json.dumps(answers(), separators=(",", ":")).encode()
    GOLDEN.write_bytes(gzip.compress(data, mtime=0))
    print(GOLDEN, len(data))

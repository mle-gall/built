# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The placement walks of ``core/group.py`` must walk as the Python
generators did at ``61fe6326``. ``iter_placements`` and
``iter_painted_placements`` and (round 11) ``visible_placements`` are
native iterators over the Rust document (``model/placement_walks.rs``),
and (round 12) ``model_placements``: ``iter_placements``, ``iter_painted_placements``,
``visible_placements`` and ``model_placements`` over seeded trees (classic
groups and instances, three levels, hidden and locked tags, tags in a
folder switched off, Hide, paints at any depth, a base matrix, a filter),
each placement with its matrix. They stay lazy: a child added to a node
after it was yielded is walked, as a generator resumed would.

Regenerate only on purpose, against the reference implementation:
``python tests/test_placement_walks_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "placement_walks_golden.json.gz"


def _walks():
    import built_core as N
    from core import group as G
    return (N.group_iter_placements, N.group_iter_painted_placements,
            N.group_visible_placements, N.group_model_placements)


def _scene(rng):
    from built_core import Matrix4 as M
    from core import layers as L
    from core.group import Group
    from core.mesh import Mesh
    from core.scene import Scene
    scene = Scene()
    for t in ("A", "B", "C"):
        scene.layers.append(L.Layer(t, visible=rng.random() < 0.6, locked=rng.random() < 0.3))
    if rng.random() < 0.5:
        f = __import__("built_core").tag_edits.add_folder(scene, "F")
        __import__("built_core").tag_edits.move_to_folder(scene, "C", f)
        scene.folder_visibility = {f: rng.random() < 0.5}
    count = [0]

    def node(depth):
        g = Group(Mesh(), name=f"n{count[0]}")
        g.uid = f"u{count[0]}"
        count[0] += 1
        if rng.random() < 0.7:
            x = M()
            x.translate(rng.uniform(-9, 9), rng.uniform(-9, 9), rng.uniform(-2, 2))
            x.rotate(rng.uniform(0, 360), 0.1, 0.2, 1.0)
            if rng.random() < 0.3:
                x.scale(rng.choice((2.0, -1.0, 0.5)), 1.0, 1.0)
            g.xform = x
        g.layer = rng.choice((None, None, "A", "B", "C", "ghost"))
        g.hidden = rng.random() < 0.2
        if rng.random() < 0.3:
            g.material = rng.choice(("Brick", "Oak"))
        if depth < 3:
            for _ in range(rng.choice((0, 0, 1, 2, 3))):
                g.children.append(node(depth + 1))
        return g

    for _ in range(rng.randint(1, 4)):
        scene.groups.append(node(0))
    return scene


def _m(m):
    if m is None:
        return None
    return [float(v) for v in (m.ravel() if hasattr(m, "ravel") else m.data())]


def answers():
    import itertools
    from built_core import Matrix4 as M
    from core import layers as L
    plain, painted, visible, model = _walks()
    counter = itertools.count()
    real = L.new_uid
    L.new_uid = lambda: f"k{next(counter)}"
    try:
        rng = random.Random(20261125)
        out = []
        for k in range(80):
            scene = _scene(rng)
            base = M()
            base.translate(1.0, 2.0, 3.0)
            rec = []
            for g in scene.groups:
                rec.append([[p.uid, _m(m)] for p, m in plain(g)])
                rec.append([[p.uid, _m(m)] for p, m in plain(g, base)])
                rec.append([[p.uid, _m(m), paint] for p, m, paint in painted(g)])
                rec.append([[p.uid, _m(m), paint] for p, m, paint in painted(g, base, "Steel")])
                rec.append([[p.uid, _m(m)] for p, m in visible(scene, g)])
                rec.append([[p.uid, _m(m)] for p, m in model(g)])
                rec.append([[p.uid, _m(m)] for p, m in model(g, None, lambda c: not c.hidden)])
            out.append(rec)
        return out
    finally:
        L.new_uid = real


def test_the_walks_answer_as_the_generators_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers()))
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        bad = [j for j, (a, b) in enumerate(zip(g, w)) if a != b]
        assert len(g) == len(w) and not bad, (i, bad[:5])


def test_the_walks_stay_lazy():
    """One placement per step, and a node's children read when the walk
    comes back to it."""
    from core.group import Group
    from core.mesh import Mesh
    plain, painted, _visible, model = _walks()
    for walk in (plain, painted, model):
        root, late = Group(Mesh(), name="root"), Group(Mesh(), name="late")
        it = iter(walk(root))
        assert next(it)[0] is root
        root.children.append(late)
        assert [row[0] for row in it] == [late]


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    GOLDEN.parent.mkdir(exist_ok=True)
    data = json.dumps(answers(), separators=(",", ":")).encode()
    GOLDEN.write_bytes(gzip.compress(data, mtime=0))
    d = json.loads(data)
    print(GOLDEN, len(data), sum(len(w) for s in d for w in s))

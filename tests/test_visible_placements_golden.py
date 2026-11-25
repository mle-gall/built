# SPDX-License-Identifier: GPL-3.0-or-later
"""``core.group.visible_placements`` must answer as the Python generator at
``6977fb85``: seeded trees four levels deep (rotated and untransformed
placements, Hide at any level, tags hidden, locked, in hidden folders, by
key and by label, unknown tags, untagged parts inheriting), every top-level
group walked, each yield as (uid, matrix values or None). The walk stays
lazy and hands out the placement's own matrix object where nothing above
it is transformed. ``tests/fixtures/visible_placements_golden.json`` holds
the answers; regenerate only on purpose:
``python tests/test_visible_placements_golden.py --write``."""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import pytest

GOLDEN = Path(__file__).parent / "fixtures" / "visible_placements_golden.json"
CASES = 60


def scene_of(seed):
    from built_core import Matrix4
    from core.group import Group
    from core.layers import Layer
    from core.mesh import Mesh
    from core.scene import Scene
    rng = random.Random(seed)
    s = Scene()
    s.layer_folders = ["F", "G"]
    s.folder_visibility = {"F": rng.random() < 0.5}
    tags = []
    for i in range(rng.randint(1, 6)):
        kw = dict(visible=rng.random() < 0.7, locked=rng.random() < 0.2)
        if rng.random() < 0.3:
            kw["folder"] = rng.choice(["F", "G"])
        if rng.random() < 0.3:
            kw["key"] = f"k{i}"
        ly = Layer(f"T{i}", **kw)
        s.layers.append(ly)
        tags.append(ly.key)
        if ly.key != ly.name:
            tags.append(ly.name)        # asked by label
    tags += ["Unknown", None, None, ""]
    n = [0]

    def make(depth):
        n[0] += 1
        g = Group(Mesh(), name=f"g{n[0]}", uid=f"u{seed}-{n[0]}")
        if rng.random() < 0.7:
            m = Matrix4()
            if rng.random() < 0.6:
                m.translate(rng.choice([0.0, 1.5]), rng.choice([0.0, -2.0]), 0)
                m.rotate(rng.choice([0.0, 30.0]), 0, 0, 1)
            g.xform = m
        g.layer = rng.choice(tags) or None
        g.hidden = rng.random() < 0.15
        if depth < 4 and rng.random() < 0.7:
            g.adopt([make(depth + 1) for _ in range(rng.randint(1, 3))])
        return g
    for _ in range(rng.randint(1, 4)):
        s.groups.append(make(0))
    return s


def _walk():
    import built_core
    native = getattr(built_core, "group_visible_placements", None)
    if native is not None:
        return native
    from core.group import visible_placements
    return visible_placements


def answers(seed):
    visible_placements = _walk()
    s = scene_of(seed)
    out = []
    for g in s.groups:
        rows = []
        for p, m in visible_placements(s, g):
            rows.append([p.uid, None if m is None else [round(v, 12) for v in m.data()]])
        out.append(rows)
    return out


@pytest.mark.parametrize("seed", range(CASES))
def test_visible_placements_answer_as_the_python_did(seed):
    assert answers(seed) == json.loads(GOLDEN.read_text())[seed]


def test_the_walk_is_lazy_and_hands_out_own_matrices():
    visible_placements = _walk()
    s = scene_of(3)
    g = s.groups[0]
    walk = visible_placements(s, g)
    top, m = next(walk)
    assert top is g and m is g.xform
    if g.xform is None:
        for child, cm in walk:
            if child.owner is None and child in list(g.children):
                assert cm is child.xform
                break


if __name__ == "__main__":
    if "--write" in sys.argv:
        GOLDEN.write_text(json.dumps([answers(seed) for seed in range(CASES)]))
        print("wrote", GOLDEN)

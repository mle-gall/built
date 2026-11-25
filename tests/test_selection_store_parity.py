# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A seeded run of native set operations must read exactly as the
pure-Python ``ElementSet`` read (``core/element_set.py`` at ``0a553619``).

``tests/fixtures/selection_golden.json.gz`` holds those answers; regenerate
only on purpose, against the reference implementation, with
``python tests/test_selection_store_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "selection_golden.json.gz"


def _pool():
    """Handles of every kind, with fresh handles of the same mesh elements
    (a store view is made anew on each read: same key, another object)."""
    from ingetrazo_core import Vec3 as V
    from core.dimension import Dimension
    from core.group import Group
    from core.guide import Guide
    from core.mesh import Mesh
    from core.scene import Scene
    scene = Scene()
    m = scene.loose_mesh
    for k in range(4):
        x = 2.0 * k
        m.add_face([V(x, 0, 0), V(x + 1, 0, 0), V(x + 1, 1, 0), V(x, 1, 0)])
    groups = []
    for k in range(4):
        g = Group(Mesh(), f"g{k}")
        g.mesh.add_face([V(0, 0, k), V(1, 0, k), V(1, 1, k)])
        scene.groups.append(g)
        groups.append(g)
    others = [Guide(V(0, 0, 0), V(1, 0, 0)), Guide(V(0, 0, 0), V(0, 1, 0)),
              Dimension(V(0, 0, 0), V(1, 0, 0), V(0, 1, 0))]
    labels = {}
    pool = []

    def put(handle, label):
        labels[id(handle)] = label
        pool.append(handle)

    for rnd in range(2):
        for i, f in enumerate(m.faces):
            put(f, f"face{i}.{rnd}")
        for i, e in enumerate(m.edges):
            put(e, f"edge{i}.{rnd}")
        for i, v in enumerate(m.vertices):
            put(v, f"vertex{i}.{rnd}")
    for i, g in enumerate(groups):
        put(g, f"group{i}")
    for i, o in enumerate(others):
        put(o, f"other{i}")
    return scene, pool, labels


def run(ElementSet):
    """Every answer of a seeded run, as labels."""
    scene, pool, labels = _pool()
    keep = [scene, pool]          # nothing freed: id() stays meaningful

    def lab(items):
        return [labels.get(id(v), "?") for v in items]

    rng = random.Random(20261124)
    a, b = ElementSet(), ElementSet()
    out = []
    for step in range(1500):
        op = rng.randrange(20)
        pick = rng.sample(pool, rng.randint(0, 6))
        one = rng.choice(pool)
        try:
            if op == 0:
                a.add(one)
            elif op == 1:
                a.discard(one)
            elif op == 2:
                a.update(pick)
            elif op == 3:
                a.difference_update(pick)
            elif op == 4:
                a.intersection_update(pick + list(a)[: rng.randint(0, 5)])
            elif op == 5:
                a.symmetric_difference_update(pick)
            elif op == 6:
                out.append(["union", lab(a.union(pick))])
            elif op == 7:
                out.append(["difference", lab(a.difference(pick))])
            elif op == 8:
                out.append(["intersection", lab(a.intersection(pick + list(a)[:3]))])
            elif op == 9:
                out.append(["symdiff", lab(a.symmetric_difference(pick))])
            elif op == 10:
                out.append(["subset", a.issubset(pick), a.issuperset(pick),
                            a <= ElementSet(pick), a == ElementSet(list(a))])
            elif op == 11:
                b = a.copy()
                b.add(one)
                out.append(["copy", lab(b), lab(a)])
            elif op == 12:
                if rng.random() < 0.2:
                    a.clear()
            elif op == 13:
                out.append(["contains", [v in a for v in pick]])
            elif op == 14:
                a |= ElementSet(pick)
            elif op == 15:
                a -= ElementSet(pick)
            elif op == 16:
                a &= ElementSet(pick + list(a))
            elif op == 17:
                a ^= ElementSet(pick)
            elif op == 18:
                if a:
                    out.append(["pop", lab([a.pop()])])
            else:
                c = ElementSet(pick)
                out.append(["made", lab(c), len(c), a.isdisjoint(c)])
        except Exception as exc:  # noqa: BLE001 — part of the answer
            out.append(["error", type(exc).__name__])
        out.append([len(a), lab(a)])
    del keep
    return out


def test_element_sets_read_as_the_python_ones_did():
    from ingetrazo_core import ElementSet
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(run(ElementSet)))
    assert len(got) == len(want)
    wrong = [i for i, (x, y) in enumerate(zip(got, want)) if x != y]
    assert not wrong, f"{len(wrong)} answers differ, first at {wrong[0]}: " \
        f"{got[wrong[0]]!r} != {want[wrong[0]]!r}"


def test_the_snapshot_counts_by_kind_and_moves_with_the_content():
    from ingetrazo_core import ElementSet
    scene, pool, labels = _pool()
    faces = [v for v in pool if labels[id(v)].startswith("face")]
    groups = [v for v in pool if labels[id(v)].startswith("group")]
    edges = [v for v in pool if labels[id(v)].startswith("edge")]
    s = ElementSet()
    r0 = s.revision
    s.update(faces[:3] + groups[:2] + edges[:1])
    count, placed, nf, ne, nv, other, r1 = s.snapshot()
    assert (count, placed, nf, ne, nv, other) == (6, 2, 3, 1, 0, 0)
    assert r1 > r0 == r0
    s.add(faces[0])                     # already in: nothing changes
    assert s.revision == r1
    s.discard(groups[3])                # not in: nothing changes
    assert s.revision == r1
    s.discard(groups[0])
    assert s.snapshot()[:2] == (5, 1) and s.revision > r1
    assert s.placement_uids() == [groups[1].uid]
    assert s.element_keys("face") == [f.key for f in faces[:3]]
    first, last = s.ends()
    assert first is faces[0] and last is edges[0]
    # A selection replaced by another object reads as a change too.
    before = scene.selection.revision
    scene.selection = set(groups)
    assert scene.selection.revision > before
    assert scene.selection.snapshot()[:2] == (4, 4)


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from ingetrazo_core import ElementSet as _ElementSet
    rows = run(_ElementSet)
    GOLDEN.write_bytes(gzip.compress(json.dumps(
        rows, separators=(",", ":")).encode(), mtime=0))
    print(len(rows), "answers ->", GOLDEN)


def test_collecting_a_selection_cycle_releases_its_handles():
    import gc
    import weakref
    from ingetrazo_core import ElementSet

    selection = ElementSet()
    selection.add(selection)
    ref = weakref.ref(selection)
    del selection
    gc.collect()
    assert ref() is None

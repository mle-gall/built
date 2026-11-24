# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""``ingetrazo_core``'s emulation of CPython's set of ints (``pyset.rs``)
iterates as the interpreter's own sets do, after any mix of the operations
the native plane rebuild replays: a set from a list, adds, ``set(s)``,
``s |= t``, ``s - t`` (both of CPython's paths) and discards."""
from __future__ import annotations

import random

import pytest

ingetrazo_core = pytest.importorskip("ingetrazo_core")
replay = getattr(ingetrazo_core, "py_set_replay", None)
pytestmark = pytest.mark.skipif(replay is None, reason="engine without py_set_replay")


def _python(ops):
    cur, saved, out = set(), [], []
    for op, arg in ops:
        if op == "add":
            cur.add(arg[0])
        elif op == "discard":
            cur.discard(arg[0])
        elif op == "new":
            cur = set(arg)
        elif op == "copy":
            cur = set(cur)
        elif op == "save":
            saved.append(set(cur))
        elif op == "update":
            cur.update(saved[arg[0]])
        elif op == "minus":
            cur = cur - saved[arg[0]]
        out.append(list(cur))
    return out


def _keys(rng):
    # Element keys: a namespace in the high bits, a kind and a slot below
    # (store keys are u128); some small ints and collisions mod 2**61 - 1.
    base = rng.choice([0, 1 << 64, (rng.getrandbits(40) << 64), (1 << 61) - 1])
    kind = rng.choice([0, 1, 2]) << 38
    return lambda: base | kind | rng.randrange(0, rng.choice([16, 200, 5000, 1 << 30]))


@pytest.mark.parametrize("seed", range(200))
def test_iteration_order_matches_cpython(seed):
    rng = random.Random(seed)
    key = _keys(rng)
    ops = [("new", [key() for _ in range(rng.randrange(0, 300))])]
    saved = 0
    for _ in range(rng.randrange(5, 60)):
        r = rng.random()
        if r < 0.35:
            ops.append(("add", [key()]))
        elif r < 0.45:
            ops.append(("discard", [key()]))
        elif r < 0.55:
            ops.append(("save", [0]))
            saved += 1
        elif r < 0.65 and saved:
            ops.append(("update", [rng.randrange(saved)]))
        elif r < 0.78 and saved:
            ops.append(("minus", [rng.randrange(saved)]))
        elif r < 0.85:
            ops.append(("copy", [0]))
        else:
            ops.append(("new", [key() for _ in range(rng.randrange(0, 2000))]))
    assert replay(ops) == _python(ops)


def test_large_sets_and_both_difference_paths():
    rng = random.Random(5)
    big = [(7 << 64) | (2 << 38) | rng.randrange(1 << 20) for _ in range(60000)]
    small = big[:100]
    ops = [("new", small), ("save", [0]), ("new", big), ("save", [0]),
           ("minus", [0]), ("copy", [0]), ("new", big[:1000]), ("minus", [1]),
           ("new", big[:40]), ("update", [1])]
    assert replay(ops) == _python(ops)


@pytest.mark.parametrize("seed", range(40))
def test_pair_sets_iterate_as_cpython(seed):
    """Sets of plane cells (int pairs, negative ones and big ones too), as
    the planar arrangement and the outline tracer build them."""
    order = getattr(ingetrazo_core, "py_pair_set_order", None)
    if order is None:
        pytest.skip("engine without py_pair_set_order")
    rng = random.Random(seed)
    span = rng.choice([3, 50, 10 ** 6, 2 ** 62, 2 ** 70])
    pairs = [(rng.randrange(-span, span), rng.randrange(-span, span))
             for _ in range(rng.randrange(0, rng.choice([6, 40, 3000])))]
    if pairs and rng.random() < 0.5:
        pairs += rng.sample(pairs, len(pairs) // 2)      # repeats
    s = set()
    for p in pairs:
        s.add(p)
    assert order(pairs, False) == list(s)
    assert order(pairs, True) == list(set(s))

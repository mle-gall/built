# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Rust painters (ingetrazo_core sheets::items) draw each sheet item as
the Python painters of views/composer.py drew it: the same primitives in
the same order, with the same pens, brushes, texts and geometry. The
goldens were recorded from the Python painters (tests/sheet_paint_recorder.py,
scripts/sheet_paint_goldens.py) before they moved (docs/rust-sheets.md §6).

Geometry matches within 1e-9 mm; flattened curves (paths) within 0.02 mm,
since Qt and Rust flatten arcs with different steps."""
from __future__ import annotations

import gzip
import json
import math
import os
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402
from PySide6.QtWidgets import QApplication  # noqa: E402

import ingetrazo_core as NATIVE  # noqa: E402
from tests.sheet_paint_cases import cases, profile_of  # noqa: E402

_app = QApplication.instance() or QApplication([])
FIXTURES = Path(__file__).parent / "fixtures" / "sheet_paint"
GROUPS = {g: json.loads(gzip.decompress((FIXTURES / f"goldens_{g}.json.gz").read_bytes()))
          for g in ("a", "b") if (FIXTURES / f"goldens_{g}.json.gz").exists()}
CASES = [(g, name, make) for g in GROUPS for name, make in cases(g)]


def _seg_dist(p, a, b):
    ax, ay = a
    bx, by = b
    dx, dy = bx - ax, by - ay
    L = dx * dx + dy * dy
    t = 0.0 if L == 0 else max(0.0, min(1.0, ((p[0] - ax) * dx + (p[1] - ay) * dy) / L))
    return math.hypot(p[0] - (ax + t * dx), p[1] - (ay + t * dy))


def _ring_dist(pts, ring):
    segs = list(zip(ring, ring[1:] + ring[:1]))
    return max(min(_seg_dist(p, a, b) for a, b in segs) for p in pts)


def _close(a, b, tol, where):
    if isinstance(a, (int, float)) and not isinstance(a, bool):
        assert abs(a - b) <= tol, (where, a, b)
    elif isinstance(a, list):
        assert len(a) == len(b), (where, len(a), len(b))
        for i, (x, y) in enumerate(zip(a, b)):
            _close(x, y, tol, f"{where}[{i}]")
    else:
        assert a == b, (where, a, b)


def assert_same_ops(got, want, name):
    assert [o["op"] for o in got] == [o["op"] for o in want], name
    for i, (g, w) in enumerate(zip(got, want)):
        where = f"{name}#{i}:{w['op']}"
        for key in set(w) | set(g):
            if key in ("op",) or (w["op"] == "image" and key == "source"):
                continue
            if key == "rings":
                assert len(g[key]) == len(w[key]), where
                for rg, rw in zip(g[key], w[key]):
                    assert _ring_dist(rg, rw) <= 0.02 and _ring_dist(rw, rg) <= 0.02, (where, rg, rw)
                continue
            _close(g.get(key), w.get(key), 1e-9, f"{where}.{key}")


def test_every_case_has_a_golden():
    for g, golden in GROUPS.items():
        assert sorted(golden) == sorted(name for name, _m in cases(g))


@pytest.mark.parametrize("group,name,make", CASES, ids=[c[1] for c in CASES])
def test_the_rust_painter_draws_what_python_drew(group, name, make):
    profile = profile_of(name) if name.startswith("profile-") else None
    got = json.loads(NATIVE.sheet_item_display(make(), profile))
    assert_same_ops(got, GROUPS[group][name], name)


@pytest.mark.parametrize("group,name,make", CASES, ids=[c[1] for c in CASES])
def test_the_box_is_the_record_s(group, name, make):
    """The item's box (what it is moved and resized by) is the one its
    record class states: ``w_mm``/``h_mm``, fields or derived."""
    item = make()
    if not hasattr(item, "w_mm") or not hasattr(item, "h_mm"):
        return                                       # a level mark has no box
    w, h = NATIVE.sheet_item_size(item)
    assert (w, h) == pytest.approx((float(item.w_mm), float(item.h_mm)), abs=1e-9)

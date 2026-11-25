# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Rust drawing pass (built_core sheets::drawing) draws what the
Python pass drew: the same segments, world points, kinds and layers, for
every synthetic case of tests/hlr_parity_cases.py. The goldens were written
by core/hlr.py before its loop moved to Rust (docs/rust-sheets.md §6)."""
from __future__ import annotations

import gzip
import json
import os
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402

from tests.hlr_parity_cases import canonical, cases  # noqa: E402

GOLDENS = json.loads(gzip.decompress(
    (Path(__file__).parent / "fixtures" / "hlr_parity" / "goldens.json.gz").read_bytes()))
CASES = cases()


def test_every_case_has_a_golden():
    assert sorted(GOLDENS) == sorted(name for name, _run in CASES)


@pytest.mark.parametrize("name,run", CASES, ids=[c[0] for c in CASES])
def test_the_drawing_matches_the_python_pass(name, run):
    want = GOLDENS[name]
    got = canonical(run())
    assert len(got) == len(want)
    for g, w in zip(got, want):
        assert g[:2] == tuple(w[:2]) or list(g[:2]) == w[:2], (name, g[:2], w[:2])
        assert max(abs(a - b) for a, b in zip(g[2:], w[2:])) < 1e-9, (name, g, w)

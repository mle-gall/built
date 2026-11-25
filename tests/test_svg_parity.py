# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Rust SVG writer (built_core sheets::svg, behind
formats.svg_out.svg_text) writes what the Python writer wrote for a view's
line groups: the same text, byte for byte, and the same chains. The
goldens were taken from the Python writer before the lines path moved to
Rust."""
from __future__ import annotations

from tests.platform_golden import matches
import gzip
import json
from pathlib import Path

import pytest

from formats.svg_out import chain_segments, svg_text
from tests.svg_parity_cases import cases, groups

GOLDENS = json.loads(gzip.decompress(
    (Path(__file__).parent / "fixtures" / "svg_parity" / "goldens.json.gz").read_bytes()))
CASES = cases()


def test_every_case_has_a_golden():
    assert sorted(GOLDENS["svg"]) == sorted(name for name, _kw in CASES)


@pytest.mark.parametrize("name,kw", CASES, ids=[c[0] for c in CASES])
def test_the_svg_is_the_python_writer_s(name, kw):
    text, n = svg_text(**kw)
    want_text, want_n = GOLDENS["svg"][name]
    assert n == want_n
    assert text == want_text


@pytest.mark.parametrize("seed", range(10))
def test_the_chains_are_the_python_writer_s(seed):
    for gi, g in enumerate(groups(seed)):
        got = [[[list(p) for p in pts], closed] for pts, closed in chain_segments(g[1])]
        assert matches(got, GOLDENS["chains"][f"{seed}-{gi}"]), (seed, gi)

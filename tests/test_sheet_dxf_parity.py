# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The DXF writer (built_core sheets::dxf) writes byte for byte what
formats/dxf_out.py wrote (goldens recorded from it before it moved)."""
from __future__ import annotations

import gzip
import json
from pathlib import Path

import pytest

from tests.sheet_dxf_cases import cases

GOLDENS = json.loads(gzip.decompress((Path(__file__).parent / "fixtures" / "sheet_dxf" / "goldens.json.gz")
                                    .read_bytes()))
CASES = cases()


@pytest.mark.parametrize("name", sorted(CASES))
def test_the_dxf_is_the_python_writer_s(name, tmp_path):
    from formats.dxf_out import save_dxf_layers
    groups, solids, texts = CASES[name]
    path = tmp_path / "view.dxf"
    n = save_dxf_layers(path, groups, solids, texts)
    want_n, want = GOLDENS[name]
    assert n == want_n
    assert path.read_bytes().decode("ascii") == want

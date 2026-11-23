# SPDX-License-Identifier: GPL-3.0-or-later
"""The tools replay the scenarios recorded from the Python tools before
they moved to Rust (docs/rust-tools.md §11, tests/tool_parity.py)."""
import pytest

from tests import tool_parity as tp

LINE = tp.load("line")
SELECT = tp.load("select")


@pytest.mark.parametrize("name", sorted(tp.LINE_SCENARIOS))
def test_line_matches_the_recorded_python_tool(name):
    far = name == "far_from_the_origin"
    got = tp.run_line(tp.LINE_SCENARIOS[name], far=far)
    want = LINE[name]
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        g = {k: (list(map(list, v)) if k == "edges" else v) for k, v in g.items()}
        w = {k: (list(map(list, v)) if k == "edges" else v) for k, v in w.items()}
        assert g == w, f"step {i}: {tp.LINE_SCENARIOS[name][i]}"


@pytest.mark.parametrize("name", sorted(tp.SELECT_SCENARIOS))
def test_select_matches_the_recorded_python_tool(name):
    got = tp.run_select(tp.SELECT_SCENARIOS[name])
    assert got == SELECT[name]

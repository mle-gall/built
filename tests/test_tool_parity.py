# SPDX-License-Identifier: GPL-3.0-or-later
"""The tools replay the scenarios recorded from the Python tools before
they moved to Rust (docs/rust-tools.md §11, tests/tool_parity.py)."""
import json

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


RECT = tp.load("rectangle")


@pytest.mark.parametrize("name", sorted(tp.RECT_SCENARIOS))
def test_rectangle_matches_the_recorded_python_tool(name):
    far = name == "typed_far_from_the_origin"
    got = tp.run_rect(tp.RECT_SCENARIOS[name], far=far)
    want = RECT[name]
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        g = json.loads(json.dumps(g))
        assert g == w, f"step {i}: {tp.RECT_SCENARIOS[name][i]}"


MOVE = tp.load("move")


@pytest.mark.parametrize("name", sorted(tp.MOVE_SCENARIOS))
def test_move_matches_the_recorded_python_tool(name):
    got = tp.run_move(tp.MOVE_SCENARIOS[name])
    want = MOVE[name]
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        g = json.loads(json.dumps(g))
        assert g == w, f"step {i}: {tp.MOVE_SCENARIOS[name][i]}"


CIRCLE = tp.load("circle")


@pytest.mark.parametrize("name,steps,polygon,far", list(tp.circle_cases()),
                         ids=[c[0] for c in tp.circle_cases()])
def test_circle_and_polygon_match_the_recorded_python_tools(name, steps, polygon, far):
    got = tp.run_circle(steps, polygon=polygon, far=far)
    want = CIRCLE[name]
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        g = json.loads(json.dumps(g))
        assert g == w, f"step {i}: {steps[i]}"


ROT = tp.load("rotated_rectangle")


@pytest.mark.parametrize("name", sorted(tp.ROT_SCENARIOS))
def test_rotated_rectangle_matches_the_recorded_python_tool(name):
    got = tp.run_rot(tp.ROT_SCENARIOS[name])
    want = ROT[name]
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        g = json.loads(json.dumps(g))
        assert g == w, f"step {i}: {tp.ROT_SCENARIOS[name][i]}"


ARC = tp.load("arc")


@pytest.mark.parametrize("name", sorted(tp.ARC_SCENARIOS))
def test_the_arcs_match_the_recorded_python_tools(name):
    got = tp.run_arc(name, tp.ARC_SCENARIOS[name])
    want = ARC[name]
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        g = json.loads(json.dumps(g))
        assert g == w, f"step {i}: {tp.ARC_SCENARIOS[name][i]}"


@pytest.mark.parametrize("name", sorted(tp.SCALE_SCENARIOS))
def test_scale_matches_the_recorded_python_tool(name):
    assert tp.run_scale(tp.SCALE_SCENARIOS[name]) == tp.load("scale")[name]


@pytest.mark.parametrize("name", sorted(tp.ROTATE_SCENARIOS))
def test_rotate_matches_the_recorded_python_tool(name):
    assert tp.run_rotate(tp.ROTATE_SCENARIOS[name]) == tp.load("rotate")[name]


@pytest.mark.parametrize("name", sorted(tp.WALKTHROUGH_SCENARIOS))
def test_walkthrough_matches_the_recorded_python_tool(name):
    assert tp.run_walkthrough(*tp.WALKTHROUGH_SCENARIOS[name]) == tp.load("walkthrough")[name]


@pytest.mark.parametrize("name", sorted(tp.OFFSET_SCENARIOS))
def test_offset_matches_the_recorded_python_tool(name):
    assert tp.run_offset(tp.OFFSET_SCENARIOS[name]) == tp.load("offset")[name]


@pytest.mark.parametrize("name", sorted(tp.PAINT_SCENARIOS))
def test_paint_matches_the_recorded_python_tool(name):
    assert tp.run_paint(tp.PAINT_SCENARIOS[name]) == tp.load("paint")[name]


@pytest.mark.parametrize("name", sorted(tp.PROTRACTOR_SCENARIOS))
def test_protractor_matches_the_recorded_python_tool(name):
    assert tp.run_protractor(tp.PROTRACTOR_SCENARIOS[name]) == tp.load("protractor")[name]


@pytest.mark.parametrize("name", sorted(tp.FILLET_SCENARIOS))
def test_fillet_matches_the_recorded_python_tool(name):
    assert tp.run_fillet(tp.FILLET_SCENARIOS[name]) == tp.load("fillet")[name]


@pytest.mark.parametrize("name", sorted(tp.TAPE_SCENARIOS))
def test_tape_matches_the_recorded_python_tool(name):
    assert tp.run_tape(tp.TAPE_SCENARIOS[name]) == tp.load("tape")[name]


@pytest.mark.parametrize("name", sorted(tp.PASTE_SCENARIOS))
def test_paste_matches_the_recorded_python_tool(name):
    assert tp.run_paste(tp.PASTE_SCENARIOS[name]) == tp.load("paste")[name]


@pytest.mark.parametrize("name", sorted(tp.FLIP_SCENARIOS))
def test_flip_matches_the_recorded_python_tool(name):
    assert tp.run_flip(tp.FLIP_SCENARIOS[name]) == tp.load("flip")[name]


@pytest.mark.parametrize("name", sorted(tp.FREEHAND_SCENARIOS))
def test_freehand_matches_the_recorded_python_tool(name):
    assert tp.run_freehand(tp.FREEHAND_SCENARIOS[name]) == tp.load("freehand")[name]


@pytest.mark.parametrize("name", sorted(tp.CHANGE_AXES_SCENARIOS))
def test_change_axes_matches_the_recorded_python_tool(name):
    assert tp.run_change_axes(tp.CHANGE_AXES_SCENARIOS[name]) == tp.load("change_axes")[name]
